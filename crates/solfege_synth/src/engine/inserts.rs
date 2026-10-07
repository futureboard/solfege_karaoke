//! Master effect slots: a chain of up to [`INSERT_SLOTS`] effects on the
//! main output, after the reverb / chorus returns and before the master
//! gain. Each slot holds one effect, can be bypassed, and is processed in
//! slot order.
//!
//! An [`Insert`] (effect plus its buffers) is built on the UI thread and
//! sent with [`super::Command::SetInsert`]; parameter changes of the same
//! effect go through [`super::Command::SetInsertParams`] and never allocate.

use serde::{Deserialize, Serialize};

use super::dsp::{Biquad, Chorus, Reverb};

pub const INSERT_SLOTS: usize = 10;
/// Most frames processed in one go (longer blocks are split).
const CHUNK: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InsertKind {
    Eq,
    Compressor,
    Limiter,
    Delay,
    Reverb,
    Chorus,
    Drive,
    Filter,
    Width,
}

/// One parameter of an effect, in its own units.
#[derive(Clone, Copy, Debug)]
pub struct ParamSpec {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: &'static str,
}

const fn p(name: &'static str, min: f32, max: f32, default: f32, unit: &'static str) -> ParamSpec {
    ParamSpec { name, min, max, default, unit }
}

impl InsertKind {
    pub const ALL: [InsertKind; 9] = [
        InsertKind::Eq,
        InsertKind::Compressor,
        InsertKind::Limiter,
        InsertKind::Delay,
        InsertKind::Reverb,
        InsertKind::Chorus,
        InsertKind::Drive,
        InsertKind::Filter,
        InsertKind::Width,
    ];

    pub fn name(self) -> &'static str {
        match self {
            InsertKind::Eq => "EQ 3 Band",
            InsertKind::Compressor => "Compressor",
            InsertKind::Limiter => "Limiter",
            InsertKind::Delay => "Delay",
            InsertKind::Reverb => "Reverb",
            InsertKind::Chorus => "Chorus",
            InsertKind::Drive => "Drive",
            InsertKind::Filter => "Filter",
            InsertKind::Width => "Stereo Width",
        }
    }

    /// Parameters, at most four.
    pub fn params(self) -> &'static [ParamSpec] {
        match self {
            InsertKind::Eq => {
                const P: &[ParamSpec] = &[
                    p("Low", -12.0, 12.0, 0.0, "dB"),
                    p("Mid", -12.0, 12.0, 0.0, "dB"),
                    p("Mid freq", 200.0, 5000.0, 1000.0, "Hz"),
                    p("High", -12.0, 12.0, 0.0, "dB"),
                ];
                P
            }
            InsertKind::Compressor => {
                const P: &[ParamSpec] = &[
                    p("Threshold", -40.0, 0.0, -18.0, "dB"),
                    p("Ratio", 1.0, 20.0, 4.0, ":1"),
                    p("Attack", 1.0, 100.0, 10.0, "ms"),
                    p("Release", 20.0, 1000.0, 150.0, "ms"),
                ];
                P
            }
            InsertKind::Limiter => {
                const P: &[ParamSpec] = &[
                    p("Gain", 0.0, 12.0, 0.0, "dB"),
                    p("Ceiling", -12.0, 0.0, -1.0, "dB"),
                    p("Release", 10.0, 500.0, 80.0, "ms"),
                ];
                P
            }
            InsertKind::Delay => {
                const P: &[ParamSpec] = &[
                    p("Time", 20.0, 1500.0, 350.0, "ms"),
                    p("Feedback", 0.0, 90.0, 35.0, "%"),
                    p("Mix", 0.0, 100.0, 25.0, "%"),
                    p("Tone", 1000.0, 16000.0, 6000.0, "Hz"),
                ];
                P
            }
            InsertKind::Reverb => {
                const P: &[ParamSpec] = &[
                    p("Room", 0.0, 100.0, 60.0, "%"),
                    p("Damp", 0.0, 100.0, 40.0, "%"),
                    p("Width", 0.0, 100.0, 100.0, "%"),
                    p("Mix", 0.0, 100.0, 25.0, "%"),
                ];
                P
            }
            InsertKind::Chorus => {
                const P: &[ParamSpec] = &[
                    p("Rate", 0.05, 5.0, 0.8, "Hz"),
                    p("Depth", 0.0, 10.0, 3.0, "ms"),
                    p("Delay", 2.0, 30.0, 12.0, "ms"),
                    p("Mix", 0.0, 100.0, 50.0, "%"),
                ];
                P
            }
            InsertKind::Drive => {
                const P: &[ParamSpec] = &[
                    p("Drive", 0.0, 30.0, 8.0, "dB"),
                    p("Tone", 1000.0, 16000.0, 8000.0, "Hz"),
                    p("Mix", 0.0, 100.0, 100.0, "%"),
                    p("Output", -24.0, 0.0, -6.0, "dB"),
                ];
                P
            }
            InsertKind::Filter => {
                const P: &[ParamSpec] = &[p("Low cut", 20.0, 1000.0, 20.0, "Hz"), p("High cut", 1000.0, 20000.0, 20000.0, "Hz")];
                P
            }
            InsertKind::Width => {
                const P: &[ParamSpec] = &[p("Width", 0.0, 200.0, 100.0, "%")];
                P
            }
        }
    }
}

