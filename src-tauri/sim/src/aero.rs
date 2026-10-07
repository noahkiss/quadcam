//! Aerodynamics (sim-design 4.3-4.6, 4.9): prop thrust with inflow, ground effect, vortex
//! ring state, prop wash, duct momentum drag, H-force, body drag and wind.
//!
//! Sources: momentum theory and the advance-ratio thrust form, Cheeseman-Bennett ground
//! effect, the VRS band from Leishman's induced-velocity curve, rotor momentum drag and
//! H-force, quadratic body drag and Dryden turbulence are published physics (fact). The
//! prop-wash severity and noise model and the Dryden-lite wind are ported from propwash's
//! `PropWash.ts` and `Aero.ts` (MIT, `LICENSES/propwash.txt`).

use crate::filters::{BandPass, Pt1};
use crate::rng::Rng;

pub const RHO: f64 = crate::profile::RHO_SEA_LEVEL;

fn smoothstep(x: f64) -> f64 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Ground effect T_IGE / T_OGE = 1 / (1 − (R / 4z)²), clamped at its value for z = R/2.
pub fn ground_effect(height: f64, radius: f64) -> f64 {
    let z = height.max(radius / 2.0);
    let k = radius / (4.0 * z);
    1.0 / (1.0 - k * k)
}

/// Thrust factor from axial inflow: CT(J) / CT0 = 1 − J / J0, with J = V_axial / (n·D).
/// `v_in` is positive when air flows into the disc from above (climbing). Descent raises
/// thrust, up to a cap; the vortex ring state is applied separately.
pub fn inflow_factor(v_in: f64, omega: f64, diameter: f64, j0: f64) -> f64 {
    let n = omega.abs() / std::f64::consts::TAU;
    if n < 1.0 {
        return 1.0;
    }
    let j = v_in / (n * diameter);
    (1.0 - j / j0).clamp(0.0, 1.3)
}

/// Induced velocity at the disc for thrust `t` (N): √(T / 2ρA).
pub fn induced_velocity(thrust: f64, radius: f64) -> f64 {
    let area = std::f64::consts::PI * radius * radius;
    (thrust.max(0.0) / (2.0 * RHO * area)).sqrt()
}

/// The VRS band, as multiples of the hover induced velocity: thrust loss ramps in from
/// `ON0` to `ON1`, holds, and ramps out from `OFF0` to `OFF1`.
const VRS_ON: (f64, f64) = (0.4, 0.6);
const VRS_OFF: (f64, f64) = (1.4, 1.7);
/// Full-severity VRS thrust loss.
pub const VRS_LOSS: f64 = 0.3;

/// VRS severity 0..1 for a rotor descending axially at `v_desc` (m/s, positive down along
/// the rotor axis) with `v_plane` in-plane airspeed, against hover induced velocity `vh`.
pub fn vrs_severity(v_desc: f64, v_plane: f64, vh: f64) -> f64 {
    if vh <= 0.0 {
        return 0.0;
    }
    let r = v_desc / vh;
    let band = smoothstep((r - VRS_ON.0) / (VRS_ON.1 - VRS_ON.0))
        * (1.0 - smoothstep((r - VRS_OFF.0) / (VRS_OFF.1 - VRS_OFF.0)));
    // Horizontal speed carries the rotor out of its own wake: gone by one vh in-plane.
    let fade = 1.0 - smoothstep(v_plane / vh);
    band * fade
}

/// Prop wash: per-rotor severity from descending into the wake, faded by horizontal speed,
/// costing a little thrust and adding band-limited (10-40 Hz) noise. One amplitude, the
/// slider. The random stream is drawn every step, used or not.
#[derive(Debug, Clone)]
pub struct PropWash {
    noise: [BandPass; 4],
    smooth: [Pt1; 4],
    gain: f64,
    rng: Rng,
    pub severity: [f64; 4],
}

const WASH_BAND: (f64, f64) = (10.0, 40.0);
/// Onset and full severity, as multiples of hover induced velocity of descent.
const WASH_ONSET: f64 = 0.2;
const WASH_FULL: f64 = 0.6;
const WASH_LOSS: f64 = 0.05;

impl PropWash {
    pub fn new(rng: Rng, dt: f64) -> PropWash {
        let mk = || BandPass::new(WASH_BAND.0, WASH_BAND.1, dt);
        let sm = || Pt1::new(8.0, dt);
        PropWash {
            noise: [mk(), mk(), mk(), mk()],
            smooth: [sm(), sm(), sm(), sm()],
            gain: 1.0 / BandPass::white_noise_rms(WASH_BAND.0, WASH_BAND.1, dt),
            rng,
            severity: [0.0; 4],
        }
    }

