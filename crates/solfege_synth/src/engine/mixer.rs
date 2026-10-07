//! Mixer model. Every slot owns one strip ("DSP node") per MIDI channel, so
//! two channels playing the same preset still get separate processing, plus
//! one strip per drum note group so channel 10 can be split into a
//! multi-out kit (kick, snare, hats, ...). Strips feed output buses and the
//! global reverb / chorus sends.

use serde::{Deserialize, Serialize};

use super::dsp::Biquad;
use crate::instrument::db_to_gain;

pub const MAX_GROUPS: usize = 8;
/// Strips 0..16 are MIDI channels, 16.. are channel-10 note groups.
pub const DRUM_STRIP_BASE: usize = 16;
pub const MAX_STRIPS: usize = DRUM_STRIP_BASE + MAX_GROUPS;
/// Stereo output buses: 0 = main (device 1/2), 1 = device 3/4, ...
pub const MAX_BUSES: usize = 8;
pub const DRUM_CHANNEL: u8 = 9;
pub const BLOCK: usize = 512;
/// Frames a strip keeps processing after its last input (filter tails).
const TAIL_FRAMES: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StripParams {
    pub gain_db: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub eq_low_db: f32,
    pub eq_mid_db: f32,
    pub eq_mid_hz: f32,
    pub eq_high_db: f32,
    /// High-pass (low cut); 20 Hz = off.
    pub hpf_hz: f32,
    /// Low-pass (high cut); 20 kHz = off.
    pub lpf_hz: f32,
    /// Scales the channel's CC91 (reverb) / CC93 (chorus) send level.
    pub reverb: f32,
    pub chorus: f32,
    /// Added to the (scaled) send level, so a channel the song leaves dry
    /// can still be sent; -1..1 of the full send.
    pub reverb_add: f32,
    pub chorus_add: f32,
    pub output: u8,
}

impl Default for StripParams {
    fn default() -> Self {
        Self {
            gain_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
            eq_low_db: 0.0,
            eq_mid_db: 0.0,
            eq_mid_hz: 1000.0,
            eq_high_db: 0.0,
            hpf_hz: 20.0,
            lpf_hz: 20_000.0,
            reverb: 1.0,
            chorus: 1.0,
            reverb_add: 0.0,
            chorus_add: 0.0,
            output: 0,
        }
    }
}

impl StripParams {
    pub fn clamped(mut self) -> Self {
        self.gain_db = self.gain_db.clamp(-60.0, 12.0);
        self.pan = self.pan.clamp(-1.0, 1.0);
        for db in [&mut self.eq_low_db, &mut self.eq_mid_db, &mut self.eq_high_db] {
            *db = db.clamp(-18.0, 18.0);
        }
        self.eq_mid_hz = self.eq_mid_hz.clamp(100.0, 10_000.0);
        self.hpf_hz = self.hpf_hz.clamp(20.0, 2000.0);
        self.lpf_hz = self.lpf_hz.clamp(500.0, 20_000.0);
        self.reverb = self.reverb.clamp(0.0, 2.0);
        self.chorus = self.chorus.clamp(0.0, 2.0);
        self.reverb_add = self.reverb_add.clamp(-1.0, 1.0);
        self.chorus_add = self.chorus_add.clamp(-1.0, 1.0);
        self.output = self.output.min(MAX_BUSES as u8 - 1);
        self
    }

