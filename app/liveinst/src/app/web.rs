//! Bridge between the App (source of truth) and the web UI: builds the JSON
//! snapshot the browser renders and applies `WebAction`s it sends back.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde::Serialize;

use super::{App, LoadTarget, channels_text, format_time};
use crate::engine::mixer::{DRUM_STRIP_BASE, FxParams, MAX_BUSES, MAX_GROUPS, NoteGroups, StripParams};
use crate::engine::{Command, MAX_SLOTS};
use crate::instrument::{LoopMode, db_to_gain};
use crate::web::{SlotMeta, WebAction};

#[derive(Serialize)]
struct AudioView<'a> {
    host: &'a str,
    device: &'a str,
    sample_rate: u32,
    channels: u16,
    format: &'a str,
    buffer: &'a str,
}

#[derive(Serialize)]
struct ParamsView {
    volume_db: f32,
    pan: f32,
    channels: u16,
    channels_text: String,
    transpose: i32,
    tune: f32,
    key_lo: u8,
    key_hi: u8,
    vel_lo: u8,
    vel_hi: u8,
    bend_range: f32,
    mute: bool,
    solo: bool,
}

#[derive(Serialize)]
struct WavView {
    root: u8,
    keytrack: bool,
    loop_mode: &'static str,
    attack: f32,
    hold: f32,
    decay: f32,
    sustain: f32,
    release: f32,
}

#[derive(Serialize)]
struct StripView {
    index: usize,
    name: String,
    channel: u8,
    group: Option<usize>,
    params: StripParams,
}

#[derive(Serialize)]
struct GroupsView<'a> {
    enabled: bool,
    map: Vec<u8>,
    names: &'a [String],
}

#[derive(Serialize)]
struct SlotView<'a> {
    id: u64,
    index: usize,
    name: &'a str,
    kind: &'static str,
    path: String,
    preset: usize,
    preset_name: String,
    presets: usize,
    zones: usize,
    sample_mb: f64,
    warnings: Vec<&'a str>,
    params: ParamsView,
    wav: Option<WavView>,
    strips: Vec<StripView>,
    groups: GroupsView<'a>,
}

#[derive(Serialize)]
struct SongView<'a> {
    name: &'a str,
    path: String,
    duration: f64,
    duration_text: String,
    bpm: f64,
    format: u16,
    tracks: usize,
    events: usize,
    channels_used: u16,
}

#[derive(Serialize)]
struct PlayerView<'a> {
    song: Option<SongView<'a>>,
    looping: bool,
    speed: f64,
    mutes: u16,
}

#[derive(Serialize)]
struct LogView<'a> {
    text: &'a str,
    error: bool,
}

#[derive(Serialize)]
struct Snapshot<'a> {
    audio: Option<AudioView<'a>>,
    audio_error: Option<&'a str>,
    midi_inputs: Vec<String>,
    midi_output: Option<String>,
    midi_thru: bool,
    forward_player: bool,
    master_db: f32,
    fx: FxParams,
    max_buses: usize,
    max_slots: usize,
    slots: Vec<SlotView<'a>>,
    player: PlayerView<'a>,
    loading: Vec<String>,
    log: Vec<LogView<'a>>,
    browse_dir: String,
}

fn loop_name(m: LoopMode) -> &'static str {
    match m {
        LoopMode::NoLoop => "no_loop",
        LoopMode::OneShot => "one_shot",
        LoopMode::Continuous => "loop_continuous",
        LoopMode::Sustain => "loop_sustain",
    }
}

fn parse_loop(s: &str) -> Option<LoopMode> {
    Some(match s {
        "no_loop" => LoopMode::NoLoop,
        "one_shot" => LoopMode::OneShot,
        "loop_continuous" => LoopMode::Continuous,
        "loop_sustain" => LoopMode::Sustain,
        _ => return None,
    })
}

