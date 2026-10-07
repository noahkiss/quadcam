//! The physics tests of sim-design 4.12: step convergence, determinism, energy, and the
//! per-term unit checks in the full sim (S1 acceptance).

mod common;

use common::*;
use quadcam_sim::aero;
use quadcam_sim::profile::{G, RPM_TO_RADS};
use quadcam_sim::rapier3d_f64::glamx::DQuat;
use quadcam_sim::record::Recorder;
use quadcam_sim::ring::{FakeRc, RcFrame, RcSource};
use quadcam_sim::rng::Rng;
use quadcam_sim::world::Material;
use quadcam_sim::{preset, Sim, SimSettings, WorldSpec};

const ARMED: Switches = Switches {
    arm: true,
    angle: false,
    turtle: false,
};

/// A scripted acro flight as 250 Hz radio frames: take off, climb, a roll flip at 3 s, a
/// pitch-forward run, a yaw turn, a second flip, and settle.
fn script(seconds: f64) -> Vec<RcFrame> {
    let mut out = Vec::new();
    let n = (seconds * 250.0) as usize;
    for k in 0..n {
        let t = k as f64 / 250.0;
        let s = match t {
            t if t < 0.1 => sticks(0.0, 0.0, 0.0, 0.0),
            t if t < 1.2 => sticks(0.45, 0.0, 0.0, 0.0),
            t if t < 3.0 => sticks(0.34, 0.0, 0.0, 0.0),
            t if t < 3.54 => sticks(0.25, 1.0, 0.0, 0.0),
            t if t < 4.0 => sticks(0.42, 0.0, 0.0, 0.0),
            t if t < 6.0 => sticks(0.36, 0.0, 0.15, 0.0),
            t if t < 7.0 => sticks(0.36, 0.0, -0.1, 0.4),
            t if t < 7.6 => sticks(0.25, -1.0, 0.0, 0.0),
            _ => sticks(0.38, 0.0, 0.0, 0.0),
        };
        let sw = if t < 0.05 { Switches::default() } else { ARMED };
        let mut f = rc(s, sw);
        f.t_ns = (t * 1e9).round() as u64;
        out.push(f);
    }
    out
}

struct Flight {
    pos: [f64; 3],
    peak_roll_rate: f64,
}

fn fly(rate_hz: f64, frames: &[RcFrame], seconds: f64) -> Flight {
    let settings = SimSettings {
        rate_hz,
        ..SimSettings::default()
    };
    let mut sim = Sim::new(
        &preset("meteor75").unwrap(),
        WorldSpec::empty(),
        settings,
        11,
    );
    sim.set_body([0.0, 0.0, 20.0], DQuat::IDENTITY, [0.0; 3], [0.0; 3]);
    let mut input = FakeRc::new(frames.to_vec());
    let steps = (seconds * rate_hz).round() as u64;
    let mut peak: f64 = 0.0;
    for i in 0..steps {
        let t_ns = (i as f64 / rate_hz * 1e9).round() as u64;
        sim.step(&input.frame_at(t_ns));
        peak = peak.max(sim.body_rates()[0].to_degrees().abs());
    }
    Flight {
        pos: sim.position(),
        peak_roll_rate: peak,
    }
}

#[test]
fn steps_converge_across_1_2_and_4_khz() {
    let frames = script(10.0);
    let f1 = fly(1000.0, &frames, 10.0);
    let f2 = fly(2000.0, &frames, 10.0);
    let f4 = fly(4000.0, &frames, 10.0);
    let dist = |a: [f64; 3], b: [f64; 3]| {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    };
    let travel = dist(f4.pos, [0.0, 0.0, 20.0]);
    let d2 = dist(f2.pos, f4.pos);
    let d1 = dist(f1.pos, f4.pos);
    println!(
        "convergence: travel {travel:.2} m; 2 kHz off by {d2:.3} m, 1 kHz by {d1:.3} m; \
         peak roll {:.1} / {:.1} / {:.1} deg/s",
        f1.peak_roll_rate, f2.peak_roll_rate, f4.peak_roll_rate
    );
    // Bands: position after 10 s within 2 % of the distance flown (2 kHz) and 4 % (1 kHz)
    // of the 4 kHz flight; peak rate on the flip within 2 %.
    assert!(travel > 5.0, "the script flies somewhere: {travel:.2} m");
    assert!(d2 < 0.02 * travel, "2 kHz: {d2:.3} m of {travel:.1}");
    assert!(d1 < 0.04 * travel, "1 kHz: {d1:.3} m of {travel:.1}");
    for f in [&f1, &f2] {
        let e = (f.peak_roll_rate - f4.peak_roll_rate).abs() / f4.peak_roll_rate;
        assert!(e < 0.02, "peak roll rate off by {:.1} %", e * 100.0);
    }
    assert!(f4.peak_roll_rate > 500.0);
}

