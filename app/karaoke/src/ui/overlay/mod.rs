//! The command overlay: one panel over the stage for everything that is not
//! singing. Pages: songs, queue, commands, mixer and settings (plus a file
//! browser behind settings). List pages share one keyboard model:
//! type to filter, Up / Down to move, Enter to act, Shift+Enter for the
//! second action, Tab to switch page, Esc to close.

mod browse;
mod commands;
mod settings;
mod sounds;

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Frame, Key, Margin, Modifiers, Rect, Sense, Shadow, Stroke, pos2, vec2};

pub use browse::Target;
use browse::{Browse, Entry};
use commands::Cmd;

use crate::app::KaraokeApp;
use crate::icons;
use crate::style::{ACCENT, DIM, INK, LINE, PANEL, RAISED, SUNG, TEXT};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Songs,
    Queue,
    Commands,
    Sounds,
    Settings,
    Browse,
}

const TABS: [(Page, &str); 5] = [
    (Page::Songs, "เพลง"),
    (Page::Queue, "คิว"),
    (Page::Commands, "คำสั่ง"),
    (Page::Sounds, "เสียง"),
    (Page::Settings, "ตั้งค่า"),
];

pub struct Overlay {
    pub page: Page,
    pub query: String,
    cursor: usize,
    /// List scroll offset last frame.
    scroll: f32,
    /// Bring the cursor row into view this frame (it moved by keyboard).
    reveal: bool,
    browse: Option<Browse>,
    /// Page the browser returns to.
    back: Page,
}

/// What an action leaves behind.
pub enum Outcome {
    Stay,
    Close,
    Goto(Page),
}

#[derive(Clone, Copy)]
enum Item {
    Song(usize),
    Queued(usize),
    Cmd(Cmd),
    Entry(Entry),
}

impl Overlay {
    pub fn new(page: Page) -> Self {
        Self { page, query: String::new(), cursor: 0, scroll: 0.0, reveal: true, browse: None, back: Page::Settings }
    }

    fn goto(&mut self, page: Page) {
        self.page = page;
        self.query.clear();
        self.cursor = 0;
        self.scroll = 0.0;
        self.reveal = true;
    }

    fn browse(&mut self, target: Target, start: Option<std::path::PathBuf>) {
        self.browse = Some(Browse::new(target, start));
        self.back = match target {
            Target::Library => Page::Settings,
            Target::SoundFont => Page::Sounds,
        };
        self.goto(Page::Browse);
    }

    fn is_list(&self) -> bool {
        !matches!(self.page, Page::Sounds | Page::Settings)
    }
}

fn items(app: &KaraokeApp, ov: &Overlay) -> Vec<Item> {
    let q = ov.query.to_lowercase();
    match ov.page {
        Page::Songs => app.library.results.iter().map(|&i| Item::Song(i)).collect(),
        Page::Queue => app
            .queue
            .iter()
            .enumerate()
            .filter(|(_, h)| q.is_empty() || format!("{} {} {}", h.id, h.title, h.artist).to_lowercase().contains(&q))
            .map(|(i, _)| Item::Queued(i))
            .collect(),
        Page::Commands => commands::ALL.iter().filter(|c| c.matches(app, &q)).map(|&c| Item::Cmd(c)).collect(),
        Page::Browse => ov.browse.as_ref().map(|b| b.entries(&ov.query).into_iter().map(Item::Entry).collect()).unwrap_or_default(),
        Page::Sounds | Page::Settings => Vec::new(),
    }
}

fn row_height(page: Page) -> f32 {
    match page {
        Page::Songs | Page::Queue => 54.0,
        _ => 42.0,
    }
}

pub fn show(app: &mut KaraokeApp, ctx: &egui::Context) {
    let Some(mut ov) = app.overlay.take() else { return };
    let outcome = panel(app, &mut ov, ctx);
    match outcome {
        Outcome::Close => {}
        Outcome::Stay => app.overlay = Some(ov),
        Outcome::Goto(page) => {
            ov.goto(page);
            app.overlay = Some(ov);
        }
    }
}