impl App {
    fn snapshot_json(&self) -> String {
        let slots: Vec<SlotView> = self
            .slots
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let p = &s.params;
                let strips = self
                    .visible_strips(i)
                    .into_iter()
                    .map(|k| StripView {
                        index: k,
                        name: self.strip_name(i, k),
                        channel: if k >= DRUM_STRIP_BASE { 9 } else { k as u8 },
                        group: (k >= DRUM_STRIP_BASE).then(|| k - DRUM_STRIP_BASE),
                        params: s.mixer.strips[k],
                    })
                    .collect();
                SlotView {
                    id: s.id,
                    index: i,
                    name: &s.inst.name,
                    kind: s.inst.kind.label(),
                    path: s.inst.path.display().to_string(),
                    preset: s.preset,
                    preset_name: s
                        .inst
                        .presets
                        .get(s.preset)
                        .map(|p| format!("{:03}:{:03} {}", p.bank, p.program, p.name))
                        .unwrap_or_default(),
                    presets: s.inst.presets.len(),
                    zones: s.inst.zone_count(),
                    sample_mb: s.inst.sample_bytes as f64 / 1_048_576.0,
                    warnings: s.inst.warnings.iter().take(5).map(String::as_str).collect(),
                    params: ParamsView {
                        volume_db: s.volume_db,
                        pan: p.pan,
                        channels: p.channels,
                        channels_text: channels_text(p.channels),
                        transpose: p.transpose,
                        tune: p.tune,
                        key_lo: p.key_lo,
                        key_hi: p.key_hi,
                        vel_lo: p.vel_lo,
                        vel_hi: p.vel_hi,
                        bend_range: p.bend_range,
                        mute: p.mute,
                        solo: p.solo,
                    },
                    wav: s.wav.map(|w| WavView {
                        root: w.root,
                        keytrack: w.keytrack,
                        loop_mode: loop_name(w.loop_mode),
                        attack: w.env.attack,
                        hold: w.env.hold,
                        decay: w.env.decay,
                        sustain: w.env.sustain,
                        release: w.env.release,
                    }),
                    strips,
                    groups: GroupsView {
                        enabled: s.mixer.groups.enabled,
                        map: s.mixer.groups.map.to_vec(),
                        names: &s.mixer.group_names,
                    },
                }
            })
            .collect();
        let snap = Snapshot {
            audio: self.audio.as_ref().map(|a| AudioView {
                host: &a.host,
                device: &a.device,
                sample_rate: a.sample_rate,
                channels: a.channels,
                format: &a.format,
                buffer: &a.buffer,
            }),
            audio_error: self.audio_error.as_deref(),
            midi_inputs: self.midi.connected_inputs(),
            midi_output: self.midi.output_name(),
            midi_thru: self.midi.thru.load(Ordering::Relaxed),
            forward_player: self.shared.forward_player.load(Ordering::Relaxed),
            master_db: self.master_db,
            fx: self.fx,
            max_buses: MAX_BUSES,
            max_slots: MAX_SLOTS,
            slots,
            player: PlayerView {
                song: self.player.song.as_ref().map(|s| SongView {
                    name: &s.name,
                    path: self.player.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
                    duration: s.duration,
                    duration_text: format_time(s.duration),
                    bpm: s.bpm,
                    format: s.format,
                    tracks: s.tracks,
                    events: s.events.len(),
                    channels_used: s.channels_used,
                }),
                looping: self.player.looping,
                speed: self.player.speed,
                mutes: self.player.mutes,
            },
            loading: self.loading.iter().map(|p| p.display().to_string()).collect(),
            log: self.log.iter().rev().take(40).rev().map(|l| LogView { text: &l.text, error: l.error }).collect(),
            browse_dir: self.browse_dir.display().to_string(),
        };
        serde_json::to_string(&snap).unwrap_or_else(|_| "{}".into())
    }

    /// Called every tick: apply browser actions, then publish state.
    pub(super) fn web_tick(&mut self) {
        while let Ok(a) = self.web_rx.try_recv() {
            self.web_action(a);
        }
        let Some(hub) = self.web.clone() else { return };
        let metas = self.slots.iter().map(|s| SlotMeta { id: s.id, inst: s.inst.clone() }).collect();
        hub.publish(self.snapshot_json(), metas, &self.browse_dir);
    }

    fn slot_index(&self, id: u64) -> Option<usize> {
        self.slots.iter().position(|s| s.id == id)
    }

    fn web_action(&mut self, a: WebAction) {
        match a {
            WebAction::AddInstrument { path } => self.load(PathBuf::from(path), LoadTarget::New),
            WebAction::ReplaceInstrument { slot, path } => {
                if self.slot_index(slot).is_some() {
                    self.load(PathBuf::from(path), LoadTarget::Replace(slot));
                }
            }
            WebAction::RemoveSlot { slot } => {
                if let Some(i) = self.slot_index(slot) {
                    self.sel = i;
                    self.remove_selected();
                }
            }
            WebAction::SetSlot { slot, patch } => {
                let Some(i) = self.slot_index(slot) else { return };
                let s = &mut self.slots[i];
                if let Some(db) = patch.volume_db {
                    s.volume_db = db.clamp(-60.0, 12.0);
                    s.params.gain = if s.volume_db <= -60.0 { 0.0 } else { db_to_gain(s.volume_db) };
                }
                let p = &mut s.params;
                if let Some(v) = patch.pan {
                    p.pan = v.clamp(-1.0, 1.0);
                }
                if let Some(v) = patch.transpose {
                    p.transpose = v.clamp(-48, 48);
                }
                if let Some(v) = patch.tune {
                    p.tune = v.clamp(-100.0, 100.0);
                }
                if let Some(v) = patch.key_lo {
                    p.key_lo = v.min(127);
                }
                if let Some(v) = patch.key_hi {
                    p.key_hi = v.min(127);
                }
                if let Some(v) = patch.vel_lo {
                    p.vel_lo = v.clamp(1, 127);
                }
                if let Some(v) = patch.vel_hi {
                    p.vel_hi = v.clamp(1, 127);
                }
                if let Some(v) = patch.bend_range {
                    p.bend_range = v.clamp(0.0, 48.0);
                }
                if let Some(v) = patch.mute {
                    p.mute = v;
                }
                if let Some(v) = patch.solo {
                    p.solo = v;
                }
                let channels = patch.channels;
                self.update_params(i, |p| {
                    if let Some(c) = channels {
                        p.channels = c;
                    }
                });
            }
            WebAction::SetWav { slot, patch } => {
                let Some(i) = self.slot_index(slot) else { return };
                let Some(w) = self.slots[i].wav.as_mut() else { return };
                if let Some(v) = patch.root {
                    w.root = v.min(127);
                }
                if let Some(v) = patch.keytrack {
                    w.keytrack = v;
                }
                if let Some(m) = patch.loop_mode.as_deref().and_then(parse_loop) {
                    w.loop_mode = m;
                }
                let t = |v: f32| v.clamp(0.0, 30.0);
                if let Some(v) = patch.attack {
                    w.env.attack = t(v);
                }
                if let Some(v) = patch.hold {
                    w.env.hold = t(v);
                }
                if let Some(v) = patch.decay {
                    w.env.decay = t(v);
                }
                if let Some(v) = patch.sustain {
                    w.env.sustain = v.clamp(0.0, 1.0);
                }
                if let Some(v) = patch.release {
                    w.env.release = t(v).max(0.001);
                }
                self.rebuild_wav(i);
            }
            WebAction::SetPreset { slot, preset } => {
                let Some(i) = self.slot_index(slot) else { return };
                if preset < self.slots[i].inst.presets.len() {
                    self.slots[i].preset = preset;
                    self.send(Command::SetPreset { slot: i, preset });
                }
            }
            WebAction::SetChannelPreset { slot, channel, preset } => {
                let Some(i) = self.slot_index(slot) else { return };
                self.send(Command::SetChannelPreset { slot: i, ch: channel.min(15), preset });
            }
            WebAction::SetStrip { slot, strip, params } => {
                if let Some(i) = self.slot_index(slot) {
                    self.set_strip(i, strip, params);
                }
            }
            WebAction::SetNoteGroups { slot, enabled, map, names } => {
                let Some(i) = self.slot_index(slot) else { return };
                let mut groups = NoteGroups { enabled, map: self.slots[i].mixer.groups.map };
                for (dst, src) in groups.map.iter_mut().zip(map) {
                    *dst = if (src as usize) < MAX_GROUPS { src } else { crate::engine::mixer::NO_GROUP };
                }
                let mut new_names = self.slots[i].mixer.group_names.clone();
                for (dst, src) in new_names.iter_mut().zip(names) {
                    let trimmed: String = src.trim().chars().take(16).collect();
                    if !trimmed.is_empty() {
                        *dst = trimmed;
                    }
                }
                self.set_note_groups(i, groups, Some(new_names));
            }
            WebAction::SetFx { fx } => self.set_fx(fx),
            WebAction::SetMaster { db } => self.set_master_db(db),
            WebAction::Transport { op } => match op.as_str() {
                "play" => self.play(),
                "pause" if self.player.state == crate::engine::PlayState::Playing => self.toggle_play(),
                "toggle" if self.player.song.is_some() => self.toggle_play(),
                "stop" => self.stop(),
                _ => {}
            },
            WebAction::Seek { time } => self.seek_to(time),
            WebAction::SetLoop { on } => {
                if self.player.looping != on {
                    self.toggle_loop();
                }
            }
            WebAction::SetSpeed { speed } => self.set_speed(speed),
            WebAction::SetMutes { mutes } => self.set_mutes(mutes),
            WebAction::SetForward { on } => self.shared.forward_player.store(on, Ordering::Relaxed),
            WebAction::LoadSong { path } => {
                self.load_song(PathBuf::from(path));
            }
            WebAction::Note { slot, key, vel } => {
                if let Some(i) = self.slot_index(slot) {
                    self.send(Command::Note { slot: i, key: key.min(127), vel: vel.min(127) });
                }
            }
            WebAction::Panic => self.panic(),
        }
    }

    pub fn start_web(&mut self, addr: std::net::SocketAddr, dir: Option<PathBuf>) {
        let hub = Arc::new(crate::web::WebHub::new(self.web_tx.clone(), self.shared.clone()));
        match crate::web::start(addr, hub.clone(), dir.clone()) {
            Ok(bound) => {
                let url = format!("http://{bound}");
                self.info(format!("web UI: {url}"));
                if dir.is_none() {
                    self.error("web UI not built: cd app/liveinst/webui && npm install && npm run build");
                }
                self.web = Some(hub);
                self.web_url = Some(url);
            }
            Err(e) => self.error(format!("web UI: {e:#}")),
        }
    }
}