/// What a slot holds: the effect, whether it is bypassed, and its
/// parameter values (in [`InsertKind::params`] order and units).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct InsertParams {
    pub kind: InsertKind,
    #[serde(default)]
    pub bypass: bool,
    pub values: [f32; 4],
}

impl InsertParams {
    pub fn new(kind: InsertKind) -> Self {
        let mut values = [0.0; 4];
        for (v, spec) in values.iter_mut().zip(kind.params()) {
            *v = spec.default;
        }
        Self { kind, bypass: false, values }
    }

    /// Values kept inside their ranges.
    pub fn clamped(mut self) -> Self {
        for (v, spec) in self.values.iter_mut().zip(self.kind.params()) {
            *v = if v.is_finite() { v.clamp(spec.min, spec.max) } else { spec.default };
        }
        self
    }
}

fn db(x: f32) -> f32 {
    10f32.powf(x / 20.0)
}

/// Gain envelope follower coefficient for a time constant.
fn coeff(sr: f32, ms: f32) -> f32 {
    (-1.0 / (sr * ms.max(0.1) / 1000.0)).exp()
}

enum State {
    Eq([Biquad; 3]),
    /// Envelope (linear) and smoothed gain.
    Compressor { env: f32 },
    Limiter { gain: f32 },
    Delay { buf: [Vec<f32>; 2], pos: usize, tone: Biquad },
    Reverb(Box<Reverb>),
    Chorus(Box<Chorus>),
    Drive { tone: Biquad },
    Filter([Biquad; 2]),
    Width,
}

/// One effect slot with its state; see the module docs.
pub struct Insert {
    params: InsertParams,
    sr: f32,
    state: State,
    /// Copy of the dry input for effects that add a wet signal.
    scratch: [Vec<f32>; 2],
}

impl Insert {
    /// Build an effect (allocates; call off the audio thread).
    pub fn new(params: InsertParams, sr: f32) -> Self {
        let params = params.clamped();
        let state = match params.kind {
            InsertKind::Eq => State::Eq([Biquad::default(); 3]),
            InsertKind::Compressor => State::Compressor { env: 0.0 },
            InsertKind::Limiter => State::Limiter { gain: 1.0 },
            InsertKind::Delay => {
                let len = (sr * 1.6) as usize + 2;
                State::Delay { buf: [vec![0.0; len], vec![0.0; len]], pos: 0, tone: Biquad::default() }
            }
            InsertKind::Reverb => State::Reverb(Box::new(Reverb::new(sr))),
            InsertKind::Chorus => State::Chorus(Box::new(Chorus::new(sr))),
            InsertKind::Drive => State::Drive { tone: Biquad::default() },
            InsertKind::Filter => State::Filter([Biquad::default(); 2]),
            InsertKind::Width => State::Width,
        };
        let mut insert = Self { params, sr, state, scratch: [vec![0.0; CHUNK], vec![0.0; CHUNK]] };
        insert.retune();
        insert
    }

    pub fn params(&self) -> InsertParams {
        self.params
    }

    /// New values for the same effect (no allocation). Another kind of
    /// effect needs a new `Insert`; such a change is ignored here.
    pub fn set(&mut self, params: InsertParams) {
        if params.kind == self.params.kind {
            self.params = params.clamped();
            self.retune();
        }
    }

