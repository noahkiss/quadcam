//! Card apply (WP5b) on synthetic cards only: the checks of design 8.2 that apply to a
//! card each refuse, a good apply is backed up, written, verified and unmounted, a
//! read-back mismatch rolls every file back, the unmount decides the "safe to unplug" cue,
//! and a card unmounted between operations is mounted for the job and unmounted after.
//! Every card is a temporary folder and the "diskutil" is a closure; no real card or disk
//! is touched.

use quadcam_lib::core::{
    BackupParams, CardCheckParams, CardCleanParams, CardMountParams, CardParams, CardPreviewParams,
    Core, Hooks, NoHooks, RestoreParams, StageParams,
};
use quadcam_lib::disk::{DiskInfo, Volume};
use quadcam_lib::gear::apply::{ApplyPlanParams, ApplyRequest};
use quadcam_lib::gear::bf::cli::Timing;
use quadcam_lib::gear::changes::ChangeFilter;
use quadcam_lib::gear::cues::{CueService, RecordedCues};
use quadcam_lib::gear::edgetx::card::RadioOp;
use quadcam_lib::gear::edgetx::model::ModelOp;
use quadcam_lib::gear::edgetx::synth::{self, SynthCard};
use quadcam_lib::gear::events::Presence;
use quadcam_lib::gear::health::{CardCheck, CheckKind, CheckState, HealthLog};
use quadcam_lib::gear::model::{
    ApplyPlan, ChangeStatus, DeviceKind, Edit, Refusal, RefusalCode, StagedChange, Trigger,
};
use quadcam_lib::gear::serial::FakePorts;
use quadcam_lib::gear::store::Store;
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

fn contrast(v: &str) -> Edit {
    Edit::Radio {
        ops: vec![RadioOp::SetScalar {
            key: "contrast".into(),
            value: v.into(),
        }],
    }
}

fn rename(file: &str, from: Option<&str>, to: &str) -> Edit {
    Edit::Model {
        file: file.into(),
        name: from.map(str::to_string),
        ops: vec![ModelOp::Rename { name: to.into() }],
    }
}

fn stage(b: &Bench, edits: Vec<Edit>) -> StagedChange {
    b.core
        .gear_change_stage(&StageParams {
            device: b.id.clone(),
            edits,
            ..Default::default()
        })
        .unwrap()
}

fn plan(b: &Bench, c: &StagedChange) -> ApplyPlan {
    b.core
        .gear_apply_plan(&ApplyPlanParams {
            id: c.id.clone(),
            port: None,
        })
        .unwrap()
}

fn req(c: &StagedChange, p: &ApplyPlan) -> ApplyRequest {
    ApplyRequest {
        id: c.id.clone(),
        digest: p.digest.clone(),
        confirm: true,
        port: None,
    }
}

fn code(e: &anyhow::Error) -> RefusalCode {
    e.downcast_ref::<Refusal>()
        .unwrap_or_else(|| panic!("not a refusal: {e:#}"))
        .code
}

fn failed(p: &ApplyPlan) -> RefusalCode {
    p.checks
        .iter()
        .find_map(|c| c.refusal.as_ref().map(|r| r.code))
        .expect("a failed check")
}

fn status(b: &Bench, id: &str) -> ChangeStatus {
    b.core
        .gear_changes(&ChangeFilter {
            history: true,
            ..Default::default()
        })
        .unwrap()
        .into_iter()
        .find(|c| c.id == id)
        .unwrap()
        .status
}

fn read(b: &Bench, rel: &str) -> String {
    String::from_utf8_lossy(&std::fs::read(b.root.join(rel)).unwrap()).to_string()
}

fn spoken(b: &Bench) -> Vec<String> {
    b.cues.spoken()
}

