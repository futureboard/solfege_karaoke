//! Standard MIDI File reader (format 0/1/2, PPQ or SMPTE timing, RIFF RMID
//! wrapper). All tracks are merged into one time-sorted list of channel
//! messages with absolute times in seconds, tempo map already applied.

use std::path::Path;

use anyhow::{Context, Result, bail};

#[derive(Clone, Copy, Debug)]
pub struct SongEvent {
    pub time: f64,
    pub msg: [u8; 3],
    pub len: u8,
}

pub struct Song {
    pub name: String,
    pub format: u16,
    pub tracks: usize,
    pub events: Vec<SongEvent>,
    pub duration: f64,
    pub bpm: f64,
    /// Bit per MIDI channel that carries at least one note.
    pub channels_used: u16,
}

/// Internal 3-byte form of the SysEx messages the engine understands:
/// `[0xF0, kind, arg]`.
pub const SYS_RESET: u8 = 1;
/// `arg` = channel | 0x10 when the channel becomes a rhythm (drum) part.
pub const SYS_DRUM_PART: u8 = 2;

/// GS block number (part) to MIDI channel: block 0 is part 10.
fn gs_block_channel(block: u8) -> u8 {
    match block {
        0 => 9,
        1..=9 => block - 1,
        _ => block,
    }
}

/// Decode GM/GM2/GS/XG resets and GS/XG rhythm-part assignments.
pub fn parse_sysex(b: &[u8]) -> Option<[u8; 3]> {
    let b = b.strip_suffix(&[0xF7]).unwrap_or(b);
    match b {
        // Universal non-realtime GM System On / Off, GM2 On.
        [0xF0, 0x7E, _, 0x09, 0x01..=0x03, ..] => Some([0xF0, SYS_RESET, 0]),
        // Roland GS: F0 41 dev 42 12 <addr hi mid lo> <data> <checksum>.
        [0xF0, 0x41, _, 0x42, 0x12, a_hi, a_mid, a_lo, data, ..] => match (*a_hi, *a_mid, *a_lo) {
            (0x40, 0x00, 0x7F) | (0x00, 0x00, 0x7F) => Some([0xF0, SYS_RESET, 0]),
            (0x40, m, 0x15) if m & 0xF0 == 0x10 => {
                let ch = gs_block_channel(m & 0x0F);
                Some([0xF0, SYS_DRUM_PART, ch | if *data != 0 { 0x10 } else { 0 }])
            }
            _ => None,
        },
        // Yamaha XG: F0 43 1n 4C <addr hi mid lo> <data>.
        [0xF0, 0x43, dev, 0x4C, a_hi, a_mid, a_lo, data, ..] if dev & 0xF0 == 0x10 => {
            match (*a_hi, *a_mid, *a_lo) {
                (0x00, 0x00, 0x7E) | (0x00, 0x00, 0x7F) => Some([0xF0, SYS_RESET, 0]),
                (0x08, part, 0x07) if part < 16 => Some([0xF0, SYS_DRUM_PART, part | if *data != 0 { 0x10 } else { 0 }]),
                _ => None,
            }
        }
        _ => None,
    }
}

pub fn is_midi(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    matches!(ext.as_str(), "mid" | "midi" | "smf" | "kar" | "rmi")
}

pub fn load(path: &Path) -> Result<Song> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    parse(&bytes, &stem).with_context(|| format!("parse {}", path.display()))
}

fn be16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}

fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// RMID files wrap a complete SMF inside RIFF `data`.
fn unwrap_rmid(b: &[u8]) -> &[u8] {
    if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"RMID" {
        let mut pos = 12;
        while pos + 8 <= b.len() {
            let size = u32::from_le_bytes([b[pos + 4], b[pos + 5], b[pos + 6], b[pos + 7]]) as usize;
            let body = &b[pos + 8..(pos + 8).saturating_add(size).min(b.len())];
            if &b[pos..pos + 4] == b"data" {
                return body;
            }
            pos = pos + 8 + size + (size & 1);
        }
    }
    b
}

struct Raw {
    tick: u64,
    prio: u8,
    order: usize,
    msg: [u8; 3],
    len: u8,
}

fn read_vlq(b: &[u8], pos: &mut usize) -> Result<u32> {
    let mut v: u32 = 0;
    for _ in 0..4 {
        let c = *b.get(*pos).context("truncated variable-length value")?;
        *pos += 1;
        v = (v << 7) | (c & 0x7F) as u32;
        if c & 0x80 == 0 {
            return Ok(v);
        }
    }
    bail!("variable-length value too long")
}

