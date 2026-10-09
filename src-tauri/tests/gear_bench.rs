//! The Bench half of WP5b against `FakeFc`: Try changes with Keep and Revert, Read first,
//! copying settings between two quads (golden diff, release and firmware checks), the OSD
//! element writer, the `apply_ready` on-connect step (off by default, never without a window
//! to click in), and the MCP actions. No real port opens.

use chrono::Utc;
use quadcam_lib::core::{
    apply_ready_hooks, BackupParams, ChangeUpdateParams, CopyParams, Core, Hooks, NoHooks,
    StageParams,
};
use quadcam_lib::gear::apply::{ApplyPlanParams, ApplyRequest};
use quadcam_lib::gear::backup::Snapshots;
use quadcam_lib::gear::bf::cli::Timing;
use quadcam_lib::gear::bf::dump::Config;
use quadcam_lib::gear::bf::fake::FakeFc;
use quadcam_lib::gear::changes::ChangeFilter;
use quadcam_lib::gear::copy::{CopyPart, CopySelect};
use quadcam_lib::gear::cues::{CueService, RecordedCues};
use quadcam_lib::gear::model::{
    device_id, ApplyPlan, ChangeStatus, DeviceKind, Edit, LineOp, Refusal, RefusalCode, Section,
    StagedChange, Trigger,
};
use quadcam_lib::gear::osd::Pos;
use quadcam_lib::gear::serial::Ports;
use quadcam_lib::gear::store::Store;
use quadcam_lib::gear::Env;
use quadcam_lib::photos::Recorder;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const PORT: &str = "/dev/cu.usbmodemFAKE1";
const G473: &str = include_str!("fixtures/bf/g473-2025.12.5.dump_all.txt");
const UID: [u8; 12] = [
    0x2f, 0, 0x38, 0, 0x35, 0x34, 0x11, 0x51, 0x39, 0x36, 0x30, 0x13,
];

struct Bench {
    core: Arc<Core>,
    id: String,
    dir: tempfile::TempDir,
}

struct Window {
    asked: AtomicBool,
}

impl Hooks for Window {
    fn has_gui(&self) -> bool {
        true
    }
    fn confirm_apply(&self, _c: &StagedChange, _p: &ApplyPlan) -> anyhow::Result<()> {
        self.asked.store(true, Ordering::SeqCst);
        Ok(())
    }
}

fn bench_with(fc: &FakeFc, hooks: Arc<dyn Hooks>, settings: &str) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let ports: Arc<dyn Ports> = Arc::new(fc.ports(PORT, Some(dir.path().join("locks"))));
    let mut env = Env::fake(vec![], ports);
    env.cues = Arc::new(CueService::inline(Arc::new(RecordedCues::default())));
    std::fs::create_dir_all(dir.path().join("support")).unwrap();
    std::fs::write(dir.path().join("support/settings.json"), settings).unwrap();
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
    core.gear_backup(&BackupParams {
        port: Some(PORT.into()),
        ..Default::default()
    })
    .unwrap();
    Bench {
        core,
        id: device_id(DeviceKind::Fc, &format!("bf-uid:{}", fc.uid_hex())),
        dir,
    }
}

