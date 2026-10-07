//! SoundFont multitimbral (Omni) and Bank Select / Program Change tests.

use std::sync::Arc;

use crate::engine::{Command, Engine, Shared, Slot, SlotParams, resolve_preset};
use crate::instrument::{Instrument, Kind, Preset, SampleData, Zone};

/// Presets (bank:program): 000:000 Piano, 000:001 Bright, 008:001 Bright Var (GS),
/// 128:000 Standard Kit, 128:025 TR-808.
pub fn gm_bank() -> Arc<Instrument> {
    let sample = Arc::new(SampleData::from_f32(1, (0..4800).map(|i| (i as f32 * 0.1).sin()).collect()));
    let preset = |bank: u16, program: u8, name: &str| {
        let mut z = Zone::new(sample.clone(), 48000.0);
        z.loop_mode = crate::instrument::LoopMode::Continuous;
        z.sanitize();
        Preset { name: name.into(), bank, program, zones: vec![z] }
    };
    Arc::new(Instrument {
        name: "GM".into(),
        kind: Kind::Sf2,
        path: "gm.sf2".into(),
        presets: vec![
            preset(0, 0, "Piano"),
            preset(0, 1, "Bright"),
            preset(8, 1, "Bright Var"),
            preset(128, 0, "Standard Kit"),
            preset(128, 25, "TR-808"),
        ],
        warnings: Vec::new(),
        sample_bytes: 0,
        wav: None,
    })
}

#[test]
fn bank_select_conventions() {
    let gm = gm_bank();
    // GM: plain program change.
    assert_eq!(resolve_preset(&gm, false, 0, 0, 1), Some((1, true)));
    // GS: bank = MSB.
    assert_eq!(resolve_preset(&gm, false, 8, 0, 1), Some((2, true)));
    // XG-style: variation in LSB with MSB 0.
    assert_eq!(resolve_preset(&gm, false, 0, 8, 1), Some((2, true)));
    // Missing variation falls back to bank 0, same program.
    assert_eq!(resolve_preset(&gm, false, 5, 0, 1), Some((1, false)));
    // Channel 10 is drums whatever the bank says.
    assert_eq!(resolve_preset(&gm, true, 0, 0, 25), Some((4, true)));
    // XG drum MSB 127 on a normal channel.
    assert_eq!(resolve_preset(&gm, false, 127, 0, 0), Some((3, true)));
    // Unknown kit falls back to the standard kit.
    assert_eq!(resolve_preset(&gm, true, 0, 0, 40), Some((3, false)));
    // Program that exists nowhere.
    assert_eq!(resolve_preset(&gm, false, 0, 0, 99), None);
}

fn rig() -> (crossbeam_channel::Sender<Command>, Engine, Arc<Shared>, crossbeam_channel::Receiver<crate::engine::Garbage>) {
    let (tx, rx) = crossbeam_channel::bounded(256);
    let (gtx, grx) = crossbeam_channel::bounded(256);
    let shared = Arc::new(Shared::new());
    let engine = Engine::new(48000.0, rx, gtx, shared.clone());
    tx.send(Command::AddSlot(Box::new(Slot::new(gm_bank(), 0, SlotParams::default())))).unwrap();
    (tx, engine, shared, grx)
}

fn run(e: &mut Engine) {
    let mut l = vec![0f32; 256];
    let mut r = vec![0f32; 256];
    e.process(&mut l, &mut r);
}

#[test]
fn one_soundfont_serves_all_16_channels() {
    let (tx, mut e, shared, _g) = rig();
    let msgs: &[[u8; 3]] = &[
        [0xC0, 1, 0],                     // ch1  -> 000:001
        [0xB1, 0, 8], [0xC1, 1, 0],       // ch2  -> 008:001 (GS)
        [0xB2, 0, 5], [0xC2, 1, 0],       // ch3  -> fallback 000:001
        [0xB3, 0, 127], [0xC3, 0, 0],     // ch4  -> XG drums 128:000
        [0xC9, 25, 0],                    // ch10 -> 128:025
        [0xB4, 7, 90], [0xB4, 10, 20], [0xB4, 64, 127], [0xE4, 0, 0x50],
    ];
    for m in msgs {
        tx.send(Command::Midi(*m)).unwrap();
    }
    for ch in 0..16u8 {
        tx.send(Command::Midi([0x90 | ch, 60, 100])).unwrap();
    }
    run(&mut e);
    let info = |c: usize| shared.slots[0].channels[c].load();

    assert_eq!((info(0).preset, info(0).program, info(0).explicit), (1, Some(1), true));
    assert_eq!((info(1).preset, info(1).bank_msb), (2, 8));
    assert_eq!((info(2).preset, info(2).fallback), (1, true));
    assert_eq!(info(3).preset, 3);
    assert_eq!(info(9).preset, 4);
    // Untouched channel follows the slot preset; ch10 default is the kit.
    assert_eq!((info(5).preset, info(5).explicit, info(5).program), (0, false, None));
    let ch5 = info(4);
    assert_eq!((ch5.volume, ch5.pan, ch5.sustain), (90, 20, true));
    assert_eq!(ch5.bend, (0x50 << 7) - 8192);
    // Every channel played its own voice on the single slot.
    for c in 0..16 {
        assert_eq!(info(c).voices, 1, "channel {}", c + 1);
    }
    assert_eq!(shared.slots[0].voices.load(std::sync::atomic::Ordering::Relaxed), 16);
}

