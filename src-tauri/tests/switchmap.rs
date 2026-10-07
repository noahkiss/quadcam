//! The switch map (WP6): golden maps for two synthetic aircraft (a 3-position select,
//! `REPL` lines, `adjrange`), the live highlight from `FakeFc`'s `MSP_RC` and from the
//! radio's joystick (`FakeHid`), and the radio stream. No real device opens.

use quadcam_lib::api::Event;
use quadcam_lib::core::{Core, Hooks, NoHooks, RadioParams, RadioWatchParams, SwitchMapParams};
use quadcam_lib::gear::bf::cli::Timing;
use quadcam_lib::gear::bf::fake::FakeFc;
use quadcam_lib::gear::radio_hid::{report, FakeHid, RadioEvent};
use quadcam_lib::gear::switchmap::{self, SwitchMap};
use quadcam_lib::gear::Env;
use quadcam_lib::photos::Recorder;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PORT: &str = "/dev/cu.usbmodemFAKE1";

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/switchmap")
}

fn core_with(env: Env, hooks: Arc<dyn Hooks>, dir: &tempfile::TempDir) -> Core {
    Core::new(
        dir.path().join("cache"),
        None,
        hooks,
        Arc::new(Recorder::default()),
    )
    .with_settings(dir.path().join("support/settings.json"))
    .with_gear_env(env)
    .with_fc_timing(Timing::fast())
}

fn plain_core(dir: &tempfile::TempDir) -> Core {
    let ports = quadcam_lib::gear::serial::FakePorts::new(vec![]);
    core_with(Env::fake(vec![], Arc::new(ports)), Arc::new(NoHooks), dir)
}

fn whoop() -> SwitchMapParams {
    SwitchMapParams {
        radio: Some(fixtures().join("whoop")),
        fc: vec![fixtures().join("whoop/fc.diff_all.txt")],
        ..Default::default()
    }
}

fn cine() -> SwitchMapParams {
    SwitchMapParams {
        radio: Some(fixtures().join("cine/MODELS/model03.yml")),
        fc: vec![fixtures().join("cine/fc.dump_all.txt")],
        ..Default::default()
    }
}

fn map(p: SwitchMapParams) -> SwitchMap {
    let dir = tempfile::tempdir().unwrap();
    plain_core(&dir).gear_switch_map(&p).unwrap()
}

#[test]
fn golden_whoop_three_position_select() {
    let m = map(whoop());
    insta::assert_snapshot!("whoop", switchmap::render_text(&m));
}

#[test]
fn golden_cine_repl_and_adjrange() {
    let m = map(cine());
    insta::assert_snapshot!("cine", switchmap::render_text(&m));
}

const G473: &str = include_str!("fixtures/bf/g473-2025.12.5.dump_all.txt");

/// Whoop: SA down (armed), SB mid, SC down, SD up.
const WHOOP_LIVE: [u16; 8] = [1500, 1500, 988, 1500, 2012, 1500, 2012, 988];

fn check_whoop_live(m: &SwitchMap, source: &str) {
    let l = m.live.as_ref().expect("a live view");
    assert_eq!(l.source, source);
    assert_eq!(l.positions["SA"], Some(1), "SA down");
    assert_eq!(l.positions["SB"], Some(1), "SB mid");
    assert_eq!(l.positions["SC"], Some(2), "SC down");
    assert_eq!(l.positions["SD"], Some(0), "SD up");
    assert_eq!(l.positions["SE"], None, "SE moves no channel");
    assert_eq!(l.modes, vec!["ARM", "HORIZON"]);
    assert_eq!(l.adjustments, vec!["Rate profile 3"]);
}

