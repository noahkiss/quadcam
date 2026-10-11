//! Staged changes and the FC apply (WP5a), against `FakeFc`: every check in design 8.2 that
//! applies to an FC refuses, a run that fails discards, a saved run is verified against
//! `dump all` (a default-valued `set` too), and confirm is the digest plus the click.
//! No real port opens.

use quadcam_lib::core::{
    BackupFilter, BackupParams, ChangeUpdateParams, Core, Hooks, NoHooks, RestoreParams,
    StageParams,
};
use quadcam_lib::gear::apply::{ApplyPlanParams, ApplyRequest};
use quadcam_lib::gear::bf::cli::Timing;
use quadcam_lib::gear::bf::fake::FakeFc;
use quadcam_lib::gear::changes::ChangeFilter;
use quadcam_lib::gear::cues::{CueService, RecordedCues};
use quadcam_lib::gear::model::{
    device_id, ApplyPlan, ChangeStatus, DeviceKind, Edit, Refusal, RefusalCode, Section,
    StagedChange, Trigger,
};
use quadcam_lib::gear::serial::{lock_port, FakePorts, PortInfo, Ports};
use quadcam_lib::gear::Env;
use quadcam_lib::photos::Recorder;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PORT: &str = "/dev/cu.usbmodemFAKE1";
const G473: &str = include_str!("fixtures/bf/g473-2025.12.5.dump_all.txt");
const UID: [u8; 12] = [
    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x10, 0x32, 0x54, 0x76,
];

struct Bench {
    core: Arc<Core>,
    cues: Arc<RecordedCues>,
    locks: std::path::PathBuf,
    id: String,
    _dir: tempfile::TempDir,
}

fn core_with(
    dir: &tempfile::TempDir,
    ports: Arc<dyn Ports>,
    holders: Option<quadcam_lib::gear::HoldersFn>,
    hooks: Arc<dyn Hooks>,
) -> (Arc<Core>, Arc<RecordedCues>) {
    let cues = Arc::new(RecordedCues::default());
    let mut env = Env::fake(vec![], ports);
    env.cues = Arc::new(CueService::inline(cues.clone()));
    if let Some(h) = holders {
        env.holders = h;
    }
    let core = Arc::new(
        Core::new(
            dir.path().join("cache"),
            None,
            hooks,
            Arc::new(Recorder::default()),
        )
        .with_settings(dir.path().join("support/settings.json"))
        .with_gear_env(env)
        .with_fc_timing(Timing::fast()),
    );
    (core, cues)
}

fn fc_device(id: &str) -> quadcam_lib::gear::model::Device {
    serde_json::from_value(serde_json::json!({"id": id, "kind": "fc"})).unwrap()
}

fn id_of(fc: &FakeFc) -> String {
    device_id(DeviceKind::Fc, &format!("bf-uid:{}", fc.uid_hex()))
}

/// A bench with one FC, backed up once so the plan has a base.
fn bench(fc: &FakeFc) -> Bench {
    bench_with(fc, None, Arc::new(NoHooks), true)
}

fn bench_with(
    fc: &FakeFc,
    holders: Option<quadcam_lib::gear::HoldersFn>,
    hooks: Arc<dyn Hooks>,
    backup: bool,
) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let locks = dir.path().join("locks");
    let ports = Arc::new(fc.ports(PORT, Some(locks.clone())));
    let (core, cues) = core_with(&dir, ports, holders, hooks);
    if backup {
        core.gear_backup(&BackupParams {
            port: Some(PORT.into()),
            ..Default::default()
        })
        .unwrap();
    }
    Bench {
        core,
        cues,
        locks,
        id: id_of(fc),
        _dir: dir,
    }
}

