//! The NCN song library: scanned on a background thread, searched as you type.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossbeam_channel::Receiver;
use solfege_ncnparser::{NcnLibrary, NcnSong, SongHeader};

type Scan = Result<(NcnLibrary, Vec<SongHeader>), String>;

pub struct Library {
    pub root: Option<PathBuf>,
    lib: Option<Arc<NcnLibrary>>,
    pub songs: Vec<SongHeader>,
    /// Lower-cased "id title artist" per song, for search.
    haystack: Vec<String>,
    job: Option<Receiver<Scan>>,
    pub error: Option<String>,
    pub query: String,
    /// Indices into `songs` matching `query`.
    pub results: Vec<usize>,
}

impl Library {
    pub fn new() -> Self {
        Self {
            root: None,
            lib: None,
            songs: Vec::new(),
            haystack: Vec::new(),
            job: None,
            error: None,
            query: String::new(),
            results: Vec::new(),
        }
    }

    pub fn open(&mut self, root: PathBuf) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let dir = root.clone();
        std::thread::spawn(move || {
            let r = NcnLibrary::open(&dir).map_err(|e| e.to_string()).map(|lib| {
                // Only songs with all three files can be sung.
                let headers = lib.headers().into_iter().filter(|h| lib.get(&h.id).is_some_and(|e| e.is_complete())).collect();
                (lib, headers)
            });
            let _ = tx.send(r);
        });
        self.root = Some(root);
        self.job = Some(rx);
        self.error = None;
    }

    pub fn scanning(&self) -> bool {
        self.job.is_some()
    }

    /// Finish a background scan. Returns true when the song list changed.
    pub fn poll(&mut self) -> bool {
        let Some(job) = &self.job else { return false };
        let Ok(result) = job.try_recv() else { return false };
        self.job = None;
        match result {
            Ok((lib, songs)) => {
                self.haystack = songs.iter().map(|h| format!("{} {} {}", h.id, h.title, h.artist).to_lowercase()).collect();
                self.songs = songs;
                self.lib = Some(Arc::new(lib));
                self.error = None;
            }
            Err(e) => {
                self.error = Some(e);
                self.lib = None;
                self.songs.clear();
                self.haystack.clear();
            }
        }
        self.search();
        true
    }

    /// Recompute `results` for `query`: every word must appear somewhere
    /// in the id, title or artist.
    pub fn search(&mut self) {
        let q = self.query.to_lowercase();
        let words: Vec<&str> = q.split_whitespace().collect();
        self.results = (0..self.songs.len()).filter(|&i| words.iter().all(|w| self.haystack[i].contains(w))).collect();
    }

    pub fn load(&self, id: &str) -> Result<NcnSong, String> {
        let lib = self.lib.as_ref().ok_or("no library open")?;
        lib.load(id).map_err(|e| e.to_string())
    }
}

/// `shared/NCN` next to the working directory or any ancestor of the exe.
pub fn find_default_root() -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from("shared/NCN")];
    if let Ok(exe) = std::env::current_exe() {
        candidates.extend(exe.ancestors().skip(1).map(|a| a.join("shared/NCN")));
    }
    candidates.into_iter().find(|c| c.is_dir())
}

/// A SoundFont in the usual places: `shared/`, next to the exe, or where
/// Linux distributions install General MIDI banks.
pub fn find_soundfont() -> Option<PathBuf> {
    let mut dirs = vec![PathBuf::from("shared"), PathBuf::from("shared/soundfonts")];
    if let Ok(exe) = std::env::current_exe() {
        for a in exe.ancestors().skip(1) {
            dirs.push(a.to_path_buf());
            dirs.push(a.join("shared"));
            dirs.push(a.join("shared/soundfonts"));
        }
    }
    dirs.extend(["/usr/share/sounds/sf2", "/usr/share/soundfonts", "/usr/local/share/soundfonts"].map(PathBuf::from));
    dirs.iter().find_map(|d| first_sf2(d))
}

fn first_sf2(dir: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("sf2")))
        .collect();
    found.sort();
    found.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_and_searches_the_sample_library() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shared/NCN");
        if !root.is_dir() {
            eprintln!("skipped: {} missing", root.display());
            return;
        }
        let mut lib = Library::new();
        lib.open(root);
        let t0 = std::time::Instant::now();
        while lib.scanning() && t0.elapsed().as_secs() < 30 {
            lib.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(lib.songs.len(), 139);
        assert_eq!(lib.results.len(), 139);
        lib.query = "ลำไย".into();
        lib.search();
        assert!(lib.results.iter().any(|&i| lib.songs[i].id == "Z2608001"));
        lib.query = "z2608001".into();
        lib.search();
        assert_eq!(lib.results.len(), 1);
        lib.query = "ลำไย zzzz".into();
        lib.search();
        assert!(lib.results.is_empty());
        let song = lib.load("Z2608001").unwrap();
        assert!(!song.lines.is_empty());
    }
}
