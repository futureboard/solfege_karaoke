//! The command overlay: one panel over the stage for everything that is not
//! singing. Songs and queue are tabs of one panel; commands, settings and
//! about open as popups of their own (files and folders
//! are picked with the system's own dialogs). List pages share one keyboard model:
//! type to filter, Up / Down to move, Enter to act, Shift+Enter for the
//! second action, Tab to switch page, Esc to close.

mod about;
mod commands;
mod settings;

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Frame, Key, Margin, Modifiers, Rect, Sense, Shadow, Stroke, pos2, vec2};

use commands::Cmd;

use crate::app::KaraokeApp;
use crate::icons;
use crate::style::{ACCENT, DIM, INK, LINE, PANEL, RAISED, SUNG, TEXT};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Songs,
    Queue,
    Commands,
    Settings,
    About,
}

/// Songs and queue share one panel with tabs; the other pages are popups
/// of their own.
const TABS: [(Page, &str); 2] = [(Page::Songs, "เพลง"), (Page::Queue, "คิว")];

/// Icon, title and width of a page shown as a popup of its own.
fn popup(page: Page) -> Option<(&'static str, &'static str, f32)> {
    match page {
        Page::Commands => Some((icons::COMMAND, "คำสั่งทั้งหมด", 640.0)),
        Page::Settings => Some((icons::SETTINGS, "ตั้งค่า", 940.0)),
        Page::About => Some((icons::INFO, "เกี่ยวกับ", 660.0)),
        Page::Songs | Page::Queue => None,
    }
}

pub struct Overlay {
    pub page: Page,
    pub query: String,
    cursor: usize,
    /// List scroll offset last frame.
    scroll: f32,
    /// Bring the cursor row into view this frame (it moved by keyboard).
    reveal: bool,
    /// Section shown in the settings popup.
    settings: settings::Section,
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
}

impl Overlay {
    pub fn new(page: Page) -> Self {
        Self { page, query: String::new(), cursor: 0, scroll: 0.0, reveal: true, settings: settings::Section::default() }
    }

    fn goto(&mut self, page: Page) {
        self.page = page;
        self.query.clear();
        self.cursor = 0;
        self.scroll = 0.0;
        self.reveal = true;
    }

