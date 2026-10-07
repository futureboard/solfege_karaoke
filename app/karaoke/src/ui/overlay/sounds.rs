//! Sounds: the SoundFont rack, which font (and, if pinned, which sound)
//! every MIDI channel plays, the locked drum kit, and a sound of choice
//! for any of the 128 General MIDI instruments.

use std::path::PathBuf;

use eframe::egui::{self, Margin, RichText};

use super::Target;
use crate::app::KaraokeApp;
use crate::icons;
use crate::style::{ACCENT, DANGER, DIM, SUNG, TEXT};
use crate::gm;
use crate::synth::{DRUM_CH, InstrumentSound, MAX_FONTS};

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
            heading(ui, icons::MIXER, "แชนแนล", "SoundFont ของแต่ละแชนแนล และปักเสียงแทนที่เพลงเลือก (เฉพาะเพลงนี้)");
            channels(app, ui);
            ui.add_space(16.0);
            heading(
                ui,
                icons::GUITAR,
                "เครื่องดนตรี",
                "เสียงของเครื่องดนตรีแต่ละชิ้น ใช้กับทุกแชนแนลที่เล่นชิ้นนั้น (ยกเว้นแชนแนลที่ปักเสียง)",
            );
            instruments(app, ui);
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
        for h in ["แชนแนล", "กำลังเล่น", "SoundFont", "ปักเสียง / ล็อกกลอง"] {
            ui.label(RichText::new(h).size(12.0).color(DIM));
        }
        ui.end_row();
        for ch in list {
            let label = if ch == DRUM_CH { format!("{}  10", icons::DRUM) } else { (ch + 1).to_string() };
            ui.label(RichText::new(label).strong().color(TEXT));
            let mut playing = app.synth.channel_sound(ch).unwrap_or("—").to_string();
            // Say so when an instrument override plays it from another font.
            if let Some(f) = app.synth.sounding_font(ch).filter(|&f| f != app.synth.routing()[ch]) {
                playing.push_str(&format!("  · SF{}", f + 1));
            }
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
            // On channel 10 the pin is the drum-kit lock, kept for every song.
            let (free, lock) = if ch == DRUM_CH { ("ไม่ล็อก (ตามเพลง)", format!("{}  ", icons::LOCK)) } else { ("ตามเพลง", String::new()) };
            let shown = pin.and_then(|p| presets.iter().find(|(i, _)| *i == p)).map_or(free.to_string(), |(_, n)| format!("{lock}{n}"));
            let combo = egui::ComboBox::from_id_salt(("pin", ch)).selected_text(shown).width(240.0).height(320.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut pin, None, free);
                for (i, name) in &presets {
                    ui.selectable_value(&mut pin, Some(*i), name);
                }
            });
            if ch == DRUM_CH {
                combo.response.on_hover_text("ล็อกชุดกลองไว้ทุกเพลง เพลงเปลี่ยนชุดกลองเองไม่ได้");
            }
            if pin != app.synth.pin(ch) {
                app.synth.set_pin(ch, pin);
            }
            ui.end_row();
        }
    });
}

/// The 128 GM instruments in their families, each with a sound of choice.
fn instruments(app: &mut KaraokeApp, ui: &mut egui::Ui) {
    let fonts: Vec<(usize, String)> = app
        .synth
        .fonts()
        .iter()
        .enumerate()
        .filter(|(_, f)| f.inst.is_some())
        .map(|(i, f)| (i, format!("{}  {}", i + 1, f.name())))
        .collect();
    if fonts.is_empty() {
        ui.label(RichText::new("เพิ่ม SoundFont ก่อน").color(DIM));
        return;
    }
    let chosen = app.synth.instruments().clone();
    let mut change: Option<(u8, Option<InstrumentSound>)> = None;
    for (family, name) in gm::FAMILIES.iter().enumerate() {
        let programs = (family * 8) as u8..(family * 8 + 8) as u8;
        let set = programs.clone().filter(|p| chosen.contains_key(p)).count();
        let title = if set > 0 { format!("{name}   ·   เปลี่ยน {set}") } else { (*name).to_string() };
        egui::CollapsingHeader::new(RichText::new(title).color(if set > 0 { SUNG } else { TEXT }))
            .id_salt(("gm-family", family))
            .show(ui, |ui| {
                egui::Grid::new(("gm-grid", family)).num_columns(3).spacing([14.0, 6.0]).show(ui, |ui| {
                    for program in programs {
                        ui.label(RichText::new(format!("{:03}  {}", program + 1, gm::INSTRUMENTS[program as usize])).color(TEXT));
                        let current = chosen.get(&program).copied();
                        // Font: "per channel" or one of the fonts.
                        let mut font = current.map(|s| s.font);
                        let label = font.and_then(|f| fonts.iter().find(|(i, _)| *i == f)).map_or("ตามแชนแนล".to_string(), |(_, n)| n.clone());
                        egui::ComboBox::from_id_salt(("gm-font", program)).selected_text(label).width(200.0).show_ui(ui, |ui| {
                            ui.selectable_value(&mut font, None, "ตามแชนแนล");
                            for (i, n) in &fonts {
                                ui.selectable_value(&mut font, Some(*i), n);
                            }
                        });
                        if font != current.map(|s| s.font) {
                            change = Some((program, font.and_then(|f| default_sound(app, f, program))));
                        }
                        // Sound within that font.
                        match current {
                            Some(sound) => {
                                let presets = melodic_presets(app, sound.font);
                                let mut pick = (sound.bank, sound.program);
                                let shown = presets.iter().find(|p| (p.0, p.1) == pick).map_or("—".to_string(), |p| p.2.clone());
                                egui::ComboBox::from_id_salt(("gm-preset", program)).selected_text(shown).width(240.0).height(320.0).show_ui(ui, |ui| {
                                    for (bank, prog, name) in &presets {
                                        ui.selectable_value(&mut pick, (*bank, *prog), name);
                                    }
                                });
                                if pick != (sound.bank, sound.program) {
                                    change = Some((program, Some(InstrumentSound { font: sound.font, bank: pick.0, program: pick.1 })));
                                }
                            }
                            None => {
                                ui.label("");
                            }
                        }
                        ui.end_row();
                    }
                });
            });
    }
    if let Some((program, sound)) = change {
        app.synth.set_instrument(program, sound);
    }
}

/// Non-drum presets of a font as (bank, program, label).
fn melodic_presets(app: &KaraokeApp, font: usize) -> Vec<(u16, u8, String)> {
    let Some(inst) = app.synth.fonts().get(font).and_then(|f| f.inst.as_ref()) else { return Vec::new() };
    let mut out: Vec<(u16, u8, String)> =
        inst.presets.iter().filter(|p| p.bank != 128).map(|p| (p.bank, p.program, format!("{:03}:{:03}  {}", p.bank, p.program, p.name))).collect();
    out.sort();
    out
}

/// The sound a font offers for a GM program: the same program in bank 0,
/// else its first melodic preset.
fn default_sound(app: &KaraokeApp, font: usize, program: u8) -> Option<InstrumentSound> {
    let presets = melodic_presets(app, font);
    let (bank, prog, _) = presets.iter().find(|p| p.0 == 0 && p.1 == program).or(presets.first())?;
    Some(InstrumentSound { font, bank: *bank, program: *prog })
}
