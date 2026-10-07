//! Loader and engine tests using synthetic WAV / SFZ / SF2 fixtures.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::engine::{Command, Engine, Shared, Slot, SlotParams};
use crate::instrument::{self, Kind, LoopMode, parse_note};

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("simpletui-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = id.to_vec();
    v.extend((body.len() as u32).to_le_bytes());
    v.extend(body);
    if body.len() % 2 == 1 {
        v.push(0);
    }
    v
}

fn list(kind: &[u8; 4], parts: &[Vec<u8>]) -> Vec<u8> {
    let mut body = kind.to_vec();
    for p in parts {
        body.extend(p);
    }
    chunk(b"LIST", &body)
}

fn sine_i16(frames: usize, period: usize) -> Vec<i16> {
    (0..frames)
        .map(|i| ((i as f32 / period as f32 * std::f32::consts::TAU).sin() * 16000.0) as i16)
        .collect()
}

/// 16-bit mono WAV, optional smpl chunk with root key and loop.
fn write_wav(path: &Path, rate: u32, data: &[i16], smpl: Option<(u32, u32, u32)>) {
    let mut fmt = Vec::new();
    fmt.extend(1u16.to_le_bytes());
    fmt.extend(1u16.to_le_bytes());
    fmt.extend(rate.to_le_bytes());
    fmt.extend((rate * 2).to_le_bytes());
    fmt.extend(2u16.to_le_bytes());
    fmt.extend(16u16.to_le_bytes());
    let pcm: Vec<u8> = data.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut body = b"WAVE".to_vec();
    body.extend(chunk(b"fmt ", &fmt));
    if let Some((root, ls, le)) = smpl {
        let mut s = vec![0u8; 36 + 24];
        s[12..16].copy_from_slice(&root.to_le_bytes());
        s[28..32].copy_from_slice(&1u32.to_le_bytes());
        s[36 + 8..36 + 12].copy_from_slice(&ls.to_le_bytes());
        s[36 + 12..36 + 16].copy_from_slice(&le.to_le_bytes());
        body.extend(chunk(b"smpl", &s));
    }
    body.extend(chunk(b"data", &pcm));
    std::fs::write(path, chunk(b"RIFF", &body)).unwrap();
}

fn name20(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.resize(20, 0);
    v
}

fn gen_rec(op: u16, amount: [u8; 2]) -> Vec<u8> {
    let mut v = op.to_le_bytes().to_vec();
    v.extend(amount);
    v
}

pub fn write_sf2_for(path: &Path) {
    write_sf2(path);
}

