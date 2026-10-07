//! Song list with search. Click selects, double-click (or ▶) sings now,
//! + (or Enter in the search box) adds to the queue.

use eframe::egui::{self, Align2, CornerRadius, FontId, RichText, Sense, Stroke, TextEdit};

use crate::icons;
use crate::app::KaraokeApp;
use crate::style::{ACCENT, DIM, INK, LINE, RAISED, SUNG, TEXT};

const ROW: f32 = 50.0;

pub fn show(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{}  เพลง", icons::MUSIC)).size(16.0).strong().color(TEXT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let lib = &app.library;
            let text = if lib.scanning() {
                "กำลังสแกน…".to_string()
            } else if lib.query.trim().is_empty() {
                format!("{} เพลง", lib.songs.len())
            } else {
                format!("{} / {}", lib.results.len(), lib.songs.len())
            };
            ui.label(RichText::new(text).size(12.0).color(DIM));
        });
    });
    ui.add_space(4.0);

    let search = ui.add(
        TextEdit::singleline(&mut app.library.query)
            .hint_text(format!("{}  ค้นหา ชื่อเพลง / ศิลปิน / รหัส  ( / )", icons::SEARCH))
            .desired_width(f32::INFINITY)
            .margin(egui::vec2(10.0, 7.0)),
    );
    if app.focus_search {
        search.request_focus();
        app.focus_search = false;
    }
    if search.changed() {
        app.library.search();
        app.selected = app.library.results.first().copied();
    }
    if search.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        if let Some(i) = app.selected.or(app.library.results.first().copied()) {
            let h = app.library.songs[i].clone();
            app.enqueue(h);
        }
        app.focus_search = true;
    }
    if search.has_focus() {
        let (up, down) = ui.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown)));
        move_selection(app, down as i32 - up as i32);
    }
    ui.add_space(8.0);

    if let Some(e) = &app.library.error {
        ui.colored_label(crate::style::DANGER, e);
        return;
    }
    if app.library.songs.is_empty() && !app.library.scanning() {
        ui.label(RichText::new("ยังไม่มีเพลง — เปิดคลังเพลง NCN ได้ที่ ตั้งค่า").color(DIM));
        return;
    }

    let results = app.library.results.clone();
    let mut action: Option<(usize, bool)> = None;
    egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW, results.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for &i in &results[range] {
            if let Some(play) = row(app, ui, i) {
                action = Some((i, play));
            }
        }
    });
    if let Some((i, play)) = action {
        let h = app.library.songs[i].clone();
        if play { app.play_now(h) } else { app.enqueue(h) }
    }
}

fn move_selection(app: &mut KaraokeApp, delta: i32) {
    if delta == 0 || app.library.results.is_empty() {
        return;
    }
    let r = &app.library.results;
    let pos = app.selected.and_then(|s| r.iter().position(|&i| i == s));
    let next = match pos {
        Some(p) => (p as i32 + delta).clamp(0, r.len() as i32 - 1) as usize,
        None => 0,
    };
    app.selected = Some(r[next]);
}

/// One song row. Returns `Some(true)` to sing now, `Some(false)` to queue.
fn row(app: &mut KaraokeApp, ui: &mut egui::Ui, i: usize) -> Option<bool> {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, ROW), Sense::click());
    let selected = app.selected == Some(i);
    let playing = app.now.as_ref().is_some_and(|n| n.header.id == app.library.songs[i].id);
    let hovered = resp.hovered() || ui.rect_contains_pointer(rect);
    let p = ui.painter_at(rect);
    let card = rect.shrink2(egui::vec2(0.0, 2.0));
    if selected || hovered {
        p.rect_filled(card, CornerRadius::same(10), RAISED);
    }
    if selected {
        p.rect_stroke(card, CornerRadius::same(10), Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
    }
    let h = &app.library.songs[i];

    // Key badge.
    let badge = egui::Rect::from_center_size(card.left_center() + egui::vec2(22.0, 0.0), egui::vec2(34.0, 34.0));
    p.rect_filled(badge, CornerRadius::same(9), if playing { SUNG } else { INK });
    let key = h.key.as_deref().unwrap_or("–");
    let key_size = if key.chars().count() > 3 { 10.0 } else { 13.0 };
    p.text(badge.center(), Align2::CENTER_CENTER, key, FontId::proportional(key_size), if playing { INK } else { ACCENT });

    let left = badge.right() + 10.0;
    let buttons_w = if hovered || selected { 66.0 } else { 0.0 };
    let text_clip = egui::Rect::from_min_max(egui::pos2(left, card.top()), egui::pos2(card.right() - 6.0 - buttons_w, card.bottom()));
    let tp = p.with_clip_rect(text_clip);
    tp.text(egui::pos2(left, card.center().y - 9.0), Align2::LEFT_CENTER, &h.title, FontId::proportional(15.0), TEXT);
    let sub = if h.artist.is_empty() { h.id.clone() } else { format!("{}  ·  {}", h.artist, h.id) };
    tp.text(egui::pos2(left, card.center().y + 11.0), Align2::LEFT_CENTER, sub, FontId::proportional(12.0), DIM);

    let mut out = None;
    if resp.clicked() {
        app.selected = Some(i);
    }
    if resp.double_clicked() {
        out = Some(true);
    }
    if hovered || selected {
        let b = egui::vec2(28.0, 28.0);
        let queue_r = egui::Rect::from_center_size(egui::pos2(card.right() - 20.0, card.center().y), b);
        let play_r = egui::Rect::from_center_size(egui::pos2(card.right() - 52.0, card.center().y), b);
        if ui.put(play_r, egui::Button::new(icons::PLAY).corner_radius(CornerRadius::same(14))).on_hover_text("ร้องเลย").clicked() {
            out = Some(true);
        }
        if ui.put(queue_r, egui::Button::new(icons::QUEUE_ADD).corner_radius(CornerRadius::same(14))).on_hover_text("เพิ่มในคิว").clicked() {
            out = Some(false);
        }
    }
    out
}
