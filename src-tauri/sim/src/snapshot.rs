//! Snapshots: what the renderer, audio and OSD read after each step (sim-design 2.3).
//!
//! The physics thread writes the newest pair (previous and current step) into a triple
//! buffer. A reader takes the newest pair without blocking the writer and interpolates the
//! pose to `present_time − one step`.

use serde::Serialize;

use crate::fc::{ArmBlock, FlightMode};
use crate::ring::Sticks;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Snapshot {
    pub step: u64,
    /// Sim time (s): steps × dt. It does not advance through a dropped backlog.
    pub t: f64,
    /// Host time (ns) the step stands for; the renderer interpolates on this.
    pub host_ns: u64,
    /// World position (m), z up.
    pub pos: [f64; 3],
    /// Attitude, body to world (w, x, y, z).
    pub quat: [f64; 4],
    /// World velocity (m/s).
    pub vel: [f64; 3],
    /// Body rates (deg/s): roll right, pitch nose down, yaw left (body z) positive.
    pub rates: [f64; 3],
    pub motor_rpm: [f64; 4],
    /// Mixer output per motor (negative: reversed in turtle).
    pub motor_u: [f64; 4],
    pub vbat: f64,
    pub current: f64,
    pub mah: f64,
    pub low_battery: bool,
    pub armed: bool,
    pub turtle: bool,
    pub airmode: bool,
    pub mode: FlightMode,
    /// Why the quad will not arm, for the OSD.
    pub arm_block: Option<ArmBlock>,
    pub sticks: Sticks,
    pub throttle: f64,
    /// Setpoint, body axes (deg/s).
    pub setpoint: [f64; 3],
    /// Contacts begun since the start, whether touching now, and the last impact speed.
    pub contacts: u32,
    pub touching: bool,
    pub last_impact: f64,
    /// Upside down or stopped against something with the motors idle: turtle or reset.
    pub stuck: bool,
    /// Largest prop-wash severity of the four rotors, 0..1.
    pub prop_wash: f64,
    /// Steps the runner dropped so far (stalls).
    pub dropped_steps: u64,
}

impl Default for Snapshot {
    fn default() -> Snapshot {
        Snapshot {
            step: 0,
            t: 0.0,
            host_ns: 0,
            pos: [0.0; 3],
            quat: [1.0, 0.0, 0.0, 0.0],
            vel: [0.0; 3],
            rates: [0.0; 3],
            motor_rpm: [0.0; 4],
            motor_u: [0.0; 4],
            vbat: 0.0,
            current: 0.0,
            mah: 0.0,
            low_battery: false,
            armed: false,
            turtle: false,
            airmode: false,
            mode: FlightMode::Acro,
            arm_block: None,
            sticks: Sticks::default(),
            throttle: 0.0,
            setpoint: [0.0; 3],
            contacts: 0,
            touching: false,
            last_impact: 0.0,
            stuck: false,
            prop_wash: 0.0,
            dropped_steps: 0,
        }
    }
}

/// The two newest snapshots.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SnapshotPair {
    pub prev: Snapshot,
    pub cur: Snapshot,
}

impl SnapshotPair {
    /// Position and attitude at host time `t_ns`, interpolated between the pair (clamped
    /// to it): linear position, normalised linear blend of the quaternions.
    pub fn pose_at(&self, t_ns: u64) -> ([f64; 3], [f64; 4]) {
        let (a, b) = (&self.prev, &self.cur);
        let span = b.host_ns.saturating_sub(a.host_ns);
        let k = if span == 0 {
            1.0
        } else {
            (t_ns.saturating_sub(a.host_ns) as f64 / span as f64).clamp(0.0, 1.0)
        };
        let pos = std::array::from_fn(|i| a.pos[i] + (b.pos[i] - a.pos[i]) * k);
        let dot: f64 = (0..4).map(|i| a.quat[i] * b.quat[i]).sum();
        let s = if dot < 0.0 { -1.0 } else { 1.0 };
        let mut q: [f64; 4] = std::array::from_fn(|i| a.quat[i] * (1.0 - k) + s * b.quat[i] * k);
        let n = q.iter().map(|x| x * x).sum::<f64>().sqrt();
        for x in &mut q {
            *x /= n;
        }
        (pos, q)
    }
}

pub type SnapshotWriter = triple_buffer::Input<SnapshotPair>;
pub type SnapshotReader = triple_buffer::Output<SnapshotPair>;

/// A writer for the physics thread and a reader for the renderer or audio.
pub fn snapshot_buffer() -> (SnapshotWriter, SnapshotReader) {
    triple_buffer::triple_buffer(&SnapshotPair::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pose_interpolates_between_the_pair() {
        let mut p = SnapshotPair::default();
        p.prev.host_ns = 1_000;
        p.cur.host_ns = 2_000;
        p.cur.pos = [1.0, 0.0, 2.0];
        let h = std::f64::consts::FRAC_1_SQRT_2;
        p.cur.quat = [h, 0.0, 0.0, h];
        let (pos, q) = p.pose_at(1_500);
        assert_eq!(pos, [0.5, 0.0, 1.0]);
        let n: f64 = q.iter().map(|x| x * x).sum();
        assert!((n - 1.0).abs() < 1e-12);
        assert!(q[3] > 0.3 && q[3] < h);
        assert_eq!(p.pose_at(5_000).0, [1.0, 0.0, 2.0]);
    }

    #[test]
    fn reader_sees_the_newest_pair() {
        let (mut w, mut r) = snapshot_buffer();
        for i in 1..=3 {
            let mut p = SnapshotPair::default();
            p.cur.step = i;
            w.write(p);
        }
        assert_eq!(r.read().cur.step, 3);
    }
}