fn panel(app: &mut KaraokeApp, ov: &mut Overlay, ctx: &egui::Context) -> Outcome {
    // Songs search drives the library's own filter.
    if ov.page == Page::Songs && app.library.query != ov.query {
        app.library.query = ov.query.clone();
        app.library.search();
        ov.cursor = 0;
        ov.reveal = true;
    }
    let list = items(app, ov);
    ov.cursor = ov.cursor.min(list.len().saturating_sub(1));

    // Keys first, before the text field can claim them.
    let mut outcome = Outcome::Stay;
    let key = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
    // Esc first closes an open drop-down (egui handles that), then the overlay.
    if !ctx.any_popup_open() && key(Modifiers::NONE, Key::Escape) {
        return if ov.page == Page::Browse { Outcome::Goto(ov.back) } else { Outcome::Close };
    }
    if key(Modifiers::SHIFT, Key::Tab) {
        return Outcome::Goto(cycle(ov.page, -1));
    }
    if key(Modifiers::NONE, Key::Tab) {
        return Outcome::Goto(cycle(ov.page, 1));
    }
    if ov.is_list() && !list.is_empty() {
        let n = list.len();
        if ov.page == Page::Queue
            && let Some(Item::Queued(i)) = list.get(ov.cursor).copied()
        {
            if key(Modifiers::ALT, Key::ArrowUp) && i > 0 {
                app.queue.swap(i, i - 1);
                ov.cursor = ov.cursor.saturating_sub(1);
                ov.reveal = true;
            }
            if key(Modifiers::ALT, Key::ArrowDown) && i + 1 < app.queue.len() {
                app.queue.swap(i, i + 1);
                ov.cursor += 1;
                ov.reveal = true;
            }
            if ov.query.is_empty() && key(Modifiers::NONE, Key::Delete) {
                app.queue.remove(i);
                return Outcome::Stay;
            }
        }
        if ov.page == Page::Songs
            && let Some(Item::Song(i)) = list.get(ov.cursor).copied()
            && key(Modifiers::COMMAND, Key::D)
        {
            let uid = app.library.song(i).uid.clone();
            if let Err(e) = app.library.toggle_favorite(&uid) {
                app.toast_error(e);
            }
            // The empty-query order puts favourites first: follow the song.
            if let Some(k) = app.library.results.iter().position(|&j| app.library.song(j).uid == uid) {
                ov.cursor = k;
                ov.reveal = true;
            }
            return Outcome::Stay;
        }
        let page = row_height(ov.page) as usize;
        let jump = (360 / page).max(1);
        let moves = [
            (Key::ArrowDown, 1i64),
            (Key::ArrowUp, -1),
            (Key::PageDown, jump as i64),
            (Key::PageUp, -(jump as i64)),
        ];
        for (k, d) in moves {
            if key(Modifiers::NONE, k) {
                ov.cursor = (ov.cursor as i64 + d).clamp(0, n as i64 - 1) as usize;
                ov.reveal = true;
            }
        }
        if key(Modifiers::SHIFT, Key::Enter) {
            outcome = activate(app, ov, list[ov.cursor], true, ctx);
        } else if key(Modifiers::NONE, Key::Enter) {
            outcome = activate(app, ov, list[ov.cursor], false, ctx);
        }
    } else if ov.page == Page::Browse && key(Modifiers::NONE, Key::Enter) {
        // A typed path with nothing listed: try to go there.
        if let Some(b) = &mut ov.browse {
            b.go(std::path::PathBuf::from(ov.query.trim()));
            ov.query.clear();
        }
    }
    if !matches!(outcome, Outcome::Stay) {
        return outcome;
    }

    let screen = ctx.content_rect();
    // Dim the stage; a click outside the panel closes the overlay.
    let backdrop = egui::Area::new(egui::Id::new("overlay-backdrop"))
        .order(egui::Order::Middle)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha(150));
            resp.clicked()
        })
        .inner;

    let wide = if ov.page == Page::Sounds { 900.0 } else { 760.0 };
    let width = (screen.width() - 48.0).min(wide);
    let max_list = (screen.height() * 0.58).max(160.0);
    let mut result = Outcome::Stay;
    egui::Area::new(egui::Id::new("overlay"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_TOP, vec2(0.0, (screen.height() * 0.09).max(16.0)))
        .show(ctx, |ui| {
            Frame::new()
                .fill(PANEL)
                .stroke(Stroke::new(1.0, LINE))
                .corner_radius(CornerRadius::same(14))
                .shadow(Shadow { offset: [0, 16], blur: 48, spread: 0, color: Color32::from_black_alpha(170) })
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    if let Some(page) = tabs(app, ov, ui) {
                        result = Outcome::Goto(page);
                    }
                    divider(ui);
                    match ov.page {
                        Page::Sounds => {
                            if let Some((target, start)) = sounds::show(app, ui, max_list) {
                                ov.browse(target, start);
                            }
                        }
                        Page::Settings => {
                            if let Some((target, start)) = settings::show(app, ui, max_list) {
                                ov.browse(target, start);
                            }
                        }
                        _ => {
                            search_field(ov, ui);
                            divider(ui);
                            if let Some(o) = list_ui(app, ov, &list, ui, max_list, ctx) {
                                result = o;
                            }
                        }
                    }
                    divider(ui);
                    footer(ov.page, ui);
                });
        });
    if backdrop && matches!(result, Outcome::Stay) {
        return Outcome::Close;
    }
    result
}