    pub fn gain(&self) -> f32 {
        if self.gain_db <= -60.0 { 0.0 } else { db_to_gain(self.gain_db) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FxParams {
    pub reverb_return: f32,
    pub reverb_room: f32,
    pub reverb_damp: f32,
    pub reverb_width: f32,
    pub chorus_return: f32,
    pub chorus_rate: f32,
    pub chorus_depth: f32,
    pub chorus_delay: f32,
}

impl Default for FxParams {
    fn default() -> Self {
        Self {
            reverb_return: 0.5,
            reverb_room: 0.6,
            reverb_damp: 0.4,
            reverb_width: 1.0,
            chorus_return: 0.5,
            chorus_rate: 0.8,
            chorus_depth: 3.0,
            chorus_delay: 12.0,
        }
    }
}

impl FxParams {
    pub fn clamped(mut self) -> Self {
        self.reverb_return = self.reverb_return.clamp(0.0, 2.0);
        self.reverb_room = self.reverb_room.clamp(0.0, 1.0);
        self.reverb_damp = self.reverb_damp.clamp(0.0, 1.0);
        self.reverb_width = self.reverb_width.clamp(0.0, 1.0);
        self.chorus_return = self.chorus_return.clamp(0.0, 2.0);
        self.chorus_rate = self.chorus_rate.clamp(0.05, 8.0);
        self.chorus_depth = self.chorus_depth.clamp(0.0, 15.0);
        self.chorus_delay = self.chorus_delay.clamp(2.0, 30.0);
        self
    }
}

pub const GM_GROUP_NAMES: [&str; MAX_GROUPS] =
    ["Kick", "Snare", "Hi-Hat", "Toms", "Cymbals", "Percussion", "Group 7", "Group 8"];

/// Note -> group map for channel 10. `NO_GROUP` keeps the note on the
/// channel-10 strip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteGroups {
    pub enabled: bool,
    pub map: [u8; 128],
}

pub const NO_GROUP: u8 = 0xFF;

impl NoteGroups {
    /// General MIDI percussion key map split into kit pieces.
    pub fn gm() -> Self {
        let mut map = [5u8; 128];
        for (notes, g) in [
            (&[35u8, 36][..], 0u8),
            (&[37, 38, 39, 40][..], 1),
            (&[42, 44, 46][..], 2),
            (&[41, 43, 45, 47, 48, 50][..], 3),
            (&[49, 51, 52, 53, 55, 57, 59][..], 4),
        ] {
            for &n in notes {
                map[n as usize] = g;
            }
        }
        Self { enabled: true, map }
    }

    #[inline]
    pub fn strip_for(&self, note: u8) -> Option<usize> {
        let g = self.map[note as usize & 127];
        (self.enabled && (g as usize) < MAX_GROUPS).then(|| DRUM_STRIP_BASE + g as usize)
    }

    /// Groups that have at least one note assigned.
    pub fn used(&self) -> u8 {
        self.map.iter().filter(|&&g| (g as usize) < MAX_GROUPS).fold(0u8, |m, &g| m | (1 << g))
    }
}

impl Default for NoteGroups {
    fn default() -> Self {
        Self::gm()
    }
}

/// Audio-thread state of one strip.
pub struct Strip {
    pub params: StripParams,
    pub buf_l: Vec<f32>,
    pub buf_r: Vec<f32>,
    /// High-pass, low shelf, mid peak, high shelf, low-pass.
    filters: [Biquad; 5],
    coeffs_sr: f32,
    /// Something was rendered into the buffers this chunk.
    pub written: bool,
    /// Frames since the last input.
    pub quiet: u32,
    gain_l: f32,
    gain_r: f32,
    pub peak: [f32; 2],
}

impl Strip {
    pub fn new(params: StripParams) -> Self {
        Self {
            params,
            buf_l: vec![0.0; BLOCK],
            buf_r: vec![0.0; BLOCK],
            filters: [Biquad::default(); 5],
            coeffs_sr: 0.0,
            written: false,
            quiet: u32::MAX / 2,
            gain_l: 0.0,
            gain_r: 0.0,
            peak: [0.0; 2],
        }
    }

    pub fn set_params(&mut self, p: StripParams) {
        self.params = p.clamped();
        self.coeffs_sr = 0.0;
    }

    /// Buffers to render into; zeroed lazily on the first write of a chunk.
    #[inline]
    pub fn target(&mut self, n: usize) -> (&mut [f32], &mut [f32]) {
        if !self.written {
            self.buf_l[..n].fill(0.0);
            self.buf_r[..n].fill(0.0);
            self.written = true;
        }
        (&mut self.buf_l[..n], &mut self.buf_r[..n])
    }

    /// Should this chunk be processed at all? Idle strips are skipped.
    pub fn prepare(&mut self, n: usize) -> bool {
        if self.written {
            self.quiet = 0;
            return true;
        }
        if self.quiet >= TAIL_FRAMES {
            self.peak = [0.0; 2];
            return false;
        }
        self.quiet = self.quiet.saturating_add(n as u32);
        self.buf_l[..n].fill(0.0);
        self.buf_r[..n].fill(0.0);
        true
    }

    fn update_coeffs(&mut self, sr: f32) {
        if self.coeffs_sr == sr {
            return;
        }
        let p = self.params;
        let q = std::f32::consts::FRAC_1_SQRT_2;
        self.filters[0].retune(Biquad::highpass(sr, p.hpf_hz, q));
        self.filters[1].retune(Biquad::low_shelf(sr, 200.0, p.eq_low_db));
        self.filters[2].retune(Biquad::peaking(sr, p.eq_mid_hz, 0.9, p.eq_mid_db));
        self.filters[3].retune(Biquad::high_shelf(sr, 5000.0, p.eq_high_db));
        self.filters[4].retune(Biquad::lowpass(sr, p.lpf_hz, q));
        self.coeffs_sr = sr;
    }

    /// EQ + filters in place over the first `n` frames.
    pub fn run_dsp(&mut self, n: usize, sr: f32) {
        self.update_coeffs(sr);
        for f in self.filters.iter_mut().filter(|f| f.active) {
            for i in 0..n {
                self.buf_l[i] = f.process(0, self.buf_l[i]);
                self.buf_r[i] = f.process(1, self.buf_r[i]);
            }
        }
    }

    /// Smoothed fader/pan gains, one step per sample.
    #[inline]
    pub fn step_gain(&mut self, target_l: f32, target_r: f32) -> (f32, f32) {
        const K: f32 = 0.0025;
        self.gain_l += (target_l - self.gain_l) * K;
        self.gain_r += (target_r - self.gain_r) * K;
        (self.gain_l, self.gain_r)
    }
}
