//! Song folders, audio device and lyric display (SoundFonts live on the
//! Sounds page).

use std::path::PathBuf;

use eframe::egui::{self, Margin, RichText};

use super::Target;
use crate::app::KaraokeApp;
use crate::icons;
use crate::style::{DANGER, DIM, TEXT};

const KEYS: [(&str, &str); 13] = [
    ("Space", "เล่น / พัก"),
    ("Left  Right", "ถอย / ข้าม 5 วินาที"),
    ("[  ]", "ลด / เพิ่มคีย์"),
    (",  .", "ช้าลง / เร็วขึ้น"),
    ("N", "เพลงถัดไปในคิว"),
    ("F  F11", "เต็มจอ (Esc ออก)"),
    ("/", "ค้นหาเพลง"),
    ("Q", "คิวเพลง"),
    ("M", "มิกเซอร์"),
    ("S", "เสียง / SoundFont"),
    ("Ctrl K", "คำสั่งทั้งหมด"),
    ("Ctrl ,", "ตั้งค่า"),
    ("Tab", "สลับหน้าในแผงนี้"),
];

/// Returns a browse request when a "change…" button was pressed.
pub fn show(app: &mut KaraokeApp, ui: &mut egui::Ui, max_h: f32) -> Option<(Target, Option<PathBuf>)> {
    let mut browse = None;
    egui::ScrollArea::vertical().max_height(max_h + 60.0).auto_shrink([false, true]).show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);

            section(ui, icons::DATABASE, "คลังเพลง", "โฟลเดอร์ NCN (Song / Lyrics / Cursor) หรือโฟลเดอร์ไฟล์ .sfkar");
            let mut remove = None;
            for (i, src) in app.library.db.sources.iter().enumerate() {
                ui.horizontal(|ui| {
                    let kind = match src.kind {
                        solfege_songdb::SourceKind::Ncn => "NCN",
                        solfege_songdb::SourceKind::Sfkar => "SFKAR",
                    };
                    let count = app.library.db.songs.iter().filter(|s| s.source == i).count();
                    ui.add_sized([46.0, 18.0], egui::Label::new(RichText::new(kind).size(10.0).color(DIM)));
                    ui.add_sized([64.0, 18.0], egui::Label::new(RichText::new(format!("{count} เพลง")).size(12.0).color(DIM)));
                    if ui.small_button(icons::REMOVE).on_hover_text("เอาออกจากคลัง (ไม่ลบไฟล์)").clicked() {
                        remove = Some(i);
                    }
                    ui.add(egui::Label::new(RichText::new(src.path.display().to_string()).monospace().size(12.0).color(TEXT)).truncate());
                });
            }
            if let Some(i) = remove {
                app.library.remove_source(i);
            }
            if app.library.db.sources.is_empty() {
                ui.label(RichText::new("ยังไม่มีโฟลเดอร์เพลง").color(DANGER));
            }
            ui.horizontal(|ui| {
                if ui.button(format!("{}  เพิ่มโฟลเดอร์…", icons::FOLDER_PLUS)).clicked() {
                    browse = Some((Target::Library, app.library.db.sources.last().map(|s| s.path.clone())));
                }
                let can_scan = !app.library.db.sources.is_empty() && !app.library.scanning();
                if ui.add_enabled(can_scan, egui::Button::new(format!("{}  สแกนใหม่", icons::REFRESH))).clicked() {
                    app.library.rescan();
                }
                if app.library.scanning() {
                    ui.spinner();
                    ui.label(RichText::new("กำลังสแกน…").color(DIM));
                } else {
                    ui.label(RichText::new(format!("{} เพลงในฐานข้อมูล", app.library.db.songs.len())).size(12.0).color(DIM));
                }
            });
            ui.add_space(14.0);

            section(ui, icons::HEADPHONES, "อุปกรณ์เสียง", "");
            if app.devices.is_empty() {
                app.devices = solfege_synth::audio::output_devices();
            }
            ui.horizontal(|ui| {
                let current = app.settings.device.clone().unwrap_or_else(|| "ค่าเริ่มต้นของระบบ".into());
                let mut choice = app.settings.device.clone();
                egui::ComboBox::from_id_salt("device").selected_text(current).width(340.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut choice, None, "ค่าเริ่มต้นของระบบ");
                    for d in &app.devices {
                        ui.selectable_value(&mut choice, Some(d.clone()), d);
                    }
                });
                if choice != app.settings.device {
                    app.settings.device = choice;
                    app.reopen_output();
                }
                if ui.button(icons::REFRESH).on_hover_text("ค้นหาอุปกรณ์ใหม่").clicked() {
                    app.devices = solfege_synth::audio::output_devices();
                }
            });
            match &app.synth.output_error {
                Some(e) => ui.colored_label(DANGER, e),
                None => ui.label(RichText::new(&app.synth.output_info).size(12.0).color(DIM)),
            };
            ui.add_space(14.0);

            section(ui, icons::TYPE, "เนื้อร้อง", "");
            egui::Grid::new("lyric-settings").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                ui.label("ขนาดตัวอักษร");
                ui.add(egui::Slider::new(&mut app.settings.lyric_scale, 0.6..=1.6).fixed_decimals(2));
                ui.end_row();
                ui.label("เลื่อนเวลาเนื้อร้อง");
                ui.add(egui::Slider::new(&mut app.settings.lyric_offset_ms, -800..=800).suffix(" ms"))
                    .on_hover_text("ค่าบวก = เนื้อร้องช้าลง ใช้ชดเชยความหน่วงของลำโพง");
                ui.end_row();
            });
            ui.add_space(14.0);

            section(ui, icons::KEYBOARD, "ปุ่มลัด", "");
            egui::Grid::new("keys").num_columns(4).spacing([14.0, 4.0]).show(ui, |ui| {
                for (i, (k, what)) in KEYS.iter().enumerate() {
                    ui.label(RichText::new(*k).monospace().color(TEXT));
                    ui.label(RichText::new(*what).color(DIM));
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
        });
    });
    browse
}

fn section(ui: &mut egui::Ui, icon: &str, title: &str, hint: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{icon}  {title}")).strong().color(TEXT));
        if !hint.is_empty() {
            ui.label(RichText::new(hint).size(12.0).color(DIM));
        }
    });
}
