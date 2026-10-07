//! UI-side state: the rack mirror (source of truth for parameters), popups,
//! computer-keyboard piano, async instrument loading, MIDI monitor and log.

mod mixer;
mod web;

pub use mixer::{MixRow, STRIP_FIELDS, SlotMixer, fx_field_text, output_name, strip_field_text};

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::audio::{self, AudioOut};
use crate::browser::{BrowseKind, Browser};
use crate::engine::mixer::FxParams;
use crate::engine::{ChannelInfo, Command, Garbage, MAX_SLOTS, NO_DRUMS, OMNI, PlayState, Shared, Slot, SlotParams, load_peak};
use crate::web::{WebAction, WebHub};
use crate::instrument::{self, Instrument, WavParams, db_to_gain, note_name};
use crate::midi::{self, Midi, MidiEvent};
use crate::smf::{self, Song};

pub struct SlotUi {
    pub id: u64,
    pub inst: Arc<Instrument>,
    pub params: SlotParams,
    pub volume_db: f32,
    pub preset: usize,
    pub wav: Option<WavParams>,
    pub meter: [f32; 2],
    pub mixer: SlotMixer,
}

impl SlotUi {
    fn engine_slot(&self) -> Box<Slot> {
        Box::new(Slot::with_mixer(self.inst.clone(), self.preset, self.params, &self.mixer.strips, self.mixer.groups))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Rack,
    Params,
    /// Per-channel bank/program map of a SoundFont slot.
    Channels,
    /// Channel strips, drum note groups, FX returns, master.
    Mixer,
    Player,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Param {
    Volume,
    Pan,
    Channel,
    Preset,
    Transpose,
    Tune,
    KeyLo,
    KeyHi,
    VelLo,
    VelHi,
    BendRange,
    Root,
    Keytrack,
    Loop,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone)]
pub enum PortItem {
    Input(String),
    Output(Option<String>),
    Thru,
    #[cfg(unix)]
    Virtual,
}

#[derive(Clone, Copy)]
pub enum LoadTarget {
    New,
    Replace(u64),
    Song,
}

pub enum Popup {
    None,
    Browser(Browser, LoadTarget),
    Ports { items: Vec<PortItem>, sel: usize },
    Devices { items: Vec<String>, sel: usize },
    /// `channel: Some(c)` assigns the preset to one MIDI channel only.
    Presets { sel: usize, filter: String, channel: Option<u8> },
    Help,
}

struct Loaded {
    seq: u64,
    target: LoadTarget,
    path: PathBuf,
    result: anyhow::Result<Instrument>,
}

pub struct LogLine {
    pub text: String,
    pub error: bool,
}

pub struct App {
    pub slots: Vec<SlotUi>,
    pub sel: usize,
    pub focus: Focus,
    pub param_sel: usize,
    pub chan_sel: usize,
    pub popup: Popup,
    pub play_mode: bool,
    pub octave: i32,
    pub velocity: u8,
    held: HashMap<char, (u8, usize, Instant)>,
    pub release_events: bool,
    tx: Sender<Command>,
    rx: Receiver<Command>,
    garbage_tx: Sender<Garbage>,
    garbage_rx: Receiver<Garbage>,
    pub shared: Arc<Shared>,
    pub audio: Option<AudioOut>,
    pub audio_error: Option<String>,
    buffer_frames: Option<u32>,
    pub midi: Midi,
    midi_rx: Receiver<MidiEvent>,
    pub midi_log: VecDeque<String>,
    pub log: VecDeque<LogLine>,
    log_count: usize,
    load_tx: Sender<Loaded>,
    load_rx: Receiver<Loaded>,
    pub loading: Vec<PathBuf>,
    /// Loads finish in any order; results are committed in request order
    /// so the rack matches the order files were chosen.
    load_seq: u64,
    commit_seq: u64,
    finished: std::collections::BTreeMap<u64, Loaded>,
    pub master_db: f32,
    pub master_meter: [f32; 2],
    pub cpu: f32,
    pub voices: u32,
    pub kbd_notes: [u64; 2],
    next_id: u64,
    pub quit: bool,
    start: Instant,
    pub browse_dir: PathBuf,
    pub player: PlayerUi,
    /// Start the player once pending instrument loads finish (`--play`).
    pub autoplay: bool,
    pub fx: FxParams,
    pub mix_row: usize,
    pub mix_field: usize,
    pub web: Option<Arc<WebHub>>,
    pub web_url: Option<String>,
    web_tx: Sender<WebAction>,
    web_rx: Receiver<WebAction>,
}

/// UI mirror of the engine's MIDI file player.
pub struct PlayerUi {
    pub song: Option<Arc<Song>>,
    pub path: Option<PathBuf>,
    pub state: PlayState,
    pub time: f64,
    pub looping: bool,
    pub speed: f64,
    pub mutes: u16,
    pub channel: usize,
    pub levels: [f32; 16],
}

pub const PIANO_LOWER: &[char] = &['z', 's', 'x', 'd', 'c', 'v', 'g', 'b', 'h', 'n', 'j', 'm', ',', 'l', '.', ';', '/'];
pub const PIANO_UPPER: &[char] = &['q', '2', 'w', '3', 'e', 'r', '5', 't', '6', 'y', '7', 'u', 'i', '9', 'o', '0', 'p'];
const AUTO_RELEASE: Duration = Duration::from_millis(600);

impl App {
    pub fn new(release_events: bool, buffer_frames: Option<u32>) -> Self {
        let (tx, rx) = crossbeam_channel::bounded(4096);
        let (garbage_tx, garbage_rx) = crossbeam_channel::bounded(256);
        let (midi_tx, midi_rx) = crossbeam_channel::bounded(1024);
        let (load_tx, load_rx) = crossbeam_channel::unbounded();
        let (web_tx, web_rx) = crossbeam_channel::unbounded();
        let midi = Midi::new(tx.clone(), midi_tx);
        let shared = Arc::new(Shared::new());
        midi.spawn_player_forwarder(shared.out_rx.clone());
        let browse_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            slots: Vec::new(),
            sel: 0,
            focus: Focus::Rack,
            param_sel: 0,
            chan_sel: 0,
            popup: Popup::None,
            play_mode: false,
            octave: 3,
            velocity: 100,
            held: HashMap::new(),
            release_events,
            tx,
            rx,
            garbage_tx,
            garbage_rx,
            shared,
            audio: None,
            audio_error: None,
            buffer_frames,
            midi,
            midi_rx,
            midi_log: VecDeque::new(),
            log: VecDeque::new(),
            log_count: 0,
            load_tx,
            load_rx,
            loading: Vec::new(),
            load_seq: 0,
            commit_seq: 0,
            finished: std::collections::BTreeMap::new(),
            master_db: 0.0,
            master_meter: [0.0; 2],
            cpu: 0.0,
            voices: 0,
            kbd_notes: [0; 2],
            next_id: 1,
            quit: false,
            start: Instant::now(),
            browse_dir,
            player: PlayerUi {
                song: None,
                path: None,
                state: PlayState::Empty,
                time: 0.0,
                looping: false,
                speed: 1.0,
                mutes: 0,
                channel: 0,
                levels: [0.0; 16],
            },
            autoplay: false,
            fx: FxParams::default(),
            mix_row: 0,
            mix_field: 0,
            web: None,
            web_url: None,
            web_tx,
            web_rx,
        }
    }

