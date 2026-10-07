//! MIDI I/O through midir (WinMM on Windows, ALSA sequencer on Linux).
//! Any number of inputs can be open at once; one output receives
//! computer-keyboard notes and, when thru is enabled, everything from the inputs.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use crossbeam_channel::{Receiver, Sender};
use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

use crate::engine::Command;

const CLIENT: &str = "simpletui";

pub struct MidiEvent {
    pub source: String,
    pub msg: [u8; 3],
    pub len: usize,
}

type OutSlot = Arc<Mutex<Option<(String, MidiOutputConnection)>>>;

pub struct Midi {
    inputs: Vec<(String, MidiInputConnection<()>)>,
    out: OutSlot,
    pub thru: Arc<AtomicBool>,
    engine: Sender<Command>,
    ui: Sender<MidiEvent>,
    #[cfg(unix)]
    virtual_in: Option<MidiInputConnection<()>>,
}

pub fn input_ports() -> Vec<String> {
    let Ok(m) = MidiInput::new(CLIENT) else { return Vec::new() };
    m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect()
}

pub fn output_ports() -> Vec<String> {
    let Ok(m) = MidiOutput::new(CLIENT) else { return Vec::new() };
    m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect()
}

/// Message length for a status byte; 0 means "not a channel message we handle".
fn channel_len(status: u8) -> usize {
    match status & 0xF0 {
        0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0 => 3,
        0xC0 | 0xD0 => 2,
        _ => 0,
    }
}

fn make_callback(
    source: String,
    engine: Sender<Command>,
    ui: Sender<MidiEvent>,
    out: OutSlot,
    thru: Arc<AtomicBool>,
) -> impl FnMut(u64, &[u8], &mut ()) + Send + 'static {
    move |_ts, bytes, _| {
        let Some(&status) = bytes.first() else { return };
        if status == 0xF0 {
            if let Some(msg) = crate::smf::parse_sysex(bytes) {
                let _ = engine.try_send(Command::Midi(msg));
                let _ = ui.try_send(MidiEvent { source: source.clone(), msg, len: 3 });
            }
            return;
        }
        let len = channel_len(status);
        if len == 0 || bytes.len() < len {
            return;
        }
        let mut msg = [0u8; 3];
        msg[..len].copy_from_slice(&bytes[..len]);
        let _ = engine.try_send(Command::Midi(msg));
        let _ = ui.try_send(MidiEvent { source: source.clone(), msg, len });
        if thru.load(Ordering::Relaxed)
            && let Ok(mut guard) = out.lock()
            && let Some((_, conn)) = guard.as_mut()
        {
            let _ = conn.send(&msg[..len]);
        }
    }
}

impl Midi {
    pub fn new(engine: Sender<Command>, ui: Sender<MidiEvent>) -> Self {
        Self {
            inputs: Vec::new(),
            out: Arc::new(Mutex::new(None)),
            thru: Arc::new(AtomicBool::new(false)),
            engine,
            ui,
            #[cfg(unix)]
            virtual_in: None,
        }
    }

    pub fn connected_inputs(&self) -> Vec<String> {
        #[allow(unused_mut)]
        let mut v: Vec<String> = self.inputs.iter().map(|(n, _)| n.clone()).collect();
        #[cfg(unix)]
        if self.virtual_in.is_some() {
            v.push(format!("{CLIENT}:in (virtual)"));
        }
        v
    }

    pub fn is_input_open(&self, name: &str) -> bool {
        self.inputs.iter().any(|(n, _)| n == name)
    }

    pub fn output_name(&self) -> Option<String> {
        self.out.lock().ok()?.as_ref().map(|(n, _)| n.clone())
    }