fn bench(fc: &FakeFc) -> Bench {
    bench_with(fc, Arc::new(NoHooks), "{}")
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

fn mark(b: &Bench, c: &StagedChange, status: ChangeStatus) {
    b.core
        .gear_change_update(&ChangeUpdateParams {
            id: c.id.clone(),
            status: Some(status),
            ..Default::default()
        })
        .unwrap();
}

fn plan(b: &Bench, id: &str) -> ApplyPlan {
    b.core
        .gear_apply_plan(&ApplyPlanParams {
            id: id.into(),
            port: None,
        })
        .unwrap()
}

fn apply(b: &Bench, id: &str) -> quadcam_lib::gear::apply::ApplyReport {
    let p = plan(b, id);
    b.core
        .gear_apply(&ApplyRequest {
            id: id.into(),
            digest: p.digest,
            confirm: true,
            port: None,
        })
        .unwrap()
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

fn code(e: &anyhow::Error) -> RefusalCode {
    e.downcast_ref::<Refusal>()
        .unwrap_or_else(|| panic!("not a refusal: {e:#}"))
        .code
}

/// A second quad: a saved FC with one backup of this `dump all`.
fn second_quad(b: &Bench, dump: &str) -> String {
    let id = device_id(DeviceKind::Fc, "bf-uid:second");
    let store = Store::new(b.dir.path().join("support/gear"));
    let d: quadcam_lib::gear::model::Device =
        serde_json::from_value(serde_json::json!({"id": id, "kind": "fc", "name": "Five-inch"}))
            .unwrap();
    store.save_device(&d).unwrap();
    Snapshots::new(store)
        .take_files(
            &id,
            &Config::parse(dump).identity(),
            Trigger::Manual,
            Utc::now(),
            &[("dump all".into(), dump.as_bytes().to_vec())],
            false,
        )
        .unwrap();
    id
}

#[test]
fn a_try_change_waits_as_applied_and_keep_makes_it_verified() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    let c = stage(&b, vec![set("osd_cap_alarm", "1500")]);
    mark(&b, &c, ChangeStatus::Try);
    assert_eq!(status(&b, &c.id), ChangeStatus::Try);
    let r = apply(&b, &c.id);
    assert_eq!(
        r.status,
        ChangeStatus::Verified,
        "the report says what the FC did"
    );
    assert_eq!(status(&b, &c.id), ChangeStatus::Applied);
    // It is no longer staged, and Keep is only for an applied change.
    assert_eq!(b.core.gear_status().unwrap().staged, 0);
    let ready = stage(&b, vec![set("osd_ah_pos", "4000")]);
    assert!(b.core.gear_change_keep(&ready.id).is_err());
    let kept = b.core.gear_change_keep(&c.id).unwrap();
    assert_eq!(kept.status, ChangeStatus::Verified);
    assert!(b.core.gear_change_keep(&c.id).is_err(), "once");
}

#[test]
fn revert_stages_a_restore_and_the_change_is_reverted_when_it_verifies() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    let c = stage(&b, vec![set("osd_cap_alarm", "1500")]);
    mark(&b, &c, ChangeStatus::Try);
    apply(&b, &c.id);
    assert_eq!(
        fc.saved_value(Section::Master, "osd_cap_alarm").as_deref(),
        Some("1500")
    );
    let rv = b.core.gear_change_revert(&c.id).unwrap();
    assert_eq!(rv.reverts.as_deref(), Some(c.id.as_str()));
    assert_eq!(rv.title, format!("Revert: {}", c.title));
    assert!(
        matches!(&rv.edits[0], Edit::Restore { backup, .. } if backup.contains("before_apply"))
    );
    assert!(
        b.core.gear_change_revert(&c.id).is_err(),
        "one revert at a time"
    );
    // Not reverted until the restore verifies.
    assert_eq!(status(&b, &c.id), ChangeStatus::Applied);
    let r = apply(&b, &rv.id);
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    assert_eq!(
        fc.saved_value(Section::Master, "osd_cap_alarm").as_deref(),
        Some("2200")
    );
    assert_eq!(status(&b, &rv.id), ChangeStatus::Verified);
    assert_eq!(status(&b, &c.id), ChangeStatus::Reverted);
}

#[test]
fn a_read_first_change_refuses_until_it_is_ready() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    let c = stage(&b, vec![set("osd_cap_alarm", "1500")]);
    mark(&b, &c, ChangeStatus::ReadFirst);
    let p = plan(&b, &c.id);
    let first = p.checks.iter().find(|k| !k.ok).unwrap();
    assert_eq!(first.refusal.as_ref().unwrap().code, RefusalCode::ReadFirst);
    let e = b
        .core
        .gear_apply(&ApplyRequest {
            id: c.id.clone(),
            digest: p.digest,
            confirm: true,
            port: None,
        })
        .unwrap_err();
    assert_eq!(code(&e), RefusalCode::ReadFirst);
    assert_eq!(fc.saves(), 0);
    mark(&b, &c, ChangeStatus::Ready);
    assert!(plan(&b, &c.id).ready());
}

