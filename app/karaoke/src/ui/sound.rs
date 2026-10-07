//! Sound settings: a window of its own (S) for the SoundFont rack, which
//! font every channel plays, a sound per GM instrument and the drum kit.
//! A sidebar picks the section; each SoundFont keeps one colour throughout.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Frame, Key, Margin, Modifiers, Rect, RichText, Sense, Shadow, Stroke, pos2, vec2};

use crate::app::KaraokeApp;
use crate::gm;
use crate::icons;
use crate::style::{self, DANGER, DIM, INK, LINE, PANEL, RAISED, SUNG, TEXT, font_color};
use crate::synth::{DRUM_CH, InstrumentSound, KIT, MAX_FONTS, kit_name};
use crate::dialog::Pick;
use crate::ui::overlay::keycap;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Tab {
    Fonts,
    Channels,
    Instruments,
    Drums,
}

pub struct SoundPanel {
    tab: Tab,
    family: usize,
    filter: String,
}

impl SoundPanel {
    pub fn new() -> Self {
        Self { tab: Tab::Fonts, family: 0, filter: String::new() }
    }

    /// Open on a given page.
    pub fn at(tab: Tab) -> Self {
        Self { tab, ..Self::new() }
    }
}

pub fn show(app: &mut KaraokeApp, ctx: &egui::Context) {
    let Some(mut panel) = app.sound.take() else { return };
    let mut open = true;
    if !crate::ui::popup_open(ctx) && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        open = false;
    }

    let screen = ctx.content_rect();
    let clicked_outside = egui::Area::new(egui::Id::new("sound-backdrop"))
        .order(egui::Order::Middle)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha(150));
            resp.clicked()
        })
        .inner;
    let size = vec2((screen.width() - 48.0).min(1060.0), (screen.height() - 96.0).clamp(360.0, 660.0));
    let offset = crate::ui::popup_offset(ctx, "sound");
    let shown = egui::Area::new(egui::Id::new("sound-panel"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, -12.0) + offset)
        .show(ctx, |ui| {
            Frame::new()
                .fill(PANEL)
                .stroke(Stroke::new(1.0, LINE))
                .corner_radius(CornerRadius::same(14))
                .shadow(Shadow { offset: [0, 16], blur: 48, spread: 0, color: Color32::from_black_alpha(170) })
                .show(ui, |ui| {
                    ui.set_width(size.x);
                    ui.set_height(size.y);
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    if header(app, ui) {
                        open = false;
                    }
                    divider(ui);
                    let body_h = ui.available_height();
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(200.0);
                            ui.set_height(body_h);
                            sidebar(app, &mut panel, ui);
                        });
                        let (line, _) = ui.allocate_exact_size(vec2(1.0, body_h), Sense::hover());
                        ui.painter().rect_filled(line, 0.0, LINE);
                        ui.vertical(|ui| {
                            ui.set_height(body_h);
                            egui::ScrollArea::vertical().id_salt(("sound-body", panel.tab)).auto_shrink([false, false]).show(ui, |ui| {
                                Frame::new().inner_margin(Margin::symmetric(22, 18)).show(ui, |ui| {
                                    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                                    // Buttons and pickers sit on raised cards: give them the
                                    // ink fill so they read as controls, not as plain text.
                                    let w = &mut ui.visuals_mut().widgets;
                                    w.inactive.weak_bg_fill = INK;
                                    w.inactive.bg_fill = INK;
                                    w.hovered.weak_bg_fill = style::mix(INK, LINE, 0.8);
                                    match panel.tab {
                                        Tab::Fonts => fonts_tab(app, ui),
                                        Tab::Channels => channels_tab(app, &mut panel, ui),
                                        Tab::Instruments => instruments_tab(app, &mut panel, ui),
                                        Tab::Drums => drums_tab(app, ui),
                                    }
                                });
                            });
                        });
                    });
                });
        });
    crate::ui::keep_popup_on_screen(ctx, "sound", shown.response.rect, 58.0);
    // Meters and "now playing" names follow the music.
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
    if clicked_outside && !crate::ui::popup_open(ctx) {
        open = false;
    }
    if open {
        app.sound = Some(panel);
    }
}

