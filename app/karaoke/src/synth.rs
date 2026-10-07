//! The backing-track player: a `solfege_synth` engine with a rack of
//! SoundFonts (or SFZ instruments). Every MIDI channel is routed to one of
//! them; each font in use gets a melodic slot for its channels and, when
//! it plays the drums, a slot for channel 10, so key changes move every
//! part except the drums. A channel can also have one sound pinned,
//! overriding the song's program changes.
//!
//! The engine normally runs inside the audio callback. Without a usable
//! output device it runs on a paced thread instead, so lyrics still follow
//! the song (silently) and the rest of the app behaves the same.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender};
use solfege_synth::audio::{self, AudioOut};
use solfege_synth::engine::mixer::{DRUM_STRIP_BASE, FxParams, GM_GROUP_NAMES, NoteGroups, StripParams};
use solfege_synth::engine::{Command, Engine, Garbage, NO_PRESET, PlayState, Shared, Slot, SlotParams, load_peak};
use solfege_synth::instrument::{self, Instrument, db_to_gain};
use solfege_synth::smf;

pub const KEY_RANGE: i32 = 12;
/// Fonts in the rack (two engine slots each at most).
pub const MAX_FONTS: usize = 8;
/// MIDI channel 10, the drum kit.
pub const DRUM_CH: usize = 9;
/// MIDI channel 9 (index 8): the guide melody of NCN karaoke songs.
pub const MELODY_CH: usize = 8;
/// Drum kit pieces with their own mixer strip (GM note groups).
pub const KIT: usize = 6;

pub fn kit_name(group: usize) -> &'static str {
    GM_GROUP_NAMES[group]
}

/// Channel 10 keys of a kit piece (GM key map), bit n = key n.
pub fn piece_keys(group: usize) -> u128 {
    let groups = NoteGroups::gm();
    (0..128).filter(|&k| groups.map[k] as usize == group).fold(0u128, |m, k| m | (1u128 << k))
}

/// Mixer settings the app owns and replays into every new engine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mixer {
    /// One strip per MIDI channel of the melodic slot (10 is unused there).
    pub channels: [StripParams; 16],
    /// Drum kit pieces on channel 10.
    pub kit: [StripParams; KIT],
    pub fx: FxParams,
}

impl Default for Mixer {
    fn default() -> Self {
        Self { channels: [StripParams::default(); 16], kit: [StripParams::default(); KIT], fx: FxParams::default() }
    }
}

/// A mixer strip as the app addresses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripId {
    Channel(usize),
    Kit(usize),
}

/// A SoundFont (or SFZ) in the rack.
pub struct Font {
    pub path: PathBuf,
    pub inst: Option<Arc<Instrument>>,
    pub error: Option<String>,
    job: Option<Receiver<Result<Instrument>>>,
}

impl Font {
    pub fn name(&self) -> String {
        match &self.inst {
            Some(i) => i.name.clone(),
            None => self.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        }
    }

    pub fn loading(&self) -> bool {
        self.job.is_some()
    }
}

/// A sound chosen for one GM instrument (program), from any font.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstrumentSound {
    pub font: usize,
    pub bank: u16,
    pub program: u8,
}

/// One engine slot: which font, which channels, and what it is for.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RackSlot {
    font: usize,
    channels: u16,
    drums: bool,
    /// Channels split by program (see `SlotParams::filtered`).
    filtered: u16,
    programs: u128,
    /// Program -> preset of this font, for instrument overrides.
    map: Option<Box<[u16; 128]>>,
    /// Channel 10 keys this slot plays (see `SlotParams::drum_keys`).
    keys: u128,
    /// Kit locked on channel 10, for the slot of a separate kit piece.
    lock: Option<usize>,
}

impl RackSlot {
    fn plain(font: usize, channels: u16, drums: bool) -> Self {
        Self { font, channels, drums, filtered: 0, programs: u128::MAX, map: None, keys: u128::MAX, lock: None }
    }
}

