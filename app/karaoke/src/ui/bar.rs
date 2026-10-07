//! The bottom bar. One row, no boxes: transport and time on the left, what
//! is up next in the middle, the three live values (key, tempo, volume)
//! and the overlay pages on the right. The song's progress runs along the
//! top edge, where the bar meets the stage.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, pos2, vec2};
use solfege_synth::engine::PlayState;

use crate::app::KaraokeApp;
use crate::icons;
use crate::music::{signed, transpose_key};
use crate::style::{DANGER, DIM, INK, LINE, RAISED, SUNG, TEXT};
use crate::synth::KEY_RANGE;
use crate::ui::clock;
use crate::ui::overlay::Page;

pub const HEIGHT: f32 = 64.0;
/// Height of the seek strip along the top edge.
const SEEK: f32 = 12.0;

pub fn show(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    // Right click on the bar's empty space: the app menu.
    let (full, bg) = ui.allocate_exact_size(ui.available_size(), Sense::click());
    bg.context_menu(|ui| crate::ui::menu::app_menu(app, ui));
    seek(app, ui, Rect::from_min_size(full.min, vec2(full.width(), SEEK)));
    let row = Rect::from_min_max(pos2(full.left() + 20.0, full.top() + SEEK), pos2(full.right() - 20.0, full.bottom() - 2.0));
    let y = row.center().y;

    // Left: play / pause, next, time.
    let playing = app.synth.state() == PlayState::Playing;
    let play = play_button(ui, pos2(row.left() + 17.0, y), playing).on_hover_text(if playing { "พัก (Space)" } else { "เล่น (Space)" });
    if play.clicked() {
        if let Some(n) = &mut app.now {
            n.finished = false;
        }
        app.synth.toggle();
    }
    if ghost(ui, "next", pos2(row.left() + 58.0, y), icons::NEXT, false, !app.queue.is_empty())
        .on_hover_text("เพลงถัดไปในคิว (N)")
        .clicked()
    {
        app.play_next();
    }
    let t = app.scrub.unwrap_or_else(|| app.synth.time());
    let p = ui.painter();
    let now_r = p.text(pos2(row.left() + 86.0, y), Align2::LEFT_CENTER, clock(t), FontId::monospace(13.0), TEXT);
    let time_r = p.text(pos2(now_r.right(), y), Align2::LEFT_CENTER, format!(" / {}", clock(app.synth.duration())), FontId::monospace(13.0), DIM);
    let left_edge = time_r.right() + 24.0;

    // Right, laid out from the edge inwards.
    let mut x = row.right();
    let ctx = ui.ctx().clone();
    // Right to left; overlay pages, plus the mixer panel and full screen.
    let buttons: [(&str, &str, Option<Page>, bool, &str); 8] = [
        if app.fullscreen {
            ("full", icons::EXIT_FULLSCREEN, None, true, "ออกจากเต็มจอ (F / Esc)")
        } else {
            ("full", icons::FULLSCREEN, None, false, "เต็มจอ (F)")
        },
        ("settings", icons::SETTINGS, Some(Page::Settings), false, "ตั้งค่า (Ctrl+,)"),
        ("commands", icons::COMMAND, Some(Page::Commands), false, "คำสั่งทั้งหมด (Ctrl+K)"),
        ("sounds", icons::FILE_MUSIC, None, false, "เสียงและ SoundFont (S)"),
        ("mixer", icons::MIXER, None, app.mixer_open || app.synth.mixer_touched(), "มิกเซอร์ (M)"),
        if app.synth.melody_off() {
            ("melody", icons::MIC_OFF, None, true, "เมโลดี้ร้องนำปิดอยู่ — คลิกเพื่อเปิด (V)")
        } else {
            ("melody", icons::MIC, None, false, "ปิดเมโลดี้ร้องนำ ช่อง 9 (V)")
        },
        ("queue", icons::QUEUE, Some(Page::Queue), false, "คิวเพลง (Q)"),
        ("search", icons::SEARCH, Some(Page::Songs), false, "ค้นหาเพลง (/)"),
    ];
    for (id, icon, page, active, tip) in buttons {
        let c = pos2(x - 16.0, y);
        if ghost(ui, id, c, icon, active, true).on_hover_text(tip).clicked() {
            match (page, id) {
                (Some(page), _) => app.open(page),
                (None, "mixer") => app.mixer_open = !app.mixer_open,
                (None, "sounds") => app.open_sound(),
                (None, "melody") => app.toggle_melody(),
                (None, _) => app.set_fullscreen(&ctx, !app.fullscreen),
            }
        }
        if id == "queue" && !app.queue.is_empty() {
            count_badge(ui.painter(), c + vec2(9.0, -9.0), app.queue.len());
        }
        x -= 34.0;
    }
    x -= 10.0;
    ui.painter().vline(x, (y - 14.0)..=(y + 14.0), egui::Stroke::new(1.0, LINE));
    x -= 22.0;

    volume(app, ui, Rect::from_min_max(pos2(x - 92.0, row.top()), pos2(x, row.bottom())));
    x -= 92.0 + 22.0;
    tempo(app, ui, Rect::from_min_max(pos2(x - 108.0, row.top()), pos2(x, row.bottom())));
    x -= 108.0 + 22.0;
    key(app, ui, Rect::from_min_max(pos2(x - 108.0, row.top()), pos2(x, row.bottom())));
    x -= 108.0 + 16.0;
    if let Some((icon, color, tip)) = status(app) {
        let r = Rect::from_center_size(pos2(x - 10.0, y), vec2(22.0, 22.0));
        let resp = ui.interact(r, ui.id().with("status"), Sense::hover());
        ui.painter().text(r.center(), Align2::CENTER_CENTER, icon, FontId::proportional(15.0), color);
        resp.on_hover_text(tip);
        x -= 30.0;
    }

    up_next(app, ui, Rect::from_min_max(pos2(left_edge, row.top()), pos2(x - 16.0, row.bottom())));
}

