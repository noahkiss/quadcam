//! The mixer: collective thrust plus roll, pitch and yaw torque to four motor outputs.
//!
//! The allocation matrix maps per-motor thrust to [thrust, τx, τy, τz] from geometry
//! (τ = r × T) and spin (yaw = −s · kQ/kT · T); its inverse turns a request into per-motor
//! thrust, then speed, then ESC duty through the motor model's steady state. Airmode keeps
//! the differential at zero throttle by shifting the collective. After propwash's
//! `mixer.ts` (MIT, `LICENSES/propwash.txt`).

use crate::motor::Motors;
use crate::profile::Params;

#[derive(Debug, Clone)]
pub struct Mixer {
    inv: [[f64; 4]; 4],
    kt: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mix {
    /// Mixer output per motor, 0..1 (0 is idle).
    pub u: [f64; 4],
    /// The request did not fit: the collective shifted or the differential was scaled.
    pub saturated: bool,
}

fn invert(m: [[f64; 4]; 4]) -> [[f64; 4]; 4] {
    let mut a = [[0.0; 8]; 4];
    for i in 0..4 {
        a[i][..4].copy_from_slice(&m[i]);
        a[i][4 + i] = 1.0;
    }
    for c in 0..4 {
        let mut p = c;
        for r in c + 1..4 {
            if a[r][c].abs() > a[p][c].abs() {
                p = r;
            }
        }
        assert!(a[p][c].abs() > 1e-15, "mixer: singular allocation matrix");
        a.swap(c, p);
        let d = a[c][c];
        for x in a[c].iter_mut() {
            *x /= d;
        }
        for r in 0..4 {
            if r != c {
                let f = a[r][c];
                let row_c = a[c];
                for (x, y) in a[r].iter_mut().zip(row_c.iter()) {
                    *x -= f * y;
                }
            }
        }
    }
    let mut out = [[0.0; 4]; 4];
    for i in 0..4 {
        out[i].copy_from_slice(&a[i][4..]);
    }
    out
}

impl Mixer {
    pub fn new(p: &Params) -> Mixer {
        let mut alloc = [[0.0; 4]; 4];
        for i in 0..4 {
            let [x, y, _] = p.rotor_pos[i];
            alloc[0][i] = 1.0;
            alloc[1][i] = y;
            alloc[2][i] = -x;
            alloc[3][i] = -p.rotor_spin[i] * p.kq / p.kt;
        }
        Mixer {
            inv: invert(alloc),
            kt: p.kt,
        }
    }

    fn allocate(&self, w: [f64; 4]) -> [f64; 4] {
        let mut f = [0.0; 4];
        for (i, row) in self.inv.iter().enumerate() {
            f[i] = row.iter().zip(w.iter()).map(|(k, x)| k * x).sum();
        }
        f
    }

    /// Thrust (N) of one motor at mixer output `u` and pack voltage `v`.
    pub fn thrust_at(&self, motors: &Motors, u: f64, v: f64) -> f64 {
        let w = motors.speed_for_duty(motors.duty_for(u), v);
        self.kt * w * w
    }

