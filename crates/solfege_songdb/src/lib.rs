//! # solfege_songdb
//!
//! The song catalogue: every song from any number of sources (NCN library
//! folders and folders of `.sfkar` files) in one searchable list, with the
//! listener's own data on top: favourites, play counts and when a song was
//! last sung.
//!
//! The catalogue is a JSON file. Opening it is instant; a rescan (which
//! reads every lyric header) runs separately, typically on a background
//! thread via [`scan`], and its result is merged with [`SongDb::apply`].
//! Play history is keyed by song code, so it survives a rescan, moving a
//! library, or converting NCN songs to `.sfkar`. It is kept in a small
//! sibling file (`songs.stats.json` next to `songs.json`) so it can be
//! saved after every song without rewriting the whole catalogue.
//!
//! ```no_run
//! use solfege_songdb::SongDb;
//!
//! let mut db = SongDb::load("songs.json".as_ref())?;
//! db.add_source("shared/NCN".into())?;
//! let scan = solfege_songdb::scan(&db.sources);
//! db.apply(scan);
//! for i in db.search("ลำไย") {
//!     println!("{} {}", db.songs[i].id, db.songs[i].title);
//! }
//! db.save("songs.json".as_ref())?;
//! # Ok::<(), solfege_songdb::Error>(())
//! ```

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use solfege_ncnparser::{Cursor, Lyrics, MidiInfo, NcnLibrary, NcnSong};
use solfege_sfkar::KarSong;

const FORMAT: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    /// A folder with Song, Lyrics and Cursor sub-folders.
    Ncn,
    /// A folder (searched recursively) of `.sfkar` files.
    Sfkar,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub path: PathBuf,
    pub kind: SourceKind,
}