    pub fn info(&mut self, s: impl Into<String>) {
        self.push_log(s.into(), false);
    }

    pub fn error(&mut self, s: impl Into<String>) {
        self.push_log(s.into(), true);
    }

    /// Lines logged since start (the deque itself is capped).
    pub fn log_total(&self) -> usize {
        self.log_count
    }

    fn push_log(&mut self, text: String, error: bool) {
        self.log_count += 1;
        self.log.push_back(LogLine { text, error });
        while self.log.len() > 200 {
            self.log.pop_front();
        }
    }

    fn send(&self, cmd: Command) {
        let _ = self.tx.try_send(cmd);
    }

    // ------------------------------------------------------------- audio

    pub fn start_audio(&mut self, device: Option<String>) {
        self.audio = None;
        while self.rx.try_recv().is_ok() {}
        while self.garbage_rx.try_recv().is_ok() {}
        match audio::start(
            device.as_deref(),
            self.buffer_frames,
            self.rx.clone(),
            self.garbage_tx.clone(),
            self.shared.clone(),
        ) {
            Ok(a) => {
                self.info(format!(
                    "audio: {} / {} / {} Hz / {} ch / {} / buffer {}",
                    a.host, a.device, a.sample_rate, a.channels, a.format, a.buffer
                ));
                self.audio = Some(a);
                self.audio_error = None;
                self.replay();
            }
            Err(e) => {
                let msg = format!("audio: {e:#}");
                self.error(msg.clone());
                self.audio_error = Some(msg);
            }
        }
    }

    /// Re-create the whole rack inside a fresh engine (after a device switch).
    fn replay(&self) {
        self.send(Command::MasterGain(db_to_gain(self.master_db)));
        self.send(Command::SetFx(self.fx));
        for s in &self.slots {
            self.send(Command::AddSlot(s.engine_slot()));
        }
        let p = &self.player;
        if let Some(song) = &p.song {
            self.send(Command::LoadSong(song.clone()));
            self.send(Command::SetLoop(p.looping));
            self.send(Command::SetSpeed(p.speed));
            self.send(Command::ChannelMutes(p.mutes));
            if p.time > 0.0 {
                self.send(Command::Seek(p.time));
            }
            if p.state == PlayState::Playing {
                self.send(Command::Play);
            }
        }
    }

    // -------------------------------------------------------------- MIDI

    pub fn connect_all_midi_inputs(&mut self) {
        for name in midi::input_ports() {
            match self.midi.connect_input(&name) {
                Ok(n) => self.info(format!("MIDI in: {n}")),
                Err(e) => self.error(format!("MIDI in: {e:#}")),
            }
        }
    }

    fn port_items(&self) -> Vec<PortItem> {
        let mut items: Vec<PortItem> = midi::input_ports().into_iter().map(PortItem::Input).collect();
        items.push(PortItem::Output(None));
        items.extend(midi::output_ports().into_iter().map(|n| PortItem::Output(Some(n))));
        items.push(PortItem::Thru);
        #[cfg(unix)]
        items.push(PortItem::Virtual);
        items
    }

    fn activate_port(&mut self, item: PortItem) {
        match item {
            PortItem::Input(name) => match self.midi.toggle_input(&name) {
                Ok(true) => self.info(format!("MIDI in opened: {name}")),
                Ok(false) => self.info(format!("MIDI in closed: {name}")),
                Err(e) => self.error(format!("MIDI in: {e:#}")),
            },
            PortItem::Output(name) => match self.midi.set_output(name.as_deref()) {
                Ok(Some(n)) => self.info(format!("MIDI out: {n}")),
                Ok(None) => self.info("MIDI out: none"),
                Err(e) => self.error(format!("MIDI out: {e:#}")),
            },
            PortItem::Thru => {
                let on = !self.midi.thru.load(Ordering::Relaxed);
                self.midi.thru.store(on, Ordering::Relaxed);
                self.info(format!("MIDI thru {}", if on { "on" } else { "off" }));
            }
            #[cfg(unix)]
            PortItem::Virtual => match self.midi.create_virtual_ports() {
                Ok(()) => self.info("virtual MIDI ports created (simpletui:in / simpletui:out)"),
                Err(e) => self.error(format!("virtual MIDI: {e:#}")),
            },
        }
    }

    fn log_midi(&mut self, source: &str, msg: &[u8]) {
        let t = self.start.elapsed().as_secs_f32();
        let line = format!("{t:8.3} {:<14} {}", truncate(source, 14), describe_midi(msg));
        self.midi_log.push_back(line);
        while self.midi_log.len() > 200 {
            self.midi_log.pop_front();
        }
    }

    // ----------------------------------------------------------- loading

    pub fn load(&mut self, path: PathBuf, target: LoadTarget) {
        if matches!(target, LoadTarget::New) && self.slots.len() + self.loading.len() >= MAX_SLOTS {
            self.error(format!("rack full ({MAX_SLOTS} slots)"));
            return;
        }
        self.info(format!("loading {}", path.display()));
        self.loading.push(path.clone());
        let tx = self.load_tx.clone();
        let seq = self.load_seq;
        self.load_seq += 1;
        std::thread::spawn(move || {
            let result = instrument::load(&path);
            let _ = tx.send(Loaded { seq, target, path, result });
        });
    }