    fn retune(&mut self) {
        let v = self.params.values;
        let sr = self.sr;
        match &mut self.state {
            State::Eq(f) => {
                f[0].retune(Biquad::low_shelf(sr, 200.0, v[0]));
                f[1].retune(Biquad::peaking(sr, v[2], 0.8, v[1]));
                f[2].retune(Biquad::high_shelf(sr, 5000.0, v[3]));
            }
            State::Delay { tone, .. } => tone.retune(Biquad::lowpass(sr, v[3], 0.707)),
            State::Drive { tone } => tone.retune(Biquad::lowpass(sr, v[1], 0.707)),
            State::Filter(f) => {
                f[0].retune(if v[0] > 21.0 { Biquad::highpass(sr, v[0], 0.707) } else { Biquad::default() });
                f[1].retune(if v[1] < 19_900.0 { Biquad::lowpass(sr, v[1], 0.707) } else { Biquad::default() });
            }
            _ => {}
        }
    }

    /// Process a stereo block in place (bypassed: untouched).
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        if self.params.bypass {
            return;
        }
        let n = l.len().min(r.len());
        let mut start = 0;
        while start < n {
            let end = (start + CHUNK).min(n);
            self.process_chunk(&mut l[start..end], &mut r[start..end]);
            start = end;
        }
    }

    fn process_chunk(&mut self, l: &mut [f32], r: &mut [f32]) {
        let v = self.params.values;
        let sr = self.sr;
        let n = l.len();
        match &mut self.state {
            State::Eq(f) => {
                for filter in f.iter_mut().filter(|f| f.active) {
                    for i in 0..n {
                        l[i] = filter.process(0, l[i]);
                        r[i] = filter.process(1, r[i]);
                    }
                }
            }
            State::Compressor { env } => {
                let (threshold, ratio) = (v[0], v[1]);
                let (att, rel) = (coeff(sr, v[2]), coeff(sr, v[3]));
                // Make up about half the reduction at the threshold.
                let makeup = db(-threshold * (1.0 - 1.0 / ratio) * 0.5);
                for i in 0..n {
                    let peak = l[i].abs().max(r[i].abs());
                    let k = if peak > *env { att } else { rel };
                    *env = k * *env + (1.0 - k) * peak;
                    let level = 20.0 * env.max(1e-6).log10();
                    let over = (level - threshold).max(0.0);
                    let gain = db(-over * (1.0 - 1.0 / ratio)) * makeup;
                    l[i] *= gain;
                    r[i] *= gain;
                }
            }
            State::Limiter { gain } => {
                let (input, ceiling) = (db(v[0]), db(v[1]));
                let rel = coeff(sr, v[2]);
                for i in 0..n {
                    let (a, b) = (l[i] * input, r[i] * input);
                    let peak = a.abs().max(b.abs());
                    let want = if peak * *gain > ceiling { ceiling / peak } else { 1.0 };
                    // Instant attack, smooth release.
                    *gain = if want < *gain { want } else { rel * *gain + (1.0 - rel) * want };
                    l[i] = (a * *gain).clamp(-ceiling, ceiling);
                    r[i] = (b * *gain).clamp(-ceiling, ceiling);
                }
            }
            State::Delay { buf, pos, tone } => {
                let len = buf[0].len();
                let d = ((v[0] / 1000.0 * sr) as usize).clamp(1, len - 1);
                let (fb, mix) = (v[1] / 100.0, v[2] / 100.0);
                for i in 0..n {
                    let read = (*pos + len - d) % len;
                    let (wl, wr) = (buf[0][read], buf[1][read]);
                    // Ping-pong: each side feeds the other.
                    buf[0][*pos] = l[i] + tone.process(1, wr) * fb;
                    buf[1][*pos] = r[i] + tone.process(0, wl) * fb;
                    l[i] += wl * mix;
                    r[i] += wr * mix;
                    *pos = (*pos + 1) % len;
                }
            }
            State::Reverb(rev) => {
                let mix = v[3] / 100.0;
                self.scratch[0][..n].copy_from_slice(l);
                self.scratch[1][..n].copy_from_slice(r);
                for i in 0..n {
                    l[i] *= 1.0 - mix * 0.5;
                    r[i] *= 1.0 - mix * 0.5;
                }
                rev.process(&self.scratch[0][..n], &self.scratch[1][..n], l, r, v[0] / 100.0, v[1] / 100.0, v[2] / 100.0, mix);
            }
            State::Chorus(ch) => {
                let mix = v[3] / 100.0;
                self.scratch[0][..n].copy_from_slice(l);
                self.scratch[1][..n].copy_from_slice(r);
                for i in 0..n {
                    l[i] *= 1.0 - mix * 0.5;
                    r[i] *= 1.0 - mix * 0.5;
                }
                ch.process(&self.scratch[0][..n], &self.scratch[1][..n], l, r, v[0], v[1], v[2], mix);
            }
            State::Drive { tone } => {
                let (drive, mix, out) = (db(v[0]), v[2] / 100.0, db(v[3]));
                for i in 0..n {
                    let wl = tone.process(0, (l[i] * drive).tanh());
                    let wr = tone.process(1, (r[i] * drive).tanh());
                    l[i] = (l[i] * (1.0 - mix) + wl * mix) * out;
                    r[i] = (r[i] * (1.0 - mix) + wr * mix) * out;
                }
            }
            State::Filter(f) => {
                for filter in f.iter_mut().filter(|f| f.active) {
                    for i in 0..n {
                        l[i] = filter.process(0, l[i]);
                        r[i] = filter.process(1, r[i]);
                    }
                }
            }
            State::Width => {
                let w = v[0] / 100.0;
                for i in 0..n {
                    let mid = (l[i] + r[i]) * 0.5;
                    let side = (l[i] - r[i]) * 0.5 * w;
                    l[i] = mid + side;
                    r[i] = mid - side;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(n: usize, amp: f32) -> Vec<f32> {
        (0..n).map(|i| (i as f32 * 0.05).sin() * amp).collect()
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
    }

    #[test]
    fn every_effect_runs_and_stays_finite() {
        for kind in InsertKind::ALL {
            let mut fx = Insert::new(InsertParams::new(kind), 48_000.0);
            let (mut l, mut r) = (sine(5000, 0.5), sine(5000, 0.3));
            fx.process(&mut l, &mut r);
            assert!(l.iter().chain(&r).all(|v| v.is_finite()), "{kind:?}");
            assert!(peak(&l) < 4.0, "{kind:?}: {}", peak(&l));
            assert!(kind.params().len() <= 4);
        }
    }

    #[test]
    fn bypass_leaves_the_signal_alone() {
        let mut p = InsertParams::new(InsertKind::Drive);
        p.bypass = true;
        let mut fx = Insert::new(p, 48_000.0);
        let (mut l, mut r) = (sine(500, 0.5), sine(500, 0.5));
        let before = l.clone();
        fx.process(&mut l, &mut r);
        assert_eq!(l, before);
    }

    #[test]
    fn limiter_holds_the_ceiling_and_compressor_reduces() {
        let mut lim = InsertParams::new(InsertKind::Limiter);
        lim.values[0] = 12.0; // +12 dB into a -1 dB ceiling
        let mut fx = Insert::new(lim, 48_000.0);
        let (mut l, mut r) = (sine(4000, 0.9), sine(4000, 0.9));
        fx.process(&mut l, &mut r);
        assert!(peak(&l) <= db(-1.0) + 1e-4, "{}", peak(&l));

        let mut comp = InsertParams::new(InsertKind::Compressor);
        comp.values = [-30.0, 10.0, 1.0, 50.0];
        let mut fx = Insert::new(comp, 48_000.0);
        let (mut l, mut r) = (sine(48_000, 0.9), sine(48_000, 0.9));
        fx.process(&mut l, &mut r);
        assert!(peak(&l[40_000..]) < 0.6, "{}", peak(&l[40_000..]));
    }

    #[test]
    fn width_zero_is_mono_and_values_are_clamped() {
        let mut p = InsertParams::new(InsertKind::Width);
        p.values[0] = 0.0;
        let mut fx = Insert::new(p, 48_000.0);
        let (mut l, mut r) = (vec![1.0, 0.0], vec![0.0, 1.0]);
        fx.process(&mut l, &mut r);
        assert_eq!(l, r);
        let wild = InsertParams { kind: InsertKind::Delay, bypass: false, values: [99_999.0, -5.0, f32::NAN, 0.0] }.clamped();
        assert_eq!(wild.values[0], 1500.0);
        assert_eq!(wild.values[1], 0.0);
        assert_eq!(wild.values[2], 25.0);
        assert_eq!(wild.values[3], 1000.0);
    }

    #[test]
    fn delay_repeats_after_its_time() {
        let mut p = InsertParams::new(InsertKind::Delay);
        p.values = [100.0, 0.0, 100.0, 16_000.0];
        let mut fx = Insert::new(p, 1000.0);
        let mut l = vec![0.0; 300];
        let mut r = vec![0.0; 300];
        l[0] = 1.0;
        fx.process(&mut l, &mut r);
        // 100 ms at 1 kHz: the click comes back 100 samples later.
        assert!(l[100].abs() > 0.1 || r[100].abs() > 0.1);
        assert!(l[50].abs() < 1e-3 && r[50].abs() < 1e-3);
    }
}
