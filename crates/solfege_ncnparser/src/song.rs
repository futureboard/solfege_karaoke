//! A fully timed karaoke song: lyric lines broken into Thai display
//! clusters, each with the MIDI tick at which it should be highlighted.

use std::path::PathBuf;

use crate::cp874;
use crate::cursor::Cursor;
use crate::lyrics::Lyrics;
use crate::midi::{MidiInfo, TempoMap};

/// How well the cursor length matched the lyric text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    Exact,
    /// The cursor has entries past the end of the text (ignored).
    CursorLonger { extra: usize },
    /// The text outlasts the cursor; the tail reuses the last time.
    CursorShorter { missing: usize },
}

/// A base letter plus any combining marks drawn on it, the smallest unit a
/// karaoke display highlights.
#[derive(Clone, Debug, PartialEq)]
pub struct Cluster {
    pub text: String,
    /// Highlight time (MIDI ticks), never earlier than the previous cluster.
    pub tick: u32,
    /// Cursor value(s) as stored in the file for this cluster's first byte;
    /// `None` when the cursor ran out.
    pub raw: Option<u16>,
    /// Byte offset of the cluster inside the line.
    pub byte: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimedLine {
    pub text: String,
    pub clusters: Vec<Cluster>,
    /// First highlight of the line (ticks).
    pub start: u32,
    /// Time of the line-break entry: the line is finished.
    pub end: u32,
}

impl TimedLine {
    pub fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// Position inside the song at a given time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    /// Line being sung (or the next one during a gap).
    pub line: usize,
    /// Clusters of that line already highlighted.
    pub clusters: usize,
}

#[derive(Clone, Debug)]
pub struct NcnSong {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub key: Option<String>,
    pub ppq: u16,
    pub lines: Vec<TimedLine>,
    pub alignment: Alignment,
    /// Cursor entries that stepped backwards (clamped).
    pub backsteps: usize,
    /// Cursor entries discarded as garbage (past the song end, or isolated
    /// spikes / drops); their characters reuse the previous time.
    pub rejected: usize,
    pub tempo: TempoMap,
    pub midi_end_tick: u32,
    pub midi_path: Option<PathBuf>,
    pub lyrics_path: Option<PathBuf>,
    pub cursor_path: Option<PathBuf>,
}

/// Cursor units in a quarter note.
const Q: u32 = crate::cursor::CURSOR_RESOLUTION;

/// Drop cursor values that cannot be real timing: anything past the end of
/// the MIDI (some files carry an uninitialised buffer tail), and lone
/// spikes or dips of more than four bars that the next entry contradicts.
/// Long notes, instrumental breaks and small backsteps are kept (backsteps
/// are clamped later).
fn sanitize(values: &[u16], midi: &MidiInfo) -> Vec<Option<u16>> {
    let limit = if midi.end_tick > 0 {
        (midi.end_tick as u64 * Q as u64 / midi.ppq.max(1) as u64 + 4 * Q as u64).min(u16::MAX as u64) as u32
    } else {
        u16::MAX as u32
    };
    let big = 16 * Q;
    let mut out = Vec::with_capacity(values.len());
    let mut last: Option<u32> = None;
    for (i, &v) in values.iter().enumerate() {
        let v = v as u32;
        let next = values.get(i + 1).map(|&w| w as u32).filter(|&w| w <= limit);
        let ok = v <= limit
            && match last {
                // Up a long way and straight back down: a spike.
                Some(l) if v > l + big => next.is_none_or(|w| w + big >= v),
                // Down a long way while the next entry is back up: a dip.
                Some(l) if v + big < l => next.is_some_and(|w| w + big < l),
                _ => true,
            };
        if ok {
            last = Some(last.map_or(v, |l| l.max(v)));
            out.push(Some(v as u16));
        } else {
            out.push(None);
        }
    }
    out
}

impl NcnSong {
    /// Combine parsed parts. `midi` supplies the resolution and tempo map.
    pub fn from_parts(id: impl Into<String>, lyrics: Lyrics, cursor: &Cursor, midi: &MidiInfo) -> Self {
        let ppq = midi.ppq;
        let expected = lyrics.cursor_len();
        let alignment = match cursor.len().cmp(&expected) {
            std::cmp::Ordering::Equal => Alignment::Exact,
            std::cmp::Ordering::Greater => Alignment::CursorLonger { extra: cursor.len() - expected },
            std::cmp::Ordering::Less => Alignment::CursorShorter { missing: expected - cursor.len() },
        };

        let accepted = sanitize(&cursor.values, midi);
        let rejected = accepted.iter().zip(&cursor.values).take(expected).filter(|(a, _)| a.is_none()).count();
        let mut k = 0usize;
        let mut last = 0u32;
        let mut next = |k: &mut usize| -> (u32, Option<u16>) {
            let raw = accepted.get(*k).copied().flatten();
            *k += 1;
            if let Some(v) = raw {
                last = last.max(Cursor::to_tick(v, ppq));
            }
            (last, raw)
        };

        let mut lines = Vec::with_capacity(lyrics.lines.len());
        for line in &lyrics.lines {
            let mut clusters: Vec<Cluster> = Vec::new();
            for (i, &b) in line.raw.iter().enumerate() {
                let (tick, raw) = next(&mut k);
                let ch = cp874::decode_byte(b);
                match clusters.last_mut() {
                    Some(c) if cp874::is_combining(ch) => c.text.push(ch),
                    _ => clusters.push(Cluster { text: ch.to_string(), tick, raw, byte: i }),
                }
            }
            let (end, _) = next(&mut k);
            let start = clusters.first().map(|c| c.tick).unwrap_or(end);
            lines.push(TimedLine { text: line.text.clone(), clusters, start, end });
        }

        Self {
            id: id.into(),
            title: lyrics.title,
            artist: lyrics.artist,
            key: lyrics.key,
            ppq,
            lines,
            alignment,
            backsteps: cursor.backsteps(),
            rejected,
            tempo: midi.tempo.clone(),
            midi_end_tick: midi.end_tick,
            midi_path: None,
            lyrics_path: None,
            cursor_path: None,
        }
    }

