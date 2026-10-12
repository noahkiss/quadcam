//! The radio's Settings editor through the core (`radio.yml`, `Edit::Radio`): every setting
//! reads from the synthetic card and writes one line, unknown keys and every other line
//! round-trip untouched, values out of range refuse, edits join one "Radio settings" change
//! (the last word per setting wins, an edit the radio has drops out), and a staged change
//! plans and applies to the card with read-back. Every card is a temporary folder and
//! "diskutil" is a closure.

use quadcam_lib::core::{
    BackupParams, Core, Hooks, NoHooks, RadioSettingsEditParams, RadioSettingsParams,
};
use quadcam_lib::disk::{DiskInfo, Volume};
use quadcam_lib::gear::apply::{ApplyPlanParams, ApplyRequest};
use quadcam_lib::gear::bf::cli::Timing;
use quadcam_lib::gear::changes::ChangeFilter;
use quadcam_lib::gear::cues::{CueService, RecordedCues};
use quadcam_lib::gear::edgetx::card::{Card, RadioOp, RADIO_FILE};
use quadcam_lib::gear::edgetx::radio::{self, SettingKind};
use quadcam_lib::gear::edgetx::synth::{self, SynthCard};
use quadcam_lib::gear::edgetx::yaml::Doc;
use quadcam_lib::gear::events::Presence;
use quadcam_lib::gear::model::{ChangeStatus, Edit};
use quadcam_lib::gear::serial::FakePorts;
use quadcam_lib::gear::Env;
use quadcam_lib::photos::Recorder;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const DISK: &str = "disk42";

fn volume(root: &std::path::Path, uuid: &str) -> Volume {
    Volume {
        mount: root.to_path_buf(),
        info: DiskInfo {
            volume_uuid: Some(uuid.into()),
            parent_whole_disk: DISK.into(),
            bus_protocol: Some("USB".into()),
            removable: true,
            ..Default::default()
        },
        is_card: false,
        source: None,
        is_radio: true,
        warnings: vec![],
    }
}

#[allow(dead_code)]
struct Bench {
    core: Arc<Core>,
    cues: Arc<RecordedCues>,
    root: PathBuf,
    id: String,
    /// The card is mounted now.
    mounted: Arc<AtomicBool>,
    /// The card is still plugged in (its disk node shows).
    present: Arc<AtomicBool>,
    /// What the fake `diskutil` was asked: "mount disk42", "unmount disk42".
    log: Arc<Mutex<Vec<String>>>,
    unmount_fails: Arc<AtomicBool>,
    dir: tempfile::TempDir,
}

struct Opts {
    card: SynthCard,
    fail_readback: Option<String>,
    hooks: Arc<dyn Hooks>,
    uuid: &'static str,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            card: SynthCard::default(),
            fail_readback: None,
            hooks: Arc::new(NoHooks),
            uuid: "11111111-2222-3333-4444-555555555555",
        }
    }
}

/// A core with one synthetic radio card mounted, backed up once so the radio is a saved
/// device.
fn bench(o: Opts) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("CARD");
    synth::write_card(&root, &o.card).unwrap();
    let mounted = Arc::new(AtomicBool::new(true));
    let log: Arc<Mutex<Vec<String>>> = Arc::default();
    let unmount_fails = Arc::new(AtomicBool::new(false));
    let cues = Arc::new(RecordedCues::default());
    let vol = volume(&root, o.uuid);
    let mut env = Env::fake(vec![], Arc::new(FakePorts::new(vec![])));
    env.cues = Arc::new(CueService::inline(cues.clone()));
    let m = mounted.clone();
    env.volumes = Arc::new(move || {
        if m.load(Ordering::SeqCst) {
            vec![vol.clone()]
        } else {
            vec![]
        }
    });
    let present = Arc::new(AtomicBool::new(true));
    let p = present.clone();
    env.presence = Arc::new(move || {
        if p.load(Ordering::SeqCst) {
            vec![Presence::Disk { disk: DISK.into() }]
        } else {
            vec![]
        }
    });
    let (m, l, f) = (mounted.clone(), log.clone(), unmount_fails.clone());
    env.unmount = Arc::new(move |d| {
        l.lock().unwrap().push(format!("unmount {d}"));
        if f.load(Ordering::SeqCst) {
            anyhow::bail!("Unmount of {d} failed: at least one volume could not be unmounted")
        }
        m.store(false, Ordering::SeqCst);
        Ok(())
    });
    let (m, l) = (mounted.clone(), log.clone());
    env.mount = Arc::new(move |d| {
        l.lock().unwrap().push(format!("mount {d}"));
        m.store(true, Ordering::SeqCst);
        Ok(())
    });
    env.fail_readback = o.fail_readback;
    // Cues play every time here: the debounce would hide a second "safe to unplug".
    std::fs::create_dir_all(dir.path().join("support")).unwrap();
    std::fs::write(
        dir.path().join("support/settings.json"),
        r#"{"gearCues": {"debounce_s": 0}}"#,
    )
    .unwrap();
    let core = Arc::new(
        Core::new(
            dir.path().join("cache"),
            None,
            o.hooks,
            Arc::new(Recorder::default()),
        )
        .with_settings(dir.path().join("support/settings.json"))
        .with_gear_env(env)
        .with_fc_timing(Timing::fast()),
    );
    let id = core
        .gear_connected()
        .unwrap()
        .into_iter()
        .find_map(|c| c.id)
        .expect("the card has an id");
    core.gear_backup(&BackupParams {
        device: Some(id.clone()),
        ..Default::default()
    })
    .unwrap();
    // The backup unmounted the card; the tests start with it mounted, as a reader shows it.
    mounted.store(true, Ordering::SeqCst);
    log.lock().unwrap().clear();
    cues.played.lock().unwrap().clear();
    Bench {
        core,
        cues,
        root,
        id,
        mounted,
        present,
        log,
        unmount_fails,
        dir,
    }
}