    fn finish_load(&mut self, l: Loaded) {
        if let Some(i) = self.loading.iter().position(|p| *p == l.path) {
            self.loading.remove(i);
        }
        let inst = match l.result {
            Ok(i) => i,
            Err(e) => return self.error(format!("{e:#}")),
        };
        let summary = format!(
            "loaded {} [{}] {} preset(s), {} zone(s), {:.1} MB",
            inst.name,
            inst.kind.label(),
            inst.presets.len(),
            inst.zone_count(),
            inst.sample_bytes as f64 / 1_048_576.0
        );
        self.info(summary);
        for w in inst.warnings.iter().take(5) {
            self.error(format!("  {w}"));
        }
        if inst.warnings.len() > 5 {
            self.error(format!("  ... {} more warnings", inst.warnings.len() - 5));
        }
        if let Some(dir) = l.path.parent() {
            self.browse_dir = dir.to_path_buf();
        }
        let wav = inst.wav.as_ref().map(|w| w.params);
        let inst = Arc::new(inst);
        match l.target {
            LoadTarget::New => {
                if self.slots.len() >= MAX_SLOTS {
                    return self.error(format!("rack full ({MAX_SLOTS} slots)"));
                }
                let params = SlotParams::default();
                let id = self.next_id;
                self.next_id += 1;
                let slot = SlotUi { id, inst, params, volume_db: 0.0, preset: 0, wav, meter: [0.0; 2], mixer: SlotMixer::default() };
                self.send(Command::AddSlot(slot.engine_slot()));
                self.slots.push(slot);
                self.sel = self.slots.len() - 1;
            }
            LoadTarget::Replace(id) => {
                let Some(idx) = self.slots.iter().position(|s| s.id == id) else { return };
                self.send(Command::SetInstrument { slot: idx, inst: inst.clone(), preset: 0, keep_voices: false });
                let s = &mut self.slots[idx];
                s.inst = inst;
                s.preset = 0;
                s.wav = wav;
            }
            // Songs are parsed synchronously by `load_song`, never queued here.
            LoadTarget::Song => {}
        }
    }

    // -------------------------------------------------------------- tick

    pub fn tick(&mut self) {
        while let Ok(l) = self.load_rx.try_recv() {
            self.finished.insert(l.seq, l);
        }
        while let Some(l) = self.finished.remove(&self.commit_seq) {
            self.commit_seq += 1;
            self.finish_load(l);
        }
        if self.autoplay && self.loading.is_empty() {
            self.autoplay = false;
            self.play();
        }
        while let Ok(g) = self.garbage_rx.try_recv() {
            drop(g);
        }
        while let Ok(ev) = self.midi_rx.try_recv() {
            self.log_midi(&ev.source, &ev.msg[..ev.len]);
        }
        for (i, s) in self.slots.iter_mut().enumerate() {
            let m = &self.shared.slots[i];
            s.meter = [load_peak(&m.peak_l), load_peak(&m.peak_r)];
            let p = m.preset.load(Ordering::Relaxed) as usize;
            if self.audio.is_some() && p < s.inst.presets.len() {
                s.preset = p;
            }
        }
        self.master_meter = [load_peak(&self.shared.master_l), load_peak(&self.shared.master_r)];
        self.cpu = f32::from_bits(self.shared.cpu.load(Ordering::Relaxed));
        if self.audio.is_some() {
            self.player.state = PlayState::from_u32(self.shared.player_state.load(Ordering::Relaxed));
            self.player.time = f64::from_bits(self.shared.player_time.load(Ordering::Relaxed));
        }
        for (c, level) in self.player.levels.iter_mut().enumerate() {
            *level = load_peak(&self.shared.channel_activity[c]);
        }
        self.web_tick();
        self.voices = self.shared.voices.load(Ordering::Relaxed);

        if !self.release_events {
            let now = Instant::now();
            let expired: Vec<char> =
                self.held.iter().filter(|(_, (_, _, t))| now.duration_since(*t) > AUTO_RELEASE).map(|(c, _)| *c).collect();
            for c in expired {
                self.key_note_off(c);
            }
        }
        self.kbd_notes = [0; 2];
        for (note, _, _) in self.held.values() {
            let n = *note;
            if n < 64 { self.kbd_notes[0] |= 1 << n } else { self.kbd_notes[1] |= 1 << (n - 64) }
        }
    }

    pub fn channel_info(&self, slot: usize, ch: usize) -> ChannelInfo {
        self.shared.slots[slot].channels[ch].load()
    }

    /// Channels with their own program that differs from the slot preset.
    pub fn custom_channels(&self, slot: usize) -> usize {
        let Some(s) = self.slots.get(slot) else { return 0 };
        (0..16)
            .filter(|&c| {
                let i = self.channel_info(slot, c);
                i.explicit && i.preset != s.preset
            })
            .count()
    }

    fn step_channel_preset(&mut self, d: i32) {
        let slot = self.sel;
        let Some(s) = self.slots.get(slot) else { return };
        let n = s.inst.presets.len() as i32;
        let cur = self.channel_info(slot, self.chan_sel).preset as i32;
        let preset = (cur + d).rem_euclid(n.max(1)) as usize;
        self.send(Command::SetChannelPreset { slot, ch: self.chan_sel as u8, preset: Some(preset) });
    }

    pub fn slot_notes(&self, i: usize) -> [u64; 2] {
        let m = &self.shared.slots[i];
        [m.notes[0].load(Ordering::Relaxed), m.notes[1].load(Ordering::Relaxed)]
    }

    // ------------------------------------------------------------- piano

    fn piano_note(&self, c: char) -> Option<u8> {
        let base = (self.octave + 1) * 12;
        let off = if let Some(i) = PIANO_LOWER.iter().position(|&k| k == c) {
            i as i32
        } else {
            12 + PIANO_UPPER.iter().position(|&k| k == c)? as i32
        };
        let n = base + off;
        (0..128).contains(&n).then_some(n as u8)
    }

    fn out_channel(&self, slot: usize) -> u8 {
        self.slots.get(slot).map(|s| s.params.home_channel()).unwrap_or(0)
    }

    fn key_note_on(&mut self, c: char) {
        if let Some(entry) = self.held.get_mut(&c) {
            entry.2 = Instant::now();
            return;
        }
        let Some(note) = self.piano_note(c) else { return };
        if self.slots.is_empty() {
            return;
        }
        let slot = self.sel;
        self.held.insert(c, (note, slot, Instant::now()));
        self.send(Command::Note { slot, key: note, vel: self.velocity });
        let msg = [0x90 | self.out_channel(slot), note, self.velocity];
        self.midi.send(&msg);
        self.log_midi("keyboard", &msg);
    }

