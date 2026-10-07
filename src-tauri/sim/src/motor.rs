//! The four ESC + motor + prop units (sim-design 4.2).
//!
//! The electrical model: an ESC applies duty × pack voltage to the winding;
//! winding current I = (d·V − ω/Kv) / R; motor torque (I − I0·sgn ω) / Kv; the rotor obeys
//! J·dω/dt = motor torque − kQ·ω·|ω|. The pack sees d·I per motor. The time constant falls
//! out of the model. These are the DC-motor equations (fact).
//!
//! The fallback for a profile with no fit is a first-order lag to a linear speed map with
//! separate up and down time constants, after propwash's `FlightMotors.ts` (MIT,
//! `LICENSES/propwash.txt`).

use crate::profile::{MotorKind, Params};

/// Efficiency of shaft power to pack power in the first-order fallback (estimate).
const FALLBACK_EFFICIENCY: f64 = 0.7;
/// Below this speed (rad/s) friction torque fades to zero, so a stopped rotor stays stopped.
const FRICTION_EPS: f64 = 5.0;
/// Coast-down time constant of an unpowered fallback motor (s).
const FALLBACK_COAST_TAU: f64 = 0.3;

#[derive(Debug, Clone)]
pub struct Motors {
    pub omega: [f64; 4],
    /// dω/dt over the last step (rad/s²): the rotor-inertia yaw kick and audio transients.
    pub domega: [f64; 4],
    /// Winding current (A) and pack current per motor (A), last step.
    pub winding_current: [f64; 4],
    pub pack_current: [f64; 4],
    kind: MotorKind,
    ke: f64,
    kv_rads: f64,
    kq: f64,
    idle_duty: f64,
}

impl Motors {
    pub fn new(p: &Params) -> Motors {
        Motors {
            omega: [0.0; 4],
            domega: [0.0; 4],
            winding_current: [0.0; 4],
            pack_current: [0.0; 4],
            kind: p.motor,
            ke: 1.0 / p.kv_rads,
            kv_rads: p.kv_rads,
            kq: p.kq,
            idle_duty: p.idle_duty,
        }
    }

    /// ESC duty for a mixer output `u` (0..1 forward; −1..0 reversed for turtle, where 0
    /// stops the motor and there is no idle).
    pub fn duty_for(&self, u: f64) -> f64 {
        if u < 0.0 {
            u.max(-1.0)
        } else {
            self.idle_duty + (1.0 - self.idle_duty) * u.min(1.0)
        }
    }

    /// The pack current as `slope · V + offset` for these duties at the present speeds, so
    /// the battery can solve its loaded voltage exactly.
    pub fn pack_current_affine(&self, duty: &[f64; 4], driven: bool, v_prev: f64) -> (f64, f64) {
        if !driven {
            return (0.0, 0.0);
        }
        match self.kind {
            MotorKind::Electrical { r, .. } => {
                let mut slope = 0.0;
                let mut offset = 0.0;
                for i in 0..4 {
                    slope += duty[i] * duty[i] / r;
                    offset -= duty[i] * self.ke * self.omega[i] / r;
                }
                (slope, offset)
            }
            MotorKind::FirstOrder { .. } => {
                let p: f64 = (0..4).map(|i| self.kq * self.omega[i].abs().powi(3)).sum();
                (0.0, p / (FALLBACK_EFFICIENCY * v_prev.max(0.5)))
            }
        }
    }

    /// Advance one step. `driven`: the ESCs drive the motors (armed); otherwise they coast.
    pub fn step(&mut self, dt: f64, duty: &[f64; 4], v: f64, driven: bool) {
        for i in 0..4 {
            let w = self.omega[i];
            let prev = w;
            match self.kind {
                MotorKind::Electrical { r, i0, j } => {
                    let d = if driven { duty[i] } else { 0.0 };
                    let current = if driven {
                        (d * v - self.ke * w) / r
                    } else {
                        0.0
                    };
                    let friction = i0 * (w / FRICTION_EPS).clamp(-1.0, 1.0);
                    let torque = self.ke * (current - friction) - self.kq * w * w.abs();
                    let mut next = w + dt * torque / j;
                    // Unpowered friction and drag only slow the rotor, never reverse it.
                    if !driven && (next * w < 0.0 || next.abs() < 1.0) {
                        next = 0.0;
                    }
                    self.omega[i] = next;
                    self.winding_current[i] = current;
                    self.pack_current[i] = d * current;
                }
                MotorKind::FirstOrder {
                    tau_up,
                    tau_down,
                    fraction,
                    idle_rads,
                } => {
                    if driven && v > 0.0 {
                        let d = duty[i];
                        let full = fraction * self.kv_rads * v;
                        let goal = if d < 0.0 {
                            d * full
                        } else {
                            let u = ((d - self.idle_duty) / (1.0 - self.idle_duty)).clamp(0.0, 1.0);
                            idle_rads + (full - idle_rads) * u
                        };
                        let tau = if goal.abs() > w.abs() {
                            tau_up
                        } else {
                            tau_down
                        };
                        self.omega[i] = goal + (w - goal) * (-dt / tau).exp();
                        let p = self.kq * self.omega[i].abs().powi(3);
                        self.pack_current[i] = p / (FALLBACK_EFFICIENCY * v);
                    } else {
                        self.omega[i] = w * (-dt / FALLBACK_COAST_TAU).exp();
                        self.pack_current[i] = 0.0;
                    }
                    self.winding_current[i] = self.pack_current[i];
                }
            }
            self.domega[i] = (self.omega[i] - prev) / dt;
        }
    }

