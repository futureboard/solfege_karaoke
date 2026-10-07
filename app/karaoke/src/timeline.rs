//! Lyric timing in seconds, shaped for the stage: the sung lines of a song,
//! each split into syllables (runs of clusters that light up together) with
//! the span over which the highlight wipes across them.

use solfege_ncnparser::NcnSong;

/// Longest time one syllable's wipe may take. Cursor gaps longer than this
/// are rests or instrumental fills, not a held note.
pub const MAX_WIPE: f64 = 2.5;
/// A gap at least this long before a line gets a count-in.
pub const COUNT_IN_GAP: f64 = 4.0;
/// Count-in length (one dot per second).
pub const COUNT_IN: f64 = 3.0;

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
}

impl Line {
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timeline {
    pub lines: Vec<Line>,
}

/// What the stage should show besides the lyric lines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cue {
    /// Before the singing starts: show the title card.
    Intro,
    /// A line starts after a long rest: dots counting down `remaining` seconds.
    CountIn { line: usize, remaining: f64 },
    None,
}

impl Timeline {
    pub fn new(song: &NcnSong) -> Self {
        let mut lines = Vec::new();
        for l in song.lines.iter().filter(|l| !l.is_blank()) {
            let end = song.seconds(l.end);
            let mut syllables: Vec<Syllable> = Vec::new();
            let mut last_tick = None;
            for c in &l.clusters {
                match syllables.last_mut() {
                    Some(s) if last_tick == Some(c.tick) => s.text.push_str(&c.text),
                    _ => {
                        let t = song.seconds(c.tick);
                        syllables.push(Syllable { text: c.text.clone(), start: t, end: t });
                    }
                }
                last_tick = Some(c.tick);
            }
            // Each syllable wipes until the next one starts (or the line ends).
            for i in 0..syllables.len() {
                let next = syllables.get(i + 1).map_or(end, |s| s.start);
                let s = &mut syllables[i];
                s.end = next.min(s.start + MAX_WIPE).max(s.start);
            }
            let start = syllables.first().map_or(end, |s| s.start);
            lines.push(Line { text: l.text.clone(), syllables, start, end: end.max(start) });
        }
        Self { lines }
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
        if t < first.start - COUNT_IN {
            return Cue::Intro;
        }
        let next = self.lines.partition_point(|l| l.start <= t);
        let Some(line) = self.lines.get(next) else { return Cue::None };
        let rest_from = if next == 0 { 0.0 } else { self.lines[next - 1].end };
        let remaining = line.start - t;
        if line.start - rest_from >= COUNT_IN_GAP && t >= rest_from && remaining <= COUNT_IN {
            Cue::CountIn { line: next, remaining }
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
            let song = lib.load(&e.id).unwrap();
            let tl = Timeline::new(&song);
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
        }
    }

    #[test]
    fn focus_and_cues() {
        let tl = Timeline {
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
        assert_eq!(tl.cue(8.0), Cue::CountIn { line: 0, remaining: 2.0 });
        // Short gap between line one and two: no count-in.
        assert_eq!(tl.cue(11.5), Cue::None);
        // Long rest before line three: count-in only for the last 3 s.
        assert_eq!(tl.cue(20.0), Cue::None);
        assert_eq!(tl.cue(28.0), Cue::CountIn { line: 2, remaining: 2.0 });
        assert_eq!(tl.cue(40.0), Cue::None);
    }
}