    fn key_note_off(&mut self, c: char) {
        let Some((note, slot, _)) = self.held.remove(&c) else { return };
        self.send(Command::Note { slot, key: note, vel: 0 });
        let msg = [0x80 | self.out_channel(slot), note, 0];
        self.midi.send(&msg);
        self.log_midi("keyboard", &msg);
    }

    fn release_all_keys(&mut self) {
        let keys: Vec<char> = self.held.keys().copied().collect();
        for c in keys {
            self.key_note_off(c);
        }
    }

    // ------------------------------------------------------------ params

    pub fn params_for(&self, i: usize) -> Vec<Param> {
        use Param::*;
        let Some(s) = self.slots.get(i) else { return Vec::new() };
        let mut v = vec![Volume, Pan, Channel];
        if s.inst.presets.len() > 1 {
            v.push(Preset);
        }
        v.extend([Transpose, Tune, KeyLo, KeyHi, VelLo, VelHi, BendRange]);
        if s.wav.is_some() {
            v.extend([Root, Keytrack, Loop, Attack, Hold, Decay, Sustain, Release]);
        }
        v
    }

    pub fn param_text(&self, i: usize, p: Param) -> (&'static str, String) {
        let s = &self.slots[i];
        let pr = &s.params;
        let w = s.wav;
        let secs = |t: f32| if t < 1.0 { format!("{:.0} ms", t * 1000.0) } else { format!("{t:.2} s") };
        match p {
            Param::Volume => ("Volume", format!("{:+.1} dB", s.volume_db)),
            Param::Pan => ("Pan", pan_text(pr.pan)),
            Param::Channel => ("MIDI Ch", channels_text(pr.channels)),
            Param::Preset => {
                let pre = &s.inst.presets[s.preset];
                ("Preset", format!("{:03}:{:03} {}", pre.bank, pre.program, pre.name))
            }
            Param::Transpose => ("Transpose", format!("{:+} st", pr.transpose)),
            Param::Tune => ("Fine Tune", format!("{:+.0} ct", pr.tune)),
            Param::KeyLo => ("Key Low", note_name(pr.key_lo)),
            Param::KeyHi => ("Key High", note_name(pr.key_hi)),
            Param::VelLo => ("Vel Low", pr.vel_lo.to_string()),
            Param::VelHi => ("Vel High", pr.vel_hi.to_string()),
            Param::BendRange => ("Bend Range", format!("±{:.0} st", pr.bend_range)),
            Param::Root => ("Root Key", w.map(|w| note_name(w.root)).unwrap_or_default()),
            Param::Keytrack => ("Keytrack", w.map(|w| if w.keytrack { "on" } else { "off (fixed)" }.to_string()).unwrap_or_default()),
            Param::Loop => ("Loop Mode", w.map(|w| w.loop_mode.label().to_string()).unwrap_or_default()),
            Param::Attack => ("Attack", w.map(|w| secs(w.env.attack)).unwrap_or_default()),
            Param::Hold => ("Hold", w.map(|w| secs(w.env.hold)).unwrap_or_default()),
            Param::Decay => ("Decay", w.map(|w| secs(w.env.decay)).unwrap_or_default()),
            Param::Sustain => ("Sustain", w.map(|w| format!("{:.0} %", w.env.sustain * 100.0)).unwrap_or_default()),
            Param::Release => ("Release", w.map(|w| secs(w.env.release)).unwrap_or_default()),
        }
    }

    fn adjust(&mut self, p: Param, dir: i32, coarse: bool) {
        let i = self.sel;
        if i >= self.slots.len() {
            return;
        }
        let step = |fine: f32, big: f32| if coarse { big } else { fine } * dir as f32;
        let time = |t: f32| -> f32 {
            let f = if coarse { 2.0f32 } else { 1.2 };
            if dir > 0 { (t.max(0.001) * f).min(30.0) } else if t <= 0.0012 { 0.0 } else { t / f }
        };
        let istep = |v: u8, lo: u8, hi: u8| -> u8 {
            let d = if coarse { 12 } else { 1 } * dir;
            (v as i32 + d).clamp(lo as i32, hi as i32) as u8
        };
        let s = &mut self.slots[i];
        let mut wav_changed = false;
        match p {
            Param::Volume => {
                s.volume_db = (s.volume_db + step(0.5, 6.0)).clamp(-60.0, 12.0);
                s.params.gain = if s.volume_db <= -60.0 { 0.0 } else { db_to_gain(s.volume_db) };
            }
            Param::Pan => s.params.pan = (s.params.pan + step(0.05, 0.25)).clamp(-1.0, 1.0),
            Param::Channel => {
                // Omni, 1..16, "no drums"; a custom mask (set in the Channels
                // view) steps back into this list.
                let mut list = vec![OMNI];
                list.extend((0..16).map(|c| 1u16 << c));
                list.push(NO_DRUMS);
                let n = list.len() as i32;
                let next = match list.iter().position(|&m| m == s.params.channels) {
                    Some(i) => (i as i32 + dir).rem_euclid(n),
                    None if dir > 0 => 0,
                    None => n - 1,
                };
                s.params.channels = list[next as usize];
            }
            Param::Preset => {
                let n = s.inst.presets.len() as i32;
                let d = if coarse { 10 } else { 1 } * dir;
                s.preset = (s.preset as i32 + d).rem_euclid(n) as usize;
                let preset = s.preset;
                self.send(Command::SetPreset { slot: i, preset });
                return;
            }
            Param::Transpose => {
                s.params.transpose = (s.params.transpose + if coarse { 12 } else { 1 } * dir).clamp(-48, 48)
            }
            Param::Tune => s.params.tune = (s.params.tune + step(1.0, 10.0)).clamp(-100.0, 100.0),
            Param::KeyLo => s.params.key_lo = istep(s.params.key_lo, 0, s.params.key_hi),
            Param::KeyHi => s.params.key_hi = istep(s.params.key_hi, s.params.key_lo, 127),
            Param::VelLo => s.params.vel_lo = istep(s.params.vel_lo, 1, s.params.vel_hi),
            Param::VelHi => s.params.vel_hi = istep(s.params.vel_hi, s.params.vel_lo, 127),
            Param::BendRange => s.params.bend_range = (s.params.bend_range + step(1.0, 12.0)).clamp(0.0, 48.0),
            _ => {
                let Some(w) = s.wav.as_mut() else { return };
                match p {
                    Param::Root => w.root = istep(w.root, 0, 127),
                    Param::Keytrack => w.keytrack = !w.keytrack,
                    Param::Loop => w.loop_mode = if dir > 0 { w.loop_mode.next() } else { w.loop_mode.prev() },
                    Param::Attack => w.env.attack = time(w.env.attack),
                    Param::Hold => w.env.hold = time(w.env.hold),
                    Param::Decay => w.env.decay = time(w.env.decay),
                    Param::Sustain => w.env.sustain = (w.env.sustain + step(0.05, 0.25)).clamp(0.0, 1.0),
                    Param::Release => w.env.release = time(w.env.release).max(0.001),
                    _ => {}
                }
                wav_changed = true;
            }
        }
        if wav_changed {
            self.rebuild_wav(i);
        } else {
            let params = self.slots[i].params;
            self.send(Command::SetParams { slot: i, params });
            if p == Param::Channel {
                self.drum_slot_preset(i);
            }
        }
    }