/// Two presets: 000:000 "Lead" (looped, keys 0-127) and 128:000 "Kit" (keys 36-40, no loop).
fn write_sf2(path: &Path) {
    let samples = sine_i16(4800, 100);
    let mut smpl: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    smpl.extend(vec![0u8; 92]); // 46 zero frames of padding, as the spec requires

    let mut phdr = Vec::new();
    for (name, prog, bank, bag) in [("Lead", 0u16, 0u16, 0u16), ("Kit", 0, 128, 1), ("EOP", 0, 0, 2)] {
        phdr.extend(name20(name));
        phdr.extend(prog.to_le_bytes());
        phdr.extend(bank.to_le_bytes());
        phdr.extend(bag.to_le_bytes());
        phdr.extend([0u8; 12]);
    }
    let mut pbag = Vec::new();
    for (g, m) in [(0u16, 0u16), (1, 0), (2, 0)] {
        pbag.extend(g.to_le_bytes());
        pbag.extend(m.to_le_bytes());
    }
    let mut pgen = Vec::new();
    pgen.extend(gen_rec(41, 0u16.to_le_bytes()));
    pgen.extend(gen_rec(41, 1u16.to_le_bytes()));
    pgen.extend(gen_rec(0, [0, 0]));

    let mut inst = Vec::new();
    for (name, bag) in [("LeadInst", 0u16), ("KitInst", 1), ("EOI", 2)] {
        inst.extend(name20(name));
        inst.extend(bag.to_le_bytes());
    }
    let mut ibag = Vec::new();
    for (g, m) in [(0u16, 0u16), (3, 0), (6, 0)] {
        ibag.extend(g.to_le_bytes());
        ibag.extend(m.to_le_bytes());
    }
    let mut igen = Vec::new();
    igen.extend(gen_rec(43, [0, 127]));
    igen.extend(gen_rec(54, 1i16.to_le_bytes()));
    igen.extend(gen_rec(53, 0u16.to_le_bytes()));
    igen.extend(gen_rec(43, [36, 40]));
    igen.extend(gen_rec(57, 1i16.to_le_bytes()));
    igen.extend(gen_rec(53, 0u16.to_le_bytes()));
    igen.extend(gen_rec(0, [0, 0]));

    let mut shdr = Vec::new();
    shdr.extend(name20("sine"));
    for v in [0u32, 4800, 1000, 4000, 48000] {
        shdr.extend(v.to_le_bytes());
    }
    shdr.extend([60u8, 0]);
    shdr.extend(0u16.to_le_bytes());
    shdr.extend(1u16.to_le_bytes());
    shdr.extend(name20("EOS"));
    shdr.extend([0u8; 26]);

    let empty_mod = vec![0u8; 10];
    let mut body = b"sfbk".to_vec();
    body.extend(list(b"INFO", &[chunk(b"ifil", &[2, 0, 1, 0]), chunk(b"INAM", b"Test Bank\0")]));
    body.extend(list(b"sdta", &[chunk(b"smpl", &smpl)]));
    body.extend(list(
        b"pdta",
        &[
            chunk(b"phdr", &phdr),
            chunk(b"pbag", &pbag),
            chunk(b"pmod", &empty_mod),
            chunk(b"pgen", &pgen),
            chunk(b"inst", &inst),
            chunk(b"ibag", &ibag),
            chunk(b"imod", &empty_mod),
            chunk(b"igen", &igen),
            chunk(b"shdr", &shdr),
        ],
    ));
    std::fs::write(path, chunk(b"RIFF", &body)).unwrap();
}

fn render(inst: instrument::Instrument, notes: &[(u8, u8)], frames: usize) -> (f32, f32) {
    let (tx, rx) = crossbeam_channel::bounded(64);
    let (gtx, _grx) = crossbeam_channel::bounded(64);
    let shared = Arc::new(Shared::new());
    let mut e = Engine::new(48000.0, rx, gtx, shared);
    tx.send(Command::AddSlot(Box::new(Slot::new(Arc::new(inst), 0, SlotParams::default())))).unwrap();
    for &(key, vel) in notes {
        tx.send(Command::Midi([0x90, key, vel])).unwrap();
    }
    let mut l = vec![0f32; frames];
    let mut r = vec![0f32; frames];
    e.process(&mut l, &mut r);
    let on_peak = l.iter().chain(&r).fold(0f32, |m, x| m.max(x.abs()));
    for &(key, _) in notes {
        tx.send(Command::Midi([0x80, key, 0])).unwrap();
    }
    for _ in 0..20 {
        e.process(&mut l, &mut r);
    }
    let tail = l.iter().chain(&r).fold(0f32, |m, x| m.max(x.abs()));
    (on_peak, tail)
}

#[test]
fn note_names() {
    assert_eq!(parse_note("c4"), Some(60));
    assert_eq!(parse_note("C#4"), Some(61));
    assert_eq!(parse_note("eb2"), Some(39));
    assert_eq!(parse_note("c-1"), Some(0));
    assert_eq!(parse_note("72"), Some(72));
    assert_eq!(instrument::note_name(60), "C4");
}

#[test]
fn wav_with_smpl_loop() {
    let d = temp_dir("wav");
    let p = d.join("tone.wav");
    write_wav(&p, 44100, &sine_i16(44100, 100), Some((69, 100, 1099)));
    let inst = instrument::load(&p).unwrap();
    assert_eq!(inst.kind, Kind::Wav);
    let z = &inst.presets[0].zones[0];
    assert_eq!(z.root, 69.0);
    assert_eq!(z.loop_mode, LoopMode::Continuous);
    assert_eq!((z.loop_start, z.loop_end), (100, 1100));
    assert_eq!(z.sample.frames, 44100);

    let (on, tail) = render(inst, &[(69, 127)], 4800);
    assert!(on > 0.2, "note should sound, peak {on}");
    assert!(tail < 1e-3, "release should decay, tail {tail}");
}

