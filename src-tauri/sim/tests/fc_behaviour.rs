//! Flight-controller behaviour in the full sim: arming guards, airmode, angle and horizon,
//! turtle, failsafe, reset and collisions (sim-design 4.7, 5.4, 5.5; S1 acceptance).

mod common;

use common::*;
use quadcam_sim::fc::{ArmBlock, FlightMode};
use quadcam_sim::rapier3d_f64::glamx::DQuat;
use quadcam_sim::ring::FLAG_LINK_LOST;
use quadcam_sim::world::Material;
use quadcam_sim::{preset, Sim, SimSettings, WorldSpec};

fn sim(id: &str, world: WorldSpec) -> Sim {
    Sim::new(&preset(id).unwrap(), world, SimSettings::default(), 7)
}

fn on_floor(id: &str) -> Sim {
    sim(id, WorldSpec::floor(20.0, Material::Floor))
}

const IDLE: Switches = Switches {
    arm: false,
    angle: false,
    turtle: false,
};

#[test]
fn arming_needs_the_switch_seen_off_since_start() {
    let mut s = on_floor("meteor75");
    let armed_sw = Switches { arm: true, ..IDLE };
    run(&mut s, &rc(sticks(0.0, 0.0, 0.0, 0.0), armed_sw), 200);
    assert!(!s.fc.armed);
    assert_eq!(s.snapshot().arm_block, Some(ArmBlock::ArmSwitchAtStart));
    run(&mut s, &rc(sticks(0.0, 0.0, 0.0, 0.0), IDLE), 10);
    assert_eq!(s.snapshot().arm_block, None);
    run(&mut s, &rc(sticks(0.0, 0.0, 0.0, 0.0), armed_sw), 10);
    assert!(s.fc.armed);
    // Switch off disarms.
    run(&mut s, &rc(sticks(0.0, 0.0, 0.0, 0.0), IDLE), 10);
    assert!(!s.fc.armed);
}

#[test]
fn arming_refuses_throttle_up_until_the_switch_cycles() {
    let mut s = on_floor("meteor75");
    run(&mut s, &rc(sticks(0.0, 0.0, 0.0, 0.0), IDLE), 10);
    assert_eq!(
        {
            run(&mut s, &rc(sticks(0.3, 0.0, 0.0, 0.0), IDLE), 1);
            s.snapshot().arm_block
        },
        Some(ArmBlock::Throttle)
    );
    let up = Switches { arm: true, ..IDLE };
    run(&mut s, &rc(sticks(0.3, 0.0, 0.0, 0.0), up), 10);
    assert!(!s.fc.armed);
    // Throttle down with the switch still on: refused until it is cycled.
    run(&mut s, &rc(sticks(0.0, 0.0, 0.0, 0.0), up), 10);
    assert!(!s.fc.armed);
    assert_eq!(s.snapshot().arm_block, Some(ArmBlock::ArmSwitch));
    run(&mut s, &rc(sticks(0.0, 0.0, 0.0, 0.0), IDLE), 10);
    run(&mut s, &rc(sticks(0.0, 0.0, 0.0, 0.0), up), 10);
    assert!(s.fc.armed);
}

#[test]
fn arming_refuses_past_small_angle_except_in_turtle() {
    // The presets keep Betaflight's small_angle of 25 deg.
    let mut s = on_floor("meteor75");
    let tilted = DQuat::from_rotation_x(40f64.to_radians());
    s.set_body([0.0, 0.0, 0.5], tilted, [0.0; 3], [0.0; 3]);
    // Held in the air, tilted: the guard reads the attitude.
    let mut seen = None;
    for _ in 0..20 {
        s.set_body([0.0, 0.0, 0.5], tilted, [0.0; 3], [0.0; 3]);
        s.step(&rc(sticks(0.0, 0.0, 0.0, 0.0), IDLE));
        seen = s.snapshot().arm_block;
    }
    assert_eq!(seen, Some(ArmBlock::Angle));
    s.set_body([0.0, 0.0, 0.5], tilted, [0.0; 3], [0.0; 3]);
    s.step(&rc(
        sticks(0.0, 0.0, 0.0, 0.0),
        Switches { arm: true, ..IDLE },
    ));
    assert!(!s.fc.armed);

    let mut t = on_floor("meteor75");
    let turtle = Switches {
        turtle: true,
        ..IDLE
    };
    for _ in 0..20 {
        t.set_body([0.0, 0.0, 0.5], tilted, [0.0; 3], [0.0; 3]);
        t.step(&rc(sticks(0.0, 0.0, 0.0, 0.0), turtle));
    }
    t.set_body([0.0, 0.0, 0.5], tilted, [0.0; 3], [0.0; 3]);
    t.step(&rc(
        sticks(0.0, 0.0, 0.0, 0.0),
        Switches {
            arm: true,
            ..turtle
        },
    ));
    assert!(t.fc.armed && t.fc.turtle);
}