/// 60 s of seeded random sticks at 250 Hz, switches on.
fn random_input(seconds: f64, seed: u64) -> Vec<RcFrame> {
    let mut r = Rng::new(seed);
    let mut cur = sticks(0.35, 0.0, 0.0, 0.0);
    let n = (seconds * 250.0) as usize;
    (0..n)
        .map(|k| {
            let t = k as f64 / 250.0;
            if k % 50 == 0 {
                cur = sticks(
                    0.3 + 0.15 * r.uniform(),
                    0.6 * (r.uniform() - 0.5),
                    0.6 * (r.uniform() - 0.5),
                    0.4 * (r.uniform() - 0.5),
                );
            }
            let sw = if t < 0.05 { Switches::default() } else { ARMED };
            let mut f = rc(cur, sw);
            f.t_ns = (t * 1e9).round() as u64;
            f
        })
        .collect()
}

#[test]
fn the_same_input_gives_the_same_state_hash() {
    let frames = random_input(60.0, 5);
    let settings = SimSettings {
        prop_wash: 0.3,
        sensor_noise: true,
        wind_sigma: 1.0,
        ..SimSettings::default()
    };
    let world = WorldSpec::plain_room(30.0, 30.0, 40.0);
    let profile = preset("air65ii").unwrap();
    let run_once = |record: bool| {
        let mut sim = Sim::new(&profile, world.clone(), settings.clone(), 42);
        let mut rec = Recorder::new(&profile, &world, &settings, 42);
        let mut input = FakeRc::new(frames.clone());
        let steps = (60.0 * settings.rate_hz) as u64;
        for i in 0..steps {
            let f = input.frame_at((i as f64 / settings.rate_hz * 1e9).round() as u64);
            if record {
                rec.push(i, &f);
            }
            sim.step(&f);
        }
        (sim.state_hash(), rec.finish(&sim))
    };
    let (a, rec) = run_once(true);
    let (b, _) = run_once(false);
    assert_eq!(a, b);
    // A recording replays bit for bit, through JSON.
    let rec = quadcam_sim::record::Recording::from_json(&rec.to_json()).unwrap();
    assert_eq!(rec.final_hash, Some(a));
    assert_eq!(rec.replay().state_hash(), a);
    // A different seed changes the noise and so the flight.
    let mut other = Sim::new(&profile, world.clone(), settings.clone(), 43);
    for f in rec.frames_by_step() {
        other.step(&f);
    }
    assert_ne!(other.state_hash(), a);
}

fn energy(sim: &Sim) -> f64 {
    let m = sim.p.mass;
    let v = sim.velocity();
    let w = sim.body_rates();
    let i = sim.p.inertia;
    0.5 * m * v.iter().map(|x| x * x).sum::<f64>()
        + 0.5 * (0..3).map(|k| i[k] * w[k] * w[k]).sum::<f64>()
        + m * G * sim.position()[2]
}

#[test]
fn an_unpowered_body_in_still_air_never_gains_energy() {
    for id in ["meteor75", "five_inch"] {
        let mut sim = Sim::new(
            &preset(id).unwrap(),
            WorldSpec::empty(),
            SimSettings::default(),
            1,
        );
        sim.set_body(
            [0.0, 0.0, 100.0],
            DQuat::from_rotation_x(0.4),
            [6.0, -3.0, 4.0],
            [12.0, -5.0, 3.0],
        );
        let idle = RcFrame::default();
        let e0 = energy(&sim);
        let mut prev = e0;
        for _ in 0..20_000 {
            sim.step(&idle);
            let e = energy(&sim);
            assert!(
                e <= prev + 1e-12 * e0.abs(),
                "{id}: energy rose {prev} -> {e}"
            );
            prev = e;
        }
        assert!(prev < e0, "{id}: drag took nothing");
    }
}