fn settings(b: &Bench) -> quadcam_lib::core::RadioSettingsDetail {
    b.core
        .gear_radio_settings(&RadioSettingsParams {
            device: b.id.clone(),
            staged: true,
        })
        .unwrap()
}

fn set(key: &str, value: &str) -> RadioOp {
    RadioOp::SetScalar {
        key: key.into(),
        value: value.into(),
    }
}

fn edit(b: &Bench, ops: Vec<RadioOp>) -> anyhow::Result<quadcam_lib::gear::model::StagedChange> {
    b.core.gear_radio_settings_edit(&RadioSettingsEditParams {
        device: b.id.clone(),
        ops,
        editor: None,
    })
}

fn staged(b: &Bench) -> Vec<quadcam_lib::gear::model::StagedChange> {
    b.core.gear_changes(&ChangeFilter::default()).unwrap()
}

fn read(b: &Bench, rel: &str) -> String {
    String::from_utf8_lossy(&std::fs::read(b.root.join(rel)).unwrap()).to_string()
}

/// A value of the setting other than `now`, inside its range.
fn other_value(kind: &SettingKind, now: &str) -> String {
    match kind {
        SettingKind::Choice { choices } => choices
            .iter()
            .find(|c| c.value != now)
            .unwrap()
            .value
            .clone(),
        SettingKind::Number { min, max, .. } => {
            let n: i32 = now.parse().unwrap();
            if n < *max { n + 1 } else { *min }.to_string()
        }
        SettingKind::Toggle { .. } => if now == "1" { "0" } else { "1" }.into(),
        SettingKind::Text { .. } => "QC-01".into(),
    }
}

/// The synthetic radio.yml with a key QuadCam does not know, a block it does not know, and
/// one known key taken out.
fn odd_radio_yml() -> String {
    synth::radio_yml("pocket", "2.12.4", 1, true)
        .replace("USBMode: 0\r\n", "")
        .replace(
            "modelQuickSelect: 1\r\n",
            "modelQuickSelect: 1\r\nfutureKey: 7\r\nfutureBlock: \r\n   a: 1\r\n",
        )
}

