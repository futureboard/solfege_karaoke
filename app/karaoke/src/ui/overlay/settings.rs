//! Settings popup: a sidebar of sections (song library, audio output,
//! lyrics, shortcuts, data files) and the chosen section's cards beside
//! it. SoundFonts and instrument sounds have their own window
//! (`ui::sound`); the effect chain lives in the mixer.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Frame, Margin, Rect, RichText, Sense, Stroke, pos2, vec2};

use crate::app::KaraokeApp;
use crate::config::LyricMode;
use crate::dialog::Pick;
use crate::icons;
use crate::style::{self, ACCENT, DANGER, DIM, INK, LINE, RAISED, SUNG, TEXT, UNSUNG, lyrics_family};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Section {
    #[default]
    Library,
    Audio,
    Lyrics,
    Keys,
    Files,
}

const SIDEBAR: f32 = 190.0;

/// Keyboard shortcuts, by group.
const KEYS: [(&str, &[(&str, &str)]); 3] = [
    (
        "เล่นเพลง",
        &[
            ("Space", "เล่น / พัก"),
            ("Shift Space", "หยุด กลับไปต้นเพลง"),
            ("← →", "ถอย / ข้าม 5 วินาที"),
            ("N", "เพลงถัดไปในคิว"),
            ("[ ]", "ลด / เพิ่มคีย์"),
            (", .", "ช้าลง / เร็วขึ้น"),
        ],
    ),
    (
        "หน้าต่างและแผง",
        &[
            ("/", "ค้นหาเพลง"),
            ("Q", "คิวเพลง"),
            ("Ctrl K", "คำสั่งทั้งหมด"),
            ("Ctrl ,", "ตั้งค่า"),
            ("M  E", "มิกเซอร์และเอฟเฟกต์"),
            ("S", "เสียงและ SoundFont"),
            ("F  F11", "เต็มจอ (Esc ออก)"),
            ("Tab", "สลับเพลง / คิว"),
        ],
    ),
    (
        "เนื้อร้องและเสียงร้อง",
        &[("L", "สลับรูปแบบเนื้อร้อง"), ("V", "เปิด / ปิดเมโลดี้ร้องนำ"), ("คลิกขวา", "เมนูของสิ่งที่คลิก"), ("ดับเบิลคลิก", "เนื้อร้อง: เต็มจอ")],
    ),
];

pub fn show(app: &mut KaraokeApp, ui: &mut egui::Ui, section: &mut Section, max_h: f32) {
    let body_h = max_h + 60.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ui.vertical(|ui| {
            ui.set_width(SIDEBAR);
            ui.set_height(body_h);
            sidebar(app, ui, section);
        });
        let (line, _) = ui.allocate_exact_size(vec2(1.0, body_h), Sense::hover());
        ui.painter().rect_filled(line, 0.0, LINE);
        ui.vertical(|ui| {
            ui.set_height(body_h);
            egui::ScrollArea::vertical().id_salt(("settings", *section)).auto_shrink([false, false]).show(ui, |ui| {
                Frame::new().inner_margin(Margin::symmetric(22, 18)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                    let w = &mut ui.visuals_mut().widgets;
                    w.inactive.weak_bg_fill = INK;
                    w.inactive.bg_fill = INK;
                    w.hovered.weak_bg_fill = style::mix(INK, LINE, 0.8);
                    match section {
                        Section::Library => library(app, ui),
                        Section::Audio => audio(app, ui),
                        Section::Lyrics => lyrics(app, ui),
                        Section::Keys => keys(ui),
                        Section::Files => files(app, ui),
                    }
                });
            });
        });
    });
}

fn sidebar(app: &KaraokeApp, ui: &mut egui::Ui, section: &mut Section) {
    ui.add_space(12.0);
    let items = [
        (Section::Library, icons::DATABASE, "คลังเพลง", app.library.db.songs.len().to_string()),
        (Section::Audio, icons::HEADPHONES, "เสียงออก", String::new()),
        (Section::Lyrics, icons::TYPE, "เนื้อร้อง", String::new()),
        (Section::Keys, icons::KEYBOARD, "ปุ่มลัด", String::new()),
        (Section::Files, icons::FOLDER_OPEN, "ไฟล์ข้อมูล", String::new()),
    ];
    for (s, icon, label, badge) in items {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::click());
        let r = rect.shrink2(vec2(10.0, 3.0));
        let on = *section == s;
        let p = ui.painter();
        if on || resp.hovered() {
            p.rect_filled(r, CornerRadius::same(9), RAISED);
        }
        if on {
            p.rect_filled(Rect::from_min_size(r.left_top() + vec2(0.0, 9.0), vec2(3.0, r.height() - 18.0)), 2.0, SUNG);
        }
        p.text(pos2(r.left() + 16.0, r.center().y), Align2::LEFT_CENTER, icon, FontId::proportional(15.0), if on { SUNG } else { DIM });
        p.text(pos2(r.left() + 40.0, r.center().y), Align2::LEFT_CENTER, label, FontId::proportional(14.0), if on { TEXT } else { DIM });
        if !badge.is_empty() {
            p.text(pos2(r.right() - 12.0, r.center().y), Align2::RIGHT_CENTER, badge, FontId::proportional(12.0), DIM);
        }
        if resp.clicked() {
            *section = s;
        }
    }
}