    /// A SoundFont slot narrowed to channel 10 only becomes the drum slot:
    /// switch it to the kit (bank 128) unless a kit is already selected.
    fn drum_slot_preset(&mut self, i: usize) {
        let Some(s) = self.slots.get_mut(i) else { return };
        if s.params.channels != 1 << 9 || s.inst.presets.get(s.preset).is_some_and(|p| p.bank >= 128) {
            return;
        }
        let kit = s.inst.find_preset(128, 0).or_else(|| s.inst.presets.iter().position(|p| p.bank >= 128));
        if let Some(kit) = kit {
            s.preset = kit;
            let name = s.inst.presets[kit].name.clone();
            self.send(Command::SetPreset { slot: i, preset: kit });
            self.info(format!("slot {} on ch 10: drum kit {name}", i + 1));
        }
    }

    fn rebuild_wav(&mut self, i: usize) {
        let s = &mut self.slots[i];
        let (Some(src), Some(params)) = (s.inst.wav.clone(), s.wav) else { return };
        let inst = Arc::new(instrument::wav::build(s.inst.name.clone(), &s.inst.path, src, params));
        s.inst = inst.clone();
        self.send(Command::SetInstrument { slot: i, inst, preset: 0, keep_voices: true });
    }

    fn update_params(&mut self, i: usize, f: impl FnOnce(&mut SlotParams)) {
        if let Some(s) = self.slots.get_mut(i) {
            let before = s.params.channels;
            f(&mut s.params);
            let params = s.params;
            self.send(Command::SetParams { slot: i, params });
            if params.channels != before {
                self.drum_slot_preset(i);
            }
        }
    }

    fn remove_selected(&mut self) {
        if self.sel >= self.slots.len() {
            return;
        }
        self.release_all_keys();
        let s = self.slots.remove(self.sel);
        self.send(Command::RemoveSlot(self.sel));
        self.info(format!("removed {}", s.inst.name));
        self.sel = self.sel.min(self.slots.len().saturating_sub(1));
    }

    fn set_master(&mut self, delta: f32) {
        self.master_db = (self.master_db + delta).clamp(-60.0, 12.0);
        self.send(Command::MasterGain(db_to_gain(self.master_db)));
    }

    // ------------------------------------------------------------- input

    pub fn on_key(&mut self, k: KeyEvent) {
        // Release events only matter for the piano.
        if k.kind == KeyEventKind::Release {
            if let KeyCode::Char(c) = k.code {
                self.key_note_off(c.to_ascii_lowercase());
            }
            return;
        }
        let repeat = k.kind == KeyEventKind::Repeat;
        if k.modifiers.contains(KeyModifiers::CONTROL) && matches!(k.code, KeyCode::Char('c') | KeyCode::Char('q')) {
            self.quit = true;
            return;
        }

        if !matches!(self.popup, Popup::None) {
            self.on_popup_key(k);
            return;
        }

        if self.play_mode {
            match k.code {
                KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                    let c = c.to_ascii_lowercase();
                    if self.piano_note(c).is_some() {
                        if !repeat || !self.release_events {
                            self.key_note_on(c);
                        }
                        return;
                    }
                    match c {
                        '-' => self.octave = (self.octave - 1).max(-1),
                        '=' | '+' => self.octave = (self.octave + 1).min(8),
                        '[' => self.velocity = self.velocity.saturating_sub(10).max(1),
                        ']' => self.velocity = (self.velocity + 10).min(127),
                        ' ' => {
                            self.release_all_keys();
                            self.play_mode = false;
                        }
                        _ => {}
                    }
                }
                KeyCode::Esc | KeyCode::Tab => {
                    self.release_all_keys();
                    self.play_mode = false;
                }
                KeyCode::Up => self.select(-1),
                KeyCode::Down => self.select(1),
                KeyCode::Backspace => self.panic(),
                _ => {}
            }
            return;
        }

        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        if self.focus == Focus::Params {
            let params = self.params_for(self.sel);
            match k.code {
                KeyCode::Up => self.param_sel = self.param_sel.saturating_sub(1),
                KeyCode::Down => self.param_sel = (self.param_sel + 1).min(params.len().saturating_sub(1)),
                KeyCode::Left | KeyCode::Right => {
                    let dir = if k.code == KeyCode::Left { -1 } else { 1 };
                    if let Some(&p) = params.get(self.param_sel) {
                        self.adjust(p, dir, shift);
                    }
                }
                KeyCode::PageUp | KeyCode::PageDown => {
                    let dir = if k.code == KeyCode::PageDown { -1 } else { 1 };
                    if let Some(&p) = params.get(self.param_sel) {
                        self.adjust(p, dir, true);
                    }
                }
                KeyCode::Tab => self.focus = if self.sel < self.slots.len() { Focus::Channels } else { Focus::Player },
                KeyCode::Esc | KeyCode::BackTab => self.focus = Focus::Rack,
                _ => self.on_global_key(k),
            }
            return;
        }

        if self.focus == Focus::Mixer && self.on_mixer_key(k) {
            return;
        }

