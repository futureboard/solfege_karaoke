//! Folder / file browser inside the overlay, for the library folder and
//! the SoundFont, so the app needs no native dialog library.

use std::path::{Path, PathBuf};

use eframe::egui::Color32;

use crate::icons;
use crate::style::{ACCENT, TEXT};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// A folder holding Song, Lyrics and Cursor.
    Library,
    /// An .sf2 / .sfz file.
    SoundFont,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Entry {
    /// Pick the folder being shown (library only).
    UseThis,
    Up,
    Dir(usize),
    File(usize),
}

pub struct Browse {
    pub target: Target,
    dir: PathBuf,
    dirs: Vec<PathBuf>,
    files: Vec<PathBuf>,
    error: Option<String>,
}

const SOUNDFONT_EXTS: [&str; 2] = ["sf2", "sfz"];

impl Browse {
    pub fn new(target: Target, start: Option<PathBuf>) -> Self {
        let start = start
            .map(|p| if p.is_file() { p.parent().map(Path::to_path_buf).unwrap_or(p) } else { p })
            .filter(|p| p.is_dir())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let mut b = Self { target, dir: PathBuf::new(), dirs: Vec::new(), files: Vec::new(), error: None };
        b.go(start);
        b
    }

    pub fn go(&mut self, dir: PathBuf) {
        let dir = clean(dir);
        match std::fs::read_dir(&dir) {
            Ok(rd) => {
                let mut dirs = Vec::new();
                let mut files = Vec::new();
                for e in rd.flatten() {
                    let p = e.path();
                    if p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.')) {
                        continue;
                    }
                    if p.is_dir() {
                        dirs.push(p);
                    } else if self.target == Target::SoundFont
                        && p.extension().and_then(|e| e.to_str()).is_some_and(|e| SOUNDFONT_EXTS.contains(&e.to_ascii_lowercase().as_str()))
                    {
                        files.push(p);
                    }
                }
                dirs.sort_by_key(|p| p.file_name().map(|n| n.to_ascii_lowercase()));
                files.sort_by_key(|p| p.file_name().map(|n| n.to_ascii_lowercase()));
                self.dir = dir;
                self.dirs = dirs;
                self.files = files;
                self.error = None;
            }
            Err(e) => self.error = Some(format!("{}: {e}", dir.display())),
        }
    }

    pub fn dir_text(&self) -> String {
        match &self.error {
            Some(e) => e.clone(),
            None => self.dir.display().to_string(),
        }
    }

    fn looks_like_library(&self) -> bool {
        ["Song", "Lyrics", "Cursor"].iter().all(|n| self.dirs.iter().any(|d| d.file_name().is_some_and(|f| f.eq_ignore_ascii_case(n))))
    }

    fn name(p: &Path) -> String {
        p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// Entries whose name contains `query` (case-insensitive).
    pub fn entries(&self, query: &str) -> Vec<Entry> {
        let q = query.trim().to_lowercase();
        let hit = |p: &Path| q.is_empty() || Self::name(p).to_lowercase().contains(&q);
        let mut out = Vec::new();
        if q.is_empty() {
            if self.target == Target::Library {
                out.push(Entry::UseThis);
            }
            if self.dir.parent().is_some() {
                out.push(Entry::Up);
            }
        }
        out.extend((0..self.dirs.len()).filter(|&i| hit(&self.dirs[i])).map(Entry::Dir));
        out.extend((0..self.files.len()).filter(|&i| hit(&self.files[i])).map(Entry::File));
        out
    }

    /// Icon, name, right-hand note and text colour of an entry.
    pub fn describe(&self, e: Entry) -> (&'static str, String, String, Color32) {
        match e {
            Entry::UseThis => {
                let note = if self.looks_like_library() { "พบ Song / Lyrics / Cursor" } else { "" };
                (icons::CHECK, "ใช้โฟลเดอร์นี้".into(), note.into(), ACCENT)
            }
            Entry::Up => (icons::FOLDER_UP, "..".into(), String::new(), TEXT),
            Entry::Dir(i) => (icons::FOLDER, Self::name(&self.dirs[i]), String::new(), TEXT),
            Entry::File(i) => {
                let size = std::fs::metadata(&self.files[i]).map(|m| m.len()).unwrap_or(0);
                (icons::FILE_MUSIC, Self::name(&self.files[i]), format!("{:.1} MB", size as f64 / 1e6), TEXT)
            }
        }
    }

    /// Enter on an entry: a chosen path, or `None` after navigating.
    pub fn activate(&mut self, e: Entry) -> Option<PathBuf> {
        match e {
            Entry::UseThis => Some(self.dir.clone()),
            Entry::Up => {
                if let Some(p) = self.dir.parent().map(Path::to_path_buf) {
                    self.go(p);
                }
                None
            }
            Entry::Dir(i) => {
                self.go(self.dirs[i].clone());
                None
            }
            Entry::File(i) => Some(self.files[i].clone()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browses_and_picks() {
        let root = std::env::temp_dir().join(format!("karaoke-browse-{}", std::process::id()));
        std::fs::create_dir_all(root.join("Fonts")).unwrap();
        std::fs::write(root.join("Fonts/gm.SF2"), b"x").unwrap();
        std::fs::write(root.join("Fonts/notes.txt"), b"x").unwrap();

        let mut b = Browse::new(Target::SoundFont, Some(root.clone()));
        let e = b.entries("");
        assert_eq!(e, [Entry::Up, Entry::Dir(0)]);
        assert_eq!(b.activate(Entry::Dir(0)), None);
        // Only SoundFonts are listed; extension case does not matter.
        assert_eq!(b.entries(""), [Entry::Up, Entry::File(0)]);
        assert!(b.activate(Entry::File(0)).unwrap().ends_with("gm.SF2"));
        assert!(b.entries("zzz").is_empty());

        let mut lib = Browse::new(Target::Library, Some(root.clone()));
        assert_eq!(lib.entries("")[0], Entry::UseThis);
        assert_eq!(lib.entries("fon"), [Entry::Dir(0)]);
        assert_eq!(lib.activate(Entry::UseThis), Some(clean(root.clone())));
        std::fs::remove_dir_all(root).ok();
    }
}