    fn is_list(&self) -> bool {
        !matches!(self.page, Page::Settings | Page::About)
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
        Page::Settings | Page::About => Vec::new(),
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
    if !crate::ui::popup_open(ctx) && key(Modifiers::NONE, Key::Escape) {
        return Outcome::Close;
    }
    if popup(ov.page).is_none() && key(Modifiers::SHIFT, Key::Tab) {
        return Outcome::Goto(cycle(ov.page, -1));
    }
    if popup(ov.page).is_none() && key(Modifiers::NONE, Key::Tab) {
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

    let width = (screen.width() - 48.0).min(popup(ov.page).map_or(760.0, |p| p.2));
    let max_list = (screen.height() * 0.58).max(160.0);
    let mut result = Outcome::Stay;
    // Each popup remembers where it was dragged; songs and queue share one.
    let key = match popup(ov.page) {
        Some(_) => format!("{:?}", ov.page),
        None => "songs".to_string(),
    };
    let offset = crate::ui::popup_offset(ctx, &key);
    let shown = egui::Area::new(egui::Id::new("overlay"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_TOP, vec2(0.0, (screen.height() * 0.09).max(16.0)) + offset)
        .show(ctx, |ui| {
            Frame::new()
                .fill(PANEL)
                .stroke(Stroke::new(1.0, LINE))
                .corner_radius(CornerRadius::same(14))
                .shadow(Shadow { offset: [0, 16], blur: 48, spread: 0, color: Color32::from_black_alpha(170) })
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    match popup(ov.page) {
                        Some((icon, title, _)) => {
                            if title_bar(ui, icon, title, &key) {
                                result = Outcome::Close;
                            }
                        }
                        None => {
                            if let Some(page) = tabs(app, ov, ui, &key) {
                                result = Outcome::Goto(page);
                            }
                        }
                    }
                    divider(ui);
                    match ov.page {
                        Page::Settings => settings::show(app, ui, &mut ov.settings, max_list),
                        Page::About => about::show(app, ui, max_list),
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
    crate::ui::keep_popup_on_screen(ctx, &key, shown.response.rect, 48.0);
    if backdrop && matches!(result, Outcome::Stay) {
        return Outcome::Close;
    }
    result
}

/// The next tab (songs / queue); popups stay where they are.
fn cycle(page: Page, d: i32) -> Page {
    let Some(i) = TABS.iter().position(|t| t.0 == page) else { return page };
    let i = i as i32;
    TABS[(i + d).rem_euclid(TABS.len() as i32) as usize].0
}

fn divider(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, LINE);
}

fn tabs(app: &KaraokeApp, ov: &Overlay, ui: &mut egui::Ui, key: &str) -> Option<Page> {
    let mut go = None;
    // The bar is also the handle to drag the panel by.
    let (bar, handle) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click_and_drag());
    crate::ui::drag_popup(ui, &handle, key);
    let p = ui.painter();
    let mut x = bar.left() + 18.0;
    let current = ov.page;
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

/// Header of a popup page: icon and title, Esc and a close button.
/// Returns true when the close button was clicked.
fn title_bar(ui: &mut egui::Ui, icon: &str, title: &str, key: &str) -> bool {
    // The title bar is also the handle to drag the popup by.
    let (bar, handle) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click_and_drag());
    crate::ui::drag_popup(ui, &handle, key);
    let p = ui.painter();
    let y = bar.center().y;
    p.text(pos2(bar.left() + 20.0, y), Align2::LEFT_CENTER, icon, FontId::proportional(17.0), SUNG);
    p.text(pos2(bar.left() + 46.0, y), Align2::LEFT_CENTER, title, FontId::proportional(16.0), TEXT);
    let close = Rect::from_center_size(pos2(bar.right() - 26.0, y), vec2(28.0, 28.0));
    let resp = ui.interact(close, ui.id().with("popup-close"), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(close, CornerRadius::same(7), RAISED);
    }
    ui.painter().text(close.center(), Align2::CENTER_CENTER, icons::REMOVE, FontId::proportional(14.0), if resp.hovered() { TEXT } else { DIM });
    keycap(ui.painter(), pos2(close.left() - 8.0, y), "Esc");
    resp.on_hover_text("ปิด (Esc)").clicked()
}

fn search_field(ov: &mut Overlay, ui: &mut egui::Ui) {
    let hint = match ov.page {
        Page::Songs => "ค้นหาเพลง ชื่อ / ศิลปิน / รหัส",
        Page::Queue => "กรองคิว",
        Page::Commands => "พิมพ์คำสั่ง",
        _ => "",
    };
    let icon = match ov.page {
        Page::Commands => icons::COMMAND,
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
            // Typing always goes to the search field, but not while a
            // button is held: that would cancel dragging the panel.
            if !ui.input(|i| i.pointer.any_down()) {
                r.request_focus();
            }
        });
    });
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
    // Applied after the rows are drawn: an action can change the list.
    let mut picked = None;
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
            row(app, list[k], ui.painter(), rect, selected);
            if resp.clicked() {
                clicked = Some((k, ui.input(|i| i.modifiers.shift)));
            }
            resp.context_menu(|ui| {
                if let Some(a) = row_menu(app, list[k], ui) {
                    picked = Some((list[k], a));
                }
            });
        }
    });
    ov.scroll = out.state.offset.y;
    if let Some((item, action)) = picked {
        return Some(row_action(app, ov, item, action, ctx));
    }
    let (k, alt) = clicked?;
    ov.cursor = k;
    Some(activate(app, ov, list[k], alt, ctx))
}

