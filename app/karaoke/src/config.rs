//! The settings file, `config.json` in the app's data folder: the
//! SoundFont rack and its routing, sounds per instrument, the drum kit
//! lock, effects, audio device and lyric display. Plain, pretty-printed
//! JSON so it can be read, edited and backed up by hand; unknown or
//! missing fields fall back to their defaults.
//!
//! Older versions kept the settings inside eframe's own storage; they are
//! read from there once, on the first run without a `config.json`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use solfege_synth::engine::inserts::{INSERT_SLOTS, InsertParams};
use solfege_synth::engine::mixer::FxParams;

pub const FILE_NAME: &str = "config.json";
/// Key of the settings in eframe's storage (older versions).
pub const LEGACY_KEY: &str = "settings";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Library folder of older versions; moved into the song catalogue.
    #[serde(skip_serializing)]
    pub library: Option<PathBuf>,
    /// Single SoundFont of older versions; moved into `soundfonts`.
    #[serde(skip_serializing)]
    pub soundfont: Option<PathBuf>,
    /// The SoundFont rack, first one first.
    pub soundfonts: Vec<PathBuf>,
    /// Font index per MIDI channel.
    pub routing: [usize; 16],
    /// Drum kit locked on channel 10, as (bank, program).
    pub drum_lock: Option<(u16, u8)>,
    /// Sounds chosen per GM instrument.
    pub instruments: Vec<SavedInstrument>,
    /// Kit pieces (kick, snare, ...) playing from a kit of their own.
    pub pieces: Vec<SavedPiece>,
    /// Reverb and chorus (return levels and their parameters).
    pub fx: FxParams,
    /// Master effect slots, in processing order (`null` = empty).
    pub inserts: [Option<InsertParams>; INSERT_SLOTS],
    pub device: Option<String>,
    pub volume: f32,
    /// Lyric size relative to the stage height.
    pub lyric_scale: f32,
    /// Shift the lyrics against the music (positive = lyrics later).
    pub lyric_offset_ms: i32,
    /// How the lyrics move on the stage.
    pub lyric_mode: LyricMode,
    /// Mute the guide melody (MIDI channel 9) in every song.
    pub melody_off: bool,
    /// Show the time of day on the stage.
    pub show_clock: bool,
    /// Colours of the lyrics and their wipe.
    pub lyric_colors: LyricColors,
    /// Thickness of the rim around the lyric letters, relative to the
    /// default (0 = no rim).
    pub lyric_outline: f32,
    /// Font file for the lyrics (`.ttf`, `.otf`, `.ttc`); `None` = Noto Sans Thai.
    pub lyric_font: Option<PathBuf>,
}

/// Colours of the lyric stage, as RGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LyricColors {
    /// Text not sung yet.
    pub unsung: [u8; 3],
    /// Text already sung.
    pub sung: [u8; 3],
    /// The syllable being sung (the moving edge of the wipe) and the bead.
    pub wipe: [u8; 3],
    /// Rim around the letters.
    pub outline: [u8; 3],
}

impl Default for LyricColors {
    fn default() -> Self {
        Self::PRESETS[0].1
    }
}

impl LyricColors {
    /// Ready-made colour sets.
    pub const PRESETS: [(&'static str, LyricColors); 6] = [
        ("ส้มทอง", LyricColors { unsung: [0xf4, 0xf4, 0xf4], sung: [0xff, 0xb0, 0x3b], wipe: [0xff, 0x6a, 0x3d], outline: [0x15, 0x15, 0x15] }),
        ("ฟ้า", LyricColors { unsung: [0xf4, 0xf4, 0xf4], sung: [0x4f, 0xc3, 0xf7], wipe: [0x29, 0x79, 0xff], outline: [0x0b, 0x12, 0x20] }),
        ("ชมพู", LyricColors { unsung: [0xf4, 0xf4, 0xf4], sung: [0xff, 0x7e, 0xb6], wipe: [0xe0, 0x40, 0xfb], outline: [0x1a, 0x0b, 0x16] }),
        ("เขียว", LyricColors { unsung: [0xf4, 0xf4, 0xf4], sung: [0x69, 0xf0, 0xae], wipe: [0x00, 0xc8, 0x53], outline: [0x08, 0x18, 0x10] }),
        ("แดงขาว", LyricColors { unsung: [0xff, 0xff, 0xff], sung: [0xff, 0x40, 0x40], wipe: [0xff, 0xd0, 0x40], outline: [0x00, 0x00, 0x00] }),
        ("น้ำเงินเหลือง", LyricColors { unsung: [0xff, 0xf1, 0x76], sung: [0x40, 0x80, 0xff], wipe: [0x80, 0xd8, 0xff], outline: [0x00, 0x00, 0x30] }),
    ];
}

/// How the lyrics are laid out on the stage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LyricMode {
    /// The line being sung sits in the middle; lines scroll up as they
    /// finish and the next one wipes in the middle.
    #[default]
    Scroll,
    /// Two fixed lines in the middle, top and bottom in turn: a finished
    /// line is replaced in place by the one after next.
    Classic,
}

