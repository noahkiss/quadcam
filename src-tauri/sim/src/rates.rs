//! Rate curves: stick deflection to rotation rate, for every rate model a Betaflight `diff all`
//! can name, plus the throttle curve.
//!
//! The formulas are the published rate math (the Betaflight rate-model descriptions and the
//! numbers its Configurator shows), written from that description. propwash's `rates.ts` (MIT,
//! `LICENSES/propwash.txt`) was the structural reference. This is the one implementation:
//! the app's Rates segment re-exports it (sim-design 5.1).

use serde::{Deserialize, Serialize};

/// Betaflight's hard limit on any rate (deg/s).
pub const MAX_RATE_DEG_S: f64 = 1998.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RatesType {
    Betaflight,
    Raceflight,
    Kiss,
    Actual,
    Quick,
}

impl RatesType {
    /// The `rates_type` value of a `diff all`.
    pub fn from_cli(s: &str) -> Option<RatesType> {
        match s.trim().to_ascii_uppercase().as_str() {
            "BETAFLIGHT" => Some(RatesType::Betaflight),
            "RACEFLIGHT" => Some(RatesType::Raceflight),
            "KISS" => Some(RatesType::Kiss),
            "ACTUAL" => Some(RatesType::Actual),
            "QUICK" => Some(RatesType::Quick),
            _ => None,
        }
    }
}

/// One axis's three rate numbers, exactly as the CLI stores them (`roll_rc_rate`,
/// `roll_srate`, `roll_expo`). Their meaning depends on the model.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RateAxis {
    pub rc_rate: f64,
    pub srate: f64,
    pub expo: f64,
}

/// Rates for roll, pitch and yaw under one model.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rates {
    pub rates_type: RatesType,
    pub axes: [RateAxis; 3],
}

impl Default for Rates {
    /// Betaflight's defaults since 4.3: Actual, center 70, max 670, expo 0.54.
    fn default() -> Rates {
        let a = RateAxis {
            rc_rate: 7.0,
            srate: 67.0,
            expo: 54.0,
        };
        Rates {
            rates_type: RatesType::Actual,
            axes: [a; 3],
        }
    }
}

impl Rates {
    /// Rate (deg/s) for `axis` (0 roll, 1 pitch, 2 yaw) at stick `x` (-1..1).
    pub fn rate(&self, axis: usize, x: f64) -> f64 {
        rate(self.rates_type, &self.axes[axis], x)
    }
}

/// Rate (deg/s) for a stick deflection `x` in -1..1.
pub fn rate(model: RatesType, p: &RateAxis, x: f64) -> f64 {
    let x = x.clamp(-1.0, 1.0);
    let ax = x.abs();
    let r = match model {
        RatesType::Betaflight => {
            let mut rc = p.rc_rate / 100.0;
            if rc > 2.0 {
                rc += 14.54 * (rc - 2.0);
            }
            let e = p.expo / 100.0;
            let cmd = if e != 0.0 {
                x * ax.powi(3) * e + x * (1.0 - e)
            } else {
                x
            };
            let mut r = 200.0 * rc * cmd;
            if p.srate != 0.0 {
                r /= (1.0 - ax * p.srate / 100.0).clamp(0.01, 1.0);
            }
            r
        }
        RatesType::Actual => {
            let center = p.rc_rate * 10.0;
            let max = p.srate * 10.0;
            let e = p.expo / 100.0;
            let expof = ax * (x.powi(5) * e + x * (1.0 - e));
            x * center + (max - center).max(0.0) * expof
        }
        RatesType::Quick => {
            let center = p.rc_rate * 2.0;
            if center <= 0.0 {
                return 0.0;
            }
            let max = (p.srate * 10.0).max(center);
            let e = p.expo / 100.0;
            let ratio = max / center;
            let super_cfg = (ratio - 1.0) / ratio;
            let curve = ax.powi(3) * e + ax * (1.0 - e);
            let super_factor = 1.0 / (1.0 - curve * super_cfg).clamp(0.01, 1.0);
            x * center * super_factor
        }
        RatesType::Raceflight => {
            let rate = p.rc_rate * 10.0;
            let cmd = (1.0 + 0.01 * p.expo * (x * x - 1.0)) * x;
            cmd * (rate + cmd.abs() * rate * p.srate * 0.01)
        }
        RatesType::Kiss => {
            let rc = p.rc_rate / 100.0;
            let curve = p.expo / 100.0;
            let use_rates = 1.0 / (1.0 - ax * p.srate / 100.0).clamp(0.01, 1.0);
            let cmd = (x.powi(3) * curve + x * (1.0 - curve)) * (rc / 10.0);
            2000.0 * use_rates * cmd
        }
    };
    r.clamp(-MAX_RATE_DEG_S, MAX_RATE_DEG_S)
}