#[test]
fn sfz_regions_and_opcodes() {
    let d = temp_dir("sfz");
    std::fs::create_dir_all(d.join("samples")).unwrap();
    write_wav(&d.join("samples/low tone.wav"), 48000, &sine_i16(9600, 200), None);
    write_wav(&d.join("samples/high.wav"), 48000, &sine_i16(9600, 50), None);
    let sfz = r#"
// comment
#define $VOL -3
<control> default_path=samples/
<global> ampeg_release=0.05 volume=$VOL
<group> lovel=1 hivel=127
<region> sample=low tone.wav lokey=c2 hikey=b3 pitch_keycenter=c3
<region> sample=high.wav lokey=c4 hikey=c6 pitch_keycenter=60 tune=10 /* inline */ pan=-50
<region> sample=*sine key=100 loop_mode=loop_continuous
<region> sample=missing.wav key=101
"#;
    let path = d.join("test.sfz");
    std::fs::write(&path, sfz).unwrap();
    let inst = instrument::load(&path).unwrap();
    let zones = &inst.presets[0].zones;
    assert_eq!(zones.len(), 3, "missing sample region is skipped");
    assert!(!inst.warnings.is_empty());
    assert_eq!((zones[0].lokey, zones[0].hikey, zones[0].root), (36, 59, 48.0));
    assert_eq!((zones[1].lokey, zones[1].hikey), (60, 84));
    assert_eq!(zones[1].tune, 10.0);
    assert!((zones[1].pan + 0.5).abs() < 1e-6);
    assert!((zones[0].env.release - 0.05).abs() < 1e-6);
    assert!((zones[0].gain - instrument::db_to_gain(-3.0)).abs() < 1e-4);
    assert_eq!(zones[2].loop_mode, LoopMode::Continuous);

    let (on, tail) = render(inst, &[(48, 100), (72, 100)], 4800);
    assert!(on > 0.1, "peak {on}");
    assert!(tail < 1e-3, "tail {tail}");
}

#[test]
fn sf2_presets_and_zones() {
    let d = temp_dir("sf2");
    let p = d.join("test.sf2");
    write_sf2(&p);
    let inst = instrument::load(&p).unwrap();
    assert_eq!(inst.kind, Kind::Sf2);
    assert_eq!(inst.name, "Test Bank");
    assert_eq!(inst.presets.len(), 2);
    let lead = &inst.presets[0];
    assert_eq!((lead.bank, lead.program, lead.name.as_str()), (0, 0, "Lead"));
    let z = &lead.zones[0];
    assert_eq!(z.loop_mode, LoopMode::Continuous);
    assert_eq!((z.start, z.end, z.loop_start, z.loop_end), (0, 4800, 1000, 4000));
    assert_eq!(z.root, 60.0);
    let kit = &inst.presets[1];
    assert_eq!(kit.bank, 128);
    assert_eq!((kit.zones[0].lokey, kit.zones[0].hikey), (36, 40));
    assert_eq!(kit.zones[0].group, 1);
    assert_eq!(inst.find_preset(128, 0), Some(1));

    let (on, tail) = render(inst, &[(60, 110)], 4800);
    assert!(on > 0.2, "peak {on}");
    assert!(tail < 1e-3, "tail {tail}");
}

