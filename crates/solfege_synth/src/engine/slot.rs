//! Voices (sample playback, envelope, filter) and slots (one instrument,
//! 16 MIDI channel states, voice pool, per-channel mixer strips).

use std::f32::consts::PI;
use std::sync::Arc;

use super::mixer::{DRUM_CHANNEL, MAX_STRIPS, NoteGroups, Strip, StripParams};
use super::{ChannelInfo, MAX_VOICES, MAX_TRIGGER, SlotParams};
use crate::instrument::{Instrument, Kind, LoopMode, Trigger, Zone};

// ---------------------------------------------------------------- envelope

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Stage {
    Delay,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
    Done,
}

#[derive(Clone, Copy)]
pub(super) struct Env {
    pub(super) stage: Stage,
    pub(super) level: f32,
    pub(super) counter: u32,
    pub(super) delay: u32,
    pub(super) attack_inc: f32,
    pub(super) hold: u32,
    pub(super) decay_coef: f32,
    pub(super) sustain: f32,
    pub(super) release_coef: f32,
}

const ENV_FLOOR: f32 = 1.0e-4;

pub(super) fn exp_coef(seconds: f32, sr: f32, target: f32) -> f32 {
    if seconds <= 0.0 { 0.0 } else { (target.ln() / (seconds * sr)).exp() }
}

impl Env {
    pub(super) fn new(z: &Zone, sr: f32) -> Self {
        let e = &z.env;
        Self {
            stage: Stage::Delay,
            level: 0.0,
            counter: 0,
            delay: (e.delay * sr) as u32,
            attack_inc: if e.attack > 0.0 { 1.0 / (e.attack * sr) } else { 2.0 },
            hold: (e.hold * sr) as u32,
            decay_coef: exp_coef(e.decay, sr, 0.001),
            sustain: e.sustain,
            release_coef: exp_coef(e.release.max(0.001), sr, ENV_FLOOR),
        }
    }

    pub(super) fn release(&mut self) {
        if self.stage != Stage::Done {
            self.stage = Stage::Release;
        }
    }

    pub(super) fn fast_release(&mut self, sr: f32) {
        self.release_coef = exp_coef(0.006, sr, ENV_FLOOR);
        self.release();
    }

    #[inline]
    pub(super) fn next(&mut self) -> f32 {
        match self.stage {
            Stage::Delay => {
                if self.counter >= self.delay {
                    self.counter = 0;
                    self.stage = Stage::Attack;
                } else {
                    self.counter += 1;
                }
            }
            Stage::Attack => {
                self.level += self.attack_inc;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Hold;
                }
            }
            Stage::Hold => {
                if self.counter >= self.hold {
                    self.stage = Stage::Decay;
                } else {
                    self.counter += 1;
                }
            }
            Stage::Decay => {
                self.level = self.sustain + (self.level - self.sustain) * self.decay_coef;
                if (self.level - self.sustain).abs() < 1.0e-4 {
                    self.level = self.sustain;
                    self.stage = Stage::Sustain;
                }
                if self.level < ENV_FLOOR {
                    self.stage = Stage::Done;
                }
            }
            Stage::Sustain => {
                if self.level < ENV_FLOOR {
                    self.stage = Stage::Done;
                }
            }
            Stage::Release => {
                self.level *= self.release_coef;
                if self.level < ENV_FLOOR {
                    self.level = 0.0;
                    self.stage = Stage::Done;
                }
            }
            Stage::Done => self.level = 0.0,
        }
        self.level
    }
}

// ------------------------------------------------------------------ filter

/// Topology-preserving-transform state variable low-pass (Simper), stereo.
#[derive(Clone, Copy, Default)]
pub(super) struct Svf {
    pub(super) a1: f32,
    pub(super) a2: f32,
    pub(super) a3: f32,
    pub(super) ic1: [f32; 2],
    pub(super) ic2: [f32; 2],
}

