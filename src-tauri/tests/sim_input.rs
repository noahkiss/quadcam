//! The sim's input (S4) through `Core`: the calibration session against `FakeHid`, saving
//! by Gear radio id with the resolve step, and the defaults from a synthetic aircraft's
//! switch map. No real device opens, no window.

use quadcam_lib::api::Event;
use quadcam_lib::core::{
    CalibrateAction, CalibrateParams, CalibrateView, Core, Hooks, NoHooks, SimCalibrationParams,
    SimCalibrationSaveParams, SimDefaultsParams,
};
use quadcam_lib::gear::bf::cli::Timing;
use quadcam_lib::gear::model::{Device, DeviceKind, Identity};
use quadcam_lib::gear::radio_hid::{report, FakeHid};
use quadcam_lib::gear::Env;
use quadcam_lib::photos::Recorder;
use quadcam_sim::input::{CalPhase, Calibration, RadioControl};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn core(dir: &tempfile::TempDir, hooks: Arc<dyn Hooks>) -> Core {
    let ports = quadcam_lib::gear::serial::FakePorts::new(vec![]);
    Core::new(
        dir.path().join("cache"),
        None,
        hooks,
        Arc::new(Recorder::default()),
    )
    .with_settings(dir.path().join("support/settings.json"))
    .with_gear_env(Env::fake(vec![], Arc::new(ports)))
    .with_fc_timing(Timing::fast())
}

fn save_radio(c: &Core, id: &str, board: &str) {
    c.gear_store()
        .save_device(&Device {
            id: id.into(),
            kind: DeviceKind::Radio,
            name: String::new(),
            aircraft: None,
            identity: Identity {
                board: Some(board.into()),
                ..Default::default()
            },
            last_seen: None,
            last_backup: None,
            last_space: None,
        })
        .unwrap();
}

fn get(c: &Core) -> CalibrateView {
    c.gear_sim_calibrate(&CalibrateParams::default()).unwrap()
}

