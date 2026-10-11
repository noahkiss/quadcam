//! The OSD editor (WP7) against `FakeFc`: moves, toggles and profile copies join one staged
//! "OSD layout" change, the working view shows them with the check, and the change applies.
//! No real port opens.

use quadcam_lib::core::{BackupParams, Core, NoHooks, OsdEditParams, OsdParams};
use quadcam_lib::gear::apply::{ApplyPlanParams, ApplyRequest};
use quadcam_lib::gear::bf::cli::Timing;
use quadcam_lib::gear::bf::fake::FakeFc;
use quadcam_lib::gear::changes::ChangeFilter;
use quadcam_lib::gear::cues::{CueService, RecordedCues};
use quadcam_lib::gear::model::{
    device_id, ChangeStatus, DeviceKind, DiffItem, Edit, LineOp, Section, StagedChange,
};
use quadcam_lib::gear::osd::{OsdCopy, OsdMove, Pos, ProblemKind};
use quadcam_lib::gear::serial::Ports;
use quadcam_lib::gear::Env;
use quadcam_lib::mcp::{LocalBackend, Server};
use quadcam_lib::photos::Recorder;
use serde_json::json;
use std::sync::Arc;

const PORT: &str = "/dev/cu.usbmodemFAKE1";
const G473: &str = include_str!("fixtures/bf/g473-2025.12.5.dump_all.txt");
const UID: [u8; 12] = [
    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x10, 0x32, 0x54, 0x76,
];

struct Bench {
    core: Arc<Core>,
    id: String,
    _dir: tempfile::TempDir,
}

fn bench(fc: &FakeFc) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let ports: Arc<dyn Ports> = Arc::new(fc.ports(PORT, Some(dir.path().join("locks"))));
    let mut env = Env::fake(vec![], ports);
    env.cues = Arc::new(CueService::inline(Arc::new(RecordedCues::default())));
    std::fs::create_dir_all(dir.path().join("support")).unwrap();
    std::fs::write(dir.path().join("support/settings.json"), "{}").unwrap();
    let core = Arc::new(
        Core::new(
            dir.path().join("cache"),
            None,
            Arc::new(NoHooks),
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
        _dir: dir,
    }
}

fn mv(element: &str, x: Option<u8>, y: Option<u8>, profiles: Option<Vec<u8>>) -> OsdMove {
    OsdMove {
        element: element.into(),
        x,
        y,
        profiles,
    }
}

fn edit(b: &Bench, moves: Vec<OsdMove>, copy: Option<OsdCopy>) -> anyhow::Result<StagedChange> {
    b.core.gear_osd_edit(&OsdEditParams {
        device: b.id.clone(),
        moves,
        copy,
        editor: None,
    })
}

fn open_changes(b: &Bench) -> Vec<StagedChange> {
    b.core
        .gear_changes(&ChangeFilter {
            device: Some(b.id.clone()),
            ..Default::default()
        })
        .unwrap()
}

fn line(name: &str, x: u8, y: u8, profiles: u8) -> String {
    let v = Pos {
        x,
        y,
        profiles,
        variant: 0,
    }
    .encode();
    format!("set {name} = {v}")
}

fn plan_lines(b: &Bench, id: &str) -> Vec<String> {
    let p = b
        .core
        .gear_apply_plan(&ApplyPlanParams {
            id: id.into(),
            port: None,
        })
        .unwrap();
    assert!(p.ready(), "{:?}", p.checks);
    p.diff
        .iter()
        .flat_map(|d| match d {
            DiffItem::Lines { lines, .. } => lines
                .iter()
                .filter(|l| l.op == LineOp::Add)
                .map(|l| l.text.clone())
                .collect::<Vec<_>>(),
            _ => vec![],
        })
        .collect()
}

fn working(b: &Bench) -> quadcam_lib::gear::osd::OsdView {
    b.core
        .gear_osd(&OsdParams {
            device: Some(b.id.clone()),
            staged: true,
            ..Default::default()
        })
        .unwrap()
}

