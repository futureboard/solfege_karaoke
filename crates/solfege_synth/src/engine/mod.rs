//! Realtime side: the rack of instrument slots, their voices and the mixer.
//! Runs inside the audio callback; talks to the UI only through a bounded
//! command channel, a garbage channel (so deallocation happens off the audio
//! thread) and atomics for metering.

mod dsp;
pub mod mixer;
mod slot;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crossbeam_channel::{Receiver, Sender};

use crate::instrument::Instrument;
use crate::smf::Song;
use dsp::{Chorus, Reverb};
use mixer::{BLOCK, DRUM_CHANNEL, DRUM_STRIP_BASE, FxParams, MAX_BUSES, MAX_STRIPS, NoteGroups, StripParams};
use slot::find_drum_preset;
pub use slot::{Slot, resolve_preset};

pub const MAX_SLOTS: usize = 16;
pub const OMNI: u16 = 0xFFFF;
/// All channels except 10, for splitting drums onto another slot.
pub const NO_DRUMS: u16 = OMNI & !(1 << 9);
pub const MAX_VOICES: usize = 96;
const MAX_TRIGGER: usize = 256;

#[derive(Clone, Copy, Debug)]
pub struct SlotParams {
    pub gain: f32,
    pub pan: f32,
    /// MIDI channels this slot receives: bit n = channel n+1.
    /// `OMNI` = all 16; one bit = classic single-channel slot.
    pub channels: u16,
    pub transpose: i32,
    pub tune: f32,
    pub key_lo: u8,
    pub key_hi: u8,
    pub vel_lo: u8,
    pub vel_hi: u8,
    pub bend_range: f32,
    pub mute: bool,
    pub solo: bool,
    /// Channels whose notes this slot plays only while the channel's
    /// current program is in `programs` (bit n = program n). Lets several
    /// slots share a channel and split it by instrument.
    pub filtered: u16,
    pub programs: u128,
}

/// No program -> preset override (see [`Command::SetProgramMap`]).
pub const NO_PRESET: u16 = u16::MAX;

impl SlotParams {
    pub fn receives(&self, ch: u8) -> bool {
        self.channels & (1 << (ch & 15)) != 0
    }

    /// The only channel received, when exactly one is enabled.
    pub fn single(&self) -> Option<u8> {
        (self.channels.count_ones() == 1).then(|| self.channels.trailing_zeros() as u8)
    }

    /// More than one channel: each keeps its own program (multitimbral).
    pub fn multitimbral(&self) -> bool {
        self.channels.count_ones() > 1
    }

    /// Channel used for notes played directly on the slot: the first enabled one.
    pub fn home_channel(&self) -> u8 {
        if self.channels == 0 { 0 } else { self.channels.trailing_zeros() as u8 }
    }
}

impl Default for SlotParams {
    fn default() -> Self {
        Self {
            gain: 1.0,
            pan: 0.0,
            channels: OMNI,
            transpose: 0,
            tune: 0.0,
            key_lo: 0,
            key_hi: 127,
            vel_lo: 1,
            vel_hi: 127,
            bend_range: 2.0,
            mute: false,
            solo: false,
            filtered: 0,
            programs: u128::MAX,
        }
    }
}

pub enum Command {
    AddSlot(Box<Slot>),
    RemoveSlot(usize),
    SetInstrument { slot: usize, inst: Arc<Instrument>, preset: usize, keep_voices: bool },
    SetParams { slot: usize, params: SlotParams },
    SetPreset { slot: usize, preset: usize },
    /// Raw channel message, routed to every slot listening on its channel.
    Midi([u8; 3]),
    /// Note sent straight to one slot (computer keyboard); vel 0 = off.
    Note { slot: usize, key: u8, vel: u8 },
    Panic,
    MasterGain(f32),
    LoadSong(Arc<Song>),
    Play,
    Pause,
    Stop,
    Seek(f64),
    SetLoop(bool),
    SetSpeed(f64),
    /// Bit per channel; muted channels drop note-ons from the player.
    ChannelMutes(u16),
    /// Pin a preset on one channel of one slot (`None` = unpin). Pins
    /// survive program changes and GM/GS/XG resets.
    SetChannelPreset { slot: usize, ch: u8, preset: Option<usize> },
    SetStrip { slot: usize, strip: usize, params: StripParams },
    SetNoteGroups { slot: usize, groups: NoteGroups },
    SetFx(FxParams),
    /// Preset to use per program number on the slot's filtered channels
    /// (`NO_PRESET` = resolve the program as usual).
    SetProgramMap { slot: usize, map: Box<[u16; 128]> },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlayState {
    Empty = 0,
    Stopped = 1,
    Playing = 2,
    Paused = 3,
}

impl PlayState {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => PlayState::Stopped,
            2 => PlayState::Playing,
            3 => PlayState::Paused,
            _ => PlayState::Empty,
        }
    }
}