pub fn parse(b: &[u8], fallback_name: &str) -> Result<Song> {
    let b = unwrap_rmid(b);
    // Some karaoke packs replace `MThd` with `Lock`; the rest is a normal SMF.
    if b.len() < 14 || (&b[0..4] != b"MThd" && &b[0..4] != b"Lock") {
        bail!("not a Standard MIDI File");
    }
    let hlen = be32(b, 4) as usize;
    let format = be16(b, 8);
    let ntracks = be16(b, 10) as usize;
    let division = be16(b, 12);
    if hlen < 6 {
        bail!("bad MThd length");
    }

    let mut raws: Vec<Raw> = Vec::new();
    let mut tempos: Vec<(u64, u32)> = Vec::new();
    let mut name = String::new();
    let mut end_tick = 0u64;
    let mut pos = 8 + hlen;
    let mut track = 0usize;
    let mut order = 0usize;

    while pos + 8 <= b.len() && track < ntracks.max(1) * 4 {
        let id = &b[pos..pos + 4];
        let len = be32(b, pos + 4) as usize;
        let start = pos + 8;
        let end = start.saturating_add(len).min(b.len());
        pos = start.saturating_add(len);
        if id != b"MTrk" {
            continue;
        }
        let t = &b[start..end];
        let mut p = 0usize;
        let mut tick = 0u64;
        let mut running = 0u8;
        while p < t.len() {
            let Ok(delta) = read_vlq(t, &mut p) else { break };
            tick += delta as u64;
            let Some(&first) = t.get(p) else { break };
            let status = if first & 0x80 != 0 {
                p += 1;
                first
            } else if running != 0 {
                running
            } else {
                break; // data byte with no running status: corrupt track
            };
            match status {
                0xFF => {
                    let Some(&kind) = t.get(p) else { break };
                    p += 1;
                    let Ok(l) = read_vlq(t, &mut p) else { break };
                    let l = l as usize;
                    let data = &t[p.min(t.len())..(p + l).min(t.len())];
                    p += l;
                    match kind {
                        0x51 if data.len() >= 3 => {
                            let us = (data[0] as u32) << 16 | (data[1] as u32) << 8 | data[2] as u32;
                            if us > 0 {
                                tempos.push((tick, us));
                            }
                        }
                        0x03 if name.is_empty() && track == 0 => {
                            name = String::from_utf8_lossy(data).trim().to_string();
                        }
                        0x2F => {
                            end_tick = end_tick.max(tick);
                            break;
                        }
                        _ => {}
                    }
                    running = 0;
                }
                0xF0 | 0xF7 => {
                    let Ok(l) = read_vlq(t, &mut p) else { break };
                    let l = l as usize;
                    if status == 0xF0 {
                        let mut full = vec![0xF0];
                        full.extend_from_slice(&t[p.min(t.len())..(p + l).min(t.len())]);
                        if let Some(msg) = parse_sysex(&full) {
                            raws.push(Raw { tick, prio: 0, order, msg, len: 3 });
                            order += 1;
                        }
                    }
                    p += l;
                    running = 0;
                }
                0xF1..=0xFE => {
                    // System common/realtime inside a file: skip the data bytes.
                    p += match status {
                        0xF2 => 2,
                        0xF1 | 0xF3 => 1,
                        _ => 0,
                    };
                }
                _ => {
                    running = status;
                    let n = if matches!(status & 0xF0, 0xC0 | 0xD0) { 1 } else { 2 };
                    if p + n > t.len() {
                        break;
                    }
                    let mut msg = [status, 0, 0];
                    msg[1..1 + n].copy_from_slice(&t[p..p + n]);
                    p += n;
                    let kind = status & 0xF0;
                    let prio = match kind {
                        0x80 => 1,
                        0x90 if msg[2] == 0 => 1,
                        0x90 => 2,
                        _ => 0,
                    };
                    raws.push(Raw { tick, prio, order, msg, len: 1 + n as u8 });
                    order += 1;
                    end_tick = end_tick.max(tick);
                }
            }
        }
        track += 1;
    }
    if track == 0 {
        bail!("no MTrk chunks");
    }

    // Controllers/programs before note-offs before note-ons at equal ticks.
    raws.sort_by_key(|r| (r.tick, r.prio, r.order));
    tempos.sort_by_key(|t| t.0);

    let smpte = division & 0x8000 != 0;
    let (ppq, smpte_tick) = if smpte {
        let fps = -((division >> 8) as i8) as f64;
        let fps = if fps == 29.0 { 29.97 } else { fps };
        let tpf = (division & 0xFF).max(1) as f64;
        (1.0, 1.0 / (fps * tpf))
    } else {
        (division.max(1) as f64, 0.0)
    };

    // Walk ticks in order, accumulating seconds across tempo changes.
    let mut tempo_us = 500_000u32;
    let mut ti = 0usize;
    let mut last_tick = 0u64;
    let mut secs = 0.0f64;
    let advance = |to: u64, tempo_us: u32, last_tick: &mut u64, secs: &mut f64| {
        let dt = (to - *last_tick) as f64;
        *secs += if smpte { dt * smpte_tick } else { dt * tempo_us as f64 / 1_000_000.0 / ppq };
        *last_tick = to;
    };
    let initial_bpm = tempos.first().filter(|t| t.0 == 0).map(|t| 60_000_000.0 / t.1 as f64).unwrap_or(120.0);
    let mut events = Vec::with_capacity(raws.len());
    let mut channels_used = 0u16;
    for r in &raws {
        while ti < tempos.len() && tempos[ti].0 <= r.tick {
            advance(tempos[ti].0, tempo_us, &mut last_tick, &mut secs);
            tempo_us = tempos[ti].1;
            ti += 1;
        }
        advance(r.tick, tempo_us, &mut last_tick, &mut secs);
        if r.msg[0] & 0xF0 == 0x90 && r.msg[2] > 0 {
            channels_used |= 1 << (r.msg[0] & 0x0F);
        }
        events.push(SongEvent { time: secs, msg: r.msg, len: r.len });
    }
    while ti < tempos.len() && tempos[ti].0 <= end_tick {
        advance(tempos[ti].0, tempo_us, &mut last_tick, &mut secs);
        tempo_us = tempos[ti].1;
        ti += 1;
    }
    advance(end_tick.max(last_tick), tempo_us, &mut last_tick, &mut secs);

    if events.is_empty() {
        bail!("file contains no channel events");
    }
    Ok(Song {
        name: if name.is_empty() { fallback_name.to_string() } else { name },
        format,
        tracks: track,
        duration: secs.max(events.last().map(|e| e.time).unwrap_or(0.0)),
        events,
        bpm: initial_bpm,
        channels_used,
    })
}
