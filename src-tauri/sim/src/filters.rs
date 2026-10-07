//! Fixed-step digital filters: PT1, PT3 and a biquad band-pass.
//!
//! Ported from propwash's `src/sim/fc/filters.ts` (MIT, see `LICENSES/propwash.txt`). The
//! filter equations themselves are textbook (RC low-pass, RBJ cookbook band-pass).

use std::f64::consts::{PI, TAU};

/// First-order low-pass.
#[derive(Debug, Clone)]
pub struct Pt1 {
    k: f64,
    dt: f64,
    pub y: f64,
}

impl Pt1 {
    pub fn new(cutoff_hz: f64, dt: f64) -> Pt1 {
        Pt1 {
            k: Pt1::gain(cutoff_hz, dt),
            dt,
            y: 0.0,
        }
    }

    pub fn gain(cutoff_hz: f64, dt: f64) -> f64 {
        let rc = 1.0 / (TAU * cutoff_hz);
        dt / (rc + dt)
    }

    pub fn set_cutoff(&mut self, cutoff_hz: f64) {
        self.k = Pt1::gain(cutoff_hz, self.dt);
    }

    pub fn apply(&mut self, x: f64) -> f64 {
        self.y += self.k * (x - self.y);
        self.y
    }

    pub fn reset(&mut self, v: f64) {
        self.y = v;
    }
}

/// Third-order low-pass: three PT1s, each stage's cutoff raised so the whole filter's -3 dB
/// point stays at `cutoff_hz`.
#[derive(Debug, Clone)]
pub struct Pt3 {
    stages: [Pt1; 3],
}

/// The per-stage cutoff of a PT3 with -3 dB at `cutoff_hz`.
fn pt3_stage_hz(cutoff_hz: f64) -> f64 {
    cutoff_hz / (2f64.powf(1.0 / 3.0) - 1.0).sqrt()
}

impl Pt3 {
    pub fn new(cutoff_hz: f64, dt: f64) -> Pt3 {
        let c = pt3_stage_hz(cutoff_hz);
        Pt3 {
            stages: [Pt1::new(c, dt), Pt1::new(c, dt), Pt1::new(c, dt)],
        }
    }

    pub fn apply(&mut self, x: f64) -> f64 {
        let a = self.stages[0].apply(x);
        let b = self.stages[1].apply(a);
        self.stages[2].apply(b)
    }

    pub fn y(&self) -> f64 {
        self.stages[2].y
    }

    pub fn reset(&mut self, v: f64) {
        for s in &mut self.stages {
            s.reset(v);
        }
    }

    /// Low-frequency group delay (s): three stages of 1 / (2 pi f_stage).
    pub fn delay_s(cutoff_hz: f64) -> f64 {
        3.0 / (2.0 * PI * pt3_stage_hz(cutoff_hz))
    }
}

/// Biquad band-pass (RBJ cookbook, 0 dB peak), for band-limited noise.
#[derive(Debug, Clone)]
pub struct BandPass {
    b0: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl BandPass {
    pub fn new(lo_hz: f64, hi_hz: f64, dt: f64) -> BandPass {
        let f0 = (lo_hz * hi_hz).sqrt();
        let q = f0 / (hi_hz - lo_hz);
        let w = TAU * f0 * dt;
        let alpha = w.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        BandPass {
            b0: alpha / a0,
            b2: -alpha / a0,
            a1: -2.0 * w.cos() / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    pub fn apply(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    /// The RMS this filter gives on unit white noise, so callers can normalise to unit RMS.
    pub fn white_noise_rms(lo_hz: f64, hi_hz: f64, dt: f64) -> f64 {
        let mut f = BandPass::new(lo_hz, hi_hz, dt);
        let mut r = crate::rng::Rng::new(0x9a5b);
        let (n, skip) = (40_000, 2_000);
        let mut sum = 0.0;
        for i in 0..n {
            let y = f.apply(r.gaussian());
            if i >= skip {
                sum += y * y;
            }
        }
        (sum / (n - skip) as f64).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pt1_reaches_63_percent_after_one_time_constant() {
        let dt = 1e-4;
        let hz = 10.0;
        let mut f = Pt1::new(hz, dt);
        let tau = 1.0 / (TAU * hz);
        let n = (tau / dt).round() as usize;
        let mut y = 0.0;
        for _ in 0..n {
            y = f.apply(1.0);
        }
        assert!((y - 0.632).abs() < 0.01, "{y}");
    }

    #[test]
    fn pt3_is_minus_3db_at_cutoff() {
        let dt = 1.0 / 8000.0;
        let hz = 40.0;
        let mut f = Pt3::new(hz, dt);
        let mut peak: f64 = 0.0;
        for i in 0..80_000 {
            let t = i as f64 * dt;
            let y = f.apply((TAU * hz * t).sin());
            if i > 40_000 {
                peak = peak.max(y.abs());
            }
        }
        assert!(
            (peak - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.02,
            "{peak}"
        );
    }
}
