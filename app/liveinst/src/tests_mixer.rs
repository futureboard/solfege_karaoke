//! Mixer strips, drum note groups, output buses, FX and SysEx tests.

use std::sync::Arc;

use crate::engine::mixer::{DRUM_STRIP_BASE, FxParams, NoteGroups, StripParams};
use crate::engine::{Command, Engine, Shared, Slot, SlotParams, load_peak};
use crate::smf::{SYS_DRUM_PART, SYS_RESET, parse_sysex};

struct Rig {
    tx: crossbeam_channel::Sender<Command>,
    e: Engine,
    shared: Arc<Shared>,
    _g: crossbeam_channel::Receiver<crate::engine::Garbage>,
}

fn rig() -> Rig {
    let (tx, rx) = crossbeam_channel::bounded(256);
    let (gtx, g) = crossbeam_channel::bounded(256);
    let shared = Arc::new(Shared::new());
    let e = Engine::new(48000.0, rx, gtx, shared.clone());
    tx.send(Command::AddSlot(Box::new(Slot::new(crate::tests_channels::gm_bank(), 0, SlotParams::default())))).unwrap();
    // Dry: keep FX returns out of bus measurements.
    tx.send(Command::SetFx(FxParams { reverb_return: 0.0, chorus_return: 0.0, ..FxParams::default() })).unwrap();
    Rig { tx, e, shared, _g: g }
}

impl Rig {
    fn render(&mut self, n: usize) {
        self.e.render(n);
    }
    fn strip(&self, k: usize) -> f32 {
        let m = &self.shared.slots[0];
        load_peak(&m.strip_l[k]).max(load_peak(&m.strip_r[k]))
    }
    fn bus_peak(&self, b: usize) -> f32 {
        let (l, r) = self.e.bus(b);
        l.iter().chain(r).fold(0f32, |m, x| m.max(x.abs()))
    }
}

#[test]
fn same_preset_on_two_channels_gets_two_strips() {
    let mut r = rig();
    // Channels 1 and 2 both use the slot preset (Piano).
    r.tx.send(Command::Midi([0x90, 60, 100])).unwrap();
    r.render(2048);
    assert!(r.strip(0) > 0.05, "ch1 strip has signal");
    assert_eq!(r.strip(1), 0.0, "ch2 strip silent");

    r.tx.send(Command::Midi([0x91, 64, 100])).unwrap();
    r.tx.send(Command::SetStrip { slot: 0, strip: 0, params: StripParams { mute: true, ..StripParams::default() } })
        .unwrap();
    for _ in 0..8 {
        r.render(2048);
    }
    assert!(r.strip(1) > 0.05, "ch2 now playing on its own strip");
    // Muting ch1's strip leaves ch2 audible.
    assert!(r.bus_peak(0) > 0.02);
    r.tx.send(Command::SetStrip { slot: 0, strip: 1, params: StripParams { mute: true, ..StripParams::default() } })
        .unwrap();
    for _ in 0..8 {
        r.render(2048);
    }
    assert!(r.bus_peak(0) < 1e-4, "both strips muted: silence");
}

#[test]
fn channel_10_note_groups_feed_separate_strips() {
    let mut r = rig();
    r.tx.send(Command::Midi([0x99, 36, 110])).unwrap(); // kick
    r.render(1024);
    assert!(r.strip(DRUM_STRIP_BASE) > 0.05, "kick group strip");
    assert_eq!(r.strip(DRUM_STRIP_BASE + 1), 0.0, "snare strip idle");
    assert_eq!(r.strip(9), 0.0, "channel 10 strip idle while grouped");

    r.tx.send(Command::Midi([0x99, 38, 110])).unwrap(); // snare
    r.render(1024);
    assert!(r.strip(DRUM_STRIP_BASE + 1) > 0.05, "snare group strip");

    r.tx.send(Command::SetNoteGroups { slot: 0, groups: NoteGroups { enabled: false, ..NoteGroups::gm() } }).unwrap();
    r.tx.send(Command::Midi([0x99, 42, 110])).unwrap(); // hi-hat, groups off
    r.render(1024);
    assert!(r.strip(9) > 0.05, "groups off: back on the channel strip");
}