#[test]
fn an_osd_element_move_applies_and_verifies() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    let c = stage(
        &b,
        vec![Edit::OsdElement {
            element: "vbat".into(),
            x: 20,
            y: 9,
            profiles: vec![1, 2],
        }],
    );
    let p = plan(&b, &c.id);
    assert!(p.ready(), "{:?}", p.checks);
    let r = apply(&b, &c.id);
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    let held: u16 = fc
        .saved_value(Section::Master, "osd_vbat_pos")
        .unwrap()
        .parse()
        .unwrap();
    let pos = Pos::decode(held);
    assert_eq!((pos.x, pos.y, pos.profiles), (20, 9, 0b011));
}

#[test]
fn copy_settings_between_quads_stages_one_change_that_applies() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    // The other quad has its own OSD alarm and the same mode ranges.
    let other = second_quad(
        &b,
        &G473.replace("set osd_cap_alarm = 2200", "set osd_cap_alarm = 1800"),
    );
    let params = CopyParams {
        from: other.clone(),
        to: b.id.clone(),
        select: CopySelect {
            parts: vec![CopyPart::Osd, CopyPart::Modes],
            settings: vec![],
        },
        editor: None,
    };
    let plan = b.core.gear_copy_plan(&params).unwrap();
    assert!(plan.checks.iter().all(|c| c.ok), "{:?}", plan.checks);
    let diff: Vec<String> = plan
        .diff
        .iter()
        .map(|l| format!("{}{}", if l.op == LineOp::Add { "+" } else { "-" }, l.text))
        .collect();
    // Golden: the single OSD alarm differs; the modes are the same.
    assert_eq!(
        diff,
        ["-set osd_cap_alarm = 2200", "+set osd_cap_alarm = 1800"]
    );
    assert!(plan.same > 0);
    // From the device's latest backup or the named backup: the same plan.
    let backup = Snapshots::new(Store::new(b.dir.path().join("support/gear")))
        .latest(&other)
        .unwrap()
        .id;
    let by_backup = b
        .core
        .gear_copy_plan(&CopyParams {
            from: backup,
            ..params.clone()
        })
        .unwrap();
    assert_eq!(by_backup.edits, plan.edits);

    let c = b.core.gear_copy_stage(&params).unwrap();
    assert_eq!(c.device, b.id);
    assert_eq!(c.title, "Copy OSD, modes from Five-inch");
    assert_eq!(c.status, ChangeStatus::Ready);
    assert_eq!(fc.saves(), 0, "staging writes nothing");
    let r = apply(&b, &c.id);
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    assert_eq!(
        fc.saved_value(Section::Master, "osd_cap_alarm").as_deref(),
        Some("1800")
    );
    // A second copy has nothing left to do.
    let e = b.core.gear_copy_stage(&params).unwrap_err();
    assert_eq!(code(&e), RefusalCode::Incompatible);
}