#[test]
fn manual_channel_assignment_and_reset() {
    let (tx, mut e, shared, _g) = rig();
    tx.send(Command::SetChannelPreset { slot: 0, ch: 2, preset: Some(4) }).unwrap();
    run(&mut e);
    let i = shared.slots[0].channels[2].load();
    assert_eq!((i.preset, i.explicit, i.locked), (4, true, true));

    // A pin survives program changes and GM/GS/XG resets.
    tx.send(Command::Midi([0xC2, 1, 0])).unwrap();
    tx.send(Command::Midi([0xF0, crate::smf::SYS_RESET, 0])).unwrap();
    run(&mut e);
    let i = shared.slots[0].channels[2].load();
    assert_eq!((i.preset, i.locked), (4, true));

    tx.send(Command::SetChannelPreset { slot: 0, ch: 2, preset: None }).unwrap();
    run(&mut e);
    let i = shared.slots[0].channels[2].load();
    assert_eq!((i.preset, i.explicit, i.locked), (0, false, false));
}

#[test]
fn single_channel_slot_ignores_other_channels() {
    let (tx, rx) = crossbeam_channel::bounded(64);
    let (gtx, _g) = crossbeam_channel::bounded(64);
    let shared = Arc::new(Shared::new());
    let mut e = Engine::new(48000.0, rx, gtx, shared.clone());
    let params = SlotParams { channels: 1 << 2, ..SlotParams::default() };
    tx.send(Command::AddSlot(Box::new(Slot::new(gm_bank(), 0, params)))).unwrap();
    tx.send(Command::Midi([0xC0, 1, 0])).unwrap(); // ch1: ignored
    tx.send(Command::Midi([0xC2, 1, 0])).unwrap(); // ch3: selects slot preset
    tx.send(Command::Midi([0x90, 60, 100])).unwrap();
    run(&mut e);
    assert_eq!(shared.slots[0].preset.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(shared.slots[0].voices.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[test]
fn ui_renders_channel_map() {
    use ratatui::{Terminal, backend::TestBackend};
    let dir = std::env::temp_dir().join(format!("simpletui-uic-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bank.sf2");
    crate::tests::write_sf2_for(&path);
    let mut app = crate::app::App::new(true, None);
    app.load(path, crate::app::LoadTarget::New);
    for _ in 0..200 {
        app.tick();
        if !app.slots.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    app.focus = crate::app::Focus::Channels;
    let mut term = Terminal::new(TestBackend::new(130, 44)).unwrap();
    term.draw(|f| crate::ui::draw(f, &app)).unwrap();
    let buf = term.backend().buffer().clone();
    let text: String = (0..buf.area.height)
        .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>() + "
")
        .collect();
    if std::env::var("SHOW_UI").is_ok() {
        println!("{text}");
    }
    assert!(text.contains("Omni: 16-part multitimbral"));
    assert!(text.contains("Bank"));
}

#[test]
fn channel_mask_splits_drums_between_slots() {
    use crate::engine::{NO_DRUMS, OMNI};
    let (tx, rx) = crossbeam_channel::bounded(64);
    let (gtx, _g) = crossbeam_channel::bounded(64);
    let shared = Arc::new(Shared::new());
    let mut e = Engine::new(48000.0, rx, gtx, shared.clone());
    let melodic = SlotParams { channels: NO_DRUMS, ..SlotParams::default() };
    let drums = SlotParams { channels: 1 << 9, ..SlotParams::default() };
    tx.send(Command::AddSlot(Box::new(Slot::new(gm_bank(), 0, melodic)))).unwrap();
    tx.send(Command::AddSlot(Box::new(Slot::new(gm_bank(), 3, drums)))).unwrap();
    tx.send(Command::Midi([0x99, 38, 100])).unwrap();
    tx.send(Command::Midi([0x90, 60, 100])).unwrap();
    tx.send(Command::Midi([0x91, 64, 100])).unwrap();
    run(&mut e);
    let voices = |s: usize| shared.slots[s].voices.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(voices(0), 2, "melodic slot: ch1 + ch2, not ch10");
    assert_eq!(voices(1), 1, "drum slot: only ch10");
    assert_eq!(shared.slots[1].channels[9].load().preset, 3);

    // Dropping a channel from the mask releases its hanging notes.
    let mut p = SlotParams { channels: OMNI & !0b11, ..SlotParams::default() };
    p.channels &= NO_DRUMS;
    tx.send(Command::SetParams { slot: 0, params: p }).unwrap();
    let mut l = vec![0f32; 48000];
    let mut r = vec![0f32; 48000];
    e.process(&mut l, &mut r);
    assert_eq!(voices(0), 0);
}

#[test]
fn channels_text_ranges() {
    use crate::app::{channels_short, channels_text};
    assert_eq!(channels_text(0xFFFF), "Omni");
    assert_eq!(channels_text(0), "none");
    assert_eq!(channels_text(1 << 9), "10");
    assert_eq!(channels_text(crate::engine::NO_DRUMS), "1-9,11-16");
    assert_eq!(channels_text(0b1010_0000_0000_0111), "1-3,14,16");
    assert_eq!(channels_short(0b1010_1010_1010_1010, 9), "8ch");
}