        if self.focus == Focus::Channels {
            match k.code {
                KeyCode::Up => self.chan_sel = self.chan_sel.saturating_sub(1),
                KeyCode::Down => self.chan_sel = (self.chan_sel + 1).min(15),
                KeyCode::Left => self.step_channel_preset(if shift { -10 } else { -1 }),
                KeyCode::Right => self.step_channel_preset(if shift { 10 } else { 1 }),
                KeyCode::Enter => {
                    let cur = self.channel_info(self.sel, self.chan_sel).preset;
                    self.popup = Popup::Presets { sel: cur, filter: String::new(), channel: Some(self.chan_sel as u8) };
                }
                KeyCode::Delete | KeyCode::Char('d') => {
                    let ch = self.chan_sel as u8;
                    self.send(Command::SetChannelPreset { slot: self.sel, ch, preset: None });
                }
                KeyCode::Char('D') => {
                    for ch in 0..16u8 {
                        self.send(Command::SetChannelPreset { slot: self.sel, ch, preset: None });
                    }
                }
                KeyCode::Char(' ') | KeyCode::Char('x') => {
                    let bit = 1u16 << self.chan_sel;
                    self.update_params(self.sel, |p| p.channels ^= bit);
                }
                KeyCode::Char('A') => self.update_params(self.sel, |p| p.channels = OMNI),
                KeyCode::Char('I') => self.update_params(self.sel, |p| p.channels = !p.channels),
                KeyCode::Char('O') => {
                    let bit = 1u16 << self.chan_sel;
                    self.update_params(self.sel, |p| p.channels = bit);
                }
                KeyCode::Char('m') => self.toggle_channel_mute(self.chan_sel),
                KeyCode::Char('s') => self.solo_channel(self.chan_sel),
                KeyCode::Char('u') => self.set_mutes(0),
                KeyCode::Tab => self.focus = Focus::Mixer,
                KeyCode::BackTab => self.focus = Focus::Params,
                KeyCode::Esc | KeyCode::Char('v') => self.focus = Focus::Rack,
                _ => self.on_global_key(k),
            }
            return;
        }

        if self.focus == Focus::Player {
            match k.code {
                KeyCode::Left => self.player.channel = (self.player.channel + 15) % 16,
                KeyCode::Right => self.player.channel = (self.player.channel + 1) % 16,
                KeyCode::Up => self.seek_by(if shift { 30.0 } else { 5.0 }),
                KeyCode::Down => self.seek_by(if shift { -30.0 } else { -5.0 }),
                KeyCode::Home => self.seek_to(0.0),
                KeyCode::Char('m') => self.toggle_channel_mute(self.player.channel),
                KeyCode::Char('s') => self.solo_channel(self.player.channel),
                KeyCode::Char('u') => self.set_mutes(0),
                KeyCode::Tab | KeyCode::Esc => self.focus = Focus::Rack,
                KeyCode::BackTab => self.focus = if self.slots.is_empty() { Focus::Rack } else { Focus::Mixer },
                _ => self.on_global_key(k),
            }
            return;
        }

