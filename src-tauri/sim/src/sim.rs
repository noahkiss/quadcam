//! The deterministic core: one fixed step of input → FC → motors → aero → rigid body →
//! collisions → snapshot (sim-design 2.1, 2.2).
//!
//! No wall-clock reads inside a step. Every random stream is drawn every step whether used
//! or not, so the same input stream gives bit-identical flights.

use rapier3d_f64::glamx::{DQuat, DVec3};
use serde::{Deserialize, Serialize};

use crate::aero::{self, PropWash, Wind};
use crate::battery::Battery;
use crate::diff::AuxRange;
use crate::fc::{Fc, FcSense, Override};
use crate::mixer::Mixer;
use crate::motor::Motors;
use crate::profile::{Params, SimProfile, G};
use crate::ring::RcFrame;
use crate::rng::Rng;
use crate::snapshot::Snapshot;
use crate::world::{Physics, WorldSpec};

/// Run-time settings (sim-design 9.3 holds the user-facing ones).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimSettings {
    /// Physics and FC rate: 1000, 2000 (default) or 4000 Hz.
    pub rate_hz: f64,
    /// Battery sag (off by default: an ideal pack).
    pub battery_sag: bool,
    /// The ideal pack's voltage per cell, with sag off.
    pub ideal_cell_v: f64,
    /// Prop-wash noise at full severity, as a fraction of thrust (the slider).
    pub prop_wash: f64,
    /// Mean wind (m/s, world) and gust σ (m/s). Still air by default.
    pub wind_mean: [f64; 3],
    pub wind_sigma: f64,
    /// Gyro noise and motor vibration into the FC (off: an ideal gyro).
    pub sensor_noise: bool,
    /// The link's packet rate (Hz): sets RC smoothing.
    pub packet_rate_hz: f64,
    /// The sim-only reset control: a rising edge into this range resets to the start pad.
    pub reset: Option<AuxRange>,
    /// The "Ideal" controller: no PID, rates follow setpoint through a fitted lag.
    pub ideal_fc: bool,
}

impl Default for SimSettings {
    fn default() -> SimSettings {
        SimSettings {
            rate_hz: 2000.0,
            battery_sag: false,
            ideal_cell_v: crate::battery::IDEAL_CELL_V,
            prop_wash: 0.1,
            wind_mean: [0.0; 3],
            wind_sigma: 0.0,
            sensor_noise: false,
            packet_rate_hz: 250.0,
            reset: None,
            ideal_fc: false,
        }
    }
}

/// Gyro white noise (deg/s, 1σ) and motor vibration at full speed (deg/s), with noise on.
const GYRO_NOISE_DEG_S: f64 = 0.3;
const GYRO_VIBRATION_DEG_S: f64 = 8.0;
/// A struck open prop loses this fraction of thrust for this long (s).
const STRIKE_LOSS: f64 = 0.6;
const STRIKE_S: f64 = 0.15;
/// Stuck: upside down past this tilt (deg), or still, held this long (s).
const STUCK_TILT_DEG: f64 = 100.0;
const STUCK_HOLD_S: f64 = 1.0;
/// Collider streaming runs every this many steps.
const STREAM_EVERY: u64 = 64;

pub struct Sim {
    pub p: Params,
    pub profile_id: String,
    pub settings: SimSettings,
    pub dt: f64,
    pub step: u64,
    phys: Physics,
    pub motors: Motors,
    pub battery: Battery,
    pub fc: Fc,
    mixer: Mixer,
    wash: PropWash,
    wind: Wind,
    sensor_rng: Rng,
    vh: f64,
    strike: [f64; 4],
    contacts: u32,
    touching: bool,
    last_impact: f64,
    stuck_s: f64,
    stuck: bool,
    reset_prev: bool,
    last_u: [f64; 4],
    last_rc: RcFrame,
    pub resets: u32,
    vibration_phase: [f64; 4],
}

fn v3(a: [f64; 3]) -> DVec3 {
    DVec3::new(a[0], a[1], a[2])
}