fn element<'a>(
    v: &'a quadcam_lib::gear::osd::OsdView,
    name: &str,
) -> &'a quadcam_lib::gear::osd::OsdElement {
    v.elements.iter().find(|e| e.name == name).unwrap()
}

#[test]
fn a_move_stages_the_cli_line_and_later_edits_join_one_change() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    // vbat sits at x 2, y 16 on all three profiles in the dump.
    let c = edit(&b, vec![mv("vbat", Some(20), Some(9), None)], None).unwrap();
    assert_eq!(c.title, "OSD layout");
    assert_eq!(plan_lines(&b, &c.id), [line("osd_vbat_pos", 20, 9, 7)]);

    // A second element joins the same change; the first stays.
    let c2 = edit(&b, vec![mv("rssi", Some(5), Some(4), Some(vec![1]))], None).unwrap();
    assert_eq!(c2.id, c.id);
    assert_eq!(open_changes(&b).len(), 1);
    let mut lines = plan_lines(&b, &c.id);
    lines.sort();
    assert_eq!(
        lines,
        [
            line("osd_rssi_pos", 5, 4, 1),
            line("osd_vbat_pos", 20, 9, 7)
        ]
    );

    // Moving vbat again replaces its edit; putting it back drops it.
    let c3 = edit(&b, vec![mv("vbat", Some(21), None, None)], None).unwrap();
    assert_eq!(c3.edits.len(), 2);
    assert!(plan_lines(&b, &c.id).contains(&line("osd_vbat_pos", 21, 9, 7)));
    let c4 = edit(&b, vec![mv("vbat", Some(2), Some(16), None)], None).unwrap();
    assert_eq!(c4.edits.len(), 1);

    // Undoing the last edit discards the change.
    let c5 = edit(&b, vec![mv("rssi", Some(0), Some(6), Some(vec![]))], None).unwrap();
    assert_eq!(c5.status, ChangeStatus::Discarded);
    assert!(open_changes(&b).is_empty());
    // Nothing differs: nothing is staged.
    let e = edit(&b, vec![mv("vbat", Some(2), Some(16), None)], None).unwrap_err();
    assert!(e.to_string().contains("already has that layout"), "{e:#}");
}