        match k.code {
            KeyCode::Up => self.select(-1),
            KeyCode::Down => self.select(1),
            KeyCode::Left => self.adjust(Param::Volume, -1, shift),
            KeyCode::Right => self.adjust(Param::Volume, 1, shift),
            KeyCode::Tab => {
                if self.slots.is_empty() {
                    self.focus = Focus::Player;
                } else {
                    self.focus = Focus::Params;
                    self.param_sel = 0;
                }
            }
            KeyCode::BackTab => self.focus = Focus::Player,
            KeyCode::Delete => self.remove_selected(),
            _ => self.on_global_key(k),
        }
    }

    fn select(&mut self, d: i32) {
        if self.slots.is_empty() {
            return;
        }
        self.release_all_keys();
        self.sel = (self.sel as i32 + d).clamp(0, self.slots.len() as i32 - 1) as usize;
        self.param_sel = self.param_sel.min(self.params_for(self.sel).len().saturating_sub(1));
    }

    fn panic(&mut self) {
        self.release_all_keys();
        self.send(Command::Panic);
        for ch in 0..16u8 {
            self.midi.send(&[0xB0 | ch, 123, 0]);
            self.midi.send(&[0xB0 | ch, 120, 0]);
        }
        self.info("panic: all sound off");
    }

    fn on_global_key(&mut self, k: KeyEvent) {
        let KeyCode::Char(c) = k.code else {
            match k.code {
                KeyCode::F(1) => self.popup = Popup::Help,
                KeyCode::Insert => self.open_browser(LoadTarget::New),
                KeyCode::Backspace => self.panic(),
                KeyCode::Enter | KeyCode::F(5) => self.toggle_play(),
                KeyCode::F(6) => self.stop(),
                KeyCode::F(7) => self.seek_by(-5.0),
                KeyCode::F(8) => self.seek_by(5.0),
                _ => {}
            }
            return;
        };
        let sel = self.sel;
        match c {
            'q' => self.quit = true,
            '?' | 'h' => self.popup = Popup::Help,
            'a' => self.open_browser(LoadTarget::New),
            'r' => {
                if let Some(s) = self.slots.get(sel) {
                    let id = s.id;
                    self.open_browser(LoadTarget::Replace(id));
                }
            }
            'x' => self.remove_selected(),
            'm' => self.update_params(sel, |p| p.mute = !p.mute),
            's' => self.update_params(sel, |p| p.solo = !p.solo),
            'c' => self.adjust(Param::Channel, 1, false),
            'C' => self.adjust(Param::Channel, -1, false),
            '[' => self.adjust(Param::Pan, -1, false),
            ']' => self.adjust(Param::Pan, 1, false),
            ',' | '<' => self.step_preset(-1),
            '.' | '>' => self.step_preset(1),
            'p' if self.slots.get(sel).is_some_and(|s| s.inst.presets.len() > 1) => {
                let cur = self.slots[sel].preset;
                self.popup = Popup::Presets { sel: cur, filter: String::new(), channel: None };
            }
            'i' => {
                let items = self.port_items();
                self.popup = Popup::Ports { items, sel: 0 };
            }
            't' => self.activate_port(PortItem::Thru),
            'o' => {
                let items = audio::output_devices();
                let cur = self.audio.as_ref().map(|a| a.device.clone());
                let sel = items.iter().position(|d| Some(d) == cur.as_ref()).unwrap_or(0);
                self.popup = Popup::Devices { items, sel };
            }
            'k' | ' ' => {
                if self.slots.is_empty() {
                    self.error("add an instrument first (a)");
                } else {
                    self.play_mode = true;
                    self.focus = Focus::Rack;
                }
            }
            '+' | '=' => self.set_master(1.0),
            '-' => self.set_master(-1.0),
            'f' => self.open_browser(LoadTarget::Song),
            'S' => self.stop(),
            '{' => self.seek_by(-5.0),
            '}' => self.seek_by(5.0),
            '(' => self.set_speed(self.player.speed - 0.05),
            ')' => self.set_speed(self.player.speed + 0.05),
            'l' => self.toggle_loop(),
            'w' => self.toggle_forward(),
            'v' if sel < self.slots.len() => self.focus = Focus::Channels,
            'M' if sel < self.slots.len() => {
                self.focus = Focus::Mixer;
                self.mix_row = 0;
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ player

    pub fn load_song(&mut self, path: PathBuf) -> bool {
        match smf::load(&path) {
            Ok(song) => {
                let song = Arc::new(song);
                let used = (0..16).filter(|c| song.channels_used & (1 << c) != 0).count();
                self.info(format!(
                    "song: {} · format {} · {} track(s) · {} channel(s) · {:.0} BPM · {}",
                    song.name,
                    song.format,
                    song.tracks,
                    used,
                    song.bpm,
                    format_time(song.duration)
                ));
                if self.slots.is_empty() {
                    self.info("tip: add an SF2 on Omni to play General MIDI files");
                }
                if let Some(dir) = path.parent() {
                    self.browse_dir = dir.to_path_buf();
                }
                self.send(Command::LoadSong(song.clone()));
                self.send(Command::SetLoop(self.player.looping));
                self.send(Command::SetSpeed(self.player.speed));
                self.send(Command::ChannelMutes(self.player.mutes));
                self.player.song = Some(song);
                self.player.path = Some(path);
                self.player.state = PlayState::Stopped;
                self.player.time = 0.0;
                true
            }
            Err(e) => {
                self.error(format!("{e:#}"));
                false
            }
        }
    }

    pub fn toggle_play(&mut self) {
        if self.player.song.is_none() {
            self.open_browser(LoadTarget::Song);
            return;
        }
        if self.player.state == PlayState::Playing {
            self.send(Command::Pause);
            self.player.state = PlayState::Paused;
        } else {
            self.play();
        }
    }

    pub fn play(&mut self) {
        if self.player.song.is_some() {
            self.send(Command::Play);
            self.player.state = PlayState::Playing;
        }
    }

    fn stop(&mut self) {
        if self.player.song.is_some() {
            self.send(Command::Stop);
            self.player.state = PlayState::Stopped;
            self.player.time = 0.0;
        }
    }

    fn seek_to(&mut self, t: f64) {
        let Some(song) = &self.player.song else { return };
        let t = t.clamp(0.0, song.duration);
        self.send(Command::Seek(t));
        self.player.time = t;
    }

    fn seek_by(&mut self, d: f64) {
        self.seek_to(self.player.time + d);
    }

    fn set_speed(&mut self, s: f64) {
        self.player.speed = ((s * 20.0).round() / 20.0).clamp(0.25, 2.0);
        self.send(Command::SetSpeed(self.player.speed));
    }

    fn toggle_loop(&mut self) {
        self.player.looping = !self.player.looping;
        self.send(Command::SetLoop(self.player.looping));
    }

    fn toggle_forward(&mut self) {
        let on = !self.shared.forward_player.load(Ordering::Relaxed);
        self.shared.forward_player.store(on, Ordering::Relaxed);
        let out = self.midi.output_name().unwrap_or_else(|| "no output selected (i)".into());
        self.info(format!("player -> MIDI out {}: {out}", if on { "on" } else { "off" }));
    }

    fn set_mutes(&mut self, m: u16) {
        self.player.mutes = m;
        self.send(Command::ChannelMutes(m));
    }

    fn toggle_channel_mute(&mut self, ch: usize) {
        self.set_mutes(self.player.mutes ^ (1 << ch));
    }

    fn solo_channel(&mut self, ch: usize) {
        let only = !(1u16 << ch);
        self.set_mutes(if self.player.mutes == only { 0 } else { only });
    }

    fn step_preset(&mut self, d: i32) {
        if self.slots.get(self.sel).is_some_and(|s| s.inst.presets.len() > 1) {
            self.adjust(Param::Preset, d, false);
        }
    }

    fn open_browser(&mut self, target: LoadTarget) {
        if matches!(target, LoadTarget::New) && self.slots.len() >= MAX_SLOTS {
            self.error(format!("rack full ({MAX_SLOTS} slots)"));
            return;
        }
        let kind = if matches!(target, LoadTarget::Song) { BrowseKind::Song } else { BrowseKind::Instrument };
        self.popup = Popup::Browser(Browser::new(self.browse_dir.clone(), kind), target);
    }

    pub fn filtered_presets(&self, filter: &str) -> Vec<usize> {
        let Some(s) = self.slots.get(self.sel) else { return Vec::new() };
        let f = filter.to_lowercase();
        (0..s.inst.presets.len())
            .filter(|&i| {
                let p = &s.inst.presets[i];
                f.is_empty()
                    || p.name.to_lowercase().contains(&f)
                    || format!("{:03}:{:03}", p.bank, p.program).contains(&f)
            })
            .collect()
    }

    fn on_popup_key(&mut self, k: KeyEvent) {
        let popup = std::mem::replace(&mut self.popup, Popup::None);
        self.popup = match popup {
            Popup::None => Popup::None,
            Popup::Help => match k.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::F(1) => Popup::None,
                _ => Popup::Help,
            },
            Popup::Browser(mut b, target) => match k.code {
                KeyCode::Esc => Popup::None,
                KeyCode::Up => {
                    b.move_by(-1);
                    Popup::Browser(b, target)
                }
                KeyCode::Down => {
                    b.move_by(1);
                    Popup::Browser(b, target)
                }
                KeyCode::PageUp => {
                    b.move_by(-15);
                    Popup::Browser(b, target)
                }
                KeyCode::PageDown => {
                    b.move_by(15);
                    Popup::Browser(b, target)
                }
                KeyCode::Home => {
                    b.selected = 0;
                    Popup::Browser(b, target)
                }
                KeyCode::End => {
                    b.move_by(isize::MAX / 2);
                    Popup::Browser(b, target)
                }
                KeyCode::Left => {
                    b.up();
                    Popup::Browser(b, target)
                }
                KeyCode::Backspace => {
                    b.backspace();
                    Popup::Browser(b, target)
                }
                KeyCode::Enter | KeyCode::Right => match b.activate() {
                    Some(path) => {
                        if let Some(d) = &b.dir {
                            self.browse_dir = d.clone();
                        }
                        if matches!(target, LoadTarget::Song) {
                            self.load_song(path);
                        } else {
                            self.load(path, target);
                        }
                        Popup::None
                    }
                    None => Popup::Browser(b, target),
                },
                KeyCode::Char(c) => {
                    b.push_filter(c);
                    Popup::Browser(b, target)
                }
                _ => Popup::Browser(b, target),
            },
            Popup::Ports { items, mut sel } => match k.code {
                KeyCode::Esc | KeyCode::Char('i') | KeyCode::Char('q') => Popup::None,
                KeyCode::Up => {
                    sel = sel.saturating_sub(1);
                    Popup::Ports { items, sel }
                }
                KeyCode::Down => {
                    sel = (sel + 1).min(items.len().saturating_sub(1));
                    Popup::Ports { items, sel }
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    if let Some(item) = items.get(sel).cloned() {
                        self.activate_port(item);
                    }
                    Popup::Ports { items: self.port_items(), sel }
                }
                KeyCode::Char('r') => Popup::Ports { items: self.port_items(), sel: 0 },
                _ => Popup::Ports { items, sel },
            },
            Popup::Devices { items, mut sel } => match k.code {
                KeyCode::Esc | KeyCode::Char('o') | KeyCode::Char('q') => Popup::None,
                KeyCode::Up => {
                    sel = sel.saturating_sub(1);
                    Popup::Devices { items, sel }
                }
                KeyCode::Down => {
                    sel = (sel + 1).min(items.len().saturating_sub(1));
                    Popup::Devices { items, sel }
                }
                KeyCode::Enter => {
                    if let Some(d) = items.get(sel).cloned() {
                        self.release_all_keys();
                        self.start_audio(Some(d));
                    }
                    Popup::None
                }
                _ => Popup::Devices { items, sel },
            },
            Popup::Presets { mut sel, mut filter, channel } => {
                let list = self.filtered_presets(&filter);
                let pos = list.iter().position(|&i| i == sel).unwrap_or(0);
                match k.code {
                    KeyCode::Esc => Popup::None,
                    KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown => {
                        let d: isize = match k.code {
                            KeyCode::Up => -1,
                            KeyCode::Down => 1,
                            KeyCode::PageUp => -15,
                            _ => 15,
                        };
                        if !list.is_empty() {
                            let np = (pos as isize + d).clamp(0, list.len() as isize - 1) as usize;
                            sel = list[np];
                        }
                        Popup::Presets { sel, filter, channel }
                    }
                    KeyCode::Enter => {
                        if list.contains(&sel) {
                            let i = self.sel;
                            match channel {
                                Some(ch) => self.send(Command::SetChannelPreset { slot: i, ch, preset: Some(sel) }),
                                None => {
                                    self.slots[i].preset = sel;
                                    self.send(Command::SetPreset { slot: i, preset: sel });
                                }
                            }
                        }
                        Popup::None
                    }
                    KeyCode::Backspace => {
                        filter.pop();
                        Popup::Presets { sel, filter, channel }
                    }
                    KeyCode::Char(c) => {
                        filter.push(c);
                        let list = self.filtered_presets(&filter);
                        if !list.contains(&sel) {
                            sel = list.first().copied().unwrap_or(sel);
                        }
                        Popup::Presets { sel, filter, channel }
                    }
                    _ => Popup::Presets { sel, filter, channel },
                }
            }
        };
    }
}

pub fn format_time(t: f64) -> String {
    let t = t.max(0.0);
    format!("{}:{:04.1}", (t / 60.0) as u32, t % 60.0)
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 1).chain(['…']).collect() }
}

