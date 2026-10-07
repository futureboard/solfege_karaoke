//! The backing-track player: a `solfege_synth` engine with one SoundFont
//! split over two slots, so key changes move every part except the drums.
//!
//! The engine normally runs inside the audio callback. Without a usable
//! output device it runs on a paced thread instead, so lyrics still follow
//! the song (silently) and the rest of the app behaves the same.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender};
use solfege_synth::audio::{self, AudioOut};
use solfege_synth::engine::mixer::{DRUM_STRIP_BASE, FxParams, GM_GROUP_NAMES, StripParams};
use solfege_synth::engine::{Command, Engine, Garbage, NO_DRUMS, PlayState, Shared, Slot, SlotParams, load_peak};
use solfege_synth::instrument::{self, Instrument, db_to_gain};
use solfege_synth::smf;

/// Melodic parts (transposed) and channel 10 drums (never transposed).
const MELODIC: usize = 0;
const DRUMS: usize = 1;

pub const KEY_RANGE: i32 = 12;
/// Drum kit pieces with their own mixer strip (GM note groups).
pub const KIT: usize = 6;

pub fn kit_name(group: usize) -> &'static str {
    GM_GROUP_NAMES[group]
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

impl StripId {
    fn engine(self) -> (usize, usize) {
        match self {
            StripId::Channel(ch) => (MELODIC, ch),
            StripId::Kit(g) => (DRUMS, DRUM_STRIP_BASE + g),
        }
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
    font: Option<Arc<Instrument>>,
    pub font_path: Option<PathBuf>,
    font_job: Option<Receiver<(PathBuf, Result<Instrument>)>>,
    song: Option<Arc<smf::Song>>,
    state: PlayState,
    /// The engine has reported `Playing` since the last play command, so a
    /// later `Stopped` means the song ran out.
    seen_playing: bool,
    key: i32,
    speed: f64,
    volume: f32,
    mixer: Mixer,
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
            font: None,
            font_path: None,
            font_job: None,
            song: None,
            state: PlayState::Empty,
            seen_playing: false,
            key: 0,
            speed: 1.0,
            volume: 0.8,
            mixer: Mixer::default(),
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
    fn replay(&self) {
        self.send(Command::MasterGain(db_to_gain(volume_db(self.volume))));
        if let Some(inst) = &self.font {
            self.send(Command::AddSlot(Box::new(Slot::new(inst.clone(), 0, self.melodic_params()))));
            self.send(Command::AddSlot(Box::new(Slot::new(inst.clone(), 0, drum_params()))));
            self.send_strips();
        }
        self.send(Command::SetFx(self.mixer.fx));
        if let Some(song) = &self.song {
            self.send(Command::LoadSong(song.clone()));
            self.send(Command::SetSpeed(self.speed));
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

    fn send_strip(&self, id: StripId) {
        let (slot, strip) = id.engine();
        self.send(Command::SetStrip { slot, strip, params: self.strip(id) });
    }

    // --------------------------------------------------------- soundfont

    /// Load an SF2 (or SFZ) on a background thread; `poll` reports the result.
    pub fn load_soundfont(&mut self, path: PathBuf) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let r = instrument::load(&path);
            let _ = tx.send((path, r));
        });
        self.font_job = Some(rx);
    }

    pub fn loading_soundfont(&self) -> bool {
        self.font_job.is_some()
    }

    pub fn soundfont_name(&self) -> Option<&str> {
        self.font.as_ref().map(|f| f.name.as_str())
    }

    fn install_font(&mut self, inst: Arc<Instrument>) {
        if self.font.is_none() {
            self.send(Command::AddSlot(Box::new(Slot::new(inst.clone(), 0, self.melodic_params()))));
            self.send(Command::AddSlot(Box::new(Slot::new(inst.clone(), 0, drum_params()))));
            self.send_strips();
        } else {
            for slot in [MELODIC, DRUMS] {
                self.send(Command::SetInstrument { slot, inst: inst.clone(), preset: 0, keep_voices: false });
            }
        }
        self.font = Some(inst);
        // Program changes already played must reach the new instrument.
        if self.state == PlayState::Playing || self.state == PlayState::Paused {
            self.send(Command::Seek(self.time()));
        }
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
        if self.font.is_some() {
            self.send(Command::SetParams { slot: MELODIC, params: self.melodic_params() });
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
    }

    pub fn set_fx(&mut self, fx: FxParams) {
        self.mixer.fx = fx.clamped();
        self.send(Command::SetFx(self.mixer.fx));
    }

    /// Channel strips back to neutral (each song has its own parts); the
    /// drum kit and effects stay as set.
    pub fn reset_channels(&mut self) {
        self.mixer.channels = [StripParams::default(); 16];
        for ch in 0..16 {
            self.send_strip(StripId::Channel(ch));
        }
    }

    /// Any channel or kit strip muted or soloed.
    pub fn mixer_touched(&self) -> bool {
        let m = &self.mixer;
        m.channels.iter().chain(&m.kit).any(|s| s.mute || s.solo)
    }

    /// Peak level (left, right) of a strip, 0..1+.
    pub fn strip_peak(&self, id: StripId) -> (f32, f32) {
        let (slot, strip) = id.engine();
        let m = &self.shared.slots[slot];
        (load_peak(&m.strip_l[strip]), load_peak(&m.strip_r[strip]))
    }

    pub fn master_peak(&self) -> (f32, f32) {
        (load_peak(&self.shared.master_l), load_peak(&self.shared.master_r))
    }

    /// Sound (preset) a channel is playing, from the SoundFont.
    pub fn channel_sound(&self, ch: usize) -> Option<&str> {
        let font = self.font.as_ref()?;
        let info = self.shared.slots[MELODIC].channels[ch].load();
        info.program?;
        font.presets.get(info.preset).map(|p| p.name.as_str())
    }

    fn melodic_params(&self) -> SlotParams {
        SlotParams { channels: NO_DRUMS, transpose: self.key, ..SlotParams::default() }
    }

    // -------------------------------------------------------------- poll

    /// Once per frame: free engine garbage, finish background loads and
    /// notice the end of the song.
    pub fn poll(&mut self) -> Vec<SynthEvent> {
        while self.garbage_rx.try_recv().is_ok() {}
        let mut events = Vec::new();
        if let Some(job) = &self.font_job
            && let Ok((path, result)) = job.try_recv()
        {
            self.font_job = None;
            match result {
                Ok(inst) => {
                    events.push(SynthEvent::SoundFontLoaded { name: inst.name.clone(), presets: inst.presets.len() });
                    self.font_path = Some(path);
                    self.install_font(Arc::new(inst));
                }
                Err(e) => events.push(SynthEvent::SoundFontFailed(format!("{}: {e:#}", path.display()))),
            }
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

fn drum_params() -> SlotParams {
    SlotParams { channels: 1 << 9, ..SlotParams::default() }
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
        s.load_soundfont(sf2);
        let t0 = Instant::now();
        while s.loading_soundfont() && t0.elapsed() < Duration::from_secs(60) {
            for e in s.poll() {
                if let SynthEvent::SoundFontFailed(e) = e {
                    panic!("{e}");
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(s.soundfont_name().is_some());
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
            melodic = melodic.max(voices(MELODIC));
            drums = drums.max(voices(DRUMS));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(melodic > 0 && drums > 0, "melodic {melodic}, drums {drums}");
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
        s.load_soundfont(sf2);
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
}
