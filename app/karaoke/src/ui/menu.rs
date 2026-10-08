//! Context menus (right click). Every entry has an icon, a label and, when
//! there is one, its keyboard shortcut on the right, so the menu doubles as
//! a reminder of the keys. The app menu opens on the stage and on the
//! bottom bar; lists, mixer strips and SoundFonts have their own.

use eframe::egui::{self, RichText};
use solfege_synth::engine::PlayState;

use crate::app::KaraokeApp;
use crate::config::LyricMode;
use crate::icons;
use crate::style::DIM;
use crate::ui::overlay::Page;

/// One entry; returns true when picked (the menu then closes by itself).
pub fn item(ui: &mut egui::Ui, icon: &str, label: &str, keys: &str) -> bool {
    item_if(ui, true, icon, label, keys)
}

/// An entry that can be greyed out.
pub fn item_if(ui: &mut egui::Ui, enabled: bool, icon: &str, label: &str, keys: &str) -> bool {
    let text = format!("{icon}   {label}");
    let mut button = egui::Button::new(text).min_size(egui::vec2(230.0, 26.0));
    if !keys.is_empty() {
        // Arrows and Enter come from the icon font (the text fonts lack them).
        let keys = keys.replace('↵', icons::ENTER).replace('↑', icons::MOVE_UP).replace('↓', icons::MOVE_DOWN);
        button = button.shortcut_text(RichText::new(keys).size(12.0).color(DIM));
    }
    ui.add_enabled(enabled, button).clicked()
}

/// An on / off entry: a tick replaces the icon while it is on.
pub fn toggle(ui: &mut egui::Ui, on: bool, icon: &str, label: &str, keys: &str) -> bool {
    item(ui, if on { icons::CHECK } else { icon }, label, keys)
}

/// Small grey heading inside a menu.
pub fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(RichText::new(text).size(11.0).color(DIM));
}