fn divider(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, LINE);
}

/// Title row; returns true when the close button was pressed.
fn header(app: &KaraokeApp, ui: &mut egui::Ui) -> bool {
    // The header is also the handle to drag the window by.
    let (bar, handle) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::click_and_drag());
    crate::ui::drag_popup(ui, &handle, "sound");
    let p = ui.painter();
    p.text(pos2(bar.left() + 22.0, bar.center().y - 8.0), Align2::LEFT_CENTER, format!("{}  เสียงและ SoundFont", icons::FILE_MUSIC), FontId::proportional(17.0), TEXT);
    let fonts = app.synth.fonts().len();
    let overrides = app.synth.instruments().len();
    let lock = if app.synth.drum_lock().is_some() { "กลองล็อกอยู่" } else { "กลองตามเพลง" };
    let summary = format!("{fonts} SoundFont  ·  เปลี่ยนเสียง {overrides} เครื่องดนตรี  ·  {lock}");
    p.text(pos2(bar.left() + 22.0, bar.center().y + 12.0), Align2::LEFT_CENTER, summary, FontId::proportional(12.0), DIM);
    let close = Rect::from_center_size(pos2(bar.right() - 30.0, bar.center().y), vec2(30.0, 30.0));
    let resp = ui.interact(close, ui.id().with("sound-close"), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(close, CornerRadius::same(8), RAISED);
    }
    ui.painter().text(close.center(), Align2::CENTER_CENTER, icons::REMOVE, FontId::proportional(15.0), if resp.hovered() { TEXT } else { DIM });
    keycap(ui.painter(), pos2(close.left() - 8.0, bar.center().y), "Esc");
    resp.clicked()
}

fn sidebar(app: &KaraokeApp, panel: &mut SoundPanel, ui: &mut egui::Ui) {
    ui.add_space(12.0);
    let used = app.synth.channels_used();
    let items = [
        (Tab::Fonts, icons::FILE_MUSIC, "SoundFont", app.synth.fonts().len().to_string()),
        (Tab::Channels, icons::MIXER, "แชนแนล", if used == 0 { "16".into() } else { used.count_ones().to_string() }),
        (Tab::Instruments, icons::GUITAR, "เครื่องดนตรี", app.synth.instruments().len().to_string()),
        (Tab::Drums, icons::DRUM, "ชุดกลอง", if app.synth.drum_lock().is_some() { icons::LOCK.into() } else { String::new() }),
    ];
    for (tab, icon, label, badge) in items {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::click());
        let r = rect.shrink2(vec2(10.0, 3.0));
        let on = panel.tab == tab;
        let p = ui.painter();
        if on || resp.hovered() {
            p.rect_filled(r, CornerRadius::same(9), RAISED);
        }
        if on {
            p.rect_filled(Rect::from_min_size(r.left_top() + vec2(0.0, 9.0), vec2(3.0, r.height() - 18.0)), 2.0, SUNG);
        }
        let c = if on { TEXT } else { DIM };
        p.text(pos2(r.left() + 16.0, r.center().y), Align2::LEFT_CENTER, icon, FontId::proportional(15.0), if on { SUNG } else { DIM });
        p.text(pos2(r.left() + 40.0, r.center().y), Align2::LEFT_CENTER, label, FontId::proportional(14.0), c);
        if !badge.is_empty() {
            p.text(pos2(r.right() - 12.0, r.center().y), Align2::RIGHT_CENTER, badge, FontId::proportional(12.0), DIM);
        }
        if resp.clicked() {
            panel.tab = tab;
        }
    }
}

fn title(ui: &mut egui::Ui, text: &str, hint: &str) {
    ui.label(RichText::new(text).size(18.0).strong().color(TEXT));
    if !hint.is_empty() {
        ui.label(RichText::new(hint).size(12.5).color(DIM));
    }
    ui.add_space(6.0);
}

