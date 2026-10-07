//! Mixer state on the UI side: per-slot strip parameters and channel-10
//! note groups, global FX, and the TUI mixer view's editing logic.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{App, Focus, pan_text};
use crate::engine::Command;
use crate::engine::mixer::{
    DRUM_CHANNEL, DRUM_STRIP_BASE, FxParams, GM_GROUP_NAMES, MAX_BUSES, MAX_GROUPS, MAX_STRIPS, NoteGroups, StripParams,
};
use crate::instrument::db_to_gain;

#[derive(Clone, Debug)]
pub struct SlotMixer {
    pub strips: [StripParams; MAX_STRIPS],
    pub groups: NoteGroups,
    pub group_names: [String; MAX_GROUPS],
}

impl Default for SlotMixer {
    fn default() -> Self {
        Self {
            strips: [StripParams::default(); MAX_STRIPS],
            groups: NoteGroups::gm(),
            group_names: GM_GROUP_NAMES.map(String::from),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MixRow {
    Strip(usize),
    Reverb,
    Chorus,
    Master,
}

pub const STRIP_FIELDS: [&str; 10] = ["Vol", "Pan", "Low", "Mid", "High", "HPF", "LPF", "Rev", "Cho", "Out"];

pub fn output_name(bus: u8) -> String {
    if bus == 0 { "Main".into() } else { format!("{}/{}", bus * 2 + 1, bus * 2 + 2) }
}

fn hz_text(hz: f32, off_at: f32, low_is_off: bool) -> String {
    let off = if low_is_off { hz <= off_at } else { hz >= off_at };
    if off {
        "off".into()
    } else if hz >= 1000.0 {
        format!("{:.1}k", hz / 1000.0)
    } else {
        format!("{hz:.0}")
    }
}

pub fn strip_field_text(p: &StripParams, field: usize) -> String {
    match field {
        0 => {
            if p.gain_db <= -60.0 {
                "-inf".into()
            } else {
                format!("{:+.1}", p.gain_db)
            }
        }
        1 => pan_text(p.pan),
        2 => format!("{:+.1}", p.eq_low_db),
        3 => format!("{:+.1}", p.eq_mid_db),
        4 => format!("{:+.1}", p.eq_high_db),
        5 => hz_text(p.hpf_hz, 20.0, true),
        6 => hz_text(p.lpf_hz, 20_000.0, false),
        7 => format!("{:.0}%", p.reverb * 100.0),
        8 => format!("{:.0}%", p.chorus * 100.0),
        _ => output_name(p.output),
    }
}

pub fn fx_field_text(fx: &FxParams, row: MixRow, field: usize) -> String {
    match (row, field) {
        (MixRow::Reverb, 0) => format!("{:.0}%", fx.reverb_return * 100.0),
        (MixRow::Reverb, 1) => format!("{:.0}%", fx.reverb_room * 100.0),
        (MixRow::Reverb, 2) => format!("{:.0}%", fx.reverb_damp * 100.0),
        (MixRow::Reverb, 3) => format!("{:.0}%", fx.reverb_width * 100.0),
        (MixRow::Chorus, 0) => format!("{:.0}%", fx.chorus_return * 100.0),
        (MixRow::Chorus, 1) => format!("{:.2}Hz", fx.chorus_rate),
        (MixRow::Chorus, 2) => format!("{:.1}ms", fx.chorus_depth),
        (MixRow::Chorus, 3) => format!("{:.1}ms", fx.chorus_delay),
        _ => String::new(),
    }
}

impl App {
    /// Strips worth showing for a slot: every received channel, with
    /// channel 10 replaced by its used note groups when groups are on.
    pub fn visible_strips(&self, slot: usize) -> Vec<usize> {
        let Some(s) = self.slots.get(slot) else { return Vec::new() };
        let mut v = Vec::new();
        for c in 0..16u8 {
            if !s.params.receives(c) {
                continue;
            }
            if c == DRUM_CHANNEL && s.mixer.groups.enabled {
                let used = s.mixer.groups.used();
                v.extend((0..MAX_GROUPS).filter(|g| used & (1 << g) != 0).map(|g| DRUM_STRIP_BASE + g));
                // Notes mapped to no group still play on the channel strip.
                if s.mixer.groups.map.iter().any(|&g| g as usize >= MAX_GROUPS) {
                    v.push(c as usize);
                }
            } else {
                v.push(c as usize);
            }
        }
        v
    }

    pub fn strip_name(&self, slot: usize, k: usize) -> String {
        let Some(s) = self.slots.get(slot) else { return String::new() };
        if k >= DRUM_STRIP_BASE {
            return format!("10 {}", s.mixer.group_names[k - DRUM_STRIP_BASE]);
        }
        let info = self.channel_info(slot, k);
        let preset = s.inst.presets.get(info.preset).map(|p| p.name.as_str()).unwrap_or("");
        if s.inst.presets.len() > 1 { format!("{:>2} {preset}", k + 1) } else { format!("{:>2} {}", k + 1, s.inst.name) }
    }

    pub fn mixer_rows(&self) -> Vec<MixRow> {
        let mut rows: Vec<MixRow> = self.visible_strips(self.sel).into_iter().map(MixRow::Strip).collect();
        rows.extend([MixRow::Reverb, MixRow::Chorus, MixRow::Master]);
        rows
    }

    pub fn set_strip(&mut self, slot: usize, strip: usize, params: StripParams) {
        let Some(s) = self.slots.get_mut(slot) else { return };
        let Some(dst) = s.mixer.strips.get_mut(strip) else { return };
        *dst = params.clamped();
        let params = *dst;
        self.send(Command::SetStrip { slot, strip, params });
    }

    pub fn set_fx(&mut self, fx: FxParams) {
        self.fx = fx.clamped();
        self.send(Command::SetFx(self.fx));
    }

    pub fn set_note_groups(&mut self, slot: usize, groups: NoteGroups, names: Option<[String; MAX_GROUPS]>) {
        let Some(s) = self.slots.get_mut(slot) else { return };
        s.mixer.groups = groups;
        if let Some(n) = names {
            s.mixer.group_names = n;
        }
        self.send(Command::SetNoteGroups { slot, groups });
    }

    fn adjust_strip(&mut self, k: usize, field: usize, dir: f32, coarse: bool) {
        let slot = self.sel;
        let Some(s) = self.slots.get(slot) else { return };
        let mut p = s.mixer.strips[k];
        let step = |fine: f32, big: f32| if coarse { big } else { fine } * dir;
        let mul = |v: f32| v * if coarse { 1.5f32 } else { 1.12 }.powf(dir);
        match field {
            0 => p.gain_db = (p.gain_db.max(-60.0) + step(0.5, 3.0)).clamp(-60.0, 12.0),
            1 => p.pan += step(0.05, 0.25),
            2 => p.eq_low_db += step(0.5, 3.0),
            3 => p.eq_mid_db += step(0.5, 3.0),
            4 => p.eq_high_db += step(0.5, 3.0),
            5 => p.hpf_hz = mul(p.hpf_hz),
            6 => p.lpf_hz = mul(p.lpf_hz),
            7 => p.reverb += step(0.05, 0.25),
            8 => p.chorus += step(0.05, 0.25),
            _ => p.output = ((p.output as i32 + dir as i32).rem_euclid(MAX_BUSES as i32)) as u8,
        }
        self.set_strip(slot, k, p);
    }

    fn reset_strip_field(&mut self, k: usize, field: usize) {
        let slot = self.sel;
        let Some(s) = self.slots.get(slot) else { return };
        let mut p = s.mixer.strips[k];
        let d = StripParams::default();
        match field {
            0 => p.gain_db = d.gain_db,
            1 => p.pan = d.pan,
            2 => p.eq_low_db = 0.0,
            3 => p.eq_mid_db = 0.0,
            4 => p.eq_high_db = 0.0,
            5 => p.hpf_hz = d.hpf_hz,
            6 => p.lpf_hz = d.lpf_hz,
            7 => p.reverb = d.reverb,
            8 => p.chorus = d.chorus,
            _ => p.output = 0,
        }
        self.set_strip(slot, k, p);
    }

    fn adjust_fx(&mut self, row: MixRow, field: usize, dir: f32, coarse: bool) {
        let mut fx = self.fx;
        let step = |fine: f32, big: f32| if coarse { big } else { fine } * dir;
        match (row, field) {
            (MixRow::Reverb, 0) => fx.reverb_return += step(0.05, 0.25),
            (MixRow::Reverb, 1) => fx.reverb_room += step(0.05, 0.2),
            (MixRow::Reverb, 2) => fx.reverb_damp += step(0.05, 0.2),
            (MixRow::Reverb, 3) => fx.reverb_width += step(0.05, 0.2),
            (MixRow::Chorus, 0) => fx.chorus_return += step(0.05, 0.25),
            (MixRow::Chorus, 1) => fx.chorus_rate += step(0.05, 0.5),
            (MixRow::Chorus, 2) => fx.chorus_depth += step(0.25, 1.0),
            (MixRow::Chorus, 3) => fx.chorus_delay += step(0.5, 2.0),
            _ => {}
        }
        self.set_fx(fx);
    }

    pub fn set_master_db(&mut self, db: f32) {
        self.master_db = db.clamp(-60.0, 12.0);
        self.send(Command::MasterGain(db_to_gain(self.master_db)));
    }

    pub(super) fn on_mixer_key(&mut self, k: KeyEvent) -> bool {
        let rows = self.mixer_rows();
        self.mix_row = self.mix_row.min(rows.len().saturating_sub(1));
        let row = rows.get(self.mix_row).copied().unwrap_or(MixRow::Master);
        let coarse = k.modifiers.contains(KeyModifiers::SHIFT);
        let adjust = |app: &mut App, dir: f32, coarse: bool| match row {
            MixRow::Strip(s) => app.adjust_strip(s, app.mix_field, dir, coarse),
            MixRow::Reverb | MixRow::Chorus => app.adjust_fx(row, app.mix_field.min(3), dir, coarse),
            MixRow::Master => app.set_master_db(app.master_db + if coarse { 3.0 } else { 0.5 } * dir),
        };
        match k.code {
            KeyCode::Up => self.mix_row = self.mix_row.saturating_sub(1),
            KeyCode::Down => self.mix_row = (self.mix_row + 1).min(rows.len().saturating_sub(1)),
            KeyCode::Left => self.mix_field = self.mix_field.saturating_sub(1),
            KeyCode::Right => self.mix_field = (self.mix_field + 1).min(STRIP_FIELDS.len() - 1),
            KeyCode::Char('+') | KeyCode::Char('=') => adjust(self, 1.0, false),
            KeyCode::Char('-') | KeyCode::Char('_') => adjust(self, -1.0, coarse),
            KeyCode::PageUp => adjust(self, 1.0, true),
            KeyCode::PageDown => adjust(self, -1.0, true),
            KeyCode::Delete | KeyCode::Char('0') => {
                if let MixRow::Strip(s) = row {
                    self.reset_strip_field(s, self.mix_field);
                }
            }
            KeyCode::Char('m') | KeyCode::Char('s') => {
                if let MixRow::Strip(s) = row {
                    let mut p = self.slots[self.sel].mixer.strips[s];
                    if k.code == KeyCode::Char('m') {
                        p.mute = !p.mute;
                    } else {
                        p.solo = !p.solo;
                    }
                    self.set_strip(self.sel, s, p);
                }
            }
            KeyCode::Char('g') => {
                if let Some(s) = self.slots.get(self.sel) {
                    let mut g = s.mixer.groups;
                    g.enabled = !g.enabled;
                    self.set_note_groups(self.sel, g, None);
                    let state = if g.enabled { "on" } else { "off" };
                    self.info(format!("ch 10 note groups (multi out) {state}"));
                }
            }
            KeyCode::Tab => self.focus = Focus::Player,
            KeyCode::BackTab => self.focus = Focus::Channels,
            KeyCode::Esc | KeyCode::Char('M') => self.focus = Focus::Rack,
            _ => return false,
        }
        true
    }
}
