//! Instrument model shared by every loader: an instrument is a list of
//! presets, a preset is a list of key/velocity zones, a zone points into a
//! block of sample data. WAV, SFZ and SF2 loaders all produce this shape so
//! the engine only needs one voice implementation.

pub mod sf2;
pub mod sfz;
pub mod wav;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, bail};

/// Raw PCM storage. 16-bit sources stay 16-bit to halve memory for big SF2 banks.
pub enum Samples {
    I16(Vec<i16>),
    F32(Vec<f32>),
}

pub struct SampleData {
    pub channels: usize,
    pub frames: usize,
    pub samples: Samples,
}

impl SampleData {
    pub fn from_f32(channels: usize, data: Vec<f32>) -> Self {
        let frames = data.len() / channels.max(1);
        Self { channels, frames, samples: Samples::F32(data) }
    }

    pub fn from_i16(channels: usize, data: Vec<i16>) -> Self {
        let frames = data.len() / channels.max(1);
        Self { channels, frames, samples: Samples::I16(data) }
    }

    #[inline]
    fn raw(&self, idx: usize) -> f32 {
        match &self.samples {
            Samples::I16(v) => v[idx] as f32 * (1.0 / 32768.0),
            Samples::F32(v) => v[idx],
        }
    }

    /// Stereo frame; mono is duplicated to both sides.
    #[inline]
    pub fn frame(&self, i: usize) -> (f32, f32) {
        if self.channels == 1 {
            let s = self.raw(i);
            (s, s)
        } else {
            let base = i * self.channels;
            (self.raw(base), self.raw(base + 1))
        }
    }