/// A small square with the font's number in its colour.
fn font_badge(ui: &mut egui::Ui, index: usize, size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let c = font_color(index);
    ui.painter().rect_filled(rect, CornerRadius::same((size / 4.0) as u8), c.gamma_multiply(0.18));
    ui.painter().rect_stroke(rect, CornerRadius::same((size / 4.0) as u8), Stroke::new(1.0, c.gamma_multiply(0.7)), egui::StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, (index + 1).to_string(), FontId::proportional(size * 0.5), c);
}

/// SoundFont drop-down: each font with its number, colour and file name,
/// plus an optional "none" entry. Fonts still loading or broken are listed
/// but cannot be picked. Returns the new choice when it changed.
fn font_picker(
    ui: &mut egui::Ui,
    app: &KaraokeApp,
    id: impl std::hash::Hash + std::fmt::Debug,
    current: Option<usize>,
    none: Option<&str>,
    width: f32,
) -> Option<Option<usize>> {
    let label = |i: usize, name: &str| RichText::new(format!("{}  {}  ·  {}", icons::FILE_MUSIC, i + 1, name)).color(font_color(i));
    let shown = match current {
        Some(i) => app.synth.fonts().get(i).map_or_else(|| RichText::new("—"), |f| label(i, &f.name())),
        None => RichText::new(none.unwrap_or("—")).color(DIM),
    };
    let full = current.and_then(|i| app.synth.fonts().get(i)).map(|f| f.name());
    let mut pick = current;
    let combo = crate::ui::fixed_width(ui, width, |ui| egui::ComboBox::from_id_salt(id).truncate().selected_text(shown).width(width).height(320.0).show_ui(ui, |ui| {
        // Long file names end in "…" (full name on hover), so the list
        // stays a sensible width.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        ui.set_max_width(width.max(380.0));
        if let Some(none) = none {
            ui.selectable_value(&mut pick, None, RichText::new(none).color(DIM));
        }
        for (i, f) in app.synth.fonts().iter().enumerate() {
            let state = if f.loading() { Some("กำลังโหลด…") } else if f.error.is_some() { Some("โหลดไม่ได้") } else { None };
            let text = match state {
                // The state goes first so truncation only cuts the name.
                Some(state) => RichText::new(format!("{}  {}  ·  ({state})  {}", icons::FILE_MUSIC, i + 1, f.name())).color(DIM),
                None => label(i, &f.name()),
            };
            ui.add_enabled_ui(f.inst.is_some(), |ui| {
                ui.selectable_value(&mut pick, Some(i), text).on_hover_text(f.path.display().to_string());
            });
        }
    }));
    if let Some(full) = full {
        combo.response.on_hover_text(full);
    }
    (pick != current).then_some(pick)
}

// ------------------------------------------------------------------ fonts

