//! MIDI file parser and player tests.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::engine::{Command, Engine, PlayState, Shared, Slot, SlotParams};
use crate::instrument::{self, Envelope, LoopMode, WavParams, WavSource};
use crate::smf;

fn vlq(mut v: u32) -> Vec<u8> {
    let mut out = vec![(v & 0x7F) as u8];
    v >>= 7;
    while v > 0 {
        out.insert(0, (v & 0x7F) as u8 | 0x80);
        v >>= 7;
    }
    out
}

fn track(events: &[(u32, &[u8])]) -> Vec<u8> {
    let mut body = Vec::new();
    for (delta, bytes) in events {
        body.extend(vlq(*delta));
        body.extend(*bytes);
    }
    body.extend([0x00, 0xFF, 0x2F, 0x00]);
    let mut v = b"MTrk".to_vec();
    v.extend((body.len() as u32).to_be_bytes());
    v.extend(body);
    v
}

fn smf_file(format: u16, ppq: u16, tracks: &[Vec<u8>]) -> Vec<u8> {
    let mut v = b"MThd".to_vec();
    v.extend(6u32.to_be_bytes());
    v.extend(format.to_be_bytes());
    v.extend((tracks.len() as u16).to_be_bytes());
    v.extend(ppq.to_be_bytes());
    for t in tracks {
        v.extend(t);
    }
    v
}

#[test]
fn smf_tempo_map_and_running_status() {
    // Track 0: 120 BPM, then 60 BPM from tick 960.
    let t0 = track(&[
        (0, &[0xFF, 0x03, 4, b'D', b'e', b'm', b'o']),
        (0, &[0xFF, 0x51, 3, 0x07, 0xA1, 0x20]),
        (960, &[0xFF, 0x51, 3, 0x0F, 0x42, 0x40]),
    ]);
    // Track 1: running status note-off (vel 0), program change on ch 2.
    let t1 = track(&[
        (0, &[0xC1, 5]),
        (0, &[0x90, 60, 100]),
        (480, &[60, 0]),
        (960, &[62, 90]),
        (480, &[0x80, 62, 0]),
    ]);
    let song = smf::parse(&smf_file(1, 480, &[t0, t1]), "x").unwrap();
    assert_eq!(song.name, "Demo");
    assert_eq!(song.tracks, 2);
    assert!((song.bpm - 120.0).abs() < 1e-6);
    let times: Vec<(f64, u8)> = song.events.iter().map(|e| (e.time, e.msg[0])).collect();
    let expect = [(0.0, 0xC1), (0.0, 0x90), (0.5, 0x90), (2.0, 0x90), (3.0, 0x80)];
    assert_eq!(times.len(), expect.len());
    for ((t, s), (et, es)) in times.iter().zip(expect) {
        assert!((t - et).abs() < 1e-9, "time {t} expected {et}");
        assert_eq!(*s, es);
    }
    assert_eq!(song.events[2].msg, [0x90, 60, 0]);
    assert!((song.duration - 3.0).abs() < 1e-9);
    assert_eq!(song.channels_used, 0b1);
}

#[test]
fn smf_rejects_garbage() {
    assert!(smf::parse(b"not midi at all", "x").is_err());
}

fn sine_inst() -> Arc<instrument::Instrument> {
    let frames = 48000;
    let data: Vec<f32> = (0..frames).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
    let source = Arc::new(WavSource {
        sample: Arc::new(instrument::SampleData::from_f32(1, data)),
        sample_rate: 48000.0,
        loop_points: Some((0, frames)),
        params: WavParams { root: 60, tune: 0.0, keytrack: true, loop_mode: LoopMode::Continuous, env: Envelope::default() },
    });
    let params = source.params;
    Arc::new(instrument::wav::build("sine".into(), std::path::Path::new("sine.wav"), source, params))
}

struct Rig {
    tx: crossbeam_channel::Sender<Command>,
    engine: Engine,
    shared: Arc<Shared>,
    _grx: crossbeam_channel::Receiver<crate::engine::Garbage>,
}

fn rig(params: SlotParams, inst: Arc<instrument::Instrument>) -> Rig {
    let (tx, rx) = crossbeam_channel::bounded(256);
    let (gtx, grx) = crossbeam_channel::bounded(256);
    let shared = Arc::new(Shared::new());
    let engine = Engine::new(48000.0, rx, gtx, shared.clone());
    tx.send(Command::AddSlot(Box::new(Slot::new(inst, 0, params)))).unwrap();
    Rig { tx, engine, shared, _grx: grx }
}

impl Rig {
    fn run(&mut self, frames: usize) -> Vec<f32> {
        let mut l = vec![0f32; frames];
        let mut r = vec![0f32; frames];
        self.engine.process(&mut l, &mut r);
        l
    }
    fn state(&self) -> PlayState {
        PlayState::from_u32(self.shared.player_state.load(Ordering::Relaxed))
    }
    fn time(&self) -> f64 {
        f64::from_bits(self.shared.player_time.load(Ordering::Relaxed))
    }
    fn voices(&self) -> u32 {
        self.shared.slots[0].voices.load(Ordering::Relaxed)
    }
}

fn one_note_song(on: u32, off: u32) -> Arc<smf::Song> {
    // 480 ppq at 120 BPM: 960 ticks per second.
    let t = track(&[(on, &[0x90, 60, 100]), (off - on, &[0x80, 60, 0])]);
    Arc::new(smf::parse(&smf_file(0, 480, &[t]), "one").unwrap())
}