#[test]
fn copy_refuses_another_release_the_same_quad_and_a_quad_with_no_backup() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    let newer = second_quad(&b, &G473.replace("2025.12.5-alpha", "2026.6.0"));
    let params = CopyParams {
        from: newer,
        to: b.id.clone(),
        select: CopySelect {
            parts: vec![CopyPart::Rates],
            settings: vec![],
        },
        editor: None,
    };
    let plan = b.core.gear_copy_plan(&params).unwrap();
    let bad = plan.checks.iter().find(|c| !c.ok).unwrap();
    assert_eq!(bad.name, "Same release");
    assert_eq!(
        bad.refusal.as_ref().unwrap().code,
        RefusalCode::Incompatible
    );
    assert!(plan.edits.is_empty());
    assert_eq!(
        code(&b.core.gear_copy_stage(&params).unwrap_err()),
        RefusalCode::Incompatible
    );
    // The same quad.
    let e = b
        .core
        .gear_copy_plan(&CopyParams {
            from: b.id.clone(),
            ..params.clone()
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("same FC"), "{e:#}");
    // A source with no backup.
    let store = Store::new(b.dir.path().join("support/gear"));
    let empty = device_id(DeviceKind::Fc, "bf-uid:empty");
    store
        .save_device(
            &serde_json::from_value(serde_json::json!({"id": empty, "kind": "fc"})).unwrap(),
        )
        .unwrap();
    let e = b
        .core
        .gear_copy_plan(&CopyParams {
            from: empty,
            ..params
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("no backup"), "{e:#}");
}

fn connected_fc(b: &Bench) -> quadcam_lib::gear::model::Connected {
    b.core
        .gear_connected()
        .unwrap()
        .into_iter()
        .find(|c| c.kind == DeviceKind::Fc)
        .expect("the fake FC is plugged in")
}

#[test]
fn apply_ready_is_off_by_default_and_needs_a_window_to_click_in() {
    let on = r#"{"gearOnConnect": {"fc": ["apply_ready"]}}"#;
    // Off by default: nothing runs.
    let fc = FakeFc::new(G473).with_uid(UID);
    let window = Arc::new(Window {
        asked: AtomicBool::new(false),
    });
    let b = bench_with(&fc, window.clone(), "{}");
    for h in apply_ready_hooks() {
        b.core.gear_add_hook(h);
    }
    let c = stage(&b, vec![set("osd_cap_alarm", "1500")]);
    b.core.gear_on_connect(&connected_fc(&b));
    assert!(!window.asked.load(Ordering::SeqCst));
    assert_eq!(status(&b, &c.id), ChangeStatus::Ready);

    // On, but with no window (headless): the step skips; nothing is written.
    let fc2 = FakeFc::new(G473).with_uid(UID);
    let b2 = bench_with(&fc2, Arc::new(NoHooks), on);
    for h in apply_ready_hooks() {
        b2.core.gear_add_hook(h);
    }
    let c2 = stage(&b2, vec![set("osd_cap_alarm", "1500")]);
    b2.core.gear_on_connect(&connected_fc(&b2));
    assert_eq!(status(&b2, &c2.id), ChangeStatus::Ready);
    assert_eq!(fc2.saves(), 0);

    // On, with a window: the plan passes its checks, the sheet is asked, the click applies.
    let fc3 = FakeFc::new(G473).with_uid(UID);
    let window = Arc::new(Window {
        asked: AtomicBool::new(false),
    });
    let b3 = bench_with(&fc3, window.clone(), on);
    for h in apply_ready_hooks() {
        b3.core.gear_add_hook(h);
    }
    let c3 = stage(&b3, vec![set("osd_cap_alarm", "1500")]);
    let draft = stage(&b3, vec![set("osd_ah_pos", "4000")]);
    mark(&b3, &draft, ChangeStatus::Draft);
    let runs = b3.core.gear_on_connect(&connected_fc(&b3));
    assert!(window.asked.load(Ordering::SeqCst), "{runs:?}");
    assert_eq!(status(&b3, &c3.id), ChangeStatus::Verified);
    assert_eq!(
        status(&b3, &draft.id),
        ChangeStatus::Draft,
        "drafts are left alone"
    );
    assert_eq!(
        fc3.saved_value(Section::Master, "osd_cap_alarm").as_deref(),
        Some("1500")
    );
}

#[test]
fn mcp_copies_keeps_and_reverts() {
    use quadcam_lib::mcp::{LocalBackend, Server};
    use serde_json::json;
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    let other = second_quad(
        &b,
        &G473.replace("set osd_cap_alarm = 2200", "set osd_cap_alarm = 1800"),
    );
    let mut s = Server::new(LocalBackend(b.core.clone()));
    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "copy_plan", "from": other, "to": b.id, "parts": ["osd"]}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("+set osd_cap_alarm = 1800") && text.contains("pass Same release"),
        "{text}"
    );
    let r = s.call_tool(
        "quadcam_gear_edit",
        json!({"action": "copy_stage", "from": other, "to": b.id, "parts": ["osd"]}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let id = r["structuredContent"]["id"].as_str().unwrap().to_string();
    assert_eq!(r["structuredContent"]["editor"], "agent");
    let r = s.call_tool(
        "quadcam_gear_edit",
        json!({"action": "update", "change": id, "status": "try"}),
    );
    assert_eq!(r["isError"], false, "{r}");
    apply(&b, &id);
    let r = s.call_tool(
        "quadcam_gear_edit",
        json!({"action": "revert_stage", "change": id}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert!(r["structuredContent"]["reverts"] == json!(id));
    let r = s.call_tool("quadcam_gear_edit", json!({"action": "keep", "change": id}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["status"], "verified");
}