enum Output {
    None,
    /// Holding the stream keeps it playing.
    Audio(#[allow(dead_code)] AudioOut),
    /// Engine rendered on a thread at real-time pace, sound discarded.
    Silent(Arc<AtomicBool>),
}

impl Drop for Output {
    fn drop(&mut self) {
        if let Output::Silent(stop) = self {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

pub enum SynthEvent {
    SoundFontLoaded { name: String, presets: usize },
    SoundFontFailed(String),
    /// The song played to its end on its own.
    Ended,
}

pub struct Synth {
    tx: Sender<Command>,
    rx: Receiver<Command>,
    garbage_tx: Sender<Garbage>,
    garbage_rx: Receiver<Garbage>,
    pub shared: Arc<Shared>,
    output: Output,
    /// One-line description of the output, or why there is no sound.
    pub output_info: String,
    pub output_error: Option<String>,
    fonts: Vec<Font>,
    /// Font index per MIDI channel (channel 10 plays the drum kit).
    routing: [usize; 16],
    /// Sound pinned per channel (a preset of the channel's font), this song.
    pins: [Option<usize>; 16],
    /// Drum kit locked on channel 10 across songs, as (bank, program).
    drum_lock: Option<(u16, u8)>,
    /// Sound per GM instrument, whichever channel plays it.
    instruments: BTreeMap<u8, InstrumentSound>,
    /// A kit (font and drum preset) of its own per kit piece; `None`
    /// plays the piece from channel 10's kit.
    pieces: [Option<InstrumentSound>; KIT],
    /// Engine slots as last sent.
    rack: Vec<RackSlot>,
    song: Option<Arc<smf::Song>>,
    state: PlayState,
    /// The engine has reported `Playing` since the last play command, so a
    /// later `Stopped` means the song ran out.
    seen_playing: bool,
    key: i32,
    speed: f64,
    volume: f32,
    mixer: Mixer,
    /// The guide melody is muted, whatever its mixer strip says.
    melody_off: bool,
}

impl Synth {
    pub fn new() -> Self {
        let (tx, rx) = crossbeam_channel::bounded(1024);
        let (garbage_tx, garbage_rx) = crossbeam_channel::bounded(1024);
        Self {
            tx,
            rx,
            garbage_tx,
            garbage_rx,
            shared: Arc::new(Shared::new()),
            output: Output::None,
            output_info: String::new(),
            output_error: None,
            fonts: Vec::new(),
            routing: [0; 16],
            pins: [None; 16],
            drum_lock: None,
            instruments: BTreeMap::new(),
            pieces: [None; KIT],
            rack: Vec::new(),
            song: None,
            state: PlayState::Empty,
            seen_playing: false,
            key: 0,
            speed: 1.0,
            volume: 0.8,
            mixer: Mixer::default(),
            melody_off: false,
        }
    }

    fn send(&self, cmd: Command) {
        let _ = self.tx.try_send(cmd);
    }

    // ------------------------------------------------------------ output

    /// Open `device` (or the default output). Falls back to the silent
    /// clock when no device can be opened.
    pub fn start_output(&mut self, device: Option<&str>) {
        self.output = Output::None;
        while self.rx.try_recv().is_ok() {}
        match audio::start(device, None, self.rx.clone(), self.garbage_tx.clone(), self.shared.clone()) {
            Ok(a) => {
                self.output_info = format!("{} · {} Hz · buffer {}", a.device, a.sample_rate, a.buffer);
                self.output_error = None;
                self.output = Output::Audio(a);
            }
            Err(e) => {
                self.output_error = Some(format!("{e:#}"));
                self.output_info = "no audio output".into();
                self.output = Output::Silent(spawn_silent(self.rx.clone(), self.garbage_tx.clone(), self.shared.clone()));
            }
        }
        self.replay();
    }

    /// Rebuild everything inside a fresh engine.
    fn replay(&mut self) {
        self.send(Command::MasterGain(db_to_gain(volume_db(self.volume))));
        self.send(Command::SetFx(self.mixer.fx));
        if let Some(song) = &self.song {
            self.send(Command::LoadSong(song.clone()));
            self.send(Command::SetSpeed(self.speed));
        }
        // A fresh engine has no slots.
        self.rack.clear();
        self.rebuild();
    }

    /// The font a channel actually plays: its routing, or the first loaded
    /// font while that one is missing.
    fn effective_font(&self, ch: usize) -> Option<usize> {
        let f = self.routing[ch];
        if self.fonts.get(f).is_some_and(|f| f.inst.is_some()) {
            Some(f)
        } else {
            self.fonts.iter().position(|f| f.inst.is_some())
        }
    }

    /// Instrument overrides that can sound: font loaded and preset found,
    /// as program -> (font, preset).
    fn resolved_instruments(&self) -> BTreeMap<u8, (usize, usize)> {
        self.instruments
            .iter()
            .filter_map(|(&prog, s)| {
                let inst = self.fonts.get(s.font)?.inst.as_ref()?;
                Some((prog, (s.font, inst.find_preset(s.bank, s.program)?)))
            })
            .collect()
    }

    /// Channels with a pinned sound; instrument overrides leave them alone.
    fn pinned(&self) -> u16 {
        (0..16).filter(|&c| self.pins[c].is_some()).fold(0u16, |m, c| m | (1 << c))
    }

    /// Kit pieces with a sound of their own that can sound, as
    /// (font, preset) -> keys.
    fn resolved_pieces(&self) -> BTreeMap<(usize, usize), u128> {
        let mut out = BTreeMap::new();
        for (g, s) in self.pieces.iter().enumerate() {
            let Some(s) = s else { continue };
            let Some(preset) = self.fonts.get(s.font).and_then(|f| f.inst.as_ref()).and_then(|i| i.find_preset(s.bank, s.program)) else { continue };
            *out.entry((s.font, preset)).or_insert(0) |= piece_keys(g);
        }
        out
    }

    fn layout(&self) -> Vec<RackSlot> {
        let overrides = self.resolved_instruments();
        let pieces = self.resolved_pieces();
        let separate: u128 = pieces.values().fold(0, |m, k| m | k);
        let overridden: u128 = overrides.keys().fold(0, |m, &p| m | (1u128 << p));
        let melodic = !(1u16 << DRUM_CH);
        let split = if overridden == 0 { 0 } else { melodic & !self.pinned() };
        let mut out = Vec::new();
        for font in 0..self.fonts.len() {
            let channels = (0..16).filter(|&c| c != DRUM_CH && self.effective_font(c) == Some(font)).fold(0u16, |m, c| m | (1 << c));
            if channels != 0 {
                out.push(RackSlot { filtered: channels & split, programs: !overridden, ..RackSlot::plain(font, channels, false) });
            }
            if self.effective_font(DRUM_CH) == Some(font) {
                out.push(RackSlot { keys: !separate, ..RackSlot::plain(font, 1 << DRUM_CH, true) });
            }
        }
        // One slot per font that overrides instruments, over every channel
        // that is not pinned; it only plays the programs it overrides.
        if split != 0 {
            let fonts: std::collections::BTreeSet<usize> = overrides.values().map(|v| v.0).collect();
            for font in fonts {
                let mut map = Box::new([NO_PRESET; 128]);
                let mut programs = 0u128;
                for (&prog, &(f, preset)) in &overrides {
                    if f == font {
                        map[prog as usize] = preset as u16;
                        programs |= 1u128 << prog;
                    }
                }
                out.push(RackSlot { filtered: split, programs, map: Some(map), ..RackSlot::plain(font, split, false) });
            }
        }
        // One slot per separate kit, playing only its pieces' keys.
        for ((font, preset), keys) in pieces {
            out.push(RackSlot { keys, lock: Some(preset), ..RackSlot::plain(font, 1 << DRUM_CH, true) });
        }
        out
    }

    fn slot_params(&self, r: &RackSlot) -> SlotParams {
        SlotParams {
            channels: r.channels,
            transpose: if r.drums { 0 } else { self.key },
            filtered: r.filtered,
            programs: r.programs,
            drum_keys: r.keys,
            ..SlotParams::default()
        }
    }

    /// Bring the engine's slots in line with the fonts, routing, pins and
    /// instrument overrides.
    fn rebuild(&mut self) {
        let layout = self.layout();
        if layout == self.rack {
            return;
        }
        for _ in 0..self.rack.len() {
            self.send(Command::RemoveSlot(0));
        }
        for (i, r) in layout.iter().enumerate() {
            let inst = self.fonts[r.font].inst.clone().expect("layout only uses loaded fonts");
            // A slot listening to channel 10 alone is not multitimbral, so
            // its default preset is what the drums play: the font's kit.
            let preset = if r.drums { drum_kit(&inst) } else { 0 };
            self.send(Command::AddSlot(Box::new(Slot::new(inst, preset, self.slot_params(r)))));
            if let Some(map) = &r.map {
                self.send(Command::SetProgramMap { slot: i, map: map.clone() });
            }
        }
        self.rack = layout;
        self.send_strips();
        for ch in 0..16 {
            self.send_pin(ch);
        }
        for (slot, r) in self.rack.iter().enumerate() {
            if let Some(preset) = r.lock {
                self.send(Command::SetChannelPreset { slot, ch: DRUM_CH as u8, preset: Some(preset) });
            }
        }
        // Program changes already played must reach the new slots.
        if self.state == PlayState::Playing || self.state == PlayState::Paused {
            self.send(Command::Seek(self.time()));
        }
    }

    /// Engine slot of the font `ch` is routed to (where its pin lives).
    fn slot_of(&self, ch: usize) -> Option<usize> {
        let font = self.effective_font(ch)?;
        self.rack.iter().position(|r| r.font == font && r.map.is_none() && r.lock.is_none() && r.drums == (ch == DRUM_CH))
    }

    /// Every engine slot that can sound `ch`.
    fn slots_of(&self, ch: usize) -> impl Iterator<Item = usize> + '_ {
        self.rack.iter().enumerate().filter(move |(_, r)| r.channels & (1 << ch) != 0).map(|(i, _)| i)
    }

    /// Engine (slot, strip) pairs behind a mixer strip.
    fn engine_strips(&self, id: StripId) -> Vec<(usize, usize)> {
        match id {
            StripId::Channel(ch) => self.slots_of(ch).map(|s| (s, ch)).collect(),
            // Every slot on channel 10: the kit and any separate pieces.
            StripId::Kit(g) => self.rack.iter().enumerate().filter(|(_, r)| r.drums).map(|(s, _)| (s, DRUM_STRIP_BASE + g)).collect(),
        }
    }

    fn send_pin(&self, ch: usize) {
        if let Some(slot) = self.slot_of(ch) {
            self.send(Command::SetChannelPreset { slot, ch: ch as u8, preset: self.pin(ch) });
        }
    }

    fn send_strips(&self) {
        for ch in 0..16 {
            self.send_strip(StripId::Channel(ch));
        }
        for g in 0..KIT {
            self.send_strip(StripId::Kit(g));
        }
    }

    /// What the engine gets for a strip. Drum kit pieces also follow the
    /// channel 10 strip, which acts as the fader for the whole kit.
    fn engine_params(&self, id: StripId) -> StripParams {
        match id {
            StripId::Channel(MELODY_CH) if self.melody_off => StripParams { mute: true, ..self.strip(id) },
            StripId::Channel(_) => self.strip(id),
            StripId::Kit(g) => {
                let kit = self.mixer.kit[g];
                let all = self.mixer.channels[DRUM_CH];
                StripParams {
                    gain_db: if all.gain_db <= -60.0 { -60.0 } else { kit.gain_db + all.gain_db },
                    pan: (kit.pan + all.pan).clamp(-1.0, 1.0),
                    mute: kit.mute || all.mute,
                    solo: kit.solo || all.solo,
                    reverb_add: kit.reverb_add + all.reverb_add,
                    chorus_add: kit.chorus_add + all.chorus_add,
                    ..kit
                }
                .clamped()
            }
        }
    }

    fn send_strip(&self, id: StripId) {
        let params = self.engine_params(id);
        for (slot, strip) in self.engine_strips(id) {
            self.send(Command::SetStrip { slot, strip, params });
        }
    }

    // -------------------------------------------------------- soundfonts

    /// Add a SoundFont (or SFZ) to the rack; it loads on a background
    /// thread and `poll` reports the result. The first font plays every
    /// channel until routed otherwise.
    pub fn add_font(&mut self, path: PathBuf) -> Result<usize, String> {
        if let Some(i) = self.fonts.iter().position(|f| f.path == path) {
            return Ok(i);
        }
        if self.fonts.len() >= MAX_FONTS {
            return Err(format!("ใส่ SoundFont ได้สูงสุด {MAX_FONTS} ไฟล์"));
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        let p = path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(instrument::load(&p));
        });
        self.fonts.push(Font { path, inst: None, error: None, job: Some(rx) });
        Ok(self.fonts.len() - 1)
    }

    pub fn remove_font(&mut self, index: usize) {
        if index >= self.fonts.len() {
            return;
        }
        // Fix the routing before the indices shift.
        for (ch, r) in self.routing.iter_mut().enumerate() {
            if *r == index {
                *r = 0;
                self.pins[ch] = None;
                if ch == DRUM_CH {
                    self.drum_lock = None;
                }
            } else if *r > index {
                *r -= 1;
            }
        }
        self.instruments.retain(|_, s| s.font != index);
        for s in self.instruments.values_mut() {
            if s.font > index {
                s.font -= 1;
            }
        }
        for p in &mut self.pieces {
            match p {
                Some(s) if s.font == index => *p = None,
                Some(s) if s.font > index => s.font -= 1,
                _ => {}
            }
        }
        self.fonts.remove(index);
        // Slots referring to old indices: rebuild from scratch.
        let old = std::mem::take(&mut self.rack);
        for _ in 0..old.len() {
            self.send(Command::RemoveSlot(0));
        }
        self.rebuild();
    }

    pub fn fonts(&self) -> &[Font] {
        &self.fonts
    }

    pub fn font_paths(&self) -> Vec<PathBuf> {
        self.fonts.iter().map(|f| f.path.clone()).collect()
    }

    pub fn loading_soundfont(&self) -> bool {
        self.fonts.iter().any(Font::loading)
    }

    /// Any font ready to play.
    pub fn has_font(&self) -> bool {
        self.fonts.iter().any(|f| f.inst.is_some())
    }

    pub fn routing(&self) -> [usize; 16] {
        self.routing
    }

    /// Route channel `ch` (0-based; 9 = drums) to font `font`.
    pub fn set_route(&mut self, ch: usize, font: usize) {
        if ch < 16 && font < self.fonts.len().max(1) && self.routing[ch] != font {
            self.routing[ch] = font;
            self.pins[ch] = None;
            self.rebuild();
        }
    }

    /// Route every channel (drums included) to one font.
    pub fn route_all(&mut self, font: usize) {
        for ch in 0..16 {
            if self.routing[ch] != font {
                self.routing[ch] = font;
                self.pins[ch] = None;
            }
        }
        self.rebuild();
    }

    /// Restore saved routing (indices past the font list fall back to 0).
    pub fn set_routing(&mut self, routing: [usize; 16]) {
        self.routing = routing.map(|f| if f < self.fonts.len() { f } else { 0 });
        self.rebuild();
    }

    /// Pinned sound of a channel; on channel 10 the locked drum kit.
    pub fn pin(&self, ch: usize) -> Option<usize> {
        if ch == DRUM_CH {
            let (bank, program) = self.drum_lock?;
            return self.channel_font(DRUM_CH)?.find_preset(bank, program);
        }
        self.pins[ch]
    }

    /// Pin a sound (a preset of the channel's font) on a channel, or
    /// `None` to follow the song's program changes again. On channel 10
    /// this locks the drum kit for every song until unlocked.
    pub fn set_pin(&mut self, ch: usize, preset: Option<usize>) {
        if ch == DRUM_CH {
            self.drum_lock = preset.and_then(|p| self.channel_font(DRUM_CH)?.presets.get(p)).map(|p| (p.bank, p.program));
            self.send_pin(ch);
            return;
        }
        let was_pinned = self.pins[ch].is_some();
        self.pins[ch] = preset;
        if was_pinned != preset.is_some() && !self.instruments.is_empty() {
            // Pinned channels leave the instrument-override slots.
            self.rebuild();
        }
        self.send_pin(ch);
    }

    /// Locked drum kit as (bank, program).
    pub fn drum_lock(&self) -> Option<(u16, u8)> {
        self.drum_lock
    }

    /// The guide melody (channel 9) is muted.
    pub fn melody_off(&self) -> bool {
        self.melody_off
    }

    /// Mute or bring back the guide melody; a setting, so it survives the
    /// per-song mixer reset.
    pub fn set_melody_off(&mut self, off: bool) {
        self.melody_off = off;
        self.send_strip(StripId::Channel(MELODY_CH));
    }

    pub fn set_drum_lock(&mut self, lock: Option<(u16, u8)>) {
        self.drum_lock = lock;
        self.send_pin(DRUM_CH);
    }

    /// The kit each piece plays from (`None` = channel 10's kit).
    pub fn pieces(&self) -> &[Option<InstrumentSound>; KIT] {
        &self.pieces
    }

    /// Give a kit piece (kick, snare, ...) a kit of its own, from any font,
    /// or `None` to play it from channel 10's kit again.
    pub fn set_piece(&mut self, group: usize, sound: Option<InstrumentSound>) {
        if group >= KIT {
            return;
        }
        self.pieces[group] = sound.filter(|s| s.font < self.fonts.len());
        self.rebuild();
    }

    /// Name of the kit a piece sounds from now.
    pub fn piece_sound(&self, group: usize) -> Option<String> {
        match self.pieces.get(group).copied().flatten() {
            Some(s) => {
                let inst = self.fonts.get(s.font)?.inst.as_ref()?;
                Some(inst.presets.get(inst.find_preset(s.bank, s.program)?)?.name.clone())
            }
            None => self.channel_sound(DRUM_CH).map(str::to_string),
        }
    }

    pub fn instruments(&self) -> &BTreeMap<u8, InstrumentSound> {
        &self.instruments
    }

    /// Play GM program `program` with `sound` on every channel that is not
    /// pinned, or `None` to use the channel's font again.
    pub fn set_instrument(&mut self, program: u8, sound: Option<InstrumentSound>) {
        let program = program & 127;
        match sound {
            Some(s) if s.font < self.fonts.len() => {
                self.instruments.insert(program, s);
            }
            _ => {
                self.instruments.remove(&program);
            }
        }
        self.rebuild();
    }

    /// The font instrument a channel plays.
    pub fn channel_font(&self, ch: usize) -> Option<&Arc<Instrument>> {
        self.fonts.get(self.effective_font(ch)?)?.inst.as_ref()
    }

    // -------------------------------------------------------------- song

    /// Load a backing track from Standard MIDI File bytes.
    pub fn load_midi(&mut self, bytes: &[u8], name: &str) -> Result<()> {
        let song = Arc::new(smf::parse(bytes, name)?);
        self.send(Command::LoadSong(song.clone()));
        self.send(Command::SetSpeed(self.speed));
        self.song = Some(song);
        self.state = PlayState::Stopped;
        self.seen_playing = false;
        Ok(())
    }

    pub fn state(&self) -> PlayState {
        self.state
    }

    pub fn play(&mut self) {
        if self.song.is_some() {
            self.send(Command::Play);
            self.state = PlayState::Playing;
            self.seen_playing = false;
        }
    }

    pub fn pause(&mut self) {
        if self.state == PlayState::Playing {
            self.send(Command::Pause);
            self.state = PlayState::Paused;
        }
    }

    pub fn toggle(&mut self) {
        if self.state == PlayState::Playing { self.pause() } else { self.play() }
    }

    pub fn stop(&mut self) {
        if self.song.is_some() {
            self.send(Command::Stop);
            self.state = PlayState::Stopped;
        }
    }

    pub fn seek(&mut self, t: f64) {
        if self.song.is_some() {
            self.send(Command::Seek(t.clamp(0.0, self.duration())));
        }
    }

    /// Song position in seconds.
    pub fn time(&self) -> f64 {
        if self.song.is_none() {
            return 0.0;
        }
        f64::from_bits(self.shared.player_time.load(Ordering::Relaxed))
    }

    pub fn duration(&self) -> f64 {
        self.song.as_ref().map_or(0.0, |s| s.duration)
    }

    pub fn channels_used(&self) -> u16 {
        self.song.as_ref().map_or(0, |s| s.channels_used)
    }

    // ---------------------------------------------------------- controls

    pub fn key(&self) -> i32 {
        self.key
    }

    pub fn set_key(&mut self, key: i32) {
        self.key = key.clamp(-KEY_RANGE, KEY_RANGE);
        for (slot, r) in self.rack.iter().enumerate() {
            self.send(Command::SetParams { slot, params: self.slot_params(r) });
        }
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    pub fn set_speed(&mut self, speed: f64) {
        self.speed = ((speed * 20.0).round() / 20.0).clamp(0.5, 1.5);
        self.send(Command::SetSpeed(self.speed));
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.0);
        self.send(Command::MasterGain(db_to_gain(volume_db(self.volume))));
    }

    pub fn mixer(&self) -> &Mixer {
        &self.mixer
    }

    pub fn strip(&self, id: StripId) -> StripParams {
        match id {
            StripId::Channel(ch) => self.mixer.channels[ch],
            StripId::Kit(g) => self.mixer.kit[g],
        }
    }

    pub fn set_strip(&mut self, id: StripId, params: StripParams) {
        let params = params.clamped();
        match id {
            StripId::Channel(ch) => self.mixer.channels[ch] = params,
            StripId::Kit(g) => self.mixer.kit[g] = params,
        }
        self.send_strip(id);
        if id == StripId::Channel(DRUM_CH) {
            for g in 0..KIT {
                self.send_strip(StripId::Kit(g));
            }
        }
    }

    pub fn set_fx(&mut self, fx: FxParams) {
        self.mixer.fx = fx.clamped();
        self.send(Command::SetFx(self.mixer.fx));
    }

    /// Channel strips and pinned sounds back to neutral (each song has
    /// its own parts); routing, the drum kit and effects stay as set.
    pub fn reset_channels(&mut self) {
        self.mixer.channels = [StripParams::default(); 16];
        let had_pins = self.pinned() != 0;
        self.pins = [None; 16];
        self.send_strips();
        if had_pins && !self.instruments.is_empty() {
            self.rebuild();
        }
        for ch in 0..16 {
            self.send_pin(ch);
        }
    }

    /// Any channel or kit strip muted or soloed.
    pub fn mixer_touched(&self) -> bool {
        let m = &self.mixer;
        m.channels.iter().chain(&m.kit).any(|s| s.mute || s.solo)
    }

    /// Peak level (left, right) of a strip, 0..1+.
    pub fn strip_peak(&self, id: StripId) -> (f32, f32) {
        // Channel 10 shows the whole kit.
        if id == StripId::Channel(DRUM_CH) {
            return (0..KIT).map(|g| self.strip_peak(StripId::Kit(g))).fold((0.0, 0.0), |a, b| (a.0.max(b.0), a.1.max(b.1)));
        }
        self.engine_strips(id).into_iter().fold((0.0, 0.0), |acc, (slot, strip)| {
            let m = &self.shared.slots[slot];
            (acc.0.max(load_peak(&m.strip_l[strip])), acc.1.max(load_peak(&m.strip_r[strip])))
        })
    }

    pub fn master_peak(&self) -> (f32, f32) {
        (load_peak(&self.shared.master_l), load_peak(&self.shared.master_r))
    }

    /// Sound (preset) a channel is playing: its pin, or what the song's
    /// program change selected.
    pub fn channel_sound(&self, ch: usize) -> Option<&str> {
        let info = self.shared.slots[self.slot_of(ch)?].channels[ch].load();
        if info.program.is_none() && !info.locked && ch != DRUM_CH {
            return None;
        }
        let slot = self.sounding_slot(ch)?;
        let preset = self.shared.slots[slot].channels[ch].load().preset;
        let font = self.fonts.get(self.rack[slot].font)?.inst.as_ref()?;
        font.presets.get(preset).map(|p| p.name.as_str())
    }

    /// The song's own pan for a channel (MIDI CC 10), -1 (left) .. 1 (right).
    pub fn midi_pan(&self, ch: usize) -> f32 {
        let Some(slot) = self.slot_of(ch) else { return 0.0 };
        let raw = self.shared.slots[slot].channels[ch].load().pan as f32;
        ((raw - 64.0) / 63.0).clamp(-1.0, 1.0)
    }

    /// The song's own effect sends for a channel, 0..1: reverb (MIDI CC 91)
    /// and chorus (CC 93). A strip's `reverb` / `chorus` scale these.
    pub fn midi_sends(&self, ch: usize) -> (f32, f32) {
        let Some(slot) = self.slot_of(ch) else { return (40.0 / 127.0, 0.0) };
        let info = self.shared.slots[slot].channels[ch].load();
        (info.reverb as f32 / 127.0, info.chorus as f32 / 127.0)
    }

    /// Level going into the reverb and chorus (what the sends add up to).
    pub fn fx_peak(&self) -> (f32, f32) {
        (load_peak(&self.shared.fx_peak[0]), load_peak(&self.shared.fx_peak[1]))
    }

    /// Font a channel is sounding from right now (an instrument override
    /// can differ from the channel's routing).
    pub fn sounding_font(&self, ch: usize) -> Option<usize> {
        Some(self.rack[self.sounding_slot(ch)?].font)
    }

    /// The slot that plays new notes on `ch`: an instrument-override slot
    /// when the channel's current program is overridden, else its own.
    fn sounding_slot(&self, ch: usize) -> Option<usize> {
        let home = self.slot_of(ch)?;
        let program = self.shared.slots[home].channels[ch].load().program.unwrap_or(0) as u32;
        let overridden = |s: &usize| {
            let r = &self.rack[*s];
            r.map.is_some() && r.filtered & (1 << ch) != 0 && (r.programs >> program) & 1 == 1
        };
        Some(self.slots_of(ch).find(overridden).unwrap_or(home))
    }

    // -------------------------------------------------------------- poll

    /// Once per frame: free engine garbage, finish background loads and
    /// notice the end of the song.
    pub fn poll(&mut self) -> Vec<SynthEvent> {
        while self.garbage_rx.try_recv().is_ok() {}
        let mut events = Vec::new();
        let mut loaded = false;
        for f in &mut self.fonts {
            let Some(Ok(result)) = f.job.as_ref().map(|j| j.try_recv()) else { continue };
            f.job = None;
            match result {
                Ok(inst) => {
                    events.push(SynthEvent::SoundFontLoaded { name: inst.name.clone(), presets: inst.presets.len() });
                    f.inst = Some(Arc::new(inst));
                    loaded = true;
                }
                Err(e) => {
                    let msg = format!("{}: {e:#}", f.path.display());
                    f.error = Some(msg.clone());
                    events.push(SynthEvent::SoundFontFailed(msg));
                }
            }
        }
        if loaded {
            self.rebuild();
        }
        if self.state == PlayState::Playing {
            match PlayState::from_u32(self.shared.player_state.load(Ordering::Relaxed)) {
                PlayState::Playing => self.seen_playing = true,
                PlayState::Stopped if self.seen_playing => {
                    self.state = PlayState::Stopped;
                    events.push(SynthEvent::Ended);
                }
                _ => {}
            }
        }
        events
    }
}

/// The standard drum kit of a font (bank 128), or its first kit.
fn drum_kit(inst: &Instrument) -> usize {
    inst.find_preset(128, 0).or_else(|| inst.presets.iter().position(|p| p.bank == 128)).unwrap_or(0)
}

/// Volume slider (0..1) to dB: 0.8 is unity, the bottom is silence.
pub fn volume_db(v: f32) -> f32 {
    if v <= 0.001 { -120.0 } else { 40.0 * (v / 0.8).log10() }
}

fn spawn_silent(rx: Receiver<Command>, garbage: Sender<Garbage>, shared: Arc<Shared>) -> Arc<AtomicBool> {
    const SR: f32 = 48_000.0;
    const BLOCK: usize = 480;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::Builder::new()
        .name("silent-engine".into())
        .spawn(move || {
            let mut engine = Engine::new(SR, rx, garbage, shared);
            let period = Duration::from_secs_f64(BLOCK as f64 / SR as f64);
            let mut next = Instant::now();
            while !flag.load(Ordering::Relaxed) {
                engine.render(BLOCK);
                next += period;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                } else if now - next > Duration::from_millis(200) {
                    next = now;
                }
            }
        })
        .expect("spawn engine thread");
    stop
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_curve() {
        assert!(volume_db(0.8).abs() < 1e-4);
        assert!(volume_db(0.0) <= -100.0);
        assert!(volume_db(1.0) > 3.0 && volume_db(1.0) < 4.0);
    }