fn fonts_tab(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    title(ui, "SoundFont", "ไฟล์ .sf2 หรือ SFZ ที่ใช้เล่นดนตรี ไฟล์แรกเล่นทุกแชนแนลจนกว่าจะเลือกให้แชนแนลหรือเครื่องดนตรีใช้ไฟล์อื่น");
    if app.synth.fonts().is_empty() {
        Frame::new().fill(INK).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(16)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("ยังไม่มี SoundFont — เพิ่มไฟล์ General MIDI (.sf2) เพื่อให้มีเสียงดนตรี").color(DANGER));
        });
    }
    let routing = app.synth.routing();
    let instruments: Vec<usize> = app.synth.instruments().values().map(|s| s.font).collect();
    let mut remove = None;
    let mut all = None;
    let mut copy = None;
    for (i, f) in app.synth.fonts().iter().enumerate() {
        let card = ui.scope_builder(egui::UiBuilder::new().sense(Sense::click()), |ui| {
            Frame::new()
                .fill(RAISED)
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        font_badge(ui, i, 34.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            // Leave room for the buttons on the right.
                        ui.set_max_width((ui.available_width() - 190.0).max(120.0));
                        ui.add(egui::Label::new(RichText::new(f.name()).size(15.0).strong().color(TEXT)).truncate())
                            .on_hover_text(f.name());
                            let meta = if f.loading() {
                                "กำลังโหลด…".to_string()
                            } else if f.error.is_some() {
                                "โหลดไม่ได้".to_string()
                            } else {
                                let presets = f.inst.as_ref().map_or(0, |inst| inst.presets.len());
                                let channels = routing.iter().filter(|&&r| r == i).count();
                                let inst = instruments.iter().filter(|&&x| x == i).count();
                                format!("{presets} เสียง  ·  {channels} แชนแนล  ·  {inst} เครื่องดนตรี")
                            };
                            let r = ui.label(RichText::new(meta).size(12.0).color(if f.error.is_some() { DANGER } else { DIM }));
                            if let Some(e) = &f.error {
                                r.on_hover_text(e);
                            }
                            ui.add(egui::Label::new(RichText::new(f.path.display().to_string()).monospace().size(11.0).color(DIM)).truncate());
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            if ui.button(icons::REMOVE).on_hover_text("เอาออกจาก rack (ไม่ลบไฟล์)").clicked() {
                                remove = Some(i);
                            }
                            if f.inst.is_some() && routing.iter().any(|&r| r != i) && ui.button("ใช้กับทุกแชนแนล").clicked() {
                                all = Some(i);
                            }
                            if f.loading() {
                                ui.spinner();
                            }
                        });
                    });
                });
        });
        card.response.context_menu(|ui| {
            use crate::ui::menu::{heading, item, item_if};
            heading(ui, &format!("SoundFont {}  ·  {}", i + 1, f.name()));
            if item_if(ui, f.inst.is_some() && routing.iter().any(|&r| r != i), icons::MIXER, "ใช้กับทุกแชนแนล", "") {
                all = Some(i);
            }
            if item(ui, icons::COPY, "คัดลอกที่อยู่ไฟล์", "") {
                copy = Some(f.path.display().to_string());
            }
            ui.separator();
            if item(ui, icons::REMOVE, "เอาออกจาก rack", "") {
                remove = Some(i);
            }
        });
    }
    if let Some(text) = copy {
        ui.ctx().copy_text(text);
    }
    if let Some(i) = all {
        app.synth.route_all(i);
    }
    if let Some(i) = remove {
        app.synth.remove_font(i);
    }
    ui.add_space(4.0);
    let full = app.synth.fonts().len() >= MAX_FONTS;
    let picking = app.dialogs.busy();
    let label = if picking { format!("{}   กำลังเลือกไฟล์ในหน้าต่างของระบบ…", icons::FOLDER_OPEN) } else { format!("{}   เพิ่ม SoundFont / SFZ…", icons::FOLDER_PLUS) };
    let add = ui.add_enabled(
        !full && !picking,
        egui::Button::new(RichText::new(label).size(14.0))
            .min_size(vec2(ui.available_width(), 44.0))
            .corner_radius(CornerRadius::same(12)),
    );
    if add.on_hover_text("เปิดหน้าต่างเลือกไฟล์ของระบบ เลือกได้หลายไฟล์").clicked() {
        app.dialogs.ask(Pick::SoundFonts, app.synth.fonts().last().map(|f| f.path.clone()));
    }
    if full {
        ui.label(RichText::new(format!("ใส่ได้สูงสุด {MAX_FONTS} ไฟล์")).size(12.0).color(DIM));
    }
}

// --------------------------------------------------------------- channels