fn set(name: &str, value: &str) -> Edit {
    Edit::FcSet {
        section: Section::Master,
        name: name.into(),
        value: value.into(),
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

fn failed_check(p: &ApplyPlan) -> RefusalCode {
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

fn released(b: &Bench) -> bool {
    lock_port(&b.locks, PORT).is_ok()
}

#[test]
fn stage_plan_apply_verifies_and_records() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    let c = stage(&b, vec![set("osd_cap_alarm", "1500")]);
    assert_eq!(c.status, ChangeStatus::Ready);
    assert_eq!(c.title, "Set osd_cap_alarm = 1500");
    assert_eq!(b.core.gear_status().unwrap().staged, 1);
    let p = plan(&b, &c);
    assert!(p.ready(), "{:?}", p.checks);
    assert!(!p.digest.is_empty());
    assert_eq!(fc.saves(), 0, "a plan writes nothing");
    let exits_before = fc.exits();
    let cues_before = b.cues.spoken().len();

    let r = b.core.gear_apply(&req(&c, &p)).unwrap();
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    assert!(r.saved && r.verify.is_empty());
    assert_eq!(
        r.steps.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["Back up", "Write", "Read back", "Verify"]
    );
    assert_eq!(
        fc.saved_value(Section::Master, "osd_cap_alarm").as_deref(),
        Some("1500")
    );
    assert_eq!(fc.saves(), 1);
    assert!(fc.exits() > exits_before, "the backup read ended with exit");
    assert_eq!(status(&b, &c.id), ChangeStatus::Verified);
    assert_eq!(b.core.gear_status().unwrap().staged, 0);
    assert!(released(&b));
    assert_eq!(
        b.cues.spoken().len(),
        cues_before + 1,
        "one cue for the job"
    );

    // The backup before is kept; the state after is stored for the next plan.
    let list = b
        .core
        .gear_backups(&BackupFilter {
            device: Some(b.id.clone()),
        })
        .unwrap();
    assert!(list.iter().any(|s| s.trigger == Trigger::BeforeApply));
    assert!(list.iter().any(|s| s.trigger == Trigger::AfterApply));
    assert_eq!(
        r.backup.as_deref().map(|b| b.contains("before_apply")),
        Some(true)
    );

    // A second apply of the same change is refused: it is no longer staged.
    assert!(b.core.gear_apply(&req(&c, &p)).is_err());
}

#[test]
fn a_default_valued_set_verifies_against_dump_all() {
    // `diff all` hides a value equal to its default; `dump all` shows it.
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    let first = stage(&b, vec![set("osd_cap_alarm", "1500")]);
    let p = plan(&b, &first);
    b.core.gear_apply(&req(&first, &p)).unwrap();
    // 2200 is the seed's default: back to it.
    let second = stage(&b, vec![set("osd_cap_alarm", "2200")]);
    let p2 = plan(&b, &second);
    let r = b.core.gear_apply(&req(&second, &p2)).unwrap();
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    assert_eq!(
        fc.saved_value(Section::Master, "osd_cap_alarm").as_deref(),
        Some("2200")
    );
    let diff = b
        .core
        .gear_backup_read(&quadcam_lib::core::BackupReadParams {
            id: r.after_backup.clone().unwrap(),
            path: Some("diff all".into()),
        })
        .unwrap()
        .text
        .unwrap();
    assert!(
        !diff.contains("osd_cap_alarm"),
        "the default is not in diff all"
    );
}

#[test]
fn a_profile_edit_selects_and_puts_the_selection_back() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    let c = stage(
        &b,
        vec![Edit::FcSet {
            section: Section::Profile(2),
            name: "p_pitch".into(),
            value: "55".into(),
        }],
    );
    let p = plan(&b, &c);
    let r = b.core.gear_apply(&req(&c, &p)).unwrap();
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    assert_eq!(
        fc.saved_value(Section::Profile(2), "p_pitch").as_deref(),
        Some("55")
    );
    let sent: Vec<&str> = r.sent.iter().map(|s| s.line.as_str()).collect();
    assert_eq!(sent.first().copied(), Some("profile 2"));
    assert_eq!(
        sent.last().copied(),
        Some("profile 0"),
        "the original selection returns: {sent:?}"
    );
}

#[test]
fn unknown_version_and_board_refuse() {
    let v = G473.replace("2025.12.5-alpha", "4.3.2");
    let fc = FakeFc::new(&v).with_uid(UID);
    let b = bench(&fc);
    let c = stage(&b, vec![set("osd_cap_alarm", "1")]);
    let p = plan(&b, &c);
    assert!(!p.ready());
    assert_eq!(failed_check(&p), RefusalCode::UnknownVersion);
    let e = b.core.gear_apply(&req(&c, &p)).unwrap_err();
    assert_eq!(code(&e), RefusalCode::UnknownVersion);
    assert_eq!(fc.saves(), 0);

    let o = G473.replace("BETAFPVG473", "OTHERBOARD");
    let fc = FakeFc::new(&o).with_uid(UID);
    let b = bench(&fc);
    let c = stage(&b, vec![set("osd_cap_alarm", "1")]);
    assert_eq!(failed_check(&plan(&b, &c)), RefusalCode::UnknownBoard);
}

#[test]
fn another_fc_is_not_the_planned_one() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    let c = stage(&b, vec![set("osd_cap_alarm", "1500")]);
    let p = plan(&b, &c);
    // The same port now holds a different FC.
    let other = FakeFc::new(G473).with_uid([9; 12]);
    let dir = tempfile::tempdir().unwrap();
    let ports = Arc::new(other.ports(PORT, None));
    let (core2, _) = core_with(&dir, ports, None, Arc::new(NoHooks));
    // Same gear folder: the change and the device live there.
    let settings = b._dir.path().join("support/settings.json");
    let core2 = Arc::try_unwrap(core2).ok().unwrap().with_settings(settings);
    let p2 = core2
        .gear_apply_plan(&ApplyPlanParams {
            id: c.id.clone(),
            port: None,
        })
        .unwrap();
    assert!(!p2.ready());
    assert_eq!(failed_check(&p2), RefusalCode::DeviceChanged);
    let e = core2.gear_apply(&req(&c, &p)).unwrap_err();
    assert_eq!(code(&e), RefusalCode::DeviceChanged);
    assert_eq!(other.saves(), 0);
}