#[test]
fn hover_is_at_the_fitted_command_and_speed() {
    // The presets' reference hover (level hover in the example logs): command 0.30 /
    // 18.9k rpm (75 mm) and 0.338 / 27.3k rpm (65 mm), at the hover pack voltages (3.93 V
    // and 3.89 V).
    for (id, v_cell) in [("meteor75", 3.93), ("air65ii", 3.89)] {
        let pr = preset(id).unwrap();
        let settings = SimSettings {
            ideal_cell_v: v_cell,
            prop_wash: 0.0,
            ..SimSettings::default()
        };
        let mut sim = Sim::new(&pr, WorldSpec::empty(), settings, 1);
        sim.set_body([0.0, 0.0, 10.0], DQuat::IDENTITY, [0.0; 3], [0.0; 3]);
        arm(
            &mut sim,
            Switches {
                arm: true,
                angle: true,
                turtle: false,
            },
        );
        let (duty, rpm) = hover(&mut sim, 10.0, 5.0);
        let r = &pr.reference;
        println!("{id}: hover duty {duty:.3} rpm {rpm:.0}");
        assert!(
            (duty - r.hover_cmd.unwrap()).abs() <= 0.02,
            "{id}: duty {duty:.3}"
        );
        assert!(
            (rpm - r.hover_rpm.unwrap()).abs() / r.hover_rpm.unwrap() <= 0.05,
            "{id}: {rpm:.0} rpm"
        );
    }
}

/// Total rotor thrust (N) the body felt over one step, from its vertical acceleration,
/// with the motors held at `omega` and the body level at `pos` moving at `vel`.
fn thrust_in_one_step(sim: &mut Sim, pos: [f64; 3], vel: [f64; 3], omega: f64) -> f64 {
    sim.set_body(pos, DQuat::IDENTITY, vel, [0.0; 3]);
    sim.set_motor_speed(omega);
    let v0 = sim.velocity()[2];
    sim.step(&RcFrame::default());
    let a = (sim.velocity()[2] - v0) / sim.dt;
    let drag = aero::body_drag([vel[0], vel[1], vel[2]], sim.p.cda)[2];
    sim.p.mass * (a + G) - drag
}

#[test]
fn ground_effect_at_one_rotor_radius() {
    let pr = preset("meteor75").unwrap();
    let settings = SimSettings {
        prop_wash: 0.0,
        ..SimSettings::default()
    };
    let mut sim = Sim::new(&pr, WorldSpec::floor(5.0, Material::Floor), settings, 1);
    let r = sim.p.prop_radius;
    let w = 20_000.0 * RPM_TO_RADS;
    let near = thrust_in_one_step(&mut sim, [0.0, 0.0, r], [0.0; 3], w);
    let far = thrust_in_one_step(&mut sim, [0.0, 0.0, 2.0], [0.0; 3], w);
    let ratio = near / far;
    assert!((ratio - 16.0 / 15.0).abs() < 0.005, "IGE/OGE {ratio:.4}");
}

#[test]
fn vrs_costs_thrust_in_its_band_only() {
    let pr = preset("meteor75").unwrap();
    let settings = SimSettings {
        prop_wash: 0.0,
        ..SimSettings::default()
    };
    let mut sim = Sim::new(&pr, WorldSpec::empty(), settings, 1);
    let w = 25_000.0 * RPM_TO_RADS;
    let d = 2.0 * sim.p.prop_radius;
    let ratio = |sim: &mut Sim, v_desc: f64| {
        let t = thrust_in_one_step(sim, [0.0, 0.0, 50.0], [0.0, 0.0, -v_desc], w);
        // The ω after this step's motor update is what made the thrust.
        let w1 = sim.motors.omega[0];
        let pure = 4.0 * sim.p.kt * w1 * w1 * aero::inflow_factor(-v_desc, w1, d, sim.p.j0);
        t / pure
    };
    let out_low = ratio(&mut sim, 1.0);
    let in_band = ratio(&mut sim, 6.0);
    let out_high = ratio(&mut sim, 15.0);
    println!("VRS thrust ratio: 1 m/s {out_low:.3}, 6 m/s {in_band:.3}, 15 m/s {out_high:.3}");
    assert!((out_low - 1.0).abs() < 0.01);
    assert!((out_high - 1.0).abs() < 0.01);
    assert!((in_band - (1.0 - aero::VRS_LOSS)).abs() < 0.02);
}
