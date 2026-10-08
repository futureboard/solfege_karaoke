//! Screen layout: the lyric stage fills the window, the bottom bar sits
//! under it, and everything else opens as an overlay on top. Full screen
//! only changes the window; the layout stays the same. Right click opens
//! context menus (`menu`).

mod bar;
pub mod effects;
pub mod menu;
mod mixer;
pub mod overlay;
pub mod screen2;
pub mod sound;
pub mod stage;

use eframe::egui::{self, Align2, CornerRadius, FontId, Frame, Margin, Panel, RichText, Stroke};

use crate::app::KaraokeApp;
use crate::style::{self, DANGER, INK, LINE, PANEL, TEXT};

pub fn show(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    // The background fills everything above the bottom bar; the mixer
    // opens over it instead of pushing it up.
    let full = ui.max_rect();
    let behind = egui::Rect::from_min_max(full.min, egui::pos2(full.max.x, full.max.y - bar::HEIGHT));
    app.backdrop.paint(ui.painter(), behind, &app.settings.background, ui.input(|i| i.time));
    Panel::bottom("bar")
        .frame(Frame::new().fill(INK))
        .exact_size(bar::HEIGHT)
        .resizable(false)
        .show_separator_line(false)
        .show(ui, |ui| bar::show(app, ui));
    if app.mixer_open {
        Panel::bottom("mixer")
            .frame(Frame::new().fill(PANEL.gamma_multiply(0.86)).stroke(Stroke::new(1.0, LINE)))
            .exact_size(mixer::HEIGHT)
            .resizable(false)
            .show_separator_line(false)
            .show(ui, |ui| mixer::show(app, ui));
    }
    egui::CentralPanel::no_frame().show(ui, |ui| stage::show(app, ui, false));
    overlay::show(app, &ctx);
    sound::show(app, &ctx);
    effects::editor(app, &ctx);
    toasts(app, &ctx);
    screen2::show(app, &ctx);
    let open = ctx.any_popup_open();
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(POPUP_OPEN), open));
}

const POPUP_OPEN: &str = "popup-open-last-frame";

/// How far popup `key` has been dragged from its usual place (kept for the
/// session, not saved).
pub fn popup_offset(ctx: &egui::Context, key: &str) -> egui::Vec2 {
    ctx.data(|d| d.get_temp::<egui::Vec2>(egui::Id::new(("popup-offset", key)))).unwrap_or_default()
}

/// Make `handle` (a popup's header, already allocated with a drag sense)
/// move popup `key`. Double-click puts it back where it was.
pub fn drag_popup(ui: &egui::Ui, handle: &egui::Response, key: &str) {
    let id = egui::Id::new(("popup-offset", key));
    if handle.double_clicked() {
        ui.ctx().data_mut(|d| d.remove::<egui::Vec2>(id));
    } else if handle.dragged() {
        let delta = handle.drag_delta();
        ui.ctx().data_mut(|d| *d.get_temp_mut_or_default::<egui::Vec2>(id) += delta);
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if handle.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
}

/// Keep a dragged popup's header on screen: `shown` is where it was drawn,
/// `header` how tall its handle is.
pub fn keep_popup_on_screen(ctx: &egui::Context, key: &str, shown: egui::Rect, header: f32) {
    let screen = ctx.content_rect();
    let mut fix = egui::Vec2::ZERO;
    if shown.top() < screen.top() {
        fix.y = screen.top() - shown.top();
    } else if shown.top() + header > screen.bottom() {
        fix.y = screen.bottom() - header - shown.top();
    }
    let margin = 80.0_f32.min(shown.width());
    if shown.right() < screen.left() + margin {
        fix.x = screen.left() + margin - shown.right();
    } else if shown.left() > screen.right() - margin {
        fix.x = screen.right() - margin - shown.left();
    }
    if fix != egui::Vec2::ZERO {
        ctx.data_mut(|d| *d.get_temp_mut_or_default::<egui::Vec2>(egui::Id::new(("popup-offset", key))) += fix);
    }
}

/// Lay `add` out in a box exactly `width` wide. A truncating drop-down cuts
/// its text at the space it is given, which in a row is the rest of the row;
/// boxed, it ends with "…" at its own edge instead of pushing the row wider.
pub fn fixed_width<R>(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let size = egui::vec2(width, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_max_width(width);
        add(ui)
    })
    .inner
}

/// A menu or drop-down is open (or was at the end of the last frame, as
/// egui closes it on Esc before panels see the key). Esc and clicks then
/// belong to the popup, not to the panel under it.
pub fn popup_open(ctx: &egui::Context) -> bool {
    ctx.any_popup_open() || ctx.data(|d| d.get_temp::<bool>(egui::Id::new(POPUP_OPEN))).unwrap_or(false)
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