#[test]
fn no_fc_plugged_in_refuses() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    let c = stage(&b, vec![set("osd_cap_alarm", "1500")]);
    let dir2 = b._dir.path().join("support/settings.json");
    let none = tempfile::tempdir().unwrap();
    let (core2, _) = core_with(
        &none,
        Arc::new(FakePorts::new(vec![])),
        None,
        Arc::new(NoHooks),
    );
    let core2 = Arc::try_unwrap(core2).ok().unwrap().with_settings(dir2);
    let p = core2
        .gear_apply_plan(&ApplyPlanParams {
            id: c.id.clone(),
            port: None,
        })
        .unwrap();
    assert_eq!(failed_check(&p), RefusalCode::NoDevice);
}

#[test]
fn a_changed_fc_is_before_mismatch() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    let c = stage(&b, vec![set("osd_cap_alarm", "1500")]);
    let p = plan(&b, &c);
    // A configurator changes the same setting between the plan and the apply.
    fc.poke(Section::Master, "osd_cap_alarm", "999");
    let e = b.core.gear_apply(&req(&c, &p)).unwrap_err();
    assert_eq!(code(&e), RefusalCode::BeforeMismatch);
    assert_eq!(fc.saves(), 0, "nothing was written");
    assert_eq!(status(&b, &c.id), ChangeStatus::Ready);
    // And a plain wrong digest.
    let mut bad = req(&c, &p);
    bad.digest = "0000000000000000".into();
    let e = b.core.gear_apply(&bad).unwrap_err();
    assert_eq!(code(&e), RefusalCode::BeforeMismatch);
}