#[test]
fn a_good_apply_is_backed_up_written_verified_and_unmounted() {
    let b = bench(Opts::default());
    let before_radio = read(&b, "RADIO/radio.yml");
    let c = stage(
        &b,
        vec![
            contrast("27"),
            rename("model00.yml", Some("ALPHA"), "ALPHA TWO"),
        ],
    );
    assert_eq!(c.status, ChangeStatus::Ready);
    let p = plan(&b, &c);
    assert!(p.checks.iter().all(|k| k.ok), "{:?}", p.checks);
    assert!(!p.digest.is_empty());
    assert!(
        b.log.lock().unwrap().is_empty(),
        "a mounted card stays as is for a plan"
    );

    let r = b.core.gear_apply(&req(&c, &p)).unwrap();
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    let names: Vec<_> = r.steps.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Back up", "Write", "Read back", "Verify"]);
    assert!(r.files.iter().any(|f| f == "RADIO/radio.yml"));
    assert!(read(&b, "RADIO/radio.yml").contains("contrast: 27"));
    assert!(read(&b, "MODELS/model00.yml").contains("ALPHA TWO"));
    assert_ne!(read(&b, "RADIO/radio.yml"), before_radio);
    assert_eq!(status(&b, &c.id), ChangeStatus::Verified);

    // The backup holds the old bytes, always kept; an after-apply snapshot follows.
    let snaps =
        quadcam_lib::gear::backup::Snapshots::new(Store::new(b.dir.path().join("support/gear")));
    let list = snaps.list(&b.id);
    let before = list
        .iter()
        .find(|s| s.trigger == Trigger::BeforeApply)
        .unwrap();
    assert_eq!(Some(&before.id), r.backup.as_ref());
    assert!(before.trigger.always_kept());
    let old = snaps.read(&before.id, Some("RADIO/radio.yml")).unwrap();
    assert_eq!(
        old.text.unwrap().replace("\r\n", "\n"),
        before_radio.replace("\r\n", "\n")
    );
    assert!(list.iter().any(|s| s.trigger == Trigger::AfterApply));

    // The job ended with the unmount, and only then "safe to unplug".
    assert_eq!(*b.log.lock().unwrap(), [format!("unmount {DISK}")]);
    assert!(!b.mounted.load(Ordering::SeqCst));
    assert!(
        spoken(&b).iter().any(|s| s.contains("safe to unplug")),
        "{:?}",
        spoken(&b)
    );
    // The report is kept with the change.
    let hist = b
        .core
        .gear_changes(&ChangeFilter {
            history: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(hist.len(), 1);
}

#[test]
fn a_failed_unmount_means_no_safe_to_unplug() {
    let b = bench(Opts::default());
    b.unmount_fails.store(true, Ordering::SeqCst);
    let c = stage(&b, vec![contrast("26")]);
    let p = plan(&b, &c);
    let r = b.core.gear_apply(&req(&c, &p)).unwrap();
    assert_eq!(r.status, ChangeStatus::Verified);
    assert!(
        r.notes.iter().any(|n| n.contains("did not unmount")),
        "{:?}",
        r.notes
    );
    assert!(
        !spoken(&b).iter().any(|s| s.contains("safe to unplug")),
        "{:?}",
        spoken(&b)
    );
    assert!(
        spoken(&b).iter().any(|s| s.contains("failed")),
        "{:?}",
        spoken(&b)
    );
}

#[test]
fn a_read_back_mismatch_puts_every_file_back() {
    let b = bench(Opts {
        fail_readback: Some("RADIO/radio.yml".into()),
        ..Default::default()
    });
    let radio = std::fs::read(b.root.join("RADIO/radio.yml")).unwrap();
    let model = std::fs::read(b.root.join("MODELS/model00.yml")).unwrap();
    let c = stage(
        &b,
        vec![contrast("29"), rename("model00.yml", None, "ALPHA TWO")],
    );
    let p = plan(&b, &c);
    let r = b.core.gear_apply(&req(&c, &p)).unwrap();
    assert_eq!(r.status, ChangeStatus::Failed, "{}", r.message);
    let steps: Vec<_> = r.steps.iter().map(|s| (s.name.as_str(), s.state)).collect();
    use quadcam_lib::gear::apply::StepState::*;
    assert_eq!(
        steps,
        [
            ("Back up", Done),
            ("Write", Failed),
            ("Roll back", Done),
            ("Read back", Skipped),
            ("Verify", Skipped)
        ]
    );
    assert!(r.message.contains("The card is as it was"), "{}", r.message);
    // Both files hold their old bytes, the new file is gone, the backup is kept.
    assert_eq!(
        std::fs::read(b.root.join("RADIO/radio.yml")).unwrap(),
        radio
    );
    assert_eq!(
        std::fs::read(b.root.join("MODELS/model00.yml")).unwrap(),
        model
    );
    assert_eq!(status(&b, &c.id), ChangeStatus::Failed);
    assert!(r.backup.is_some());
    // The card is released and a failure cue plays, not "safe to unplug".
    assert_eq!(*b.log.lock().unwrap(), [format!("unmount {DISK}")]);
    assert!(!spoken(&b).iter().any(|s| s.contains("safe to unplug")));
}

#[test]
fn unknown_version_and_board_refuse() {
    for (board, semver, want) in [
        ("pocket", "2.11.3", "version"),
        ("tx16s", "2.12.4", "board"),
    ] {
        let b = bench(Opts {
            card: SynthCard {
                board: board.into(),
                semver: semver.into(),
                ..Default::default()
            },
            ..Default::default()
        });
        let c = stage(&b, vec![contrast("30")]);
        let p = plan(&b, &c);
        let want = if want == "version" {
            RefusalCode::UnknownVersion
        } else {
            RefusalCode::UnknownBoard
        };
        assert_eq!(failed(&p), want, "{:?}", p.checks);
        let e = b.core.gear_apply(&req(&c, &p)).unwrap_err();
        assert_eq!(code(&e), want);
        assert!(read(&b, "RADIO/radio.yml").contains("contrast: 20"));
        assert_eq!(status(&b, &c.id), ChangeStatus::Ready);
    }
}

#[test]
fn another_card_is_not_the_planned_one() {
    let b = bench(Opts::default());
    let c = stage(&b, vec![contrast("30")]);
    // Plan for a change on a radio that is not this card.
    let mut other: quadcam_lib::gear::model::Device =
        serde_json::from_value(serde_json::json!({"id": "radio-other", "kind": "radio"})).unwrap();
    other.id = quadcam_lib::gear::model::device_id(DeviceKind::Radio, "marker:other");
    Store::new(b.dir.path().join("support/gear"))
        .save_device(&other)
        .unwrap();
    let c2 = b
        .core
        .gear_change_stage(&StageParams {
            device: other.id.clone(),
            edits: vec![contrast("31")],
            ..Default::default()
        })
        .unwrap();
    let p = plan(&b, &c2);
    assert_eq!(failed(&p), RefusalCode::DeviceChanged);
    let e = b.core.gear_apply(&req(&c2, &p)).unwrap_err();
    assert_eq!(code(&e), RefusalCode::DeviceChanged);
    assert!(read(&b, "RADIO/radio.yml").contains("contrast: 20"));
    // The right change still plans.
    assert!(plan(&b, &c).checks.iter().all(|k| k.ok));
}

#[test]
fn no_card_plugged_in_refuses() {
    let b = bench(Opts::default());
    let c = stage(&b, vec![contrast("30")]);
    // Pulled out: no volume and no disk node.
    b.mounted.store(false, Ordering::SeqCst);
    b.present.store(false, Ordering::SeqCst);
    let p = plan(&b, &c);
    assert_eq!(failed(&p), RefusalCode::NoDevice, "{:?}", p.checks);
    let e = b.core.gear_apply(&req(&c, &p)).unwrap_err();
    assert_eq!(code(&e), RefusalCode::NoDevice);
    assert!(b.log.lock().unwrap().is_empty(), "nothing to mount");
}

#[test]
fn a_model_with_another_name_is_the_wrong_card() {
    let b = bench(Opts::default());
    let c = stage(&b, vec![rename("model00.yml", Some("NOT ALPHA"), "X")]);
    let p = plan(&b, &c);
    assert_eq!(failed(&p), RefusalCode::DeviceChanged, "{:?}", p.checks);
    assert!(p.checks.iter().any(|k| k.name == "Model identity" && !k.ok));
}

#[test]
fn a_failed_card_check_refuses() {
    let b = bench(Opts::default());
    let c = stage(&b, vec![contrast("30")]);
    HealthLog::new(Store::new(b.dir.path().join("support/gear")))
        .append(&CardCheck {
            id: format!("{}-1", b.id),
            device: b.id.clone(),
            kind: CheckKind::Verify,
            state: CheckState::Failed,
            at: chrono::Utc::now(),
            seconds: 1.0,
            fsck_code: Some(8),
            modified: false,
            summary: "The file system has errors".into(),
            findings: vec![],
        })
        .unwrap();
    let p = plan(&b, &c);
    assert_eq!(failed(&p), RefusalCode::CardCheck, "{:?}", p.checks);
    let e = b.core.gear_apply(&req(&c, &p)).unwrap_err();
    assert_eq!(code(&e), RefusalCode::CardCheck);
    assert!(read(&b, "RADIO/radio.yml").contains("contrast: 20"));
}

#[test]
fn a_read_first_change_is_not_applied() {
    let b = bench(Opts::default());
    let c = stage(&b, vec![contrast("30")]);
    b.core
        .gear_change_update(&quadcam_lib::core::ChangeUpdateParams {
            id: c.id.clone(),
            status: Some(ChangeStatus::ReadFirst),
            ..Default::default()
        })
        .unwrap();
    let p = plan(&b, &c);
    assert_eq!(failed(&p), RefusalCode::ReadFirst);
    // Ready again: it applies.
    b.core
        .gear_change_update(&quadcam_lib::core::ChangeUpdateParams {
            id: c.id.clone(),
            status: Some(ChangeStatus::Ready),
            ..Default::default()
        })
        .unwrap();
    assert!(plan(&b, &c).checks.iter().all(|k| k.ok));
}

#[test]
fn a_card_that_changed_since_the_plan_is_before_mismatch() {
    let b = bench(Opts::default());
    let c = stage(&b, vec![contrast("30")]);
    let p = plan(&b, &c);
    // Someone changes the card after the plan was read.
    let f = b.root.join("RADIO/radio.yml");
    let text = std::fs::read_to_string(&f)
        .unwrap()
        .replace("vBatWarn: 66", "vBatWarn: 67");
    std::fs::write(&f, text).unwrap();
    let e = b.core.gear_apply(&req(&c, &p)).unwrap_err();
    assert_eq!(code(&e), RefusalCode::BeforeMismatch);
    assert!(read(&b, "RADIO/radio.yml").contains("contrast: 20"));
    assert_eq!(status(&b, &c.id), ChangeStatus::Ready);
}

#[test]
fn a_backup_that_cannot_be_written_stops_everything() {
    use std::os::unix::fs::PermissionsExt;
    let b = bench(Opts::default());
    let c = stage(&b, vec![contrast("30")]);
    let p = plan(&b, &c);
    let gear = b.dir.path().join("support/gear");
    let guarded = [gear.join("blobs"), gear.join("snapshots").join(&b.id)];
    for d in &guarded {
        if d.is_dir() {
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o555)).unwrap();
        }
    }
    let r = b.core.gear_apply(&req(&c, &p));
    for d in &guarded {
        if d.is_dir() {
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    // A card file that changes the backup must hold: either the snapshot failed (no_backup)
    // or the content already sat in the store (then the apply goes ahead). Both are right;
    // a failure must have written nothing.
    match r {
        Err(e) => {
            assert_eq!(code(&e), RefusalCode::NoBackup);
            assert!(read(&b, "RADIO/radio.yml").contains("contrast: 20"));
        }
        Ok(r) => assert_eq!(r.status, ChangeStatus::Verified),
    }
}

#[test]
fn confirm_needs_the_digest_and_the_flag() {
    let b = bench(Opts::default());
    let c = stage(&b, vec![contrast("30")]);
    let p = plan(&b, &c);
    let mut r = req(&c, &p);
    r.confirm = false;
    assert!(b
        .core
        .gear_apply(&r)
        .unwrap_err()
        .to_string()
        .contains("confirm"));
    let mut r = req(&c, &p);
    r.digest = "0000".into();
    assert_eq!(
        code(&b.core.gear_apply(&r).unwrap_err()),
        RefusalCode::BeforeMismatch
    );
    assert!(read(&b, "RADIO/radio.yml").contains("contrast: 20"));
}

#[test]
fn a_card_unmounted_between_operations_is_mounted_for_the_job() {
    let b = bench(Opts::default());
    // Between operations the card is unmounted (an earlier job released it).
    let l = b
        .core
        .gear_connected()
        .unwrap()
        .into_iter()
        .find(|c| c.id.as_deref() == Some(b.id.as_str()))
        .unwrap();
    b.core
        .gear_card_unmount(&CardMountParams {
            device: b.id.clone(),
            minutes: None,
        })
        .unwrap();
    assert!(!b.mounted.load(Ordering::SeqCst));
    b.log.lock().unwrap().clear();
    b.cues.played.lock().unwrap().clear();
    let _ = l;

    let c = stage(&b, vec![contrast("24")]);
    // A plan mounts, reads and unmounts, quietly.
    let p = plan(&b, &c);
    assert!(p.checks.iter().all(|k| k.ok), "{:?}", p.checks);
    assert_eq!(
        *b.log.lock().unwrap(),
        [format!("mount {DISK}"), format!("unmount {DISK}")]
    );
    assert!(
        spoken(&b).is_empty(),
        "a plan plays no cue: {:?}",
        spoken(&b)
    );
    assert!(!b.mounted.load(Ordering::SeqCst));

    // The apply mounts, works and unmounts; "safe to unplug" plays once, after the unmount.
    b.log.lock().unwrap().clear();
    let r = b.core.gear_apply(&req(&c, &p)).unwrap();
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    assert_eq!(
        *b.log.lock().unwrap(),
        [format!("mount {DISK}"), format!("unmount {DISK}")]
    );
    assert!(!b.mounted.load(Ordering::SeqCst));
    assert_eq!(
        spoken(&b)
            .iter()
            .filter(|s| s.contains("safe to unplug"))
            .count(),
        1
    );
    assert!(read_unmounted(&b).contains("contrast: 24"));
}

fn read_unmounted(b: &Bench) -> String {
    read(b, "RADIO/radio.yml")
}

#[test]
fn the_person_mounts_a_card_and_it_unmounts_on_done_or_on_the_timer() {
    let b = bench(Opts::default());
    b.core
        .gear_card_unmount(&CardMountParams {
            device: b.id.clone(),
            minutes: None,
        })
        .unwrap();
    b.log.lock().unwrap().clear();

    let m = b
        .core
        .gear_card_mount(&CardMountParams {
            device: b.id.clone(),
            minutes: Some(5),
        })
        .unwrap();
    assert_eq!(m.mount, b.root);
    assert!(b.mounted.load(Ordering::SeqCst));
    assert_eq!(b.core.gear_mounted_cards().len(), 1);
    // Not yet due: nothing happens.
    b.core.gear_mount_tick(std::time::Instant::now());
    assert!(b.mounted.load(Ordering::SeqCst));
    // Past the timer it unmounts and says so.
    b.cues.played.lock().unwrap().clear();
    b.core
        .gear_mount_tick(std::time::Instant::now() + std::time::Duration::from_secs(5 * 60 + 1));
    assert!(!b.mounted.load(Ordering::SeqCst));
    assert!(b.core.gear_mounted_cards().is_empty());
    assert!(spoken(&b).iter().any(|s| s.contains("safe to unplug")));

    // Done before the timer.
    b.core
        .gear_card_mount(&CardMountParams {
            device: b.id.clone(),
            minutes: None,
        })
        .unwrap();
    b.core
        .gear_card_unmount(&CardMountParams {
            device: b.id.clone(),
            minutes: None,
        })
        .unwrap();
    assert!(!b.mounted.load(Ordering::SeqCst));
    assert!(b.core.gear_mounted_cards().is_empty());
}

#[test]
fn revert_of_a_card_change_restores_the_files_from_the_backup() {
    let b = bench(Opts::default());
    let original = std::fs::read(b.root.join("RADIO/radio.yml")).unwrap();
    let c = stage(&b, vec![contrast("28")]);
    let p = plan(&b, &c);
    let r = b.core.gear_apply(&req(&c, &p)).unwrap();
    assert_eq!(r.status, ChangeStatus::Verified);
    b.mounted.store(true, Ordering::SeqCst);
    assert!(read(&b, "RADIO/radio.yml").contains("contrast: 28"));

    // A restore stages as its own change, names files, and applies like any change.
    let rs = b
        .core
        .gear_restore_stage(&RestoreParams {
            backup: r.backup.clone().unwrap(),
            paths: vec!["RADIO/radio.yml".into()],
            editor: None,
        })
        .unwrap();
    let p = plan(&b, &rs);
    assert!(p.checks.iter().all(|k| k.ok), "{:?}", p.checks);
    let r2 = b.core.gear_apply(&req(&rs, &p)).unwrap();
    assert_eq!(r2.status, ChangeStatus::Verified, "{}", r2.message);
    b.mounted.store(true, Ordering::SeqCst);
    assert_eq!(
        std::fs::read(b.root.join("RADIO/radio.yml")).unwrap(),
        original
    );
    // A restore with no files named is refused at stage.
    let e = b
        .core
        .gear_restore_stage(&RestoreParams {
            backup: r.backup.unwrap(),
            paths: vec![],
            editor: None,
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("names the files"), "{e:#}");
}

#[test]
fn a_change_the_engine_does_not_plan_is_refused_at_stage() {
    let b = bench(Opts::default());
    let e = b
        .core
        .gear_change_stage(&StageParams {
            device: b.id.clone(),
            edits: vec![Edit::FcLines {
                lines: vec!["set x = 1".into()],
            }],
            ..Default::default()
        })
        .unwrap_err();
    assert_eq!(code(&e), RefusalCode::ShapeUnknown);
}

// ----- mount, work, unmount for every card operation -----

/// The bench with its card released, as an earlier job leaves it.
fn released(o: Opts) -> Bench {
    let b = bench(o);
    b.core
        .gear_card_unmount(&CardMountParams {
            device: b.id.clone(),
            minutes: None,
        })
        .unwrap();
    assert!(!b.mounted.load(Ordering::SeqCst));
    b.log.lock().unwrap().clear();
    b.cues.played.lock().unwrap().clear();
    b
}

fn mount_then_unmount() -> Vec<String> {
    vec![format!("mount {DISK}"), format!("unmount {DISK}")]
}

#[test]
fn a_backup_mounts_an_unmounted_card_and_unmounts_it() {
    let b = released(Opts::default());
    std::fs::write(b.root.join("RADIO/radio.yml"), "contrast: 31\n").unwrap();
    let r = b
        .core
        .gear_backup(&BackupParams {
            device: Some(b.id.clone()),
            ..Default::default()
        })
        .unwrap();
    assert!(r.notes.iter().all(|n| !n.contains("did not unmount")));
    assert_eq!(*b.log.lock().unwrap(), mount_then_unmount());
    assert!(!b.mounted.load(Ordering::SeqCst));
    assert_eq!(
        spoken(&b)
            .iter()
            .filter(|s| s.contains("safe to unplug"))
            .count(),
        1
    );
    // With no device named, the one unmounted card is the target.
    b.log.lock().unwrap().clear();
    b.core.gear_backup(&BackupParams::default()).unwrap();
    assert_eq!(*b.log.lock().unwrap(), mount_then_unmount());
}

#[test]
fn a_card_check_mounts_an_unmounted_card_and_unmounts_it() {
    let b = released(Opts::default());
    let k = b
        .core
        .gear_card_check(&CardCheckParams {
            device: Some(b.id.clone()),
            mount: None,
        })
        .unwrap();
    assert_eq!(k.state, CheckState::Ok);
    assert_eq!(*b.log.lock().unwrap(), mount_then_unmount());
    assert!(!b.mounted.load(Ordering::SeqCst));
    // No device named: the one card plugged in.
    b.log.lock().unwrap().clear();
    b.core.gear_card_check(&CardCheckParams::default()).unwrap();
    assert_eq!(*b.log.lock().unwrap(), mount_then_unmount());
}

#[test]
fn a_card_read_a_preview_and_a_clean_listing_release_the_card_quietly() {
    let b = released(Opts::default());
    b.core
        .gear_card(&CardParams {
            device: Some(b.id.clone()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(*b.log.lock().unwrap(), mount_then_unmount());
    b.log.lock().unwrap().clear();
    b.core
        .gear_card_preview(&CardPreviewParams {
            device: Some(b.id.clone()),
            edits: vec![contrast("22")],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(*b.log.lock().unwrap(), mount_then_unmount());
    b.log.lock().unwrap().clear();
    let listed = b
        .core
        .gear_card_clean(&CardCleanParams {
            device: Some(b.id.clone()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(listed.removed, 0);
    assert_eq!(*b.log.lock().unwrap(), mount_then_unmount());
    assert!(spoken(&b).is_empty(), "reads play no cue: {:?}", spoken(&b));
    assert!(!b.mounted.load(Ordering::SeqCst));
}

#[test]
fn a_clean_removal_mounts_removes_and_unmounts_once_with_the_cue() {
    let b = released(Opts::default());
    let mut junk = vec![0x00, 0x05, 0x16, 0x07, 0x00, 0x02, 0x00, 0x00];
    junk.extend_from_slice(b"Mac OS X        ");
    junk.resize(120, 0);
    std::fs::write(b.root.join("RADIO/._radio.yml"), &junk).unwrap();
    let r = b
        .core
        .gear_card_clean(&CardCleanParams {
            device: Some(b.id.clone()),
            remove: true,
            confirm: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(r.removed, 1, "{:?}", r.notes);
    assert!(!b.root.join("RADIO/._radio.yml").exists());
    assert_eq!(*b.log.lock().unwrap(), mount_then_unmount());
    assert_eq!(
        spoken(&b)
            .iter()
            .filter(|s| s.contains("safe to unplug"))
            .count(),
        1
    );
}

#[test]
fn a_card_that_is_mounted_already_is_not_mounted_again_for_a_read() {
    let b = bench(Opts::default());
    b.core
        .gear_card(&CardParams {
            device: Some(b.id.clone()),
            ..Default::default()
        })
        .unwrap();
    assert!(b.log.lock().unwrap().is_empty());
    assert!(
        b.mounted.load(Ordering::SeqCst),
        "a read leaves it as it was"
    );
}

#[test]
fn a_card_pulled_before_the_job_is_not_mounted() {
    let b = released(Opts::default());
    b.present.store(false, Ordering::SeqCst);
    let e = b
        .core
        .gear_backup(&BackupParams {
            device: Some(b.id.clone()),
            ..Default::default()
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("not plugged in"), "{e:#}");
    assert!(b.log.lock().unwrap().is_empty());
}