impl Svf {
    pub(super) fn lowpass(cutoff: f32, res_db: f32, sr: f32) -> Self {
        let fc = cutoff.clamp(10.0, sr * 0.45);
        let g = (PI * fc / sr).tan();
        let q = std::f32::consts::FRAC_1_SQRT_2 * 10f32.powf(res_db / 20.0);
        let k = 1.0 / q;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        Self { a1, a2, a3: g * a2, ..Self::default() }
    }

    #[inline]
    pub(super) fn process(&mut self, c: usize, x: f32) -> f32 {
        let v3 = x - self.ic2[c];
        let v1 = self.a1 * self.ic1[c] + self.a2 * v3;
        let v2 = self.ic2[c] + self.a2 * self.ic1[c] + self.a3 * v3;
        self.ic1[c] = 2.0 * v1 - self.ic1[c];
        self.ic2[c] = 2.0 * v2 - self.ic2[c];
        v2
    }
}

// ------------------------------------------------------------------- voice

#[derive(Clone, Copy)]
pub(super) struct Voice {
    pub(super) active: bool,
    pub(super) preset: usize,
    pub(super) zone: usize,
    pub(super) ch: u8,
    pub(super) note: u8,
    pub(super) pos: f64,
    pub(super) step: f64,
    pub(super) gain_l: f32,
    pub(super) gain_r: f32,
    pub(super) env: Env,
    pub(super) released: bool,
    pub(super) pedal_held: bool,
    pub(super) ignore_off: bool,
    pub(super) age: u64,
    pub(super) loop_mode: LoopMode,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) loop_start: usize,
    pub(super) loop_end: usize,
    pub(super) off_by: u32,
    pub(super) filter: Option<Svf>,
    /// Mixer strip this voice renders into.
    pub(super) strip: u8,
}

impl Voice {
    pub(super) fn idle() -> Self {
        Self {
            active: false,
            preset: 0,
            zone: 0,
            ch: 0,
            note: 0,
            pos: 0.0,
            step: 0.0,
            gain_l: 0.0,
            gain_r: 0.0,
            env: Env {
                stage: Stage::Done,
                level: 0.0,
                counter: 0,
                delay: 0,
                attack_inc: 0.0,
                hold: 0,
                decay_coef: 0.0,
                sustain: 0.0,
                release_coef: 0.0,
            },
            released: false,
            pedal_held: false,
            ignore_off: false,
            age: 0,
            loop_mode: LoopMode::NoLoop,
            start: 0,
            end: 0,
            loop_start: 0,
            loop_end: 0,
            off_by: 0,
            filter: None,
            strip: 0,
        }
    }

    #[inline]
    pub(super) fn looping(&self) -> bool {
        self.loop_mode == LoopMode::Continuous || (self.loop_mode == LoopMode::Sustain && !self.released)
    }

    /// Render `l.len()` frames, adding into `l`/`r`. `gain` is the live
    /// channel volume/expression, `bend` the live pitch ratio.
    pub(super) fn render(&mut self, zone: &Zone, bend: f64, gain: f32, l: &mut [f32], r: &mut [f32]) {
        let s = &*zone.sample;
        let looping = self.looping();
        let loop_len = (self.loop_end - self.loop_start) as isize;
        let (start, last) = (self.start as isize, self.end as isize - 1);
        let loop_end = self.loop_end as isize;
        let fetch = |j: isize| -> (f32, f32) {
            let mut j = j;
            if looping && j >= loop_end {
                j -= loop_len;
            }
            s.frame(j.clamp(start, last) as usize)
        };
        let step = self.step * bend;
        let (gl, gr) = (self.gain_l * gain, self.gain_r * gain);
        for i in 0..l.len() {
            let env = self.env.next();
            if self.env.stage == Stage::Done {
                self.active = false;
                return;
            }
            let ip = self.pos as isize;
            let t = (self.pos - ip as f64) as f32;
            let (x0l, x0r) = fetch(ip - 1);
            let (x1l, x1r) = fetch(ip);
            let (x2l, x2r) = fetch(ip + 1);
            let (x3l, x3r) = fetch(ip + 2);
            let mut sl = hermite(t, x0l, x1l, x2l, x3l);
            let mut sr = hermite(t, x0r, x1r, x2r, x3r);
            if let Some(f) = self.filter.as_mut() {
                sl = f.process(0, sl);
                sr = f.process(1, sr);
            }
            l[i] += sl * env * gl;
            r[i] += sr * env * gr;

            self.pos += step;
            if looping {
                if self.pos >= self.loop_end as f64 {
                    self.pos -= loop_len as f64;
                }
            } else if self.pos >= self.end as f64 {
                self.active = false;
                return;
            }
        }
    }
}

