//! # solfege_sfkar
//!
//! `.sfkar` is the Solfege karaoke song file: one self-contained file with
//! the backing track, the lyrics timed per syllable, and the song details,
//! so a song no longer needs three parallel folders (as NCN does) and its
//! text is plain UTF-8 instead of Windows-874.
//!
//! ## Layout
//!
//! ```text
//! magic    "SFKR"            4 bytes
//! version  u16 LE            currently 1
//! flags    u16 LE            0 (reserved)
//! chunks   until end of file:
//!   id     4 ASCII bytes
//!   length u32 LE
//!   data   `length` bytes
//! ```
//!
//! | chunk  | contents |
//! |--------|----------|
//! | `META` | JSON [`Meta`]: title, artist, key, source id, … |
//! | `MIDI` | a Standard MIDI File (the backing track) |
//! | `LYRC` | JSON [`Lyrics`]: lines of `[tick, text]` segments |
//!
//! All three are required and appear in that order when written. Readers
//! skip chunks they do not know, so later versions can add some (artwork,
//! a vocal guide, …) without breaking older players.
//!
//! Lyric times are MIDI ticks of the `MIDI` chunk, not seconds, so they
//! follow the song's tempo map and any playback speed. A segment is the
//! text that lights up at its tick; for Thai that is one or more display
//! clusters (a letter with its vowel and tone marks). A line's `end` is the
//! tick at which it is finished.
//!
//! ```no_run
//! use solfege_sfkar::KarSong;
//!
//! let song = KarSong::load("Z2608001.sfkar".as_ref())?;
//! let timing = song.timing()?;
//! for line in &song.lyrics.lines {
//!     println!("[{:>7.2}s] {}", timing.tempo.seconds(line.start()), line.text());
//! }
//! # Ok::<(), solfege_sfkar::Error>(())
//! ```

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use solfege_ncnparser::{MidiInfo, NcnLibrary, NcnSong};

pub const MAGIC: &[u8; 4] = b"SFKR";
pub const VERSION: u16 = 1;
pub const EXTENSION: &str = "sfkar";

/// Song details.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Meta {
    pub title: String,
    pub artist: String,
    /// Musical key as charted (`Bm`, `C#`, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Song code in the catalogue it came from (e.g. the NCN id `Z2608001`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Where the song was converted from (`"ncn"`), if anywhere.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Length of the backing track in seconds (informational).
    pub duration: f64,
}

/// Text lighting up at a tick.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment(pub u32, pub String);

impl Segment {
    pub fn tick(&self) -> u32 {
        self.0
    }

    pub fn text(&self) -> &str {
        &self.1
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub segments: Vec<Segment>,
    /// The line is finished (ticks).
    pub end: u32,
}

impl Line {
    /// First highlight of the line, or its end when it has no text.
    pub fn start(&self) -> u32 {
        self.segments.first().map_or(self.end, |s| s.0)
    }