// ------------------------------------------------------------ building blocks

fn title(ui: &mut egui::Ui, text: &str, hint: &str) {
    ui.label(RichText::new(text).size(18.0).strong().color(TEXT));
    if !hint.is_empty() {
        ui.label(RichText::new(hint).size(12.5).color(DIM));
    }
    ui.add_space(4.0);
}

/// A raised card the width of the page.
fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    Frame::new()
        .fill(RAISED)
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Card heading: icon and name, with an optional note under it.
fn card_title(ui: &mut egui::Ui, icon: &str, text: &str, note: &str) {
    ui.label(RichText::new(format!("{icon}  {text}")).size(14.0).strong().color(TEXT));
    if !note.is_empty() {
        ui.label(RichText::new(note).size(12.0).color(DIM));
    }
}

/// An on / off switch; returns true when flipped.
fn switch(ui: &mut egui::Ui, on: &mut bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 22.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.12);
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(11), style::mix(INK, ACCENT, t * 0.85));
    if !*on {
        p.rect_stroke(rect, CornerRadius::same(11), Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
    }
    let x = egui::lerp(rect.left() + 11.0..=rect.right() - 11.0, t);
    p.circle_filled(pos2(x, rect.center().y), 8.0, if *on { INK } else { DIM });
    resp.clicked()
}

/// A setting row: label and note on the left, the control on the right.
fn row(ui: &mut egui::Ui, label: &str, note: &str, control: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_max_width((ui.available_width() - 260.0).max(200.0));
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new(label).color(TEXT));
            if !note.is_empty() {
                ui.label(RichText::new(note).size(12.0).color(DIM));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
}

// ------------------------------------------------------------------- library