fn cycle(page: Page, d: i32) -> Page {
    let page = if page == Page::Browse { Page::Settings } else { page };
    let i = TABS.iter().position(|t| t.0 == page).unwrap_or(0) as i32;
    TABS[(i + d).rem_euclid(TABS.len() as i32) as usize].0
}

fn divider(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, LINE);
}

fn tabs(app: &KaraokeApp, ov: &Overlay, ui: &mut egui::Ui) -> Option<Page> {
    let mut go = None;
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
    let p = ui.painter();
    let mut x = bar.left() + 18.0;
    let current = if ov.page == Page::Browse { ov.back } else { ov.page };
    for (page, name) in TABS {
        let label = match page {
            Page::Queue if !app.queue.is_empty() => format!("{name}  {}", app.queue.len()),
            _ => name.to_string(),
        };
        let on = page == current;
        let g = p.layout_no_wrap(label, FontId::proportional(14.0), if on { TEXT } else { DIM });
        let r = Rect::from_min_size(pos2(x - 8.0, bar.top()), vec2(g.size().x + 16.0, bar.height()));
        let resp = ui.interact(r, ui.id().with(("tab", name)), Sense::click());
        let color = if on || resp.hovered() { TEXT } else { DIM };
        p.galley_with_override_text_color(pos2(x, bar.center().y - g.size().y / 2.0), g.clone(), color);
        if on {
            let line = Rect::from_min_max(pos2(x, bar.bottom() - 2.0), pos2(x + g.size().x, bar.bottom()));
            p.rect_filled(line, 1.0, SUNG);
        }
        if resp.clicked() && !on {
            go = Some(page);
        }
        x += g.size().x + 26.0;
    }
    keycap(p, pos2(bar.right() - 16.0, bar.center().y), "Esc");
    go
}

fn search_field(ov: &mut Overlay, ui: &mut egui::Ui) {
    let hint = match ov.page {
        Page::Songs => "ค้นหาเพลง ชื่อ / ศิลปิน / รหัส",
        Page::Queue => "กรองคิว",
        Page::Commands => "พิมพ์คำสั่ง",
        Page::Browse => "กรองชื่อ หรือพิมพ์ที่อยู่โฟลเดอร์แล้วกด Enter",
        _ => "",
    };
    let icon = match ov.page {
        Page::Commands => icons::COMMAND,
        Page::Browse => icons::FOLDER_OPEN,
        _ => icons::SEARCH,
    };
    Frame::new().inner_margin(Margin::symmetric(18, 12)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.label(egui::RichText::new(icon).size(18.0).color(DIM));
            let r = ui.add(
                egui::TextEdit::singleline(&mut ov.query)
                    .frame(Frame::NONE)
                    .font(FontId::proportional(18.0))
                    .text_color(TEXT)
                    .hint_text(hint)
                    .desired_width(f32::INFINITY),
            );
            r.request_focus();
        });
    });
    if ov.page == Page::Browse
        && let Some(b) = &ov.browse
    {
        Frame::new().inner_margin(Margin { left: 48, right: 18, top: 0, bottom: 10 }).show(ui, |ui| {
            ui.add(egui::Label::new(egui::RichText::new(b.dir_text()).monospace().size(12.0).color(DIM)).truncate());
        });
    }
}