#[test]
fn every_setting_reads_from_the_card_and_from_the_backup() {
    let b = bench(Opts::default());
    let d = settings(&b);
    assert_eq!(d.source, "card");
    assert_eq!(d.board.as_deref(), Some("pocket"));
    assert_eq!(d.version.as_deref(), Some("2.12.4"));
    assert_eq!(d.view.selected_model.as_deref(), Some("model01.yml"));
    assert!(d
        .models
        .iter()
        .any(|m| m.file == "model01.yml" && m.selected));
    let missing: Vec<&str> = d
        .view
        .settings
        .iter()
        .filter(|s| s.value.is_none())
        .map(|s| s.key.as_str())
        .collect();
    assert!(
        missing.is_empty(),
        "the synthetic card has every key: {missing:?}"
    );
    let get = |k: &str| {
        d.view
            .settings
            .iter()
            .find(|s| s.key == k)
            .and_then(|s| s.value.clone())
            .unwrap()
    };
    assert_eq!(get("vBatWarn"), "66");
    assert_eq!(get("templateSetup"), "21");
    assert_eq!(
        get("ownerRegistrationID"),
        "    test",
        "unquoted, padding kept"
    );
    assert_eq!(get("backlightMode"), "backlight_mode_keys");
    let text = quadcam_lib::core::radio_settings::render_text(&d);
    assert!(text.contains("Battery low: 6.6 V (vBatWarn: 66)"), "{text}");
    assert!(
        text.contains("Default channel order: AETR (templateSetup: 21)"),
        "{text}"
    );
    assert!(
        text.contains("Brightness: 100 (backlightBright: 0)"),
        "{text}"
    );
    assert!(text.contains("RSSI shutdown alarm: on"), "{text}");
    assert!(
        text.contains("Selected model: BRAVO 2 (model01.yml)"),
        "{text}"
    );
    // Unplugged: the same settings from the latest backup.
    b.mounted.store(false, Ordering::SeqCst);
    let d = settings(&b);
    assert!(d.source.starts_with("backup "), "{}", d.source);
    assert_eq!(d.view.settings.len(), radio::keys().count());
}

#[test]
fn each_setting_writes_one_line_and_reads_back() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("CARD");
    synth::write_card(&root, &SynthCard::default()).unwrap();
    let card = Card::open(&root).unwrap();
    let before = std::fs::read(root.join(RADIO_FILE)).unwrap();
    let view = radio::view(&Doc::parse("radio.yml", &before).unwrap()).unwrap();
    for s in &view.settings {
        let now = s.value.clone().unwrap();
        let new = other_value(&s.kind, &now);
        let plan = card
            .plan(
                &[Edit::Radio {
                    ops: vec![set(&s.key, &new)],
                }],
                None,
            )
            .unwrap();
        assert!(plan.ready(), "{}: {:?}", s.key, plan.checks);
        let f = plan.files.iter().find(|f| f.path == RADIO_FILE).unwrap();
        let after = f.after.clone().unwrap();
        let (a, b): (Vec<&str>, Vec<String>) = (
            std::str::from_utf8(&before)
                .unwrap()
                .split("\r\n")
                .collect(),
            String::from_utf8_lossy(&after)
                .split("\r\n")
                .map(str::to_string)
                .collect(),
        );
        assert_eq!(a.len(), b.len(), "{}: no line added or removed", s.key);
        let changed: Vec<usize> = (0..a.len()).filter(|&i| a[i] != b[i]).collect();
        // A file with a checksum gets checksum 0 as well.
        let own: Vec<&usize> = changed
            .iter()
            .filter(|&&i| !a[i].starts_with("checksum:"))
            .collect();
        assert_eq!(own.len(), 1, "{}: {changed:?}", s.key);
        assert!(
            b[*own[0]].starts_with(&format!("{}: ", s.key)),
            "{}",
            b[*own[0]]
        );
        let back = radio::view(&Doc::parse("radio.yml", &after).unwrap()).unwrap();
        let v = back.settings.iter().find(|x| x.key == s.key).unwrap();
        assert_eq!(v.value.as_deref(), Some(new.as_str()), "{}", s.key);
    }
}

#[test]
fn values_out_of_range_refuse_with_the_range() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("CARD");
    synth::write_card(&root, &SynthCard::default()).unwrap();
    let card = Card::open(&root).unwrap();
    let before = std::fs::read(root.join(RADIO_FILE)).unwrap();
    let view = radio::view(&Doc::parse("radio.yml", &before).unwrap()).unwrap();
    let refuses = |key: &str, value: &str| {
        let plan = card
            .plan(
                &[Edit::Radio {
                    ops: vec![set(key, value)],
                }],
                None,
            )
            .unwrap();
        assert!(!plan.ready(), "{key}={value} should refuse");
        plan.checks
            .iter()
            .find(|c| !c.ok)
            .and_then(|c| c.refusal.clone())
            .map(|r| r.reason)
            .unwrap_or_default()
    };
    for s in &view.settings {
        match &s.kind {
            SettingKind::Number { min, max, .. } => {
                let r = refuses(&s.key, &(min - 1).to_string());
                assert!(r.contains(&format!("from {min} to {max}")), "{r}");
                refuses(&s.key, &(max + 1).to_string());
                refuses(&s.key, "x");
            }
            SettingKind::Choice { .. } => {
                let r = refuses(&s.key, "nope");
                assert!(r.contains("is one of") || r.contains("whole number"), "{r}");
            }
            SettingKind::Toggle { .. } => {
                refuses(&s.key, "2");
            }
            SettingKind::Text { max_len } => {
                refuses(&s.key, &"a".repeat(*max_len as usize + 1));
                refuses(&s.key, "a\\b");
            }
        }
    }
}