fn wait(c: &Core, what: impl Fn(&CalibrateView) -> bool) -> CalibrateView {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let v = get(c);
        if what(&v) || Instant::now() > end {
            return v;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Plays reports at about 1 kHz for `ms`.
fn play(f: &FakeHid, ms: u64, mut at: impl FnMut(u64) -> (u32, [u16; 8])) {
    let start = Instant::now();
    while (start.elapsed().as_millis() as u64) < ms {
        let (b, ax) = at(start.elapsed().as_millis() as u64);
        f.push(report(b, ax));
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<CalibrateView>>);

impl Hooks for Events {
    fn event(&self, e: Event) {
        if let Event::SimCalibration(v) = e {
            self.0.lock().unwrap().push(v.0);
        }
    }
}

#[test]
fn the_calibration_flow_against_fake_hid_saves_by_radio_id() {
    let dir = tempfile::tempdir().unwrap();
    let events = Arc::new(Events::default());
    let f = FakeHid::plugged();
    let c = core(&dir, events.clone()).with_radio_hid(Arc::new(f.clone()));
    let v = c
        .gear_sim_calibrate(&CalibrateParams {
            action: CalibrateAction::Start,
            calibration: Some(Calibration::default()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(v.phase, CalPhase::Move);
    assert!(c.gear_radio_reading());
    // Move: each stick sweeps its full travel in turn, for 3.4 s.
    play(&f, 3400, |t| {
        let mut ax = [1024, 1024, 0, 1024, 0, 0, 0, 0];
        let k = ((t / 100) % 4) as usize;
        let ph = (t % 100) as f64 / 100.0;
        ax[k] = (1024.0 + 1030.0 * (ph * std::f64::consts::TAU).sin()).clamp(0.0, 2048.0) as u16;
        (0, ax)
    });
    // Let go: the springs rest a little off the middle.
    play(&f, 900, |_| (0, [1030, 1020, 0, 1026, 0, 0, 0, 0]));
    let v = wait(&c, |v| v.phase == CalPhase::Arm);
    assert_eq!(v.phase, CalPhase::Arm, "{v:?}");
    let cal = &v.calibration;
    assert_eq!(cal.map(), [1, 2, 3, 4]);
    assert_eq!(
        (cal.roll.centre, cal.pitch.centre, cal.yaw.centre),
        (1030, 1020, 1026)
    );
    assert!(
        cal.roll.auto_low < 30 && cal.roll.auto_high > 2018,
        "{:?}",
        cal.roll
    );
    assert!(v.coverage.pitch >= 97, "{:?}", v.coverage);
    // Flip the arm switch (CH5), then press a free button for reset.
    play(&f, 120, |t| {
        (
            0,
            [1030, 1020, 0, 1026, if t > 60 { 2048 } else { 0 }, 0, 0, 0],
        )
    });
    play(&f, 120, |t| {
        ((t > 60) as u32, [1030, 1020, 0, 1026, 2048, 0, 0, 0])
    });
    let v = wait(&c, |v| v.phase == CalPhase::Review);
    assert_eq!(v.phase, CalPhase::Review, "{v:?}");
    assert!(matches!(
        v.calibration.arm,
        Some(RadioControl::Channel { ch: 5, .. })
    ));
    assert_eq!(
        v.calibration.reset,
        Some(RadioControl::Button {
            button: 1,
            pressed: true
        })
    );
    // The live output in Review is the calibrated one.
    let s = v.sticks.unwrap();
    assert_eq!((s.roll, s.pitch, s.yaw, s.throttle), (0, 0, 0, 0));
    // The screen got events, throttled: far fewer than the ~4,500 reports.
    let n = events.0.lock().unwrap().len();
    assert!(n > 20 && n < 600, "{n} events");

    // Which radio is this: the one saved radio whose board the product name holds.
    save_radio(&c, "radio-00000000000000aa", "test radio");
    let res = c
        .gear_sim_calibration(&SimCalibrationParams::default())
        .unwrap();
    let r = res.resolution.unwrap();
    assert_eq!(r.radio.as_deref(), Some("radio-00000000000000aa"));
    assert_eq!(res.firmware.as_deref(), Some("2.12"));
    assert!(res.calibration.is_none(), "none saved yet");
    c.gear_sim_calibration_save(&SimCalibrationSaveParams {
        radio: r.radio.unwrap(),
        calibration: v.calibration.clone(),
        product: Some(r.product),
        firmware: res.firmware,
        ..Default::default()
    })
    .unwrap();
    let back = c
        .gear_sim_calibration(&SimCalibrationParams::default())
        .unwrap();
    assert_eq!(back.calibration.unwrap().calibration, v.calibration);
    // A second radio of the same model: the resolve step asks.
    save_radio(&c, "radio-00000000000000bb", "test radio");
    let res = c
        .gear_sim_calibration(&SimCalibrationParams::default())
        .unwrap();
    let r = res.resolution.unwrap();
    assert_eq!(r.radio, None);
    assert_eq!(r.choices.len(), 2);
    // Stop: the reader stops with the session.
    let stop = c
        .gear_sim_calibrate(&CalibrateParams {
            action: CalibrateAction::Stop,
            ..Default::default()
        })
        .unwrap();
    assert!(!stop.active);
    assert!(!c.gear_radio_reading());
}

#[test]
fn a_save_refuses_an_unknown_radio_and_a_shared_channel() {
    let dir = tempfile::tempdir().unwrap();
    let c = core(&dir, Arc::new(NoHooks));
    let mut p = SimCalibrationSaveParams {
        radio: "radio-0000000000000999".into(),
        ..Default::default()
    };
    assert!(c.gear_sim_calibration_save(&p).is_err());
    p.radio = "usb-0000000000000001".into();
    assert!(c.gear_sim_calibration_save(&p).unwrap().provisional);
    p.calibration.yaw.ch = 1;
    let e = c.gear_sim_calibration_save(&p).unwrap_err().to_string();
    assert!(e.contains("own channel"), "{e}");
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/switchmap")
}

#[test]
fn defaults_come_from_the_radio_model_and_the_quads_modes() {
    let dir = tempfile::tempdir().unwrap();
    let c = core(&dir, Arc::new(NoHooks));
    let d = c.gear_sim_defaults(&SimDefaultsParams::default()).unwrap();
    assert_eq!((d.map.roll, d.map.throttle), (1, 3));
    assert!(!d.map.known);
    // The synthetic whoop: AETR sticks, arm on SA, angle and horizon on SB, turtle on SD
    // (shared with BEEPER), no free control that moves a channel.
    let d = c
        .gear_sim_defaults(&SimDefaultsParams {
            radio: Some(fixtures().join("whoop")),
            fc: vec![fixtures().join("whoop/fc.diff_all.txt")],
            ..Default::default()
        })
        .unwrap();
    assert!(d.map.known);
    assert_eq!(d.map.source, "your radio model");
    let arm = d.arm.unwrap();
    assert_eq!(
        (arm.label.as_str(), arm.source.as_str()),
        ("SA down", "your radio model")
    );
    assert_eq!(
        arm.control,
        RadioControl::Channel {
            ch: 5,
            min_us: 1700,
            max_us: 2100
        }
    );
    assert_eq!(d.angle.unwrap().label, "SB up");
    assert_eq!(d.horizon.unwrap().label, "SB mid");
    assert_eq!(d.turtle.unwrap().label, "SD down");
    assert!(d.reset.is_none());
    assert!(d.notes.iter().any(|n| n.contains("setup asks")));
    // An aircraft with nothing linked: AETR, and why.
    let d = c
        .gear_sim_defaults(&SimDefaultsParams {
            aircraft: Some("Nothing linked".into()),
            ..Default::default()
        })
        .unwrap();
    assert!(d.notes.iter().any(|n| n.contains("No saved device")));
}