#[test]
fn the_working_view_shows_staged_edits_with_the_check() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    let before = working(&b);
    assert_eq!(before.grid.name, "HD");
    assert_eq!(
        (element(&before, "vbat").x, element(&before, "vbat").y),
        (2, 16)
    );
    assert!(element(&before, "rssi").profiles.is_empty());

    // vbat onto the spot of rssi, rssi on in profile 1: they overlap there only.
    edit(
        &b,
        vec![
            mv("vbat", Some(0), Some(6), None),
            mv("rssi", None, None, Some(vec![1])),
        ],
        None,
    )
    .unwrap();
    let after = working(&b);
    assert_eq!(
        (element(&after, "vbat").x, element(&after, "vbat").y),
        (0, 6)
    );
    assert_eq!(element(&after, "rssi").profiles, [1]);
    let p1 = &after.profiles[0].problems;
    assert!(
        p1.iter().any(|p| p.kind == ProblemKind::Overlap
            && [Some(p.element.as_str()), p.other.as_deref()].contains(&Some("rssi"))),
        "{p1:?}"
    );
    // rssi is off in profile 2, so nothing there involves it.
    assert!(after.profiles[1]
        .problems
        .iter()
        .all(|p| p.element != "rssi" && p.other.as_deref() != Some("rssi")));
    assert!(
        after.notes.iter().any(|n| n.contains("staged OSD edits")),
        "{:?}",
        after.notes
    );
    // Without `staged` the view is the FC's.
    let fc_view = b
        .core
        .gear_osd(&OsdParams {
            device: Some(b.id.clone()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(element(&fc_view, "vbat").y, 16);

    // A move off the grid is staged (the FC takes it) and flagged.
    edit(&b, vec![mv("vbat", Some(60), Some(6), None)], None).unwrap();
    let off = working(&b);
    assert!(off.profiles[0]
        .problems
        .iter()
        .any(|p| p.kind == ProblemKind::OffScreen && p.element == "vbat"));
}

#[test]
fn toggle_and_copy_a_profile() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    let v = working(&b);
    let on_in_1: Vec<_> = v.profiles[0].elements.clone();
    // Turn vbat off in profile 3 only.
    edit(&b, vec![mv("vbat", None, None, Some(vec![1, 2]))], None).unwrap();
    let t = working(&b);
    assert!(!t.profiles[2].elements.contains(&"vbat".to_string()));
    assert!(t.profiles[0].elements.contains(&"vbat".to_string()));
    // Copy profile 1 onto 3: profile 3 then shows exactly what 1 shows.
    edit(&b, vec![], Some(OsdCopy { from: 1, to: 3 })).unwrap();
    let c = working(&b);
    assert_eq!(c.profiles[2].elements, c.profiles[0].elements);
    assert_eq!(c.profiles[0].elements, on_in_1);
    // Profile 1 is unchanged; the change is still one change, with its edits only where
    // the FC differs.
    assert_eq!(open_changes(&b).len(), 1);
    // Copying again changes nothing and keeps the change as it was.
    let again = edit(&b, vec![], Some(OsdCopy { from: 1, to: 3 })).unwrap();
    assert_eq!(again.edits, open_changes(&b)[0].edits);
    assert!(edit(&b, vec![], None).is_err());
}

#[test]
fn refusals_and_the_apply() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    for bad in [
        mv("vbat", Some(64), None, None),
        mv("vbat", None, Some(32), None),
        mv("vbat", None, None, Some(vec![4])),
        mv("no_such_element", Some(1), Some(1), None),
    ] {
        assert!(edit(&b, vec![bad.clone()], None).is_err(), "{bad:?}");
    }
    assert!(open_changes(&b).is_empty());
    let c = edit(&b, vec![mv("vbat", Some(20), Some(9), None)], None).unwrap();
    let p = b
        .core
        .gear_apply_plan(&ApplyPlanParams {
            id: c.id.clone(),
            port: None,
        })
        .unwrap();
    let r = b
        .core
        .gear_apply(&ApplyRequest {
            id: c.id,
            digest: p.digest,
            confirm: true,
            port: None,
        })
        .unwrap();
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    let held: u16 = fc
        .saved_value(Section::Master, "osd_vbat_pos")
        .unwrap()
        .parse()
        .unwrap();
    let pos = Pos::decode(held);
    assert_eq!((pos.x, pos.y, pos.profiles), (20, 9, 7));
    // After the apply the change is done: the next edit starts a new one.
    let next = edit(&b, vec![mv("vbat", Some(21), None, None)], None).unwrap();
    assert_eq!(next.status, ChangeStatus::Ready);
}

#[test]
fn the_row_and_the_mcp_action() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    let v = b
        .core
        .dispatch(
            "gear_osd_edit",
            json!({"device": b.id, "moves": [{"element": "vbat", "x": 30, "y": 4}]}),
        )
        .unwrap();
    assert_eq!(v["title"], "OSD layout");
    assert_eq!(v["edits"][0]["kind"], "osd_element");

    let mut s = Server::new(LocalBackend(b.core.clone()));
    let r = s.call_tool(
        "quadcam_gear_edit",
        json!({"action": "osd_edit", "device": b.id,
               "moves": [{"element": "rssi", "profiles": [2]}],
               "copy": {"from": 2, "to": 3}}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("Staged."), "{text}");
    assert_eq!(open_changes(&b).len(), 1);
    assert_eq!(
        open_changes(&b)[0].editor,
        quadcam_lib::session::Editor::User
    );
    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "osd", "device": b.id, "staged": true}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("staged OSD"));
}

#[test]
fn the_cli_stages_a_move() {
    // Edits arrive as the typed enum: the kind tag is what the CLI, MCP and app all send.
    let e: Edit = serde_json::from_value(json!(
        {"kind": "osd_element", "element": "vbat", "x": 1, "y": 2, "profiles": [1]}
    ))
    .unwrap();
    assert!(matches!(e, Edit::OsdElement { x: 1, y: 2, .. }));
}