#[test]
fn unknown_keys_and_a_missing_key_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("CARD");
    synth::write_card(&root, &SynthCard::default()).unwrap();
    std::fs::write(root.join(RADIO_FILE), odd_radio_yml()).unwrap();
    let card = Card::open(&root).unwrap();
    let doc = Doc::parse("radio.yml", odd_radio_yml().as_bytes()).unwrap();
    let view = radio::view(&doc).unwrap();
    let usb = view.settings.iter().find(|s| s.key == "USBMode").unwrap();
    assert_eq!(usb.value, None, "a key the file lacks shows as absent");
    // QuadCam never adds the line.
    let plan = card
        .plan(
            &[Edit::Radio {
                ops: vec![set("USBMode", "2")],
            }],
            None,
        )
        .unwrap();
    assert!(!plan.ready());
    let plan = card
        .plan(
            &[Edit::Radio {
                ops: vec![set("beepVolume", "4"), set("futureKey", "8")],
            }],
            None,
        )
        .unwrap();
    assert!(plan.ready(), "{:?}", plan.checks);
    let after = String::from_utf8(plan.files[0].after.clone().unwrap()).unwrap();
    assert!(after.contains("beepVolume: 4\r\n"));
    assert!(
        after.contains("futureKey: 8\r\n"),
        "a key outside the table is set as given"
    );
    assert!(after.contains("futureBlock: \r\n   a: 1\r\n"));
    assert_eq!(
        after
            .replace("beepVolume: 4", "beepVolume: 2")
            .replace("futureKey: 8", "futureKey: 7")
            .replace("checksum: 0", "checksum: 12345"),
        odd_radio_yml()
    );
}

#[test]
fn edits_join_one_change_and_the_last_word_per_setting_wins() {
    let b = bench(Opts::default());
    let c1 = edit(&b, vec![set("beepVolume", "3")]).unwrap();
    assert_eq!(c1.title, "Radio settings");
    assert_eq!(c1.status, ChangeStatus::Ready);
    let c2 = edit(&b, vec![set("beepVolume", "4"), set("vBatWarn", "70")]).unwrap();
    assert_eq!(c2.id, c1.id);
    assert_eq!(staged(&b).len(), 1);
    let Edit::Radio { ops } = &c2.edits[0] else {
        panic!("{:?}", c2.edits)
    };
    assert_eq!(ops, &vec![set("beepVolume", "4"), set("vBatWarn", "70")]);
    // The editor's read shows the staged values.
    let d = settings(&b);
    assert_eq!(d.staged, 2);
    let v = |k: &str| {
        d.view
            .settings
            .iter()
            .find(|s| s.key == k)
            .and_then(|s| s.value.clone())
            .unwrap()
    };
    assert_eq!(v("beepVolume"), "4");
    assert_eq!(v("vBatWarn"), "70");
    // Back to what the radio has: that op drops out; with none left the change goes.
    let c3 = edit(&b, vec![set("vBatWarn", "66")]).unwrap();
    let Edit::Radio { ops } = &c3.edits[0] else {
        panic!()
    };
    assert_eq!(ops, &vec![set("beepVolume", "4")]);
    let c4 = edit(&b, vec![set("beepVolume", "2")]).unwrap();
    assert_eq!(c4.status, ChangeStatus::Discarded);
    assert!(staged(&b)
        .iter()
        .all(|c| c.status == ChangeStatus::Discarded));
    let e = edit(&b, vec![set("beepVolume", "2")]).unwrap_err();
    assert!(format!("{e:#}").contains("Nothing changes"), "{e:#}");
}

#[test]
fn a_refused_edit_stages_nothing() {
    let b = bench(Opts::default());
    for (op, why) in [
        (set("beepVolume", "9"), "from 0 to 4"),
        (set("currModel", "0"), "use select_model"),
        (set("semver", "2.11.0"), "not set this way"),
        (
            RadioOp::SelectModel {
                file: "model07.yml".into(),
            },
            "no MODELS/model07.yml",
        ),
    ] {
        let e = edit(&b, vec![op]).unwrap_err();
        assert!(format!("{e:#}").contains(why), "{e:#}");
    }
    assert!(staged(&b).is_empty());
    let e = edit(&b, vec![]).unwrap_err();
    assert!(format!("{e:#}").contains("at least one"));
}

