//! Sounds: the SoundFont rack and which font (and, if pinned, which
//! sound) every MIDI channel plays.

use std::path::PathBuf;

use eframe::egui::{self, Margin, RichText};

use super::Target;
use crate::app::KaraokeApp;
use crate::icons;
use crate::style::{ACCENT, DANGER, DIM, SUNG, TEXT};
use crate::synth::{DRUM_CH, MAX_FONTS};

/// Returns a browse request when "add…" was pressed.
pub fn show(app: &mut KaraokeApp, ui: &mut egui::Ui, max_h: f32) -> Option<(Target, Option<PathBuf>)> {
    let mut browse = None;
    egui::ScrollArea::vertical().max_height(max_h + 60.0).auto_shrink([false, true]).show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
            heading(ui, icons::FILE_MUSIC, "SoundFont", "ไฟล์แรกเล่นทุกแชนแนลจนกว่าจะเลือกให้แชนแนลใช้ไฟล์อื่น");
            fonts(app, ui);
            ui.horizontal(|ui| {
                let full = app.synth.fonts().len() >= MAX_FONTS;
                let add = ui.add_enabled(!full, egui::Button::new(format!("{}  เพิ่ม SoundFont / SFZ…", icons::FOLDER_PLUS)));
                if add.clicked() {
                    let start = app.synth.fonts().last().map(|f| f.path.clone());
                    browse = Some((Target::SoundFont, start));
                }
                if app.synth.loading_soundfont() {
                    ui.spinner();
                    ui.label(RichText::new("กำลังโหลด…").color(DIM));
                }
            });
            ui.add_space(16.0);
            heading(ui, icons::MIXER, "แชนแนล", "เลือก SoundFont ของแต่ละแชนแนล และปักเสียงแทนที่เพลงเลือก (ปักไว้เฉพาะเพลงนี้)");
            channels(app, ui);
        });
    });
    browse
}

fn heading(ui: &mut egui::Ui, icon: &str, title: &str, hint: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{icon}  {title}")).strong().color(TEXT));
        ui.label(RichText::new(hint).size(12.0).color(DIM));
    });
}

fn fonts(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    let mut remove = None;
    let mut all = None;
    if app.synth.fonts().is_empty() {
        ui.label(RichText::new("ยังไม่มี SoundFont — เพิ่มไฟล์ .sf2 (General MIDI) เพื่อให้มีเสียงดนตรี").color(DANGER));
    }
    let routing = app.synth.routing();
    for (i, f) in app.synth.fonts().iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{}", i + 1)).strong().color(ACCENT));
            ui.label(RichText::new(f.name()).strong().color(TEXT));
            if f.loading() {
                ui.spinner();
            } else if let Some(e) = &f.error {
                ui.label(RichText::new("โหลดไม่ได้").color(DANGER)).on_hover_text(e);
            } else if let Some(inst) = &f.inst {
                let channels = routing.iter().filter(|&&r| r == i).count();
                ui.label(RichText::new(format!("{} เสียง  ·  {channels} แชนแนล", inst.presets.len())).size(12.0).color(DIM));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(icons::REMOVE).on_hover_text("เอาออกจาก rack").clicked() {
                    remove = Some(i);
                }
                if f.inst.is_some() && ui.small_button("ใช้กับทุกแชนแนล").clicked() {
                    all = Some(i);
                }
                ui.add(egui::Label::new(RichText::new(f.path.display().to_string()).monospace().size(11.0).color(DIM)).truncate());
            });
        });
    }
    if let Some(i) = all {
        app.synth.route_all(i);
    }
    if let Some(i) = remove {
        app.synth.remove_font(i);
    }
}

fn channels(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    let used = app.synth.channels_used();
    let list: Vec<usize> = if used == 0 { (0..16).collect() } else { (0..16).filter(|&c| used & (1 << c) != 0).collect() };
    if used == 0 {
        ui.label(RichText::new("ยังไม่มีเพลง — แสดงทั้ง 16 แชนแนล").size(12.0).color(DIM));
    }
    let names: Vec<(usize, String, bool)> =
        app.synth.fonts().iter().enumerate().map(|(i, f)| (i, format!("{}  {}", i + 1, f.name()), f.inst.is_some())).collect();
    egui::Grid::new("sound-routing").num_columns(4).spacing([14.0, 6.0]).striped(false).show(ui, |ui| {
        for h in ["แชนแนล", "กำลังเล่น", "SoundFont", "ปักเสียง"] {
            ui.label(RichText::new(h).size(12.0).color(DIM));
        }
        ui.end_row();
        for ch in list {
            let label = if ch == DRUM_CH { format!("{}  10", icons::DRUM) } else { (ch + 1).to_string() };
            ui.label(RichText::new(label).strong().color(TEXT));
            let playing = app.synth.channel_sound(ch).unwrap_or("—").to_string();
            let pinned = app.synth.pin(ch).is_some();
            ui.horizontal(|ui| {
                ui.set_width(200.0);
                ui.add(egui::Label::new(RichText::new(playing).color(if pinned { SUNG } else { TEXT })).truncate());
            });

            // Font choice.
            let routing = app.synth.routing();
            let mut font = routing[ch];
            let current = names.get(font).map_or("—".to_string(), |n| n.1.clone());
            egui::ComboBox::from_id_salt(("font", ch)).selected_text(current).width(200.0).show_ui(ui, |ui| {
                for (i, name, ready) in &names {
                    ui.add_enabled_ui(*ready, |ui| ui.selectable_value(&mut font, *i, name));
                }
            });
            if font != routing[ch] {
                app.synth.set_route(ch, font);
            }

            // Pinned sound from that font.
            let mut pin = app.synth.pin(ch);
            let presets: Vec<(usize, String)> = app
                .synth
                .channel_font(ch)
                .map(|f| {
                    f.presets
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| (ch == DRUM_CH) == (p.bank == 128) || f.presets.iter().all(|q| q.bank != 128))
                        .map(|(i, p)| (i, format!("{:03}:{:03}  {}", p.bank, p.program, p.name)))
                        .collect()
                })
                .unwrap_or_default();
            let shown = pin.and_then(|p| presets.iter().find(|(i, _)| *i == p)).map_or("ตามเพลง".to_string(), |(_, n)| n.clone());
            egui::ComboBox::from_id_salt(("pin", ch)).selected_text(shown).width(240.0).height(320.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut pin, None, "ตามเพลง");
                for (i, name) in &presets {
                    ui.selectable_value(&mut pin, Some(*i), name);
                }
            });
            if pin != app.synth.pin(ch) {
                app.synth.set_pin(ch, pin);
            }
            ui.end_row();
        }
    });
}
