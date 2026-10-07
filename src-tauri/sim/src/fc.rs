//! The sim's Betaflight-style flight controller (sim-design 5): RC smoothing, rates,
//! feedforward, a rate PID with I-term relax and TPA, angle and horizon, airmode, turtle,
//! arming guards and failsafe.
//!
//! Written from Betaflight's public documentation and logged behaviour, never its source.
//! The structure (a PID in physical units: angular acceleration, then torque = I·α) follows
//! propwash's `FlightController.ts` (MIT, `LICENSES/propwash.txt`), as do the turtle and
//! I-term relax shapes.

use serde::{Deserialize, Serialize};

use crate::diff::{mode, FcConfig};
use crate::filters::{Pt1, Pt3};
use crate::mixer::Mixer;
use crate::motor::Motors;
use crate::profile::{Params, SimFc};
use crate::ring::{RcFrame, Sticks};

const DEG: f64 = std::f64::consts::PI / 180.0;
/// Below this throttle, before airmode engages, the PID output is zero (idle on the pad).
const LOW_THROTTLE: f64 = 0.05;
/// Feedforward fades in over this setpoint acceleration (deg/s²): jitter reduction.
const FF_JITTER_DEG_S2: f64 = 300.0;
/// Link lost this long (s) drops to failsafe stage 2: disarm.
pub const FAILSAFE_DELAY_S: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlightMode {
    Acro,
    Angle,
    Horizon,
}

/// Why the quad will not arm now, as the OSD shows it (Betaflight's arming-disable reasons).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArmBlock {
    /// The arm switch was on when the sim started: turn it off first.
    ArmSwitchAtStart,
    /// The arm switch went on while another guard failed: turn it off and on again.
    ArmSwitch,
    Throttle,
    Angle,
    Failsafe,
    /// The quad has no ARM range in its aux lines.
    NoArmRange,
}

/// Physics the FC senses each loop (the sensor model sits in front of it).
#[derive(Debug, Clone, Copy)]
pub struct FcSense {
    /// Gyro, body axes (rad/s).
    pub gyro: [f64; 3],
    /// Roll right and pitch nose-down angles (rad), from the attitude estimate.
    pub roll: f64,
    pub pitch: f64,
    /// Tilt from level (deg): the small-angle guard.
    pub tilt_deg: f64,
    /// Pack voltage the FC measures.
    pub vbat: f64,
}