    /// Thrust multiplier for rotor `i`; `amplitude` is the noise RMS at full severity, as a
    /// fraction of thrust.
    pub fn multiplier(
        &mut self,
        i: usize,
        v_desc: f64,
        v_plane: f64,
        vh: f64,
        amplitude: f64,
    ) -> f64 {
        let n = self.noise[i].apply(self.rng.gaussian()) * self.gain;
        let raw = if vh > 0.0 {
            smoothstep((v_desc / vh - WASH_ONSET) / (WASH_FULL - WASH_ONSET))
                * (1.0 - smoothstep(v_plane / (2.0 * vh)))
        } else {
            0.0
        };
        let s = self.smooth[i].apply(raw);
        self.severity[i] = s;
        (1.0 - WASH_LOSS * s + amplitude * s * n).max(0.0)
    }

    pub fn rng_state(&self) -> [u64; 4] {
        self.rng.state_words()
    }
}

/// Quadratic body drag per body axis: F_i = −½ ρ · CdA_i · |v| · v_i (body frame).
pub fn body_drag(v_air_body: [f64; 3], cda: [f64; 3]) -> [f64; 3] {
    let speed = (v_air_body[0].powi(2) + v_air_body[1].powi(2) + v_air_body[2].powi(2)).sqrt();
    let k = -0.5 * RHO * speed;
    [
        k * cda[0] * v_air_body[0],
        k * cda[1] * v_air_body[1],
        k * cda[2] * v_air_body[2],
    ]
}

/// Ducted rotor momentum drag: air entering sideways leaves axially. F = −c · ṁ · v_inplane,
/// ṁ = ρ · A · v_induced (one rotor; `v_plane` is the in-plane air velocity, body frame).
pub fn duct_drag(c: f64, thrust: f64, radius: f64, v_plane: [f64; 2]) -> [f64; 2] {
    let area = std::f64::consts::PI * radius * radius;
    let mdot = RHO * area * induced_velocity(thrust, radius);
    [-c * mdot * v_plane[0], -c * mdot * v_plane[1]]
}

/// Open-prop H-force (rotor drag): linear in rotor speed and in-plane velocity.
pub fn h_force(c_h: f64, omega: f64, v_plane: [f64; 2]) -> [f64; 2] {
    let w = omega.abs();
    [-c_h * w * v_plane[0], -c_h * w * v_plane[1]]
}

/// Wind: a mean vector plus Dryden-lite gusts, each axis a first-order Gauss-Markov process
/// with σ and a correlation time L / airspeed. Off by default (still air).
#[derive(Debug, Clone)]
pub struct Wind {
    pub mean: [f64; 3],
    pub sigma: f64,
    gust: [f64; 3],
    rng: Rng,
}

const GUST_LENGTH: (f64, f64) = (30.0, 10.0);
const GUST_MIN_SPEED: f64 = 1.0;

impl Wind {
    pub fn new(rng: Rng) -> Wind {
        Wind {
            mean: [0.0; 3],
            sigma: 0.0,
            gust: [0.0; 3],
            rng,
        }
    }

    pub fn velocity(&self) -> [f64; 3] {
        [
            self.mean[0] + self.gust[0],
            self.mean[1] + self.gust[1],
            self.mean[2] + self.gust[2],
        ]
    }

    pub fn step(&mut self, dt: f64, airspeed: f64) {
        let n = [
            self.rng.gaussian(),
            self.rng.gaussian(),
            self.rng.gaussian(),
        ];
        if self.sigma <= 0.0 {
            self.gust = [0.0; 3];
            return;
        }
        let mean_speed = (self.mean[0].powi(2) + self.mean[1].powi(2)).sqrt();
        let speed = airspeed.max(mean_speed).max(GUST_MIN_SPEED);
        let axis = |g: f64, s: f64, l: f64, noise: f64| {
            let a = (-dt * speed / l).exp();
            g * a + s * (1.0 - a * a).sqrt() * noise
        };
        self.gust = [
            axis(self.gust[0], self.sigma, GUST_LENGTH.0, n[0]),
            axis(self.gust[1], self.sigma, GUST_LENGTH.0, n[1]),
            axis(self.gust[2], self.sigma * 0.5, GUST_LENGTH.1, n[2]),
        ];
    }

