//! The second screen (dual display): a window of its own that shows only
//! the lyric stage, for a TV or projector, while the main window keeps the
//! controls. Drag it onto the other display and press F (or double-click)
//! for full screen there; where it was and whether it was full screen are
//! saved, so it comes back on the same display next time.

use eframe::egui::{self, CursorIcon, Key, Modifiers, ViewportBuilder, ViewportClass, ViewportCommand, ViewportId};

use crate::app::KaraokeApp;
use crate::config::SecondScreen;
use crate::ui::{menu, stage};

pub fn viewport_id() -> ViewportId {
    ViewportId::from_hash_of("second-screen")
}

/// Hide the mouse pointer on the full-screen second screen after this many
/// seconds without moving it.
const HIDE_POINTER: f32 = 2.0;

pub fn show(app: &mut KaraokeApp, ctx: &egui::Context) {
    if !app.settings.second_screen.open {
        app.second_window = None;
        return;
    }
    // Built once per opening: egui re-applies a builder that changes, and
    // this one would change with every move of the window.
    let fresh = app.second_window.is_none();
    let builder = app.second_window.get_or_insert_with(|| builder(&app.settings.second_screen)).clone();
    ctx.show_viewport_immediate(viewport_id(), builder, |ui, class| {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().close_requested()) {
            app.settings.second_screen.open = false;
            return;
        }
        let s = &mut app.settings.second_screen;
        if fresh && s.fullscreen && s.monitor.is_none() && class != ViewportClass::EmbeddedWindow {
            // Full screen once the window stands where it was, so the
            // system picks that display.
            set_fullscreen(&ctx, true, None);
        } else if let Some(full) = ctx.input(|i| i.viewport().fullscreen) {
            s.fullscreen = full;
        }
        if !s.fullscreen && class != ViewportClass::EmbeddedWindow {
            let (outer, inner) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect));
            if let Some(r) = outer {
                s.pos = Some([r.min.x, r.min.y]);
            }
            if let Some(r) = inner {
                s.size = [r.width(), r.height()];
            }
        }
        // Its own full screen; every other key works as in the main window.
        let (full, monitor) = (s.fullscreen, s.monitor);
        let pressed = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
        if pressed(Key::F) || pressed(Key::F11) {
            set_fullscreen(&ctx, !full, monitor);
        }
        if pressed(Key::Escape) && full {
            set_fullscreen(&ctx, false, monitor);
        }
        app.shortcuts(&ctx);

        if class == ViewportClass::EmbeddedWindow {
            // This system cannot open a second window: show it inside.
            ui.set_min_size(egui::vec2(480.0, 270.0));
            stage::show(app, ui, true);
        } else {
            egui::CentralPanel::no_frame().show(ui, |ui| stage::show(app, ui, true));
        }
        if full {
            let still = ctx.input(|i| i.pointer.time_since_last_movement());
            if still > HIDE_POINTER && !ctx.any_popup_open() {
                ctx.set_cursor_icon(CursorIcon::None);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f32(HIDE_POINTER - still + 0.05));
            }
        }
    });
}

fn builder(s: &SecondScreen) -> ViewportBuilder {
    let mut b = ViewportBuilder::default()
        .with_title("Solfege Karaoke — จอเนื้อร้อง")
        .with_inner_size(s.size)
        .with_min_inner_size([320.0, 180.0]);
    if let Some(p) = s.pos {
        b = b.with_position(p);
    }
    if let (true, Some(m)) = (s.fullscreen, s.monitor) {
        // Straight to full screen on the chosen display.
        b = b.with_monitor(m);
    }
    b
}

/// Full screen for the second screen, on display `monitor` (or the one it
/// is on). Call from inside its viewport.
pub fn set_fullscreen(ctx: &egui::Context, on: bool, monitor: Option<usize>) {
    ctx.send_viewport_cmd(fullscreen_cmd(on, monitor));
}

/// The command that puts the second screen in or out of full screen.
pub fn fullscreen_cmd(on: bool, monitor: Option<usize>) -> ViewportCommand {
    match (on, monitor) {
        (true, Some(m)) => ViewportCommand::SetMonitor(m),
        _ => ViewportCommand::Fullscreen(on),
    }
}

/// Right click on the second screen: playback, its full screen, close.
pub fn context_menu(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    menu::playback_items(app, ui);
    ui.separator();
    let ctx = ui.ctx().clone();
    let (full, monitor) = (app.settings.second_screen.fullscreen, app.settings.second_screen.monitor);
    let (icon, label) = if full { (crate::icons::EXIT_FULLSCREEN, "ออกจากเต็มจอ") } else { (crate::icons::FULLSCREEN, "เต็มจอ (จอนี้)") };
    if menu::item(ui, icon, label, "F") {
        set_fullscreen(&ctx, !full, monitor);
    }
    if menu::item(ui, crate::icons::SECOND_SCREEN_OFF, "ปิดจอที่สอง", "D") {
        app.settings.second_screen.open = false;
    }
}