#[inline]
pub(super) fn hermite(t: f32, x0: f32, x1: f32, x2: f32, x3: f32) -> f32 {
    let c1 = 0.5 * (x2 - x0);
    let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
    let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
    ((c3 * t + c2) * t + c1) * t + x1
}


// -------------------------------------------------------------------- slot

/// Per-MIDI-channel controller state. A slot keeps all 16 so an Omni SF2
/// slot works as a 16-part multitimbral GM synth for the MIDI player.
#[derive(Clone, Copy)]
pub(super) struct Channel {
    /// Preset chosen by program change; `None` follows the slot preset.
    pub(super) preset: Option<usize>,
    pub(super) bank_msb: u8,
    pub(super) bank_lsb: u8,
    pub(super) bend: f32,
    pub(super) volume: f32,
    pub(super) expression: f32,
    pub(super) pan: f32,
    pub(super) sustain: bool,
    /// Last program number received, raw controller values for display.
    pub(super) program: Option<u8>,
    pub(super) fallback: bool,
    pub(super) volume_raw: u8,
    pub(super) pan_raw: u8,
    pub(super) expression_raw: u8,
    pub(super) bend_raw: i16,
    /// Rhythm part: channel 10 by default, others via GS/XG SysEx.
    pub(super) drum: bool,
    /// Preset pinned from the UI; wins over program changes and resets.
    pub(super) locked: Option<usize>,
    /// CC91 / CC93 send levels, 0..1.
    pub(super) reverb: f32,
    pub(super) chorus: f32,
}

impl Default for Channel {
    fn default() -> Self {
        Self {
            preset: None,
            bank_msb: 0,
            bank_lsb: 0,
            bend: 0.0,
            volume: 1.0,
            expression: 1.0,
            pan: 0.0,
            sustain: false,
            program: None,
            fallback: false,
            volume_raw: 127,
            pan_raw: 64,
            expression_raw: 127,
            bend_raw: 0,
            drum: false,
            locked: None,
            reverb: 40.0 / 127.0,
            chorus: 0.0,
        }
    }
}

/// Power-on channel state: GM puts the rhythm part on channel 10.
pub(super) fn default_channels() -> [Channel; 16] {
    let mut ch = [Channel::default(); 16];
    ch[DRUM_CHANNEL as usize].drum = true;
    ch
}

pub struct Slot {
    pub(super) inst: Arc<Instrument>,
    pub(super) preset: usize,
    pub(super) drum_preset: Option<usize>,
    pub params: SlotParams,
    pub(super) voices: Vec<Voice>,
    pub(super) trigger: Vec<usize>,
    pub(super) ch: [Channel; 16],
    pub(super) note_vel: [[u8; 128]; 16],
    pub(super) seq: u32,
    pub(super) rng: u32,
    pub(super) age: u64,
    pub(super) strips: Vec<Strip>,
    pub(super) groups: NoteGroups,
    pub(super) peak: [f32; 2],
    /// Preset per program on filtered channels; see `SlotParams::filtered`.
    pub(super) program_map: Box<[u16; 128]>,
}