/// Presets of a font as (index, label), drum kits or melodic ones.
fn presets(font: Option<&std::sync::Arc<solfege_synth::instrument::Instrument>>, drums: bool) -> Vec<(usize, String)> {
    let Some(f) = font else { return Vec::new() };
    let has_kits = f.presets.iter().any(|p| p.bank == 128);
    let mut out: Vec<(usize, String)> = f
        .presets
        .iter()
        .enumerate()
        .filter(|(_, p)| !has_kits || drums == (p.bank == 128))
        .map(|(i, p)| (i, format!("{:03}:{:03}  {}", p.bank, p.program, p.name)))
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

fn channels_tab(app: &mut KaraokeApp, panel: &mut SoundPanel, ui: &mut egui::Ui) {
    title(ui, "แชนแนล", "เลือก SoundFont ให้แต่ละแชนแนล และปักเสียงแทนที่เพลงเลือก (ปักไว้เฉพาะเพลงนี้)");
    let used = app.synth.channels_used();
    for ch in 0..16 {
        let active = used == 0 || used & (1 << ch) != 0;
        let font = app.synth.routing()[ch];
        let row = ui.scope_builder(egui::UiBuilder::new().sense(Sense::click()), |ui| {
            Frame::new()
                .fill(if active { RAISED } else { style::mix(INK, RAISED, 0.4) })
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        // Channel number in its font's colour.
                        let (rect, _) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
                        let c = font_color(app.synth.sounding_font(ch).unwrap_or(font));
                        ui.painter().rect_filled(rect, CornerRadius::same(9), c.gamma_multiply(if active { 0.2 } else { 0.08 }));
                        let label = if ch == DRUM_CH { icons::DRUM.to_string() } else { (ch + 1).to_string() };
                        ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(14.0), if active { c } else { DIM });

                        ui.vertical(|ui| {
                            ui.set_width(210.0);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            let name = app.synth.channel_sound(ch).unwrap_or(if active { "—" } else { "ไม่ได้ใช้ในเพลงนี้" }).to_string();
                            let pinned = app.synth.pin(ch).is_some();
                            let head = if ch == DRUM_CH { "แชนแนล 10 · กลอง".to_string() } else { format!("แชนแนล {}", ch + 1) };
                            ui.label(RichText::new(head).size(11.0).color(DIM));
                            ui.add(egui::Label::new(RichText::new(name).color(if pinned { SUNG } else if active { TEXT } else { DIM })).truncate());
                        });

                        if let Some(Some(f)) = font_picker(ui, app, ("ch-font", ch), Some(font), None, 225.0) {
                            app.synth.set_route(ch, f);
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ch == DRUM_CH {
                                let text = if app.synth.drum_lock().is_some() { format!("{}  ล็อกอยู่", icons::LOCK) } else { "ชุดกลอง…".to_string() };
                                if ui.button(text).clicked() {
                                    panel.tab = Tab::Drums;
                                }
                                return;
                            }
                            let list = presets(app.synth.channel_font(ch), false);
                            let mut pin = app.synth.pin(ch);
                            let shown = pin.and_then(|p| list.iter().find(|(i, _)| *i == p)).map_or("ตามเพลง".to_string(), |(_, n)| n.clone());
                            crate::ui::fixed_width(ui, 230.0, |ui| egui::ComboBox::from_id_salt(("pin", ch)).truncate().selected_text(shown).width(230.0).height(320.0).show_ui(ui, |ui| {
                                ui.selectable_value(&mut pin, None, "ตามเพลง");
                                for (i, name) in &list {
                                    ui.selectable_value(&mut pin, Some(*i), name);
                                }
                            }));
                            if pin != app.synth.pin(ch) {
                                app.synth.set_pin(ch, pin);
                            }
                            ui.label(RichText::new("ปักเสียง").size(11.0).color(DIM));
                        });
                    });
                });
        });
        row.response.context_menu(|ui| channel_menu(app, panel, ui, ch));
    }
}

/// Right click on a channel row: its SoundFont, its pinned sound.
fn channel_menu(app: &mut KaraokeApp, panel: &mut SoundPanel, ui: &mut egui::Ui, ch: usize) {
    use crate::ui::menu::{heading, item, item_if, toggle};
    heading(ui, &if ch == DRUM_CH { "แชนแนล 10 · กลอง".to_string() } else { format!("แชนแนล {}", ch + 1) });
    let routed = app.synth.routing()[ch];
    let names: Vec<String> = app.synth.fonts().iter().map(|f| f.name().to_string()).collect();
    for (i, name) in names.iter().enumerate() {
        if toggle(ui, routed == i, icons::FILE_MUSIC, &format!("SoundFont {}  ·  {name}", i + 1), "") {
            app.synth.set_route(ch, i);
        }
    }
    ui.separator();
    if ch == DRUM_CH {
        if item(ui, icons::DRUM, "ชุดกลอง…", "") {
            panel.tab = Tab::Drums;
        }
    } else if item_if(ui, app.synth.pin(ch).is_some(), icons::UNDO, "เลิกปักเสียง (ตามเพลง)", "") {
        app.synth.set_pin(ch, None);
    }
    if item_if(ui, names.len() > 1, icons::MIXER, &format!("ใช้ SoundFont {} กับทุกแชนแนล", routed + 1), "") {
        app.synth.route_all(routed);
    }
}

