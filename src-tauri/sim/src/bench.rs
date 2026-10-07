//! The headless bench (sim-design 6.4, 2.2): step cost in free flight and in contact.
//! `examples/bench.rs` adds the real-time run; `tests/bench.rs` holds the bar in CI.

use std::time::Instant;

use rapier3d_f64::glamx::DQuat;

use crate::ring::{RcFrame, Sticks};
use crate::rng::Rng;
use crate::runner::CostHistogram;
use crate::world::WorldSpec;
use crate::{preset, Sim, SimSettings};

fn frame(s: Sticks, armed: bool, angle: bool) -> RcFrame {
    let b = |on: bool| if on { 2000 } else { 1000 };
    RcFrame::from_sticks(s, &[b(armed), b(angle)])
}

fn arm(sim: &mut Sim, angle: bool) {
    let zero = Sticks::default();
    for _ in 0..20 {
        sim.step(&frame(zero, false, angle));
    }
    for _ in 0..20 {
        sim.step(&frame(zero, true, angle));
    }
}

/// Step cost of `steps` steps of acro flight with seeded random sticks in a room, the
/// quad well clear of the walls.
pub fn free_flight(profile: &str, steps: usize) -> CostHistogram {
    let mut sim = Sim::new(
        &preset(profile).expect("preset"),
        WorldSpec::plain_room(200.0, 200.0, 400.0),
        SimSettings::default(),
        1,
    );
    sim.set_body([0.0, 0.0, 200.0], DQuat::IDENTITY, [0.0; 3], [0.0; 3]);
    arm(&mut sim, false);
    let mut r = Rng::new(3);
    let mut h = CostHistogram::default();
    let mut s = Sticks::default();
    for i in 0..steps {
        if i % 8 == 0 {
            s = Sticks {
                roll: r.uniform() - 0.5,
                pitch: r.uniform() - 0.5,
                throttle: 0.3 + 0.1 * r.uniform(),
                yaw: 0.4 * (r.uniform() - 0.5),
            };
        }
        let f = frame(s, true, false);
        let t = Instant::now();
        sim.step(&f);
        h.add(t.elapsed().as_nanos() as u64);
        // Keep it clear of the floor and ceiling.
        let z = sim.position()[2];
        if !(20.0..380.0).contains(&z) {
            sim.set_body([0.0, 0.0, 200.0], DQuat::IDENTITY, [0.0; 3], [0.0; 3]);
        }
    }
    h
}

/// Step cost with the quad in contact: armed, skidding and bumping on the floor and into a
/// wall at low throttle. Returns the histogram and the fraction of steps in contact.
pub fn contact(profile: &str, steps: usize) -> (CostHistogram, f64) {
    let mut sim = Sim::new(
        &preset(profile).expect("preset"),
        WorldSpec::plain_room(3.0, 3.0, 2.5),
        SimSettings::default(),
        1,
    );
    for _ in 0..200 {
        sim.step(&frame(Sticks::default(), false, true));
    }
    arm(&mut sim, true);
    let mut r = Rng::new(4);
    let mut h = CostHistogram::default();
    let mut touching = 0usize;
    let mut s = Sticks::default();
    for i in 0..steps {
        if i % 400 == 0 {
            s = Sticks {
                roll: r.uniform() - 0.5,
                pitch: r.uniform() - 0.5,
                throttle: 0.12 + 0.1 * r.uniform(),
                yaw: 0.0,
            };
        }
        let f = frame(s, true, true);
        let t = Instant::now();
        sim.step(&f);
        h.add(t.elapsed().as_nanos() as u64);
        if sim.snapshot().touching {
            touching += 1;
        }
    }
    (h, touching as f64 / steps as f64)
}