fn list_ui(
    app: &mut KaraokeApp,
    ov: &mut Overlay,
    list: &[Item],
    ui: &mut egui::Ui,
    max_h: f32,
    ctx: &egui::Context,
) -> Option<Outcome> {
    if ov.page == Page::Queue {
        now_line(app, ui);
    }
    if list.is_empty() {
        let text = empty_text(app, ov);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 88.0), Sense::hover());
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(14.0), DIM);
        return None;
    }
    let row_h = row_height(ov.page);
    let view_h = (list.len() as f32 * row_h).min(max_h);
    if ov.reveal {
        let top = ov.cursor as f32 * row_h;
        if top < ov.scroll {
            ov.scroll = top;
        } else if top + row_h > ov.scroll + view_h {
            ov.scroll = top + row_h - view_h;
        }
    }
    let mut area = egui::ScrollArea::vertical().max_height(view_h).auto_shrink([false, true]);
    if ov.reveal {
        area = area.vertical_scroll_offset(ov.scroll);
        ov.reveal = false;
    }
    let moved = ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
    let mut clicked = None;
    let out = area.show_rows(ui, row_h, list.len(), |ui, range| {
        for k in range {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
            if moved && resp.hovered() {
                ov.cursor = k;
            }
            let selected = k == ov.cursor;
            let rect = rect.shrink2(vec2(8.0, 2.0));
            if selected {
                ui.painter().rect_filled(rect, CornerRadius::same(9), RAISED);
            }
            row(app, ov, list[k], ui.painter(), rect, selected);
            if resp.clicked() {
                clicked = Some((k, ui.input(|i| i.modifiers.shift)));
            }
        }
    });
    ov.scroll = out.state.offset.y;
    let (k, alt) = clicked?;
    ov.cursor = k;
    Some(activate(app, ov, list[k], alt, ctx))
}

fn empty_text(app: &KaraokeApp, ov: &Overlay) -> String {
    match ov.page {
        Page::Songs if app.library.scanning() => "กำลังสแกนคลังเพลง…".into(),
        Page::Songs if app.library.db.songs.is_empty() => "ยังไม่มีเพลง — เพิ่มโฟลเดอร์เพลงได้ที่แท็บ ตั้งค่า".into(),
        Page::Songs => format!("ไม่พบ \"{}\"", ov.query),
        Page::Queue if app.queue.is_empty() => "คิวว่าง — แท็บ เพลง แล้วกด Enter เพื่อจอง".into(),
        Page::Browse => "ว่าง".into(),
        _ => "ไม่พบ".into(),
    }
}

/// What is playing, above the queue list.
fn now_line(app: &KaraokeApp, ui: &mut egui::Ui) {
    let Some(now) = &app.now else { return };
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    let p = ui.painter();
    let y = rect.center().y;
    p.text(pos2(rect.left() + 18.0, y), Align2::LEFT_CENTER, icons::MIC, FontId::proportional(15.0), SUNG);
    let title = if now.entry.artist.is_empty() {
        now.entry.title.clone()
    } else {
        format!("{}  ·  {}", now.entry.title, now.entry.artist)
    };
    let right = format!("{} / {}", super::clock(app.synth.time()), super::clock(app.synth.duration()));
    let r = p.text(pos2(rect.right() - 18.0, y), Align2::RIGHT_CENTER, right, FontId::monospace(12.0), DIM);
    let clip = Rect::from_min_max(pos2(rect.left(), rect.top()), pos2(r.left() - 12.0, rect.bottom()));
    p.with_clip_rect(clip).text(pos2(rect.left() + 44.0, y), Align2::LEFT_CENTER, title, FontId::proportional(14.0), TEXT);
    divider(ui);
}