pub fn pan_text(p: f32) -> String {
    let v = (p * 100.0).round() as i32;
    match v {
        0 => "C".into(),
        v if v < 0 => format!("L{}", -v),
        v => format!("R{v}"),
    }
}

/// `Omni`, `none`, `10`, or compact ranges such as `1-9,11-16`.
pub fn channels_text(mask: u16) -> String {
    match mask {
        OMNI => return "Omni".into(),
        0 => return "none".into(),
        _ => {}
    }
    let mut parts = Vec::new();
    let mut c = 0;
    while c < 16 {
        if mask & (1 << c) == 0 {
            c += 1;
            continue;
        }
        let start = c;
        while c + 1 < 16 && mask & (1 << (c + 1)) != 0 {
            c += 1;
        }
        parts.push(if start == c { format!("{}", c + 1) } else { format!("{}-{}", start + 1, c + 1) });
        c += 1;
    }
    parts.join(",")
}

/// Short form for narrow columns.
pub fn channels_short(mask: u16, width: usize) -> String {
    let full = channels_text(mask);
    if full.chars().count() <= width { full } else { format!("{}ch", mask.count_ones()) }
}

pub fn describe_midi(m: &[u8]) -> String {
    if m[0] == 0xF0 {
        return match (m.get(1).copied(), m.get(2).copied().unwrap_or(0)) {
            (Some(crate::smf::SYS_RESET), _) => "SysEx     GM/GS/XG reset".into(),
            (Some(crate::smf::SYS_DRUM_PART), a) => {
                format!("SysEx     ch{} {}", (a & 0x0F) + 1, if a & 0x10 != 0 { "-> drum part" } else { "-> normal part" })
            }
            _ => "SysEx".into(),
        };
    }
    let ch = (m[0] & 0x0F) + 1;
    let d1 = m.get(1).copied().unwrap_or(0);
    let d2 = m.get(2).copied().unwrap_or(0);
    let body = match m[0] & 0xF0 {
        0x80 => format!("Note Off  {:<4} vel {d2}", note_name(d1)),
        0x90 if d2 == 0 => format!("Note Off  {:<4}", note_name(d1)),
        0x90 => format!("Note On   {:<4} vel {d2}", note_name(d1)),
        0xA0 => format!("Aftertouch {} {d2}", note_name(d1)),
        0xB0 => format!("CC {d1:<3} = {d2}{}", cc_name(d1)),
        0xC0 => format!("Program   {d1}"),
        0xD0 => format!("Pressure  {d1}"),
        0xE0 => format!("Pitchbend {:+}", ((d2 as i32) << 7 | d1 as i32) - 8192),
        _ => format!("{m:02X?}"),
    };
    format!("ch{ch:<2} {body}")
}

fn cc_name(cc: u8) -> &'static str {
    match cc {
        0 => " (bank)",
        1 => " (mod)",
        7 => " (volume)",
        10 => " (pan)",
        11 => " (expr)",
        32 => " (bank lsb)",
        64 => " (sustain)",
        120 => " (sound off)",
        121 => " (reset)",
        123 => " (notes off)",
        _ => "",
    }
}