    /// Mix collective `throttle` (0..1, a motor output like Betaflight's throttle) and
    /// `torque` (N·m, body axes) at pack voltage `v`.
    pub fn mix(
        &self,
        motors: &Motors,
        throttle: f64,
        torque: [f64; 3],
        v: f64,
        airmode: bool,
    ) -> Mix {
        let f_idle = self.thrust_at(motors, 0.0, v);
        let f_max = self.thrust_at(motors, 1.0, v);
        let range = f_max - f_idle;
        let f0 = self.thrust_at(motors, throttle.clamp(0.0, 1.0), v);
        let rp = self.allocate([0.0, torque[0], torque[1], 0.0]);
        let yw = self.allocate([0.0, 0.0, 0.0, torque[2]]);
        let spread = |d: &[f64; 4]| {
            d.iter().cloned().fold(f64::MIN, f64::max) - d.iter().cloned().fold(f64::MAX, f64::min)
        };
        let mut saturated = false;
        let mut diff: [f64; 4] = std::array::from_fn(|i| rp[i] + yw[i]);
        if spread(&diff) > range {
            saturated = true;
            if spread(&rp) >= range {
                let k = range / spread(&rp);
                diff = rp.map(|x| x * k);
            } else {
                // Keep roll and pitch whole; scale yaw down until it fits.
                let (mut lo, mut hi) = (0.0, 1.0);
                for _ in 0..30 {
                    let mid = (lo + hi) / 2.0;
                    let d: [f64; 4] = std::array::from_fn(|i| rp[i] + mid * yw[i]);
                    if spread(&d) <= range {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                diff = std::array::from_fn(|i| rp[i] + lo * yw[i]);
            }
        }
        let dmin = diff.iter().cloned().fold(f64::MAX, f64::min);
        let dmax = diff.iter().cloned().fold(f64::MIN, f64::max);
        let lo = f_idle - dmin;
        let hi = f_max - dmax;
        let mut base = f0;
        if airmode {
            if base < lo || base > hi {
                saturated = true;
            }
            // lo can pass hi by rounding when the differential fills the whole range.
            base = if lo <= hi {
                base.clamp(lo, hi)
            } else {
                0.5 * (lo + hi)
            };
        } else if base > hi {
            saturated = true;
            base = hi;
        }
        let idle = motors.duty_for(0.0);
        let u = diff.map(|d| {
            let f = (base + d).max(0.0);
            let w = (f / self.kt).sqrt();
            let duty = motors.duty_for_speed(w, v);
            ((duty - idle) / (1.0 - idle)).clamp(0.0, 1.0)
        });
        Mix { u, saturated }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::preset;

    fn setup() -> (Params, Motors, Mixer) {
        let p = preset("meteor75").unwrap().params();
        let m = Motors::new(&p);
        let x = Mixer::new(&p);
        (p, m, x)
    }

    #[test]
    fn collective_alone_is_even() {
        let (_, m, x) = setup();
        let r = x.mix(&m, 0.4, [0.0; 3], 3.9, true);
        for u in r.u {
            assert!((u - 0.4).abs() < 1e-9, "{u}");
        }
        assert!(!r.saturated);
    }

    #[test]
    fn torques_come_out_on_the_right_motors() {
        let (p, m, x) = setup();
        // Roll right (+x): left motors (y > 0) push harder.
        let r = x.mix(&m, 0.4, [1e-3, 0.0, 0.0], 3.9, true);
        for i in 0..4 {
            let left = p.rotor_pos[i][1] > 0.0;
            assert_eq!(r.u[i] > 0.4, left, "motor {i}");
        }
        // Pitch nose down (+y): rear motors (x < 0) push harder.
        let r = x.mix(&m, 0.4, [0.0, 1e-3, 0.0], 3.9, true);
        for i in 0..4 {
            assert_eq!(r.u[i] > 0.4, p.rotor_pos[i][0] < 0.0, "motor {i}");
        }
        // Yaw left (+z): clockwise rotors (spin −1) speed up; their drag turns the body CCW.
        let r = x.mix(&m, 0.4, [0.0, 0.0, 1e-4], 3.9, true);
        for i in 0..4 {
            assert_eq!(r.u[i] > 0.4, p.rotor_spin[i] < 0.0, "motor {i}");
        }
    }

    #[test]
    fn airmode_keeps_authority_at_zero_throttle() {
        let (_, m, x) = setup();
        let on = x.mix(&m, 0.0, [2e-3, 0.0, 0.0], 3.9, true);
        let off = x.mix(&m, 0.0, [2e-3, 0.0, 0.0], 3.9, false);
        let spread = |u: [f64; 4]| {
            u.iter().cloned().fold(f64::MIN, f64::max) - u.iter().cloned().fold(f64::MAX, f64::min)
        };
        assert!(on.u.iter().all(|u| *u >= 0.0));
        assert!(spread(on.u) > 0.05);
        // Without airmode the low side clips at idle and the differential is lost.
        assert!(off.u.contains(&0.0));
        assert!(spread(off.u) < spread(on.u));
    }
}