fn library(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    title(ui, "คลังเพลง", "โฟลเดอร์ NCN (Song / Lyrics / Cursor) หรือโฟลเดอร์ที่มีไฟล์ .sfkar รวมกันเป็นรายการเพลงเดียว");

    let songs = app.library.db.songs.len();
    let favourites = app.library.db.stats.values().filter(|s| s.favorite).count();
    let sung = app.library.db.stats.values().filter(|s| s.plays > 0).count();
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 16.0) / 3.0;
        for (value, label, color) in [(songs, "เพลงทั้งหมด", TEXT), (favourites, "เพลงโปรด", SUNG), (sung, "เคยร้องแล้ว", ACCENT)] {
            let (rect, _) = ui.allocate_exact_size(vec2(w, 66.0), Sense::hover());
            let p = ui.painter();
            p.rect_filled(rect, CornerRadius::same(12), RAISED);
            p.text(pos2(rect.left() + 16.0, rect.top() + 14.0), Align2::LEFT_TOP, value.to_string(), FontId::proportional(22.0), color);
            p.text(pos2(rect.left() + 16.0, rect.bottom() - 12.0), Align2::LEFT_BOTTOM, label, FontId::proportional(12.0), DIM);
        }
    });
    ui.add_space(4.0);

    let mut remove = None;
    if app.library.db.sources.is_empty() {
        card(ui, |ui| {
            ui.label(RichText::new("ยังไม่มีโฟลเดอร์เพลง — เพิ่มโฟลเดอร์ NCN หรือ .sfkar เพื่อเริ่มร้อง").color(DANGER));
        });
    }
    for (i, src) in app.library.db.sources.iter().enumerate() {
        let count = app.library.db.songs.iter().filter(|s| s.source == i).count();
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let (kind, color) = match src.kind {
                    solfege_songdb::SourceKind::Ncn => ("NCN", ACCENT),
                    solfege_songdb::SourceKind::Sfkar => ("SFKAR", SUNG),
                };
                let (badge, _) = ui.allocate_exact_size(vec2(56.0, 34.0), Sense::hover());
                ui.painter().rect_filled(badge, CornerRadius::same(8), color.gamma_multiply(0.16));
                ui.painter().text(badge.center(), Align2::CENTER_CENTER, kind, FontId::proportional(12.0), color);
                ui.vertical(|ui| {
                    ui.set_max_width((ui.available_width() - 60.0).max(120.0));
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let name = src.path.file_name().map_or_else(|| src.path.display().to_string(), |n| n.to_string_lossy().into_owned());
                    ui.add(egui::Label::new(RichText::new(name).color(TEXT)).truncate());
                    let path = src.path.display().to_string();
                    ui.add(egui::Label::new(RichText::new(format!("{count} เพลง  ·  {path}")).size(12.0).color(DIM)).truncate()).on_hover_text(path);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(icons::REMOVE).on_hover_text("เอาออกจากคลัง (ไม่ลบไฟล์)").clicked() {
                        remove = Some(i);
                    }
                });
            });
        });
    }
    if let Some(i) = remove {
        app.library.remove_source(i);
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let picking = app.dialogs.busy();
        let add = egui::Button::new(RichText::new(format!("{}   เพิ่มโฟลเดอร์เพลง…", icons::FOLDER_PLUS)).size(14.0))
            .min_size(vec2(220.0, 38.0))
            .corner_radius(CornerRadius::same(10));
        if ui.add_enabled(!picking, add).on_hover_text("เปิดหน้าต่างเลือกโฟลเดอร์ของระบบ").clicked() {
            app.dialogs.ask(Pick::SongFolder, app.library.db.sources.last().map(|s| s.path.clone()));
        }
        let can_scan = !app.library.db.sources.is_empty() && !app.library.scanning();
        let scan = egui::Button::new(RichText::new(format!("{}   สแกนใหม่", icons::REFRESH)).size(14.0)).min_size(vec2(130.0, 38.0)).corner_radius(CornerRadius::same(10));
        if ui.add_enabled(can_scan, scan).on_hover_text("อ่านรายชื่อเพลงจากทุกโฟลเดอร์ใหม่").clicked() {
            app.library.rescan();
        }
        if app.library.scanning() {
            ui.spinner();
            ui.label(RichText::new("กำลังสแกน…").color(DIM));
        }
    });
}

// --------------------------------------------------------------------- audio

fn audio(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    title(ui, "เสียงออก", "อุปกรณ์ที่ใช้เล่นเสียง ความดังรวม และเมโลดี้นำร้อง");
    if app.devices.is_empty() {
        app.devices = solfege_synth::audio::output_devices();
    }
    card(ui, |ui| {
        card_title(ui, icons::HEADPHONES, "อุปกรณ์เสียง", "");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let current = app.settings.device.clone().unwrap_or_else(|| "ค่าเริ่มต้นของระบบ".into());
            let mut choice = app.settings.device.clone();
            let width = (ui.available_width() - 50.0).clamp(200.0, 460.0);
            crate::ui::fixed_width(ui, width, |ui| {
                egui::ComboBox::from_id_salt("device").truncate().selected_text(current).width(width).show_ui(ui, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                    ui.set_max_width(520.0);
                    ui.selectable_value(&mut choice, None, "ค่าเริ่มต้นของระบบ");
                    for d in &app.devices {
                        ui.selectable_value(&mut choice, Some(d.clone()), d);
                    }
                })
            });
            if choice != app.settings.device {
                app.settings.device = choice;
                app.reopen_output();
            }
            if ui.button(icons::REFRESH).on_hover_text("ค้นหาอุปกรณ์ใหม่").clicked() {
                app.devices = solfege_synth::audio::output_devices();
            }
        });
        let (dot, text) = match &app.synth.output_error {
            Some(e) => (DANGER, format!("ไม่มีเสียงออก — {e}")),
            None => (ACCENT, format!("{} · {}", solfege_synth::audio::host_name(), app.synth.output_info)),
        };
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 4.0, dot);
            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(if dot == DANGER { DANGER } else { DIM })).truncate());
        });
    });
    card(ui, |ui| {
        row(ui, "ความดังเพลง", "ระดับเสียงดนตรีทั้งหมด (เหมือนแถบ VOL ด้านล่าง)", |ui| {
            let mut v = app.synth.volume();
            ui.spacing_mut().slider_width = 200.0;
            if ui.add(egui::Slider::new(&mut v, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0}", v * 100.0))).changed() {
                app.synth.set_volume(v);
            }
        });
        ui.separator();
        row(ui, "เมโลดี้ร้องนำ", "เสียงนำร้องในแชนแนล 9 ของเพลง NCN ปิดได้เมื่อร้องได้แล้ว  ·  V", |ui| {
            let mut on = !app.synth.melody_off();
            if switch(ui, &mut on) {
                app.toggle_melody();
            }
        });
    });
    card(ui, |ui| {
        row(ui, "SoundFont และเสียงเครื่องดนตรี", "เลือกไฟล์เสียง เสียงของแต่ละแชนแนล เครื่องดนตรี และชุดกลอง", |ui| {
            if ui.button(format!("{}  เปิดหน้าต่างเสียง  ·  S", icons::FILE_MUSIC)).clicked() {
                app.open_sound();
            }
        });
    });
}

