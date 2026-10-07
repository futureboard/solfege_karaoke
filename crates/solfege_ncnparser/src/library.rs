//! An NCN library on disk: `Song`, `Lyrics` and `Cursor` folders (any
//! letter case, any depth of sub-folders) whose files share a song id.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::cursor::Cursor;
use crate::lyrics::Lyrics;
use crate::midi::MidiInfo;
use crate::song::NcnSong;
use crate::{Error, Result};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    /// File stem as found on disk (e.g. `Z2608001`).
    pub id: String,
    pub midi: Option<PathBuf>,
    pub lyrics: Option<PathBuf>,
    pub cursor: Option<PathBuf>,
}

impl Entry {
    pub fn is_complete(&self) -> bool {
        self.midi.is_some() && self.lyrics.is_some() && self.cursor.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SongHeader {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub key: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Midi,
    Lyrics,
    Cursor,
}

pub struct NcnLibrary {
    root: PathBuf,
    /// Keyed by upper-case id so lookups ignore case.
    entries: BTreeMap<String, Entry>,
}

impl NcnLibrary {
    /// Scan a library root (the folder that holds `Song`, `Lyrics`, `Cursor`).
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let read = std::fs::read_dir(&root).map_err(|source| Error::Io { path: root.clone(), source })?;
        let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
        let mut found_any = false;
        for dir in read.flatten() {
            let path = dir.path();
            if !path.is_dir() {
                continue;
            }
            let name = dir.file_name().to_string_lossy().to_ascii_lowercase();
            let part = match name.as_str() {
                "song" | "songs" | "midi" => Part::Midi,
                "lyrics" | "lyric" | "lyr" => Part::Lyrics,
                "cursor" | "cursors" | "cur" => Part::Cursor,
                _ => continue,
            };
            found_any = true;
            walk(&path, &mut |file| {
                let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                let ok = match part {
                    Part::Midi => matches!(ext.as_str(), "mid" | "midi" | "kar" | "rmi"),
                    Part::Lyrics => ext == "lyr",
                    Part::Cursor => ext == "cur",
                };
                let Some(stem) = file.file_stem().and_then(|s| s.to_str()) else { return };
                if !ok {
                    return;
                }
                let e = entries.entry(stem.to_ascii_uppercase()).or_insert_with(|| Entry { id: stem.to_string(), ..Entry::default() });
                let slot = match part {
                    Part::Midi => &mut e.midi,
                    Part::Lyrics => &mut e.lyrics,
                    Part::Cursor => &mut e.cursor,
                };
                slot.get_or_insert_with(|| file.to_path_buf());
            });
        }
        if !found_any {
            return Err(Error::NotLibrary(root));
        }
        Ok(Self { root, entries })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    /// Songs that have all three files.
    pub fn complete(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values().filter(|e| e.is_complete())
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.entries.get(&id.to_ascii_uppercase())
    }

    /// Parse MIDI, lyrics and cursor of one song.
    pub fn load(&self, id: &str) -> Result<NcnSong> {
        let e = self.get(id).ok_or_else(|| Error::NotFound(id.to_string()))?;
        let need = |p: &Option<PathBuf>, part| p.clone().ok_or_else(|| Error::MissingPart { id: e.id.clone(), part });
        let (midi_path, lyr_path, cur_path) = (need(&e.midi, "MIDI")?, need(&e.lyrics, "lyrics")?, need(&e.cursor, "cursor")?);
        let midi = MidiInfo::load(&midi_path)?;
        let lyrics = Lyrics::load(&lyr_path)?;
        let cursor = Cursor::load(&cur_path)?;
        let mut song = NcnSong::from_parts(e.id.clone(), lyrics, &cursor, &midi);
        song.midi_path = Some(midi_path);
        song.lyrics_path = Some(lyr_path);
        song.cursor_path = Some(cur_path);
        Ok(song)
    }

    /// Title / artist / key of every song with a lyric file.
    pub fn headers(&self) -> Vec<SongHeader> {
        self.entries
            .values()
            .filter_map(|e| {
                let bytes = std::fs::read(e.lyrics.as_ref()?).ok()?;
                let (title, artist, key) = Lyrics::parse_header(&bytes);
                Some(SongHeader { id: e.id.clone(), title, artist, key })
            })
            .collect()
    }

    /// Case-insensitive substring search over id, title and artist.
    pub fn search(&self, query: &str) -> Vec<SongHeader> {
        let q = query.to_lowercase();
        self.headers()
            .into_iter()
            .filter(|h| [&h.id, &h.title, &h.artist].iter().any(|s| s.to_lowercase().contains(&q)))
            .collect()
    }
}

fn walk(dir: &Path, f: &mut dyn FnMut(&Path)) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            walk(&p, f);
        } else {
            f(&p);
        }
    }
}
