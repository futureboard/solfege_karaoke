//! File picker for .wav / .sfz / .sf2. Type to filter, Enter to open,
//! Backspace on an empty filter goes to the parent directory.

use std::path::{Path, PathBuf};

use crate::instrument;

pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BrowseKind {
    Instrument,
    Song,
}

impl BrowseKind {
    fn accepts(self, path: &Path) -> bool {
        match self {
            BrowseKind::Instrument => instrument::is_supported(path),
            BrowseKind::Song => crate::smf::is_midi(path),
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            BrowseKind::Instrument => ".wav .sfz .sf2",
            BrowseKind::Song => ".mid .midi .kar .rmi",
        }
    }
}

pub struct Browser {
    pub kind: BrowseKind,
    /// `None` = drive list (Windows root).
    pub dir: Option<PathBuf>,
    all: Vec<Entry>,
    pub visible: Vec<usize>,
    pub selected: usize,
    pub filter: String,
    pub error: Option<String>,
}

impl Browser {
    pub fn new(start: PathBuf, kind: BrowseKind) -> Self {
        let mut b = Self { kind, dir: Some(start), all: Vec::new(), visible: Vec::new(), selected: 0, filter: String::new(), error: None };
        b.refresh();
        b
    }

    pub fn entry(&self, i: usize) -> Option<&Entry> {
        self.visible.get(i).and_then(|&j| self.all.get(j))
    }

    pub fn title(&self) -> String {
        match &self.dir {
            Some(d) => d.display().to_string(),
            None => "Drives".into(),
        }
    }

    pub fn refresh(&mut self) {
        self.all.clear();
        self.error = None;
        match &self.dir {
            None => {
                for letter in b'A'..=b'Z' {
                    let root = PathBuf::from(format!("{}:\\", letter as char));
                    if root.exists() {
                        self.all.push(Entry { name: root.display().to_string(), path: root, is_dir: true });
                    }
                }
            }
            Some(dir) => {
                self.all.push(Entry { name: "..".into(), path: dir.join(".."), is_dir: true });
                match std::fs::read_dir(dir) {
                    Ok(rd) => {
                        let mut dirs = Vec::new();
                        let mut files = Vec::new();
                        for e in rd.flatten() {
                            let path = e.path();
                            let name = e.file_name().to_string_lossy().into_owned();
                            if name.starts_with('.') {
                                continue;
                            }
                            let is_dir = path.is_dir();
                            if is_dir {
                                dirs.push(Entry { name, path, is_dir });
                            } else if self.kind.accepts(&path) {
                                files.push(Entry { name, path, is_dir });
                            }
                        }
                        dirs.sort_by_key(|e| e.name.to_lowercase());
                        files.sort_by_key(|e| e.name.to_lowercase());
                        self.all.extend(dirs);
                        self.all.extend(files);
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
        }
        self.apply_filter();
    }

    fn apply_filter(&mut self) {
        let f = self.filter.to_lowercase();
        self.visible = (0..self.all.len())
            .filter(|&i| f.is_empty() || self.all[i].name.to_lowercase().contains(&f))
            .collect();
        self.selected = self.selected.min(self.visible.len().saturating_sub(1));
    }

    pub fn push_filter(&mut self, c: char) {
        self.filter.push(c);
        self.selected = 0;
        self.apply_filter();
    }

    /// Backspace: shrink the filter, or go up a directory when it is empty.
    pub fn backspace(&mut self) {
        if self.filter.pop().is_some() {
            self.apply_filter();
        } else {
            self.up();
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.visible.is_empty() {
            return;
        }
        let max = self.visible.len() as isize - 1;
        self.selected = (self.selected as isize + delta).clamp(0, max) as usize;
    }

    pub fn up(&mut self) {
        let Some(dir) = self.dir.clone() else { return };
        let prev = dir.file_name().map(|n| n.to_string_lossy().into_owned());
        self.dir = match dir.parent() {
            Some(p) if p != Path::new("") => Some(p.to_path_buf()),
            _ if cfg!(windows) => None,
            _ => Some(dir.clone()),
        };
        self.filter.clear();
        self.refresh();
        let target = prev.unwrap_or_else(|| dir.display().to_string());
        if let Some(i) = self.visible.iter().position(|&j| self.all[j].name == target || self.all[j].path == dir) {
            self.selected = i;
        }
    }

    /// Enter: descend into a directory, or return the chosen file.
    pub fn activate(&mut self) -> Option<PathBuf> {
        let e = self.entry(self.selected)?;
        if e.name == ".." {
            self.up();
            return None;
        }
        if e.is_dir {
            let p = e.path.clone();
            self.dir = Some(std::fs::canonicalize(&p).map(strip_verbatim).unwrap_or(p));
            self.filter.clear();
            self.selected = 0;
            self.refresh();
            None
        } else {
            Some(e.path.clone())
        }
    }
}

/// `canonicalize` on Windows yields `\\?\D:\...`; keep paths readable.
pub fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => p,
    }
}