// ------------------------------------------------------------ instruments

fn instruments_tab(app: &mut KaraokeApp, panel: &mut SoundPanel, ui: &mut egui::Ui) {
    title(ui, "เครื่องดนตรี", "เลือกเสียงให้เครื่องดนตรี GM แต่ละชิ้นจาก SoundFont ไหนก็ได้ ใช้กับทุกแชนแนลที่เล่นชิ้นนั้น (ยกเว้นแชนแนลที่ปักเสียง)");
    if !app.synth.has_font() {
        ui.label(RichText::new("เพิ่ม SoundFont ก่อน").color(DIM));
        return;
    }
    ui.add(
        egui::TextEdit::singleline(&mut panel.filter)
            .hint_text(format!("{}  ค้นหาเครื่องดนตรี เช่น guitar, sax, 28", icons::SEARCH))
            .desired_width(f32::INFINITY)
            .margin(vec2(10.0, 7.0)),
    );
    let chosen = app.synth.instruments().clone();
    let q = panel.filter.trim().to_lowercase();
    let programs: Vec<u8> = if q.is_empty() {
        // Family chips, then that family's eight instruments.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for (f, name) in gm::FAMILIES.iter().enumerate() {
                let set = (f * 8..f * 8 + 8).filter(|&p| chosen.contains_key(&(p as u8))).count();
                let label = if set > 0 { format!("{name}  {set}") } else { (*name).to_string() };
                let on = panel.family == f;
                let text = RichText::new(label).size(12.5).color(if on { INK } else if set > 0 { SUNG } else { TEXT });
                let b = egui::Button::new(text).corner_radius(CornerRadius::same(14)).fill(if on { SUNG } else { INK });
                if ui.add(b).clicked() {
                    panel.family = f;
                }
            }
        });
        (panel.family as u8 * 8..panel.family as u8 * 8 + 8).collect()
    } else {
        (0..128u8)
            .filter(|&p| gm::INSTRUMENTS[p as usize].to_lowercase().contains(&q) || (p as usize + 1).to_string() == q)
            .collect()
    };
    ui.add_space(4.0);
    if programs.is_empty() {
        ui.label(RichText::new("ไม่พบเครื่องดนตรี").color(DIM));
    }
    let mut change: Option<(u8, Option<InstrumentSound>)> = None;
    for program in programs {
        let current = chosen.get(&program).copied();
        Frame::new()
            .fill(if current.is_some() { RAISED } else { style::mix(INK, RAISED, 0.6) })
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    ui.vertical(|ui| {
                        ui.set_width(230.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(RichText::new(format!("{:03}  ·  {}", program + 1, gm::FAMILIES[program as usize / 8])).size(11.0).color(DIM));
                        ui.label(RichText::new(gm::INSTRUMENTS[program as usize]).color(if current.is_some() { SUNG } else { TEXT }));
                    });
                    if let Some(f) = font_picker(ui, app, ("gm-font", program), current.map(|s| s.font), Some("ตามแชนแนล"), 250.0) {
                        change = Some((program, f.and_then(|f| default_sound(app, f, program))));
                    }
                    if let Some(sound) = current {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let font = app.synth.fonts().get(sound.font).and_then(|f| f.inst.clone());
                            let list = presets(font.as_ref(), false);
                            let mut pick = font.as_ref().and_then(|f| f.find_preset(sound.bank, sound.program));
                            let shown = pick.and_then(|p| list.iter().find(|(i, _)| *i == p)).map_or("—".to_string(), |(_, n)| n.clone());
                            crate::ui::fixed_width(ui, 230.0, |ui| egui::ComboBox::from_id_salt(("gm-preset", program)).truncate().selected_text(shown).width(230.0).height(320.0).show_ui(ui, |ui| {
                                for (i, name) in &list {
                                    ui.selectable_value(&mut pick, Some(*i), name);
                                }
                            }));
                            if let (Some(i), Some(f)) = (pick, &font)
                                && Some(i) != f.find_preset(sound.bank, sound.program)
                            {
                                let p = &f.presets[i];
                                change = Some((program, Some(InstrumentSound { font: sound.font, bank: p.bank, program: p.program })));
                            }
                        });
                    }
                });
            });
    }
    if let Some((program, sound)) = change {
        app.synth.set_instrument(program, sound);
    }
}