/// Map Bank Select MSB/LSB + Program Change onto an SF2 preset, accepting the
/// common conventions: GM (channel 10 = drums), GS (bank = MSB), XG (MSB
/// 127/126/120 = drum kits, LSB = variation) and the MSB*128+LSB "MMA" form.
/// Missing presets fall back the way hardware does: same program in bank 0,
/// then the standard kit for drums, then any bank with that program.
/// Returns the preset index and whether it was an exact match.
pub fn resolve_preset(inst: &Instrument, drum_part: bool, msb: u8, lsb: u8, program: u8) -> Option<(usize, bool)> {
    let find = |bank: u16| inst.find_preset(bank, program);
    let drums = drum_part || matches!(msb, 120 | 126 | 127);
    // Fixed-size candidate list: this runs on the audio thread.
    let mut exact = [u16::MAX; 3];
    if drums {
        exact[0] = 128;
        if msb != 0 {
            exact[1] = msb as u16;
        }
    } else if msb == 0 && lsb != 0 {
        // XG: MSB 0 = normal voices, LSB picks the variation bank.
        exact[0] = lsb as u16;
    } else {
        exact[0] = msb as u16;
        if lsb != 0 {
            exact[1] = msb as u16 * 128 + lsb as u16;
            exact[2] = lsb as u16;
        }
    }
    if let Some(i) = exact.iter().filter(|&&b| b != u16::MAX).find_map(|&b| find(b)) {
        return Some((i, true));
    }
    let fallback = if drums {
        inst.find_preset(128, 0).or_else(|| inst.presets.iter().position(|p| p.bank == 128))
    } else {
        find(0).or_else(|| inst.presets.iter().position(|p| p.program == program && p.bank < 128))
    };
    fallback.map(|i| (i, false))
}

pub(super) fn find_drum_preset(inst: &Instrument) -> Option<usize> {
    if inst.kind != Kind::Sf2 {
        return None;
    }
    inst.find_preset(128, 0).or_else(|| inst.presets.iter().position(|p| p.bank == 128))
}

impl Slot {
    /// Slot with default mixer strips and GM note groups.
    pub fn new(inst: Arc<Instrument>, preset: usize, params: SlotParams) -> Self {
        Self::with_mixer(inst, preset, params, &[StripParams::default(); MAX_STRIPS], NoteGroups::gm())
    }

    /// Build a slot with its mixer strips and channel-10 note groups.
    pub fn with_mixer(
        inst: Arc<Instrument>,
        preset: usize,
        params: SlotParams,
        strips: &[StripParams; MAX_STRIPS],
        groups: NoteGroups,
    ) -> Self {
        Self {
            drum_preset: find_drum_preset(&inst),
            inst,
            preset,
            params,
            voices: vec![Voice::idle(); MAX_VOICES],
            trigger: Vec::with_capacity(MAX_TRIGGER),
            ch: default_channels(),
            note_vel: [[0; 128]; 16],
            seq: 0,
            rng: 0x9E37_79B9,
            age: 0,
            strips: strips.iter().map(|&p| Strip::new(p.clamped())).collect(),
            groups,
            program_map: Box::new([super::NO_PRESET; 128]),
            peak: [0.0; 2],
        }
    }

    /// Strip for a new voice: channel 10 notes go to their note group.
    fn strip_for(&self, ch: u8, key: u8) -> u8 {
        if ch == DRUM_CHANNEL
            && let Some(s) = self.groups.strip_for(key)
        {
            return s as u8;
        }
        ch
    }

    pub(super) fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// GM convention: on a multitimbral slot channel 10 defaults to the drum kit.
    /// Program a channel plays (GM default: 0).
    fn program(&self, ch: u8) -> u8 {
        self.ch[ch as usize].program.unwrap_or(0) & 127
    }

    /// Whether new notes on `ch` belong to this slot (program filter).
    pub(super) fn accepts(&self, ch: u8) -> bool {
        let p = &self.params;
        p.filtered & (1 << (ch & 15)) == 0 || p.programs & (1u128 << self.program(ch)) != 0
    }

    pub(super) fn preset_for(&self, ch: u8) -> usize {
        if let Some(p) = self.ch[ch as usize].locked {
            return p;
        }
        if self.params.filtered & (1 << (ch & 15)) != 0 {
            let mapped = self.program_map[self.program(ch) as usize];
            if mapped != super::NO_PRESET && (mapped as usize) < self.inst.presets.len() {
                return mapped as usize;
            }
        }
        if let Some(p) = self.ch[ch as usize].preset {
            return p;
        }
        if self.params.multitimbral()
            && self.ch[ch as usize].drum
            && let Some(d) = self.drum_preset
        {
            return d;
        }
        self.preset
    }

