//! Lyric timing in seconds, shaped for the stage: the sung lines of a song,
//! each split into syllables (runs of clusters that light up together) with
//! the span over which the highlight wipes across them.

use solfege_ncnparser::MidiInfo;
use solfege_sfkar::KarSong;

/// Longest time one syllable's wipe may take. Cursor gaps longer than this
/// are rests or instrumental fills, not a held note.
pub const MAX_WIPE: f64 = 2.5;
/// A rest at least this long (and at least a beat longer than the
/// count-in) before a line gets a count-in.
pub const COUNT_IN_GAP: f64 = 4.0;
/// Count-in length in beats: one dot per quarter note.
pub const COUNT_IN_BEATS: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct Syllable {
    pub text: String,
    /// Wipe starts (seconds).
    pub start: f64,
    /// Wipe is complete.
    pub end: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub syllables: Vec<Syllable>,
    pub start: f64,
    /// The line-break entry: the line is finished.
    pub end: f64,
    /// Seconds per beat (quarter note) where the line starts.
    pub beat: f64,
}

impl Line {
    /// Length of the count-in before this line.
    pub fn count_in(&self) -> f64 {
        COUNT_IN_BEATS as f64 * self.beat
    }

    /// How far the highlight has travelled at `t`: whole syllables done,
    /// plus the fraction (0..1) of the one being sung.
    pub fn progress(&self, t: f64) -> (usize, f32) {
        let done = self.syllables.partition_point(|s| s.end <= t);
        let frac = match self.syllables.get(done) {
            Some(s) if t > s.start => ((t - s.start) / (s.end - s.start).max(1e-6)) as f32,
            _ => 0.0,
        };
        (done, frac.clamp(0.0, 1.0))
    }
}

/// The tempo map in seconds: where each tempo starts, how many quarter
/// notes came before it, and its seconds per quarter note. Also the time
/// signatures, as (quarter note where it starts, beats per bar, quarter
/// notes per beat).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tempo {
    segments: Vec<(f64, f64, f64)>,
    meters: Vec<(f64, u8, f64)>,
}

impl Tempo {
    pub fn new(midi: &MidiInfo) -> Self {
        let ppq = midi.ppq.max(1) as u32;
        let seconds = |tick: u32| midi.tempo.seconds(tick);
        let mut ticks: Vec<u32> = std::iter::once(0).chain(midi.tempo.changes().iter().map(|c| c.0)).collect();
        ticks.dedup();
        let segments = ticks
            .into_iter()
            .map(|tick| {
                let sec = seconds(tick);
                let spq = (seconds(tick + ppq) - sec).max(1e-3);
                (sec, tick as f64 / ppq as f64, spq)
            })
            .collect();
        let meters = midi.meters.iter().map(|m| (m.tick as f64 / ppq as f64, m.beats, m.beat_quarters())).collect();
        Self { segments, meters }
    }

    fn at(&self, t: f64) -> (f64, f64, f64) {
        let i = self.segments.partition_point(|s| s.0 <= t).saturating_sub(1);
        self.segments.get(i).copied().unwrap_or((0.0, 0.0, 0.5))
    }

    /// Seconds per quarter note at `t`.
    pub fn beat(&self, t: f64) -> f64 {
        self.at(t).2
    }

    /// Song tempo at `t` in quarter notes per minute.
    pub fn bpm(&self, t: f64) -> f64 {
        60.0 / self.beat(t)
    }

    /// Quarter notes elapsed at `t` (the fraction is the beat phase).
    pub fn quarters(&self, t: f64) -> f64 {
        let (sec, q, spq) = self.at(t);
        q + (t - sec) / spq
    }