/// The sound a font offers for a GM program: the same program in bank 0,
/// else its first melodic preset.
fn default_sound(app: &KaraokeApp, font: usize, program: u8) -> Option<InstrumentSound> {
    let inst = app.synth.fonts().get(font)?.inst.as_ref()?;
    let p = inst
        .find_preset(0, program)
        .map(|i| &inst.presets[i])
        .or_else(|| inst.presets.iter().filter(|p| p.bank != 128).min_by_key(|p| (p.bank, p.program)))?;
    Some(InstrumentSound { font, bank: p.bank, program: p.program })
}

// ------------------------------------------------------------------ drums

/// Kick, snare, hats, toms, cymbals and percussion: each can play from a
/// kit of its own, from any font, instead of channel 10's kit.
fn piece_rows(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    ui.label(RichText::new("แยกเสียงกลองแต่ละชิ้น").size(15.0).strong().color(TEXT));
    ui.label(RichText::new("ให้กระเดื่อง สแนร์ ไฮแฮต ฯลฯ ใช้ชุดกลองจาก SoundFont อื่นได้ทีละชิ้น ชิ้นที่ไม่ได้แยกจะใช้ชุดหลัก").size(12.0).color(DIM));
    for g in 0..KIT {
        let piece = app.synth.pieces()[g];
        let row = ui.scope_builder(egui::UiBuilder::new().sense(Sense::click()), |ui| {
            Frame::new()
                .fill(if piece.is_some() { RAISED } else { style::mix(INK, RAISED, 0.5) })
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::symmetric(12, 7))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        let (rect, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
                        let c = piece.map_or(DIM, |s| font_color(s.font));
                        ui.painter().rect_filled(rect, CornerRadius::same(8), c.gamma_multiply(0.18));
                        ui.painter().text(rect.center(), Align2::CENTER_CENTER, icons::DRUM, FontId::proportional(14.0), c);
                        ui.vertical(|ui| {
                            ui.set_width(170.0);
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.label(RichText::new(kit_name(g)).color(TEXT));
                            let sound = app.synth.piece_sound(g).unwrap_or_else(|| "—".into());
                            let note = if piece.is_some() { sound } else { format!("ตามชุดหลัก · {sound}") };
                            ui.add(egui::Label::new(RichText::new(note).size(11.0).color(if piece.is_some() { SUNG } else { DIM })).truncate());
                        });
                        if let Some(choice) = font_picker(ui, app, ("piece-font", g), piece.map(|s| s.font), Some("ตามชุดหลัก"), 250.0) {
                            let sound = choice.and_then(|font| default_kit(app, font));
                            app.synth.set_piece(g, sound);
                        }
                        let Some(s) = piece else { return };
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let font = app.synth.fonts().get(s.font).and_then(|f| f.inst.clone());
                            let list = presets(font.as_ref(), true);
                            let current = font.as_ref().and_then(|f| f.find_preset(s.bank, s.program));
                            let shown = current.and_then(|p| list.iter().find(|(i, _)| *i == p)).map_or("—".to_string(), |(_, n)| n.clone());
                            let mut pick = current;
                            crate::ui::fixed_width(ui, 220.0, |ui| egui::ComboBox::from_id_salt(("piece-kit", g)).truncate().selected_text(shown).width(220.0).height(320.0).show_ui(ui, |ui| {
                                for (i, name) in &list {
                                    ui.selectable_value(&mut pick, Some(*i), name);
                                }
                            }));
                            if pick != current
                                && let Some(p) = pick.and_then(|p| font.as_ref()?.presets.get(p))
                            {
                                app.synth.set_piece(g, Some(InstrumentSound { font: s.font, bank: p.bank, program: p.program }));
                            }
                        });
                    });
                });
        });
        row.response.context_menu(|ui| {
            use crate::ui::menu::{heading, item_if};
            heading(ui, kit_name(g));
            if item_if(ui, piece.is_some(), icons::UNDO, "กลับไปใช้ชุดหลัก", "") {
                app.synth.set_piece(g, None);
            }
            if item_if(ui, app.synth.pieces().iter().any(Option::is_some), icons::RESTART, "ทุกชิ้นใช้ชุดหลัก", "") {
                for k in 0..KIT {
                    app.synth.set_piece(k, None);
                }
            }
        });
    }
}