    pub fn rotor_inertia(&self) -> f64 {
        match self.kind {
            MotorKind::Electrical { j, .. } => j,
            MotorKind::FirstOrder { .. } => 0.0,
        }
    }

    /// The steady-state duty that holds speed `omega` at voltage `v` (the mixer's inverse).
    pub fn duty_for_speed(&self, omega: f64, v: f64) -> f64 {
        match self.kind {
            MotorKind::Electrical { r, i0, .. } => {
                let torque = self.kq * omega * omega;
                (self.ke * omega + r * (i0 + torque / self.ke)) / v.max(0.1)
            }
            MotorKind::FirstOrder {
                fraction,
                idle_rads,
                ..
            } => {
                let full = fraction * self.kv_rads * v;
                let u = (omega - idle_rads) / (full - idle_rads).max(1.0);
                self.idle_duty + (1.0 - self.idle_duty) * u
            }
        }
    }

    /// The steady-state speed at duty `d` and voltage `v`.
    pub fn speed_for_duty(&self, d: f64, v: f64) -> f64 {
        match self.kind {
            MotorKind::Electrical { r, i0, .. } => {
                // kQ/ke · ω² + ke/R · ω + (I0 − dV/R) = 0
                let a = self.kq / self.ke;
                let b = self.ke / r;
                let c = i0 - d * v / r;
                if c >= 0.0 {
                    return 0.0;
                }
                (-b + (b * b - 4.0 * a * c).sqrt()) / (2.0 * a)
            }
            MotorKind::FirstOrder {
                fraction,
                idle_rads,
                ..
            } => {
                let full = fraction * self.kv_rads * v;
                let u = ((d - self.idle_duty) / (1.0 - self.idle_duty)).clamp(0.0, 1.0);
                idle_rads + (full - idle_rads) * u
            }
        }
    }

    pub fn rpm(&self) -> [f64; 4] {
        self.omega.map(|w| w / crate::profile::RPM_TO_RADS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{preset, RPM_TO_RADS};

    fn run_to_steady(m: &mut Motors, d: f64, v: f64) {
        for _ in 0..20_000 {
            m.step(5e-4, &[d; 4], v, true);
        }
    }

    #[test]
    fn steady_state_matches_the_closed_form() {
        let p = preset("meteor75").unwrap().params();
        let mut m = Motors::new(&p);
        run_to_steady(&mut m, 0.5, 3.8);
        let w = m.speed_for_duty(0.5, 3.8);
        assert!((m.omega[0] - w).abs() / w < 1e-6);
        let d = m.duty_for_speed(w, 3.8);
        assert!((d - 0.5).abs() < 1e-9);
    }

    #[test]
    fn electrical_model_reproduces_the_logged_speed_map() {
        // Median logged rpm in bins of duty × pack voltage (bins with 1000+ steady samples),
        // from the two example whoops' decoded logs (sim-design 6.3). The fitted model must
        // reproduce the logged speed map (sim-design 4.2: within about 1.4k rpm RMS).
        // The 65 mm logs scatter 2.9k rpm RMS around any speed map, so its band is wider.
        type Bins<'a> = (&'a str, f64, &'a [(f64, f64)]);
        let logged: [Bins; 2] = [
            (
                "meteor75",
                1500.0,
                &[
                    (0.55, 9883.0),
                    (0.85, 15633.0),
                    (1.15, 19300.0),
                    (1.45, 23033.0),
                    (1.75, 27400.0),
                    (2.05, 31150.0),
                ],
            ),
            (
                "air65ii",
                2000.0,
                &[
                    (0.55, 14167.0),
                    (0.85, 18867.0),
                    (1.15, 26450.0),
                    (1.45, 29333.0),
                    (1.75, 33667.0),
                    (2.05, 36767.0),
                ],
            ),
        ];
        for (id, band, bins) in logged {
            let p = preset(id).unwrap().params();
            let m = Motors::new(&p);
            let se: f64 = bins
                .iter()
                .map(|(dv, rpm)| (m.speed_for_duty(dv / 3.8, 3.8) / RPM_TO_RADS - rpm).powi(2))
                .sum();
            let rms = (se / bins.len() as f64).sqrt();
            assert!(rms < band, "{id}: {rms:.0} rpm RMS against the logs");
        }
        let p = preset("meteor75").unwrap().params();
        let rpm = Motors::new(&p).speed_for_duty(0.8, 3.08) / RPM_TO_RADS;
        assert!((rpm - 34_400.0).abs() < 1_000.0, "{rpm}");
    }

    #[test]
    fn unpowered_rotor_spins_down_and_stops() {
        let p = preset("air65ii").unwrap().params();
        let mut m = Motors::new(&p);
        run_to_steady(&mut m, 0.4, 3.8);
        for _ in 0..40_000 {
            m.step(5e-4, &[0.0; 4], 3.8, false);
        }
        assert_eq!(m.omega, [0.0; 4]);
        assert_eq!(m.pack_current, [0.0; 4]);
    }
}