fn row(app: &KaraokeApp, ov: &Overlay, item: Item, p: &egui::Painter, rect: Rect, selected: bool) {
    let y = rect.center().y;
    let left = rect.left() + 10.0;
    let mut right = rect.right() - 10.0;
    match item {
        Item::Song(i) => {
            let h = app.library.song(i);
            let playing = app.now.as_ref().is_some_and(|n| n.entry.id == h.id);
            badge(p, pos2(left + 18.0, y), h.key.as_deref().unwrap_or("–"), playing);
            if selected {
                right = hint(p, right, y, "ร้องเลย", Some("Shift"), icons::ENTER);
                right = hint(p, right - 14.0, y, "จองคิว", None, icons::ENTER);
            } else if let Some(pos) = app.queue.iter().position(|q| q.id == h.id) {
                right = p.text(pos2(right, y), Align2::RIGHT_CENTER, format!("คิว {}", pos + 1), FontId::proportional(12.0), ACCENT).left();
            }
            let stats = app.library.db.stats(&h.uid);
            if stats.favorite {
                right = p.text(pos2(right - 10.0, y), Align2::RIGHT_CENTER, icons::STAR, FontId::proportional(13.0), SUNG).left();
            }
            let mut sub = if h.artist.is_empty() { h.id.clone() } else { format!("{}  ·  {}", h.artist, h.id) };
            if stats.plays > 0 {
                sub.push_str(&format!("  ·  ร้องแล้ว {} ครั้ง", stats.plays));
            }
            two_lines(p, rect, left + 46.0, right - 12.0, &h.title, &sub);
        }
        Item::Queued(i) => {
            let h = &app.queue[i];
            p.text(pos2(left + 18.0, y), Align2::CENTER_CENTER, (i + 1).to_string(), FontId::proportional(18.0), ACCENT);
            if selected {
                right = hint(p, right, y, "ถัดไป", Some("Shift"), icons::ENTER);
                right = hint(p, right - 14.0, y, "ร้องเลย", None, icons::ENTER);
            }
            two_lines(p, rect, left + 46.0, right - 12.0, &h.title, &h.artist);
        }
        Item::Cmd(c) => {
            p.text(pos2(left + 12.0, y), Align2::CENTER_CENTER, c.icon(app), FontId::proportional(16.0), if selected { SUNG } else { DIM });
            for k in c.keys().iter().rev() {
                right = keycap(p, pos2(right, y), k).left() - 4.0;
            }
            let clip = Rect::from_min_max(rect.min, pos2(right - 8.0, rect.max.y));
            p.with_clip_rect(clip).text(pos2(left + 36.0, y), Align2::LEFT_CENTER, c.label(app), FontId::proportional(15.0), TEXT);
        }
        Item::Entry(e) => {
            let Some(b) = &ov.browse else { return };
            let (icon, name, note, color) = b.describe(e);
            p.text(pos2(left + 12.0, y), Align2::CENTER_CENTER, icon, FontId::proportional(16.0), if selected { SUNG } else { DIM });
            if !note.is_empty() {
                right = p.text(pos2(right, y), Align2::RIGHT_CENTER, note, FontId::proportional(12.0), DIM).left();
            }
            let clip = Rect::from_min_max(rect.min, pos2(right - 8.0, rect.max.y));
            p.with_clip_rect(clip).text(pos2(left + 36.0, y), Align2::LEFT_CENTER, name, FontId::proportional(15.0), color);
        }
    }
}

fn two_lines(p: &egui::Painter, rect: Rect, x: f32, right: f32, top: &str, sub: &str) {
    let clip = Rect::from_min_max(pos2(x, rect.top()), pos2(right.max(x), rect.bottom()));
    let p = p.with_clip_rect(clip);
    p.text(pos2(x, rect.center().y - 9.0), Align2::LEFT_CENTER, top, FontId::proportional(15.0), TEXT);
    p.text(pos2(x, rect.center().y + 11.0), Align2::LEFT_CENTER, sub, FontId::proportional(12.0), DIM);
}

/// Key name in a small square: the song's musical key.
fn badge(p: &egui::Painter, c: egui::Pos2, key: &str, playing: bool) {
    let r = Rect::from_center_size(c, vec2(34.0, 34.0));
    p.rect_filled(r, CornerRadius::same(8), if playing { SUNG } else { INK });
    let size = if key.chars().count() > 3 { 10.0 } else { 13.0 };
    p.text(c, Align2::CENTER_CENTER, key, FontId::proportional(size), if playing { INK } else { ACCENT });
}

/// "label [mod] ⏎" right-aligned at `right`; returns the new right edge.
fn hint(p: &egui::Painter, right: f32, y: f32, label: &str, modifier: Option<&str>, key: &str) -> f32 {
    let mut x = keycap(p, pos2(right, y), key).left() - 4.0;
    if let Some(m) = modifier {
        x = keycap(p, pos2(x, y), m).left() - 4.0;
    }
    p.text(pos2(x - 4.0, y), Align2::RIGHT_CENTER, label, FontId::proportional(12.0), DIM).left()
}

