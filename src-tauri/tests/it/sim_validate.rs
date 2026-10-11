//! The sim's validation harness (S2) through `Core` and `quadcam-cli`: a folder of decoded
//! logs (the sim crate's CI fixtures) against the built-in example profiles.

use quadcam_lib::core::{Core, NoHooks, SimValidateParams};
use quadcam_lib::photos::Recorder;
use quadcam_sim::validate::CheckOutcome;
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

#[test]
fn core_validates_a_folder_of_decoded_logs() {
    let dir = tempfile::tempdir().unwrap();
    let c = core(&dir);
    for id in ["meteor75", "air65ii"] {
        let r = c
            .gear_sim_validate(&SimValidateParams {
                aircraft: id.into(),
                logs: fixtures(id),
                motor_poles: None,
            })
            .unwrap();
        assert!(r.passed(), "{id}: {:?}", r.failures());
        assert!(r
            .checks
            .iter()
            .any(|c| c.check == "punch" && c.outcome == CheckOutcome::Pass));
    }
    let err = c
        .gear_sim_validate(&SimValidateParams {
            aircraft: "nope".into(),
            logs: fixtures("meteor75"),
            motor_poles: None,
        })
        .unwrap_err();
    assert!(err.to_string().contains("meteor75"), "{err}");
    let empty = tempfile::tempdir().unwrap();
    assert!(c
        .gear_sim_validate(&SimValidateParams {
            aircraft: "meteor75".into(),
            logs: empty.path().into(),
            motor_poles: None,
        })
        .is_err());
}

#[test]
fn cli_validates_a_folder_of_decoded_logs() {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
        .env("HOME", home.path())
        .env("QUADCAM_PHOTOS", "dry-run")
        .args(["--json", "gear", "sim", "validate", "air65ii", "--logs"])
        .arg(fixtures("air65ii"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true);
    let v = &v["result"];
    assert_eq!(v["profile"], "air65ii");
    assert_eq!(v["logs"].as_array().unwrap().len(), 5);
    assert!(v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["outcome"] != "fail"));
    let text = Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
        .env("HOME", home.path())
        .env("QUADCAM_PHOTOS", "dry-run")
        .args(["gear", "sim", "validate", "meteor75", "--text", "--logs"])
        .arg(fixtures("meteor75"))
        .output()
        .unwrap();
    let t = String::from_utf8_lossy(&text.stdout);
    assert!(t.contains("hover") && t.trim_end().ends_with("PASS"), "{t}");
}