    pub(super) fn alloc_voice(&mut self) -> usize {
        if let Some(i) = self.voices.iter().position(|v| !v.active) {
            return i;
        }
        // Steal: oldest released voice first, otherwise the oldest voice.
        let pick = |released: bool| {
            self.voices
                .iter()
                .enumerate()
                .filter(|(_, v)| v.released == released)
                .min_by_key(|(_, v)| v.age)
                .map(|(i, _)| i)
        };
        pick(true).or_else(|| pick(false)).unwrap_or(0)
    }

    pub(super) fn note_on(&mut self, ch: u8, key: u8, vel: u8, sr: f32) {
        let p = self.params;
        if !self.accepts(ch) {
            return;
        }
        if key < p.key_lo || key > p.key_hi || vel < p.vel_lo || vel > p.vel_hi {
            return;
        }
        if self.ch[ch as usize].drum && p.drum_keys & (1u128 << (key & 127)) == 0 {
            return;
        }
        self.note_vel[ch as usize][key as usize] = vel;
        self.seq = self.seq.wrapping_add(1);
        let rnd = self.random();
        self.start_zones(ch, key, vel, Trigger::Attack, rnd, sr);
    }

    pub(super) fn note_off(&mut self, ch: u8, key: u8, sr: f32) {
        let sustain = self.ch[ch as usize].sustain;
        for v in self.voices.iter_mut().filter(|v| v.active && v.ch == ch && v.note == key && !v.released) {
            if v.ignore_off {
                continue;
            }
            if sustain {
                v.pedal_held = true;
            } else {
                v.released = true;
                v.env.release();
            }
        }
        let vel = self.note_vel[ch as usize][key as usize];
        if vel > 0 {
            self.note_vel[ch as usize][key as usize] = 0;
            let rnd = self.random();
            self.start_zones(ch, key, vel, Trigger::Release, rnd, sr);
        }
    }

    pub(super) fn start_zones(&mut self, ch: u8, key: u8, vel: u8, trigger: Trigger, rnd: f32, sr: f32) {
        let inst = self.inst.clone();
        let preset_idx = self.preset_for(ch);
        let Some(preset) = inst.presets.get(preset_idx) else { return };
        let played = (key as i32 + self.params.transpose).clamp(0, 127) as u8;

        self.trigger.clear();
        for (zi, z) in preset.zones.iter().enumerate() {
            if z.trigger != trigger || !z.matches(played, vel) {
                continue;
            }
            if z.seq_length > 1 && (self.seq.wrapping_sub(1) % z.seq_length) + 1 != z.seq_position {
                continue;
            }
            if rnd < z.lorand || (rnd >= z.hirand && z.hirand < 1.0) {
                continue;
            }
            if self.trigger.len() < MAX_TRIGGER {
                self.trigger.push(zi);
            }
        }

        // Choke groups: new notes in group G cut voices with off_by == G.
        for t in 0..self.trigger.len() {
            let g = preset.zones[self.trigger[t]].group;
            if g == 0 {
                continue;
            }
            for v in self.voices.iter_mut().filter(|v| v.active && v.ch == ch && v.off_by == g) {
                v.released = true;
                v.env.fast_release(sr);
            }
        }

        let velf = vel as f32 / 127.0;
        let ch_pan = self.ch[ch as usize].pan;
        let strip = self.strip_for(ch, key);
        for t in 0..self.trigger.len() {
            let zi = self.trigger[t];
            let z = &preset.zones[zi];
            let cents = (played as f32 - z.root) * z.keytrack + z.tune + self.params.tune;
            let step = 2f64.powf(cents as f64 / 1200.0) * z.sample_rate as f64 / sr as f64;
            let vel_gain = 1.0 - z.veltrack + z.veltrack * velf * velf;
            let pan = (z.pan + ch_pan).clamp(-1.0, 1.0);
            let gain = z.gain * vel_gain;
            self.age += 1;
            let vi = self.alloc_voice();
            self.voices[vi] = Voice {
                active: true,
                preset: preset_idx,
                zone: zi,
                ch,
                note: key,
                pos: z.start as f64,
                step,
                gain_l: gain * (1.0 - pan).min(1.0),
                gain_r: gain * (1.0 + pan).min(1.0),
                env: Env::new(z, sr),
                released: false,
                pedal_held: false,
                ignore_off: z.loop_mode == LoopMode::OneShot || trigger == Trigger::Release,
                age: self.age,
                loop_mode: z.loop_mode,
                start: z.start,
                end: z.end,
                loop_start: z.loop_start,
                loop_end: z.loop_end,
                off_by: z.off_by,
                filter: z.cutoff.map(|c| Svf::lowpass(c, z.resonance_db, sr)),
                strip,
            };
        }
    }

