//! About: what this is, its status and licence, how it runs on this
//! machine, and the work it is built on.

use eframe::egui::{self, CornerRadius, Margin, RichText};

use crate::app::KaraokeApp;
use crate::icons;
use crate::style::{ACCENT, DANGER, DIM, INK, SUNG, TEXT};

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Libraries and assets the player is built on, with their licences.
const CREDITS: [(&str, &str, &str); 9] = [
    ("egui / eframe", "MIT OR Apache-2.0", "หน้าจอและหน้าต่าง"),
    ("wgpu", "MIT OR Apache-2.0", "วาดภาพผ่าน Vulkan / Metal / Direct3D 12 / OpenGL"),
    ("winit", "Apache-2.0", "หน้าต่างและอินพุต"),
    ("cpal", "Apache-2.0", "ส่งเสียงออกอุปกรณ์"),
    ("rfd", "MIT", "หน้าต่างเลือกไฟล์ของระบบ"),
    ("rusqlite / SQLite", "MIT / Public Domain", "ฐานข้อมูลเพลง songs.dat"),
    ("chrono", "MIT OR Apache-2.0", "นาฬิกา"),
    ("Noto Sans Thai, Noto Sans", "SIL OFL 1.1", "ฟอนต์"),
    ("Lucide", "ISC", "ไอคอน"),
];

pub fn show(app: &mut KaraokeApp, ui: &mut egui::Ui, max_h: f32) {
    egui::ScrollArea::vertical().max_height(max_h + 60.0).auto_shrink([false, true]).show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin::symmetric(22, 18)).show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);

            ui.horizontal(|ui| {
                ui.label(RichText::new(icons::MIC).size(30.0).color(SUNG));
                ui.vertical(|ui| {
                    ui.label(RichText::new("Solfege Karaoke").size(22.0).strong().color(TEXT));
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("เวอร์ชัน {VERSION}")).color(DIM));
                        badge(ui, "EXPERIMENTAL", SUNG);
                        badge(ui, "NOT FOR PRODUCTION", DANGER);
                    });
                });
            });
            ui.add_space(4.0);
            ui.label(RichText::new("โปรแกรมคาราโอเกะสำหรับเพลงไทย เล่นเพลง NCN และไฟล์ .sfkar ด้วยเสียงจาก SoundFont พร้อมเนื้อร้องที่ไล่สีทีละพยางค์").color(TEXT));
            ui.label(RichText::new("A karaoke player for Thai songs: NCN and .sfkar, SoundFont backing tracks, lyrics that light up syllable by syllable.").size(12.0).color(DIM));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(icons::LINK).color(DIM));
                ui.hyperlink_to(RichText::new(REPOSITORY).monospace().size(12.0).color(ACCENT), REPOSITORY);
            });
            ui.add_space(12.0);

            section(ui, icons::MONITOR, "เครื่องนี้");
            egui::Grid::new("about-system").num_columns(2).spacing([16.0, 5.0]).show(ui, |ui| {
                row(ui, "การแสดงผล", &app.renderer);
                let audio = match &app.synth.output_error {
                    Some(e) => format!("ไม่มีเสียงออก ({e})"),
                    None => format!("{} · {}", solfege_synth::audio::host_name(), app.synth.output_info),
                };
                row(ui, "เสียง", &audio);
                row(ui, "ระบบ", &format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH));
                row(ui, "คลังเพลง", &format!("{} เพลง จาก {} โฟลเดอร์", app.library.db.songs.len(), app.library.db.sources.len()));
                row(ui, "SoundFont", &format!("{} ไฟล์", app.synth.fonts().len()));
            });
            ui.add_space(12.0);

            section(ui, icons::INFO, "สัญญาอนุญาต");
            ui.label(RichText::new("MIT OR Apache-2.0 เลือกใช้ได้ตามต้องการ ซอฟต์แวร์ให้ไว้ \"ตามสภาพ\" (as is) โดยไม่มีการรับประกัน").color(TEXT));
            egui::Frame::new().fill(INK).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(
                        "ไม่รองรับ และไม่สนับสนุนการนำโค้ดไปรองรับ ฟอร์แมตคาราโอเกะเชิงพาณิชย์ที่เข้ารหัสหรือมีระบบป้องกัน (EMK, XMK, SIB, Sonic Karaoke) หรือเพลงที่ไม่ได้รับอนุญาตจากเจ้าของลิขสิทธิ์ ผู้นำไปใช้ต้องรับผิดชอบเอง",
                    )
                    .size(12.0)
                    .color(DIM),
                );
                ui.label(
                    RichText::new("No support for proprietary or copy-protected karaoke formats or unlicensed songs; such use is at your own responsibility.")
                        .size(11.0)
                        .color(DIM),
                );
            });
            ui.add_space(12.0);

            section(ui, icons::DATABASE, "สร้างด้วย");
            egui::Grid::new("about-credits").num_columns(3).spacing([16.0, 4.0]).show(ui, |ui| {
                for (name, licence, what) in CREDITS {
                    ui.label(RichText::new(name).color(TEXT));
                    ui.label(RichText::new(licence).monospace().size(11.0).color(ACCENT));
                    ui.label(RichText::new(what).size(12.0).color(DIM));
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            ui.label(
                RichText::new("ส่วนหนึ่งของโค้ดเขียนร่วมกับผู้ช่วยเขียนโค้ด AI และผ่านการรีวิวก่อนรวม ดู AI_POLICY.md ใน repository")
                    .size(12.0)
                    .color(DIM),
            );
        });
    });
}

fn section(ui: &mut egui::Ui, icon: &str, title: &str) {
    ui.label(RichText::new(format!("{icon}  {title}")).strong().color(TEXT));
}

fn row(ui: &mut egui::Ui, what: &str, value: &str) {
    ui.label(RichText::new(what).color(DIM));
    ui.add(egui::Label::new(RichText::new(value).color(TEXT)).truncate());
    ui.end_row();
}

fn badge(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(6, 1))
        .show(ui, |ui| ui.label(RichText::new(text).size(10.0).color(color)));
}
