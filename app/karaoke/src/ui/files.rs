//! A small built-in file / folder chooser, so the app needs no native
//! dialog library on any platform.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};

use crate::app::{KaraokeApp, Picking};
use crate::style::{ACCENT, DIM};

#[derive(Clone, Copy)]
pub enum Mode {
    Folder,
    /// Files with one of these extensions (lower case).
    File(&'static [&'static str]),
}

pub struct FilePicker {
    mode: Mode,
    dir: PathBuf,
    typed: String,
    dirs: Vec<PathBuf>,
    files: Vec<PathBuf>,
    error: Option<String>,
}

impl FilePicker {
    pub fn new(mode: Mode, start: Option<PathBuf>) -> Self {
        let start = start
            .map(|p| if p.is_file() { p.parent().map(Path::to_path_buf).unwrap_or(p) } else { p })
            .filter(|p| p.is_dir())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let mut p = Self { mode, dir: PathBuf::new(), typed: String::new(), dirs: Vec::new(), files: Vec::new(), error: None };
        p.go(start);
        p
    }

    fn go(&mut self, dir: PathBuf) {
        let dir = clean(dir);
        match std::fs::read_dir(&dir) {
            Ok(rd) => {
                let mut dirs = Vec::new();
                let mut files = Vec::new();
                for e in rd.flatten() {
                    let p = e.path();
                    let hidden = p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'));
                    if hidden {
                        continue;
                    }
                    if p.is_dir() {
                        dirs.push(p);
                    } else if let Mode::File(exts) = self.mode
                        && p.extension().and_then(|e| e.to_str()).is_some_and(|e| exts.contains(&e.to_ascii_lowercase().as_str()))
                    {
                        files.push(p);
                    }
                }
                dirs.sort_by_key(|p| p.file_name().map(|n| n.to_ascii_lowercase()));
                files.sort_by_key(|p| p.file_name().map(|n| n.to_ascii_lowercase()));
                self.typed = dir.display().to_string();
                self.dir = dir;
                self.dirs = dirs;
                self.files = files;
                self.error = None;
            }
            Err(e) => self.error = Some(format!("{}: {e}", dir.display())),
        }
    }
}

/// Absolute path without `..`, and without the `\\?\` prefix Windows adds.
fn clean(dir: PathBuf) -> PathBuf {
    let Ok(c) = dir.canonicalize() else { return dir };
    match c.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => c,
    }
}

pub fn show(app: &mut KaraokeApp, ctx: &egui::Context) {
    let Some((what, picker)) = &mut app.picker else { return };
    let title = match what {
        Picking::Library => "เลือกโฟลเดอร์คลังเพลง NCN",
        Picking::SoundFont => "เลือกไฟล์ SoundFont",
    };
    let mut open = true;
    let mut chosen: Option<PathBuf> = None;
    let mut go: Option<PathBuf> = None;
    egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .default_size([560.0, 460.0])
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("⬆").on_hover_text("โฟลเดอร์แม่").clicked()
                    && let Some(parent) = picker.dir.parent()
                {
                    go = Some(parent.to_path_buf());
                }
                let r = ui.add(egui::TextEdit::singleline(&mut picker.typed).desired_width(f32::INFINITY));
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    go = Some(PathBuf::from(picker.typed.trim()));
                }
            });
            if let Some(e) = &picker.error {
                ui.colored_label(crate::style::DANGER, e);
            }
            ui.separator();
            egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, false]).show(ui, |ui| {
                for d in &picker.dirs {
                    let name = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if ui.selectable_label(false, format!("📁 {name}")).clicked() {
                        go = Some(d.clone());
                    }
                }
                for f in &picker.files {
                    let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let size = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
                    let r = ui.selectable_label(false, format!("🎵 {name}   {:.1} MB", size as f64 / 1e6));
                    if r.clicked() {
                        chosen = Some(f.clone());
                    }
                }
                if picker.dirs.is_empty() && picker.files.is_empty() {
                    ui.label(RichText::new("(ว่าง)").color(DIM));
                }
            });
            if let Mode::Folder = picker.mode {
                ui.separator();
                ui.horizontal(|ui| {
                    let looks_ncn = ["Song", "Lyrics", "Cursor"]
                        .iter()
                        .all(|n| picker.dirs.iter().any(|d| d.file_name().is_some_and(|f| f.eq_ignore_ascii_case(n))));
                    if looks_ncn {
                        ui.label(RichText::new("✔ พบ Song / Lyrics / Cursor").color(ACCENT));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("ใช้โฟลเดอร์นี้").clicked() {
                            chosen = Some(picker.dir.clone());
                        }
                    });
                });
            }
        });
    if let Some(d) = go {
        picker.go(d);
    }
    if let Some(path) = chosen {
        let (what, _) = app.picker.take().expect("picker open");
        match what {
            Picking::Library => app.open_library(path),
            Picking::SoundFont => app.open_soundfont(path),
        }
    } else if !open {
        app.picker = None;
    }
}
