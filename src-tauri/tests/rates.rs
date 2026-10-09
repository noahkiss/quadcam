//! Rates and sims, the read side (design 6.6, WP8): the rate profiles of a dump, the curves
//! for the three rate models, the throttle curve, the sims read from synthetic files at their
//! design paths under a temp home, the comparison with the quad, and the three surfaces (the
//! `api` rows, the CLI, the MCP actions).

use quadcam_lib::core::{Core, NoHooks, RatesParams, SimsParams};
use quadcam_lib::gear::rates;
use quadcam_lib::mcp::{LocalBackend, Server};
use quadcam_lib::photos::Recorder;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn fixture(dir: &str, name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(dir)
        .join(name)
}

fn core(dir: &Path) -> Arc<Core> {
    Arc::new(
        Core::new(
            dir.join("cache"),
            None,
            Arc::new(NoHooks),
            Arc::new(Recorder::default()),
        )
        .with_settings(dir.join("support/settings.json"))
        .with_gear_env(quadcam_lib::gear::Env::fake(
            vec![],
            Arc::new(quadcam_lib::gear::serial::FakePorts::new(vec![])),
        )),
    )
}

fn three() -> PathBuf {
    fixture("rates", "three-profiles.dump_all.txt")
}

/// Puts a sim's synthetic file where the game keeps it, under `home`.
fn put(home: &Path, rel: &str, bytes: &[u8]) {
    let p = home.join("Library/Application Support").join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

/// The nine floats of an Uncrashed profile on the Betaflight type.
fn gvas(floats: &[f32]) -> Vec<u8> {
    let mut b = b"GVAS".to_vec();
    b.extend_from_slice(&[3, 0, 0, 0, 10, 2, 0, 0]);
    for s in ["Rates", "ArrayProperty", "FloatProperty"] {
        b.extend_from_slice(&(s.len() as i32 + 1).to_le_bytes());
        b.extend_from_slice(s.as_bytes());
        b.push(0);
    }
    b.push(0);
    b.extend_from_slice(&(floats.len() as i32).to_le_bytes());
    for f in floats {
        b.extend_from_slice(&f.to_le_bytes());
    }
    b.extend_from_slice(b"\x05\0\0\0None\0");
    b
}

fn sims_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let fx = |n: &str| std::fs::read(fixture("sims", n)).unwrap();
    put(
        home.path(),
        "LuGus Studios/Liftoff/Saves/Player/UserData.xml",
        &fx("liftoff.UserData.xml"),
    );
    put(
        home.path(),
        "Steam/steamapps/common/Liftoff Micro Drones/Liftoff Micro Drones.app/Contents/Saves/Player/UserData.xml",
        &fx("micro.UserData.xml"),
    );
    // Uncrashed: the FREE profile equals the quad's profile 0; throttle mid 30, expo 50.
    put(
        home.path(),
        "Uncrashed/abc123/rates/FREE.sav",
        &gvas(&[
            0.72, 1.27, 0.40, 0.72, 1.27, 0.40, 0.75, 1.00, 0.0, 0.0, 0.30, 0.50,
        ]),
    );
    put(
        home.path(),
        "Godot/app_userdata/The Zone/settings.cfg",
        &fx("zone.settings.cfg"),
    );
    home
}