fn empty_text(app: &KaraokeApp, ov: &Overlay) -> String {
    match ov.page {
        Page::Songs if app.library.scanning() => "กำลังสแกนคลังเพลง…".into(),
        Page::Songs if app.library.db.songs.is_empty() => "ยังไม่มีเพลง — เพิ่มโฟลเดอร์เพลงได้ที่ ตั้งค่า (Ctrl+,)".into(),
        Page::Songs => format!("ไม่พบ \"{}\"", ov.query),
        Page::Queue if app.queue.is_empty() => "คิวว่าง — แท็บ เพลง แล้วกด Enter เพื่อจอง".into(),
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

fn row(app: &KaraokeApp, item: Item, p: &egui::Painter, rect: Rect, selected: bool) {
    let y = rect.center().y;
    let left = rect.left() + 10.0;
    let mut right = rect.right() - 10.0;
    match item {
        Item::Song(i) => {
            let h = app.library.song(i);
            // By file: an NCN song and its .sfkar copy share a code.
            let playing = app.now.as_ref().is_some_and(|n| n.entry.location == h.location);
            badge(p, pos2(left + 18.0, y), h.key.as_deref().unwrap_or("–"), playing);
            if selected {
                right = hint(p, right, y, "ร้องเลย", Some("Shift"), icons::ENTER);
                right = hint(p, right - 14.0, y, "จองคิว", None, icons::ENTER);
            } else if let Some(pos) = app.queue.iter().position(|q| q.location == h.location) {
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
            // With NCN and .sfkar folders both in the library a code can be
            // listed twice: say which copy this is.
            let kinds = &app.library.db.sources;
            if kinds.iter().any(|s| s.kind != kinds[0].kind) {
                let (tag, color) = match h.location {
                    solfege_songdb::Location::Ncn { .. } => ("NCN", ACCENT),
                    solfege_songdb::Location::Sfkar(_) => ("SFKAR", SUNG),
                };
                right = format_tag(p, pos2(right - 10.0, y), tag, color);
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
    }
}

/// A small coloured label right-aligned at `right_center`; returns its left edge.
fn format_tag(p: &egui::Painter, right_center: egui::Pos2, tag: &str, color: Color32) -> f32 {
    let g = p.layout_no_wrap(tag.to_string(), FontId::proportional(10.0), color);
    let r = Rect::from_min_size(right_center - vec2(g.size().x + 12.0, 9.0), vec2(g.size().x + 12.0, 18.0));
    p.rect_filled(r, CornerRadius::same(5), color.gamma_multiply(0.15));
    p.galley(r.center() - g.size() / 2.0, g, color);
    r.left()
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
        Page::Songs => &[("↵", "จองคิว"), ("Shift ↵", "ร้องเลย"), ("Ctrl D", "เพลงโปรด"), ("Tab", "คิว")],
        Page::Queue => &[("↵", "ร้องเลย"), ("Shift ↵", "ขึ้นเป็นเพลงถัดไป"), ("Alt ↑↓", "เลื่อน"), ("Del", "เอาออก")],
        Page::Commands => &[("↵", "ทำคำสั่ง"), ("↑↓", "เลือก"), ("Esc", "ปิด")],
        Page::Settings | Page::About => &[("Esc", "ปิด")],
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
        Item::Cmd(c) => commands::run(app, c, ctx),
    }
}

/// What a row's context menu can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowAction {
    SingNow,
    Reserve,
    PlayNext,
    Favorite,
    CopyCode,
    CopyPath,
    MoveUp,
    MoveDown,
    Remove,
    ClearQueue,
}

fn row_menu(app: &KaraokeApp, item: Item, ui: &mut egui::Ui) -> Option<RowAction> {
    use crate::ui::menu::{heading, item as entry, item_if};
    let mut out = None;
    let mut pick = |hit: bool, a: RowAction| {
        if hit {
            out = Some(a);
        }
    };
    match item {
        Item::Song(i) => {
            let s = app.library.song(i);
            heading(ui, &format!("{}  ·  {}", s.title, s.id));
            pick(entry(ui, icons::MIC, "ร้องเลย", "Shift ↵"), RowAction::SingNow);
            pick(entry(ui, icons::ENQUEUE, "จองคิว", "↵"), RowAction::Reserve);
            pick(entry(ui, icons::PLAY_NEXT, "ร้องเป็นเพลงถัดไป", ""), RowAction::PlayNext);
            let fav = app.library.is_favorite(&s.uid);
            let (icon, label) = if fav { (icons::STAR_OFF, "เอาออกจากเพลงโปรด") } else { (icons::STAR, "เพิ่มในเพลงโปรด") };
            pick(entry(ui, icon, label, "Ctrl D"), RowAction::Favorite);
            ui.separator();
            pick(entry(ui, icons::COPY, "คัดลอกรหัสเพลง", ""), RowAction::CopyCode);
            pick(entry(ui, icons::COPY, "คัดลอกที่อยู่ไฟล์", ""), RowAction::CopyPath);
        }
        Item::Queued(i) => {
            let n = app.queue.len();
            heading(ui, &format!("คิวที่ {}  ·  {}", i + 1, app.queue[i].title));
            pick(entry(ui, icons::MIC, "ร้องเลย", "↵"), RowAction::SingNow);
            pick(item_if(ui, i > 0, icons::PLAY_NEXT, "ขึ้นเป็นเพลงถัดไป", "Shift ↵"), RowAction::PlayNext);
            pick(item_if(ui, i > 0, icons::MOVE_UP, "เลื่อนขึ้น", "Alt ↑"), RowAction::MoveUp);
            pick(item_if(ui, i + 1 < n, icons::MOVE_DOWN, "เลื่อนลง", "Alt ↓"), RowAction::MoveDown);
            pick(entry(ui, icons::REMOVE, "เอาออกจากคิว", "Del"), RowAction::Remove);
            ui.separator();
            pick(entry(ui, icons::CLEAR_QUEUE, "ล้างคิวทั้งหมด", ""), RowAction::ClearQueue);
        }
        Item::Cmd(_) => {
            ui.close();
        }
    }
    out
}

fn row_action(app: &mut KaraokeApp, ov: &mut Overlay, item: Item, action: RowAction, ctx: &egui::Context) -> Outcome {
    match (item, action) {
        (Item::Song(i), RowAction::SingNow) => activate(app, ov, Item::Song(i), true, ctx),
        (Item::Song(i), RowAction::Reserve) => activate(app, ov, Item::Song(i), false, ctx),
        (Item::Song(i), RowAction::PlayNext) => {
            let s = app.library.song(i).clone();
            app.toast(format!("ร้องถัดไป: {}", s.title));
            app.queue.push_front(s);
            Outcome::Stay
        }
        (Item::Song(i), RowAction::Favorite) => {
            let uid = app.library.song(i).uid.clone();
            if let Err(e) = app.library.toggle_favorite(&uid) {
                app.toast_error(e);
            }
            Outcome::Stay
        }
        (Item::Song(i), RowAction::CopyCode) => {
            ctx.copy_text(app.library.song(i).id.clone());
            Outcome::Stay
        }
        (Item::Song(i), RowAction::CopyPath) => {
            let path = match &app.library.song(i).location {
                solfege_songdb::Location::Ncn { midi, .. } => midi.clone(),
                solfege_songdb::Location::Sfkar(p) => p.clone(),
            };
            ctx.copy_text(path.display().to_string());
            Outcome::Stay
        }
        (Item::Queued(i), RowAction::SingNow) => activate(app, ov, Item::Queued(i), false, ctx),
        (Item::Queued(i), RowAction::PlayNext) => activate(app, ov, Item::Queued(i), true, ctx),
        (Item::Queued(i), RowAction::MoveUp) if i > 0 => {
            app.queue.swap(i, i - 1);
            Outcome::Stay
        }
        (Item::Queued(i), RowAction::MoveDown) if i + 1 < app.queue.len() => {
            app.queue.swap(i, i + 1);
            Outcome::Stay
        }
        (Item::Queued(i), RowAction::Remove) => {
            app.queue.remove(i);
            Outcome::Stay
        }
        (Item::Queued(_), RowAction::ClearQueue) => {
            app.queue.clear();
            Outcome::Stay
        }
        _ => Outcome::Stay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_cycles_through_pages() {
        assert_eq!(cycle(Page::Songs, 1), Page::Queue);
        assert_eq!(cycle(Page::Queue, 1), Page::Songs);
        assert_eq!(cycle(Page::Songs, -1), Page::Queue);
        // Commands, settings and about are popups of their own: no tabs.
        for page in [Page::Commands, Page::Settings, Page::About] {
            assert_eq!(cycle(page, 1), page);
            assert!(popup(page).is_some());
        }
        assert!(popup(Page::Songs).is_none() && popup(Page::Queue).is_none());
    }
}
