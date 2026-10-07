//! The song catalogue as the app uses it: `songs.dat` (SQLite, see
//! `solfege_songdb`) in the app's data folder, rescanned on a background
//! thread, searched as you type, saved when it changes. A `songs.json`
//! catalogue of older versions is imported on first start.

use std::path::{Path, PathBuf};

use crossbeam_channel::Receiver;
use solfege_sfkar::KarSong;
use solfege_songdb::{Scan, Song, SongDb, SourceKind};

pub struct Library {
    pub db: SongDb,
    /// Catalogue file; `None` keeps everything in memory (tests).
    path: Option<PathBuf>,
    job: Option<Receiver<Scan>>,
    /// The catalogue (sources / songs) changed since it was saved.
    dirty: bool,
    pub query: String,
    /// Indices into `db.songs` matching `query`, best first.
    pub results: Vec<usize>,
}

/// What a finished rescan found.
pub struct ScanReport {
    pub songs: usize,
    pub errors: Vec<String>,
}

/// The catalogue database in the data folder.
pub const CATALOGUE: &str = "songs.dat";
/// The JSON catalogue of older versions.
const OLD_CATALOGUE: &str = "songs.json";

impl Library {
    /// Open the catalogue in `data_dir` (`None` keeps it in memory). A
    /// broken database is reported, kept aside as `songs.dat.bak` and
    /// started afresh.
    pub fn open(data_dir: Option<&Path>) -> (Self, Option<String>) {
        let path = data_dir.map(|d| d.join(CATALOGUE));
        let mut dirty = false;
        let (db, error) = match &path {
            None => (SongDb::default(), None),
            Some(p) if !p.exists() => match SongDb::import_json(&p.with_file_name(OLD_CATALOGUE)) {
                Ok(Some(db)) => {
                    dirty = true;
                    (db, None)
                }
                Ok(None) => (SongDb::default(), None),
                Err(e) => (SongDb::default(), Some(e.to_string())),
            },
            Some(p) => match SongDb::load(p) {
                Ok(db) => (db, None),
                Err(e) => {
                    let _ = std::fs::rename(p, p.with_extension("dat.bak"));
                    (SongDb::default(), Some(e.to_string()))
                }
            },
        };
        let mut lib = Self { db, path, job: None, dirty, query: String::new(), results: Vec::new() };
        lib.search();
        (lib, error)
    }

    pub fn add_source(&mut self, path: PathBuf) -> Result<SourceKind, String> {
        let kind = self.db.add_source(path).map_err(|e| e.to_string())?;
        self.dirty = true;
        self.rescan();
        Ok(kind)
    }

    pub fn remove_source(&mut self, index: usize) {
        self.db.remove_source(index);
        self.dirty = true;
        self.search();
    }

    pub fn rescan(&mut self) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let sources = self.db.sources.clone();
        std::thread::spawn(move || {
            let _ = tx.send(solfege_songdb::scan(&sources));
        });
        self.job = Some(rx);
    }

    /// The catalogue database (`None` = in memory only).
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn scanning(&self) -> bool {
        self.job.is_some()
    }

    /// Take a finished rescan.
    pub fn poll(&mut self) -> Option<ScanReport> {
        let scan = self.job.as_ref()?.try_recv().ok()?;
        self.job = None;
        let errors = scan.errors.clone();
        self.db.apply(scan);
        self.dirty = true;
        self.search();
        Some(ScanReport { songs: self.db.songs.len(), errors })
    }

    pub fn search(&mut self) {
        self.results = self.db.search(&self.query);
    }

    pub fn song(&self, index: usize) -> &Song {
        &self.db.songs[index]
    }

    pub fn find(&self, code: &str) -> Option<&Song> {
        self.db.find(code).map(|i| &self.db.songs[i])
    }

    pub fn is_favorite(&self, uid: &str) -> bool {
        self.db.stats(uid).favorite
    }

    pub fn toggle_favorite(&mut self, uid: &str) -> Result<bool, String> {
        let on = self.db.toggle_favorite(uid);
        if self.query.trim().is_empty() {
            self.search();
        }
        self.save_stats().map(|_| on)
    }

    pub fn record_play(&mut self, uid: &str) -> Result<(), String> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        self.db.record_play(uid, now);
        self.save_stats()
    }

    fn save_stats(&self) -> Result<(), String> {
        match &self.path {
            Some(p) => self.db.save_stats(p).map_err(|e| e.to_string()),
            None => Ok(()),
        }
    }

    /// Write the catalogue if anything changed.
    pub fn save(&mut self) -> Result<(), String> {
        let (Some(path), true) = (&self.path, self.dirty) else { return Ok(()) };
        self.db.save(path).map_err(|e| e.to_string())?;
        self.dirty = false;
        Ok(())
    }
}

pub fn load(song: &Song) -> Result<KarSong, String> {
    solfege_songdb::load_song(song).map_err(|e| e.to_string())
}

/// A `.sfkar` file opened directly, outside the catalogue.
pub fn loose_song(path: &Path) -> Result<Song, String> {
    let meta = KarSong::read_meta(path).map_err(|e| e.to_string())?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let id = meta.id.filter(|id| !id.is_empty()).unwrap_or(stem);
    Ok(Song {
        uid: id.to_uppercase(),
        id,
        title: meta.title,
        artist: meta.artist,
        key: meta.key,
        source: usize::MAX,
        location: solfege_songdb::Location::Sfkar(path.to_path_buf()),
    })
}

/// `shared/NCN` next to the working directory or any ancestor of the exe.
pub fn find_default_root() -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from("shared/NCN")];
    if let Ok(exe) = std::env::current_exe() {
        candidates.extend(exe.ancestors().skip(1).map(|a| a.join("shared/NCN")));
    }
    candidates.into_iter().find(|c| c.is_dir()).map(|c| c.canonicalize().unwrap_or(c))
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
    fn scans_searches_and_loads_the_sample_library() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shared/NCN");
        if !root.is_dir() {
            eprintln!("skipped: {} missing", root.display());
            return;
        }
        let (mut lib, err) = Library::open(None);
        assert!(err.is_none());
        assert_eq!(lib.add_source(root), Ok(SourceKind::Ncn));
        let t0 = std::time::Instant::now();
        let report = loop {
            if let Some(r) = lib.poll() {
                break r;
            }
            assert!(t0.elapsed().as_secs() < 30, "scan timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(report.songs, 139);
        assert!(report.errors.is_empty());
        lib.query = "ลำไย".into();
        lib.search();
        assert!(lib.results.iter().any(|&i| lib.song(i).id == "Z2608001"));
        let song = load(lib.find("z2608001").unwrap()).unwrap();
        assert!(!song.lyrics.lines.is_empty());
        assert_eq!(lib.toggle_favorite("Z2608001"), Ok(true));
        lib.query.clear();
        lib.search();
        assert_eq!(lib.song(lib.results[0]).uid, "Z2608001");
    }
}