    pub fn seconds(&self, tick: u32) -> f64 {
        self.tempo.seconds(tick)
    }

    /// Length of the backing track in seconds.
    pub fn duration(&self) -> f64 {
        self.tempo.seconds(self.midi_end_tick)
    }

    /// Lines that contain singable text (not blank).
    pub fn sung_lines(&self) -> impl Iterator<Item = (usize, &TimedLine)> {
        self.lines.iter().enumerate().filter(|(_, l)| !l.is_blank())
    }

    /// Where the highlight is at `tick`: the last line that has started
    /// (or line 0 before the first one) and how much of it is sung.
    pub fn progress(&self, tick: u32) -> Progress {
        let line = self.lines.partition_point(|l| l.start <= tick).saturating_sub(1);
        let clusters = self
            .lines
            .get(line)
            .map(|l| l.clusters.partition_point(|c| c.tick <= tick))
            .unwrap_or(0);
        Progress { line, clusters }
    }

    /// LRC export with per-line timestamps (and `<mm:ss.xx>` word times when
    /// `enhanced`), handy for checking timing in other players.
    pub fn to_lrc(&self, enhanced: bool) -> String {
        let stamp = |secs: f64| {
            let cs = (secs * 100.0).round() as u64;
            format!("{:02}:{:02}.{:02}", cs / 6000, (cs / 100) % 60, cs % 100)
        };
        let mut out = format!("[ti:{}]\n[ar:{}]\n", self.title, self.artist);
        if let Some(k) = &self.key {
            out.push_str(&format!("[key:{k}]\n"));
        }
        for line in self.lines.iter().filter(|l| !l.is_blank()) {
            out.push_str(&format!("[{}]", stamp(self.seconds(line.start))));
            if enhanced {
                for c in &line.clusters {
                    out.push_str(&format!("<{}>{}", stamp(self.seconds(c.tick)), c.text));
                }
            } else {
                out.push_str(&line.text);
            }
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn midi(ppq: u16) -> MidiInfo {
        MidiInfo {
            format: 0,
            tracks: 1,
            ppq,
            smpte: None,
            tempo: TempoMap::new(ppq, vec![(0, 500_000)]),
            end_tick: 0,
            locked: false,
        }
    }

    #[test]
    fn garbage_tail_is_rejected() {
        let lyr = Lyrics::parse(b"T\r\nA\r\n\r\n\r\nabcdef\r\n");
        let mut m = midi(24);
        m.end_tick = 200;
        // Real timing, then junk: past the end, a lone spike, a huge drop.
        let s = NcnSong::from_parts("x", lyr, &cursor(&[10, 20, 30, 30000, 900, 40, 50]), &m);
        let ticks: Vec<u32> = s.lines[0].clusters.iter().map(|c| c.tick).collect();
        assert_eq!(ticks, [10, 20, 30, 30, 30, 40]);
        assert_eq!(s.rejected, 2);
        // A lone spike and a lone dip inside the song are dropped.
        let lyr = Lyrics::parse(b"T\r\nA\r\n\r\n\r\nabcde\r\n");
        m.end_tick = 100_000;
        let s = NcnSong::from_parts("x", lyr, &cursor(&[1000, 1010, 5000, 10, 1020]), &m);
        let ticks: Vec<u32> = s.lines[0].clusters.iter().map(|c| c.tick).collect();
        assert_eq!(ticks, [1000, 1010, 1010, 1010, 1020]);
        assert_eq!(s.rejected, 2);
        // Slow singing and instrumental breaks (big steps that continue) are kept.
        let lyr = Lyrics::parse(b"T\r\nA\r\n\r\n\r\nabcd\r\n");
        let s = NcnSong::from_parts("x", lyr, &cursor(&[10, 20, 150, 160, 170]), &m);
        assert_eq!(s.lines[0].clusters[2].tick, 150);
        assert_eq!(s.rejected, 0);
    }

    fn cursor(values: &[u16]) -> Cursor {
        Cursor { values: values.to_vec(), truncated_byte: false }
    }

    #[test]
    fn maps_bytes_and_line_breaks() {
        let lyr = Lyrics::parse("T\r\nA\r\nC\r\n\r\nab\r\ncd\r\n".as_bytes());
        // a b <nl> c d <nl>
        let s = NcnSong::from_parts("x", lyr, &cursor(&[24, 48, 60, 72, 96, 120]), &midi(480));
        assert_eq!(s.alignment, Alignment::Exact);
        assert_eq!(s.lines[0].clusters.iter().map(|c| c.tick).collect::<Vec<_>>(), [480, 960]);
        assert_eq!((s.lines[0].start, s.lines[0].end), (480, 1200));
        assert_eq!((s.lines[1].start, s.lines[1].end), (1440, 2400));
        assert!((s.seconds(s.lines[1].start) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn thai_marks_join_their_letter() {
        // "น้อง": NO NU + MAI THO + O ANG + NGO NGU
        let mut bytes = b"T\r\nA\r\n\r\n\r\n".to_vec();
        bytes.extend(cp874::encode("น้อง"));
        let s = NcnSong::from_parts("x", Lyrics::parse(&bytes), &cursor(&[10, 10, 20, 30, 40]), &midi(24));
        let texts: Vec<&str> = s.lines[0].clusters.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["น้", "อ", "ง"]);
        assert_eq!(s.lines[0].clusters[1].byte, 2);
    }

    #[test]
    fn clamps_backsteps_and_handles_short_cursor() {
        let lyr = Lyrics::parse(b"T\r\nA\r\n\r\n\r\nabcd\r\n");
        let s = NcnSong::from_parts("x", lyr, &cursor(&[10, 30, 20]), &midi(24));
        assert_eq!(s.alignment, Alignment::CursorShorter { missing: 2 });
        assert_eq!(s.backsteps, 1);
        let ticks: Vec<u32> = s.lines[0].clusters.iter().map(|c| c.tick).collect();
        assert_eq!(ticks, [10, 30, 30, 30]);
        assert_eq!(s.lines[0].clusters[3].raw, None);
    }

    #[test]
    fn progress_and_lrc() {
        let lyr = Lyrics::parse(b"Song\r\nSinger\r\nAm\r\n\r\nab\r\ncd\r\n");
        let s = NcnSong::from_parts("x", lyr, &cursor(&[24, 48, 60, 72, 96, 120]), &midi(24));
        assert_eq!(s.progress(0), Progress { line: 0, clusters: 0 });
        assert_eq!(s.progress(30), Progress { line: 0, clusters: 1 });
        assert_eq!(s.progress(80), Progress { line: 1, clusters: 1 });
        let lrc = s.to_lrc(false);
        assert!(lrc.contains("[ti:Song]") && lrc.contains("[key:Am]"));
        assert!(lrc.contains("[00:00.50]ab"));
        assert!(s.to_lrc(true).contains("<00:01.00>b"));
    }
}