#[test]
fn strip_outputs_route_to_buses_and_fold() {
    let mut r = rig();
    r.e.set_out_pairs(2);
    let to_bus1 = StripParams { output: 1, ..StripParams::default() };
    r.tx.send(Command::SetStrip { slot: 0, strip: 0, params: to_bus1 }).unwrap();
    r.tx.send(Command::Midi([0x90, 60, 100])).unwrap();
    for _ in 0..4 {
        r.render(1024);
    }
    assert!(r.bus_peak(1) > 0.05, "ch1 strip on output 3/4");
    assert!(r.bus_peak(0) < 1e-6, "main bus empty");

    // A device with only one pair: bus 1 folds back into main.
    r.e.set_out_pairs(1);
    r.render(1024);
    assert!(r.bus_peak(0) > 0.05);
    assert!(r.bus_peak(1) < 1e-6);
}

#[test]
fn strip_low_pass_cuts_high_tone() {
    let level = |lpf: f32| {
        let mut r = rig();
        r.tx.send(Command::SetStrip { slot: 0, strip: 0, params: StripParams { lpf_hz: lpf, ..StripParams::default() } })
            .unwrap();
        // gm_bank is a 0.1 rad/sample sine (~764 Hz at root); play 3 octaves up.
        r.tx.send(Command::Midi([0x90, 96, 127])).unwrap();
        for _ in 0..6 {
            r.render(1024);
        }
        r.bus_peak(0)
    };
    let open = level(20_000.0);
    let filtered = level(500.0);
    assert!(open > 0.1);
    assert!(filtered < open * 0.2, "open {open} filtered {filtered}");
}

#[test]
fn reverb_send_adds_tail() {
    let mut r = rig();
    r.tx.send(Command::SetFx(FxParams::default())).unwrap();
    r.tx.send(Command::Midi([0xB0, 91, 127])).unwrap();
    r.tx.send(Command::Midi([0x90, 60, 100])).unwrap();
    r.render(4096);
    r.tx.send(Command::Midi([0x80, 60, 0])).unwrap();
    for _ in 0..3 {
        r.render(4096);
    }
    assert!(load_peak(&r.shared.fx_peak[0]) > 0.0, "signal reached the reverb send");
    assert!(r.bus_peak(0) > 1e-4, "reverb tail after release");
}

#[test]
fn sysex_decoding() {
    let gs_reset = [0xF0, 0x41, 0x10, 0x42, 0x12, 0x40, 0x00, 0x7F, 0x00, 0x41, 0xF7];
    assert_eq!(parse_sysex(&gs_reset), Some([0xF0, SYS_RESET, 0]));
    assert_eq!(parse_sysex(&[0xF0, 0x7E, 0x7F, 0x09, 0x01, 0xF7]), Some([0xF0, SYS_RESET, 0]));
    assert_eq!(parse_sysex(&[0xF0, 0x43, 0x10, 0x4C, 0x00, 0x00, 0x7E, 0x00, 0xF7]), Some([0xF0, SYS_RESET, 0]));
    // GS "use for rhythm part": block 2 = channel 2.
    let gs_drum = [0xF0, 0x41, 0x10, 0x42, 0x12, 0x40, 0x12, 0x15, 0x02, 0x17, 0xF7];
    assert_eq!(parse_sysex(&gs_drum), Some([0xF0, SYS_DRUM_PART, 1 | 0x10]));
    // Block 0 is part 10.
    let gs_ch10_off = [0xF0, 0x41, 0x10, 0x42, 0x12, 0x40, 0x10, 0x15, 0x00, 0x1B, 0xF7];
    assert_eq!(parse_sysex(&gs_ch10_off), Some([0xF0, SYS_DRUM_PART, 9]));
    // XG part 3 -> drum.
    assert_eq!(parse_sysex(&[0xF0, 0x43, 0x10, 0x4C, 0x08, 0x03, 0x07, 0x02, 0xF7]), Some([0xF0, SYS_DRUM_PART, 3 | 0x10]));
    assert_eq!(parse_sysex(&[0xF0, 0x00, 0x01, 0xF7]), None);
}