    #[test]
    fn silent_engine_plays_a_song_to_the_end() {
        // Two quarter notes at 120 BPM: about one second of song.
        let mut midi = b"MThd\0\0\0\x06\0\0\0\x01\x01\xE0MTrk".to_vec();
        let track = [
            0x00, 0x90, 60, 100, 0x83, 0x60, 0x80, 60, 0, 0x00, 0x90, 62, 100, 0x83, 0x60, 0x80, 62, 0, 0x00, 0xFF, 0x2F, 0,
        ];
        midi.extend((track.len() as u32).to_be_bytes());
        midi.extend(track);
        let dir = std::env::temp_dir().join(format!("karaoke-synth-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("two.mid");
        std::fs::write(&path, midi).unwrap();

        let mut s = Synth::new();
        // A device name nothing matches forces the silent engine.
        s.start_output(Some("\u{1}no such device\u{1}"));
        assert!(s.output_error.is_some());
        s.load_midi(&std::fs::read(&path).unwrap(), "two").unwrap();
        assert!((s.duration() - 1.0).abs() < 0.01, "{}", s.duration());
        s.set_speed(1.5);
        s.play();
        let t0 = Instant::now();
        let mut ended = false;
        let mut max_time = 0.0f64;
        while t0.elapsed() < Duration::from_secs(3) && !ended {
            max_time = max_time.max(s.time());
            ended = s.poll().iter().any(|e| matches!(e, SynthEvent::Ended));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(ended, "song should end on its own");
        assert!(max_time > 0.5, "clock advanced to {max_time}");
        assert!(t0.elapsed() < Duration::from_millis(1500), "1.5x speed: {:?}", t0.elapsed());
        assert_eq!(s.state(), PlayState::Stopped);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn soundfont_plays_melody_and_drums_on_separate_slots() {
        let midi = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shared/NCN/Song/Z/Z2608001.mid");
        let Some(sf2) = crate::library::find_soundfont().filter(|_| midi.is_file()) else {
            eprintln!("skipped: needs a .sf2 and the shared/NCN sample library");
            return;
        };
        let mut s = Synth::new();
        s.start_output(Some("\u{1}no such device\u{1}"));
        s.add_font(sf2).unwrap();
        let t0 = Instant::now();
        while s.loading_soundfont() && t0.elapsed() < Duration::from_secs(60) {
            for e in s.poll() {
                if let SynthEvent::SoundFontFailed(e) = e {
                    panic!("{e}");
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(s.has_font());
        // Before any program change, channel 10 already sits on a drum kit.
        std::thread::sleep(Duration::from_millis(50));
        let info = s.shared.slots[1].channels[DRUM_CH].load();
        assert_eq!(s.channel_font(DRUM_CH).unwrap().presets[info.preset].bank, 128, "default drum kit");
        s.load_midi(&std::fs::read(&midi).unwrap(), "Z2608001").unwrap();
        s.set_key(2);
        s.play();
        s.seek(20.0);
        let shared = s.shared.clone();
        let voices = |slot: usize| shared.slots[slot].voices.load(Ordering::Relaxed);
        let (mut melodic, mut drums) = (0, 0);
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(3) {
            s.poll();
            melodic = melodic.max(voices(0));
            drums = drums.max(voices(1));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(melodic > 0 && drums > 0, "melodic {melodic}, drums {drums}");
        let kit = s.channel_font(DRUM_CH).unwrap().presets[shared.slots[1].channels[DRUM_CH].load().preset].bank;
        assert_eq!(kit, 128, "channel 10 plays a drum kit");
        assert!(s.time() > 20.0);
    }

    #[test]
    fn mixer_strips_reach_the_engine() {
        let midi = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shared/NCN/Song/Z/Z2608001.mid");
        let Some(sf2) = crate::library::find_soundfont().filter(|_| midi.is_file()) else {
            eprintln!("skipped: needs a .sf2 and the shared/NCN sample library");
            return;
        };
        let mut s = Synth::new();
        s.start_output(Some("\u{1}no such device\u{1}"));
        s.add_font(sf2).unwrap();
        let t0 = Instant::now();
        while s.loading_soundfont() && t0.elapsed() < Duration::from_secs(60) {
            s.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        s.load_midi(&std::fs::read(&midi).unwrap(), "Z2608001").unwrap();
        s.play();
        s.seek(20.0);
        let loudest = |s: &mut Synth, ms: u64| {
            let mut peak = 0.0f32;
            let t0 = Instant::now();
            while t0.elapsed() < Duration::from_millis(ms) {
                s.poll();
                let (l, r) = s.master_peak();
                peak = peak.max(l).max(r);
                std::thread::sleep(Duration::from_millis(5));
            }
            peak
        };
        assert!(loudest(&mut s, 800) > 0.01, "music is audible");
        assert!(s.channel_sound(0).is_some(), "channel 1 reports its sound");
        for id in (0..16).map(StripId::Channel).chain((0..KIT).map(StripId::Kit)) {
            let p = s.strip(id);
            s.set_strip(id, StripParams { mute: true, ..p });
        }
        assert!(s.mixer_touched());
        loudest(&mut s, 2000); // the reverb tail dies away
        assert!(loudest(&mut s, 400) < 0.01, "every strip muted");
        s.reset_channels();
        for g in 0..KIT {
            let p = s.strip(StripId::Kit(g));
            s.set_strip(StripId::Kit(g), StripParams { mute: false, ..p });
        }
        assert!(!s.mixer_touched());
    }

    #[test]
    fn channels_route_to_different_fonts_and_pin_sounds() {
        let midi = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shared/NCN/Song/Z/Z2608001.mid");
        let Some(sf2) = crate::library::find_soundfont().filter(|_| midi.is_file()) else {
            eprintln!("skipped: needs a .sf2 and the shared/NCN sample library");
            return;
        };
        // The same bank under a second name stands in for a second font.
        let dir = std::env::temp_dir().join(format!("karaoke-fonts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let second = dir.join("second.sf2");
        std::fs::copy(&sf2, &second).unwrap();

        let mut s = Synth::new();
        s.start_output(Some("\u{1}no such device\u{1}"));
        assert_eq!(s.add_font(sf2.clone()), Ok(0));
        assert_eq!(s.add_font(second), Ok(1));
        assert_eq!(s.add_font(sf2), Ok(0), "same file is not added twice");
        let t0 = Instant::now();
        while s.loading_soundfont() && t0.elapsed() < Duration::from_secs(60) {
            s.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(s.rack.len(), 2, "one font: melodic + drums");
        s.set_route(0, 1);
        assert_eq!(
            s.rack,
            [
                RackSlot::plain(0, !(1 | 1 << DRUM_CH), false),
                RackSlot::plain(0, 1 << DRUM_CH, true),
                RackSlot::plain(1, 1, false),
            ]
        );
        s.load_midi(&std::fs::read(&midi).unwrap(), "Z2608001").unwrap();
        s.play();
        s.seek(20.0);
        let font = s.channel_font(0).unwrap().clone();
        let organ = font.presets.iter().position(|p| p.program == 19 && p.bank == 0).unwrap();
        s.set_pin(0, Some(organ));
        let shared = s.shared.clone();
        let mut second_font = 0;
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(2) {
            s.poll();
            second_font = second_font.max(shared.slots[2].voices.load(Ordering::Relaxed));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(second_font > 0, "channel 1 plays on the second font");
        assert_eq!(s.channel_sound(0), Some(font.presets[organ].name.as_str()));
        s.reset_channels();
        assert_eq!(s.pin(0), None);

        // Removing the second font sends channel 1 back to the first.
        s.remove_font(1);
        assert_eq!(s.routing()[0], 0);
        assert_eq!(s.rack.len(), 2);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn stop_goes_back_to_the_start_without_ending_the_song() {
        let midi = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shared/NCN/Song/Z/Z2608001.mid");
        if !midi.is_file() {
            eprintln!("skipped: needs the shared/NCN sample library");
            return;
        }
        let mut s = Synth::new();
        s.start_output(Some("\u{1}no such device\u{1}"));
        s.load_midi(&std::fs::read(&midi).unwrap(), "Z2608001").unwrap();
        s.play();
        s.seek(30.0);
        let wait = |s: &mut Synth, ms: u64| {
            let mut events = Vec::new();
            let t0 = Instant::now();
            while t0.elapsed() < Duration::from_millis(ms) {
                events.extend(s.poll());
                std::thread::sleep(Duration::from_millis(5));
            }
            events
        };
        wait(&mut s, 300);
        assert!(s.time() > 29.0, "playing from 30 s: {}", s.time());
        s.stop();
        let events = wait(&mut s, 400);
        assert_eq!(s.state(), PlayState::Stopped);
        assert!(!events.iter().any(|e| matches!(e, SynthEvent::Ended)), "a stop is not the end of the song");
        assert!(s.time() < 0.5, "back at the start: {}", s.time());
        s.play();
        wait(&mut s, 400);
        assert_eq!(s.state(), PlayState::Playing);
        assert!(s.time() > 0.1 && s.time() < 2.0, "plays again from the start: {}", s.time());
    }

    #[test]
    fn kit_pieces_play_from_their_own_font() {
        let midi = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shared/NCN/Song/Z/Z2608001.mid");
        let Some(sf2) = crate::library::find_soundfont().filter(|_| midi.is_file()) else {
            eprintln!("skipped: needs a .sf2 and the shared/NCN sample library");
            return;
        };
        let dir = std::env::temp_dir().join(format!("karaoke-pieces-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let second = dir.join("second.sf2");
        std::fs::copy(&sf2, &second).unwrap();
        let mut s = Synth::new();
        s.start_output(Some("\u{1}no such device\u{1}"));
        s.add_font(sf2).unwrap();
        s.add_font(second).unwrap();
        let t0 = Instant::now();
        while s.loading_soundfont() && t0.elapsed() < Duration::from_secs(60) {
            s.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(s.rack.len(), 2, "melodic and drum slots of font 1");

        // The kick from font 2's Room kit (128:8).
        let room = InstrumentSound { font: 1, bank: 128, program: 8 };
        s.set_piece(0, Some(room));
        assert_eq!(s.rack.len(), 3);
        let piece = s.rack.iter().position(|r| r.lock.is_some()).unwrap();
        assert_eq!(s.rack[piece].font, 1);
        assert_eq!(s.rack[piece].keys, piece_keys(0));
        let main = s.slot_of(DRUM_CH).unwrap();
        assert_eq!(s.rack[main].keys & piece_keys(0), 0, "the main kit leaves the kick out");
        assert_eq!(s.piece_sound(0).as_deref(), Some("Room"));
        assert!(piece_keys(0) & (1u128 << 36) != 0 && piece_keys(1) & (1u128 << 38) != 0);
        // Kit strips reach both drum slots.
        assert_eq!(s.engine_strips(StripId::Kit(0)).len(), 2);

        // Kicks sound on the piece's slot.
        s.load_midi(&std::fs::read(&midi).unwrap(), "Z2608001").unwrap();
        s.play();
        s.seek(20.0);
        let mut most = 0;
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_millis(3000) {
            s.poll();
            most = most.max(s.shared.slots[piece].voices.load(Ordering::Relaxed));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(most > 0, "the separate kick slot plays");

        // Back to channel 10's kit; removing the font also clears it.
        s.set_piece(0, None);
        assert_eq!(s.rack.len(), 2);
        s.set_piece(1, Some(room));
        s.remove_font(1);
        assert!(s.pieces().iter().all(Option::is_none));
        assert_eq!(s.rack.len(), 2);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn guide_melody_mutes_and_outlives_the_mixer_reset() {
        let midi = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shared/NCN/Song/Z/Z2608001.mid");
        let Some(sf2) = crate::library::find_soundfont().filter(|_| midi.is_file()) else {
            eprintln!("skipped: needs a .sf2 and the shared/NCN sample library");
            return;
        };
        let mut s = Synth::new();
        s.start_output(Some("\u{1}no such device\u{1}"));
        s.add_font(sf2).unwrap();
        let t0 = Instant::now();
        while s.loading_soundfont() && t0.elapsed() < Duration::from_secs(60) {
            s.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        s.load_midi(&std::fs::read(&midi).unwrap(), "Z2608001").unwrap();
        s.play();
        s.seek(30.0);
        let loudest = |s: &mut Synth, ms: u64| {
            let mut peak = 0.0f32;
            let t0 = Instant::now();
            while t0.elapsed() < Duration::from_millis(ms) {
                s.poll();
                let (l, r) = s.master_peak();
                peak = peak.max(l).max(r);
                std::thread::sleep(Duration::from_millis(5));
            }
            peak
        };
        // Only the melody channel left on.
        let melody = StripId::Channel(MELODY_CH);
        let p = s.strip(melody);
        s.set_strip(melody, StripParams { solo: true, ..p });
        loudest(&mut s, 500);
        assert!(loudest(&mut s, 3000) > 0.005, "the guide melody plays on channel 9");
        s.set_melody_off(true);
        assert!(s.melody_off());
        loudest(&mut s, 2000); // the reverb tail dies away
        assert!(loudest(&mut s, 1500) < 0.002, "muted");
        // A new song resets the strips, not the setting.
        s.reset_channels();
        let p = s.strip(melody);
        s.set_strip(melody, StripParams { solo: true, ..p });
        loudest(&mut s, 1500);
        assert!(loudest(&mut s, 1500) < 0.002, "still muted after the reset");
        s.set_melody_off(false);
        assert!(loudest(&mut s, 3000) > 0.005, "back on");
    }

    #[test]
    fn instruments_drum_lock_and_kit_fader() {
        let midi = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shared/NCN/Song/Z/Z2608001.mid");
        let Some(sf2) = crate::library::find_soundfont().filter(|_| midi.is_file()) else {
            eprintln!("skipped: needs a .sf2 and the shared/NCN sample library");
            return;
        };
        let dir = std::env::temp_dir().join(format!("karaoke-gm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let second = dir.join("second.sf2");
        std::fs::copy(&sf2, &second).unwrap();
        let mut s = Synth::new();
        s.start_output(Some("\u{1}no such device\u{1}"));
        s.add_font(sf2).unwrap();
        s.add_font(second).unwrap();
        let t0 = Instant::now();
        while s.loading_soundfont() && t0.elapsed() < Duration::from_secs(60) {
            s.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        s.load_midi(&std::fs::read(&midi).unwrap(), "Z2608001").unwrap();
        s.play();
        s.seek(20.0);
        let shared = s.shared.clone();
        let wait = |s: &mut Synth, ms: u64| {
            let t0 = Instant::now();
            while t0.elapsed() < Duration::from_millis(ms) {
                s.poll();
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        wait(&mut s, 300);
        // Channel 1 plays GM program 27 (clean guitar) in this song.
        let home = s.slot_of(0).unwrap();
        let program = shared.slots[home].channels[0].load().program.expect("program change seen");
        let font = s.fonts[1].inst.clone().unwrap();
        let organ = font.presets.iter().find(|p| p.bank == 0 && p.program == 19).unwrap();
        s.set_instrument(program, Some(InstrumentSound { font: 1, bank: 0, program: 19 }));
        assert_eq!(s.rack.len(), 3, "main, drums, instrument slot");
        wait(&mut s, 600);
        assert_eq!(s.sounding_font(0), Some(1));
        assert_eq!(s.channel_sound(0), Some(organ.name.as_str()));
        let override_voices = shared.slots[2].voices.load(Ordering::Relaxed);
        assert!(override_voices > 0 || s.strip_peak(StripId::Channel(0)).0 > 0.0, "the override slot plays");
        // A pinned channel leaves the override.
        s.set_pin(0, Some(0));
        assert_eq!(s.rack[2].channels & 1, 0);
        s.set_pin(0, None);
        s.set_instrument(program, None);
        assert_eq!(s.rack.len(), 2);

        // Drum lock: kept through a new song and channel resets.
        let kit = s.channel_font(DRUM_CH).unwrap().presets.iter().position(|p| p.bank == 128 && p.program == 25);
        let kit = kit.or_else(|| s.channel_font(DRUM_CH).unwrap().presets.iter().position(|p| p.bank == 128)).unwrap();
        s.set_pin(DRUM_CH, Some(kit));
        let lock = s.drum_lock().unwrap();
        s.reset_channels();
        s.load_midi(&std::fs::read(&midi).unwrap(), "Z2608001").unwrap();
        assert_eq!(s.drum_lock(), Some(lock));
        assert_eq!(s.pin(DRUM_CH), Some(kit));
        s.play();
        wait(&mut s, 300);
        let drum_slot = s.slot_of(DRUM_CH).unwrap();
        assert_eq!(shared.slots[drum_slot].channels[DRUM_CH].load().preset, kit, "the song cannot change the kit");

        // Channel 10's strip drives every kit piece.
        let p = s.strip(StripId::Channel(DRUM_CH));
        s.set_strip(StripId::Channel(DRUM_CH), StripParams { mute: true, gain_db: -6.0, ..p });
        let kick = s.engine_params(StripId::Kit(0));
        assert!(kick.mute && (kick.gain_db + 6.0).abs() < 1e-4);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn midi_pan_and_sends_follow_the_song() {
        let midi = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shared/NCN/Song/Z/Z2608002.mid");
        let Some(sf2) = crate::library::find_soundfont().filter(|_| midi.is_file()) else {
            eprintln!("skipped: needs a .sf2 and the shared/NCN sample library");
            return;
        };
        let mut s = Synth::new();
        s.start_output(Some("\u{1}no such device\u{1}"));
        s.add_font(sf2).unwrap();
        let t0 = Instant::now();
        while s.loading_soundfont() && t0.elapsed() < Duration::from_secs(60) {
            s.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        s.load_midi(&std::fs::read(&midi).unwrap(), "Z2608002").unwrap();
        s.play();
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_millis(400) {
            s.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        // This song pans channel 4 right (CC10 = 85) and channel 6 left (40).
        assert!((s.midi_pan(3) - 21.0 / 63.0).abs() < 0.02, "channel 4: {}", s.midi_pan(3));
        assert!((s.midi_pan(5) + 24.0 / 63.0).abs() < 0.02, "channel 6: {}", s.midi_pan(5));
        assert_eq!(s.midi_pan(1), 0.0, "channel 2 stays centred");

        // Effect sends: channel 1 has CC91 = 63, channel 2 is left dry.
        assert!((s.midi_sends(0).0 - 63.0 / 127.0).abs() < 0.01, "channel 1 reverb: {:?}", s.midi_sends(0));
        assert_eq!(s.midi_sends(1).0, 0.0, "channel 2 has no reverb send");

        // A send offset puts the dry channel into the reverb.
        let reverb_in = |s: &mut Synth, ms: u64| {
            let mut peak = 0.0f32;
            let t0 = Instant::now();
            while t0.elapsed() < Duration::from_millis(ms) {
                s.poll();
                peak = peak.max(s.fx_peak().0);
                std::thread::sleep(Duration::from_millis(5));
            }
            peak
        };
        s.seek(20.0);
        let bass = StripId::Channel(1);
        let p = s.strip(bass);
        s.set_strip(bass, StripParams { solo: true, ..p });
        reverb_in(&mut s, 1000); // earlier sends fade from the meter
        assert!(reverb_in(&mut s, 400) < 1e-4, "a dry channel sends nothing");
        let p = s.strip(bass);
        s.set_strip(bass, StripParams { reverb_add: 0.5, ..p });
        assert!(reverb_in(&mut s, 1500) > 1e-3, "the offset sends it to the reverb");
    }
}