/// Playback, key and tempo, the panels and full screen: right click on
/// the stage or the bottom bar.
pub fn app_menu(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    playback_items(app, ui);
    ui.menu_button(format!("{}   คีย์และความเร็ว", icons::KEY), |ui| {
        let key = app.synth.key();
        if item(ui, icons::MINUS, "ลดคีย์", "[") {
            app.synth.set_key(key - 1);
        }
        if item(ui, icons::PLUS, "เพิ่มคีย์", "]") {
            app.synth.set_key(key + 1);
        }
        if item_if(ui, key != 0, icons::UNDO, "คีย์เดิมของเพลง", "") {
            app.synth.set_key(0);
        }
        ui.separator();
        let speed = app.synth.speed();
        if item(ui, icons::MINUS, "ช้าลง", ",") {
            app.synth.set_speed(speed - 0.05);
        }
        if item(ui, icons::PLUS, "เร็วขึ้น", ".") {
            app.synth.set_speed(speed + 0.05);
        }
        if item_if(ui, (speed - 1.0).abs() > 1e-3, icons::UNDO, "ความเร็วเดิม (100%)", "") {
            app.synth.set_speed(1.0);
        }
    });
    if let Some(uid) = app.now.as_ref().map(|n| n.entry.uid.clone()) {
        let fav = app.library.is_favorite(&uid);
        let (icon, label) = if fav { (icons::STAR_OFF, "เอาออกจากเพลงโปรด") } else { (icons::STAR, "เพิ่มในเพลงโปรด") };
        if item(ui, icon, label, "")
            && let Err(e) = app.library.toggle_favorite(&uid)
        {
            app.toast_error(e);
        }
    }
    let melody_off = app.synth.melody_off();
    if toggle(ui, melody_off, icons::MIC_OFF, "ปิดเมโลดี้ร้องนำ (ช่อง 9)", "V") {
        app.toggle_melody();
    }
    ui.menu_button(format!("{}   รูปแบบเนื้อร้อง", icons::TYPE), |ui| {
        for (mode, icon) in [(LyricMode::Scroll, icons::LYRICS_SCROLL), (LyricMode::Classic, icons::LYRICS_CLASSIC)] {
            let on = app.settings.lyric_mode == mode;
            if toggle(ui, on, icon, mode.label(), "L") && !on {
                app.toggle_lyric_mode();
            }
        }
        ui.separator();
        let slideshow = matches!(app.settings.background.source, crate::config::BgSource::Folder(_));
        if item_if(ui, slideshow, icons::SHUFFLE, "รูปพื้นหลังถัดไป", "B") {
            app.next_background();
        }
        if toggle(ui, app.settings.show_clock, icons::CLOCK, "แสดงนาฬิกา", "") {
            app.settings.show_clock = !app.settings.show_clock;
        }
        if toggle(ui, app.settings.show_beats, icons::METRONOME, "จังหวะ 4 จุด", "") {
            app.settings.show_beats = !app.settings.show_beats;
        }
    });
    ui.separator();
    if item(ui, icons::SEARCH, "ค้นหาเพลง", "/") {
        app.open(Page::Songs);
    }
    if item(ui, icons::FILE_MUSIC, "เปิดไฟล์เพลง (.sfkar / MIDI / KAR)…", "Ctrl O") {
        app.ask_open_file();
    }
    let queue = if app.queue.is_empty() { "คิวเพลง".to_string() } else { format!("คิวเพลง ({})", app.queue.len()) };
    if item(ui, icons::QUEUE, &queue, "Q") {
        app.open(Page::Queue);
    }
    if toggle(ui, app.mixer_open, icons::MIXER, "มิกเซอร์", "M") {
        app.mixer_open = !app.mixer_open;
    }

    if item(ui, icons::FILE_MUSIC, "เสียงและ SoundFont", "S") {
        app.open_sound();
    }
    if item(ui, icons::COMMAND, "คำสั่งทั้งหมด", "Ctrl K") {
        app.open(Page::Commands);
    }
    if item(ui, icons::SETTINGS, "ตั้งค่า", "Ctrl ,") {
        app.open(Page::Settings);
    }
    if item(ui, icons::INFO, "เกี่ยวกับ", "") {
        app.open(Page::About);
    }
    ui.separator();
    let (icon, label) = if app.fullscreen { (icons::EXIT_FULLSCREEN, "ออกจากเต็มจอ") } else { (icons::FULLSCREEN, "เต็มจอ") };
    if item(ui, icon, label, "F") {
        app.set_fullscreen(&ctx, !app.fullscreen);
    }
    if toggle(ui, app.settings.second_screen.open, icons::SECOND_SCREEN, "จอที่สอง (เนื้อร้องอย่างเดียว)", "D") {
        app.toggle_second_screen();
    }
}

/// The song's title, play / pause, stop, restart and next.
pub fn playback_items(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    let loaded = app.synth.state() != PlayState::Empty;
    if let Some(now) = &app.now {
        heading(ui, &now.entry.title);
    }
    let playing = app.synth.state() == PlayState::Playing;
    let (icon, label) = if playing { (icons::PAUSE, "พัก") } else { (icons::PLAY, "เล่น") };
    if item_if(ui, loaded, icon, label, "Space") {
        app.synth.toggle();
    }
    let stopped = app.synth.state() == PlayState::Stopped;
    if item_if(ui, loaded && !stopped, icons::STOP, "หยุด (กลับไปต้นเพลง)", "Shift Space") {
        app.stop();
    }
    if item_if(ui, loaded, icons::RESTART, "เริ่มเพลงนี้ใหม่", "") {
        app.synth.seek(0.0);
        app.synth.play();
        if let Some(n) = &mut app.now {
            n.finished = false;
        }
    }
    let next = app.queue.front().map(|s| format!("เพลงถัดไป: {}", s.title));
    if item_if(ui, next.is_some(), icons::NEXT, next.as_deref().unwrap_or("เพลงถัดไป (คิวว่าง)"), "N") {
        app.play_next();
    }
}
