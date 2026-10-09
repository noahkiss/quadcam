//! Upgrade path: the settings file and gear folder an older release wrote load in this
//! build without loss, and a write keeps every entry and key it does not own.
//!
//! `tests/fixtures/upgrade/0.7.0` and `0.9.0` are hand-made folders in the shape those
//! releases wrote (read off the release tags): `settings.json` and a gear folder with
//! `gear.json`, snapshots, `flights.json` and, for 0.9.0, staged changes, the sim
//! calibrations and the voice block. Every path in them points under `/tmp/quadcam-upgrade`,
//! which `lab` moves into a temporary folder.

use quadcam_lib::core::{BackupFilter, Core, DeviceSaveParams, NoHooks, VoiceParams};
use quadcam_lib::core::{CrashSaveParams, FlightFilter, PackSaveParams, PacksParams};
use quadcam_lib::gear::changes::ChangeFilter;
use quadcam_lib::gear::crashes::CrashFilter;
use quadcam_lib::gear::flights::{self, synth};
use quadcam_lib::gear::model::{ChangeStatus, Trigger};
use quadcam_lib::gear::packs::Pack;
use quadcam_lib::gear::serial::FakePorts;
use quadcam_lib::gear::{blobs, Env};
use quadcam_lib::photos::Recorder;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const PLACEHOLDER: &str = "/tmp/quadcam-upgrade";

struct Lab {
    dir: tempfile::TempDir,
    core: Core,
}

impl Lab {
    fn root(&self) -> &Path {
        self.dir.path()
    }
    fn settings_file(&self) -> PathBuf {
        self.root().join("support/settings.json")
    }
    fn gear(&self) -> PathBuf {
        self.root().join("gear")
    }
    fn settings(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.settings_file()).unwrap()).unwrap()
    }
    fn gear_json(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.gear().join("gear.json")).unwrap()).unwrap()
    }
}

fn fixture(version: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/upgrade")
        .join(version)
}

fn copy_dir(from: &Path, to: &Path, root: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            copy_dir(&src, &dst, root);
        } else {
            // The fixtures are text; their paths move under the lab.
            let text = std::fs::read_to_string(&src).unwrap();
            let moved = text.replace(PLACEHOLDER, root.to_str().unwrap());
            std::fs::write(&dst, moved).unwrap();
        }
    }
}