#[test]
fn live_highlight_from_fake_fc_msp_rc() {
    let dir = tempfile::tempdir().unwrap();
    let locks = dir.path().join("locks");
    let fc = FakeFc::new(G473);
    fc.set_rc(&WHOOP_LIVE);
    let env = Env::fake(vec![], Arc::new(fc.ports(PORT, Some(locks.clone()))));
    let core = core_with(env, Arc::new(NoHooks), &dir);
    let m = core
        .gear_switch_map(&SwitchMapParams {
            live: true,
            ..whoop()
        })
        .unwrap();
    check_whoop_live(&m, "fc");
    assert!(fc.log().iter().any(|l| l == "msp 105"), "{:?}", fc.log());
    assert!(
        quadcam_lib::gear::serial::lock_port(&locks, PORT).is_ok(),
        "the port is released after the read"
    );
    insta::assert_snapshot!("whoop_live", switchmap::render_text(&m));
    // A position no switch gives matches nothing.
    fc.set_rc(&[1500, 1500, 988, 1500, 1250, 1500, 2012, 988]);
    let m = core
        .gear_switch_map(&SwitchMapParams {
            live: true,
            ..whoop()
        })
        .unwrap();
    assert_eq!(m.live.unwrap().positions["SA"], None);
}

#[test]
fn live_highlight_from_the_radio_joystick_and_given_channels() {
    let dir = tempfile::tempdir().unwrap();
    let hid = FakeHid::plugged();
    // Axes are CH1-8: 0 is 988 µs, 1024 is 1500, 2048 is 2012.
    hid.push(report(0, [1024, 1024, 0, 1024, 2048, 1024, 2048, 0]));
    let core = plain_core(&dir).with_radio_hid(Arc::new(hid));
    let m = core
        .gear_switch_map(&SwitchMapParams {
            live: true,
            ..whoop()
        })
        .unwrap();
    check_whoop_live(&m, "radio");
    let m = core
        .gear_switch_map(&SwitchMapParams {
            channels: WHOOP_LIVE.to_vec(),
            ..whoop()
        })
        .unwrap();
    check_whoop_live(&m, "given");
    // No FC and no radio: says why.
    let dir2 = tempfile::tempdir().unwrap();
    let e = plain_core(&dir2)
        .gear_switch_map(&SwitchMapParams {
            live: true,
            ..whoop()
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("USB Joystick"), "{e:#}");
}

#[test]
fn cine_live_repl_and_step_adjustment() {
    // SC down replaces CH6 with 1500 (HORIZON); SE pressed puts CH8 at 1500.
    let m = map(SwitchMapParams {
        channels: vec![1500, 1500, 988, 1500, 988, 1500, 2012, 1500],
        ..cine()
    });
    let l = m.live.unwrap();
    assert_eq!(l.positions["SC"], Some(2));
    assert_eq!(l.positions["SE"], Some(1));
    assert_eq!(l.modes, vec!["HORIZON", "VTX PIT MODE", "AIR MODE"]);
    assert_eq!(l.adjustments, vec!["OSD profile 3", "Roll rate adjust"]);
}

#[test]
fn sources_and_refusals() {
    let dir = tempfile::tempdir().unwrap();
    let core = plain_core(&dir);
    // FC only: modes, no rows.
    let m = core
        .gear_switch_map(&SwitchMapParams {
            fc: vec![fixtures().join("whoop/fc.diff_all.txt")],
            ..Default::default()
        })
        .unwrap();
    assert!(m.rows.is_empty());
    assert_eq!(m.modes.len(), 6);
    assert!(m.notes[0].contains("No EdgeTX model"));
    // A named model on the card.
    let m = core
        .gear_switch_map(&SwitchMapParams {
            radio: Some(fixtures().join("cine")),
            model: Some("model03.yml".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(m.model.as_deref(), Some("CINE TWO"));
    assert!(core.gear_switch_map(&SwitchMapParams::default()).is_err());
    let e = core
        .gear_switch_map(&SwitchMapParams {
            aircraft: Some("Whoop".into()),
            ..Default::default()
        })
        .unwrap_err();
    assert!(e.to_string().contains("backups"));
}

#[derive(Default)]
struct Rec(Mutex<Vec<RadioEvent>>);
impl Hooks for Rec {
    fn event(&self, e: Event) {
        if let Event::RadioInput(r) = e {
            self.0.lock().unwrap().push(r.0);
        }
    }
}

#[test]
fn radio_snapshot_and_stream() {
    let dir = tempfile::tempdir().unwrap();
    let hid = FakeHid::plugged();
    let rec = Arc::new(Rec::default());
    let core = core_with(
        Env::fake(
            vec![],
            Arc::new(quadcam_lib::gear::serial::FakePorts::new(vec![])),
        ),
        rec.clone(),
        &dir,
    )
    .with_radio_hid(Arc::new(hid.clone()));
    hid.push(report(0b101, [1024, 1024, 0, 1024, 2048, 0, 0, 0]));
    let s = core
        .gear_radio(&RadioParams { wait_ms: Some(500) })
        .unwrap();
    let f = s.frame.unwrap();
    assert_eq!(f.buttons, 0b101);
    assert_eq!(&f.channels[..5], &[1500, 1500, 988, 1500, 2012]);
    assert!(core
        .gear_radio_watch(&RadioWatchParams { on: true })
        .unwrap());
    assert!(
        core.gear_radio_watch(&RadioWatchParams { on: true })
            .unwrap(),
        "a second start keeps one stream"
    );
    hid.push(report(0, [0, 1024, 0, 1024, 0, 0, 0, 0]));
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end
        && !rec
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.frame.as_ref().is_some_and(|f| f.channels[0] == 988))
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(rec.0.lock().unwrap().iter().any(|e| e.connected));
    assert!(rec
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|e| e.frame.as_ref().is_some_and(|f| f.channels[0] == 988)));
    assert!(!core
        .gear_radio_watch(&RadioWatchParams { on: false })
        .unwrap());
    let n = rec.0.lock().unwrap().len();
    hid.push(report(0, [2048; 8]));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        rec.0.lock().unwrap().len(),
        n,
        "a stopped stream sends nothing"
    );
}

#[test]
fn cli_mcp_and_dispatch_surfaces() {
    use quadcam_lib::mcp::{LocalBackend, Server};
    use serde_json::{json, Value};
    let dir = tempfile::tempdir().unwrap();
    let hid = FakeHid::plugged();
    hid.push(report(0, [1024, 1024, 0, 1024, 2048, 1024, 2048, 0]));
    hid.push(report(0, [1024, 1024, 0, 1024, 2048, 1024, 2048, 0]));
    let core = Arc::new(plain_core(&dir).with_radio_hid(Arc::new(hid)));
    let whoop = fixtures().join("whoop");
    let dump = fixtures().join("whoop/fc.diff_all.txt");
    let v = core
        .dispatch(
            "gear_switch_map",
            json!({"radio": whoop, "fc": [dump], "channels": WHOOP_LIVE}),
        )
        .unwrap();
    assert_eq!(v["live"]["modes"], json!(["ARM", "HORIZON"]));

    let mut s = Server::new(LocalBackend(core.clone()));
    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "switch_map", "mount": whoop, "paths": [dump], "live": true}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("* down      CH5 2012"), "{text}");
    assert_eq!(r["structuredContent"]["live"]["source"], "radio");
    let r = s.call_tool("quadcam_gear", json!({"action": "radio"}));
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("CH5 2012"), "{text}");

    let run = |args: &[&str]| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
            .args(args)
            .env("HOME", dir.path())
            .env("QUADCAM_PHOTOS", "dry-run")
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };
    let (w, d) = (whoop.to_str().unwrap(), dump.to_str().unwrap());
    let (code, out) = run(&[
        "--json",
        "gear",
        "map",
        "--radio",
        w,
        "--fc",
        d,
        "--channels",
        "1500,1500,988,1500,2012,1500,2012,988",
    ]);
    assert_eq!(code, 0, "{out}");
    let j: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        j["result"]["live"]["adjustments"],
        json!(["Rate profile 3"])
    );
    let (code, out) = run(&["gear", "map", "--radio", w, "--fc", d, "--text"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with("Switch map: WHOOP ONE"), "{out}");
    // The CLI under cargo reaches no real radio.
    let (code, out) = run(&["--json", "gear", "radio", "--wait-ms", "10"]);
    assert_eq!(code, 0, "{out}");
    let j: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(j["result"]["connected"], false);
}