    /// Where `quarters` falls in its bar: the beat (0-based), the beats in
    /// the bar, and how far into the beat (0..1). Bars count from each time
    /// signature; without one the song is 4/4 from its start.
    pub fn bar_beat(&self, quarters: f64) -> (usize, usize, f32) {
        let q = quarters.max(0.0);
        let i = self.meters.partition_point(|m| m.0 <= q);
        let (start, beats, len) = if i == 0 { (0.0, 4, 1.0) } else { self.meters[i - 1] };
        let n = (q - start) / len.max(1e-6);
        ((n.floor() as usize) % beats.max(1) as usize, beats.max(1) as usize, n.fract() as f32)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timeline {
    pub lines: Vec<Line>,
    pub tempo: Tempo,
}

/// What the stage should show besides the lyric lines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cue {
    /// Before the singing starts: show the title card.
    Intro,
    /// A line starts after a long rest: one dot per beat still to go
    /// (`beats` counts down from `COUNT_IN_BEATS` to 0).
    CountIn { line: usize, beats: f64 },
    None,
}

impl Timeline {
    /// Timed lines of a song; `midi` is its backing track's timing.
    pub fn new(song: &KarSong, midi: &MidiInfo) -> Self {
        let tempo = Tempo::new(midi);
        let seconds = |tick: u32| midi.tempo.seconds(tick);
        let mut lines = Vec::new();
        for l in song.lyrics.lines.iter().filter(|l| !l.text().trim().is_empty()) {
            let end = seconds(l.end);
            let mut syllables: Vec<Syllable> = Vec::new();
            for seg in &l.segments {
                match syllables.last_mut() {
                    // Segments are already per tick; equal ticks only in hand-made files.
                    Some(s) if s.start == seconds(seg.tick()) => s.text.push_str(seg.text()),
                    _ => {
                        let t = seconds(seg.tick());
                        syllables.push(Syllable { text: seg.text().to_string(), start: t, end: t });
                    }
                }
            }
            // Each syllable wipes until the next one starts (or the line ends).
            for i in 0..syllables.len() {
                let next = syllables.get(i + 1).map_or(end, |s| s.start);
                let s = &mut syllables[i];
                s.end = next.min(s.start + MAX_WIPE).max(s.start);
            }
            let start = syllables.first().map_or(end, |s| s.start);
            let beat = tempo.beat(start);
            lines.push(Line { text: l.text(), syllables, start, end: end.max(start), beat });
        }
        Self { lines, tempo }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The line in focus at `t`: the last one that has started, or 0
    /// before the first line.
    pub fn focus(&self, t: f64) -> usize {
        self.lines.partition_point(|l| l.start <= t).saturating_sub(1)
    }

    pub fn cue(&self, t: f64) -> Cue {
        let Some(first) = self.lines.first() else { return Cue::None };
        if t < first.start - first.count_in() {
            return Cue::Intro;
        }
        let next = self.lines.partition_point(|l| l.start <= t);
        let Some(line) = self.lines.get(next) else { return Cue::None };
        let rest_from = if next == 0 { 0.0 } else { self.lines[next - 1].end };
        let rest = line.start - rest_from;
        let remaining = line.start - t;
        let long_rest = rest >= COUNT_IN_GAP.max(line.count_in() + line.beat);
        if long_rest && t >= rest_from && remaining <= line.count_in() {
            Cue::CountIn { line: next, beats: remaining / line.beat }
        } else {
            Cue::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, syl: &[(&str, f64, f64)], start: f64, end: f64) -> Line {
        Line {
            text: text.into(),
            syllables: syl.iter().map(|&(t, s, e)| Syllable { text: t.into(), start: s, end: e }).collect(),
            start,
            end,
            beat: 0.5,
        }
    }

    #[test]
    fn progress_wipes_through_syllables() {
        let l = line("abc", &[("a", 1.0, 2.0), ("b", 2.0, 2.5), ("c", 3.0, 4.0)], 1.0, 4.0);
        assert_eq!(l.progress(0.5), (0, 0.0));
        assert_eq!(l.progress(1.5), (0, 0.5));
        assert_eq!(l.progress(2.25), (1, 0.5));
        // Between "b" finishing and "c" starting the wipe waits.
        assert_eq!(l.progress(2.8), (2, 0.0));
        assert_eq!(l.progress(9.0), (3, 0.0));
    }

    #[test]
    fn every_sample_song_builds_a_sane_timeline() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shared/NCN");
        let Ok(lib) = solfege_ncnparser::NcnLibrary::open(&root) else {
            eprintln!("skipped: {} missing", root.display());
            return;
        };
        for e in lib.complete() {
            let kar = solfege_sfkar::convert(&lib, &e.id).unwrap();
            let tl = Timeline::new(&kar, &kar.timing().unwrap());
            assert!(tl.lines.len() > 3, "{}", e.id);
            let mut last = 0.0;
            for l in &tl.lines {
                assert!(!l.text.trim().is_empty());
                assert!(l.start <= l.end, "{}: {l:?}", e.id);
                for s in &l.syllables {
                    assert!(s.start >= last && s.end >= s.start && s.end - s.start <= MAX_WIPE + 1e-9, "{}: {s:?}", e.id);
                    last = s.start;
                }
            }
            assert_eq!(tl.cue(-10.0), Cue::Intro, "{}", e.id);
            let bpm = tl.tempo.bpm(tl.lines[0].start);
            assert!((40.0..=260.0).contains(&bpm), "{}: {bpm} BPM", e.id);
        }
    }

    #[test]
    fn focus_and_cues() {
        let tl = Timeline {
            tempo: Tempo::default(),
            lines: vec![
                line("one", &[("one", 10.0, 11.0)], 10.0, 11.0),
                line("two", &[("two", 12.0, 13.0)], 12.0, 13.0),
                line("three", &[("three", 30.0, 31.0)], 30.0, 31.0),
            ],
        };
        assert_eq!(tl.focus(0.0), 0);
        assert_eq!(tl.focus(12.5), 1);
        assert_eq!(tl.focus(29.0), 1);
        assert_eq!(tl.focus(30.0), 2);
        assert_eq!(tl.cue(2.0), Cue::Intro);
        assert_eq!(tl.cue(8.0), Cue::CountIn { line: 0, beats: 4.0 });
        // Short gap between line one and two: no count-in.
        assert_eq!(tl.cue(11.5), Cue::None);
        // Long rest before line three: four beats of count-in (120 BPM).
        assert_eq!(tl.cue(20.0), Cue::None);
        assert_eq!(tl.cue(27.9), Cue::None);
        assert_eq!(tl.cue(28.0), Cue::CountIn { line: 2, beats: 4.0 });
        assert_eq!(tl.cue(29.25), Cue::CountIn { line: 2, beats: 1.5 });
        assert_eq!(tl.cue(40.0), Cue::None);
    }

    #[test]
    fn tempo_in_seconds() {
        use solfege_ncnparser::{MidiInfo, TempoMap};
        let lyr = solfege_ncnparser::Lyrics::parse(b"T\r\nA\r\n\r\n\r\nab\r\n");
        let cur = solfege_ncnparser::Cursor { values: vec![24, 48, 72], truncated_byte: false };
        // 120 BPM, then 60 BPM from the third quarter (ppq 480).
        let midi = MidiInfo {
            format: 0,
            tracks: 1,
            ppq: 480,
            smpte: None,
            tempo: TempoMap::new(480, vec![(0, 500_000), (960, 1_000_000)]),
            end_tick: 4800,
            locked: false,
            meters: Vec::new(),
            texts: Vec::new(),
        };
        let song = solfege_ncnparser::NcnSong::from_parts("x", lyr, &cur, &midi);
        let tl = Timeline::new(&KarSong::from_ncn(&song, Vec::new()), &midi);
        assert!((tl.tempo.bpm(0.5) - 120.0).abs() < 1e-6);
        assert!((tl.tempo.bpm(1.5) - 60.0).abs() < 1e-6);
        assert!((tl.tempo.quarters(1.0) - 2.0).abs() < 1e-6);
        // No time signature: 4/4 from the start.
        assert_eq!(tl.tempo.bar_beat(0.0), (0, 4, 0.0));
        assert_eq!(tl.tempo.bar_beat(5.25), (1, 4, 0.25));
        assert_eq!(tl.tempo.bar_beat(-1.0).0, 0, "before the song");

        // 3/4, then 6/8 from quarter 6 (bar 3): bars restart at the change.
        let midi = MidiInfo {
            meters: vec![
                solfege_ncnparser::Meter { tick: 0, beats: 3, unit: 2 },
                solfege_ncnparser::Meter { tick: 6 * 480, beats: 6, unit: 3 },
            ],
            ..midi
        };
        let tempo = Tempo::new(&midi);
        assert_eq!(tempo.bar_beat(2.5), (2, 3, 0.5));
        assert_eq!(tempo.bar_beat(3.0).0, 0, "bar two");
        assert_eq!(tempo.bar_beat(6.0), (0, 6, 0.0));
        assert_eq!(tempo.bar_beat(7.5), (3, 6, 0.0), "eighth-note beats");
        assert_eq!(tempo.bar_beat(9.0).0, 0, "next 6/8 bar");
        assert!((tl.tempo.quarters(2.5) - 3.5).abs() < 1e-6);
        assert!((tl.lines[0].beat - 0.5).abs() < 1e-6);
        assert!((tl.lines[0].count_in() - 2.0).abs() < 1e-6);
    }
}
