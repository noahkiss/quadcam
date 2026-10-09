//! The sim's in-process host (S5) through `Core`: the physics thread fed by `FakeHid` through the
//! calibration, the frames the page reads, arm, turtle and reset from the derived controls, and
//! the radio going away. No real device opens, no window.

use quadcam_lib::core::{Core, NoHooks, SimFrame, SimStartParams};
use quadcam_lib::gear::bf::cli::Timing;
use quadcam_lib::gear::radio_hid::{report, FakeHid};
use quadcam_lib::gear::Env;
use quadcam_lib::photos::Recorder;
use quadcam_sim::input::{Calibration, RadioControl};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Raw axes: roll, pitch, throttle, yaw, then CH5 (arm), CH6-8.
const REST: [u16; 8] = [1024, 1024, 0, 1024, 0, 0, 0, 0];
const ARM_ON: u16 = 2047;
const BTN_TURTLE: u32 = 1;
const BTN_RESET: u32 = 1 << 1;

fn core(dir: &tempfile::TempDir, hid: &FakeHid) -> Core {
    let ports = quadcam_lib::gear::serial::FakePorts::new(vec![]);
    Core::new(
        dir.path().join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(dir.path().join("support/settings.json"))
    .with_gear_env(Env::fake(vec![], Arc::new(ports)))
    .with_fc_timing(Timing::fast())
    .with_radio_hid(Arc::new(hid.clone()))
}

/// What the derived defaults give: arm on CH5 high, turtle on button 1, reset on button 2.
fn calibration() -> Calibration {
    Calibration {
        arm: Some(RadioControl::Channel {
            ch: 5,
            min_us: 1700,
            max_us: 2100,
        }),
        turtle: Some(RadioControl::Button {
            button: 1,
            pressed: true,
        }),
        reset: Some(RadioControl::Button {
            button: 2,
            pressed: true,
        }),
        ..Calibration::default()
    }
}

fn start(c: &Core) {
    c.sim_start(&SimStartParams {
        profile: Some("meteor75".into()),
        calibration: Some(calibration()),
    })
    .unwrap();
}

/// Sends reports at about 1 kHz for `ms`.
fn hold(f: &FakeHid, ms: u64, buttons: u32, axes: [u16; 8]) {
    let t = Instant::now();
    while t.elapsed() < Duration::from_millis(ms) {
        f.push(report(buttons, axes));
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn with(axes: [u16; 8], i: usize, v: u16) -> [u16; 8] {
    let mut a = axes;
    a[i] = v;
    a
}

fn wait(c: &Core, f: &FakeHid, axes: [u16; 8], buttons: u32, ok: impl Fn(&SimFrame) -> bool) -> SimFrame {
    let end = Instant::now() + Duration::from_secs(8);
    loop {
        hold(f, 20, buttons, axes);
        let fr = c.sim_frame().unwrap();
        if ok(&fr) || Instant::now() > end {
            return fr;
        }
    }
}

#[test]
fn start_describes_the_room_and_frames_advance() {
    let dir = tempfile::tempdir().unwrap();
    let f = FakeHid::plugged();
    let c = core(&dir, &f);
    assert!(c.sim_frame().is_err(), "no sim runs yet");
    let info = c
        .sim_start(&SimStartParams {
            profile: Some("meteor75".into()),
            calibration: Some(calibration()),
        })
        .unwrap();
    assert_eq!(info.profile, "meteor75");
    assert!(info.calibrated);
    assert_eq!(info.stick_mode, 2);
    assert!((info.dt - 0.0005).abs() < 1e-9, "2 kHz");
    // The floor, ceiling and four walls, plus the crate, table and two gates (3 boxes each).
    assert!(info.boxes.len() >= 12, "{}", info.boxes.len());
    assert!(info.boxes.iter().any(|b| b.material == "gate"));
    assert!(info.camera.uptilt_deg > 0.0 && info.camera.fov_deg > 60.0);

    hold(&f, 300, 0, REST);
    let a = c.sim_frame().unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let b = c.sim_frame().unwrap();
    assert!(a.radio && b.radio);
    assert!(b.cur.step > a.cur.step + 50, "{} {}", a.cur.step, b.cur.step);
    assert_eq!(b.cur.step, b.prev.step + 1);
    assert!(b.cur.host_ns > b.prev.host_ns);
    assert!(b.host_ns >= b.cur.host_ns, "the read is after the step it holds");
    // A quad at rest on the pad, disarmed, battery full, no sag.
    assert!(!b.hud.armed);
    assert!(b.cur.pos[2].abs() < 0.05);
    assert!(b.hud.vbat > 3.8, "{}", b.hud.vbat);
    // The radio sample's arrival time reaches the frame.
    assert!(b.cur.input_ns > 0 && b.cur.input_ns <= b.cur.host_ns);

    let stop = c.sim_stop().unwrap();
    assert!(stop.steps > 100);
    assert!(stop.step_p99_us < 1000.0);
    assert!(c.sim_stop().is_none());
    assert!(c.sim_frame().is_err());
}

#[test]
fn an_unknown_profile_is_refused_and_a_second_start_replaces_the_first() {
    let dir = tempfile::tempdir().unwrap();
    let f = FakeHid::plugged();
    let c = core(&dir, &f);
    let e = c
        .sim_start(&SimStartParams {
            profile: Some("nope".into()),
            calibration: None,
        })
        .unwrap_err();
    assert!(e.to_string().contains("meteor75"), "{e}");
    let one = c.sim_start(&SimStartParams::default()).unwrap();
    assert_eq!(one.profile, quadcam_lib::core::DEFAULT_PRESET);
    assert!(!one.calibrated);
    let two = c
        .sim_start(&SimStartParams {
            profile: Some("five_inch".into()),
            calibration: None,
        })
        .unwrap();
    assert_eq!(two.profile, "five_inch");
    std::thread::sleep(Duration::from_millis(50));
    assert!(c.sim_frame().unwrap().cur.step < 1000, "a fresh run, not the old one");
    c.sim_stop();
}

#[test]
fn the_arm_switch_arms_at_low_throttle_and_the_sticks_show_calibrated() {
    let dir = tempfile::tempdir().unwrap();
    let f = FakeHid::plugged();
    let c = core(&dir, &f);
    start(&c);
    // The switch off first (the arm-at-boot guard), then on with the throttle down.
    hold(&f, 200, 0, REST);
    let fr = wait(&c, &f, with(REST, 4, ARM_ON), 0, |fr| fr.hud.armed);
    assert!(fr.hud.armed, "{:?}", fr.hud.arm_block);
    assert!(!fr.hud.turtle);
    // Half roll right, throttle a quarter: the HUD's sticks are the calibrated values.
    let axes = with(with(with(REST, 0, 1536), 2, 512), 4, ARM_ON);
    let fr = wait(&c, &f, axes, 0, |fr| fr.hud.sticks.roll > 0.4);
    assert!((fr.hud.sticks.roll - 0.5).abs() < 0.05, "{}", fr.hud.sticks.roll);
    assert!((fr.hud.sticks.throttle - 0.25).abs() < 0.05);
    assert!(fr.hud.sticks.pitch.abs() < 0.05 && fr.hud.sticks.yaw.abs() < 0.05);
    // The switch off disarms.
    let fr = wait(&c, &f, REST, 0, |fr| !fr.hud.armed);
    assert!(!fr.hud.armed);
    c.sim_stop();
}

#[test]
fn a_raised_throttle_refuses_to_arm_and_the_hud_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let f = FakeHid::plugged();
    let c = core(&dir, &f);
    start(&c);
    // Throttle up, switch off: the quad names the throttle.
    let fr = wait(&c, &f, with(REST, 2, 1500), 0, |fr| fr.hud.arm_block.is_some());
    let why = fr.hud.arm_block.unwrap();
    assert!(why.to_lowercase().contains("throttle"), "{why}");
    // The switch on anyway: refused, and the quad asks for the switch off and on again.
    let up = with(with(REST, 2, 1500), 4, ARM_ON);
    let fr = wait(&c, &f, up, 0, |fr| {
        fr.hud.arm_block.as_deref().is_some_and(|w| w.contains("off and on"))
    });
    assert!(!fr.hud.armed);
    assert!(fr.hud.arm_block.unwrap().contains("off and on"));
    c.sim_stop();
}

#[test]
fn turtle_follows_its_switch_held_at_arming_with_the_throttle_low() {
    let dir = tempfile::tempdir().unwrap();
    let f = FakeHid::plugged();
    let c = core(&dir, &f);
    start(&c);
    hold(&f, 200, 0, REST);
    let fr = wait(&c, &f, with(REST, 4, ARM_ON), BTN_TURTLE, |fr| fr.hud.armed);
    assert!(fr.hud.armed && fr.hud.turtle);
    c.sim_stop();
}

#[test]
fn the_reset_control_returns_the_quad_to_the_pad() {
    let dir = tempfile::tempdir().unwrap();
    let f = FakeHid::plugged();
    let c = core(&dir, &f);
    start(&c);
    hold(&f, 200, 0, REST);
    wait(&c, &f, with(REST, 4, ARM_ON), 0, |fr| fr.hud.armed);
    // Throttle well above hover: the quad climbs.
    let climb = with(with(REST, 2, 1700), 4, ARM_ON);
    let fr = wait(&c, &f, climb, 0, |fr| fr.cur.pos[2] > 0.3);
    assert!(fr.cur.pos[2] > 0.3, "{:?}", fr.cur.pos);
    // The reset button (the arm switch still on, as it would be in the air).
    let fr = wait(&c, &f, with(REST, 4, ARM_ON), BTN_RESET, |fr| fr.cur.pos[2] < 0.1);
    assert!(fr.cur.pos[2] < 0.1, "{:?}", fr.cur.pos);
    c.sim_stop();
}

#[test]
fn the_page_can_reset_without_the_radio() {
    let dir = tempfile::tempdir().unwrap();
    let f = FakeHid::plugged();
    let c = core(&dir, &f);
    start(&c);
    hold(&f, 200, 0, REST);
    wait(&c, &f, with(REST, 4, ARM_ON), 0, |fr| fr.hud.armed);
    wait(&c, &f, with(with(REST, 2, 1700), 4, ARM_ON), 0, |fr| fr.cur.pos[2] > 0.3);
    c.sim_reset().unwrap();
    let fr = wait(&c, &f, REST, 0, |fr| fr.cur.pos[2] < 0.1);
    assert!(fr.cur.pos[2] < 0.1);
    c.sim_stop();
}

#[test]
fn unplugging_the_radio_marks_the_link_lost_and_disarms() {
    let dir = tempfile::tempdir().unwrap();
    let f = FakeHid::plugged();
    let c = core(&dir, &f);
    start(&c);
    hold(&f, 200, 0, REST);
    wait(&c, &f, with(REST, 4, ARM_ON), 0, |fr| fr.hud.armed);
    f.unplug();
    // No more reports: the held arm switch must not keep the quad flying.
    let end = Instant::now() + Duration::from_secs(8);
    let mut fr = c.sim_frame().unwrap();
    while fr.hud.armed && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
        fr = c.sim_frame().unwrap();
    }
    assert!(!fr.radio, "the page is told");
    assert!(!fr.hud.armed, "failsafe disarmed the quad");
    c.sim_stop();
}