#[test]
fn rate_profiles_of_a_dump_with_names_curves_and_throttle() {
    let dir = tempfile::tempdir().unwrap();
    let v = core(dir.path())
        .gear_rates(&RatesParams {
            paths: vec![three()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(v.profiles.len(), 3);
    assert_eq!(v.active, Some(0));
    let names: Vec<_> = v
        .profiles
        .iter()
        .map(|p| p.name.as_deref().unwrap())
        .collect();
    assert_eq!(names, ["FREE", "RACE", "CINE"]);
    let types: Vec<_> = v.profiles.iter().map(|p| p.rates_type.as_str()).collect();
    assert_eq!(types, ["betaflight", "actual", "quick"]);
    // Maximum and centre rates, from the published formulas.
    let (free, race, cine) = (&v.profiles[0], &v.profiles[1], &v.profiles[2]);
    assert!((free.axes[0].max_deg_s - 907.14).abs() < 0.01);
    assert!((race.axes[0].max_deg_s - 670.0).abs() < 1e-9);
    assert!((race.axes[0].center_deg_s - 70.0).abs() < 0.1);
    // Quick: rc 60 -> 120 deg/s centre; max 300, under the 500 limit; yaw max 200.
    assert!((cine.axes[0].center_deg_s - 120.0).abs() < 0.5);
    assert!((cine.axes[0].max_deg_s - 300.0).abs() < 1e-9);
    assert!((cine.axes[2].max_deg_s - 200.0).abs() < 1e-9);
    assert_eq!(cine.throttle.limit, "clip");
    assert!((cine.throttle.curve[50] - 0.8).abs() < 1e-12);
    // thr_mid 30, hover 34: the output at stick 0.3 is the hover value.
    assert!((free.throttle.curve[15] - 0.34).abs() < 1e-12);
    insta::assert_snapshot!("three_profiles", rates::render_text(&v));
}

#[test]
fn a_real_dump_reads_and_a_diff_says_it_is_partial() {
    let dir = tempfile::tempdir().unwrap();
    let c = core(dir.path());
    let real = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/bf/g473v2-2026.6.0.dump_all.txt");
    let v = c
        .gear_rates(&RatesParams {
            paths: vec![real],
            ..Default::default()
        })
        .unwrap();
    assert!(v.profiles.len() >= 3, "{}", v.profiles.len());
    assert!(v.profiles.iter().all(|p| p.complete));
    assert!((v.profiles[0].axes[0].max_deg_s - 907.14).abs() < 0.01);
    let v = c
        .gear_rates(&RatesParams {
            paths: vec![fixture("rates", "diff-only.diff_all.txt")],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(v.profiles.len(), 1);
    assert!(!v.profiles[0].complete);
    assert_eq!(v.profiles[0].axes[0].srate, 60.0);
    assert_eq!(v.profiles[0].axes[1].srate, 67.0);
    assert!(v.notes.iter().any(|n| n.contains("dump")));
}

#[test]
fn refusals() {
    let dir = tempfile::tempdir().unwrap();
    let c = core(dir.path());
    let err = |p: RatesParams| format!("{:#}", c.gear_rates(&p).unwrap_err());
    assert!(err(RatesParams::default()).contains("Pass a device"));
    assert!(err(RatesParams {
        device: Some("fc-unknown".into()),
        ..Default::default()
    })
    .starts_with("No device"));
    assert!(err(RatesParams {
        backup: Some("fc-unknown/none".into()),
        ..Default::default()
    })
    .contains("No backup"));
    assert!(err(RatesParams {
        paths: vec![dir.path().to_path_buf()],
        ..Default::default()
    })
    .contains("is not a Betaflight dump"));
}

#[test]
fn sims_are_read_from_their_files_and_compared_with_the_quad() {
    let dir = tempfile::tempdir().unwrap();
    let home = sims_home();
    let c = core(dir.path());
    let running = |name: &str| name == "Liftoff";
    let list = c
        .gear_sims_at(
            &SimsParams {
                paths: vec![three()],
                ..Default::default()
            },
            home.path(),
            &running,
        )
        .unwrap();
    let ids: Vec<_> = list.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        ids,
        ["liftoff", "micro", "uncrashed", "zone", "velocidrone"]
    );
    let by = |id: &str| list.iter().find(|s| s.id == id).unwrap();

    // Liftoff: Freestyle equals the quad's profile 0 (the one in use); Race does not.
    let lift = by("liftoff");
    assert!(lift.found && lift.running && lift.enabled);
    assert_eq!(
        lift.files[0].path,
        "~/Library/Application Support/LuGus Studios/Liftoff/Saves/Player/UserData.xml"
    );
    assert_eq!(lift.in_sync, Some(true));
    let p = &lift.files[0].profiles;
    assert_eq!(p[0].name, "Freestyle");
    assert!(p[0].diff.as_ref().unwrap().same);
    assert!(p[0].throttle.is_none());
    assert!(!p[1].diff.as_ref().unwrap().same);
    assert!(p[1].diff.as_ref().unwrap().max_diff[0] > 100.0);

    // Micro Drones is found inside the Steam app bundle; not running.
    let micro = by("micro");
    assert!(micro.found && !micro.running);
    assert_eq!(micro.in_sync, Some(false));
    assert_eq!(micro.files[0].profiles[0].name, "Micro");

    // Uncrashed: rates equal and the throttle's mid and expo equal (30 and 50). A sim has no
    // hover value, so the quad's hover (34) is not a difference; the sim matches.
    let unc = by("uncrashed");
    let prof = &unc.files[0].profiles[0];
    assert_eq!(prof.name, "FREE");
    assert!(prof.throttle.is_some());
    assert_eq!(prof.diff.as_ref().unwrap().throttle_differs, Some(false));
    assert_eq!(unc.in_sync, Some(true));

    // The Zone: profile 0 equals the quad; profile 1 uses another type.
    let zone = by("zone");
    assert_eq!(zone.in_sync, Some(true));
    assert!(!zone.files[0].profiles[1].supported);

    // Velocidrone ships off.
    let velo = by("velocidrone");
    assert!(!velo.enabled && !velo.found && !velo.running);
    assert!(velo.note.is_some());

    // Against the quad's profile 1 (Actual): the sim holds Betaflight, so the quad is
    // fitted first and the fit error is reported.
    let list = c
        .gear_sims_at(
            &SimsParams {
                paths: vec![three()],
                profile: Some(1),
                ..Default::default()
            },
            home.path(),
            &|_| false,
        )
        .unwrap();
    let d = list[0].files[0].profiles[0].diff.as_ref().unwrap();
    assert!(d.fit_error[0] > 0.0);
    assert_eq!(list[0].in_sync, Some(false));
    assert!(!list[0].running);
}

#[test]
fn sims_without_a_quad_list_only_and_a_missing_home_finds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let c = core(dir.path());
    let home = sims_home();
    let list = c
        .gear_sims_at(&SimsParams::default(), home.path(), &|_| false)
        .unwrap();
    assert!(list.iter().all(|s| s.in_sync.is_none()));
    assert!(list[0].files[0].profiles[0].diff.is_none());
    let empty = tempfile::tempdir().unwrap();
    let list = c
        .gear_sims_at(&SimsParams::default(), empty.path(), &|_| false)
        .unwrap();
    assert!(list.iter().all(|s| !s.found));
}

#[test]
fn a_broken_sim_file_is_reported_not_guessed() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    put(
        home.path(),
        "LuGus Studios/Liftoff/Saves/Player/UserData.xml",
        b"<UserData><rateProfiles>",
    );
    let list = core(dir.path())
        .gear_sims_at(&SimsParams::default(), home.path(), &|_| false)
        .unwrap();
    let f = &list[0].files[0];
    assert!(f.error.as_deref().unwrap().contains("never closed"));
    assert!(f.profiles.is_empty());
}

#[test]
fn the_rows_cli_and_mcp_actions() {
    let dir = tempfile::tempdir().unwrap();
    let c = core(dir.path());
    let v = c
        .dispatch("gear_rates", json!({"paths": [three()]}))
        .unwrap();
    assert_eq!(v["profiles"].as_array().unwrap().len(), 3);
    assert_eq!(v["profiles"][1]["rates_type"], "actual");
    // Under cargo a sim counts as running only when the test says so.
    let v = c.dispatch("gear_sims", json!({})).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 5);
    assert!(v.as_array().unwrap().iter().all(|s| s["running"] == false));

    let home = sims_home();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
            .args(args)
            .env("HOME", home.path())
            .env("QUADCAM_PHOTOS", "dry-run")
            .env("QUADCAM_SIMS_RUNNING", "Uncrashed")
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };
    let quad = three();
    let quad = quad.to_str().unwrap();
    let (code, out) = run(&["--json", "gear", "rates", quad]);
    assert_eq!(code, 0, "{out}");
    let j: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(j["result"]["profiles"][2]["name"], "CINE");
    let (code, out) = run(&["gear", "rates", quad, "--text"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("Rate profile 0 FREE (in use) - betaflight"),
        "{out}"
    );
    let (code, out) = run(&["--json", "gear", "sims", quad]);
    assert_eq!(code, 0, "{out}");
    let j: Value = serde_json::from_str(&out).unwrap();
    let list = j["result"].as_array().unwrap();
    let unc = list.iter().find(|s| s["id"] == "uncrashed").unwrap();
    assert_eq!(unc["running"], true);
    let lift = list.iter().find(|s| s["id"] == "liftoff").unwrap();
    assert_eq!(lift["running"], false);
    assert_eq!(lift["in_sync"], true);
    let (code, out) = run(&["gear", "sims", quad, "--text"]);
    assert_eq!(code, 0);
    assert!(out.contains("Liftoff - matches the quad"), "{out}");
    assert!(
        out.contains("Uncrashed (running) - matches the quad"),
        "{out}"
    );
    let (code, out) = run(&["--json", "gear", "rates", "fc-nothing"]);
    assert_eq!(code, 4, "{out}");

    let mut s = Server::new(LocalBackend(c));
    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "rates", "paths": [three()]}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Rate profile 1 RACE - actual"), "{text}");
    assert_eq!(r["structuredContent"]["profiles"][0]["name"], "FREE");
    let r = s.call_tool("quadcam_gear", json!({"action": "sims"}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["sims"].as_array().unwrap().len(), 5);
    let r = s.call_tool("quadcam_gear", json!({"action": "rates"}));
    assert_eq!(r["isError"], true);
}

#[test]
fn a_preview_redraws_an_edited_profile_and_converts_with_the_fit() {
    use quadcam_lib::core::RatesPreviewParams;
    let dir = tempfile::tempdir().unwrap();
    let c = core(dir.path());
    let view = c
        .gear_rates(&RatesParams {
            paths: vec![three()],
            ..Default::default()
        })
        .unwrap();
    let free = view.profiles[0].clone();
    // As read: the preview is the profile, curve for curve.
    let same = c
        .gear_rates_preview(&RatesPreviewParams {
            profile: free.clone(),
            to: None,
        })
        .unwrap();
    assert_eq!(same.profile.axes[0].curve, free.axes[0].curve);
    assert_eq!(same.fit_error, [0.0, 0.0, 0.0]);
    // A higher RC rate raises the maximum, and the other axes stay.
    let mut edited = free.clone();
    edited.axes[0].rc_rate = 150.0;
    let p = c
        .gear_rates_preview(&RatesPreviewParams {
            profile: edited,
            to: None,
        })
        .unwrap();
    assert!(p.profile.axes[0].max_deg_s > free.axes[0].max_deg_s + 10.0);
    assert_eq!(p.profile.axes[1].curve, free.axes[1].curve);
    // Actual to Betaflight: the existing fit, with the gap per axis, whole numbers.
    let race = view.profiles[1].clone();
    assert_eq!(race.rates_type, "actual");
    let to = c
        .gear_rates_preview(&RatesPreviewParams {
            profile: race.clone(),
            to: Some("betaflight".into()),
        })
        .unwrap();
    assert_eq!(to.profile.rates_type, "betaflight");
    assert!(to.fit_error[0] > 0.0 && to.fit_share[0] < 0.12);
    assert_eq!(to.profile.axes[0].rc_rate.fract(), 0.0);
    assert_eq!(to.profile.index, race.index);
    let gap = to.profile.axes[0]
        .curve
        .iter()
        .zip(&race.axes[0].curve)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f64::max);
    assert!(
        (gap - to.fit_error[0]).abs() < 12.0,
        "{gap} {}",
        to.fit_error[0]
    );
    // Refusals: a model that does not exist, a value that is not a number.
    assert!(c
        .gear_rates_preview(&RatesPreviewParams {
            profile: free.clone(),
            to: Some("nope".into()),
        })
        .is_err());
    let mut nan = free;
    nan.axes[0].expo = f64::NAN;
    assert!(c
        .gear_rates_preview(&RatesPreviewParams {
            profile: nan,
            to: None,
        })
        .is_err());
}
