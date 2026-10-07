//! Mixer DSP building blocks: RBJ biquads (EQ / filters), a Freeverb-style
//! stereo reverb and a modulated-delay stereo chorus. Buffers are allocated
//! in `new` (UI thread); processing never allocates.

use std::f32::consts::{PI, TAU};

/// Transposed direct form II biquad, stereo state.
#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: [f32; 2],
    z2: [f32; 2],
    /// False when the filter is a no-op, so processing can skip it.
    pub active: bool,
}

impl Default for Biquad {
    fn default() -> Self {
        Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, z1: [0.0; 2], z2: [0.0; 2], active: false }
    }
}

struct Omega {
    cos: f32,
    sin: f32,
}

fn omega(sr: f32, f: f32) -> Omega {
    let w = TAU * f.clamp(10.0, sr * 0.49) / sr;
    Omega { cos: w.cos(), sin: w.sin() }
}

impl Biquad {
    fn from(b: [f32; 3], a: [f32; 3]) -> Self {
        Self {
            b0: b[0] / a[0],
            b1: b[1] / a[0],
            b2: b[2] / a[0],
            a1: a[1] / a[0],
            a2: a[2] / a[0],
            active: true,
            ..Self::default()
        }
    }

    pub fn peaking(sr: f32, f: f32, q: f32, db: f32) -> Self {
        if db.abs() < 0.05 {
            return Self::default();
        }
        let a = 10f32.powf(db / 40.0);
        let w = omega(sr, f);
        let alpha = w.sin / (2.0 * q);
        Self::from(
            [1.0 + alpha * a, -2.0 * w.cos, 1.0 - alpha * a],
            [1.0 + alpha / a, -2.0 * w.cos, 1.0 - alpha / a],
        )
    }

    pub fn low_shelf(sr: f32, f: f32, db: f32) -> Self {
        if db.abs() < 0.05 {
            return Self::default();
        }
        let a = 10f32.powf(db / 40.0);
        let w = omega(sr, f);
        let k = 2.0 * a.sqrt() * (w.sin / 2.0 * std::f32::consts::SQRT_2);
        Self::from(
            [
                a * ((a + 1.0) - (a - 1.0) * w.cos + k),
                2.0 * a * ((a - 1.0) - (a + 1.0) * w.cos),
                a * ((a + 1.0) - (a - 1.0) * w.cos - k),
            ],
            [(a + 1.0) + (a - 1.0) * w.cos + k, -2.0 * ((a - 1.0) + (a + 1.0) * w.cos), (a + 1.0) + (a - 1.0) * w.cos - k],
        )
    }

    pub fn high_shelf(sr: f32, f: f32, db: f32) -> Self {
        if db.abs() < 0.05 {
            return Self::default();
        }
        let a = 10f32.powf(db / 40.0);
        let w = omega(sr, f);
        let k = 2.0 * a.sqrt() * (w.sin / 2.0 * std::f32::consts::SQRT_2);
        Self::from(
            [
                a * ((a + 1.0) + (a - 1.0) * w.cos + k),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * w.cos),
                a * ((a + 1.0) + (a - 1.0) * w.cos - k),
            ],
            [(a + 1.0) - (a - 1.0) * w.cos + k, 2.0 * ((a - 1.0) - (a + 1.0) * w.cos), (a + 1.0) - (a - 1.0) * w.cos - k],
        )
    }

    pub fn lowpass(sr: f32, f: f32, q: f32) -> Self {
        if f >= 19_999.0 {
            return Self::default();
        }
        let w = omega(sr, f);
        let alpha = w.sin / (2.0 * q);
        let b = (1.0 - w.cos) / 2.0;
        Self::from([b, 1.0 - w.cos, b], [1.0 + alpha, -2.0 * w.cos, 1.0 - alpha])
    }

    pub fn highpass(sr: f32, f: f32, q: f32) -> Self {
        if f <= 20.0 {
            return Self::default();
        }
        let w = omega(sr, f);
        let alpha = w.sin / (2.0 * q);
        let b = (1.0 + w.cos) / 2.0;
        Self::from([b, -(1.0 + w.cos), b], [1.0 + alpha, -2.0 * w.cos, 1.0 - alpha])
    }

    /// Take new coefficients but keep the running state (no click).
    pub fn retune(&mut self, new: Biquad) {
        let (z1, z2) = (self.z1, self.z2);
        *self = new;
        if self.active {
            self.z1 = z1;
            self.z2 = z2;
        }
    }

    #[inline]
    pub fn process(&mut self, c: usize, x: f32) -> f32 {
        let y = self.b0 * x + self.z1[c];
        self.z1[c] = self.b1 * x - self.a1 * y + self.z2[c];
        self.z2[c] = self.b2 * x - self.a2 * y;
        y
    }
}

// ------------------------------------------------------------------ reverb

