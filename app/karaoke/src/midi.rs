//! MIDI I/O through midir (WinMM on Windows, CoreMIDI on macOS, the ALSA
//! sequencer on Linux).
//!
//! **Output.** The song plays either on the Solfege Engine (the SoundFont
//! synth inside the app) or on a MIDI device: a keyboard, a sound module,
//! another program. For a device the engine's player keeps time and hands
//! its events to a thread here, which applies what the app would otherwise
//! apply in the engine: the key, mute / solo, the guide melody switch and
//! the volume (as GM Master Volume). SoundFonts, faders and effects only
//! shape the engine's own sound.
//!
//! **Input.** A MIDI keyboard plays along on whichever output is in use,
//! on its own channels or moved to one channel.

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use solfege_synth::smf::{SYS_DRUM_PART, SYS_RESET};

const CLIENT: &str = "Solfege Karaoke";
/// Name of the built-in synth as an output choice.
pub const ENGINE: &str = "Solfege Engine";
/// GM drum channel (10), zero-based.
const DRUM_CH: u8 = 9;
/// GM System On.
const GM_ON: [u8; 6] = [0xF0, 0x7E, 0x7F, 0x09, 0x01, 0xF7];

/// The MIDI devices the system offers (or why it offers none).
pub struct MidiPorts {
    pub outputs: Result<Vec<String>, String>,
    pub inputs: Result<Vec<String>, String>,
}

impl MidiPorts {
    pub fn scan() -> Self {
        Self { outputs: output_ports(), inputs: input_ports() }
    }
}

fn output_ports() -> Result<Vec<String>, String> {
    let m = MidiOutput::new(CLIENT).map_err(|e| e.to_string())?;
    // Our own input shows up as an output on some systems: skip it.
    Ok(m.ports().iter().filter_map(|p| m.port_name(p).ok()).filter(|n| !n.starts_with(CLIENT)).collect())
}

fn input_ports() -> Result<Vec<String>, String> {
    let m = MidiInput::new(CLIENT).map_err(|e| e.to_string())?;
    Ok(m.ports().iter().filter_map(|p| m.port_name(p).ok()).filter(|n| !n.starts_with(CLIENT)).collect())
}

/// What the app sets for the output thread, read on every event.
#[derive(Default)]
pub struct OutState {
    /// Semitones to move every note (not the drums).
    pub key: AtomicI32,
    /// Bit per channel whose notes are not sent (muted, not soloed, the
    /// guide melody switched off).
    pub blocked: AtomicU32,
    /// GM Master Volume, 0..=16383.
    pub volume: AtomicU32,
}

/// GM Master Volume SysEx for a 14-bit volume.
pub fn master_volume(v: u32) -> Vec<u8> {
    let v = v.min(16383);
    vec![0xF0, 0x7F, 0x7F, 0x04, 0x01, (v & 0x7F) as u8, (v >> 7) as u8, 0xF7]
}

enum OutCmd {
    /// A new song: GM reset and everything off.
    Reset,
    /// A message from the MIDI input, sent as it is.
    Thru([u8; 3], u8),
}

/// An open MIDI output device.
pub struct MidiOut {
    pub name: String,
    tx: Option<Sender<OutCmd>>,
    thread: Option<JoinHandle<()>>,
}

impl MidiOut {
    /// Open the device called `name`; it plays what arrives on `events`
    /// (the engine's player, see `Shared::out_rx`).
    pub fn open(name: &str, events: Receiver<([u8; 3], u8)>, state: Arc<OutState>) -> Result<Self, String> {
        let m = MidiOutput::new(CLIENT).map_err(|e| e.to_string())?;
        let port = m.ports().into_iter().find(|p| m.port_name(p).is_ok_and(|n| n == name)).ok_or_else(|| format!("{name}: ไม่พบอุปกรณ์"))?;
        let conn = m.connect(&port, "out").map_err(|e| format!("{name}: {e}"))?;
        // Events queued while no device listened are stale.
        while events.try_recv().is_ok() {}
        let (tx, rx) = crossbeam_channel::bounded(1024);
        let thread = std::thread::Builder::new()
            .name("midi-out".into())
            .spawn(move || run_output(conn, events, rx, state))
            .map_err(|e| e.to_string())?;
        Ok(Self { name: name.to_string(), tx: Some(tx), thread: Some(thread) })
    }

    /// GM reset and all notes off, before a new song.
    pub fn reset(&self) {
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(OutCmd::Reset);
        }
    }

    fn sender(&self) -> Option<Sender<OutCmd>> {
        self.tx.clone()
    }
}