#[test]
fn no_backup_refuses() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench_with(&fc, None, Arc::new(NoHooks), false);
    // The device is known, but nothing has been backed up.
    let store = b.core.gear_store();
    store.save_device(&fc_device(&b.id.clone())).unwrap();
    let c = stage(&b, vec![set("osd_cap_alarm", "1500")]);
    let p = plan(&b, &c);
    assert_eq!(failed_check(&p), RefusalCode::NoBackup);
    let e = b.core.gear_apply(&req(&c, &p)).unwrap_err();
    assert_eq!(code(&e), RefusalCode::NoBackup);
    assert_eq!(fc.saves(), 0);
}

#[test]
fn bad_settings_refuse_at_stage_and_at_the_range_check() {
    let fc = FakeFc::new(G473)
        .with_uid(UID)
        .with_allowed("osd_cap_alarm", "Allowed range: 0 - 20000");
    let b = bench(&fc);
    // A name the FC does not have: refused at stage time.
    let e = b
        .core
        .gear_change_stage(&StageParams {
            device: b.id.clone(),
            edits: vec![set("no_such_setting", "1")],
            ..Default::default()
        })
        .unwrap_err();
    assert_eq!(code(&e), RefusalCode::BadSetting);
    // A value out of range: the plan passes, the FC's `get` refuses before any write.
    let c = stage(&b, vec![set("osd_cap_alarm", "30000")]);
    let p = plan(&b, &c);
    assert!(p.ready());
    let e = b.core.gear_apply(&req(&c, &p)).unwrap_err();
    assert_eq!(code(&e), RefusalCode::BadSetting);
    assert!(format!("{e}").contains("0-20000"), "{e}");
    assert_eq!(fc.saves(), 0);
    assert_eq!(
        fc.saved_value(Section::Master, "osd_cap_alarm").as_deref(),
        Some("2200")
    );
    assert_eq!(status(&b, &c.id), ChangeStatus::Ready);
    assert!(released(&b));
}

#[test]
fn lines_the_engine_never_sends_refuse() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    for l in ["save", "defaults", "bl", "set osd_cap_alarm"] {
        let e = b
            .core
            .gear_change_stage(&StageParams {
                device: b.id.clone(),
                edits: vec![Edit::FcLines {
                    lines: vec![l.into()],
                }],
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(code(&e), RefusalCode::ShapeUnknown, "{l}");
    }
}

#[test]
fn a_port_another_program_holds_refuses_and_so_do_two_unknown_fcs() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let held = Arc::new(AtomicBool::new(false));
    let h = held.clone();
    let holders: quadcam_lib::gear::HoldersFn = Arc::new(move |_| {
        if h.load(Ordering::SeqCst) {
            vec![(4242, "Configurator".to_string())]
        } else {
            Vec::new()
        }
    });
    let b = bench_with(&fc, Some(holders), Arc::new(NoHooks), true);
    let c = stage(&b, vec![set("osd_cap_alarm", "1500")]);
    let p = plan(&b, &c);
    assert!(p.ready());
    held.store(true, Ordering::SeqCst);
    let p2 = plan(&b, &c);
    assert_eq!(failed_check(&p2), RefusalCode::PortBusy);
    assert!(p2.checks.iter().any(|c| c
        .refusal
        .as_ref()
        .is_some_and(|r| r.reason.contains("Configurator"))));
    let e = b.core.gear_apply(&req(&c, &p)).unwrap_err();
    assert_eq!(code(&e), RefusalCode::PortBusy);
    assert_eq!(fc.saves(), 0);

    // Two FCs whose ports another program holds: neither can be identified, so pick one.
    let a = FakeFc::new(G473).with_uid(UID);
    let z = FakeFc::new(G473).with_uid([7; 12]);
    let (a2, z2) = (a.clone(), z.clone());
    let info = |port: &str| PortInfo {
        port: port.into(),
        vid: 0x0483,
        pid: 0x5740,
        serial_number: None,
        manufacturer: None,
        product: None,
    };
    let ports = Arc::new(
        FakePorts::new(vec![info("/dev/cu.usbmodemA"), info("/dev/cu.usbmodemZ")]).with_opener(
            move |port, _| {
                if port.ends_with('A') {
                    a2.open(port)
                } else {
                    z2.open(port)
                }
            },
        ),
    );
    let dir = tempfile::tempdir().unwrap();
    let busy: quadcam_lib::gear::HoldersFn = Arc::new(|_| vec![(1, "Configurator".to_string())]);
    let (core, _) = core_with(&dir, ports, Some(busy), Arc::new(NoHooks));
    core.gear_store()
        .save_device(&fc_device(&id_of(&a)))
        .unwrap();
    let c = core
        .gear_change_stage(&StageParams {
            device: id_of(&a),
            edits: vec![set("osd_cap_alarm", "1")],
            ..Default::default()
        })
        .unwrap();
    let p = core
        .gear_apply_plan(&ApplyPlanParams {
            id: c.id.clone(),
            port: None,
        })
        .unwrap();
    assert_eq!(failed_check(&p), RefusalCode::SeveralDevices);
}