#[test]
fn player_is_sample_accurate() {
    let mut r = rig(SlotParams::default(), sine_inst());
    // Note at 96 ticks = 0.1 s = sample 4800.
    r.tx.send(Command::LoadSong(one_note_song(96, 960))).unwrap();
    r.tx.send(Command::Play).unwrap();
    let out = r.run(9600);
    assert!(out[..4800].iter().all(|x| *x == 0.0), "silent before the note");
    assert!(out[4800..4900].iter().any(|x| x.abs() > 1e-3), "sound right at the note");
    assert_eq!(r.state(), PlayState::Playing);
    assert!((r.time() - 0.2).abs() < 1e-9);
}

#[test]
fn player_stops_at_end_and_loops_when_asked() {
    let mut r = rig(SlotParams::default(), sine_inst());
    r.tx.send(Command::LoadSong(one_note_song(0, 480))).unwrap();
    r.tx.send(Command::Play).unwrap();
    r.run(48000);
    assert_eq!(r.state(), PlayState::Stopped, "past the 0.5 s end");
    assert_eq!(r.time(), 0.0);

    r.tx.send(Command::SetLoop(true)).unwrap();
    r.tx.send(Command::Play).unwrap();
    r.run(36000); // 0.75 s: wrapped once
    assert_eq!(r.state(), PlayState::Playing);
    assert!((r.time() - 0.25).abs() < 0.02, "time {}", r.time());
    assert!(r.voices() > 0, "note retriggered after loop");
}

#[test]
fn player_channel_mute_and_pause() {
    let mut r = rig(SlotParams::default(), sine_inst());
    r.tx.send(Command::LoadSong(one_note_song(0, 960))).unwrap();
    r.tx.send(Command::ChannelMutes(1)).unwrap();
    r.tx.send(Command::Play).unwrap();
    r.run(4800);
    assert_eq!(r.voices(), 0, "channel 1 muted");
    assert_eq!(r.shared.channel_activity[0].load(Ordering::Relaxed), 0);

    r.tx.send(Command::ChannelMutes(0)).unwrap();
    r.tx.send(Command::Seek(0.0)).unwrap();
    r.run(4800);
    assert!(r.voices() > 0);
    let activity = crate::engine::load_peak(&r.shared.channel_activity[0]);
    assert!(activity > 0.3 && activity <= 100.0 / 127.0, "activity {activity}");

    r.tx.send(Command::Pause).unwrap();
    let t = { r.run(480); r.time() };
    r.run(48000);
    assert_eq!(r.state(), PlayState::Paused);
    assert_eq!(r.time(), t, "time frozen while paused");
    assert_eq!(r.voices(), 0, "notes released on pause");
}

#[test]
fn seek_chases_program_change() {
    let dir = std::env::temp_dir().join(format!("simpletui-chase-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bank.sf2");
    crate::tests::write_sf2_for(&path);
    let inst = Arc::new(instrument::load(&path).unwrap());
    // Single-channel slot on ch 1: program change selects the slot preset.
    let mut r = rig(SlotParams { channels: 1, ..SlotParams::default() }, inst);
    let t = track(&[(0, &[0xB0, 0, 128]), (0, &[0xC0, 0]), (960, &[0x90, 38, 100]), (480, &[0x80, 38, 0])]);
    let song = Arc::new(smf::parse(&smf_file(0, 480, &[t]), "chase").unwrap());
    r.tx.send(Command::LoadSong(song)).unwrap();
    r.tx.send(Command::Seek(0.5)).unwrap();
    r.run(64);
    assert_eq!(r.shared.slots[0].preset.load(Ordering::Relaxed), 1, "bank 128 kit chased");
}

#[test]
fn omni_sf2_defaults_channel_10_to_drums() {
    let dir = std::env::temp_dir().join(format!("simpletui-gm-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bank.sf2");
    crate::tests::write_sf2_for(&path);
    let inst = Arc::new(instrument::load(&path).unwrap());
    let mut r = rig(SlotParams::default(), inst);
    // Key 72 exists only in the Lead preset (kit covers 36-40).
    r.tx.send(Command::Midi([0x99, 72, 100])).unwrap();
    r.run(64);
    assert_eq!(r.voices(), 0, "ch10 uses the kit, which has no key 72");
    r.tx.send(Command::Midi([0x99, 38, 100])).unwrap();
    r.tx.send(Command::Midi([0x90, 72, 100])).unwrap();
    r.run(64);
    assert_eq!(r.voices(), 2, "kit on ch10 plus lead on ch1");
}

#[test]
fn ui_renders_player() {
    use ratatui::{Terminal, backend::TestBackend};
    let dir = std::env::temp_dir().join(format!("simpletui-uip-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t = track(&[(0, &[0x90, 60, 100]), (0, &[0x99, 38, 100]), (1920, &[0x80, 60, 0])]);
    let path = dir.join("tune.mid");
    std::fs::write(&path, smf_file(0, 480, &[t])).unwrap();
    let mut app = crate::app::App::new(true, None);
    assert!(app.load_song(path));
    app.player.time = 0.7;
    app.player.levels[0] = 0.8;
    app.player.mutes = 1 << 9;
    let mut term = Terminal::new(TestBackend::new(130, 40)).unwrap();
    term.draw(|f| crate::ui::draw(f, &app)).unwrap();
    let buf = term.backend().buffer().clone();
    let text: String = (0..buf.area.height)
        .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>() + "
")
        .collect();
    if std::env::var("SHOW_UI").is_ok() {
        println!("{text}");
    }
    assert!(text.contains("MIDI Player"));
    assert!(text.contains("tune"));
    assert!(text.contains("0:00.7 / 0:02.0"));
    assert!(text.contains("10×"));
}