impl Drop for MidiOut {
    fn drop(&mut self) {
        // Closing the channel ends the thread, which silences the device.
        self.tx = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run_output(mut conn: MidiOutputConnection, events: Receiver<([u8; 3], u8)>, cmds: Receiver<OutCmd>, state: Arc<OutState>) {
    let mut filter = OutFilter::default();
    let mut out = Vec::new();
    // Volume last sent; a GM reset puts the device back to full volume.
    let mut volume = None;
    loop {
        crossbeam_channel::select! {
            recv(events) -> ev => {
                if let Ok((msg, len)) = ev {
                    filter.player(msg, len, state.key.load(Ordering::Relaxed), &mut out);
                }
            }
            recv(cmds) -> cmd => match cmd {
                Ok(OutCmd::Reset) => {
                    filter.reset(true, &mut out);
                    volume = None;
                }
                Ok(OutCmd::Thru(msg, len)) => out.push(msg[..len as usize].to_vec()),
                Err(_) => {
                    filter.reset(false, &mut out);
                    send_all(&mut conn, &mut out);
                    return;
                }
            },
            // Mute / solo changes silence held notes even when nothing plays.
            default(Duration::from_millis(30)) => {}
        }
        filter.block(state.blocked.load(Ordering::Relaxed) as u16, &mut out);
        if std::mem::take(&mut filter.was_reset) {
            volume = None;
        }
        let v = state.volume.load(Ordering::Relaxed);
        if volume != Some(v) {
            out.push(master_volume(v));
            volume = Some(v);
        }
        send_all(&mut conn, &mut out);
    }
}

fn send_all(conn: &mut MidiOutputConnection, out: &mut Vec<Vec<u8>>) {
    for m in out.drain(..) {
        let _ = conn.send(&m);
    }
}

/// Turns the player's events into what the device gets: moved to the
/// key, without blocked channels, decoded SysEx made real again. Every
/// note-off goes to the key its note-on went to, so changing the key or
/// muting a channel never leaves a note hanging.
#[derive(Default)]
pub struct OutFilter {
    /// Key sent + 1 per (channel, key played); 0 = not sounding.
    sounding: Vec<[u8; 128]>,
    /// Channels that are rhythm parts besides channel 10.
    drums: u16,
    blocked: u16,
    /// The song sent a GM / GS / XG reset since this was last cleared.
    pub was_reset: bool,
}

impl OutFilter {
    fn sounding(&mut self) -> &mut Vec<[u8; 128]> {
        if self.sounding.is_empty() {
            self.sounding = vec![[0; 128]; 16];
        }
        &mut self.sounding
    }

    fn is_drums(&self, ch: u8) -> bool {
        ch == DRUM_CH || self.drums & (1 << ch) != 0
    }

    /// One player event; what to send is pushed to `out`.
    pub fn player(&mut self, msg: [u8; 3], len: u8, key: i32, out: &mut Vec<Vec<u8>>) {
        let ch = msg[0] & 0x0F;
        match msg[0] & 0xF0 {
            0xF0 if msg[0] == 0xF0 => self.system(msg[1], msg[2], out),
            0x90 if msg[2] > 0 => {
                if self.blocked & (1 << ch) != 0 {
                    return;
                }
                let shift = if self.is_drums(ch) { 0 } else { key };
                let Ok(k) = u8::try_from(msg[1] as i32 + shift) else { return };
                if k > 127 {
                    return;
                }
                // The same key struck again: end the earlier note first.
                let held = self.sounding()[ch as usize][msg[1] as usize & 0x7F];
                if held != 0 {
                    out.push(vec![0x80 | ch, held - 1, 0]);
                }
                self.sounding()[ch as usize][msg[1] as usize & 0x7F] = k + 1;
                out.push(vec![msg[0], k, msg[2]]);
            }
            0x80 | 0x90 => {
                let slot = &mut self.sounding()[ch as usize][msg[1] as usize & 0x7F];
                if *slot != 0 {
                    out.push(vec![0x80 | ch, *slot - 1, msg[2]]);
                    *slot = 0;
                }
            }
            0xA0 => {
                let held = self.sounding()[ch as usize][msg[1] as usize & 0x7F];
                if held != 0 {
                    out.push(vec![msg[0], held - 1, msg[2]]);
                }
            }
            0xB0 if matches!(msg[1], 120 | 123) => {
                self.sounding()[ch as usize] = [0; 128];
                out.push(msg[..3].to_vec());
            }
            _ => out.push(msg[..(len as usize).clamp(1, 3)].to_vec()),
        }
    }

    fn system(&mut self, kind: u8, arg: u8, out: &mut Vec<Vec<u8>>) {
        match kind {
            SYS_RESET => {
                self.drums = 0;
                self.was_reset = true;
                out.push(GM_ON.to_vec());
            }
            SYS_DRUM_PART => {
                let (ch, on) = (arg & 0x0F, arg & 0x10 != 0);
                if ch != DRUM_CH {
                    self.drums = if on { self.drums | (1 << ch) } else { self.drums & !(1 << ch) };
                }
                // The song's own SysEx is gone; say it both ways (GS and
                // XG), a device ignores the one it does not speak.
                let block = match ch {
                    DRUM_CH => 0,
                    c if c < DRUM_CH => c + 1,
                    c => c,
                };
                let (addr, data) = ([0x40, 0x10 | block, 0x15], on as u8);
                let sum = addr.iter().map(|&b| b as u32).sum::<u32>() + data as u32;
                let check = ((128 - sum % 128) % 128) as u8;
                out.push(vec![0xF0, 0x41, 0x10, 0x42, 0x12, addr[0], addr[1], addr[2], data, check, 0xF7]);
                out.push(vec![0xF0, 0x43, 0x10, 0x4C, 0x08, ch, 0x07, on as u8, 0xF7]);
            }
            _ => {}
        }
    }

    /// Follow the blocked channels: notes still sounding on a channel that
    /// just got blocked are ended.
    pub fn block(&mut self, blocked: u16, out: &mut Vec<Vec<u8>>) {
        let newly = blocked & !self.blocked;
        self.blocked = blocked;
        for ch in (0..16u8).filter(|c| newly & (1 << c) != 0) {
            self.release(ch, out);
        }
    }

    fn release(&mut self, ch: u8, out: &mut Vec<Vec<u8>>) {
        let notes = std::mem::replace(&mut self.sounding()[ch as usize], [0; 128]);
        for k in notes.into_iter().filter(|&k| k != 0) {
            out.push(vec![0x80 | ch, k - 1, 0]);
        }
    }

    /// Everything off (and with `gm`, a GM reset first).
    pub fn reset(&mut self, gm: bool, out: &mut Vec<Vec<u8>>) {
        if gm {
            out.push(GM_ON.to_vec());
            self.drums = 0;
        }
        for ch in 0..16u8 {
            self.release(ch, out);
            out.push(vec![0xB0 | ch, 64, 0]);
            out.push(vec![0xB0 | ch, 123, 0]);
        }
    }
}

/// An open MIDI input device.
pub struct MidiIn {
    pub name: String,
    _conn: MidiInputConnection<()>,
}

/// Where the MIDI input plays.
#[derive(Clone)]
pub enum InputTarget {
    /// The Solfege Engine (`Command::Midi`).
    Engine(Sender<solfege_synth::engine::Command>),
    /// The open MIDI output device.
    Device,
}

impl MidiIn {
    /// Open input `name`. Channel messages play on `target`; with
    /// `channel`, all of them move to that channel (0-based).
    pub fn open(name: &str, target: InputTarget, out: Option<&MidiOut>, channel: Option<u8>) -> Result<Self, String> {
        let mut m = MidiInput::new(CLIENT).map_err(|e| e.to_string())?;
        m.ignore(Ignore::TimeAndActiveSense);
        let port = m.ports().into_iter().find(|p| m.port_name(p).is_ok_and(|n| n == name)).ok_or_else(|| format!("{name}: ไม่พบอุปกรณ์"))?;
        let thru = out.and_then(MidiOut::sender);
        let conn = m
            .connect(
                &port,
                "in",
                move |_, bytes, _| {
                    let Some((msg, len)) = channel_message(bytes, channel) else { return };
                    match &target {
                        InputTarget::Engine(tx) => {
                            let _ = tx.try_send(solfege_synth::engine::Command::Midi(msg));
                        }
                        InputTarget::Device => {
                            if let Some(tx) = &thru {
                                let _ = tx.try_send(OutCmd::Thru(msg, len));
                            }
                        }
                    }
                },
                (),
            )
            .map_err(|e| format!("{name}: {e}"))?;
        Ok(Self { name: name.to_string(), _conn: conn })
    }
}

/// A channel message from raw input bytes, optionally moved to `channel`.
pub fn channel_message(bytes: &[u8], channel: Option<u8>) -> Option<([u8; 3], u8)> {
    let &status = bytes.first()?;
    let len = match status & 0xF0 {
        0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0 => 3,
        0xC0 | 0xD0 => 2,
        _ => return None,
    };
    if bytes.len() < len {
        return None;
    }
    let mut msg = [0u8; 3];
    msg[..len].copy_from_slice(&bytes[..len]);
    if let Some(c) = channel {
        msg[0] = (msg[0] & 0xF0) | (c & 0x0F);
    }
    Some((msg, len as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(f: &mut OutFilter, msg: [u8; 3], key: i32) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        f.player(msg, if msg[0] & 0xE0 == 0xC0 { 2 } else { 3 }, key, &mut out);
        out
    }

    #[test]
    fn notes_follow_the_key_and_end_where_they_started() {
        let mut f = OutFilter::default();
        assert_eq!(run(&mut f, [0x90, 60, 100], 2), [vec![0x90, 62, 100]]);
        // The key changes while the note sounds: its note-off still ends 62.
        assert_eq!(run(&mut f, [0x80, 60, 0], -3), [vec![0x80, 62, 0]]);
        // Drums never move.
        assert_eq!(run(&mut f, [0x99, 36, 90], 5), [vec![0x99, 36, 90]]);
        // Out of range: dropped, and so is its note-off.
        assert!(run(&mut f, [0x90, 126, 90], 5).is_empty());
        assert!(run(&mut f, [0x90, 126, 0], 5).is_empty());
        // Programs and controllers pass as they are.
        assert_eq!(run(&mut f, [0xC1, 24, 0], 2), [vec![0xC1, 24]]);
        assert_eq!(run(&mut f, [0xB1, 7, 90], 2), [vec![0xB1, 7, 90]]);
    }

    #[test]
    fn blocked_channels_go_quiet() {
        let mut f = OutFilter::default();
        let mut out = Vec::new();
        run(&mut f, [0x98, 64, 100], 0);
        f.block(1 << 8, &mut out);
        assert_eq!(out, [vec![0x88, 64, 0]], "the held note ends");
        assert!(run(&mut f, [0x98, 65, 100], 0).is_empty(), "no new notes");
        assert!(run(&mut f, [0x88, 65, 0], 0).is_empty());
        out.clear();
        f.block(0, &mut out);
        assert!(out.is_empty());
        assert_eq!(run(&mut f, [0x98, 65, 100], 0), [vec![0x98, 65, 100]]);
    }

    #[test]
    fn decoded_sysex_becomes_real_sysex() {
        let mut f = OutFilter::default();
        assert_eq!(run(&mut f, [0xF0, SYS_RESET, 0], 0), [GM_ON.to_vec()]);
        // Channel 11 becomes a drum part (GS block 0x1A... checksum).
        let out = run(&mut f, [0xF0, SYS_DRUM_PART, 10 | 0x10], 0);
        assert_eq!(out[0], [0xF0, 0x41, 0x10, 0x42, 0x12, 0x40, 0x1A, 0x15, 0x01, 0x10, 0xF7]);
        assert_eq!(out[1], [0xF0, 0x43, 0x10, 0x4C, 0x08, 10, 0x07, 0x01, 0xF7]);
        // ... so its notes are no longer moved to the key.
        assert_eq!(run(&mut f, [0x9A, 38, 100], 4), [vec![0x9A, 38, 100]]);
    }

    #[test]
    fn reset_ends_every_note() {
        let mut f = OutFilter::default();
        run(&mut f, [0x90, 60, 100], 0);
        run(&mut f, [0x93, 70, 100], 1);
        let mut out = Vec::new();
        f.reset(true, &mut out);
        assert_eq!(out[0], GM_ON);
        assert!(out.contains(&vec![0x80, 60, 0]) && out.contains(&vec![0x83, 71, 0]));
        assert!(out.contains(&vec![0xBF, 123, 0]));
    }

    #[test]
    fn master_volume_is_14_bit() {
        assert_eq!(master_volume(16383), [0xF0, 0x7F, 0x7F, 0x04, 0x01, 0x7F, 0x7F, 0xF7]);
        assert_eq!(master_volume(0x2000), [0xF0, 0x7F, 0x7F, 0x04, 0x01, 0x00, 0x40, 0xF7]);
        assert_eq!(master_volume(99_999), master_volume(16383));
        let mut f = OutFilter::default();
        run(&mut f, [0xF0, SYS_RESET, 0], 0);
        assert!(f.was_reset, "the volume is sent again after a reset");
    }

    #[test]
    fn input_messages_can_move_channel() {
        assert_eq!(channel_message(&[0x90, 60, 100], None), Some(([0x90, 60, 100], 3)));
        assert_eq!(channel_message(&[0x91, 60, 100], Some(15)), Some(([0x9F, 60, 100], 3)));
        assert_eq!(channel_message(&[0xC0, 5], Some(3)), Some(([0xC3, 5, 0], 2)));
        assert_eq!(channel_message(&[0xF8], None), None);
        assert_eq!(channel_message(&[0x90, 60], None), None);
    }
}
