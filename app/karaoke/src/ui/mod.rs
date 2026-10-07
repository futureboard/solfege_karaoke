//! Screen layout: song list on the left, queue on the right, transport at
//! the bottom and the lyric stage in the middle.

pub mod files;
mod library;
mod queue;
mod settings;
mod stage;
mod transport;

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Frame, Margin, Panel, RichText, Stroke};

use crate::icons;
use crate::app::KaraokeApp;
use crate::style::{self, ACCENT, DANGER, DIM, INK, LINE, PANEL, RAISED, SUNG, TEXT};

pub fn show(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    if !app.stage_only {
        Panel::top("top")
            .frame(Frame::new().fill(INK).inner_margin(Margin::symmetric(14, 8)))
            .show_separator_line(false)
            .show(ui, |ui| top_bar(app, ui));
        Panel::bottom("transport")
            .frame(Frame::new().fill(PANEL).inner_margin(Margin::symmetric(14, 10)).stroke(Stroke::new(1.0, LINE)))
            .show_separator_line(false)
            .show(ui, |ui| transport::show(app, ui));
        Panel::left("library")
            .frame(side_frame())
            .default_size(360.0)
            .size_range(260.0..=560.0)
            .show(ui, |ui| library::show(app, ui));
        Panel::right("queue")
            .frame(side_frame())
            .default_size(280.0)
            .size_range(220.0..=420.0)
            .show(ui, |ui| queue::show(app, ui));
    }
    egui::CentralPanel::no_frame().show(ui, |ui| stage::show(app, ui));

    settings::show(app, &ctx);
    files::show(app, &ctx);
    toasts(app, &ctx);
}

fn side_frame() -> Frame {
    Frame::new().fill(PANEL).inner_margin(Margin::same(12))
}

fn top_bar(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        logo(ui);
        ui.add_space(6.0);
        ui.label(RichText::new("Solfege").size(18.0).strong().color(TEXT));
        ui.label(RichText::new("Karaoke").size(18.0).color(SUNG));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(format!("{}  ตั้งค่า", icons::SETTINGS)).on_hover_text("คลังเพลง, SoundFont, อุปกรณ์เสียง").clicked() {
                app.show_settings = !app.show_settings;
            }
            if ui.button(format!("{}  เต็มจอ", icons::FULLSCREEN)).on_hover_text("แสดงเฉพาะเนื้อร้อง (F / F11, Esc เพื่อออก)").clicked() {
                let ctx = ui.ctx().clone();
                app.set_stage_only(&ctx, true);
            }
            ui.add_space(8.0);
            status_chip(app, ui);
        });
    });
}

/// Two overlapping note heads: the app mark.
fn logo(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
    let p = ui.painter();
    p.circle_filled(rect.left_center() + egui::vec2(9.0, 3.0), 7.0, SUNG);
    p.circle_filled(rect.left_center() + egui::vec2(17.0, -3.0), 7.0, ACCENT.gamma_multiply(0.85));
    p.line_segment(
        [rect.left_center() + egui::vec2(23.5, -3.0), rect.left_center() + egui::vec2(23.5, -14.0)],
        Stroke::new(2.0, ACCENT),
    );
}

fn status_chip(app: &KaraokeApp, ui: &mut egui::Ui) {
    let (text, color) = if app.synth.loading_soundfont() {
        (format!("{}  กำลังโหลด SoundFont…", icons::LOADER), DIM)
    } else if app.synth.output_error.is_some() {
        (format!("{}  ไม่มีอุปกรณ์เสียง", icons::ALERT), DANGER)
    } else if let Some(name) = app.synth.soundfont_name() {
        (format!("{}  {name}", icons::MUSIC), DIM)
    } else {
        (format!("{}  ยังไม่มี SoundFont", icons::ALERT), DANGER)
    };
    let r = ui.label(RichText::new(text).size(12.0).color(color));
    if let Some(e) = &app.synth.output_error {
        r.on_hover_text(e);
    } else {
        r.on_hover_text(&app.synth.output_info);
    }
}

fn toasts(app: &KaraokeApp, ctx: &egui::Context) {
    if app.toasts.is_empty() {
        return;
    }
    let now = ctx.input(|i| i.time);
    egui::Area::new(egui::Id::new("toasts"))
        .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-16.0, if app.stage_only { -16.0 } else { -86.0 }))
        .interactable(false)
        .show(ctx, |ui| {
            for t in app.toasts.iter().rev().take(4) {
                let age = (now - t.at) as f32;
                let fade = (age * 4.0).min(1.0);
                Frame::new()
                    .fill(RAISED.gamma_multiply(0.96 * fade))
                    .stroke(Stroke::new(1.0, if t.error { DANGER } else { LINE }.gamma_multiply(fade)))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_max_width(380.0);
                        let c = if t.error { style::mix(TEXT, DANGER, 0.35) } else { TEXT };
                        ui.label(RichText::new(&t.text).font(FontId::proportional(13.0)).color(c.gamma_multiply(fade)));
                    });
                ui.add_space(6.0);
            }
        });
}

/// `m:ss`.
pub fn clock(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Small rounded label with a coloured outline.
pub fn chip(ui: &mut egui::Ui, text: impl Into<String>, color: Color32) -> egui::Response {
    Frame::new()
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.7)))
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| ui.label(RichText::new(text.into()).size(12.0).color(color)))
        .response
}

#[cfg(test)]
mod tests {
    #[test]
    fn clock_format() {
        assert_eq!(super::clock(0.0), "0:00");
        assert_eq!(super::clock(65.4), "1:05");
        assert_eq!(super::clock(-3.0), "0:00");
    }
}