fn status(app: &KaraokeApp) -> Option<(&'static str, Color32, String)> {
    if app.synth.loading_soundfont() {
        return Some((icons::LOADER, DIM, "กำลังโหลด SoundFont…".into()));
    }
    if let Some(e) = &app.synth.output_error {
        return Some((icons::ALERT, DANGER, format!("ไม่มีอุปกรณ์เสียง — เนื้อร้องยังเดินตามเพลง\n{e}")));
    }
    if !app.synth.has_font() {
        return Some((icons::ALERT, DANGER, "ยังไม่มี SoundFont — เพิ่มได้ที่แท็บ เสียง (S)".into()));
    }
    None
}

fn seek(app: &mut KaraokeApp, ui: &mut egui::Ui, strip: Rect) {
    let dur = app.synth.duration();
    let resp = ui.interact(strip, ui.id().with("seek"), Sense::click_and_drag());
    let active = dur > 0.0 && (resp.hovered() || resp.dragged());
    let thick = if active { 4.0 } else { 2.0 };
    let track = Rect::from_min_size(strip.min, vec2(strip.width(), thick));
    let p = ui.painter();
    p.rect_filled(track, 0.0, LINE);
    if dur <= 0.0 {
        return;
    }
    let t = app.scrub.unwrap_or_else(|| app.synth.time());
    let x_of = |s: f64| track.left() + track.width() * (s / dur).clamp(0.0, 1.0) as f32;
    let mut fill = track;
    fill.set_right(x_of(t));
    p.rect_filled(fill, 0.0, SUNG);
    if active {
        p.circle_filled(pos2(x_of(t), track.center().y), 6.0, SUNG);
    }
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

fn play_button(ui: &mut egui::Ui, c: Pos2, playing: bool) -> Response {
    let rect = Rect::from_center_size(c, vec2(34.0, 34.0));
    let resp = ui.interact(rect, ui.id().with("play"), Sense::click());
    let p = ui.painter();
    p.circle_filled(c, 17.0, if resp.hovered() { SUNG } else { TEXT });
    let icon = if playing { icons::PAUSE } else { icons::PLAY };
    // The play triangle sits optically left of centre; nudge it.
    let nudge = if playing { 0.0 } else { 1.5 };
    p.text(c + vec2(nudge, 0.0), Align2::CENTER_CENTER, icon, FontId::proportional(16.0), INK);
    resp
}

/// Icon button with no frame until hovered.
fn ghost(ui: &mut egui::Ui, id: &str, c: Pos2, icon: &str, active: bool, enabled: bool) -> Response {
    let rect = Rect::from_center_size(c, vec2(30.0, 30.0));
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let resp = ui.interact(rect, ui.id().with(id), sense);
    let p = ui.painter();
    if enabled && resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(8), RAISED);
    }
    let color = if !enabled {
        DIM.gamma_multiply(0.4)
    } else if active {
        SUNG
    } else if resp.hovered() {
        TEXT
    } else {
        DIM
    };
    p.text(c, Align2::CENTER_CENTER, icon, FontId::proportional(16.0), color);
    resp
}