/// Where a song's data lives.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Location {
    Ncn { midi: PathBuf, lyrics: PathBuf, cursor: PathBuf },
    Sfkar(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Song {
    /// Upper-case song code; the key for stats and duplicates.
    pub uid: String,
    /// Song code as written by its source.
    pub id: String,
    pub title: String,
    pub artist: String,
    pub key: Option<String>,
    /// Index into [`SongDb::sources`].
    pub source: usize,
    pub location: Location,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    pub plays: u32,
    /// Unix seconds; 0 = never.
    pub last_played: u64,
    pub favorite: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SongDb {
    format: u32,
    pub sources: Vec<Source>,
    pub songs: Vec<Song>,
    #[serde(skip)]
    pub stats: BTreeMap<String, Stats>,
    /// Lower-cased "id title artist" per song.
    #[serde(skip)]
    haystack: Vec<String>,
}

/// Result of reading the sources; see [`scan`].
#[derive(Debug, Default)]
pub struct Scan {
    pub sources: Vec<Source>,
    pub songs: Vec<Song>,
    /// One message per source or file that could not be read.
    pub errors: Vec<String>,
}

#[derive(Debug)]
pub enum Error {
    Io { path: PathBuf, source: std::io::Error },
    Json { path: PathBuf, source: serde_json::Error },
    /// The folder is neither an NCN library nor holds `.sfkar` files.
    NotASource(PathBuf),
    Song(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Error::Json { path, source } => write!(f, "{}: {source}", path.display()),
            Error::NotASource(p) => write!(f, "{}: no NCN library (Song/Lyrics/Cursor) or .sfkar files", p.display()),
            Error::Song(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

impl SourceKind {
    /// What kind of song folder `path` is, if any.
    pub fn detect(path: &Path) -> Option<Self> {
        if NcnLibrary::open(path).is_ok() {
            Some(SourceKind::Ncn)
        } else if !sfkar_files(path).is_empty() {
            Some(SourceKind::Sfkar)
        } else {
            None
        }
    }
}

impl SongDb {
    /// Open a catalogue file; a missing file is an empty catalogue.
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => return Err(Error::Io { path: path.to_path_buf(), source }),
        };
        let mut db: Self = serde_json::from_slice(&bytes).map_err(|source| Error::Json { path: path.to_path_buf(), source })?;
        let stats = stats_path(path);
        match std::fs::read(&stats) {
            Ok(b) => db.stats = serde_json::from_slice(&b).map_err(|source| Error::Json { path: stats, source })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(Error::Io { path: stats, source }),
        }
        db.reindex();
        Ok(db)
    }

    /// Write the catalogue and the stats next to it.
    pub fn save(&self, path: &Path) -> Result<()> {
        let mut out = self.clone();
        out.format = FORMAT;
        write_json(path, &out)?;
        self.save_stats(path)
    }

    /// Write only the favourites and play history (cheap; call it often).
    pub fn save_stats(&self, catalogue: &Path) -> Result<()> {
        write_json(&stats_path(catalogue), &self.stats)
    }

    fn reindex(&mut self) {
        self.haystack = self.songs.iter().map(|s| format!("{} {} {}", s.id, s.title, s.artist).to_lowercase()).collect();
    }

    /// Add a song folder (kind detected). Songs appear after the next scan.
    pub fn add_source(&mut self, path: PathBuf) -> Result<SourceKind> {
        let kind = SourceKind::detect(&path).ok_or_else(|| Error::NotASource(path.clone()))?;
        if !self.sources.iter().any(|s| s.path == path) {
            self.sources.push(Source { path, kind });
        }
        Ok(kind)
    }

    pub fn remove_source(&mut self, index: usize) {
        if index >= self.sources.len() {
            return;
        }
        self.sources.remove(index);
        self.songs.retain(|s| s.source != index);
        for s in &mut self.songs {
            if s.source > index {
                s.source -= 1;
            }
        }
        self.reindex();
    }

    /// Take the songs of a scan. Sources added or removed since the scan
    /// started are respected: the scan only replaces songs of sources that
    /// are still listed.
    pub fn apply(&mut self, scan: Scan) {
        let mut songs = Vec::with_capacity(scan.songs.len());
        for mut s in scan.songs {
            let Some(path) = scan.sources.get(s.source).map(|src| &src.path) else { continue };
            if let Some(i) = self.sources.iter().position(|src| &src.path == path) {
                s.source = i;
                songs.push(s);
            }
        }
        // Sources the scan did not know keep their previous songs.
        let scanned: HashSet<&PathBuf> = scan.sources.iter().map(|s| &s.path).collect();
        let kept = self.songs.iter().filter(|s| !scanned.contains(&self.sources[s.source].path)).cloned();
        songs.extend(kept.collect::<Vec<_>>());
        songs.sort_by(|a, b| a.source.cmp(&b.source).then_with(|| a.uid.cmp(&b.uid)));
        let mut seen = HashSet::new();
        songs.retain(|s| seen.insert(s.uid.clone()));
        self.songs = songs;
        self.reindex();
    }

    pub fn stats(&self, uid: &str) -> Stats {
        self.stats.get(uid).copied().unwrap_or_default()
    }

    pub fn record_play(&mut self, uid: &str, now_unix: u64) {
        let s = self.stats.entry(uid.to_string()).or_default();
        s.plays += 1;
        s.last_played = now_unix;
    }

    /// Flip the favourite mark; returns the new state.
    pub fn toggle_favorite(&mut self, uid: &str) -> bool {
        let s = self.stats.entry(uid.to_string()).or_default();
        s.favorite = !s.favorite;
        s.favorite
    }

    /// Songs matching every word of `query` (id, title, artist), best
    /// first: exact code, title starting with the query, title containing
    /// it, then artist matches; favourites first within each. With an
    /// empty query: favourites, then recently sung, then the rest.
    pub fn search(&self, query: &str) -> Vec<usize> {
        let q = query.trim().to_lowercase();
        let words: Vec<&str> = q.split_whitespace().collect();
        let mut hits: Vec<(u8, bool, u64, usize)> = Vec::new();
        for (i, s) in self.songs.iter().enumerate() {
            if !words.iter().all(|w| self.haystack[i].contains(w)) {
                continue;
            }
            let st = self.stats(&s.uid);
            let rank = if q.is_empty() {
                if st.favorite { 0 } else if st.last_played > 0 { 1 } else { 2 }
            } else {
                let title = s.title.to_lowercase();
                if s.id.to_lowercase() == q {
                    0
                } else if title.starts_with(&q) {
                    1
                } else if title.contains(&q) {
                    2
                } else if s.artist.to_lowercase().contains(&q) {
                    3
                } else {
                    4
                }
            };
            hits.push((rank, !st.favorite, u64::MAX - st.last_played, i));
        }
        if q.is_empty() {
            hits.sort_by_key(|h| (h.0, h.2, h.3));
        } else {
            hits.sort_by_key(|h| (h.0, h.1, h.3));
        }
        hits.into_iter().map(|h| h.3).collect()
    }

    pub fn find(&self, code: &str) -> Option<usize> {
        let uid = code.to_uppercase();
        self.songs.iter().position(|s| s.uid == uid)
    }

    /// Read a song, whatever its source format.
    pub fn load_song(&self, index: usize) -> Result<KarSong> {
        let s = self.songs.get(index).ok_or_else(|| Error::Song("no such song".into()))?;
        load_song(s)
    }
}

/// `songs.json` -> `songs.stats.json`.
pub fn stats_path(catalogue: &Path) -> PathBuf {
    let stem = catalogue.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "songs".into());
    catalogue.with_file_name(format!("{stem}.stats.json"))
}

/// Write atomically (temporary file, then rename).
fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let io = |source| Error::Io { path: path.to_path_buf(), source };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let json = serde_json::to_vec(value).map_err(|source| Error::Json { path: path.to_path_buf(), source })?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, json).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

pub fn load_song(s: &Song) -> Result<KarSong> {
    let err = |e: &dyn fmt::Display| Error::Song(format!("{}: {e}", s.id));
    match &s.location {
        Location::Sfkar(path) => KarSong::load(path).map_err(|e| err(&e)),
        Location::Ncn { midi, lyrics, cursor } => {
            let info = MidiInfo::load(midi).map_err(|e| err(&e))?;
            let lyr = Lyrics::load(lyrics).map_err(|e| err(&e))?;
            let cur = Cursor::load(cursor).map_err(|e| err(&e))?;
            let song = NcnSong::from_parts(s.id.clone(), lyr, &cur, &info);
            let bytes = std::fs::read(midi).map_err(|source| Error::Io { path: midi.clone(), source })?;
            Ok(KarSong::from_ncn(&song, bytes))
        }
    }
}

/// Read every source. Slow on big libraries (it opens each song's
/// details), so run it off the UI thread.
pub fn scan(sources: &[Source]) -> Scan {
    let mut out = Scan { sources: sources.to_vec(), ..Scan::default() };
    for (i, src) in sources.iter().enumerate() {
        match src.kind {
            SourceKind::Ncn => match NcnLibrary::open(&src.path) {
                Ok(lib) => {
                    for h in lib.headers() {
                        let Some(e) = lib.get(&h.id).filter(|e| e.is_complete()) else { continue };
                        out.songs.push(Song {
                            uid: h.id.to_uppercase(),
                            id: h.id,
                            title: h.title,
                            artist: h.artist,
                            key: h.key,
                            source: i,
                            location: Location::Ncn {
                                midi: e.midi.clone().expect("complete"),
                                lyrics: e.lyrics.clone().expect("complete"),
                                cursor: e.cursor.clone().expect("complete"),
                            },
                        });
                    }
                }
                Err(e) => out.errors.push(e.to_string()),
            },
            SourceKind::Sfkar => {
                if !src.path.is_dir() {
                    out.errors.push(format!("{}: folder not found", src.path.display()));
                    continue;
                }
                for path in sfkar_files(&src.path) {
                    match KarSong::read_meta(&path) {
                        Ok(meta) => {
                            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                            let id = meta.id.clone().filter(|id| !id.is_empty()).unwrap_or(stem);
                            out.songs.push(Song {
                                uid: id.to_uppercase(),
                                id,
                                title: meta.title,
                                artist: meta.artist,
                                key: meta.key,
                                source: i,
                                location: Location::Sfkar(path),
                            });
                        }
                        Err(e) => out.errors.push(format!("{}: {e}", path.display())),
                    }
                }
            }
        }
    }
    out
}

/// `.sfkar` files under `dir`, sorted.
fn sfkar_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case(solfege_sfkar::EXTENSION)) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(uid: &str, title: &str, artist: &str, source: usize) -> Song {
        Song {
            uid: uid.into(),
            id: uid.into(),
            title: title.into(),
            artist: artist.into(),
            key: None,
            source,
            location: Location::Sfkar(PathBuf::from(format!("{uid}.sfkar"))),
        }
    }

    fn db() -> SongDb {
        let mut db = SongDb {
            sources: vec![Source { path: "a".into(), kind: SourceKind::Sfkar }],
            songs: vec![
                song("A1", "รักเธอ", "นักร้อง", 0),
                song("A2", "เธอคือรัก", "วง", 0),
                song("A3", "ลมหนาว", "รักดี", 0),
                song("A4", "ฝน", "ใคร", 0),
            ],
            ..SongDb::default()
        };
        db.reindex();
        db
    }

    #[test]
    fn search_ranks_titles_then_artists() {
        let mut db = db();
        assert_eq!(db.search("รัก"), [0, 1, 2]);
        db.toggle_favorite("A2");
        assert_eq!(db.search("รัก"), [0, 1, 2], "rank beats favourite");
        assert_eq!(db.search("เธอ"), [1, 0], "favourite first within a rank");
        assert_eq!(db.search("a3"), [2]);
        assert_eq!(db.search("รัก ลม"), [2]);
        assert!(db.search("zzz").is_empty());
    }

    #[test]
    fn empty_query_puts_favourites_and_recent_first() {
        let mut db = db();
        db.record_play("A3", 100);
        db.record_play("A4", 200);
        db.toggle_favorite("A1");
        assert_eq!(db.search(""), [0, 3, 2, 1]);
        assert_eq!(db.stats("A4").plays, 1);
    }

    #[test]
    fn apply_dedupes_and_keeps_unscanned_sources() {
        let mut db = db();
        db.sources.push(Source { path: "b".into(), kind: SourceKind::Sfkar });
        db.songs.push(song("B1", "บี", "", 1));
        // A scan of source "a" only, with a duplicate code.
        let scan = Scan {
            sources: vec![db.sources[0].clone()],
            songs: vec![song("A9", "ใหม่", "", 0), song("A9", "ซ้ำ", "", 0)],
            errors: vec![],
        };
        db.apply(scan);
        let ids: Vec<&str> = db.songs.iter().map(|s| s.uid.as_str()).collect();
        assert_eq!(ids, ["A9", "B1"]);
        db.remove_source(0);
        assert_eq!(db.songs.len(), 1);
        assert_eq!(db.songs[0].source, 0);
    }

    #[test]
    fn saves_and_loads() {
        let mut db = db();
        db.toggle_favorite("A1");
        let path = std::env::temp_dir().join(format!("songdb-{}.json", std::process::id()));
        db.save(&path).unwrap();
        let back = SongDb::load(&path).unwrap();
        assert_eq!(back.songs, db.songs);
        assert!(back.stats("A1").favorite);
        assert_eq!(back.search("รัก"), db.search("รัก"));
        // Stats alone can be written without the catalogue.
        db.record_play("A2", 5);
        db.save_stats(&path).unwrap();
        assert_eq!(SongDb::load(&path).unwrap().stats("A2").plays, 1);
        std::fs::remove_file(stats_path(&path)).ok();
        std::fs::remove_file(path).ok();
        assert!(SongDb::load(Path::new("/no/such/songs.json")).unwrap().songs.is_empty());
    }

    #[test]
    fn scans_ncn_and_sfkar_sources() {
        let ncn = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shared/NCN");
        let Ok(lib) = NcnLibrary::open(&ncn) else {
            eprintln!("skipped: sample library missing");
            return;
        };
        // Convert a few songs into a .sfkar folder; they share codes with
        // the NCN library, so only the first source's copies are listed.
        let dir = std::env::temp_dir().join(format!("songdb-sfkar-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for id in ["Z2608001", "Z2608002"] {
            solfege_sfkar::convert(&lib, id).unwrap().save(&dir.join("sub").join(format!("{id}.sfkar"))).unwrap();
        }
        let mut db = SongDb::default();
        assert_eq!(db.add_source(dir.clone()).unwrap(), SourceKind::Sfkar);
        assert_eq!(db.add_source(ncn.clone()).unwrap(), SourceKind::Ncn);
        assert!(matches!(db.add_source(std::env::temp_dir().join("no-such-dir")), Err(Error::NotASource(_))));
        let scan = scan(&db.sources);
        assert!(scan.errors.is_empty(), "{:?}", scan.errors);
        db.apply(scan);
        assert_eq!(db.songs.len(), 139);
        let i = db.find("z2608001").unwrap();
        assert!(matches!(db.songs[i].location, Location::Sfkar(_)));
        let j = db.find("Z2608003").unwrap();
        assert!(matches!(db.songs[j].location, Location::Ncn { .. }));
        // Both formats load into the same song shape.
        let a = db.load_song(i).unwrap();
        let b = load_song(&Song { location: Location::Sfkar(PathBuf::new()), ..db.songs[j].clone() });
        assert!(b.is_err());
        let c = db.load_song(j).unwrap();
        assert_eq!(a.meta.id.as_deref(), Some("Z2608001"));
        assert!(!c.lyrics.lines.is_empty());
        std::fs::remove_dir_all(dir).ok();
    }
}
