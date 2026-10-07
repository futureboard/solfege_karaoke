//! Play controls, seek bar, key / tempo / volume and the part mutes.

use eframe::egui::{self, Align2, CornerRadius, FontId, RichText, Sense, Stroke};
use solfege_synth::engine::PlayState;

use crate::app::KaraokeApp;
use crate::music::{signed, transpose_key};
use crate::style::{self, ACCENT, DIM, INK, LINE, RAISED, SUNG, SUNG_HOT, TEXT};
use crate::synth::KEY_RANGE;
use crate::ui::clock;

pub fn show(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let playing = app.synth.state() == PlayState::Playing;
        if round_button(ui, 44.0, if playing { "⏸" } else { "▶" }, true).on_hover_text("เล่น / พัก (Space)").clicked() {
            if app.now.as_ref().is_some_and(|n| n.finished) {
                if let Some(n) = &mut app.now {
                    n.finished = false;
                }
                app.synth.play();
            } else {
                app.synth.toggle();
            }
        }
        if round_button(ui, 34.0, "⏹", false).on_hover_text("หยุด").clicked() {
            app.synth.stop();
        }
        if round_button(ui, 34.0, "⏭", false).on_hover_text("เพลงถัดไปในคิว (N)").clicked() {
            app.play_next();
        }
        ui.add_space(10.0);

        // Right-hand controls are laid out first so the seek bar takes the rest.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            volume(app, ui);
            ui.add_space(6.0);
            parts(app, ui);
            ui.add_space(6.0);
            tempo(app, ui);
            ui.add_space(6.0);
            key(app, ui);
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| seek_bar(app, ui));
        });
    });
}

fn round_button(ui: &mut egui::Ui, d: f32, icon: &str, primary: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(d, d), Sense::click());
    let hot = resp.hovered();
    let fill = match (primary, hot) {
        (true, false) => SUNG,
        (true, true) => SUNG_HOT,
        (false, false) => RAISED,
        (false, true) => style::mix(RAISED, ACCENT, 0.25),
    };
    let p = ui.painter();
    p.circle_filled(rect.center(), d / 2.0, fill);
    p.text(rect.center(), Align2::CENTER_CENTER, icon, FontId::proportional(d * 0.42), if primary { INK } else { TEXT });
    resp
}

fn seek_bar(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    let dur = app.synth.duration();
    let t = app.scrub.unwrap_or_else(|| app.synth.time());
    ui.label(RichText::new(clock(t)).monospace().color(TEXT));
    let w = (ui.available_width() - 56.0).max(80.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 30.0), Sense::click_and_drag());
    let track = egui::Rect::from_center_size(rect.center(), egui::vec2(rect.width(), 6.0));
    let p = ui.painter();
    p.rect_filled(track, CornerRadius::same(3), LINE);
    if dur > 0.0 {
        let x_of = |secs: f64| track.left() + track.width() * (secs / dur).clamp(0.0, 1.0) as f32;
        // Line starts as ticks, so verses and breaks are visible.
        if let Some(now) = &app.now {
            for l in &now.timeline.lines {
                let x = x_of(l.start);
                p.line_segment([egui::pos2(x, track.top() - 5.0), egui::pos2(x, track.top() - 1.0)], Stroke::new(1.0, DIM.gamma_multiply(0.6)));
            }
        }
        let mut fill = track;
        fill.set_right(x_of(t));
        p.rect_filled(fill, CornerRadius::same(3), SUNG);
        let knob = egui::pos2(x_of(t), track.center().y);
        p.circle_filled(knob, if resp.hovered() || resp.dragged() { 8.0 } else { 6.0 }, SUNG_HOT);

        let to_time = |x: f32| ((x - track.left()) / track.width()).clamp(0.0, 1.0) as f64 * dur;
        if let Some(pos) = resp.interact_pointer_pos() {
            if resp.dragged() {
                app.scrub = Some(to_time(pos.x));
            }
            if resp.clicked() {
                app.synth.seek(to_time(pos.x));
            }
        }
        if resp.drag_stopped()
            && let Some(s) = app.scrub.take()
        {
            app.synth.seek(s);
        }
        if let Some(h) = resp.hover_pos() {
            resp.on_hover_text_at_pointer(clock(to_time(h.x)));
        }
    }
    ui.label(RichText::new(clock(dur)).monospace().color(DIM));
}