#[test]
fn turtle_flips_an_upside_down_quad() {
    for id in ["meteor75", "air65ii"] {
        let mut s = on_floor(id);
        // Upside down on the floor, settled.
        let flipped = DQuat::from_rotation_x(std::f64::consts::PI);
        s.set_body([0.0, 0.0, 0.05], flipped, [0.0; 3], [0.0; 3]);
        run(&mut s, &rc(sticks(0.0, 0.0, 0.0, 0.0), IDLE), 4000);
        assert!(s.tilt_deg() > 150.0);
        assert!(
            s.snapshot().stuck,
            "{id}: upside down and still reads stuck"
        );
        let sw = Switches {
            turtle: true,
            ..IDLE
        };
        arm(&mut s, sw);
        assert!(s.fc.turtle);
        // Hold roll to one side for up to 2 s.
        let flip = rc(sticks(0.0, 1.0, 0.0, 0.0), Switches { arm: true, ..sw });
        let mut done = false;
        for _ in 0..4000 {
            s.step(&flip);
            if s.tilt_deg() < 60.0 {
                done = true;
                break;
            }
        }
        assert!(done, "{id}: still at {:.0} deg", s.tilt_deg());
        let snap = s.snapshot();
        // Reversed motors only on the side the stick points at.
        assert!(snap.motor_u.iter().any(|u| *u < -0.5), "{:?}", snap.motor_u);
        assert!(snap.motor_u.contains(&0.0), "{:?}", snap.motor_u);
    }
}

#[test]
fn airmode_keeps_control_at_zero_throttle() {
    let roll_rate_at_zero_throttle = |diff: &str| {
        let p = preset("meteor75").unwrap().with_diff(diff);
        let mut s = Sim::new(&p, WorldSpec::empty(), SimSettings::default(), 3);
        s.set_body([0.0, 0.0, 50.0], DQuat::IDENTITY, [0.0; 3], [0.0; 3]);
        let sw = Switches { arm: true, ..IDLE };
        arm(&mut s, sw);
        // Punch through the airmode start throttle, chop, roll hard right, then reverse.
        run(&mut s, &rc(sticks(0.5, 0.0, 0.0, 0.0), sw), 200);
        run(&mut s, &rc(sticks(0.0, 1.0, 0.0, 0.0), sw), 400);
        let mut low: f64 = f64::MAX;
        for _ in 0..400 {
            s.step(&rc(sticks(0.0, -1.0, 0.0, 0.0), sw));
            low = low.min(s.body_rates()[0].to_degrees());
        }
        low
    };
    let base = "aux 0 0 0 1700 2100 0 0\naux 1 1 1 1700 2100 0 0\n";
    let on = roll_rate_at_zero_throttle(base);
    let off = roll_rate_at_zero_throttle(&format!("{base}feature -AIRMODE\n"));
    // Full left stick on the default Actual rates asks -670 deg/s: airmode gets there;
    // without it, at zero throttle, the motors have no authority to reverse the roll.
    assert!(on < -500.0, "airmode on: {on:.0} deg/s");
    assert!(off > -200.0, "airmode off: {off:.0} deg/s");
}

#[test]
fn angle_mode_holds_the_angle_limit_and_levels() {
    let mut s = sim("meteor75", WorldSpec::empty());
    s.set_body([0.0, 0.0, 100.0], DQuat::IDENTITY, [0.0; 3], [0.0; 3]);
    let sw = Switches {
        arm: true,
        angle: true,
        turtle: false,
    };
    arm(&mut s, sw);
    run(&mut s, &rc(sticks(0.4, 1.0, 0.0, 0.0), sw), 2000);
    assert_eq!(s.fc.mode, FlightMode::Angle);
    let bank = s.tilt_deg();
    assert!((bank - 60.0).abs() < 4.0, "bank {bank:.1}");
    run(&mut s, &rc(sticks(0.4, 0.0, 0.0, 0.0), sw), 2000);
    assert!(s.tilt_deg() < 2.0, "level {:.1}", s.tilt_deg());
}

