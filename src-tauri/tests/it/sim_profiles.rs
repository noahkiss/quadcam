//! Sim profiles fitted from logs (S3) through `Core` and `quadcam-cli`: the fit on the sim
//! crate's CI fixtures, the save behind confirm and the fit's digest, the list, and the
//! harness on a fitted profile by its id, and the aircraft's radio logs (the synthetic
//! `gear::flights::synth` sag log) for pack resistance.

use quadcam_lib::api;
use quadcam_lib::core::{
    Core, NoHooks, SimFitParams, SimFitSaveParams, SimProfileDeleteParams, SimValidateParams,
};
use quadcam_lib::gear::flights::synth;
use quadcam_lib::gear::serial::FakePorts;
use quadcam_lib::gear::Env;
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
    // The Sim page lists it after the built-in profiles.
    let page = c.sim_presets();
    assert!(page.iter().any(|p| p.id == "my-air65" && p.fitted));
    assert!(page.iter().filter(|p| !p.fitted).count() >= 4);
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
fn a_fit_reads_the_aircraft_radio_logs_for_pack_resistance() {
    let dir = tempfile::tempdir().unwrap();
    let support = dir.path().join("support");
    std::fs::create_dir_all(&support).unwrap();
    std::fs::write(
        support.join("settings.json"),
        r#"{"profiles":[{"name":"Sagger","edgetx_models":["Sagger"]},{"name":"Other","edgetx_models":["Whoop"]}]}"#,
    )
    .unwrap();
    let logs = dir.path().join("radio");
    synth::write_sag(&logs, "Sagger").unwrap();
    // Another aircraft's log: never paired to Sagger.
    synth::write_known(&logs).unwrap();
    let c = core(&dir).with_gear_env(Env::fake(vec![], Arc::new(FakePorts::new(vec![]))));
    c.gear_flight_folders(&api::FlightFoldersParams {
        add: Some(logs),
        remove: None,
    })
    .unwrap();

    let mut p = params();
    p.aircraft = Some("Sagger".into());
    let f = c.gear_sim_fit(&p).unwrap();
    let radio: Vec<&String> = f
        .sources
        .iter()
        .filter(|s| s.starts_with("radio log"))
        .collect();
    assert_eq!(
        radio,
        ["radio log Sagger-2026-10-05-120000.csv (1 flight)"],
        "{:?}",
        f.sources
    );
    let r = f
        .report
        .measured
        .iter()
        .find(|m| m.name == "pack_resistance_radio")
        .expect("a radio-log pack resistance");
    assert!((r.value - synth::SAG_OHMS).abs() < 0.005, "{}", r.value);
    // The guard scored the motor and pack group against the base's.
    let power = f.report.groups.iter().find(|g| g.name == "power").unwrap();
    assert_eq!(power.kept, power.note.is_none());
    // The radio logs are fit input: the same fit gives the same digest.
    assert_eq!(c.gear_sim_fit(&p).unwrap().digest, f.digest);

    // Another aircraft reads only its own log, whose current holds too steady to fit.
    p.aircraft = Some("Other".into());
    let f = c.gear_sim_fit(&p).unwrap();
    assert!(f.sources.iter().all(|s| !s.contains("Sagger")));
    assert!(
        f.sources.iter().any(|s| s.contains(synth::FILE)),
        "{:?}",
        f.sources
    );
    assert!(f
        .report
        .measured
        .iter()
        .all(|m| m.name != "pack_resistance_radio"));
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
