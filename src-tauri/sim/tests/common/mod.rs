//! Shared helpers for the sim's integration tests: RC frames from sticks and switches, and a
//! simple altitude-holding "pilot".
#![allow(dead_code)]

use quadcam_sim::ring::{RcFrame, Sticks};
use quadcam_sim::Sim;

pub const HIGH: u16 = 2000;
pub const LOW: u16 = 1000;

/// Switches on the presets' aux layout: AUX1 arm, AUX2 angle, AUX3 turtle.
#[derive(Debug, Clone, Copy, Default)]
pub struct Switches {
    pub arm: bool,
    pub angle: bool,
    pub turtle: bool,
}

pub fn rc(s: Sticks, sw: Switches) -> RcFrame {
    let b = |on: bool| if on { HIGH } else { LOW };
    RcFrame::from_sticks(s, &[b(sw.arm), b(sw.angle), b(sw.turtle)])
}

pub fn sticks(throttle: f64, roll: f64, pitch: f64, yaw: f64) -> Sticks {
    Sticks {
        roll,
        pitch,
        throttle,
        yaw,
    }
}

/// Run `n` steps of one frame.
pub fn run(sim: &mut Sim, f: &RcFrame, n: usize) {
    for _ in 0..n {
        sim.step(f);
    }
}

/// Arm on the pad: switch off, then on with throttle low.
pub fn arm(sim: &mut Sim, sw: Switches) {
    let off = rc(sticks(0.0, 0.0, 0.0, 0.0), Switches { arm: false, ..sw });
    run(sim, &off, 20);
    let on = rc(sticks(0.0, 0.0, 0.0, 0.0), Switches { arm: true, ..sw });
    run(sim, &on, 20);
    assert!(sim.fc.armed, "did not arm: {:?}", sim.fc.block);
}

pub use quadcam_sim::validate::HoverPilot;

/// Fly an Angle-mode hover at `z` for `seconds`; returns the mean motor duty and rpm over
/// the last second.
pub fn hover(sim: &mut Sim, z: f64, seconds: f64) -> (f64, f64) {
    let sw = Switches {
        arm: true,
        angle: true,
        turtle: false,
    };
    let mut pilot = HoverPilot::new(z, 0.35);
    let n = (seconds / sim.dt) as usize;
    let last = (1.0 / sim.dt) as usize;
    let (mut duty, mut rpm, mut k) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let t = pilot.throttle(sim);
        sim.step(&rc(sticks(t, 0.0, 0.0, 0.0), sw));
        if i + last >= n {
            let s = sim.snapshot();
            duty += s
                .motor_u
                .iter()
                .map(|u| sim.motors.duty_for(*u))
                .sum::<f64>()
                / 4.0;
            rpm += s.motor_rpm.iter().sum::<f64>() / 4.0;
            k += 1.0;
        }
    }
    (duty / k, rpm / k)
}