#[test]
fn a_battery_in_past_the_usb_limit_refuses() {
    let fc = FakeFc::new(G473).with_uid(UID).with_reboot_opens(0);
    let b = bench(&fc);
    let c = stage(&b, vec![set("osd_cap_alarm", "1500")]);
    fc.set_battery(4.0);
    // The battery has been in for 12 minutes; this board's limit is 10.
    let past = Instant::now()
        .checked_sub(Duration::from_secs(12 * 60))
        .expect("uptime over 12 minutes");
    b.core.gear_usb_tick(past);
    let p = plan(&b, &c);
    assert_eq!(failed_check(&p), RefusalCode::UsbHeat);
    let e = b.core.gear_apply(&req(&c, &p)).unwrap_err();
    assert_eq!(code(&e), RefusalCode::UsbHeat);
    assert_eq!(fc.saves(), 0);
    // Battery out: the same change plans clean.
    fc.set_battery(0.0);
    b.core.gear_usb_tick(past + Duration::from_secs(31));
    assert!(plan(&b, &c).ready());
}

#[test]
fn a_battery_with_no_usb_timer_is_a_warning_on_an_apply() {
    let fc = FakeFc::new(G473).with_uid(UID).with_reboot_opens(0);
    let b = bench(&fc);
    let c = stage(&b, vec![set("osd_cap_alarm", "1500")]);
    // The battery is in, but no probe has run: the job reads it and warns.
    fc.set_battery(4.0);
    let p = plan(&b, &c);
    assert!(p.ready());
    let r = b.core.gear_apply(&req(&c, &p)).unwrap();
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    assert!(
        r.notes.iter().any(|n| n.contains("not counting")),
        "{:?}",
        r.notes
    );
}

#[test]
fn an_error_before_save_discards_everything() {
    let fc = FakeFc::new(G473).with_uid(UID).reject("set osd_ah_pos = 1");
    let b = bench(&fc);
    let c = stage(
        &b,
        vec![set("osd_cap_alarm", "1500"), set("osd_ah_pos", "1")],
    );
    let p = plan(&b, &c);
    let r = b.core.gear_apply(&req(&c, &p)).unwrap();
    assert_eq!(r.status, ChangeStatus::Failed);
    assert!(!r.saved);
    assert_eq!(r.failed_line.as_ref().unwrap().line, "set osd_ah_pos = 1");
    assert_eq!(r.sent.len(), 2, "stops at the first error");
    assert_eq!(fc.saves(), 0, "no save");
    assert_eq!(
        fc.saved_value(Section::Master, "osd_cap_alarm").as_deref(),
        Some("2200"),
        "the first line is discarded with the session"
    );
    assert_eq!(status(&b, &c.id), ChangeStatus::Failed);
    assert!(r
        .steps
        .iter()
        .any(|s| s.name == "Write" && s.detail.is_some()));
    assert!(released(&b));
}