/// Values the audio thread hands back for dropping on the UI thread.
pub enum Garbage {
    Slot(#[allow(dead_code)] Box<Slot>),
    Inst(#[allow(dead_code)] Arc<Instrument>),
    Song(#[allow(dead_code)] Arc<Song>),
    ProgramMap(#[allow(dead_code)] Box<[u16; 128]>),
}

/// Packed per-channel state of a slot, published for the Channels view.
pub struct ChannelMeter {
    pub a: AtomicU64,
    pub b: AtomicU64,
}

impl Default for ChannelMeter {
    fn default() -> Self {
        let m = Self { a: AtomicU64::new(0), b: AtomicU64::new(0) };
        m.store(&ChannelInfo { volume: 127, pan: 64, expression: 127, ..ChannelInfo::default() });
        m
    }
}

/// Decoded `ChannelMeter`.
#[derive(Clone, Copy, Debug, Default)]
pub struct ChannelInfo {
    /// Preset actually used for new notes.
    pub preset: usize,
    /// Set by program change / manual assignment (vs. slot default).
    pub explicit: bool,
    /// Requested bank:program was missing; a fallback preset is used.
    pub fallback: bool,
    pub program: Option<u8>,
    pub bank_msb: u8,
    pub bank_lsb: u8,
    pub volume: u8,
    pub pan: u8,
    pub expression: u8,
    pub bend: i16,
    pub sustain: bool,
    /// Rhythm (drum) part.
    pub drum: bool,
    /// Preset pinned from the UI.
    pub locked: bool,
    pub voices: u8,
}

impl ChannelMeter {
    fn store(&self, c: &ChannelInfo) {
        let a = (c.preset as u64 & 0xFFFF)
            | (c.program.map(|p| p as u64).unwrap_or(0xFF) << 16)
            | ((c.bank_msb as u64) << 24)
            | ((c.bank_lsb as u64) << 32)
            | ((c.volume as u64) << 40)
            | ((c.pan as u64) << 48)
            | ((c.expression as u64) << 56);
        let b = ((c.bend as i32 + 8192) as u64 & 0xFFFF)
            | ((c.sustain as u64) << 16)
            | ((c.fallback as u64) << 17)
            | ((c.explicit as u64) << 18)
            | ((c.drum as u64) << 19)
            | ((c.locked as u64) << 20)
            | ((c.voices as u64) << 24);
        self.a.store(a, Ordering::Relaxed);
        self.b.store(b, Ordering::Relaxed);
    }

    pub fn load(&self) -> ChannelInfo {
        let a = self.a.load(Ordering::Relaxed);
        let b = self.b.load(Ordering::Relaxed);
        let program = ((a >> 16) & 0xFF) as u8;
        ChannelInfo {
            preset: (a & 0xFFFF) as usize,
            program: (program != 0xFF).then_some(program),
            bank_msb: (a >> 24) as u8,
            bank_lsb: (a >> 32) as u8,
            volume: (a >> 40) as u8,
            pan: (a >> 48) as u8,
            expression: (a >> 56) as u8,
            bend: ((b & 0xFFFF) as i32 - 8192) as i16,
            sustain: b & (1 << 16) != 0,
            fallback: b & (1 << 17) != 0,
            explicit: b & (1 << 18) != 0,
            drum: b & (1 << 19) != 0,
            locked: b & (1 << 20) != 0,
            voices: (b >> 24) as u8,
        }
    }
}

#[derive(Default)]
pub struct SlotMeter {
    pub peak_l: AtomicU32,
    pub peak_r: AtomicU32,
    pub voices: AtomicU32,
    pub preset: AtomicU32,
    pub notes: [AtomicU64; 2],
    pub channels: [ChannelMeter; 16],
    pub strip_l: [AtomicU32; MAX_STRIPS],
    pub strip_r: [AtomicU32; MAX_STRIPS],
    /// Bit per strip that had signal during the last second.
    pub strips_live: AtomicU32,
}

pub struct Shared {
    pub slots: [SlotMeter; MAX_SLOTS],
    pub master_l: AtomicU32,
    pub master_r: AtomicU32,
    pub cpu: AtomicU32,
    pub voices: AtomicU32,
    pub errors: AtomicU32,
    /// Player position in seconds (f64 bits) and `PlayState`.
    pub player_time: AtomicU64,
    pub player_state: AtomicU32,
    /// Decaying note-on velocity (0..1, f32 bits) per channel from the player.
    pub channel_activity: [AtomicU32; 16],
    pub bus_l: [AtomicU32; MAX_BUSES],
    pub bus_r: [AtomicU32; MAX_BUSES],
    /// Send levels into reverb / chorus.
    pub fx_peak: [AtomicU32; 2],
    /// Stereo pairs of the open output device.
    pub out_pairs: AtomicU32,
    /// Forward player events to the MIDI output (drained by a MIDI thread).
    pub forward_player: AtomicBool,
    pub out_tx: Sender<([u8; 3], u8)>,
    pub out_rx: Receiver<([u8; 3], u8)>,
}

impl Shared {
    pub fn new() -> Self {
        let (out_tx, out_rx) = crossbeam_channel::bounded(4096);
        Self {
            slots: std::array::from_fn(|_| SlotMeter::default()),
            master_l: AtomicU32::new(0),
            master_r: AtomicU32::new(0),
            cpu: AtomicU32::new(0),
            voices: AtomicU32::new(0),
            errors: AtomicU32::new(0),
            player_time: AtomicU64::new(0),
            player_state: AtomicU32::new(0),
            channel_activity: std::array::from_fn(|_| AtomicU32::new(0)),
            bus_l: std::array::from_fn(|_| AtomicU32::new(0)),
            bus_r: std::array::from_fn(|_| AtomicU32::new(0)),
            fx_peak: std::array::from_fn(|_| AtomicU32::new(0)),
            out_pairs: AtomicU32::new(1),
            forward_player: AtomicBool::new(false),
            out_tx,
            out_rx,
        }
    }

    pub fn reset(&self) {
        for m in &self.slots {
            m.peak_l.store(0, Ordering::Relaxed);
            m.peak_r.store(0, Ordering::Relaxed);
            m.voices.store(0, Ordering::Relaxed);
            m.notes[0].store(0, Ordering::Relaxed);
            m.notes[1].store(0, Ordering::Relaxed);
        }
        self.voices.store(0, Ordering::Relaxed);
    }
}

/// Peak meter with decay applied on the audio thread, so any number of
/// readers (TUI, WebUI) can simply load the value.
#[inline]
fn hold(a: &AtomicU32, v: f32, decay: f32) {
    let old = f32::from_bits(a.load(Ordering::Relaxed));
    a.store(v.max(old * decay).max(0.0).to_bits(), Ordering::Relaxed);
}

pub fn load_peak(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Ordering::Relaxed))
}

// ------------------------------------------------------------------ player

struct Player {
    song: Option<Arc<Song>>,
    time: f64,
    idx: usize,
    playing: bool,
    looping: bool,
    speed: f64,
    mutes: u16,
}

// ------------------------------------------------------------------ engine

/// Largest block `render` handles at once; callers split bigger buffers.
pub const MAX_FRAMES: usize = 4096;

struct Bus {
    l: Vec<f32>,
    r: Vec<f32>,
}

impl Bus {
    fn new() -> Self {
        Self { l: vec![0.0; MAX_FRAMES], r: vec![0.0; MAX_FRAMES] }
    }

    fn clear(&mut self, n: usize) {
        self.l[..n].fill(0.0);
        self.r[..n].fill(0.0);
    }
}

pub struct Engine {
    sr: f32,
    // Boxed so slots arrive and leave as one pointer; no allocation or
    // deallocation happens on the audio thread when the rack changes.
    #[allow(clippy::vec_box)]
    slots: Vec<Box<Slot>>,
    rx: Receiver<Command>,
    garbage: Sender<Garbage>,
    shared: Arc<Shared>,
    master: f32,
    player: Player,
    /// Stereo pairs the device really has; buses beyond fold into main.
    out_pairs: usize,
    buses: Vec<Bus>,
    reverb_in: Bus,
    chorus_in: Bus,
    reverb: Reverb,
    chorus: Chorus,
    fx: FxParams,
}

impl Engine {
    pub fn new(sr: f32, rx: Receiver<Command>, garbage: Sender<Garbage>, shared: Arc<Shared>) -> Self {
        shared.reset();
        Self {
            sr,
            slots: Vec::with_capacity(MAX_SLOTS),
            rx,
            garbage,
            shared,
            master: 1.0,
            player: Player { song: None, time: 0.0, idx: 0, playing: false, looping: false, speed: 1.0, mutes: 0 },
            out_pairs: 1,
            buses: (0..MAX_BUSES).map(|_| Bus::new()).collect(),
            reverb_in: Bus::new(),
            chorus_in: Bus::new(),
            reverb: Reverb::new(sr),
            chorus: Chorus::new(sr),
            fx: FxParams::default(),
        }
    }

    pub fn set_out_pairs(&mut self, pairs: usize) {
        self.out_pairs = pairs.clamp(1, MAX_BUSES);
        self.shared.out_pairs.store(self.out_pairs as u32, Ordering::Relaxed);
    }

    fn trash(&self, g: Garbage) {
        // If the UI is not draining, dropping here is the lesser evil.
        let _ = self.garbage.try_send(g);
    }

    fn route(&mut self, msg: [u8; 3]) {
        let sr = self.sr;
        if msg[0] == 0xF0 {
            // Decoded SysEx is system-wide: every slot hears it.
            self.slots.iter_mut().for_each(|s| s.system(msg[1], msg[2]));
            return;
        }
        let ch = msg[0] & 0x0F;
        for s in self.slots.iter_mut().filter(|s| s.listens(ch)) {
            s.midi(msg, sr);
        }
    }

    fn forward(&self, msg: [u8; 3], len: u8) {
        if msg[0] != 0xF0 && self.shared.forward_player.load(Ordering::Relaxed) {
            let _ = self.shared.out_tx.try_send((msg, len));
        }
    }

    fn forward_notes_off(&self) {
        for ch in 0..16u8 {
            self.forward([0xB0 | ch, 64, 0], 3);
            self.forward([0xB0 | ch, 123, 0], 3);
        }
    }

    /// Send a message to the rack and (optionally) the MIDI output.
    fn send_both(&mut self, msg: [u8; 3], len: u8) {
        self.route(msg);
        self.forward(msg, len);
    }

    fn emit_player(&mut self, msg: [u8; 3], len: u8) {
        let ch = msg[0] & 0x0F;
        if msg[0] & 0xF0 == 0x90 && msg[2] > 0 {
            if self.player.mutes & (1 << ch) != 0 {
                return;
            }
            hold(&self.shared.channel_activity[ch as usize], msg[2] as f32 / 127.0, 1.0);
        }
        self.send_both(msg, len);
    }

    fn player_release(&mut self) {
        self.slots.iter_mut().for_each(|s| s.release_all());
        self.forward_notes_off();
    }

    fn player_stop(&mut self) {
        self.player.playing = false;
        self.player_release();
        self.slots.iter_mut().for_each(|s| s.reset_channels());
        self.player.time = 0.0;
        self.player.idx = 0;
    }

    /// Jump to `t` seconds, re-sending the state ("chase") that the skipped
    /// part of the song would have set up: SysEx first, then controllers.
    fn seek(&mut self, t: f64) {
        let Some(song) = self.player.song.clone() else { return };
        let t = t.clamp(0.0, song.duration);
        let sr = self.sr;
        for s in self.slots.iter_mut() {
            s.fade_all(sr);
            s.reset_channels();
        }
        self.forward_notes_off();
        let idx = song.events.partition_point(|e| e.time < t);

        let mut program = [None::<u8>; 16];
        let mut cc = [[None::<u8>; 128]; 16];
        let mut bend = [None::<(u8, u8)>; 16];
        for e in &song.events[..idx] {
            let c = (e.msg[0] & 0x0F) as usize;
            match e.msg[0] & 0xF0 {
                0xF0 => {
                    if e.msg[1] == crate::smf::SYS_RESET {
                        program = [None; 16];
                        cc = [[None; 128]; 16];
                        bend = [None; 16];
                    }
                    self.route(e.msg);
                }
                0xC0 => program[c] = Some(e.msg[1]),
                0xB0 if !matches!(e.msg[1], 64 | 120..=127) => cc[c][e.msg[1] as usize & 0x7F] = Some(e.msg[2]),
                0xE0 => bend[c] = Some((e.msg[1], e.msg[2])),
                _ => {}
            }
        }
        for c in 0..16u8 {
            let ci = c as usize;
            // Bank select has to land before the program change it qualifies.
            for num in [0u8, 32] {
                if let Some(v) = cc[ci][num as usize] {
                    self.send_both([0xB0 | c, num, v], 3);
                }
            }
            if let Some(p) = program[ci] {
                self.send_both([0xC0 | c, p, 0], 2);
            }
            for num in (1..120u8).filter(|&n| n != 32) {
                if let Some(v) = cc[ci][num as usize] {
                    self.send_both([0xB0 | c, num, v], 3);
                }
            }
            if let Some((lsb, msb)) = bend[ci] {
                self.send_both([0xE0 | c, lsb, msb], 3);
            }
        }
        self.player.idx = idx;
        self.player.time = t;
    }

    fn dispatch_due(&mut self) {
        let Some(song) = self.player.song.clone() else {
            self.player.playing = false;
            return;
        };
        while let Some(&ev) = song.events.get(self.player.idx) {
            if ev.time > self.player.time {
                break;
            }
            self.player.idx += 1;
            self.emit_player(ev.msg, ev.len);
        }
        if self.player.idx >= song.events.len() && self.player.time >= song.duration {
            if self.player.looping && song.duration > 0.05 {
                self.player_release();
                self.seek(0.0);
            } else {
                self.player_stop();
            }
        }
    }

    fn handle(&mut self, cmd: Command) {
        let sr = self.sr;
        match cmd {
            Command::AddSlot(slot) => {
                if self.slots.len() < MAX_SLOTS {
                    self.slots.push(slot);
                } else {
                    self.trash(Garbage::Slot(slot));
                }
            }
            Command::RemoveSlot(i) => {
                if i < self.slots.len() {
                    let s = self.slots.remove(i);
                    self.trash(Garbage::Slot(s));
                }
            }
            Command::SetInstrument { slot, inst, preset, keep_voices } => {
                if let Some(s) = self.slots.get_mut(slot) {
                    if !keep_voices {
                        s.kill_all();
                        s.reset_channels();
                    }
                    s.preset = preset.min(inst.presets.len().saturating_sub(1));
                    s.drum_preset = find_drum_preset(&inst);
                    let old = std::mem::replace(&mut s.inst, inst);
                    self.trash(Garbage::Inst(old));
                }
            }
            Command::SetParams { slot, params } => {
                if let Some(s) = self.slots.get_mut(slot) {
                    // Channels dropped from the mask must not leave notes hanging.
                    let dropped = s.params.channels & !params.channels;
                    s.params = params;
                    for ch in (0..16u8).filter(|c| dropped & (1 << c) != 0) {
                        s.release_channel(ch);
                    }
                }
            }
            Command::SetChannelPreset { slot, ch, preset } => {
                if let Some(s) = self.slots.get_mut(slot) {
                    let ch = ch.min(15);
                    let preset = preset.filter(|&p| p < s.inst.presets.len());
                    let c = &mut s.ch[ch as usize];
                    c.locked = preset;
                    c.fallback = false;
                    if preset.is_none() {
                        c.preset = None;
                    }
                }
            }
            Command::SetPreset { slot, preset } => {
                if let Some(s) = self.slots.get_mut(slot) {
                    s.preset = preset.min(s.inst.presets.len().saturating_sub(1));
                    let home = s.home_channel() as usize;
                    s.ch[home].preset = None;
                }
            }
            Command::SetStrip { slot, strip, params } => {
                if let Some(st) = self.slots.get_mut(slot).and_then(|s| s.strips.get_mut(strip)) {
                    st.set_params(params);
                }
            }
            Command::SetNoteGroups { slot, groups } => {
                if let Some(s) = self.slots.get_mut(slot) {
                    s.groups = groups;
                }
            }
            Command::SetFx(fx) => self.fx = fx.clamped(),
            Command::SetProgramMap { slot, map } => match self.slots.get_mut(slot) {
                Some(s) => {
                    let old = std::mem::replace(&mut s.program_map, map);
                    self.trash(Garbage::ProgramMap(old));
                }
                None => self.trash(Garbage::ProgramMap(map)),
            },
            Command::Midi(msg) => self.route(msg),
            Command::Note { slot, key, vel } => {
                if let Some(s) = self.slots.get_mut(slot) {
                    let ch = s.home_channel();
                    if vel > 0 {
                        s.note_on(ch, key, vel, sr);
                    } else {
                        s.note_off(ch, key, sr);
                    }
                }
            }
            Command::Panic => {
                self.player.playing = false;
                self.slots.iter_mut().for_each(|s| s.kill_all());
                self.forward_notes_off();
            }
            Command::MasterGain(g) => self.master = g,
            Command::LoadSong(song) => {
                self.player_stop();
                if let Some(old) = self.player.song.replace(song) {
                    self.trash(Garbage::Song(old));
                }
            }
            Command::Play => {
                if let Some(song) = &self.player.song {
                    if self.player.time >= song.duration {
                        self.player_stop();
                    }
                    if self.player.idx == 0 {
                        self.slots.iter_mut().for_each(|s| s.reset_channels());
                    }
                    self.player.playing = true;
                }
            }
            Command::Pause => {
                if self.player.playing {
                    self.player.playing = false;
                    self.player_release();
                }
            }
            Command::Stop => self.player_stop(),
            Command::Seek(t) => self.seek(t),
            Command::SetLoop(on) => self.player.looping = on,
            Command::SetSpeed(s) => self.player.speed = s.clamp(0.1, 4.0),
            Command::ChannelMutes(m) => {
                let newly = m & !self.player.mutes;
                self.player.mutes = m;
                for ch in (0..16u8).filter(|c| newly & (1 << c) != 0) {
                    self.slots.iter_mut().for_each(|s| s.release_channel(ch));
                    self.forward([0xB0 | ch, 123, 0], 3);
                }
            }
        }
    }

    /// Render `n` (<= BLOCK) frames of every slot through its strips into
    /// the buses and FX sends, starting at `off`.
    fn render_chunk(&mut self, off: usize, n: usize, any_solo: bool) {
        let sr = self.sr;
        let out_pairs = self.out_pairs;
        for slot in self.slots.iter_mut() {
            slot.render(n);
            let sp = slot.params;
            let mut slot_peak = slot.peak;
            for (k, st) in slot.strips.iter_mut().enumerate() {
                if !st.prepare(n) {
                    continue;
                }
                st.run_dsp(n, sr);
                let p = st.params;
                let ch = if k < DRUM_STRIP_BASE { k } else { DRUM_CHANNEL as usize };
                let audible = !sp.mute && !p.mute && (!any_solo || sp.solo || p.solo);
                let g = p.gain() * sp.gain;
                let pan = (p.pan + sp.pan).clamp(-1.0, 1.0);
                let (tl, tr) = (g * (1.0 - pan).min(1.0), g * (1.0 + pan).min(1.0));
                let (ml, mr) = if audible { (tl, tr) } else { (0.0, 0.0) };
                let rev = p.reverb * slot.ch[ch].reverb;
                let cho = p.chorus * slot.ch[ch].chorus;
                let bus = if (p.output as usize) < out_pairs { p.output as usize } else { 0 };
                let [mut pl, mut pr] = st.peak;
                for i in 0..n {
                    let (gl, gr) = st.step_gain(ml, mr);
                    let (xl, xr) = (st.buf_l[i], st.buf_r[i]);
                    pl = pl.max((xl * tl).abs());
                    pr = pr.max((xr * tr).abs());
                    let (l, r) = (xl * gl, xr * gr);
                    let j = off + i;
                    self.buses[bus].l[j] += l;
                    self.buses[bus].r[j] += r;
                    if rev > 0.0 {
                        self.reverb_in.l[j] += l * rev;
                        self.reverb_in.r[j] += r * rev;
                    }
                    if cho > 0.0 {
                        self.chorus_in.l[j] += l * cho;
                        self.chorus_in.r[j] += r * cho;
                    }
                }
                st.peak = [pl, pr];
                slot_peak = [slot_peak[0].max(pl), slot_peak[1].max(pr)];
            }
            slot.peak = slot_peak;
        }
    }

    /// Render `frames` (<= MAX_FRAMES) into the output buses. Player events
    /// land on their exact sample by splitting the block around them.
    pub fn render(&mut self, frames: usize) {
        while let Ok(cmd) = self.rx.try_recv() {
            self.handle(cmd);
        }
        let frames = frames.min(MAX_FRAMES);
        for b in &mut self.buses {
            b.clear(frames);
        }
        self.reverb_in.clear(frames);
        self.chorus_in.clear(frames);
        for s in self.slots.iter_mut() {
            s.peak = [0.0; 2];
            for st in &mut s.strips {
                st.peak = [0.0; 2];
            }
        }
        let any_solo = self.slots.iter().any(|s| s.params.solo || s.strips.iter().any(|st| st.params.solo));
        let sr = self.sr as f64;
        let mut done = 0;
        while done < frames {
            if self.player.playing {
                self.dispatch_due();
            }
            let mut n = (frames - done).min(BLOCK);
            if self.player.playing
                && let Some(song) = &self.player.song
            {
                let next = song.events.get(self.player.idx).map(|e| e.time).unwrap_or(song.duration);
                let ahead = ((next - self.player.time) / self.player.speed * sr).ceil();
                n = n.min(ahead.max(1.0) as usize);
            }
            self.render_chunk(done, n, any_solo);
            if self.player.playing {
                self.player.time += n as f64 / sr * self.player.speed;
            }
            done += n;
        }

        // Global FX return into the main bus.
        let fx = self.fx;
        let decay = (-(frames as f32) / (self.sr * 0.3)).exp();
        let (main, _) = self.buses.split_at_mut(1);
        let main = &mut main[0];
        let fx_peak = |l: &[f32], r: &[f32]| l.iter().chain(r).fold(0f32, |m, x| m.max(x.abs()));
        let rev_in_peak = fx_peak(&self.reverb_in.l[..frames], &self.reverb_in.r[..frames]);
        let cho_in_peak = fx_peak(&self.chorus_in.l[..frames], &self.chorus_in.r[..frames]);
        if fx.reverb_return > 0.0 {
            self.reverb.process(
                &self.reverb_in.l[..frames],
                &self.reverb_in.r[..frames],
                &mut main.l[..frames],
                &mut main.r[..frames],
                fx.reverb_room,
                fx.reverb_damp,
                fx.reverb_width,
                fx.reverb_return,
            );
        }
        if fx.chorus_return > 0.0 {
            self.chorus.process(
                &self.chorus_in.l[..frames],
                &self.chorus_in.r[..frames],
                &mut main.l[..frames],
                &mut main.r[..frames],
                fx.chorus_rate,
                fx.chorus_depth,
                fx.chorus_delay,
                fx.chorus_return,
            );
        }
        hold(&self.shared.fx_peak[0], rev_in_peak, decay);
        hold(&self.shared.fx_peak[1], cho_in_peak, decay);

        // Buses the device cannot play fold into main, then master + clip.
        for b in out_pairs_fold(self.out_pairs) {
            let (head, tail) = self.buses.split_at_mut(b);
            for i in 0..frames {
                head[0].l[i] += tail[0].l[i];
                head[0].r[i] += tail[0].r[i];
            }
            tail[0].clear(frames);
        }
        for (bi, b) in self.buses.iter_mut().enumerate() {
            let (mut pl, mut pr) = (0f32, 0f32);
            for i in 0..frames {
                let l = b.l[i] * self.master;
                let r = b.r[i] * self.master;
                pl = pl.max(l.abs());
                pr = pr.max(r.abs());
                // Soft clip so overs do not wrap or crackle hard.
                b.l[i] = soft_clip(l);
                b.r[i] = soft_clip(r);
            }
            hold(&self.shared.bus_l[bi], pl, decay);
            hold(&self.shared.bus_r[bi], pr, decay);
        }
        hold(&self.shared.master_l, load_peak(&self.shared.bus_l[0]), 0.0);
        hold(&self.shared.master_r, load_peak(&self.shared.bus_r[0]), 0.0);

        self.publish(decay);
    }

    fn publish(&self, decay: f32) {
        let mut voices_total = 0u32;
        for (si, slot) in self.slots.iter().enumerate() {
            let meter = &self.shared.slots[si];
            let (mut n0, mut n1, mut count) = (0u64, 0u64, 0u32);
            let mut per_ch = [0u8; 16];
            for v in slot.voices.iter().filter(|v| v.active) {
                count += 1;
                per_ch[v.ch as usize & 15] = per_ch[v.ch as usize & 15].saturating_add(1);
                if !v.released || v.pedal_held {
                    if v.note < 64 { n0 |= 1 << v.note } else { n1 |= 1 << (v.note - 64) }
                }
            }
            voices_total += count;
            hold(&meter.peak_l, slot.peak[0], decay);
            hold(&meter.peak_r, slot.peak[1], decay);
            let mut live = 0u32;
            for (k, st) in slot.strips.iter().enumerate() {
                hold(&meter.strip_l[k], st.peak[0], decay);
                hold(&meter.strip_r[k], st.peak[1], decay);
                if st.quiet < self.sr as u32 {
                    live |= 1 << k;
                }
            }
            meter.strips_live.store(live, Ordering::Relaxed);
            meter.voices.store(count, Ordering::Relaxed);
            meter.preset.store(slot.preset as u32, Ordering::Relaxed);
            meter.notes[0].store(n0, Ordering::Relaxed);
            meter.notes[1].store(n1, Ordering::Relaxed);
            for c in 0..16u8 {
                meter.channels[c as usize].store(&slot.channel_info(c, per_ch[c as usize]));
            }
        }
        for si in self.slots.len()..MAX_SLOTS {
            let m = &self.shared.slots[si];
            m.voices.store(0, Ordering::Relaxed);
            m.notes[0].store(0, Ordering::Relaxed);
            m.notes[1].store(0, Ordering::Relaxed);
            m.strips_live.store(0, Ordering::Relaxed);
        }
        for a in &self.shared.channel_activity {
            hold(a, 0.0, decay);
        }
        self.shared.voices.store(voices_total, Ordering::Relaxed);
        let state = match (&self.player.song, self.player.playing) {
            (None, _) => PlayState::Empty,
            (Some(_), true) => PlayState::Playing,
            (Some(_), false) if self.player.time > 0.0 => PlayState::Paused,
            _ => PlayState::Stopped,
        };
        self.shared.player_state.store(state as u32, Ordering::Relaxed);
        self.shared.player_time.store(self.player.time.to_bits(), Ordering::Relaxed);
    }

    pub fn bus(&self, k: usize) -> (&[f32], &[f32]) {
        let b = &self.buses[k.min(MAX_BUSES - 1)];
        (&b.l, &b.r)
    }

    /// Render into a stereo pair (the main bus; other buses fold into it
    /// when only one pair is configured). Any length.
    pub fn process(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        let mut done = 0;
        while done < out_l.len() {
            let n = (out_l.len() - done).min(MAX_FRAMES);
            self.render(n);
            out_l[done..done + n].copy_from_slice(&self.buses[0].l[..n]);
            out_r[done..done + n].copy_from_slice(&self.buses[0].r[..n]);
            done += n;
        }
    }
}

/// Bus indices (descending) that must be folded into the main bus.
fn out_pairs_fold(out_pairs: usize) -> impl Iterator<Item = usize> {
    (out_pairs.max(1)..MAX_BUSES).rev()
}

#[inline]
fn soft_clip(x: f32) -> f32 {
    if x.abs() <= 0.9 {
        x
    } else {
        let s = x.signum();
        let over = x.abs() - 0.9;
        s * (0.9 + 0.1 * (over / 0.1).tanh())
    }
}