    pub fn text(&self) -> String {
        self.segments.iter().map(|s| s.1.as_str()).collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Lyrics {
    pub lines: Vec<Line>,
}

/// A decoded `.sfkar` song.
#[derive(Clone, Debug, PartialEq)]
pub struct KarSong {
    pub meta: Meta,
    /// Standard MIDI File bytes.
    pub midi: Vec<u8>,
    pub lyrics: Lyrics,
}

#[derive(Debug)]
pub enum Error {
    Io { path: PathBuf, source: std::io::Error },
    /// Not an `.sfkar` file, or a broken one.
    Format(String),
    /// A newer format version than this reader knows.
    Version(u16),
    Midi(solfege_ncnparser::Error),
    /// Reading the NCN source of a conversion failed.
    Ncn(solfege_ncnparser::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Error::Format(why) => write!(f, "not a valid .sfkar file: {why}"),
            Error::Version(v) => write!(f, ".sfkar version {v} is newer than this program ({VERSION})"),
            Error::Midi(e) => write!(f, "backing track: {e}"),
            Error::Ncn(e) => write!(f, "NCN: {e}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn bad(why: impl Into<String>) -> Error {
    Error::Format(why.into())
}

/// Iterate `(id, data)` over the chunks after the header.
fn chunks(bytes: &[u8]) -> Result<Vec<([u8; 4], &[u8])>> {
    if bytes.len() < 8 || &bytes[0..4] != MAGIC {
        return Err(bad("missing SFKR header"));
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version > VERSION {
        return Err(Error::Version(version));
    }
    let mut out = Vec::new();
    let mut pos = 8;
    while pos < bytes.len() {
        if pos + 8 > bytes.len() {
            return Err(bad("truncated chunk header"));
        }
        let id: [u8; 4] = bytes[pos..pos + 4].try_into().expect("4 bytes");
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().expect("4 bytes")) as usize;
        let start = pos + 8;
        let end = start.checked_add(len).filter(|&e| e <= bytes.len()).ok_or_else(|| bad("truncated chunk"))?;
        out.push((id, &bytes[start..end]));
        pos = end;
    }
    Ok(out)
}

fn read_file(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|source| Error::Io { path: path.to_path_buf(), source })
}

impl KarSong {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let (mut meta, mut midi, mut lyrics) = (None, None, None);
        for (id, data) in chunks(bytes)? {
            match &id {
                b"META" => meta = Some(serde_json::from_slice(data).map_err(|e| bad(format!("META: {e}")))?),
                b"MIDI" => midi = Some(data.to_vec()),
                b"LYRC" => lyrics = Some(serde_json::from_slice(data).map_err(|e| bad(format!("LYRC: {e}")))?),
                _ => {}
            }
        }
        Ok(Self {
            meta: meta.ok_or_else(|| bad("no META chunk"))?,
            midi: midi.ok_or_else(|| bad("no MIDI chunk"))?,
            lyrics: lyrics.ok_or_else(|| bad("no LYRC chunk"))?,
        })
    }

    pub fn load(path: &Path) -> Result<Self> {
        Self::parse(&read_file(path)?)
    }

    /// Only the song details, without decoding lyrics (for catalogues).
    pub fn read_meta(path: &Path) -> Result<Meta> {
        let bytes = read_file(path)?;
        let (_, data) = chunks(&bytes)?.into_iter().find(|(id, _)| id == b"META").ok_or_else(|| bad("no META chunk"))?;
        serde_json::from_slice(data).map_err(|e| bad(format!("META: {e}")))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let meta = serde_json::to_vec(&self.meta).expect("meta serializes");
        let lyrics = serde_json::to_vec(&self.lyrics).expect("lyrics serialize");
        let mut out = Vec::with_capacity(8 + 24 + meta.len() + self.midi.len() + lyrics.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        for (id, data) in [(b"META", &meta), (b"MIDI", &self.midi), (b"LYRC", &lyrics)] {
            out.extend_from_slice(id);
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(data);
        }
        out
    }

    /// Write atomically: a temporary file next to `path`, then a rename.
    pub fn save(&self, path: &Path) -> Result<()> {
        let io = |source| Error::Io { path: path.to_path_buf(), source };
        let tmp = path.with_extension("sfkar.tmp");
        std::fs::write(&tmp, self.to_bytes()).map_err(io)?;
        std::fs::rename(&tmp, path).map_err(io)
    }

    /// Resolution and tempo map of the backing track, to turn lyric ticks
    /// into seconds.
    pub fn timing(&self) -> Result<MidiInfo> {
        MidiInfo::parse(&self.midi).map_err(Error::Midi)
    }

    /// Build from a parsed NCN song and the bytes of its MIDI file.
    ///
    /// Segments are runs of clusters sharing a highlight tick; blank lyric
    /// lines are dropped (they only space the display in NCN).
    pub fn from_ncn(song: &NcnSong, midi: Vec<u8>) -> Self {
        let mut lines = Vec::new();
        for l in song.lines.iter().filter(|l| !l.is_blank()) {
            let mut segments: Vec<Segment> = Vec::new();
            for c in &l.clusters {
                match segments.last_mut() {
                    Some(s) if s.0 == c.tick => s.1.push_str(&c.text),
                    _ => segments.push(Segment(c.tick, c.text.clone())),
                }
            }
            lines.push(Line { segments, end: l.end });
        }
        Self {
            meta: Meta {
                title: song.title.clone(),
                artist: song.artist.clone(),
                key: song.key.clone(),
                id: Some(song.id.clone()),
                source: Some("ncn".into()),
                duration: (song.duration() * 1000.0).round() / 1000.0,
            },
            midi: standard_midi(midi),
            lyrics: Lyrics { lines },
        }
    }
}

/// Convert one song of an NCN library.
pub fn convert(lib: &NcnLibrary, id: &str) -> Result<KarSong> {
    let song = lib.load(id).map_err(Error::Ncn)?;
    let path = song.midi_path.clone().ok_or_else(|| bad("NCN song has no MIDI file"))?;
    Ok(KarSong::from_ncn(&song, read_file(&path)?))
}

/// Some NCN packs replace the `MThd` header with `Lock`; restore it so the
/// MIDI chunk is a normal file any player can use.
pub fn standard_midi(mut midi: Vec<u8>) -> Vec<u8> {
    if midi.starts_with(b"Lock") {
        midi[0..4].copy_from_slice(b"MThd");
    }
    midi
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> KarSong {
        let mut midi = b"MThd\0\0\0\x06\0\0\0\x01\x01\xE0MTrk\0\0\0\x04".to_vec();
        midi.extend([0x00, 0xFF, 0x2F, 0x00]);
        KarSong {
            meta: Meta { title: "เพลง".into(), artist: "ศิลปิน".into(), key: Some("Bm".into()), id: Some("X1".into()), source: None, duration: 1.5 },
            midi,
            lyrics: Lyrics {
                lines: vec![Line { segments: vec![Segment(0, "น้".into()), Segment(240, "อง".into())], end: 480 }],
            },
        }
    }

    #[test]
    fn round_trips() {
        let s = sample();
        let bytes = s.to_bytes();
        assert_eq!(&bytes[0..4], MAGIC);
        assert_eq!(KarSong::parse(&bytes).unwrap(), s);
        assert_eq!(s.lyrics.lines[0].text(), "น้อง");
        assert_eq!(s.lyrics.lines[0].start(), 0);
        assert_eq!(s.timing().unwrap().ppq, 480);
    }

    #[test]
    fn skips_unknown_chunks_and_rejects_bad_files() {
        let mut bytes = sample().to_bytes();
        bytes.extend(b"ARTW");
        bytes.extend(3u32.to_le_bytes());
        bytes.extend([1, 2, 3]);
        assert_eq!(KarSong::parse(&bytes).unwrap(), sample());

        assert!(matches!(KarSong::parse(b"RIFF...."), Err(Error::Format(_))));
        let mut newer = sample().to_bytes();
        newer[4] = 9;
        assert!(matches!(KarSong::parse(&newer), Err(Error::Version(9))));
        let full = sample().to_bytes();
        assert!(matches!(KarSong::parse(&full[..full.len() - 3]), Err(Error::Format(_))));
    }

    #[test]
    fn lock_header_is_restored() {
        assert_eq!(&standard_midi(b"Lock\0\0\0\x06".to_vec())[0..4], b"MThd");
        assert_eq!(standard_midi(b"MThd".to_vec()), b"MThd");
    }
}