/// A lab holding a copy of the fixture, and a core on its settings file. The blobs the
/// fixture's snapshots name are written into the blob store.
fn lab(version: &str) -> Lab {
    let dir = tempfile::tempdir().unwrap();
    // The path the settings name is the one the lab reports: no `/private` alias.
    let root = dir.path().to_path_buf();
    let src = fixture(version);
    std::fs::create_dir_all(root.join("support")).unwrap();
    std::fs::copy(
        src.join("settings.json"),
        root.join("support/settings.json"),
    )
    .unwrap();
    let settings = std::fs::read_to_string(root.join("support/settings.json"))
        .unwrap()
        .replace(PLACEHOLDER, root.to_str().unwrap());
    std::fs::write(root.join("support/settings.json"), settings).unwrap();
    copy_dir(&src.join("gear"), &root.join("gear"), &root);
    std::fs::create_dir_all(root.join("logs")).unwrap();
    let core = Core::new(
        root.join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(root.join("support/settings.json"))
    .with_gear_env(Env::fake(vec![], Arc::new(FakePorts::new(vec![]))));
    let b = blobs::Blobs::new(core.gear_store());
    b.put(b"semver: 2.12.4\nboard: pocket\n").unwrap();
    b.put(b"set osd_vbat_pos = 2444\n").unwrap();
    Lab { dir, core }
}

/// Every top-level key of `old` is in `new` with the same value, except those in `except`.
fn keeps(old: &Value, new: &Value, except: &[&str]) {
    for (k, v) in old.as_object().unwrap() {
        if except.contains(&k.as_str()) {
            continue;
        }
        assert_eq!(new.get(k), Some(v), "{k} survived the write");
    }
}

fn set(v: Value) -> serde_json::Map<String, Value> {
    serde_json::from_value(v).unwrap()
}

// ----- settings -----

#[test]
fn a_0_7_0_settings_file_loads_with_every_value() {
    let l = lab("0.7.0");
    let old = l.settings();
    let v = l.core.settings().unwrap();
    let e = &v.effective;
    assert_eq!(e.photos_album, "Drone");
    assert_eq!(e.format_label, "ECHO");
    assert_eq!(e.default_profile.as_deref(), Some("Whoop A"));
    assert_eq!(e.profiles.len(), 1);
    assert_eq!(e.tunables.max_log_age_days, 60);
    // Secrets read redacted, never as the file's value.
    assert_eq!(v.values["ttsKey"], "(set)");

    let g = l.core.gear_settings();
    assert_eq!(g.gear_dir, l.gear());
    assert_eq!(g.keep_recent, 7);
    assert_eq!(g.keep_weeks, 4);
    assert!(!g.keep_monthly);
    assert_eq!(g.usb_minutes, 15);
    assert_eq!(g.firmware_check, "daily");
    assert_eq!(g.cues.voice.as_deref(), Some("Samantha"));
    assert!(g.cues.notification);
    // A 0.7.0 file has none of the later keys: they take their defaults.
    assert_eq!(v.values.get("simPreview"), None);
    assert_eq!(l.core.settings().unwrap().path, l.settings_file());
    assert_eq!(l.settings(), old, "reading changes nothing");
}

#[test]
fn a_0_7_0_settings_file_round_trips_through_a_write() {
    let l = lab("0.7.0");
    let old = l.settings();
    // A write from the 0.9.0 surface: a key the file never had, and one it did.
    l.core
        .settings_set(&set(
            json!({"sim_preview": true, "tts_voice": "af_heart", "geocoder": "census"}),
        ))
        .unwrap();
    let new = l.settings();
    keeps(&old, &new, &["geocoder"]);
    assert_eq!(new["simPreview"], true);
    assert_eq!(new["ttsVoice"], "af_heart");
    assert_eq!(new["geocoder"], "census");
    assert_eq!(new["stickMode"], 2, "a key outside the checked list stays");
    // Keys no release knows stay too.
    assert_eq!(new["someKeyFrom0_7_x"], json!({"nested": [1, 2, 3]}));
    // A profile's link keeps the field a later version added.
    assert_eq!(new["profiles"][0]["gear"]["futureLink"], "kept");
    assert_eq!(new["gearCues"]["futureCueField"], "kept");
}

#[test]
fn a_0_9_0_settings_file_loads_and_round_trips() {
    let l = lab("0.9.0");
    let old = l.settings();
    let v = l.core.settings().unwrap();
    assert_eq!(v.values["ttsBaseUrl"], "http://127.0.0.1:8880");
    assert_eq!(v.values["ttsModel"], "kokoro");
    assert_eq!(v.values["simPreview"], true);
    assert_eq!(v.values["simSettings"]["view"], "chase");
    assert_eq!(l.core.gear_settings().tts_provider, "openai");

    l.core
        .settings_set(&set(json!({"tts_model": "kokoro-v2"})))
        .unwrap();
    let new = l.settings();
    keeps(&old, &new, &["ttsModel"]);
    assert_eq!(new["ttsModel"], "kokoro-v2");
    // The Sim page's object keeps a key this build does not know.
    assert_eq!(new["simSettings"]["futureSimKey"], 1);
    assert_eq!(new["someKeyFrom0_9_x"], "kept");
}

// ----- gear folder -----

#[test]
fn a_0_7_0_gear_folder_loads() {
    let l = lab("0.7.0");
    let devices = l.core.gear_devices().unwrap();
    assert_eq!(devices.len(), 2);
    let radio = devices
        .iter()
        .find(|d| d.id == "radio-0123456789abcdef")
        .unwrap();
    assert_eq!(radio.name, "Pocket");
    assert_eq!(radio.identity.version.as_deref(), Some("2.11.2"));
    assert!(radio.last_space.is_none(), "0.7.0 kept no card space");
    let fc = devices
        .iter()
        .find(|d| d.kind == quadcam_lib::gear::model::DeviceKind::Fc)
        .unwrap();
    assert_eq!(fc.display_name(), "Unnamed FC");

    let packs = l.core.gear_packs(&PacksParams::default()).unwrap();
    assert_eq!(packs.packs.len(), 2);
    assert_eq!(packs.types.len(), 1);
    assert_eq!(packs.notes, "Charge to storage after a week.");
    let crashes = l.core.gear_crashes(&CrashFilter::default()).unwrap();
    assert_eq!(crashes.len(), 1);
    assert!(crashes[0].repaired);

    let backups = l.core.gear_backups(&BackupFilter::default()).unwrap();
    assert_eq!(backups.len(), 1);
    let read = l
        .core
        .gear_backup_read(&quadcam_lib::core::BackupReadParams {
            id: backups[0].id.clone(),
            path: Some("RADIO/radio.yml".into()),
        })
        .unwrap();
    assert!(serde_json::to_string(&read).unwrap().contains("pocket"));
}

#[test]
fn a_0_7_0_gear_folder_keeps_everything_through_writes() {
    let l = lab("0.7.0");
    let old = l.gear_json();
    // A device edit, a pack save and a crash save, the three writers of gear.json.
    l.core
        .gear_device_save(&DeviceSaveParams {
            id: "radio-0123456789abcdef".into(),
            name: Some("Pocket 2".into()),
            aircraft: None,
        })
        .unwrap();
    l.core
        .gear_pack_save(&PackSaveParams {
            pack: Pack {
                label: "A2".into(),
                pack_type: Some("1S 300".into()),
                ..Default::default()
            },
            charged: None,
        })
        .unwrap();
    l.core
        .gear_crash_save(&CrashSaveParams {
            id: Some("crash-1".into()),
            note: Some("tree, again".into()),
            ..Default::default()
        })
        .unwrap();
    let new = l.gear_json();
    keeps(&old, &new, &["devices", "packs", "crashes"]);
    assert_eq!(new["futureTopLevelKey"], json!({"keep": "me"}));
    // The device keeps the field this build does not know, and the edit landed.
    assert_eq!(new["devices"][0]["name"], "Pocket 2");
    assert_eq!(new["devices"][0]["nickname"], "kept");
    // The identity gains the fields it did not have, as null; none it had changes.
    for (k, v) in old["devices"][0]["identity"].as_object().unwrap() {
        assert_eq!(new["devices"][0]["identity"][k], *v, "identity.{k}");
    }
    assert_eq!(new["devices"][1], old["devices"][1]);
    // The pack type's unknown field is untouched, A1 is as it was, A2 has its type.
    assert_eq!(new["pack_types"], old["pack_types"]);
    assert_eq!(new["packs"][0], old["packs"][0]);
    assert_eq!(new["packs"][1]["pack_type"], "1S 300");
    assert_eq!(new["crashes"][0]["note"], "tree, again");
    assert_eq!(new["crashes"][0]["broke"], "prop");
    assert_eq!(new["crashes"][0]["parts"], old["crashes"][0]["parts"]);
}

#[test]
fn a_version_1_flights_cache_is_rebuilt_not_trusted() {
    let l = lab("0.7.0");
    let logs = l.root().join("logs");
    synth::write_known(&logs).unwrap();
    let cache = l.gear().join("flights.json");
    let before: Value = serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
    assert_eq!(before["version"], 1);

    let v = l
        .core
        .gear_flights(&FlightFilter {
            logs: Some(logs.clone()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(v.flights.len(), synth::KNOWN.flights);
    let after: Value = serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
    assert_eq!(after["version"], flights::CACHE_VERSION);
    assert!(!std::fs::read_to_string(&cache)
        .unwrap()
        .contains("old-log.csv"));
}

#[test]
fn a_cache_that_does_not_parse_is_rebuilt() {
    let l = lab("0.7.0");
    let logs = l.root().join("logs");
    synth::write_known(&logs).unwrap();
    std::fs::write(l.gear().join("flights.json"), b"{not json").unwrap();
    let v = l
        .core
        .gear_flights(&FlightFilter {
            logs: Some(logs),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(v.flights.len(), synth::KNOWN.flights);
}

#[test]
fn a_0_9_0_gear_folder_loads() {
    let l = lab("0.9.0");
    let devices = l.core.gear_devices().unwrap();
    let radio = devices
        .iter()
        .find(|d| d.id == "radio-0123456789abcdef")
        .unwrap();
    let space = radio.last_space.as_ref().expect("last_space");
    assert_eq!(
        (space.free, space.total),
        (1_000_000_000, Some(2_000_000_000))
    );

    // All three triggers load, the after-apply one included, with the pin kept.
    let backups = l.core.gear_backups(&BackupFilter::default()).unwrap();
    assert_eq!(backups.len(), 3);
    let all = quadcam_lib::gear::backup::Snapshots::new(l.core.gear_store()).all();
    let triggers: Vec<Trigger> = all.iter().map(|b| b.trigger).collect();
    assert!(triggers.contains(&Trigger::AfterApply));
    assert!(all
        .iter()
        .any(|b| b.pinned && b.trigger == Trigger::BeforeApply));

    // Staged changes written before `reverts` existed still load, in their status.
    let changes = l
        .core
        .gear_changes(&ChangeFilter {
            history: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(changes.len(), 2, "{changes:?}");
    let osd = changes.iter().find(|c| c.id == "chg-0001").unwrap();
    assert_eq!(osd.status, ChangeStatus::Ready);
    assert_eq!(osd.edits.len(), 2);
    assert!(osd.reverts.is_none());
    let model = changes.iter().find(|c| c.id == "chg-0002").unwrap();
    assert_eq!(model.status, ChangeStatus::Verified);
    assert_eq!(model.history.len(), 3);

    // The sim calibration file and the voice block.
    let cal = l
        .core
        .gear_sim_calibration(&quadcam_lib::core::SimCalibrationParams {
            radio: Some("radio-0123456789abcdef".into()),
        })
        .unwrap();
    assert_eq!(cal.calibration.unwrap().calibration.mode, 2);
    let voice = l
        .core
        .gear_voice(&VoiceParams {
            radio: Some("radio-0123456789abcdef".into()),
            refresh_index: false,
        })
        .unwrap();
    let line = voice
        .lines
        .iter()
        .find(|x| x.path == "custom/pack_ready")
        .expect("the custom line");
    assert!(line.custom);
    assert_eq!(line.override_.as_ref().unwrap().kind, "text");

    // A 0.9.0 flights cache is current: a read leaves it as it is.
    let cache = l.gear().join("flights.json");
    let before = std::fs::read(&cache).unwrap();
    l.core.gear_flights(&FlightFilter::default()).unwrap();
    assert_eq!(std::fs::read(&cache).unwrap(), before);
}

#[test]
fn a_0_9_0_gear_folder_keeps_everything_through_writes() {
    let l = lab("0.9.0");
    let old = l.gear_json();
    l.core
        .gear_device_save(&DeviceSaveParams {
            id: "fc-fedcba9876543210".into(),
            name: Some("Whoop FC".into()),
            aircraft: Some("Whoop A".into()),
        })
        .unwrap();
    l.core
        .gear_crash_save(&CrashSaveParams {
            aircraft: Some("Whoop A".into()),
            broke: Some("motor".into()),
            day: Some("2026-10-04".parse().unwrap()),
            ..Default::default()
        })
        .unwrap();
    let new = l.gear_json();
    keeps(&old, &new, &["devices", "crashes"]);
    assert_eq!(new["voice"], old["voice"], "the voice block is whole");
    assert_eq!(new["voice"]["futureVoiceKey"], 1);
    assert_eq!(
        new["devices"][0], old["devices"][0],
        "the radio is untouched"
    );
    assert_eq!(new["devices"][1]["name"], "Whoop FC");
    assert_eq!(new["devices"][1]["aircraft"], "Whoop A");
    assert_eq!(new["crashes"].as_array().unwrap().len(), 2);
    // The staged changes and the calibrations are files of their own: untouched.
    for f in [
        "changes/chg-0001/change.json",
        "changes/chg-0002/change.json",
        "sim/calibrations.json",
    ] {
        let a = std::fs::read(fixture("0.9.0").join("gear").join(f)).unwrap();
        let b = std::fs::read(l.gear().join(f)).unwrap();
        assert_eq!(a, b, "{f} changed");
    }
}

#[test]
fn a_staged_change_survives_a_status_edit() {
    let l = lab("0.9.0");
    let c = l
        .core
        .gear_change_update(&quadcam_lib::core::ChangeUpdateParams {
            id: "chg-0001".into(),
            note: Some("check the goggles".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(c.edits.len(), 2);
    assert_eq!(c.note, "check the goggles");
    // It still loads next to the unchanged one.
    let all = l
        .core
        .gear_changes(&ChangeFilter {
            history: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(all.len(), 2);
}