/// A test or validation override: fly these rates (deg/s, Betaflight axes: roll right,
/// pitch forward, yaw right) instead of the sticks'.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Override {
    pub setpoint: Option<[f64; 3]>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FcOut {
    /// Mixer output per motor; negative is reversed (turtle).
    pub u: [f64; 4],
    /// The ESCs drive the motors.
    pub driven: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct FcTelemetry {
    /// Setpoint and gyro, deg/s, body axes.
    pub setpoint: [f64; 3],
    pub gyro: [f64; 3],
    pub p: [f64; 3],
    pub i: [f64; 3],
    pub d: [f64; 3],
    pub f: [f64; 3],
    /// Throttle after smoothing and the curve.
    pub throttle: f64,
    pub saturated: bool,
}

#[derive(Debug, Clone)]
pub struct Fc {
    pub cfg: FcConfig,
    tune: Tune,
    dt: f64,
    pub armed: bool,
    pub turtle: bool,
    pub airmode_active: bool,
    pub mode: FlightMode,
    pub block: Option<ArmBlock>,
    /// The ideal controller: rates follow setpoint through a first-order lag, no PID.
    pub ideal: bool,
    arm_prev: bool,
    arm_seen_off: bool,
    refused: bool,
    link_lost_s: f64,
    sp_filter: [Pt3; 3],
    thr_filter: Pt3,
    prev_sp: [f64; 3],
    iterm: [f64; 3],
    relax: [Pt1; 3],
    dlpf: [Pt1; 3],
    prev_gyro_d: [f64; 3],
    saturated: bool,
    ideal_rate: [f64; 3],
    pub telemetry: FcTelemetry,
}

#[derive(Debug, Clone, Copy)]
struct Tune {
    p: [f64; 3],
    i: [f64; 3],
    d: [f64; 3],
    ff: [f64; 3],
    relax_threshold: f64,
    i_limit: f64,
    tpa_breakpoint: f64,
    tpa_rate: f64,
    ideal_lag: f64,
}

/// RC smoothing cutoff for a link packet rate: Betaflight's automatic mode scales the
/// setpoint cutoff with the frame rate (a third of the rate here), within 15-100 Hz.
pub fn rc_smoothing_hz(packet_rate_hz: f64) -> f64 {
    (packet_rate_hz / 3.0).clamp(15.0, 100.0)
}

impl Fc {
    pub fn new(cfg: FcConfig, fc: &SimFc, dt: f64, packet_rate_hz: f64) -> Fc {
        let g = |k: fn(&crate::profile::Gains) -> f64| -> [f64; 3] {
            [k(&fc.gains[0]), k(&fc.gains[1]), k(&fc.gains[2])]
        };
        let tune = Tune {
            p: g(|x| x.p.value),
            i: g(|x| x.i.value),
            d: g(|x| x.d.value),
            ff: g(|x| x.ff.value),
            relax_threshold: fc.iterm_relax_threshold.value * DEG,
            i_limit: fc.i_limit.value,
            tpa_breakpoint: fc.tpa_breakpoint.value,
            tpa_rate: fc.tpa_rate.value,
            ideal_lag: fc.ideal_lag.value,
        };
        let sc = rc_smoothing_hz(packet_rate_hz);
        let pt3 = || Pt3::new(sc, dt);
        let relax = || Pt1::new(fc.iterm_relax_hz.value, dt);
        let dl = || Pt1::new(fc.dterm_lpf_hz.value, dt);
        Fc {
            cfg,
            tune,
            dt,
            armed: false,
            turtle: false,
            airmode_active: false,
            mode: FlightMode::Acro,
            block: None,
            ideal: false,
            arm_prev: false,
            arm_seen_off: false,
            refused: false,
            link_lost_s: 0.0,
            sp_filter: [pt3(), pt3(), pt3()],
            thr_filter: pt3(),
            prev_sp: [0.0; 3],
            iterm: [0.0; 3],
            relax: [relax(), relax(), relax()],
            dlpf: [dl(), dl(), dl()],
            prev_gyro_d: [0.0; 3],
            saturated: false,
            ideal_rate: [0.0; 3],
            telemetry: FcTelemetry::default(),
        }
    }

    /// Setpoint for the sticks under the rates, body axes (rad/s): roll right is +x, pitch
    /// forward (nose down) is +y, yaw right (clockwise from above) is −z.
    fn acro_setpoint(&self, s: &Sticks) -> [f64; 3] {
        let r = &self.cfg.rates;
        [
            r.rate(0, s.roll) * DEG,
            r.rate(1, s.pitch) * DEG,
            -r.rate(2, s.yaw) * DEG,
        ]
    }

    fn arming_blocks(&self, rc: &RcFrame, sticks: &Sticks, sense: &FcSense) -> Option<ArmBlock> {
        if !self.cfg.has_mode(mode::ARM) {
            return Some(ArmBlock::NoArmRange);
        }
        if !self.arm_seen_off {
            return Some(ArmBlock::ArmSwitchAtStart);
        }
        if rc.link_lost() {
            return Some(ArmBlock::Failsafe);
        }
        if self.refused {
            return Some(ArmBlock::ArmSwitch);
        }
        let throttle_us = 1000.0 + 1000.0 * sticks.throttle;
        if throttle_us >= self.cfg.min_check as f64 {
            return Some(ArmBlock::Throttle);
        }
        let turtle_on = self.cfg.mode_on(mode::FLIP_OVER_AFTER_CRASH, &rc.ch);
        if !turtle_on && sense.tilt_deg > self.cfg.small_angle {
            return Some(ArmBlock::Angle);
        }
        None
    }

    fn reset_loops(&mut self, sticks: &Sticks) {
        self.iterm = [0.0; 3];
        self.prev_sp = [0.0; 3];
        self.prev_gyro_d = [0.0; 3];
        self.saturated = false;
        self.airmode_active = false;
        for f in &mut self.sp_filter {
            f.reset(0.0);
        }
        self.thr_filter.reset(sticks.throttle);
        for f in self.relax.iter_mut().chain(self.dlpf.iter_mut()) {
            f.reset(0.0);
        }
    }

    /// One control loop. Returns the motor outputs.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        rc: &RcFrame,
        sense: &FcSense,
        ov: &Override,
        p: &Params,
        motors: &Motors,
        mixer: &Mixer,
    ) -> FcOut {
        let sticks = rc.sticks();
        let arm_on = self.cfg.mode_on(mode::ARM, &rc.ch);
        if !arm_on {
            self.arm_seen_off = true;
            self.refused = false;
        }
        self.link_lost_s = if rc.link_lost() {
            self.link_lost_s + self.dt
        } else {
            0.0
        };

        // Arming and disarming.
        if self.armed {
            if !arm_on || self.link_lost_s >= FAILSAFE_DELAY_S {
                self.armed = false;
                self.turtle = false;
            }
            self.block = None;
        } else {
            let block = self.arming_blocks(rc, &sticks, sense);
            if arm_on && !self.arm_prev && self.arm_seen_off {
                match block {
                    None => {
                        self.armed = true;
                        self.turtle = self.cfg.mode_on(mode::FLIP_OVER_AFTER_CRASH, &rc.ch);
                        self.reset_loops(&sticks);
                    }
                    Some(_) => self.refused = true,
                }
            }
            self.block = if self.armed {
                None
            } else {
                self.arming_blocks(rc, &sticks, sense)
            };
        }
        self.arm_prev = arm_on;

        self.mode = if self.cfg.mode_on(mode::ANGLE, &rc.ch) {
            FlightMode::Angle
        } else if self.cfg.mode_on(mode::HORIZON, &rc.ch) {
            FlightMode::Horizon
        } else {
            FlightMode::Acro
        };

        if !self.armed {
            self.telemetry = FcTelemetry {
                gyro: sense.gyro.map(|g| g / DEG),
                ..FcTelemetry::default()
            };
            return FcOut {
                u: [0.0; 4],
                driven: false,
            };
        }
        if self.turtle {
            return self.turtle_out(rc, &sticks, p);
        }

        let dt = self.dt;
        let raw_sp = match ov.setpoint {
            Some(s) => [s[0] * DEG, s[1] * DEG, -s[2] * DEG],
            None => self.acro_setpoint(&sticks),
        };
        let mut sp = [0.0; 3];
        for (k, f) in self.sp_filter.iter_mut().enumerate() {
            sp[k] = f.apply(raw_sp[k]);
        }
        let throttle_stick = self.thr_filter.apply(sticks.throttle).clamp(0.0, 1.0);
        let throttle = self.cfg.throttle.apply(throttle_stick);

        if ov.setpoint.is_none() && self.mode != FlightMode::Acro {
            let k = self.cfg.angle_p_gain / 10.0;
            let lim = self.cfg.angle_limit * DEG;
            let s = [sticks.roll, sticks.pitch];
            let att = [sense.roll, sense.pitch];
            match self.mode {
                FlightMode::Angle => {
                    for a in 0..2 {
                        sp[a] = k * (s[a] * lim - att[a]);
                    }
                }
                FlightMode::Horizon => {
                    let stick = s[0].abs().max(s[1].abs());
                    let w_stick =
                        1.0 - (stick / (self.cfg.horizon_limit_sticks / 100.0)).clamp(0.0, 1.0);
                    let w_att =
                        1.0 - (sense.tilt_deg / self.cfg.horizon_limit_degrees).clamp(0.0, 1.0);
                    let level = w_stick * w_att * self.cfg.horizon_level_strength / 100.0;
                    for a in 0..2 {
                        sp[a] += level * k * (0.0 - att[a]);
                    }
                }
                FlightMode::Acro => {}
            }
        }

        let airmode_on = self.cfg.airmode || self.cfg.mode_on(mode::AIRMODE, &rc.ch);
        if airmode_on && throttle > self.cfg.airmode_start_throttle / 100.0 {
            self.airmode_active = true;
        }
        let stabilising = (airmode_on && self.airmode_active) || throttle >= LOW_THROTTLE;

        let gyro = sense.gyro;
        let tpa = 1.0
            - self.tune.tpa_rate
                * ((throttle - self.tune.tpa_breakpoint) / (1.0 - self.tune.tpa_breakpoint))
                    .clamp(0.0, 1.0);
        let mut torque = [0.0; 3];
        let tel = &mut self.telemetry;
        for a in 0..3 {
            let dsp = (sp[a] - self.prev_sp[a]) / dt;
            self.prev_sp[a] = sp[a];
            let alpha = if self.ideal {
                // Rates follow setpoint through a first-order lag: the needed acceleration.
                let r = self.ideal_rate[a]
                    + (sp[a] - self.ideal_rate[a]) * (dt / self.tune.ideal_lag).min(1.0);
                self.ideal_rate[a] = r;
                (r - gyro[a]) / dt
            } else {
                let e = sp[a] - gyro[a];
                let pt = self.tune.p[a] * e;
                // I-term relax: a fast-moving setpoint (its high-pass) pauses accumulation.
                let hp = sp[a] - self.relax[a].apply(sp[a]);
                let relax = if a < 2 {
                    (1.0 - hp.abs() / self.tune.relax_threshold).max(0.0)
                } else {
                    1.0
                };
                if !self.saturated {
                    self.iterm[a] = (self.iterm[a] + self.tune.i[a] * e * relax * dt)
                        .clamp(-self.tune.i_limit, self.tune.i_limit);
                }
                if !stabilising {
                    self.iterm[a] = 0.0;
                }
                let gd = self.dlpf[a].apply(gyro[a]);
                let dt_term = -self.tune.d[a] * tpa * (gd - self.prev_gyro_d[a]) / dt;
                self.prev_gyro_d[a] = gd;
                let jitter = (dsp.abs() / (FF_JITTER_DEG_S2 * DEG)).clamp(0.0, 1.0);
                let ft = self.tune.ff[a] * dsp * jitter;
                tel.p[a] = pt / DEG;
                tel.i[a] = self.iterm[a] / DEG;
                tel.d[a] = dt_term / DEG;
                tel.f[a] = ft / DEG;
                pt + self.iterm[a] + dt_term + ft
            };
            torque[a] = if stabilising {
                p.inertia[a] * alpha
            } else {
                0.0
            };
        }
        let mix = mixer.mix(
            motors,
            throttle,
            torque,
            sense.vbat,
            airmode_on && self.airmode_active,
        );
        self.saturated = mix.saturated;
        tel.setpoint = sp.map(|x| x / DEG);
        tel.gyro = gyro.map(|x| x / DEG);
        tel.throttle = throttle;
        tel.saturated = mix.saturated;
        FcOut {
            u: mix.u,
            driven: true,
        }
    }

    /// Turtle: upside down on the ground, the stick picks the side to lift and those motors
    /// spin in reverse. Roll right spins the right motors, pitch forward the front ones;
    /// crashflip_expo shapes the stick, crashflip_motor_percent drives the others.
    fn turtle_out(&mut self, rc: &RcFrame, sticks: &Sticks, p: &Params) -> FcOut {
        let on = self.cfg.mode_on(mode::FLIP_OVER_AFTER_CRASH, &rc.ch);
        let e = self.cfg.crashflip_expo / 100.0;
        let shape = |x: f64| x * (1.0 - e) + x.powi(3) * e;
        let mag = sticks.roll.abs().max(sticks.pitch.abs());
        let others = self.cfg.crashflip_motor_percent / 100.0 * shape(mag);
        let u = std::array::from_fn(|i| {
            if !on {
                return 0.0;
            }
            let [x, y, _] = p.rotor_pos[i];
            let score = -y.signum() * sticks.roll + x.signum() * sticks.pitch;
            let s = score.clamp(0.0, 1.0);
            if s > 0.0 {
                -shape(s)
            } else {
                -others
            }
        });
        self.telemetry = FcTelemetry::default();
        FcOut { u, driven: true }
    }
}