#[test]
fn horizon_levels_at_centre_and_flips_at_full_stick() {
    let diff = "aux 0 0 0 1700 2100 0 0\naux 1 2 1 1700 2100 0 0\n";
    let p = preset("meteor75").unwrap().with_diff(diff);
    let mut s = Sim::new(&p, WorldSpec::empty(), SimSettings::default(), 3);
    s.set_body(
        [0.0, 0.0, 100.0],
        DQuat::from_rotation_x(0.3),
        [0.0; 3],
        [0.0; 3],
    );
    let sw = Switches {
        arm: true,
        angle: true,
        turtle: false,
    };
    arm(&mut s, sw);
    run(&mut s, &rc(sticks(0.4, 0.0, 0.0, 0.0), sw), 4000);
    assert_eq!(s.fc.mode, FlightMode::Horizon);
    assert!(s.tilt_deg() < 3.0, "level {:.1}", s.tilt_deg());
    // Full stick: no levelling, the quad rolls through upside down.
    let mut max_tilt: f64 = 0.0;
    for _ in 0..1000 {
        s.step(&rc(sticks(0.4, 1.0, 0.0, 0.0), sw));
        max_tilt = max_tilt.max(s.tilt_deg());
    }
    assert!(max_tilt > 170.0, "max tilt {max_tilt:.0}");
}

#[test]
fn failsafe_disarms_after_the_delay() {
    let mut s = sim("meteor75", WorldSpec::empty());
    s.set_body([0.0, 0.0, 100.0], DQuat::IDENTITY, [0.0; 3], [0.0; 3]);
    let sw = Switches { arm: true, ..IDLE };
    arm(&mut s, sw);
    let mut lost = rc(sticks(0.4, 0.0, 0.0, 0.0), sw);
    lost.flags = FLAG_LINK_LOST;
    run(&mut s, &lost, 1000);
    assert!(s.fc.armed, "still armed in stage 1");
    run(&mut s, &lost, 1100);
    assert!(!s.fc.armed);
}

#[test]
fn reset_control_returns_to_the_pad() {
    let settings = SimSettings {
        reset: Some(quadcam_sim::diff::AuxRange {
            mode_id: 0,
            ch: 7,
            start: 1700,
            end: 2100,
        }),
        ..SimSettings::default()
    };
    let mut s = Sim::new(
        &preset("air65ii").unwrap(),
        WorldSpec::floor(20.0, Material::Floor),
        settings,
        1,
    );
    s.set_body([3.0, 2.0, 4.0], DQuat::IDENTITY, [1.0, 0.0, 0.0], [0.0; 3]);
    let mut f = rc(sticks(0.0, 0.0, 0.0, 0.0), IDLE);
    s.step(&f);
    f.ch[7] = 2000;
    s.step(&f);
    let p = s.position();
    assert!(
        p[0].abs() < 1e-3 && p[1].abs() < 1e-3 && p[2] < 0.05,
        "{p:?}"
    );
    assert_eq!(s.resets, 1);
}

#[test]
fn a_whoop_bounces_off_a_wall_and_keeps_flying() {
    let mut s = sim("meteor75", WorldSpec::plain_room(4.0, 4.0, 2.5));
    s.set_body([1.5, 0.0, 1.2], DQuat::IDENTITY, [5.0, 0.0, 0.0], [0.0; 3]);
    let sw = Switches {
        arm: true,
        angle: true,
        turtle: false,
    };
    arm(&mut s, sw);
    s.set_body([1.5, 0.0, 1.2], DQuat::IDENTITY, [5.0, 0.0, 0.0], [0.0; 3]);
    let mut pilot = HoverPilot::new(1.2, 0.33);
    let mut min_vx: f64 = 0.0;
    let mut impact: f64 = 0.0;
    for _ in 0..4000 {
        let t = pilot.throttle(&s);
        s.step(&rc(sticks(t, 0.0, 0.0, 0.0), sw));
        min_vx = min_vx.min(s.velocity()[0]);
        impact = impact.max(s.snapshot().last_impact);
    }
    let snap = s.snapshot();
    assert!(snap.contacts >= 1);
    assert!(impact > 3.0, "impact {impact:.2}");
    assert!(min_vx < -0.5, "bounced back at {min_vx:.2} m/s");
    assert!(
        snap.armed && snap.pos[2] > 0.5 && snap.pos[0] < 2.0,
        "{:?}",
        snap.pos
    );
}

#[test]
fn a_dropped_quad_comes_to_rest_on_the_floor() {
    let mut s = on_floor("five_inch");
    s.set_body(
        [0.0, 0.0, 1.0],
        DQuat::from_rotation_y(0.3),
        [0.0; 3],
        [0.0; 3],
    );
    run(&mut s, &rc(sticks(0.0, 0.0, 0.0, 0.0), IDLE), 8000);
    let snap = s.snapshot();
    assert!(snap.touching && snap.contacts >= 1);
    assert!(snap.pos[2] < 0.06 && snap.pos[2] > 0.0, "{:?}", snap.pos);
    let v: f64 = snap.vel.iter().map(|x| x * x).sum::<f64>().sqrt();
    assert!(v < 0.05, "still moving at {v}");
}