    pub(super) fn set_sustain(&mut self, ch: u8, on: bool) {
        self.ch[ch as usize].sustain = on;
        if !on {
            for v in self.voices.iter_mut().filter(|v| v.active && v.ch == ch && v.pedal_held) {
                v.pedal_held = false;
                v.released = true;
                v.env.release();
            }
        }
    }

    pub(super) fn release_channel(&mut self, ch: u8) {
        self.ch[ch as usize].sustain = false;
        self.note_vel[ch as usize] = [0; 128];
        for v in self.voices.iter_mut().filter(|v| v.active && v.ch == ch) {
            v.released = true;
            v.pedal_held = false;
            v.env.release();
        }
    }

    pub(super) fn release_all(&mut self) {
        for ch in 0..16 {
            self.release_channel(ch);
        }
    }

    /// Quick fade instead of a hard cut, used when the player seeks.
    pub(super) fn fade_all(&mut self, sr: f32) {
        for v in self.voices.iter_mut().filter(|v| v.active) {
            v.released = true;
            v.pedal_held = false;
            v.env.fast_release(sr);
        }
        self.note_vel = [[0; 128]; 16];
    }

    pub(super) fn kill_all(&mut self) {
        for v in &mut self.voices {
            v.active = false;
        }
        for c in &mut self.ch {
            c.sustain = false;
        }
        self.note_vel = [[0; 128]; 16];
    }

    pub(super) fn reset_channels(&mut self) {
        let locks = self.ch.map(|c| c.locked);
        self.ch = default_channels();
        for (c, lock) in self.ch.iter_mut().zip(locks) {
            c.locked = lock;
        }
    }

    /// Internal system messages decoded from SysEx (see `smf::parse_sysex`).
    pub(super) fn system(&mut self, kind: u8, arg: u8) {
        match kind {
            crate::smf::SYS_RESET => {
                self.release_all();
                self.reset_channels();
            }
            crate::smf::SYS_DRUM_PART => {
                let c = &mut self.ch[(arg & 0x0F) as usize];
                c.drum = arg & 0x10 != 0;
                c.preset = None;
                c.fallback = false;
            }
            _ => {}
        }
    }

    pub(super) fn program_change(&mut self, ch: u8, program: u8) {
        self.ch[ch as usize].program = Some(program);
        if self.inst.kind != Kind::Sf2 {
            return;
        }
        let c = self.ch[ch as usize];
        let Some((i, exact)) = resolve_preset(&self.inst, c.drum, c.bank_msb, c.bank_lsb, program) else { return };
        self.ch[ch as usize].fallback = !exact;
        if self.params.multitimbral() {
            self.ch[ch as usize].preset = Some(i);
        } else {
            // Single-channel slot: program change selects the slot preset.
            self.preset = i;
        }
    }