impl LyricMode {
    pub fn label(self) -> &'static str {
        match self {
            LyricMode::Scroll => "เลื่อนขึ้นแล้วปาด",
            LyricMode::Classic => "ปาดตรงกลาง 2 บรรทัด",
        }
    }

    pub fn other(self) -> Self {
        match self {
            LyricMode::Scroll => LyricMode::Classic,
            LyricMode::Classic => LyricMode::Scroll,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            library: None,
            soundfont: None,
            soundfonts: Vec::new(),
            routing: [0; 16],
            drum_lock: None,
            instruments: Vec::new(),
            pieces: Vec::new(),
            fx: FxParams::default(),
            inserts: [None; INSERT_SLOTS],
            device: None,
            volume: 0.8,
            lyric_scale: 1.0,
            lyric_offset_ms: 0,
            lyric_mode: LyricMode::Scroll,
            melody_off: false,
            show_clock: true,
            lyric_colors: LyricColors::default(),
            lyric_outline: 1.0,
            lyric_font: None,
        }
    }
}

/// A GM instrument's sound, saved by font file so it survives the rack
/// being reordered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedInstrument {
    /// GM program 0..127.
    pub instrument: u8,
    pub font: PathBuf,
    pub bank: u16,
    pub program: u8,
}

/// A kit piece's own kit, saved by font file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedPiece {
    /// 0 kick, 1 snare, 2 hi-hat, 3 toms, 4 cymbals, 5 percussion, 6 cowbell.
    pub piece: usize,
    pub font: PathBuf,
    pub bank: u16,
    pub program: u8,
}

impl Settings {
    /// Read a settings file: `Ok(None)` when there is none yet.
    pub fn load(path: &Path) -> Result<Option<Self>, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        serde_json::from_str(&text).map(Some).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("settings serialize");
        text.push('\n');
        text
    }
}

/// Where the settings live, and what was last written there.
pub struct ConfigFile {
    pub path: Option<PathBuf>,
    written: String,
}

impl ConfigFile {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self { path, written: String::new() }
    }

    /// Open the settings: the file, else `legacy` (older versions), else
    /// defaults. A file that cannot be read is kept aside as
    /// `config.json.bak` (it would be overwritten otherwise) and reported.
    pub fn load(&mut self, legacy: Option<Settings>) -> (Settings, Option<String>) {
        let Some(path) = &self.path else { return (legacy.unwrap_or_default(), None) };
        match Settings::load(path) {
            Ok(Some(settings)) => {
                self.written = settings.to_json();
                (settings, None)
            }
            Ok(None) => (legacy.unwrap_or_default(), None),
            Err(e) => {
                let _ = std::fs::copy(path, path.with_extension("json.bak"));
                (legacy.unwrap_or_default(), Some(e))
            }
        }
    }

    /// Settings were read from the file (it is not the first run).
    pub fn existed(&self) -> bool {
        !self.written.is_empty()
    }

    /// Write the settings if they changed since the last write (atomic:
    /// a temporary file renamed over the old one).
    pub fn save(&mut self, settings: &Settings) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        let text = settings.to_json();
        if text == self.written {
            return Ok(());
        }
        let err = |e: std::io::Error| format!("{}: {e}", path.display());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(err)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &text).map_err(err)?;
        std::fs::rename(&tmp, path).map_err(err)?;
        self.written = text;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_writes_only_changes() {
        let dir = std::env::temp_dir().join(format!("karaoke-config-{}", std::process::id()));
        let path = dir.join(FILE_NAME);
        let mut file = ConfigFile::new(Some(path.clone()));
        let legacy = Settings { volume: 0.5, ..Settings::default() };
        let (mut s, err) = file.load(Some(legacy));
        assert!(err.is_none());
        assert!(!file.existed(), "first run");
        assert_eq!(s.volume, 0.5, "first run takes the old settings");
        s.soundfonts = vec![PathBuf::from("/fonts/gm.sf2")];
        s.drum_lock = Some((128, 16));
        s.fx.reverb_room = 0.9;
        s.lyric_colors = LyricColors::PRESETS[2].1;
        s.lyric_outline = 2.5;
        s.lyric_font = Some(PathBuf::from("/fonts/lyrics.ttf"));
        file.save(&s).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"soundfonts\": [\n"), "pretty JSON:\n{text}");
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        file.save(&s).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), modified, "unchanged: not rewritten");

        let mut again = ConfigFile::new(Some(path.clone()));
        let (back, _) = again.load(None);
        assert!(again.existed(), "the file is there now");
        assert_eq!(back.to_json(), s.to_json());
        assert_eq!(back.fx.reverb_room, 0.9);
        assert!(back.lyric_colors == LyricColors::PRESETS[2].1);
        assert_eq!(back.lyric_font, s.lyric_font);
        assert_eq!(back.lyric_outline, 2.5);

        // Hand-edited with fields missing: defaults fill in.
        std::fs::write(&path, r#"{ "volume": 0.3 }"#).unwrap();
        let (partial, err) = ConfigFile::new(Some(path.clone())).load(None);
        assert!(err.is_none());
        assert_eq!(partial.volume, 0.3);
        assert_eq!(partial.lyric_scale, 1.0);
        assert!(partial.lyric_colors == LyricColors::default());
        assert_eq!(partial.lyric_outline, 1.0);

        // Broken: reported, defaults used, the file kept aside.
        std::fs::write(&path, "{ not json").unwrap();
        let (broken, err) = ConfigFile::new(Some(path.clone())).load(None);
        assert!(err.is_some());
        assert_eq!(broken.volume, Settings::default().volume);
        assert_eq!(std::fs::read_to_string(path.with_extension("json.bak")).unwrap(), "{ not json");
        std::fs::remove_dir_all(dir).ok();
    }
}