// -------------------------------------------------------------------- lyrics

fn lyrics(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    title(ui, "เนื้อร้อง", "รูปแบบ ขนาด และจังหวะของเนื้อร้องบนจอ");
    ui.label(RichText::new("รูปแบบ  ·  L").size(12.0).color(DIM));
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 8.0) / 2.0;
        for mode in [LyricMode::Scroll, LyricMode::Classic] {
            if layout_card(ui, mode, app.settings.lyric_mode == mode, w) {
                app.settings.lyric_mode = mode;
            }
        }
    });
    ui.add_space(4.0);
    card(ui, |ui| {
        row(ui, "ขนาดตัวอักษร", "เทียบกับความสูงของจอ", |ui| {
            ui.spacing_mut().slider_width = 200.0;
            ui.add(egui::Slider::new(&mut app.settings.lyric_scale, 0.6..=1.6).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
        });
        // Live sample at the chosen size.
        let size = 30.0 * app.settings.lyric_scale;
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), size * 1.6 + 12.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(10), INK);
        let g = p.layout_no_wrap("ตัวอย่างเนื้อร้อง".into(), FontId::new(size, lyrics_family()), UNSUNG);
        let pos = rect.center() - g.size() / 2.0;
        let half = pos.x + g.size().x * 0.45;
        p.galley(pos, g.clone(), UNSUNG);
        p.with_clip_rect(Rect::from_min_max(rect.min, pos2(half, rect.max.y))).galley_with_override_text_color(pos, g, SUNG);
        ui.separator();
        row(ui, "เลื่อนเวลาเนื้อร้อง", "ค่าบวก = เนื้อร้องช้าลง ใช้ชดเชยความหน่วงของลำโพงหรือ Bluetooth", |ui| {
            if ui.add_enabled(app.settings.lyric_offset_ms != 0, egui::Button::new(icons::UNDO)).on_hover_text("กลับเป็น 0").clicked() {
                app.settings.lyric_offset_ms = 0;
            }
            ui.spacing_mut().slider_width = 200.0;
            ui.add(egui::Slider::new(&mut app.settings.lyric_offset_ms, -800..=800).suffix(" ms"));
        });
        ui.separator();
        row(ui, "นาฬิกา", "แสดงเวลาตอนนี้ที่มุมขวาบนของจอเนื้อร้อง", |ui| {
            switch(ui, &mut app.settings.show_clock);
        });
    });
}