    pub fn rng_state(&self) -> [u64; 4] {
        self.rng.state_words()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_effect_at_one_radius_and_far_away() {
        let r = 0.0225;
        // h = R: 1 / (1 − 1/16).
        assert!((ground_effect(r, r) - 16.0 / 15.0).abs() < 1e-12);
        assert!((ground_effect(10.0 * r, r) - 1.0).abs() < 1e-3);
        // Clamped near the ground.
        assert_eq!(ground_effect(0.0, r), ground_effect(r / 2.0, r));
        assert!((ground_effect(0.0, r) - 4.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn inflow_reduces_thrust_climbing_and_raises_it_descending() {
        let (w, d, j0) = (2000.0, 0.045, 0.72);
        assert_eq!(inflow_factor(0.0, w, d, j0), 1.0);
        let n = w / std::f64::consts::TAU;
        // Zero thrust at J0.
        assert!(inflow_factor(j0 * n * d, w, d, j0).abs() < 1e-12);
        assert!(inflow_factor(3.0, w, d, j0) < 1.0);
        assert!(inflow_factor(-3.0, w, d, j0) > 1.0);
        assert_eq!(inflow_factor(-100.0, w, d, j0), 1.3);
    }

    #[test]
    fn vrs_loses_thrust_in_its_band_and_not_outside() {
        // A 75 mm whoop: hover induced velocity about 6 m/s, band roughly 3-9 m/s.
        let vh = induced_velocity(0.056 * 9.80665 / 4.0, 0.0225);
        assert!((5.0..7.0).contains(&vh), "vh {vh}");
        for v in [0.0, 1.0, 2.0, 12.0, 15.0] {
            assert_eq!(vrs_severity(v, 0.0, vh), 0.0, "descent {v}");
        }
        for v in [4.0, 6.0, 8.0] {
            assert!(vrs_severity(v, 0.0, vh) > 0.99, "descent {v}");
        }
        // Climbing is never VRS; horizontal speed takes the rotor out of the band.
        assert_eq!(vrs_severity(-6.0, 0.0, vh), 0.0);
        assert_eq!(vrs_severity(6.0, vh, vh), 0.0);
        assert!(vrs_severity(6.0, 0.5 * vh, vh) < 0.6);
    }

    #[test]
    fn prop_wash_is_quiet_level_and_rough_in_the_wake() {
        let dt = 5e-4;
        let mut w = PropWash::new(Rng::new(3), dt);
        let vh = 6.0;
        let mut level = Vec::new();
        for _ in 0..4000 {
            level.push(w.multiplier(0, 0.0, 0.0, vh, 0.15));
        }
        assert!(level.iter().all(|m| (*m - 1.0).abs() < 1e-9));
        let mut wake = Vec::new();
        for _ in 0..8000 {
            wake.push(w.multiplier(0, 6.0, 0.0, vh, 0.15));
        }
        let tail = &wake[4000..];
        let mean = tail.iter().sum::<f64>() / tail.len() as f64;
        let rms = (tail.iter().map(|m| (m - mean).powi(2)).sum::<f64>() / tail.len() as f64).sqrt();
        assert!((mean - 0.95).abs() < 0.02, "{mean}");
        assert!((rms - 0.15).abs() < 0.05, "{rms}");
    }

    #[test]
    fn drag_opposes_motion() {
        let f = body_drag([5.0, -2.0, 1.0], [0.003, 0.003, 0.008]);
        assert!(f[0] < 0.0 && f[1] > 0.0 && f[2] < 0.0);
        let d = duct_drag(1.0, 0.137, 0.0225, [3.0, 0.0]);
        assert!(d[0] < 0.0 && d[1] == 0.0);
        let h = h_force(2e-6, 1000.0, [0.0, -4.0]);
        assert!(h[1] > 0.0);
    }

    #[test]
    fn still_air_by_default_and_gusts_have_sigma() {
        let mut w = Wind::new(Rng::new(5));
        for _ in 0..1000 {
            w.step(5e-4, 3.0);
        }
        assert_eq!(w.velocity(), [0.0; 3]);
        w.sigma = 1.0;
        let mut s2 = 0.0;
        let n = 400_000;
        for _ in 0..n {
            w.step(5e-3, 5.0);
            s2 += w.velocity()[0].powi(2);
        }
        let sd = (s2 / n as f64).sqrt();
        assert!((sd - 1.0).abs() < 0.15, "{sd}");
    }
}
