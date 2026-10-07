//! Screen layout: the lyric stage fills the window, the bottom bar sits
//! under it, and everything else opens as an overlay on top. Full screen
//! only changes the window; the layout stays the same. Right click opens
//! context menus (`menu`).

mod bar;
pub mod menu;
mod mixer;
pub mod overlay;
pub mod sound;
mod stage;

use eframe::egui::{self, Align2, CornerRadius, FontId, Frame, Margin, Panel, RichText, Stroke};

use crate::app::KaraokeApp;
use crate::style::{self, DANGER, INK, LINE, PANEL, TEXT};

pub fn show(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    Panel::bottom("bar")
        .frame(Frame::new().fill(INK))
        .exact_size(bar::HEIGHT)
        .resizable(false)
        .show_separator_line(false)
        .show(ui, |ui| bar::show(app, ui));
    if app.mixer_open {
        Panel::bottom("mixer")
            .frame(Frame::new().fill(PANEL).stroke(Stroke::new(1.0, LINE)))
            .exact_size(mixer::HEIGHT)
            .resizable(false)
            .show_separator_line(false)
            .show(ui, |ui| mixer::show(app, ui));
    }
    egui::CentralPanel::no_frame().show(ui, |ui| stage::show(app, ui));
    overlay::show(app, &ctx);
    sound::show(app, &ctx);
    toasts(app, &ctx);
}

fn toasts(app: &KaraokeApp, ctx: &egui::Context) {
    // The overlay has the user's attention; notes wait until it closes.
    if app.toasts.is_empty() || app.overlay.is_some() || app.sound.is_some() {
        return;
    }
    let now = ctx.input(|i| i.time);
    let lift = bar::HEIGHT + 12.0 + if app.mixer_open { mixer::HEIGHT } else { 0.0 };
    egui::Area::new(egui::Id::new("toasts"))
        .order(egui::Order::Tooltip)
        .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -lift))
        .interactable(false)
        .show(ctx, |ui| {
            for t in app.toasts.iter().rev().take(4) {
                let fade = ((now - t.at) as f32 * 4.0).min(1.0);
                Frame::new()
                    .fill(PANEL.gamma_multiply(0.97 * fade))
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

#[cfg(test)]
mod tests {
    #[test]
    fn clock_format() {
        assert_eq!(super::clock(0.0), "0:00");
        assert_eq!(super::clock(65.4), "1:05");
        assert_eq!(super::clock(-3.0), "0:00");
    }
}