#[test]
fn sf2_program_change_and_channel_filter() {
    let d = temp_dir("sf2pc");
    let p = d.join("test.sf2");
    write_sf2(&p);
    let inst = Arc::new(instrument::load(&p).unwrap());
    let (tx, rx) = crossbeam_channel::bounded(64);
    let (gtx, _grx) = crossbeam_channel::bounded(64);
    let shared = Arc::new(Shared::new());
    let mut e = Engine::new(48000.0, rx, gtx, shared.clone());
    let params = SlotParams { channels: 1 << 9, ..SlotParams::default() };
    tx.send(Command::AddSlot(Box::new(Slot::new(inst, 0, params)))).unwrap();
    // Channel 10 program change picks the bank 128 kit.
    tx.send(Command::Midi([0xC9, 0, 0])).unwrap();
    // Note on channel 1 must be ignored by a channel-10 slot.
    tx.send(Command::Midi([0x90, 60, 100])).unwrap();
    let mut l = vec![0f32; 512];
    let mut r = vec![0f32; 512];
    e.process(&mut l, &mut r);
    assert_eq!(shared.slots[0].preset.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(shared.slots[0].voices.load(std::sync::atomic::Ordering::Relaxed), 0);
    tx.send(Command::Midi([0x99, 38, 100])).unwrap();
    e.process(&mut l, &mut r);
    assert_eq!(shared.slots[0].voices.load(std::sync::atomic::Ordering::Relaxed), 1);
}

#[test]
fn sustain_pedal_holds_notes() {
    let d = temp_dir("sus");
    let p = d.join("tone.wav");
    write_wav(&p, 48000, &sine_i16(48000, 100), Some((60, 0, 47999)));
    let inst = Arc::new(instrument::load(&p).unwrap());
    let (tx, rx) = crossbeam_channel::bounded(64);
    let (gtx, _grx) = crossbeam_channel::bounded(64);
    let shared = Arc::new(Shared::new());
    let mut e = Engine::new(48000.0, rx, gtx, shared.clone());
    tx.send(Command::AddSlot(Box::new(Slot::new(inst, 0, SlotParams::default())))).unwrap();
    // Dry signal only: the reverb tail would outlast the release check.
    let dry = crate::engine::mixer::FxParams { reverb_return: 0.0, chorus_return: 0.0, ..Default::default() };
    tx.send(Command::SetFx(dry)).unwrap();
    for m in [[0xB0, 64, 127], [0x90, 60, 100], [0x80, 60, 0]] {
        tx.send(Command::Midi(m)).unwrap();
    }
    let mut l = vec![0f32; 48000];
    let mut r = vec![0f32; 48000];
    e.process(&mut l, &mut r);
    assert!(l[47000].abs() > 0.0 || l[46990..47010].iter().any(|x| x.abs() > 0.01), "held by pedal");
    tx.send(Command::Midi([0xB0, 64, 0])).unwrap();
    e.process(&mut l, &mut r);
    assert!(l[40000..].iter().all(|x| x.abs() < 1e-3), "released after pedal up");
}

#[test]
fn ui_renders_rack() {
    use ratatui::{Terminal, backend::TestBackend};
    let d = temp_dir("ui");
    write_sf2(&d.join("bank.sf2"));
    write_wav(&d.join("kick.wav"), 48000, &sine_i16(4800, 100), None);
    let mut app = crate::app::App::new(true, None);
    app.load(d.join("bank.sf2"), crate::app::LoadTarget::New);
    app.load(d.join("kick.wav"), crate::app::LoadTarget::New);
    for _ in 0..200 {
        app.tick();
        if app.slots.len() == 2 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(app.slots.len(), 2);
    let mut term = Terminal::new(TestBackend::new(130, 36)).unwrap();
    term.draw(|f| crate::ui::draw(f, &app)).unwrap();
    let buf = term.backend().buffer().clone();
    let text: String = (0..buf.area.height)
        .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>() + "\n")
        .collect();
    if std::env::var("SHOW_UI").is_ok() {
        println!("{text}");
    }
    assert!(text.contains("Test Bank"));
    assert!(text.contains("kick"));
    assert!(text.contains("SF2"));
}

/// Opens the real default output device (WASAPI / ALSA) and checks the
/// callback runs. Silent: no notes are played. `cargo test -- --ignored`
#[test]
#[ignore]
fn audio_device_runs() {
    let (_tx, rx) = crossbeam_channel::bounded(16);
    let (gtx, _grx) = crossbeam_channel::bounded(16);
    let shared = Arc::new(Shared::new());
    let out = crate::audio::start(None, None, rx, gtx, shared.clone()).expect("audio start");
    println!("{} / {} / {} Hz / {} / {}ch / {}", out.host, out.device, out.sample_rate, out.format, out.channels, out.buffer);
    std::thread::sleep(std::time::Duration::from_millis(500));
    let cpu = f32::from_bits(shared.cpu.load(std::sync::atomic::Ordering::Relaxed));
    assert!(cpu > 0.0, "audio callback never ran");
    assert_eq!(shared.errors.load(std::sync::atomic::Ordering::Relaxed), 0);
}
