//! Now playing and the queue of reserved songs.

use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, Stroke};

use crate::icons;
use crate::app::KaraokeApp;
use crate::music::transpose_key;
use crate::style::{ACCENT, DIM, INK, LINE, SUNG, TEXT};
use crate::ui::{chip, clock};

pub fn show(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    ui.label(RichText::new(format!("{}  กำลังร้อง", icons::MIC)).size(16.0).strong().color(TEXT));
    ui.add_space(4.0);
    now_card(app, ui);
    ui.add_space(14.0);

    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{}  คิวเพลง", icons::QUEUE)).size(16.0).strong().color(TEXT));
        ui.label(RichText::new(app.queue.len().to_string()).size(12.0).color(DIM));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !app.queue.is_empty() && ui.small_button(icons::TRASH).on_hover_text("ล้างคิว").clicked() {
                app.queue.clear();
            }
            if ui.add_enabled(!app.queue.is_empty(), egui::Button::new(format!("{}  ถัดไป", icons::NEXT)).small()).clicked() {
                app.play_next();
            }
        });
    });
    ui.add_space(4.0);
    if app.queue.is_empty() {
        ui.label(RichText::new("ว่าง — กด + ที่เพลงเพื่อจองคิว").size(12.0).color(DIM));
        return;
    }

    enum Op {
        Up(usize),
        Remove(usize),
        Play(usize),
    }
    let mut op = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for (i, h) in app.queue.iter().enumerate() {
            let card = Frame::new()
                .fill(INK)
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{}", i + 1)).size(18.0).strong().color(ACCENT));
                        ui.vertical(|ui| {
                            ui.set_max_width(ui.available_width() - 64.0);
                            ui.add(egui::Label::new(RichText::new(&h.title).color(TEXT)).truncate());
                            ui.add(egui::Label::new(RichText::new(&h.artist).size(12.0).color(DIM)).truncate());
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button(icons::CLOSE).on_hover_text("เอาออก").clicked() {
                                op = Some(Op::Remove(i));
                            }
                            if i > 0 && ui.small_button(icons::MOVE_UP).on_hover_text("เลื่อนขึ้น").clicked() {
                                op = Some(Op::Up(i));
                            }
                        });
                    });
                })
                .response
                .interact(egui::Sense::click())
                .on_hover_text("ดับเบิลคลิกเพื่อร้องเลย");
            if card.double_clicked() {
                op = Some(Op::Play(i));
            }
            ui.add_space(6.0);
        }
    });
    match op {
        Some(Op::Up(i)) => app.queue.swap(i, i - 1),
        Some(Op::Remove(i)) => {
            app.queue.remove(i);
        }
        Some(Op::Play(i)) => {
            if let Some(h) = app.queue.remove(i) {
                app.play_now(h);
            }
        }
        None => {}
    }
}

fn now_card(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    let Some(now) = &app.now else {
        Frame::new()
            .stroke(Stroke::new(1.0, LINE))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("ยังไม่ได้เลือกเพลง").color(DIM));
            });
        return;
    };
    let t = app.synth.time();
    let dur = app.synth.duration().max(0.001);
    Frame::new()
        .fill(INK)
        .stroke(Stroke::new(1.0, SUNG.gamma_multiply(0.5)))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(&now.header.title).size(17.0).strong().color(TEXT)).wrap());
            if !now.header.artist.is_empty() {
                ui.label(RichText::new(&now.header.artist).color(DIM));
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let key = app.synth.key();
                if let Some(k) = &now.song.key {
                    let shown = transpose_key(k, key).unwrap_or_else(|| k.clone());
                    chip(ui, format!("{}  คีย์ {shown}", icons::KEY), ACCENT);
                }
                if let Some(bpm) = app.bpm() {
                    chip(ui, format!("{}  {bpm:.0} BPM", icons::METRONOME), DIM);
                }
            });
            ui.add_space(6.0);
            let (bar, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 4.0), egui::Sense::hover());
            ui.painter().rect_filled(bar, CornerRadius::same(2), LINE);
            let mut fill = bar;
            fill.set_width(bar.width() * (t / dur).clamp(0.0, 1.0) as f32);
            ui.painter().rect_filled(fill, CornerRadius::same(2), SUNG);
            ui.label(RichText::new(format!("{} / {}", clock(t), clock(dur))).size(12.0).color(DIM));
        });
}