fn count_badge(p: &egui::Painter, c: Pos2, n: usize) {
    let text = if n > 99 { "99+".to_string() } else { n.to_string() };
    let g = p.layout_no_wrap(text, FontId::proportional(9.5), INK);
    let w = (g.size().x + 7.0).max(14.0);
    let r = Rect::from_center_size(c, vec2(w, 14.0));
    p.rect_filled(r, CornerRadius::same(7), SUNG);
    p.galley(r.center() - g.size() / 2.0, g, INK);
}

/// A small caps label over a value, with − / + at the sides. Returns the
/// step taken (-1, 0 or 1).
fn readout(ui: &mut egui::Ui, id: &str, rect: Rect, label: &str, value: &str, hint: &str) -> i32 {
    let y = rect.center().y;
    let hovered = ui.rect_contains_pointer(rect);
    let p = ui.painter();
    p.text(pos2(rect.center().x, y - 11.0), Align2::CENTER_CENTER, label, FontId::proportional(10.0), DIM);
    p.text(pos2(rect.center().x, y + 7.0), Align2::CENTER_CENTER, value, FontId::proportional(15.0), TEXT);
    let mut step = 0;
    for (d, icon, cx) in [(-1, icons::MINUS, rect.left() + 10.0), (1, icons::PLUS, rect.right() - 10.0)] {
        let r = Rect::from_center_size(pos2(cx, y + 7.0), vec2(20.0, 20.0));
        let resp = ui.interact(r, ui.id().with((id, d)), Sense::click());
        let p = ui.painter();
        if resp.hovered() {
            p.rect_filled(r, CornerRadius::same(6), RAISED);
        }
        let color = if resp.hovered() { TEXT } else if hovered { DIM } else { DIM.gamma_multiply(0.45) };
        p.text(r.center(), Align2::CENTER_CENTER, icon, FontId::proportional(12.0), color);
        if resp.clicked() {
            step = d;
        }
    }
    ui.interact(rect, ui.id().with((id, "hint")), Sense::hover()).on_hover_text(hint);
    step
}

fn key(app: &mut KaraokeApp, ui: &mut egui::Ui, rect: Rect) {
    let k = app.synth.key();
    let song_key = app.now.as_ref().and_then(|n| n.song.meta.key.as_deref());
    let value = match song_key.and_then(|s| transpose_key(s, k)) {
        Some(name) => name,
        None => signed(k),
    };
    let label = if k == 0 { "KEY".to_string() } else { format!("KEY  {}", signed(k)) };
    let d = readout(ui, "key", rect, &label, &value, "คีย์ทีละครึ่งเสียง  [ ]  — กลองไม่เปลี่ยน");
    if d != 0 {
        app.synth.set_key((k + d).clamp(-KEY_RANGE, KEY_RANGE));
    }
}