fn plan_and_apply(
    b: &Bench,
    c: &quadcam_lib::gear::model::StagedChange,
) -> (
    quadcam_lib::gear::model::ApplyPlan,
    quadcam_lib::gear::apply::ApplyReport,
) {
    let p = b
        .core
        .gear_apply_plan(&ApplyPlanParams {
            id: c.id.clone(),
            port: None,
        })
        .unwrap();
    assert!(p.checks.iter().all(|k| k.ok), "{:?}", p.checks);
    let r = b
        .core
        .gear_apply(&ApplyRequest {
            id: c.id.clone(),
            digest: p.digest.clone(),
            confirm: true,
            port: None,
        })
        .unwrap();
    (p, r)
}

#[test]
fn a_staged_change_plans_and_applies_to_the_card_with_read_back() {
    let b = bench(Opts::default());
    let models = read(&b, "MODELS/model01.yml");
    let c = edit(
        &b,
        vec![
            set("backlightMode", "backlight_mode_on"),
            set("ownerRegistrationID", "QC-01"),
            set("stickMode", "0"),
            RadioOp::SelectModel {
                file: "model00.yml".into(),
            },
        ],
    )
    .unwrap();
    let (p, r) = plan_and_apply(&b, &c);
    assert!(
        p.warnings.iter().any(|w| w.contains("stickMode")),
        "{:?}",
        p.warnings
    );
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    let y = read(&b, RADIO_FILE);
    assert!(y.contains("backlightMode: backlight_mode_on\r\n"));
    assert!(y.contains("ownerRegistrationID: \"QC-01\"\r\n"));
    assert!(y.contains("stickMode: 0\r\n"));
    assert!(y.contains("currModel: 0\r\n"));
    assert!(
        y.contains("bluetoothName: \"TEST-\\x80\"\r\n"),
        "other lines untouched"
    );
    assert_eq!(read(&b, "MODELS/model01.yml"), models);
    // The card was mounted for the job and unmounted after.
    assert!(b
        .log
        .lock()
        .unwrap()
        .iter()
        .any(|l| l == &format!("unmount {DISK}")));
    // The next read from the card shows the applied values and no staged edits.
    b.mounted.store(true, Ordering::SeqCst);
    let d = settings(&b);
    assert_eq!(d.staged, 0);
    assert_eq!(d.view.selected_model.as_deref(), Some("model00.yml"));
}

#[test]
fn the_row_and_the_mcp_actions() {
    use quadcam_lib::mcp::{LocalBackend, Server};
    use serde_json::json;
    let b = bench(Opts::default());
    let v = b
        .core
        .dispatch(
            "gear_radio_settings_edit",
            json!({"device": b.id, "ops": [{"op": "set_scalar", "key": "speakerVolume", "value": "20"}]}),
        )
        .unwrap();
    assert_eq!(v["title"], "Radio settings");
    assert_eq!(v["edits"][0]["kind"], "radio");
    let mut s = Server::new(LocalBackend(b.core.clone()));
    let r = s.call_tool(
        "quadcam_gear_edit",
        json!({"action": "radio_settings_edit", "device": b.id,
               "ops": [{"op": "set_scalar", "key": "hapticMode", "value": "mode_nokeys"}]}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with("Staged."));
    assert_eq!(staged(&b).len(), 1);
    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "radio_settings", "device": b.id}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("from the card"), "{text}");
    assert!(text.contains("Volume: 20 (speakerVolume: 20)"), "{text}");
    assert!(
        text.contains("Haptic mode: No keys (hapticMode: mode_nokeys)"),
        "{text}"
    );
    assert!(text.contains("Shows 2 staged edits"), "{text}");
    let r = s.call_tool(
        "quadcam_gear_edit",
        json!({"action": "radio_settings_edit", "device": b.id,
               "ops": [{"op": "set_scalar", "key": "vBatWarn", "value": "200"}]}),
    );
    assert_eq!(r["isError"], true);
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("from 30 to 120"));
    let r = s.call_tool("quadcam_gear", json!({"action": "radio_settings"}));
    assert_eq!(r["isError"], true);
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("device is required"));
}
