//! Just enough Standard MIDI File reading for karaoke timing: the time
//! division, the tempo map and the song length. Accepts the RIFF `RMID`
//! wrapper as well.

use std::path::Path;

use crate::Error;

#[derive(Clone, Debug, PartialEq)]
pub struct MidiInfo {
    pub format: u16,
    pub tracks: u16,
    /// Ticks per quarter note. For SMPTE files this is a nominal 96 and
    /// `smpte` is set.
    pub ppq: u16,
    /// Frames per second and ticks per frame for SMPTE-timed files.
    pub smpte: Option<(f64, u16)>,
    pub tempo: TempoMap,
    /// Tick of the last event in any track.
    pub end_tick: u32,
    /// Header was `Lock` instead of `MThd` (a copy-protection trick used
    /// by some karaoke packs; the rest of the file is a normal SMF).
    pub locked: bool,
    /// Time signatures, by tick (empty = 4/4 throughout).
    pub meters: Vec<Meter>,
    /// Text meta events (text, track name, lyric), by tick: the lyrics of
    /// `.kar` and other karaoke MIDI files.
    pub texts: Vec<MidiText>,
}

/// A time signature from tick `tick`: `beats` per bar, each a
/// `1 / 2^unit` note (4/4 is beats 4, unit 2; 6/8 is beats 6, unit 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Meter {
    pub tick: u32,
    pub beats: u8,
    pub unit: u8,
}

impl Meter {
    /// Length of one beat in quarter notes.
    pub fn beat_quarters(&self) -> f64 {
        4.0 / f64::from(1u32 << self.unit.min(6))
    }
}

/// A text meta event, its bytes as found (the encoding is not declared).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidiText {
    pub tick: u32,
    /// `0x01` text, `0x03` track name, `0x05` lyric.
    pub kind: u8,
    /// Track the event came from.
    pub track: u16,
    pub bytes: Vec<u8>,
}

/// Tempo changes as `(tick, microseconds per quarter note)`.
#[derive(Clone, Debug, PartialEq)]
pub struct TempoMap {
    ppq: u16,
    smpte: Option<(f64, u16)>,
    changes: Vec<(u32, u32)>,
}

impl TempoMap {
    pub fn new(ppq: u16, mut changes: Vec<(u32, u32)>) -> Self {
        changes.sort_by_key(|c| c.0);
        Self { ppq: ppq.max(1), smpte: None, changes }
    }

    pub fn changes(&self) -> &[(u32, u32)] {
        &self.changes
    }

    /// Initial tempo in BPM (120 when the file sets none at tick 0).
    pub fn initial_bpm(&self) -> f64 {
        match self.changes.first() {
            Some(&(0, us)) => 60_000_000.0 / us as f64,
            _ => 120.0,
        }
    }

    pub fn seconds(&self, tick: u32) -> f64 {
        if let Some((fps, tpf)) = self.smpte {
            return tick as f64 / (fps * tpf.max(1) as f64);
        }
        let ppq = self.ppq as f64;
        let (mut last_tick, mut us, mut secs) = (0u32, 500_000u32, 0.0f64);
        for &(t, tempo) in &self.changes {
            if t >= tick {
                break;
            }
            secs += (t - last_tick) as f64 * us as f64 / 1e6 / ppq;
            last_tick = t;
            us = tempo;
        }
        secs + (tick - last_tick) as f64 * us as f64 / 1e6 / ppq
    }
}

fn be16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}

fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn vlq(b: &[u8], pos: &mut usize) -> Option<u32> {
    let mut v = 0u32;
    for _ in 0..4 {
        let c = *b.get(*pos)?;
        *pos += 1;
        v = (v << 7) | (c & 0x7F) as u32;
        if c & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

fn unwrap_rmid(b: &[u8]) -> &[u8] {
    if b.len() >= 20 && &b[0..4] == b"RIFF" && &b[8..12] == b"RMID" {
        let mut pos = 12;
        while pos + 8 <= b.len() {
            let size = u32::from_le_bytes([b[pos + 4], b[pos + 5], b[pos + 6], b[pos + 7]]) as usize;
            if &b[pos..pos + 4] == b"data" {
                return &b[pos + 8..(pos + 8 + size).min(b.len())];
            }
            pos += 8 + size + (size & 1);
        }
    }
    b
}

impl MidiInfo {
    pub fn parse(bytes: &[u8]) -> crate::Result<Self> {
        let bad = |reason: &str| Error::InvalidMidi { path: None, reason: reason.into() };
        let b = unwrap_rmid(bytes);
        let locked = b.len() >= 4 && &b[0..4] == b"Lock";
        if b.len() < 14 || (&b[0..4] != b"MThd" && !locked) {
            return Err(bad("missing MThd header"));
        }
        let hlen = be32(b, 4) as usize;
        let format = be16(b, 8);
        let tracks = be16(b, 10);
        let division = be16(b, 12);
        let (ppq, smpte) = if division & 0x8000 != 0 {
            let fps = -((division >> 8) as i8) as f64;
            let fps = if fps == 29.0 { 29.97 } else { fps };
            (96, Some((fps, division & 0xFF)))
        } else {
            (division.max(1), None)
        };

        let mut changes = Vec::new();
        let mut meters = Vec::new();
        let mut texts = Vec::new();
        let mut track_no = 0u16;
        let mut end_tick = 0u32;
        let mut pos = 8 + hlen;
        while pos + 8 <= b.len() {
            let len = be32(b, pos + 4) as usize;
            let start = pos + 8;
            let end = start.saturating_add(len).min(b.len());
            let is_track = &b[pos..pos + 4] == b"MTrk";
            pos = start.saturating_add(len);
            if !is_track {
                continue;
            }
            track_no += 1;
            let t = &b[start..end];
            let (mut p, mut tick, mut running) = (0usize, 0u32, 0u8);
            while p < t.len() {
                let Some(delta) = vlq(t, &mut p) else { break };
                tick = tick.saturating_add(delta);
                let Some(&first) = t.get(p) else { break };
                let status = if first & 0x80 != 0 {
                    p += 1;
                    first
                } else if running != 0 {
                    running
                } else {
                    break;
                };
                match status {
                    0xFF => {
                        let Some(&kind) = t.get(p) else { break };
                        p += 1;
                        let Some(l) = vlq(t, &mut p) else { break };
                        let data = &t[p.min(t.len())..(p + l as usize).min(t.len())];
                        if kind == 0x51 && data.len() >= 3 {
                            let us = (data[0] as u32) << 16 | (data[1] as u32) << 8 | data[2] as u32;
                            if us > 0 {
                                changes.push((tick, us));
                            }
                        }
                        if kind == 0x58 && data.len() >= 2 && data[0] > 0 {
                            meters.push(Meter { tick, beats: data[0], unit: data[1].min(6) });
                        }
                        if matches!(kind, 0x01 | 0x03 | 0x05) && !data.is_empty() {
                            texts.push(MidiText { tick, kind, track: track_no - 1, bytes: data.to_vec() });
                        }
                        p += l as usize;
                        running = 0;
                        if kind == 0x2F {
                            break;
                        }
                    }
                    0xF0 | 0xF7 => {
                        let Some(l) = vlq(t, &mut p) else { break };
                        p += l as usize;
                        running = 0;
                    }
                    0xF1..=0xFE => {}
                    _ => {
                        running = status;
                        p += if matches!(status & 0xF0, 0xC0 | 0xD0) { 1 } else { 2 };
                    }
                }
                end_tick = end_tick.max(tick);
            }
        }
        let mut tempo = TempoMap::new(ppq, changes);
        tempo.smpte = smpte;
        // By tick; of two at the same tick (one per track) the last wins.
        meters.sort_by_key(|m| m.tick);
        meters.dedup_by(|later, earlier| {
            let same = later.tick == earlier.tick;
            if same {
                *earlier = *later;
            }
            same
        });
        texts.sort_by_key(|t| t.tick);
        Ok(Self { format, tracks, ppq, smpte, tempo, end_tick, locked, meters, texts })
    }

    pub fn load(path: &Path) -> crate::Result<Self> {
        Self::parse(&crate::error::read(path)?).map_err(|e| match e {
            Error::InvalidMidi { reason, .. } => Error::InvalidMidi { path: Some(path.to_path_buf()), reason },
            other => other,
        })
    }

    pub fn duration(&self) -> f64 {
        self.tempo.seconds(self.end_tick)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tempo_map_seconds() {
        // 120 BPM, then 60 BPM from tick 960 (ppq 480).
        let t = TempoMap::new(480, vec![(0, 500_000), (960, 1_000_000)]);
        assert!((t.seconds(960) - 1.0).abs() < 1e-9);
        assert!((t.seconds(1440) - 2.0).abs() < 1e-9);
        assert!((t.initial_bpm() - 120.0).abs() < 1e-9);
    }

    #[test]
    fn accepts_lock_header() {
        let mut f = b"Lock".to_vec();
        f.extend([0, 0, 0, 6, 0, 0, 0, 1, 0x01, 0xE0]);
        f.extend(b"MTrk");
        f.extend([0, 0, 0, 4, 0, 0xFF, 0x2F, 0]);
        let m = MidiInfo::parse(&f).unwrap();
        assert!(m.locked);
        assert_eq!(m.ppq, 480);
    }

    #[test]
    fn reads_time_signatures_and_texts() {
        let mut f = b"MThd".to_vec();
        f.extend([0, 0, 0, 6, 0, 0, 0, 1, 0x01, 0xE0]);
        let body: Vec<u8> = [
            &[0x00, 0xFF, 0x58, 0x04, 0x03, 0x02, 0x18, 0x08][..], // 3/4 at 0
            &[0x00, 0xFF, 0x03, 0x04, b'S', b'o', b'n', b'g'],
            &[0x83, 0x60, 0xFF, 0x05, 0x02, b'L', b'a'], // lyric at 480
            &[0x8B, 0x20, 0xFF, 0x58, 0x04, 0x06, 0x03, 0x18, 0x08], // 6/8 at 1920
            &[0x00, 0xFF, 0x2F, 0x00],
        ]
        .concat();
        f.extend(b"MTrk");
        f.extend((body.len() as u32).to_be_bytes());
        f.extend(body);
        let m = MidiInfo::parse(&f).unwrap();
        assert_eq!(m.meters, [Meter { tick: 0, beats: 3, unit: 2 }, Meter { tick: 1920, beats: 6, unit: 3 }]);
        assert_eq!(m.meters[1].beat_quarters(), 0.5);
        assert_eq!(m.texts.len(), 2);
        assert_eq!((m.texts[1].tick, m.texts[1].kind, m.texts[1].bytes.as_slice()), (480, 0x05, &b"La"[..]));
    }

    #[test]
    fn rejects_non_midi() {
        assert!(MidiInfo::parse(b"hello world, not midi").is_err());
    }
}