/// Throttle curve and limit: `thr_mid`, `thr_expo` (0-100), Betaflight 2025's `thr_hover`
/// (the output at `thr_mid`; equal to `thr_mid` when absent), and `throttle_limit_type` with
/// `throttle_limit_percent`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ThrottleCurve {
    pub mid: f64,
    pub expo: f64,
    pub hover: Option<f64>,
    pub limit: ThrottleLimit,
    pub limit_percent: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThrottleLimit {
    Off,
    Scale,
    Clip,
}

impl Default for ThrottleCurve {
    fn default() -> ThrottleCurve {
        ThrottleCurve {
            mid: 50.0,
            expo: 0.0,
            hover: None,
            limit: ThrottleLimit::Off,
            limit_percent: 100.0,
        }
    }
}

impl ThrottleCurve {
    /// Throttle stick 0..1 to throttle output 0..1.
    pub fn apply(&self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        let mid = (self.mid / 100.0).clamp(0.01, 0.99);
        let hover = (self.hover.unwrap_or(self.mid) / 100.0).clamp(0.0, 1.0);
        let e = (self.expo / 100.0).clamp(0.0, 1.0);
        let g = |x: f64| x * (1.0 - e + e * x * x);
        let out = if t < mid {
            hover + hover * g((t - mid) / mid)
        } else {
            hover + (1.0 - hover) * g((t - mid) / (1.0 - mid))
        };
        let lim = (self.limit_percent / 100.0).clamp(0.0, 1.0);
        match self.limit {
            ThrottleLimit::Off => out,
            ThrottleLimit::Scale => out * lim,
            ThrottleLimit::Clip => out.min(lim),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis(rc_rate: f64, srate: f64, expo: f64) -> RateAxis {
        RateAxis {
            rc_rate,
            srate,
            expo,
        }
    }

    fn near(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "{a} vs {b}");
    }

    // Reference values: the max rates the Betaflight Configurator shows for these settings,
    // and hand-worked points on each published curve.

    #[test]
    fn betaflight_rates_match_reference_values() {
        let m = RatesType::Betaflight;
        // Configurator: RC 1.00, super 0.70, expo 0 -> max 667 deg/s.
        let p = axis(100.0, 70.0, 0.0);
        near(rate(m, &p, 1.0), 666.67, 0.01);
        near(rate(m, &p, 0.5), 100.0 / 0.65, 0.01);
        near(rate(m, &p, -1.0), -666.67, 0.01);
        // RC 1.27, super 0.72, expo 0.40 -> max 907 deg/s.
        let p = axis(127.0, 72.0, 40.0);
        near(rate(m, &p, 1.0), 907.14, 0.01);
        // x = 0.5: cmd = 0.5 * (0.125 * 0.4) + 0.5 * 0.6 = 0.325; 254 * 0.325 / (1 - 0.36).
        near(rate(m, &p, 0.5), 254.0 * 0.325 / 0.64, 0.01);
        // RC rate above 2.0 is boosted: 2.20 -> 2.2 + 14.54 * 0.2 = 5.108.
        let p = axis(220.0, 0.0, 0.0);
        near(rate(m, &p, 1.0), 1021.6, 0.01);
        near(rate(m, &p, 0.0), 0.0, 1e-12);
    }

    #[test]
    fn actual_rates_match_reference_values() {
        let m = RatesType::Actual;
        // Betaflight 4.3+ defaults: center 70, max 670, expo 0.54.
        let p = axis(7.0, 67.0, 54.0);
        near(rate(m, &p, 1.0), 670.0, 1e-9);
        // x = 0.5: expof = 0.5 * (0.03125 * 0.54 + 0.5 * 0.46) = 0.1234375.
        near(rate(m, &p, 0.5), 35.0 + 600.0 * 0.1234375, 1e-9);
        // Slope at center is the center sensitivity: 70 deg/s per full stick.
        near(rate(m, &p, 1e-6) / 1e-6, 70.0, 0.01);
        // Max below center: linear at center sensitivity.
        let p = axis(20.0, 10.0, 0.0);
        near(rate(m, &p, 1.0), 200.0, 1e-9);
    }

    #[test]
    fn quick_rates_match_reference_values() {
        let m = RatesType::Quick;
        // RC rate 1.00 (200 deg/s center), max 670, expo 0.
        let p = axis(100.0, 67.0, 0.0);
        near(rate(m, &p, 1.0), 670.0, 1e-9);
        let cfg = (3.35 - 1.0) / 3.35;
        near(rate(m, &p, 0.5), 100.0 / (1.0 - 0.5 * cfg), 1e-9);
        near(rate(m, &p, 0.001) / 0.001, 200.0, 0.5);
        // Expo flattens the middle but keeps the end points.
        let e = axis(100.0, 67.0, 50.0);
        near(rate(m, &e, 1.0), 670.0, 1e-9);
        assert!(rate(m, &e, 0.5) < rate(m, &p, 0.5));
    }

    #[test]
    fn raceflight_and_kiss_presets() {
        // RaceFlight: rate 370, acro+ 80, expo 50 -> full stick 370 * 1.8.
        near(
            rate(RatesType::Raceflight, &axis(37.0, 80.0, 50.0), 1.0),
            666.0,
            1e-9,
        );
        // KISS: RC 1.00, rate 0.70, curve 0.40 -> 2000 * 0.1 / 0.3.
        near(
            rate(RatesType::Kiss, &axis(100.0, 70.0, 40.0), 1.0),
            666.67,
            0.01,
        );
    }

    #[test]
    fn rates_are_odd_monotonic_and_limited() {
        let p = axis(250.0, 99.0, 0.0);
        near(rate(RatesType::Betaflight, &p, 1.0), MAX_RATE_DEG_S, 1e-9);
        for m in [
            RatesType::Betaflight,
            RatesType::Actual,
            RatesType::Quick,
            RatesType::Raceflight,
            RatesType::Kiss,
        ] {
            let p = Rates::default().axes[0];
            let p = if m == RatesType::Actual {
                p
            } else {
                axis(100.0, 70.0, 30.0)
            };
            let mut last = f64::MIN;
            for i in -100..=100 {
                let x = i as f64 / 100.0;
                let r = rate(m, &p, x);
                assert!(r >= last, "{m:?} not monotonic at {x}");
                near(r, -rate(m, &p, -x), 1e-9);
                last = r;
            }
        }
    }

    #[test]
    fn throttle_curve_hits_mid_hover_and_ends() {
        let c = ThrottleCurve {
            mid: 30.0,
            expo: 50.0,
            hover: Some(34.0),
            ..ThrottleCurve::default()
        };
        near(c.apply(0.0), 0.0, 1e-12);
        near(c.apply(0.3), 0.34, 1e-12);
        near(c.apply(1.0), 1.0, 1e-12);
        // Expo flattens the curve around the hover point.
        assert!((c.apply(0.35) - 0.34).abs() < 0.05 * 0.66);
        let lin = ThrottleCurve::default();
        near(lin.apply(0.42), 0.42, 1e-12);
        let clip = ThrottleCurve {
            limit: ThrottleLimit::Clip,
            limit_percent: 80.0,
            ..ThrottleCurve::default()
        };
        near(clip.apply(1.0), 0.8, 1e-12);
        near(clip.apply(0.5), 0.5, 1e-12);
        let scale = ThrottleCurve {
            limit: ThrottleLimit::Scale,
            limit_percent: 80.0,
            ..ThrottleCurve::default()
        };
        near(scale.apply(0.5), 0.4, 1e-12);
    }
}