fn stepper(ui: &mut egui::Ui, label: &str, value: String, hint: &str) -> i32 {
    let mut d = 0;
    egui::Frame::new()
        .fill(INK)
        .corner_radius(CornerRadius::same(255))
        .inner_margin(egui::Margin::symmetric(4, 2))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            // Right-to-left parent: add in reverse visual order.
            if ui.small_button("+").clicked() {
                d = 1;
            }
            ui.add_sized([62.0, 20.0], egui::Label::new(RichText::new(value).strong().color(TEXT)));
            if ui.small_button("−").clicked() {
                d = -1;
            }
            ui.label(RichText::new(label).size(12.0).color(DIM));
        })
        .response
        .on_hover_text(hint);
    d
}

fn key(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    let k = app.synth.key();
    let value = match app.now.as_ref().and_then(|n| n.song.key.as_deref()).and_then(|key| transpose_key(key, k)) {
        Some(name) if k != 0 => format!("{name} ({})", signed(k)),
        Some(name) => name,
        None => signed(k),
    };
    let d = stepper(ui, "คีย์", value, "เปลี่ยนคีย์ทีละครึ่งเสียง ( [ / ] ) — กลองไม่เปลี่ยน");
    if d != 0 {
        app.synth.set_key((k + d).clamp(-KEY_RANGE, KEY_RANGE));
    }
}

fn tempo(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    let s = app.synth.speed();
    let d = stepper(ui, "ความเร็ว", format!("{:.0}%", s * 100.0), "ช้าลง / เร็วขึ้น ( , / . )");
    if d != 0 {
        app.synth.set_speed(s + d as f64 * 0.05);
    }
}

fn volume(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    let mut v = app.synth.volume();
    ui.spacing_mut().slider_width = 90.0;
    if ui.add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(false)).on_hover_text("ระดับเสียงดนตรี").changed() {
        app.synth.set_volume(v);
    }
    ui.label(RichText::new(if v <= 0.001 { "🔇" } else { "🔊" }).color(DIM));
}

/// Mute any of the 16 MIDI parts, e.g. the guide melody.
fn parts(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    let muted = app.synth.mutes().count_ones();
    let label = if muted > 0 { format!("แทร็ก ({muted} ปิด)") } else { "แทร็ก".to_string() };
    let resp = ui.button(label).on_hover_text("ปิด/เปิดเสียงแต่ละแชนแนล เช่น เมโลดี้นำร้อง");
    egui::Popup::menu(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.label(RichText::new("แชนแนล MIDI").strong());
            ui.add_space(4.0);
            let used = app.synth.channels_used();
            let mut mutes = app.synth.mutes();
            egui::Grid::new("parts").spacing([6.0, 6.0]).show(ui, |ui| {
                for ch in 0..16usize {
                    let on = mutes & (1 << ch) == 0;
                    let active = used & (1 << ch) != 0;
                    let level = app.synth.activity(ch);
                    let (rect, r) = ui.allocate_exact_size(egui::vec2(46.0, 34.0), Sense::click());
                    let p = ui.painter();
                    let fill = if !active { INK } else if on { RAISED } else { style::mix(INK, style::DANGER, 0.25) };
                    p.rect_filled(rect, CornerRadius::same(8), fill);
                    if on && active {
                        let mut bar = rect.shrink(4.0);
                        bar.set_top(bar.bottom() - 3.0);
                        bar.set_width(bar.width() * level.clamp(0.0, 1.0));
                        p.rect_filled(bar, CornerRadius::same(2), ACCENT);
                    }
                    let c = if !active { DIM.gamma_multiply(0.5) } else if on { TEXT } else { DIM };
                    p.text(rect.center() - egui::vec2(0.0, 3.0), Align2::CENTER_CENTER, (ch + 1).to_string(), FontId::proportional(13.0), c);
                    if ch == 9 {
                        p.text(rect.center() + egui::vec2(0.0, 9.0), Align2::CENTER_CENTER, "กลอง", FontId::proportional(9.0), c);
                    }
                    if r.clicked() && active {
                        mutes ^= 1 << ch;
                    }
                    if ch % 4 == 3 {
                        ui.end_row();
                    }
                }
            });
            if mutes != app.synth.mutes() {
                app.synth.set_mutes(mutes);
            }
            ui.add_space(4.0);
            if ui.button("เปิดทุกแชนแนล").clicked() {
                app.synth.set_mutes(0);
            }
        });
}