#[test]
fn a_value_the_fc_does_not_keep_fails_verify_and_a_restore_undoes_it() {
    let fc = FakeFc::new(G473)
        .with_uid(UID)
        .forget("set osd_cap_alarm = 1500");
    let b = bench(&fc);
    let c = stage(
        &b,
        vec![set("osd_cap_alarm", "1500"), set("osd_ah_pos", "4000")],
    );
    let p = plan(&b, &c);
    let r = b.core.gear_apply(&req(&c, &p)).unwrap();
    assert_eq!(r.status, ChangeStatus::Failed);
    assert!(r.saved);
    assert_eq!(r.verify.len(), 1);
    assert_eq!(r.verify[0].line, "set osd_cap_alarm = 1500");
    assert_eq!(
        fc.saved_value(Section::Master, "osd_ah_pos").as_deref(),
        Some("4000")
    );

    // Restore backup: the backup taken before the write, staged as a change.
    let rc = b
        .core
        .gear_restore_stage(&RestoreParams {
            backup: r.backup.clone().unwrap(),
            ..Default::default()
        })
        .unwrap();
    let rp = plan(&b, &rc);
    assert!(rp.ready(), "{:?}", rp.checks);
    let rr = b.core.gear_apply(&req(&rc, &rp)).unwrap();
    assert_eq!(
        rr.status,
        ChangeStatus::Verified,
        "{} {:?} {:?}",
        rr.message,
        rr.verify,
        rr.sent
    );
    assert_eq!(
        fc.saved_value(Section::Master, "osd_ah_pos").as_deref(),
        Some("4281")
    );
}

struct Gate {
    answer: AtomicBool,
    asked: AtomicBool,
    /// Each outcome `apply_done` got: the report's status, or the error.
    done: Mutex<Vec<String>>,
}

impl Hooks for Gate {
    fn confirm_apply(&self, _c: &StagedChange, _p: &ApplyPlan) -> anyhow::Result<()> {
        self.asked.store(true, Ordering::SeqCst);
        if self.answer.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "Refused: the user cancelled the apply in quadcam."
            ))
        }
    }
    fn apply_done(&self, out: &anyhow::Result<quadcam_lib::gear::apply::ApplyReport>) {
        self.done.lock().unwrap().push(match out {
            Ok(r) => format!("{:?}", r.status),
            Err(e) => format!("{e}"),
        });
    }
}

#[test]
fn confirm_is_the_digest_the_flag_and_the_click() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let gate = Arc::new(Gate {
        answer: AtomicBool::new(false),
        asked: AtomicBool::new(false),
        done: Mutex::new(vec![]),
    });
    let b = bench_with(&fc, None, gate.clone(), true);
    let c = stage(&b, vec![set("osd_cap_alarm", "1500")]);
    let p = plan(&b, &c);
    // No confirm flag.
    let mut r = req(&c, &p);
    r.confirm = false;
    let e = b.core.gear_apply(&r).unwrap_err();
    assert!(format!("{e}").starts_with("Refused"), "{e}");
    assert!(!gate.asked.load(Ordering::SeqCst));
    // The person says no in the app.
    let e = b.core.gear_apply(&req(&c, &p)).unwrap_err();
    assert!(format!("{e}").contains("cancelled"), "{e}");
    assert!(gate.asked.load(Ordering::SeqCst));
    assert_eq!(fc.saves(), 0);
    assert_eq!(status(&b, &c.id), ChangeStatus::Ready);
    // The sheet's own Apply click is the confirm.
    let r = b.core.gear_apply_click(&req(&c, &p)).unwrap();
    assert_eq!(r.status, ChangeStatus::Verified);
    // Each agent call's outcome reaches the host; the sheet's own click does not.
    let done = gate.done.lock().unwrap().clone();
    assert_eq!(done.len(), 2, "{done:?}");
    assert!(done.iter().all(|d| d.starts_with("Refused")), "{done:?}");
    // The person says yes: the host gets the report of the write that follows.
    let c = stage(&b, vec![set("osd_cap_alarm", "1400")]);
    let p = plan(&b, &c);
    gate.answer.store(true, Ordering::SeqCst);
    let r = b.core.gear_apply(&req(&c, &p)).unwrap();
    assert_eq!(r.status, ChangeStatus::Verified);
    assert_eq!(gate.done.lock().unwrap().last().unwrap(), "Verified");
}