impl Sim {
    pub fn new(profile: &SimProfile, world: WorldSpec, settings: SimSettings, seed: u64) -> Sim {
        let p = profile.params();
        let dt = 1.0 / settings.rate_hz;
        let mut fc = Fc::new(
            profile.diff.clone(),
            &profile.fc,
            dt,
            settings.packet_rate_hz,
        );
        fc.ideal = settings.ideal_fc;
        let mut wind = Wind::new(Rng::fork(seed, "wind"));
        wind.mean = settings.wind_mean;
        wind.sigma = settings.wind_sigma;
        let vh = aero::induced_velocity(p.mass * G / 4.0, p.prop_radius);
        let mut battery = Battery::new(&p, settings.battery_sag);
        battery.ideal_cell_v = settings.ideal_cell_v;
        battery.voltage = battery.resting_voltage();
        Sim {
            phys: Physics::new(&p, world, dt),
            motors: Motors::new(&p),
            battery,
            mixer: Mixer::new(&p),
            wash: PropWash::new(Rng::fork(seed, "prop wash"), dt),
            wind,
            sensor_rng: Rng::fork(seed, "sensors"),
            vh,
            fc,
            profile_id: profile.id.clone(),
            settings,
            dt,
            step: 0,
            p,
            strike: [0.0; 4],
            contacts: 0,
            touching: false,
            last_impact: 0.0,
            stuck_s: 0.0,
            stuck: false,
            reset_prev: false,
            last_u: [0.0; 4],
            last_rc: RcFrame::default(),
            resets: 0,
            vibration_phase: [0.0; 4],
        }
    }

    pub fn time(&self) -> f64 {
        self.step as f64 * self.dt
    }

    fn body(&self) -> &rapier3d_f64::prelude::RigidBody {
        &self.phys.world.bodies[self.phys.body]
    }

    pub fn position(&self) -> [f64; 3] {
        self.body().translation().into()
    }

    pub fn velocity(&self) -> [f64; 3] {
        self.body().linvel().into()
    }

    /// Body to world.
    pub fn attitude(&self) -> DQuat {
        *self.body().rotation()
    }

    /// Body rates (rad/s).
    pub fn body_rates(&self) -> [f64; 3] {
        let q = self.attitude();
        (q.inverse() * self.body().angvel()).into()
    }

    /// Tilt from level (deg).
    pub fn tilt_deg(&self) -> f64 {
        let up = self.attitude() * DVec3::Z;
        up.z.clamp(-1.0, 1.0).acos().to_degrees()
    }

    /// Put the body somewhere, for tests and validation.
    pub fn set_body(&mut self, pos: [f64; 3], att: DQuat, vel: [f64; 3], rates_body: [f64; 3]) {
        let rb = &mut self.phys.world.bodies[self.phys.body];
        rb.set_translation(v3(pos), true);
        rb.set_rotation(att, true);
        rb.set_linvel(v3(vel), true);
        rb.set_angvel(att * v3(rates_body), true);
    }

    /// Spin the motors up to a speed instantly (validation starts from a steady state).
    pub fn set_motor_speed(&mut self, omega: f64) {
        self.motors.omega = [omega; 4];
    }

    /// Back to the start pad: still, level, disarmed state kept as the switches say.
    pub fn reset(&mut self) {
        self.phys.place_at_start();
        self.motors.omega = [0.0; 4];
        self.motors.domega = [0.0; 4];
        self.stuck = false;
        self.stuck_s = 0.0;
        self.strike = [0.0; 4];
        self.resets += 1;
    }

    pub fn step(&mut self, rc: &RcFrame) {
        self.step_with(rc, &Override::default());
    }