struct Comb {
    buf: Vec<f32>,
    idx: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], idx: 0, store: 0.0 }
    }

    #[inline]
    fn process(&mut self, x: f32, feedback: f32, damp1: f32, damp2: f32) -> f32 {
        let out = self.buf[self.idx];
        self.store = out * damp2 + self.store * damp1;
        self.buf[self.idx] = x + self.store * feedback;
        self.idx += 1;
        if self.idx == self.buf.len() {
            self.idx = 0;
        }
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    idx: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], idx: 0 }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let b = self.buf[self.idx];
        self.buf[self.idx] = x + b * 0.5;
        self.idx += 1;
        if self.idx == self.buf.len() {
            self.idx = 0;
        }
        b - x
    }
}

const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
const SPREAD: usize = 23;

/// Jezar's Freeverb topology: 8 parallel combs + 4 series allpasses per side.
pub struct Reverb {
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
}

impl Reverb {
    pub fn new(sr: f32) -> Self {
        let scale = sr / 44_100.0;
        let len = |n: usize, side: usize| ((n + side * SPREAD) as f32 * scale) as usize;
        Self {
            combs: [0, 1].map(|s| COMBS.iter().map(|&n| Comb::new(len(n, s))).collect()),
            allpasses: [0, 1].map(|s| ALLPASSES.iter().map(|&n| Allpass::new(len(n, s))).collect()),
        }
    }

    /// Mixes the wet signal of `input` into `out` at `gain`.
    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        in_l: &[f32],
        in_r: &[f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
        room: f32,
        damp: f32,
        width: f32,
        gain: f32,
    ) {
        let feedback = room.clamp(0.0, 1.0) * 0.28 + 0.7;
        let damp1 = damp.clamp(0.0, 1.0) * 0.4;
        let damp2 = 1.0 - damp1;
        let wet = gain * 3.0;
        let width = width.clamp(0.0, 1.0);
        let wet1 = wet * (width / 2.0 + 0.5);
        let wet2 = wet * ((1.0 - width) / 2.0);
        for i in 0..in_l.len() {
            let x = (in_l[i] + in_r[i]) * 0.015;
            let mut side = [0f32; 2];
            for (s, acc) in side.iter_mut().enumerate() {
                let mut sum = 0.0;
                for c in &mut self.combs[s] {
                    sum += c.process(x, feedback, damp1, damp2);
                }
                for a in &mut self.allpasses[s] {
                    sum = a.process(sum);
                }
                *acc = sum;
            }
            out_l[i] += side[0] * wet1 + side[1] * wet2;
            out_r[i] += side[1] * wet1 + side[0] * wet2;
        }
    }
}

// ------------------------------------------------------------------ chorus

pub struct Chorus {
    buf: [Vec<f32>; 2],
    mask: usize,
    idx: usize,
    phase: f32,
    sr: f32,
}

impl Chorus {
    pub fn new(sr: f32) -> Self {
        let len = ((sr * 0.08) as usize).next_power_of_two();
        Self { buf: [vec![0.0; len], vec![0.0; len]], mask: len - 1, idx: 0, phase: 0.0, sr }
    }

    #[inline]
    fn read(&self, side: usize, delay: f32) -> f32 {
        let pos = self.idx as f32 - delay;
        let base = pos.floor();
        let t = pos - base;
        let i0 = (base as isize).rem_euclid(self.buf[side].len() as isize) as usize;
        let i1 = (i0 + 1) & self.mask;
        self.buf[side][i0] * (1.0 - t) + self.buf[side][i1] * t
    }

    /// Two delay lines swept by quadrature LFOs; wet signal is mixed into `out`.
    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        in_l: &[f32],
        in_r: &[f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
        rate_hz: f32,
        depth_ms: f32,
        delay_ms: f32,
        gain: f32,
    ) {
        let max_ms = (self.mask as f32 - 4.0) / self.sr * 1000.0;
        let delay_ms = delay_ms.clamp(1.0, max_ms * 0.5);
        let depth_ms = depth_ms.clamp(0.0, max_ms * 0.5);
        let inc = TAU * rate_hz.clamp(0.01, 10.0) / self.sr;
        let to_samples = self.sr / 1000.0;
        for i in 0..in_l.len() {
            self.buf[0][self.idx] = in_l[i];
            self.buf[1][self.idx] = in_r[i];
            let lfo_l = self.phase.sin();
            let lfo_r = (self.phase + PI / 2.0).sin();
            let dl = (delay_ms + depth_ms * 0.5 * (1.0 + lfo_l)) * to_samples;
            let dr = (delay_ms + depth_ms * 0.5 * (1.0 + lfo_r)) * to_samples;
            out_l[i] += self.read(0, dl) * gain;
            out_r[i] += self.read(1, dr) * gain;
            self.idx = (self.idx + 1) & self.mask;
            self.phase += inc;
            if self.phase > TAU {
                self.phase -= TAU;
            }
        }
    }
}