    pub(super) fn channel_info(&self, ch: u8, voices: u8) -> ChannelInfo {
        let c = &self.ch[ch as usize];
        ChannelInfo {
            preset: self.preset_for(ch),
            explicit: c.preset.is_some() || c.locked.is_some(),
            locked: c.locked.is_some(),
            fallback: c.fallback,
            program: c.program,
            bank_msb: c.bank_msb,
            bank_lsb: c.bank_lsb,
            volume: c.volume_raw,
            pan: c.pan_raw,
            expression: c.expression_raw,
            reverb: (c.reverb * 127.0).round() as u8,
            chorus: (c.chorus * 127.0).round() as u8,
            bend: c.bend_raw,
            sustain: c.sustain,
            drum: c.drum,
            voices,
        }
    }

    pub(super) fn midi(&mut self, msg: [u8; 3], sr: f32) {
        let ch = msg[0] & 0x0F;
        let c = ch as usize;
        match msg[0] & 0xF0 {
            0x90 if msg[2] > 0 => self.note_on(ch, msg[1] & 0x7F, msg[2] & 0x7F, sr),
            0x80 | 0x90 => self.note_off(ch, msg[1] & 0x7F, sr),
            0xB0 => {
                let v = msg[2] as f32 / 127.0;
                match msg[1] {
                    0 => self.ch[c].bank_msb = msg[2],
                    32 => self.ch[c].bank_lsb = msg[2],
                    7 => {
                        self.ch[c].volume = v * v;
                        self.ch[c].volume_raw = msg[2];
                    }
                    10 => {
                        self.ch[c].pan = (msg[2] as f32 - 64.0) / 63.0;
                        self.ch[c].pan_raw = msg[2];
                    }
                    11 => {
                        self.ch[c].expression = v * v;
                        self.ch[c].expression_raw = msg[2];
                    }
                    64 => self.set_sustain(ch, msg[2] >= 64),
                    91 => self.ch[c].reverb = v,
                    93 => self.ch[c].chorus = v,
                    120 => {
                        for v in self.voices.iter_mut().filter(|v| v.ch == ch) {
                            v.active = false;
                        }
                    }
                    121 => {
                        self.set_sustain(ch, false);
                        let keep = self.ch[c];
                        self.ch[c] = Channel {
                            preset: keep.preset,
                            bank_msb: keep.bank_msb,
                            bank_lsb: keep.bank_lsb,
                            volume: keep.volume,
                            volume_raw: keep.volume_raw,
                            pan: keep.pan,
                            pan_raw: keep.pan_raw,
                            program: keep.program,
                            fallback: keep.fallback,
                            ..Channel::default()
                        };
                    }
                    123 => self.release_channel(ch),
                    _ => {}
                }
            }
            0xC0 => self.program_change(ch, msg[1] & 0x7F),
            0xF0 => self.system(msg[1], msg[2]),
            0xE0 => {
                let raw = ((msg[2] as i32) << 7 | msg[1] as i32) - 8192;
                self.ch[c].bend = raw as f32 / 8192.0;
                self.ch[c].bend_raw = raw as i16;
            }
            _ => {}
        }
    }

    pub(super) fn listens(&self, ch: u8) -> bool {
        self.params.receives(ch)
    }

    /// Channel used for notes played directly on this slot.
    pub(super) fn home_channel(&self) -> u8 {
        self.params.home_channel()
    }

    /// Render all voices of this chunk into their mixer strips.
    pub(super) fn render(&mut self, n: usize) {
        for s in &mut self.strips {
            s.written = false;
        }
        let range = self.params.bend_range;
        let bends: [f64; 16] = std::array::from_fn(|c| 2f64.powf((self.ch[c].bend * range) as f64 / 12.0));
        let gains: [f32; 16] = std::array::from_fn(|c| self.ch[c].volume * self.ch[c].expression);
        let inst = &self.inst;
        let strips = &mut self.strips;
        for v in self.voices.iter_mut().filter(|v| v.active) {
            let Some(zone) = inst.presets.get(v.preset).and_then(|p| p.zones.get(v.zone)) else {
                v.active = false;
                continue;
            };
            let c = v.ch as usize;
            let (l, r) = strips[v.strip as usize].target(n);
            v.render(zone, bends[c], gains[c], l, r);
        }
    }
}