fn tempo(app: &mut KaraokeApp, ui: &mut egui::Ui, rect: Rect) {
    let s = app.synth.speed();
    let value = app.bpm().map_or("–".to_string(), |b| format!("{b:.0}"));
    let label = if (s - 1.0).abs() < 1e-3 { "BPM".to_string() } else { format!("BPM  {:.0}%", s * 100.0) };
    let d = readout(ui, "tempo", rect, &label, &value, "ความเร็วทีละ 5%  , .");
    // A light beside the value flashes on every quarter note.
    if let Some(now) = &app.now
        && app.synth.state() == solfege_synth::engine::PlayState::Playing
    {
        let phase = now.timeline.tempo.quarters(app.synth.time()).rem_euclid(1.0) as f32;
        let w = ui.painter().layout_no_wrap(value.clone(), FontId::proportional(15.0), TEXT).size().x;
        let light = pos2(rect.center().x + w / 2.0 + 8.0, rect.center().y + 7.0);
        ui.painter().circle_filled(light, 3.0, crate::style::mix(DIM.gamma_multiply(0.4), crate::style::SUNG_HOT, (1.0 - phase).powi(3)));
    }
    if d != 0 {
        app.synth.set_speed(s + d as f64 * 0.05);
    }
}

fn volume(app: &mut KaraokeApp, ui: &mut egui::Ui, rect: Rect) {
    let y = rect.center().y;
    let track = Rect::from_center_size(pos2(rect.center().x, y + 7.0), vec2(rect.width() - 8.0, 3.0));
    let hit = track.expand2(vec2(4.0, 9.0));
    let resp = ui.interact(hit, ui.id().with("volume"), Sense::click_and_drag());
    if let Some(pos) = resp.interact_pointer_pos()
        && (resp.dragged() || resp.clicked())
    {
        app.synth.set_volume(((pos.x - track.left()) / track.width()).clamp(0.0, 1.0));
    }
    let v_now = app.synth.volume();
    let p = ui.painter();
    let label = format!("VOL  {:.0}", v_now * 100.0);
    p.text(pos2(rect.center().x, y - 11.0), Align2::CENTER_CENTER, label, FontId::proportional(10.0), DIM);
    p.rect_filled(track, 1.5, LINE);
    let mut fill = track;
    fill.set_width(track.width() * v_now);
    let hot = resp.hovered() || resp.dragged();
    p.rect_filled(fill, 1.5, if hot { SUNG } else { TEXT });
    if hot {
        p.circle_filled(pos2(fill.right(), track.center().y), 5.0, SUNG);
    }
    resp.on_hover_text("ระดับเสียงดนตรี");
}

/// "Next: …" in the middle; opens the queue.
fn up_next(app: &mut KaraokeApp, ui: &mut egui::Ui, rect: Rect) {
    if rect.width() < 80.0 {
        return;
    }
    let y = rect.center().y;
    let (label, title) = match app.queue.front() {
        Some(h) => ("ถัดไป", h.title.clone()),
        None if app.now.is_none() => ("", "กด / เพื่อค้นหาเพลง".to_string()),
        None => ("", "คิวว่าง".to_string()),
    };
    let p = ui.painter().with_clip_rect(rect);
    let lg = p.layout_no_wrap(label.to_string(), FontId::proportional(12.0), DIM);
    let tg = p.layout_no_wrap(title, FontId::proportional(14.0), if label.is_empty() { DIM } else { TEXT });
    let gap = if label.is_empty() { 0.0 } else { 10.0 };
    let w = lg.size().x + gap + tg.size().x;
    let x0 = (rect.center().x - w / 2.0).max(rect.left());
    let hit = Rect::from_min_max(pos2(x0 - 8.0, y - 14.0), pos2((x0 + w + 8.0).min(rect.right()), y + 14.0));
    let resp = ui.interact(hit, ui.id().with("up-next"), Sense::click());
    if resp.hovered() {
        p.rect_filled(hit, CornerRadius::same(8), RAISED);
    }
    p.galley(pos2(x0, y - lg.size().y / 2.0), lg, DIM);
    p.galley(pos2(x0 + w - tg.size().x, y - tg.size().y / 2.0), tg, TEXT);
    if resp.on_hover_text("คิวเพลง (Q)").clicked() {
        app.open(if app.queue.is_empty() { Page::Songs } else { Page::Queue });
    }
}
