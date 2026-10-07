//! Mute any of the 16 MIDI parts, e.g. the guide melody.

use eframe::egui::{self, Align2, CornerRadius, FontId, Margin, Sense, Stroke, pos2, vec2};

use crate::app::KaraokeApp;
use crate::icons;
use crate::style::{self, ACCENT, DANGER, DIM, INK, LINE, RAISED, TEXT};

pub fn show(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    egui::Frame::new().inner_margin(Margin::same(18)).show(ui, |ui| {
        let used = app.synth.channels_used();
        if used == 0 {
            ui.label(egui::RichText::new("ยังไม่มีเพลง — เลือกเพลงก่อนแล้วกลับมาที่หน้านี้").color(DIM));
            return;
        }
        ui.label(egui::RichText::new("คลิกเพื่อปิด / เปิดเสียงแชนแนล — เช่นปิดเมโลดี้นำร้อง").size(13.0).color(DIM));
        ui.add_space(12.0);
        let mut mutes = app.synth.mutes();
        let gap = 8.0;
        let w = (ui.available_width() - gap * 7.0) / 8.0;
        for row in 0..2 {
            let (band, _) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::hover());
            for col in 0..8 {
                let ch = row * 8 + col;
                let rect = egui::Rect::from_min_size(pos2(band.left() + col as f32 * (w + gap), band.top()), vec2(w, 64.0));
                let resp = ui.interact(rect, ui.id().with(("track", ch)), Sense::click());
                let active = used & (1 << ch) != 0;
                let on = mutes & (1 << ch) == 0;
                let p = ui.painter();
                let fill = if !active { INK } else if resp.hovered() { style::mix(RAISED, TEXT, 0.06) } else { RAISED };
                p.rect_filled(rect, CornerRadius::same(10), fill);
                if active && !on {
                    p.rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, DANGER.gamma_multiply(0.7)), egui::StrokeKind::Inside);
                }
                let c = if !active { DIM.gamma_multiply(0.4) } else if on { TEXT } else { DIM };
                p.text(rect.left_top() + vec2(10.0, 8.0), Align2::LEFT_TOP, (ch + 1).to_string(), FontId::proportional(16.0), c);
                let icon = if ch == 9 { icons::DRUM } else if on { icons::VOLUME } else { icons::MUTE };
                let ic = if active && !on { DANGER } else { c };
                p.text(rect.right_top() + vec2(-10.0, 9.0), Align2::RIGHT_TOP, icon, FontId::proportional(14.0), ic);
                if active {
                    let track = egui::Rect::from_min_max(pos2(rect.left() + 10.0, rect.bottom() - 13.0), pos2(rect.right() - 10.0, rect.bottom() - 10.0));
                    p.rect_filled(track, 1.5, LINE);
                    if on {
                        let mut lit = track;
                        lit.set_width(track.width() * app.synth.activity(ch).clamp(0.0, 1.0));
                        p.rect_filled(lit, 1.5, ACCENT);
                    }
                }
                if resp.clicked() && active {
                    mutes ^= 1 << ch;
                }
            }
            if row == 0 {
                ui.add_space(gap);
            }
        }
        if mutes != app.synth.mutes() {
            app.synth.set_mutes(mutes);
        }
        ui.add_space(12.0);
        if ui.add_enabled(mutes != 0, egui::Button::new(format!("{}  เปิดเสียงทุกแทร็ก", icons::VOLUME))).clicked() {
            app.synth.set_mutes(0);
        }
    });
    // Activity bars move with the music.
    ui.ctx().request_repaint();
}