    pub fn step_with(&mut self, rc: &RcFrame, ov: &Override) {
        let dt = self.dt;
        self.last_rc = *rc;

        // Sim-only reset control: the calibration's, or a range in the settings.
        let on = rc.reset() || self.settings.reset.is_some_and(|r| r.active(&rc.ch));
        if on && !self.reset_prev {
            self.reset();
        }
        self.reset_prev = on;

        let q = self.attitude();
        let qi = q.inverse();
        let pos = self.body().translation();
        let vel = self.body().linvel();
        let w_world = self.body().angvel();
        let w_body = qi * w_world;

        // Sensors: an ideal gyro unless noise is on; the streams are drawn either way.
        let mut gyro: [f64; 3] = w_body.into();
        let noise = [
            self.sensor_rng.gaussian(),
            self.sensor_rng.gaussian(),
            self.sensor_rng.gaussian(),
        ];
        if self.settings.sensor_noise {
            let mean_w = self.motors.omega.iter().map(|w| w.abs()).sum::<f64>() / 4.0;
            let w_max = self.motors.speed_for_duty(1.0, self.battery.cells * 4.2);
            let vib_amp = GYRO_VIBRATION_DEG_S.to_radians() * (mean_w / w_max).powi(2);
            for i in 0..4 {
                self.vibration_phase[i] = (self.vibration_phase[i]
                    + self.motors.omega[i].abs() * dt)
                    % std::f64::consts::TAU;
            }
            for (a, g) in gyro.iter_mut().enumerate() {
                let vib = vib_amp * self.vibration_phase[a].sin();
                *g += GYRO_NOISE_DEG_S.to_radians() * noise[a] + vib;
            }
        }
        let fwd = q * DVec3::X;
        let left = q * DVec3::Y;
        let up = q * DVec3::Z;
        let sense = FcSense {
            gyro,
            roll: left.z.atan2(up.z),
            pitch: (-fwd.z).clamp(-1.0, 1.0).asin(),
            tilt_deg: up.z.clamp(-1.0, 1.0).acos().to_degrees(),
            vbat: self.battery.voltage,
        };

        // FC, then the ESCs and the pack.
        let out = self
            .fc
            .update(rc, &sense, ov, &self.p, &self.motors, &self.mixer);
        self.last_u = out.u;
        let duty: [f64; 4] = if out.driven {
            out.u.map(|u| self.motors.duty_for(u))
        } else {
            [0.0; 4]
        };
        let (slope, offset) =
            self.motors
                .pack_current_affine(&duty, out.driven, self.battery.voltage);
        let v = self.battery.loaded_voltage(slope, offset);
        self.motors.step(dt, &duty, v, out.driven);
        let motor_i: f64 = self.motors.pack_current.iter().sum();
        self.battery.update(dt, v, motor_i);

        // Aerodynamics, in the body frame.
        let wind = v3(self.wind.velocity());
        let mut force = DVec3::ZERO;
        let mut torque = DVec3::ZERO;
        let p = &self.p;
        let down = -up;
        let j_rotor = self.motors.rotor_inertia();
        let mut h_rotors = DVec3::ZERO;
        for i in 0..4 {
            let rb = v3(p.rotor_pos[i]);
            let r_world = q * rb;
            let v_rotor = vel + w_world.cross(r_world) - wind;
            let vb = qi * v_rotor;
            let v_plane = (vb.x * vb.x + vb.y * vb.y).sqrt();
            let v_desc = -vb.z;
            let w = self.motors.omega[i];
            let mut t = if w >= 0.0 {
                p.kt * w * w * aero::inflow_factor(vb.z, w, 2.0 * p.prop_radius, p.j0)
            } else {
                -p.reverse_factor * p.kt * w * w
            };
            let h = self
                .phys
                .ray(pos + r_world, down, 4.0 * p.prop_radius)
                .unwrap_or(f64::INFINITY);
            t *= aero::ground_effect(h, p.prop_radius);
            let wash = self
                .wash
                .multiplier(i, v_desc, v_plane, self.vh, self.settings.prop_wash);
            if w > 0.0 {
                t *= (1.0 - aero::VRS_LOSS * aero::vrs_severity(v_desc, v_plane, self.vh)) * wash;
            }
            if self.strike[i] > 0.0 {
                t *= 1.0 - STRIKE_LOSS;
                self.strike[i] = (self.strike[i] - dt).max(0.0);
            }
            let in_plane = if p.ducted {
                aero::duct_drag(p.c_duct, t.abs(), p.prop_radius, [vb.x, vb.y])
            } else {
                aero::h_force(p.c_h, w, [vb.x, vb.y])
            };
            let f = DVec3::new(in_plane[0], in_plane[1], t);
            force += f;
            torque += rb.cross(f);
            // Reaction to the motor driving its rotor: drag torque plus spin-up.
            let s = p.rotor_spin[i];
            let q_aero = p.kq * w * w.abs();
            torque.z -= s * (q_aero + j_rotor * self.motors.domega[i]);
            h_rotors.z += s * j_rotor * w;
        }
        // Rotor gyroscopic torque: −ω × h.
        torque -= w_body.cross(h_rotors);
        let drag = aero::body_drag((qi * (vel - wind)).into(), p.cda);
        force += v3(drag);
        let inertia = v3(p.inertia);
        torque -= p.angular_damping * inertia * w_body;

        let rb = &mut self.phys.world.bodies[self.phys.body];
        rb.reset_forces(false);
        rb.reset_torques(false);
        rb.add_force(q * force, true);
        rb.add_torque(q * torque, true);

        let speed_before = vel.length();
        self.phys.world.step();
        self.wind.step(dt, (vel - wind).length());

        // Contacts, strikes and stuck.
        let c = self.phys.contacts();
        if c.touching && !self.touching {
            self.contacts += 1;
            self.last_impact = speed_before;
        }
        self.touching = c.touching;
        for i in 0..4 {
            if c.strikes[i] {
                self.strike[i] = STRIKE_S;
            }
        }
        let still = self.body().linvel().length() < 0.2;
        let idle = !self.fc.armed || self.last_u.iter().all(|u| u.abs() < 0.05);
        let candidate =
            self.touching && (self.tilt_deg() > STUCK_TILT_DEG || (still && idle && self.fc.armed));
        self.stuck_s = if candidate { self.stuck_s + dt } else { 0.0 };
        self.stuck = self.stuck_s >= STUCK_HOLD_S;

        self.step += 1;
        if self.step.is_multiple_of(STREAM_EVERY) {
            self.phys.stream();
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let q = self.attitude();
        let rates = self.body_rates();
        Snapshot {
            step: self.step,
            t: self.time(),
            host_ns: 0,
            input_ns: self.last_rc.t_ns,
            pos: self.position(),
            quat: [q.w, q.x, q.y, q.z],
            vel: self.velocity(),
            rates: rates.map(f64::to_degrees),
            motor_rpm: self.motors.rpm(),
            motor_u: self.last_u,
            vbat: self.battery.voltage,
            current: self.battery.current,
            mah: self.battery.used_mah,
            low_battery: self.battery.low_warning,
            armed: self.fc.armed,
            turtle: self.fc.turtle,
            airmode: self.fc.airmode_active,
            mode: self.fc.mode,
            arm_block: self.fc.block,
            sticks: self.last_rc.sticks(),
            throttle: self.fc.telemetry.throttle,
            setpoint: self.fc.telemetry.setpoint,
            contacts: self.contacts,
            touching: self.touching,
            last_impact: self.last_impact,
            stuck: self.stuck,
            prop_wash: self.wash.severity.iter().cloned().fold(0.0, f64::max),
            dropped_steps: 0,
        }
    }

    /// A hash of the whole dynamic state, for determinism checks.
    pub fn state_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |x: u64| {
            h ^= x;
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
            h = h.rotate_left(29);
        };
        let b = self.body();
        let q = b.rotation();
        for v in [
            b.translation().to_array(),
            b.linvel().to_array(),
            b.angvel().to_array(),
        ] {
            v.iter().for_each(|x| eat(x.to_bits()));
        }
        [q.w, q.x, q.y, q.z].iter().for_each(|x| eat(x.to_bits()));
        self.motors.omega.iter().for_each(|x| eat(x.to_bits()));
        eat(self.battery.voltage.to_bits());
        eat(self.battery.used_mah.to_bits());
        self.fc
            .telemetry
            .setpoint
            .iter()
            .for_each(|x| eat(x.to_bits()));
        self.fc.telemetry.i.iter().for_each(|x| eat(x.to_bits()));
        eat(self.fc.armed as u64);
        self.wash.rng_state().iter().for_each(|x| eat(*x));
        self.wind.rng_state().iter().for_each(|x| eat(*x));
        self.sensor_rng.state_words().iter().for_each(|x| eat(*x));
        eat(self.step);
        h
    }

    pub fn live_colliders(&self) -> usize {
        self.phys.live_colliders()
    }
}