/// A clickable picture of a lyric layout; returns true when picked.
fn layout_card(ui: &mut egui::Ui, mode: LyricMode, on: bool, w: f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 150.0), Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(12), if on { SUNG.gamma_multiply(0.10) } else { RAISED });
    p.rect_stroke(rect, CornerRadius::same(12), Stroke::new(if on { 1.5 } else { 1.0 }, if on { SUNG } else if resp.hovered() { DIM } else { LINE }), egui::StrokeKind::Inside);
    // Mini stage.
    let stage = Rect::from_min_size(rect.min + vec2(14.0, 14.0), vec2(rect.width() - 28.0, 86.0));
    p.rect_filled(stage, CornerRadius::same(8), INK);
    let bar = |y: f32, len: f32, sung: f32, color: Color32, h: f32| {
        let x0 = stage.center().x - len / 2.0;
        let full = Rect::from_min_size(pos2(x0, y - h / 2.0), vec2(len, h));
        p.rect_filled(full, CornerRadius::same(2), color);
        if sung > 0.0 {
            p.rect_filled(Rect::from_min_size(full.min, vec2(len * sung, h)), CornerRadius::same(2), SUNG);
        }
    };
    let len = stage.width() * 0.62;
    match mode {
        LyricMode::Scroll => {
            let c = stage.center().y;
            bar(c - 30.0, len * 0.7, 0.0, UNSUNG.gamma_multiply(0.35), 5.0);
            bar(c - 16.0, len * 0.8, 1.0, UNSUNG.gamma_multiply(0.6), 6.0);
            bar(c, len, 0.55, UNSUNG, 9.0);
            bar(c + 16.0, len * 0.85, 0.0, UNSUNG.gamma_multiply(0.7), 6.0);
            bar(c + 30.0, len * 0.65, 0.0, UNSUNG.gamma_multiply(0.35), 5.0);
        }
        LyricMode::Classic => {
            let c = stage.center().y;
            bar(c - 11.0, len, 0.55, UNSUNG, 9.0);
            bar(c + 11.0, len * 0.9, 0.0, UNSUNG.gamma_multiply(0.8), 9.0);
        }
    }
    let note = match mode {
        LyricMode::Scroll => "บรรทัดที่ร้องอยู่ตรงกลาง ร้องจบแล้วเลื่อนขึ้น",
        LyricMode::Classic => "สองบรรทัดอยู่กับที่ ร้องสลับบน / ล่าง",
    };
    let mark = if on { format!("{}  ", icons::CHECK) } else { String::new() };
    p.text(pos2(rect.left() + 16.0, rect.bottom() - 34.0), Align2::LEFT_CENTER, format!("{mark}{}", mode.label()), FontId::proportional(14.0), if on { SUNG } else { TEXT });
    p.text(pos2(rect.left() + 16.0, rect.bottom() - 15.0), Align2::LEFT_CENTER, note, FontId::proportional(11.5), DIM);
    resp.clicked()
}

// ---------------------------------------------------------------------- keys

fn keys(ui: &mut egui::Ui) {
    title(ui, "ปุ่มลัด", "ใช้ได้เมื่อไม่มีหน้าต่างอื่นเปิดอยู่  ·  คลิกขวาที่ไหนก็ได้เพื่อดูเมนูพร้อมปุ่มลัด");
    for (group, list) in KEYS {
        card(ui, |ui| {
            ui.label(RichText::new(group).size(13.0).strong().color(TEXT));
            ui.add_space(2.0);
            egui::Grid::new(("keys", group)).num_columns(2).spacing([18.0, 8.0]).min_col_width(130.0).show(ui, |ui| {
                for (k, what) in list.iter() {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for cap in k.split_whitespace() {
                            keycap(ui, cap);
                        }
                    });
                    ui.label(RichText::new(*what).color(DIM));
                    ui.end_row();
                }
            });
        });
    }
}

fn keycap(ui: &mut egui::Ui, key: &str) {
    let key = key.replace('←', icons::PREVIOUS).replace('→', icons::COLLAPSED);
    let g = ui.painter().layout_no_wrap(key, FontId::proportional(12.0), TEXT);
    let (rect, _) = ui.allocate_exact_size(vec2((g.size().x + 14.0).max(26.0), 24.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(6), INK);
    p.rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
    p.hline(rect.x_range().shrink(3.0), rect.bottom() - 1.5, Stroke::new(1.0, LINE));
    p.galley(rect.center() - g.size() / 2.0, g, TEXT);
}

// --------------------------------------------------------------------- files

fn files(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    title(ui, "ไฟล์ข้อมูล", "การตั้งค่าและคลังเพลงเก็บในโฟลเดอร์ข้อมูลของโปรแกรม");
    let entries = [
        (icons::SETTINGS, "การตั้งค่า (config.json)", "JSON อ่านง่าย แก้เองได้ขณะปิดโปรแกรม ค่าที่หายไปจะใช้ค่าเริ่มต้น", app.config_path().map(|p| p.to_path_buf())),
        (icons::DATABASE, "ฐานข้อมูลเพลง (songs.dat)", "SQLite: โฟลเดอร์เพลง รายชื่อเพลง เพลงโปรด และประวัติการร้อง", app.library.path().map(|p| p.to_path_buf())),
    ];
    for (icon, name, note, path) in entries {
        card(ui, |ui| {
            card_title(ui, icon, name, note);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let text = path.as_ref().map_or("— (ไม่ได้บันทึกลงดิสก์)".to_string(), |p| p.display().to_string());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(path.is_some(), egui::Button::new(format!("{}  คัดลอก", icons::COPY))).on_hover_text("คัดลอกที่อยู่ไฟล์").clicked() {
                        ui.ctx().copy_text(text.clone());
                    }
                    Frame::new().fill(INK).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(egui::Label::new(RichText::new(&text).monospace().size(12.0).color(TEXT)).truncate()).on_hover_text(&text);
                    });
                });
            });
        });
    }
}
