//! Sim profiles fitted from logs (S3) through `Core` and `quadcam-cli`: the fit on the sim
//! crate's CI fixtures, the save behind confirm and the fit's digest, the list, and the
//! harness on a fitted profile by its id.

use quadcam_lib::core::{
    Core, NoHooks, SimFitParams, SimFitSaveParams, SimProfileDeleteParams, SimValidateParams,
};
use quadcam_lib::photos::Recorder;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

fn fixtures(id: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("sim/tests/fixtures")
        .join(id)
}

fn core(dir: &tempfile::TempDir) -> Core {
    Core::new(
        dir.path().join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(dir.path().join("support/settings.json"))
}

fn params() -> SimFitParams {
    SimFitParams {
        base: "air65ii".into(),
        logs: vec![fixtures("air65ii")],
        mass_g: Some(26.0),
        id: Some("my-air65".into()),
        ..Default::default()
    }
}

#[test]
fn a_fit_saves_only_with_confirm_and_its_digest() {
    let dir = tempfile::tempdir().unwrap();
    let c = core(&dir);
    let f = c.gear_sim_fit(&params()).unwrap();
    assert!(f.validation.passed(), "{:?}", f.validation.failures());
    assert!(!f.replaces);
    assert_eq!(f.report.profile_id, "my-air65");
    assert!(c
        .gear_sim_profiles()
        .unwrap()
        .iter()
        .all(|p| p.kind == "preset"));
    let save = |digest: &str, confirm: bool| {
        c.gear_sim_fit_save(&SimFitSaveParams {
            fit: params(),
            digest: digest.into(),
            confirm,
        })
    };
    assert!(save(&f.digest, false).is_err(), "no confirm");
    assert!(save("0123", true).is_err(), "another digest");
    let info = save(&f.digest, true).unwrap();
    assert_eq!(
        (info.id.as_str(), info.kind.as_str()),
        ("my-air65", "fitted")
    );
    assert!((info.mass_g - 26.0).abs() < 1e-9);
    let list = c.gear_sim_profiles().unwrap();
    assert!(list
        .iter()
        .any(|p| p.id == "my-air65" && p.base.as_deref() == Some("air65ii")));
    // The harness takes the fitted profile by its id.
    let r = c
        .gear_sim_validate(&SimValidateParams {
            aircraft: "my-air65".into(),
            logs: fixtures("air65ii"),
            motor_poles: None,
        })
        .unwrap();
    assert!(r.passed(), "{:?}", r.failures());
    assert!(c.gear_sim_fit(&params()).unwrap().replaces);
    assert!(c
        .gear_sim_profile_delete(&SimProfileDeleteParams {
            id: "my-air65".into(),
            confirm: false
        })
        .is_err());
    assert!(c
        .gear_sim_profile_delete(&SimProfileDeleteParams {
            id: "my-air65".into(),
            confirm: true
        })
        .unwrap());
    assert!(c
        .gear_sim_profiles()
        .unwrap()
        .iter()
        .all(|p| p.kind == "preset"));
}

#[test]
fn a_fit_refuses_a_preset_id_no_logs_and_an_unknown_base() {
    let dir = tempfile::tempdir().unwrap();
    let c = core(&dir);
    let mut p = params();
    p.id = Some("air65ii".into());
    assert!(c.gear_sim_fit(&p).is_err());
    let mut p = params();
    p.logs.clear();
    let e = c.gear_sim_fit(&p).unwrap_err().to_string();
    assert!(e.contains("No blackbox logs"), "{e}");
    let mut p = params();
    p.base = "nope".into();
    assert!(c.gear_sim_fit(&p).is_err());
    let mut p = params();
    p.aircraft = Some("Nobody's quad".into());
    p.logs.clear();
    let e = c.gear_sim_fit(&p).unwrap_err().to_string();
    assert!(e.contains("No FC is linked"), "{e}");
}

#[test]
fn cli_fits_and_saves_with_the_digest() {
    let home = tempfile::tempdir().unwrap();
    let cli = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
            .env("HOME", home.path())
            .env("QUADCAM_PHOTOS", "dry-run")
            .args(args)
            .output()
            .unwrap();
        let v: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
        (
            out.status.success(),
            v,
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    };
    let logs = fixtures("meteor75");
    let logs = logs.to_str().unwrap();
    let fit = [
        "--json", "gear", "sim", "fit", "--base", "meteor75", "--logs", logs, "--id", "mine",
    ];
    let (ok, v, err) = cli(&fit);
    assert!(ok, "{err}");
    let digest = v["result"]["digest"].as_str().unwrap().to_string();
    let mut save = fit.to_vec();
    save.extend(["--digest", &digest]);
    let (ok, _, _) = cli(&save);
    assert!(!ok, "a save needs --yes");
    save.push("--yes");
    let (ok, v, err) = cli(&save);
    assert!(ok, "{err}");
    assert_eq!(v["result"]["id"], "mine");
    let (ok, v, _) = cli(&["--json", "gear", "sim", "profiles"]);
    assert!(ok);
    assert!(v["result"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["id"] == "mine" && p["kind"] == "fitted"));
}