#[test]
fn sysex_drum_part_switches_channel_to_kit() {
    let mut r = rig();
    // Key 72 exists only in melodic presets; the kit covers everything in gm_bank,
    // so check the resolved preset instead.
    r.tx.send(Command::Midi([0xF0, SYS_DRUM_PART, 1 | 0x10])).unwrap();
    r.render(64);
    let ch2 = r.shared.slots[0].channels[1].load();
    assert!(ch2.drum);
    assert_eq!(ch2.preset, 3, "ch2 now uses the standard kit");
    r.tx.send(Command::Midi([0xF0, SYS_RESET, 0])).unwrap();
    r.render(64);
    let ch2 = r.shared.slots[0].channels[1].load();
    assert!(!ch2.drum);
    assert_eq!(ch2.preset, 0);
    assert!(r.shared.slots[0].channels[9].load().drum, "reset restores ch10 as drums");
}

#[test]
fn smf_keeps_sysex_events() {
    // Format 0, one track: GS rhythm part on ch2, then a note on ch2.
    let sysex: &[u8] = &[0x0A, 0x41, 0x10, 0x42, 0x12, 0x40, 0x12, 0x15, 0x02, 0x17, 0xF7];
    let mut body = vec![0x00, 0xF0];
    body.extend(sysex);
    body.extend([0x00, 0x91, 38, 100, 0x60, 0x81, 38, 0, 0x00, 0xFF, 0x2F, 0x00]);
    let mut file = b"MThd".to_vec();
    file.extend(6u32.to_be_bytes());
    file.extend([0, 0, 0, 1, 0x01, 0xE0]);
    file.extend(b"MTrk");
    file.extend((body.len() as u32).to_be_bytes());
    file.extend(body);
    let song = crate::smf::parse(&file, "x").unwrap();
    assert_eq!(song.events[0].msg, [0xF0, SYS_DRUM_PART, 1 | 0x10]);
    assert_eq!(song.events[1].msg, [0x91, 38, 100]);
}

#[test]
fn web_actions_deserialize() {
    use crate::web::WebAction;
    let a: WebAction = serde_json::from_str(r#"{"type":"set_slot","slot":3,"patch":{"volume_db":-6,"channels":511}}"#).unwrap();
    assert!(matches!(a, WebAction::SetSlot { slot: 3, ref patch } if patch.volume_db == Some(-6.0) && patch.channels == Some(511) && patch.pan.is_none()));
    let a: WebAction =
        serde_json::from_str(r#"{"type":"set_strip","slot":1,"strip":16,"params":{"gain_db":-3,"output":2}}"#).unwrap();
    assert!(matches!(a, WebAction::SetStrip { strip: 16, params, .. } if params.output == 2 && params.lpf_hz == 20000.0));
    let a: WebAction = serde_json::from_str(r#"{"type":"transport","op":"toggle"}"#).unwrap();
    assert!(matches!(a, WebAction::Transport { .. }));
    let a: WebAction = serde_json::from_str(r#"{"type":"set_channel_preset","slot":1,"channel":9,"preset":null}"#).unwrap();
    assert!(matches!(a, WebAction::SetChannelPreset { channel: 9, preset: None, .. }));
}

#[test]
fn tui_renders_mixer() {
    use ratatui::{Terminal, backend::TestBackend};
    let dir = std::env::temp_dir().join(format!("simpletui-uim-{}", std::process::id()));
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
    app.focus = crate::app::Focus::Mixer;
    let render = |app: &crate::app::App| {
        let mut term = Terminal::new(TestBackend::new(140, 48)).unwrap();
        term.draw(|f| crate::ui::draw(f, app)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>() + "
")
            .collect::<String>()
    };
    let text = render(&app);
    if std::env::var("SHOW_UI").is_ok() {
        println!("{text}");
    }
    for needle in ["Mixer", "10 Kick", "10 Snare", "HPF", "Out"] {
        assert!(text.contains(needle), "missing {needle}");
    }
    // FX returns and master sit below the strips; selecting them scrolls.
    app.mix_row = app.mixer_rows().len() - 1;
    let text = render(&app);
    for needle in ["FX Reverb", "FX Chorus", "MASTER"] {
        assert!(text.contains(needle), "missing {needle}");
    }
    // 15 channel strips + 7 drum groups (cowbell has its own) for an Omni slot.
    assert_eq!(app.visible_strips(0).len(), 15 + 7);
}
