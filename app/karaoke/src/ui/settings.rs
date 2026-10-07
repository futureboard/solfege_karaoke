//! Settings window: library folder, SoundFont, audio device, lyric display.

use eframe::egui::{self, RichText};

use crate::icons;
use crate::app::{KaraokeApp, Picking};
use crate::style::{DANGER, DIM, TEXT};
use crate::ui::files::{FilePicker, Mode};

const KEYS: [(&str, &str); 10] = [
    ("Space", "เล่น / พัก"),
    ("Left Right", "ถอย / ข้าม 5 วินาที"),
    ("[  ]", "ลด / เพิ่มคีย์"),
    (",  .", "ช้าลง / เร็วขึ้น"),
    ("N", "เพลงถัดไปในคิว"),
    ("F  F11", "เต็มจอ (Esc ออก)"),
    ("/", "ค้นหาเพลง"),
    ("Enter", "จองเพลงที่เลือก"),
    ("Up Down", "เลือกเพลงในผลค้นหา"),
    ("ดับเบิลคลิก", "ร้องทันที / สลับเต็มจอ"),
];

pub fn show(app: &mut KaraokeApp, ctx: &egui::Context) {
    if !app.show_settings {
        return;
    }
    let mut open = true;
    egui::Window::new("ตั้งค่า")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(520.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            section(ui, icons::MUSIC, "คลังเพลง NCN", "โฟลเดอร์ที่มี Song, Lyrics และ Cursor");
            path_row(ui, app.library.root.as_deref().map(|p| p.display().to_string()));
            ui.horizontal(|ui| {
                if ui.button(format!("{}  เลือกโฟลเดอร์…", icons::FOLDER_OPEN)).clicked() {
                    let start = app.library.root.clone();
                    app.picker = Some((Picking::Library, FilePicker::new(Mode::Folder, start)));
                }
                if ui.add_enabled(app.library.root.is_some(), egui::Button::new(format!("{}  สแกนใหม่", icons::REFRESH))).clicked()
                    && let Some(root) = app.library.root.clone()
                {
                    app.open_library(root);
                }
            });
            ui.add_space(12.0);

            section(ui, icons::FILE_MUSIC, "SoundFont", "ไฟล์ .sf2 (General MIDI) สำหรับเล่นดนตรี");
            path_row(ui, app.synth.font_path.as_deref().map(|p| p.display().to_string()));
            ui.horizontal(|ui| {
                if ui.button(format!("{}  เลือกไฟล์…", icons::FILE_MUSIC)).clicked() {
                    let start = app.synth.font_path.clone();
                    app.picker = Some((Picking::SoundFont, FilePicker::new(Mode::File(&["sf2", "sfz"]), start)));
                }
                if app.synth.loading_soundfont() {
                    ui.spinner();
                    ui.label(RichText::new("กำลังโหลด…").color(DIM));
                }
            });
            ui.add_space(12.0);

            section(ui, icons::HEADPHONES, "อุปกรณ์เสียง", "");
            if app.devices.is_empty() {
                app.devices = solfege_synth::audio::output_devices();
            }
            ui.horizontal(|ui| {
                let current = app.settings.device.clone().unwrap_or_else(|| "ค่าเริ่มต้นของระบบ".into());
                let mut choice = app.settings.device.clone();
                egui::ComboBox::from_id_salt("device").selected_text(current).width(320.0).show_ui(ui, |ui| {
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
                None => ui.label(RichText::new(&app.synth.output_info).color(DIM)),
            };
            ui.add_space(12.0);

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
            ui.add_space(12.0);

            section(ui, icons::KEYBOARD, "ปุ่มลัด", "");
            egui::Grid::new("keys").num_columns(4).spacing([12.0, 4.0]).show(ui, |ui| {
                for (i, (k, what)) in KEYS.iter().enumerate() {
                    ui.label(RichText::new(*k).monospace().color(TEXT));
                    ui.label(RichText::new(*what).color(DIM));
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
        });
    if !open {
        app.show_settings = false;
    }
}

fn section(ui: &mut egui::Ui, icon: &str, title: &str, hint: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{icon}  {title}")).strong().color(TEXT));
        if !hint.is_empty() {
            ui.label(RichText::new(hint).size(12.0).color(DIM));
        }
    });
}

fn path_row(ui: &mut egui::Ui, path: Option<String>) {
    match path {
        Some(p) => ui.add(egui::Label::new(RichText::new(p).monospace().size(12.0)).truncate()),
        None => ui.label(RichText::new("ยังไม่ได้เลือก").color(DANGER)),
    };
}