    pub fn bytes(&self) -> usize {
        match &self.samples {
            Samples::I16(v) => v.len() * 2,
            Samples::F32(v) => v.len() * 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopMode {
    NoLoop,
    OneShot,
    Continuous,
    Sustain,
}

impl LoopMode {
    pub fn label(self) -> &'static str {
        match self {
            LoopMode::NoLoop => "no loop",
            LoopMode::OneShot => "one shot",
            LoopMode::Continuous => "loop",
            LoopMode::Sustain => "loop sustain",
        }
    }

    pub fn next(self) -> Self {
        match self {
            LoopMode::NoLoop => LoopMode::OneShot,
            LoopMode::OneShot => LoopMode::Continuous,
            LoopMode::Continuous => LoopMode::Sustain,
            LoopMode::Sustain => LoopMode::NoLoop,
        }
    }

    pub fn prev(self) -> Self {
        self.next().next().next()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Attack,
    Release,
}

/// Amplitude envelope, times in seconds, sustain as linear level 0..1.
#[derive(Clone, Copy, Debug)]
pub struct Envelope {
    pub delay: f32,
    pub attack: f32,
    pub hold: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

impl Default for Envelope {
    fn default() -> Self {
        Self { delay: 0.0, attack: 0.0, hold: 0.0, decay: 0.0, sustain: 1.0, release: 0.005 }
    }
}

#[derive(Clone)]
pub struct Zone {
    pub sample: Arc<SampleData>,
    pub sample_rate: f32,
    pub lokey: u8,
    pub hikey: u8,
    pub lovel: u8,
    pub hivel: u8,
    pub root: f32,
    /// Fine/coarse tuning in cents.
    pub tune: f32,
    /// Cents per key; 100 is normal, 0 disables keytracking.
    pub keytrack: f32,
    pub gain: f32,
    pub pan: f32,
    /// 0..1, how much velocity scales amplitude.
    pub veltrack: f32,
    pub start: usize,
    pub end: usize,
    pub loop_mode: LoopMode,
    pub loop_start: usize,
    pub loop_end: usize,
    pub env: Envelope,
    pub trigger: Trigger,
    pub group: u32,
    pub off_by: u32,
    pub seq_length: u32,
    pub seq_position: u32,
    pub lorand: f32,
    pub hirand: f32,
    /// Low-pass cutoff in Hz, `None` = filter bypassed.
    pub cutoff: Option<f32>,
    pub resonance_db: f32,
}

impl Zone {
    pub fn new(sample: Arc<SampleData>, sample_rate: f32) -> Self {
        let end = sample.frames;
        Self {
            sample,
            sample_rate,
            lokey: 0,
            hikey: 127,
            lovel: 1,
            hivel: 127,
            root: 60.0,
            tune: 0.0,
            keytrack: 100.0,
            gain: 1.0,
            pan: 0.0,
            veltrack: 1.0,
            start: 0,
            end,
            loop_mode: LoopMode::NoLoop,
            loop_start: 0,
            loop_end: end,
            env: Envelope::default(),
            trigger: Trigger::Attack,
            group: 0,
            off_by: 0,
            seq_length: 1,
            seq_position: 1,
            lorand: 0.0,
            hirand: 1.0,
            cutoff: None,
            resonance_db: 0.0,
        }
    }

    /// Clamp offsets so the voice never indexes outside the sample buffer.
    pub fn sanitize(&mut self) {
        let frames = self.sample.frames;
        self.end = self.end.min(frames);
        self.start = self.start.min(self.end.saturating_sub(1));
        self.loop_end = self.loop_end.min(self.end);
        self.loop_start = self.loop_start.clamp(self.start, self.loop_end);
        if self.loop_end.saturating_sub(self.loop_start) < 2
            && matches!(self.loop_mode, LoopMode::Continuous | LoopMode::Sustain)
        {
            self.loop_mode = LoopMode::NoLoop;
        }
        if self.lokey > self.hikey {
            std::mem::swap(&mut self.lokey, &mut self.hikey);
        }
        if self.lovel > self.hivel {
            std::mem::swap(&mut self.lovel, &mut self.hivel);
        }
    }

    #[inline]
    pub fn matches(&self, key: u8, vel: u8) -> bool {
        key >= self.lokey && key <= self.hikey && vel >= self.lovel && vel <= self.hivel
    }
}

pub struct Preset {
    pub name: String,
    pub bank: u16,
    pub program: u8,
    pub zones: Vec<Zone>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Wav,
    Sfz,
    Sf2,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Wav => "WAV",
            Kind::Sfz => "SFZ",
            Kind::Sf2 => "SF2",
        }
    }
}

/// Editable parameters of a single-WAV sampler instrument.
#[derive(Clone, Copy, Debug)]
pub struct WavParams {
    pub root: u8,
    pub tune: f32,
    pub keytrack: bool,
    pub loop_mode: LoopMode,
    pub env: Envelope,
}

pub struct WavSource {
    pub sample: Arc<SampleData>,
    pub sample_rate: f32,
    pub loop_points: Option<(usize, usize)>,
    pub params: WavParams,
}

pub struct Instrument {
    pub name: String,
    pub kind: Kind,
    pub path: PathBuf,
    pub presets: Vec<Preset>,
    pub warnings: Vec<String>,
    pub sample_bytes: usize,
    /// Present only for single-WAV instruments so the UI can rebuild them.
    pub wav: Option<Arc<WavSource>>,
}

impl Instrument {
    pub fn zone_count(&self) -> usize {
        self.presets.iter().map(|p| p.zones.len()).sum()
    }

    pub fn find_preset(&self, bank: u16, program: u8) -> Option<usize> {
        self.presets.iter().position(|p| p.bank == bank && p.program == program)
    }
}

pub fn is_supported(path: &Path) -> bool {
    matches!(ext(path).as_str(), "wav" | "sfz" | "sf2")
}

fn ext(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

pub fn load(path: &Path) -> Result<Instrument> {
    match ext(path).as_str() {
        "wav" => wav::load_instrument(path),
        "sfz" => sfz::load(path),
        "sf2" => sf2::load(path),
        other => bail!("unsupported file type '.{other}'"),
    }
}

/// Parse a MIDI note number or SFZ-style note name (`c4` = 60, `f#3`, `eb2`).
pub fn parse_note(s: &str) -> Option<i32> {
    let s = s.trim();
    if let Ok(n) = s.parse::<i32>() {
        return Some(n);
    }
    let mut chars = s.chars();
    let base = match chars.next()?.to_ascii_lowercase() {
        'c' => 0,
        'd' => 2,
        'e' => 4,
        'f' => 5,
        'g' => 7,
        'a' => 9,
        'b' => 11,
        _ => return None,
    };
    let rest = chars.as_str();
    let (acc, oct) = if let Some(r) = rest.strip_prefix('#') {
        (1, r)
    } else if let Some(r) = rest.strip_prefix('b') {
        (-1, r)
    } else {
        (0, rest)
    };
    let oct: i32 = oct.trim().parse().ok()?;
    Some((oct + 1) * 12 + base + acc)
}

pub fn note_name(n: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[(n % 12) as usize], n as i32 / 12 - 1)
}

/// Linear gain from decibels.
pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}