pub fn keycap_width(p: &egui::Painter, key: &str) -> f32 {
    (p.layout_no_wrap(key.to_string(), FontId::proportional(11.0), DIM).size().x + 10.0).max(20.0)
}

/// A key drawn as a small outlined cap, right-aligned at `right_center`.
pub fn keycap(p: &egui::Painter, right_center: egui::Pos2, key: &str) -> Rect {
    let g = p.layout_no_wrap(key.to_string(), FontId::proportional(11.0), DIM);
    let w = keycap_width(p, key);
    let r = Rect::from_min_max(pos2(right_center.x - w, right_center.y - 10.0), pos2(right_center.x, right_center.y + 10.0));
    p.rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
    p.galley(r.center() - g.size() / 2.0, g, DIM);
    r
}

fn footer(page: Page, ui: &mut egui::Ui) {
    let hints: &[(&str, &str)] = match page {
        Page::Songs => &[("↵", "จองคิว"), ("Shift ↵", "ร้องเลย"), ("Ctrl D", "เพลงโปรด"), ("Tab", "หน้าถัดไป")],
        Page::Queue => &[("↵", "ร้องเลย"), ("Shift ↵", "ขึ้นเป็นเพลงถัดไป"), ("Alt ↑↓", "เลื่อน"), ("Del", "เอาออก")],
        Page::Commands => &[("↵", "ทำคำสั่ง"), ("Tab", "หน้าถัดไป")],
        Page::Browse => &[("↵", "เปิด / เลือก"), ("Esc", "กลับ")],
        Page::Sounds => &[("S", "เปิดหน้านี้"), ("Tab", "หน้าถัดไป"), ("Esc", "ปิด")],
        Page::Settings => &[("Tab", "หน้าถัดไป"), ("Esc", "ปิด")],
    };
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
    let p = ui.painter();
    let mut x = bar.left() + 18.0;
    for (k, what) in hints {
        let k = k.replace('↵', icons::ENTER).replace('↑', icons::MOVE_UP).replace('↓', icons::MOVE_DOWN);
        let cap = keycap(p, pos2(x + keycap_width(p, &k), bar.center().y), &k);
        let t = p.text(pos2(cap.right() + 6.0, bar.center().y), Align2::LEFT_CENTER, *what, FontId::proportional(12.0), DIM);
        x = t.right() + 18.0;
    }
}

fn activate(app: &mut KaraokeApp, ov: &mut Overlay, item: Item, alt: bool, ctx: &egui::Context) -> Outcome {
    match item {
        Item::Song(i) => {
            let h = app.library.song(i).clone();
            if alt {
                app.play_now(h);
                return Outcome::Close;
            }
            let was_idle = app.now.as_ref().is_none_or(|n| n.finished) && app.queue.is_empty();
            app.enqueue(h);
            if was_idle { Outcome::Close } else { Outcome::Stay }
        }
        Item::Queued(i) => {
            if alt {
                if let Some(h) = app.queue.remove(i) {
                    app.queue.push_front(h);
                }
                ov.cursor = 0;
                ov.reveal = true;
                Outcome::Stay
            } else {
                match app.queue.remove(i) {
                    Some(h) => {
                        app.play_now(h);
                        Outcome::Close
                    }
                    None => Outcome::Stay,
                }
            }
        }
        Item::Cmd(c) => commands::run(app, ov, c, ctx),
        Item::Entry(e) => {
            let Some(b) = &mut ov.browse else { return Outcome::Stay };
            match b.activate(e) {
                Some(path) => {
                    match b.target {
                        Target::Library => app.add_source(path),
                        Target::SoundFont => app.add_soundfont(path),
                    }
                    Outcome::Goto(ov.back)
                }
                None => {
                    ov.query.clear();
                    ov.cursor = 0;
                    ov.reveal = true;
                    Outcome::Stay
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_cycles_through_pages() {
        assert_eq!(cycle(Page::Songs, 1), Page::Queue);
        assert_eq!(cycle(Page::Settings, 1), Page::Songs);
        assert_eq!(cycle(Page::Songs, -1), Page::Settings);
        assert_eq!(cycle(Page::Browse, 1), Page::Songs);
    }
}