    pub fn connect_input(&mut self, want: &str) -> Result<String> {
        let mut m = MidiInput::new(CLIENT)?;
        m.ignore(Ignore::TimeAndActiveSense);
        let ports = m.ports();
        let port = ports
            .iter()
            .find(|p| m.port_name(p).is_ok_and(|n| n == want || n.to_lowercase().contains(&want.to_lowercase())))
            .ok_or_else(|| anyhow!("no MIDI input matching '{want}'"))?;
        let name = m.port_name(port)?;
        if self.is_input_open(&name) {
            return Ok(name);
        }
        let cb = make_callback(name.clone(), self.engine.clone(), self.ui.clone(), self.out.clone(), self.thru.clone());
        let conn = m.connect(port, "simpletui-in", cb, ()).map_err(|e| anyhow!("connect '{name}': {e}"))?;
        self.inputs.push((name.clone(), conn));
        Ok(name)
    }

    pub fn disconnect_input(&mut self, name: &str) {
        if let Some(i) = self.inputs.iter().position(|(n, _)| n == name) {
            let (_, conn) = self.inputs.remove(i);
            conn.close();
        }
    }

    /// Toggle an input; returns true if it is now open.
    pub fn toggle_input(&mut self, name: &str) -> Result<bool> {
        if self.is_input_open(name) {
            self.disconnect_input(name);
            Ok(false)
        } else {
            self.connect_input(name)?;
            Ok(true)
        }
    }

    pub fn set_output(&mut self, want: Option<&str>) -> Result<Option<String>> {
        let mut guard = self.out.lock().map_err(|_| anyhow!("MIDI output lock poisoned"))?;
        if let Some((_, conn)) = guard.take() {
            conn.close();
        }
        let Some(want) = want else { return Ok(None) };
        let m = MidiOutput::new(CLIENT)?;
        let ports = m.ports();
        let port = ports
            .iter()
            .find(|p| m.port_name(p).is_ok_and(|n| n == want || n.to_lowercase().contains(&want.to_lowercase())))
            .ok_or_else(|| anyhow!("no MIDI output matching '{want}'"))?;
        let name = m.port_name(port)?;
        let conn = m.connect(port, "simpletui-out").map_err(|e| anyhow!("connect '{name}': {e}"))?;
        *guard = Some((name.clone(), conn));
        Ok(Some(name))
    }

    /// Drain player events the audio thread queued for the MIDI output.
    pub fn spawn_player_forwarder(&self, rx: Receiver<([u8; 3], u8)>) {
        let out = self.out.clone();
        std::thread::Builder::new()
            .name("midi-out".into())
            .spawn(move || {
                while let Ok((msg, len)) = rx.recv() {
                    if let Ok(mut guard) = out.lock()
                        && let Some((_, conn)) = guard.as_mut()
                    {
                        let _ = conn.send(&msg[..len as usize]);
                    }
                }
            })
            .expect("spawn midi-out thread");
    }

    pub fn send(&self, msg: &[u8]) {
        if let Ok(mut guard) = self.out.lock()
            && let Some((_, conn)) = guard.as_mut()
        {
            let _ = conn.send(msg);
        }
    }

    /// Linux/ALSA: expose virtual ports other apps can connect to.
    #[cfg(unix)]
    pub fn create_virtual_ports(&mut self) -> Result<()> {
        use midir::os::unix::{VirtualInput, VirtualOutput};
        if self.virtual_in.is_none() {
            let mut m = MidiInput::new(CLIENT)?;
            m.ignore(Ignore::TimeAndActiveSense);
            let cb = make_callback(
                "virtual".into(),
                self.engine.clone(),
                self.ui.clone(),
                self.out.clone(),
                self.thru.clone(),
            );
            let conn = m.create_virtual("in", cb, ()).map_err(|e| anyhow!("virtual input: {e}"))?;
            self.virtual_in = Some(conn);
        }
        let mut guard = self.out.lock().map_err(|_| anyhow!("MIDI output lock poisoned"))?;
        if guard.is_none() {
            let m = MidiOutput::new(CLIENT)?;
            let conn = m.create_virtual("out").map_err(|e| anyhow!("virtual output: {e}"))?;
            *guard = Some((format!("{CLIENT}:out (virtual)"), conn));
        }
        Ok(())
    }
}