#[test]
fn update_discard_and_filters() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    let a = stage(&b, vec![set("osd_cap_alarm", "1500")]);
    let z = stage(&b, vec![set("osd_ah_pos", "4000")]);
    let u = b
        .core
        .gear_change_update(&ChangeUpdateParams {
            id: a.id.clone(),
            edits: Some(vec![set("osd_cap_alarm", "1600")]),
            status: Some(ChangeStatus::Draft),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(u.status, ChangeStatus::Draft);
    // An edit that cannot be sent is refused on update too.
    let e = b
        .core
        .gear_change_update(&ChangeUpdateParams {
            id: a.id.clone(),
            edits: Some(vec![set("nope", "1")]),
            ..Default::default()
        })
        .unwrap_err();
    assert_eq!(code(&e), RefusalCode::BadSetting);
    b.core.gear_change_discard(&z.id).unwrap();
    let staged = b.core.gear_changes(&ChangeFilter::default()).unwrap();
    assert_eq!(staged.len(), 1);
    assert_eq!(
        b.core
            .gear_changes(&ChangeFilter {
                history: true,
                ..Default::default()
            })
            .unwrap()
            .len(),
        2
    );
    assert!(b
        .core
        .gear_apply_plan(&ApplyPlanParams {
            id: z.id.clone(),
            port: None
        })
        .is_err());
}

#[test]
fn mcp_apply_needs_the_digest_and_confirm() {
    use quadcam_lib::mcp::{LocalBackend, Server};
    use serde_json::json;
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    let mut s = Server::new(LocalBackend(b.core.clone()));
    let r = s.call_tool(
        "quadcam_gear_edit",
        json!({"action": "stage", "device": b.id, "lines": ["set osd_cap_alarm = 1500"], "title": "Alarm"}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let change = r["structuredContent"]["id"].as_str().unwrap().to_string();
    assert_eq!(r["structuredContent"]["editor"], "agent");
    let r = s.call_tool("quadcam_gear", json!({"action": "changes"}));
    assert!(r["content"][0]["text"].as_str().unwrap().contains(&change));
    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "apply_plan", "change": change}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let digest = r["structuredContent"]["digest"]
        .as_str()
        .unwrap()
        .to_string();
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("+set osd_cap_alarm = 1500") && text.contains("digest="),
        "{text}"
    );
    // No digest, no confirm, wrong digest: refused, FC untouched.
    for args in [
        json!({"action": "apply", "change": change}),
        json!({"action": "apply", "change": change, "digest": digest}),
        json!({"action": "apply", "change": change, "digest": "0000000000000000", "confirm": true}),
    ] {
        let r = s.call_tool("quadcam_gear_apply", args);
        assert_eq!(r["isError"], true, "{r}");
    }
    assert_eq!(fc.saves(), 0);
    let r = s.call_tool(
        "quadcam_gear_apply",
        json!({"action": "apply", "change": change, "digest": digest, "confirm": true}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with("Verified"));
    assert_eq!(
        fc.saved_value(Section::Master, "osd_cap_alarm").as_deref(),
        Some("1500")
    );
}