/// A font's standard kit (128:0, else its first kit, else its first preset).
fn default_kit(app: &KaraokeApp, font: usize) -> Option<InstrumentSound> {
    let inst = app.synth.fonts().get(font)?.inst.as_ref()?;
    let i = inst.find_preset(128, 0).or_else(|| inst.presets.iter().position(|p| p.bank == 128)).unwrap_or(0);
    let p = inst.presets.get(i)?;
    Some(InstrumentSound { font, bank: p.bank, program: p.program })
}

fn drums_tab(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    title(ui, "ชุดกลอง (แชนแนล 10)", "เลือก SoundFont ของกลอง และล็อกชุดกลองไว้ทุกเพลง — เพลงจะเปลี่ยนชุดกลองเองไม่ได้");
    ui.horizontal(|ui| {
        ui.label(RichText::new("SoundFont").color(DIM));
        let font = app.synth.routing()[DRUM_CH];
        if let Some(Some(f)) = font_picker(ui, app, "drum-font", Some(font), None, 250.0) {
            app.synth.set_route(DRUM_CH, f);
        }
    });
    let now = app.synth.channel_sound(DRUM_CH).unwrap_or("—").to_string();
    ui.label(RichText::new(format!("กำลังเล่น: {now}")).color(TEXT));
    ui.add_space(10.0);
    piece_rows(app, ui);
    ui.add_space(10.0);
    ui.label(RichText::new("ล็อกชุดกลองหลัก").size(15.0).strong().color(TEXT));
    ui.label(RichText::new("ชุดที่ชิ้นกลองซึ่งไม่ได้แยกเสียงไว้ใช้เล่น").size(12.0).color(DIM));
    let kits = presets(app.synth.channel_font(DRUM_CH), true);
    let current = app.synth.pin(DRUM_CH);
    let mut pick: Option<Option<usize>> = None;
    let mut row = |ui: &mut egui::Ui, value: Option<usize>, name: &str, note: &str| {
        let on = current == value;
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(9), if on { SUNG.gamma_multiply(0.16) } else if resp.hovered() { RAISED } else { INK });
        if on {
            p.rect_stroke(rect, CornerRadius::same(9), Stroke::new(1.0, SUNG.gamma_multiply(0.7)), egui::StrokeKind::Inside);
        }
        let mark = if on { if value.is_some() { icons::LOCK } else { icons::CHECK } } else { "" };
        p.text(pos2(rect.left() + 18.0, rect.center().y), Align2::CENTER_CENTER, mark, FontId::proportional(14.0), SUNG);
        p.text(pos2(rect.left() + 40.0, rect.center().y), Align2::LEFT_CENTER, name, FontId::proportional(14.0), if on { SUNG } else { TEXT });
        p.text(pos2(rect.right() - 14.0, rect.center().y), Align2::RIGHT_CENTER, note, FontId::monospace(11.0), DIM);
        if resp.clicked() && !on {
            pick = Some(value);
        }
    };
    row(ui, None, "ไม่ล็อก — ใช้ชุดที่เพลงเลือก", "");
    for (i, label) in &kits {
        let (code, name) = label.split_at(7);
        row(ui, Some(*i), name.trim(), code);
    }
    if let Some(v) = pick {
        app.synth.set_pin(DRUM_CH, v);
    }
}
