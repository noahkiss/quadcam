//! OSD layouts (design 7.3): golden renders of two real dumps (an HD and an analog NTSC
//! quad, scrubbed to their OSD lines with made-up craft names), a synthetic PAL dump with an
//! overlap and an element off screen, a diff, a dump with an apply file on top, and the
//! three surfaces (the `api` row, the CLI, the MCP action).

use quadcam_lib::core::{Core, NoHooks, OsdParams};
use quadcam_lib::gear::osd::{self, Grid, OsdConfig, ProblemKind};
use quadcam_lib::mcp::{LocalBackend, Server};
use quadcam_lib::photos::Recorder;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/osd")
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

fn view(files: &[&str], grid: Option<&str>) -> osd::OsdView {
    let dir = tempfile::tempdir().unwrap();
    core(dir.path())
        .gear_osd(&OsdParams {
            paths: files.iter().map(|f| fixture(f)).collect(),
            device: None,
            grid: grid.map(str::to_string),
        })
        .unwrap()
}

#[test]
fn hd_dump_golden() {
    let v = view(&["hd.dump_all.txt"], None);
    assert_eq!(v.grid, Grid::hd());
    assert!(v.complete && v.ok && v.unknown.is_empty(), "{:?}", v.notes);
    assert_eq!(v.profiles[0].name, "FULL");
    assert!(v.profiles[0].active);
    insta::assert_snapshot!("hd_dump", osd::render_text(&v));
}

#[test]
fn ntsc_dump_golden() {
    let v = view(&["ntsc.dump_all.txt"], None);
    assert_eq!(v.grid, Grid::ntsc());
    assert!(v.complete && v.ok, "{:?}", v.profiles);
    insta::assert_snapshot!("ntsc_dump", osd::render_text(&v));
}

#[test]
fn pal_synthetic_golden_with_problems() {
    let v = view(&["pal-synthetic.dump_all.txt"], None);
    assert_eq!(v.grid, Grid::pal());
    assert!(!v.ok);
    let kinds = |i: usize| -> Vec<(ProblemKind, String)> {
        v.profiles[i]
            .problems
            .iter()
            .map(|p| (p.kind, p.element.clone()))
            .collect()
    };
    assert_eq!(kinds(0), vec![(ProblemKind::Overlap, "mah_drawn".into())]);
    assert_eq!(kinds(1), vec![(ProblemKind::OffScreen, "rssi_dbm".into())]);
    assert!(kinds(2).is_empty());
    assert!(v.profiles[1].active, "osd_profile = 2");
    assert_eq!(v.profiles[2].name, "", "an empty profile name");
    // altitude has no profile bits: listed, drawn nowhere.
    assert!(v
        .elements
        .iter()
        .any(|e| e.name == "altitude" && e.profiles.is_empty()));
    assert!(v
        .profiles
        .iter()
        .all(|p| !p.elements.contains(&"altitude".into())));
    insta::assert_snapshot!("pal_synthetic", osd::render_text(&v));
}

#[test]
fn the_same_layout_on_each_grid() {
    // The NTSC quad's layout drawn on PAL and HD fits; its horizon fits NTSC too.
    for g in ["PAL", "HD", "NTSC"] {
        let v = view(&["ntsc.dump_all.txt"], Some(g));
        assert_eq!(v.grid.name, g);
        assert_eq!(v.profiles[0].rows.len(), v.grid.height as usize);
        assert!(v.profiles[0]
            .rows
            .iter()
            .all(|r| r.chars().count() == v.grid.width as usize));
        assert!(v.ok, "{g}");
    }
    // The HD quad's layout on NTSC: the right column and the bottom rows fall off.
    let v = view(&["hd.dump_all.txt"], Some("NTSC"));
    assert!(!v.ok);
    let p = &v.profiles[0];
    assert!(p
        .problems
        .iter()
        .any(|x| x.kind == ProblemKind::OffScreen && x.element == "flymode"));
    let v = view(&["hd.dump_all.txt"], Some("60x22"));
    assert!(v.ok);
}

#[test]
fn a_diff_is_incomplete_and_an_apply_file_wins() {
    let v = view(&["ntsc.diff_all.txt"], None);
    assert!(!v.complete);
    assert!(v.notes.iter().any(|n| n.contains("firmware default")));
    let rows = |v: &osd::OsdView| -> Vec<Vec<String>> {
        v.profiles.iter().map(|p| p.rows.clone()).collect()
    };
    assert!(
        !v.profiles[0].active,
        "osd_profile is at its default, so the diff leaves it out"
    );
    assert_eq!(
        rows(&v),
        rows(&view(&["ntsc.dump_all.txt"], None)),
        "every element on in the dump differs from its default, so the diff lists it"
    );

    // An apply file on top of the dump moves the horizon up one row in every profile.
    let dir = tempfile::tempdir().unwrap();
    let dump = fixture("ntsc.dump_all.txt");
    let before = OsdConfig::parse([std::fs::read_to_string(&dump).unwrap().as_str()]);
    let mut ah = osd::Pos::decode(before.positions["ah"]);
    ah.y -= 1;
    let apply = dir.path().join("apply.cli");
    std::fs::write(&apply, format!("set osd_ah_pos = {}\nsave\n", ah.encode())).unwrap();
    let v = core(dir.path())
        .gear_osd(&OsdParams {
            paths: vec![dump, apply],
            device: None,
            grid: None,
        })
        .unwrap();
    let e = v.elements.iter().find(|e| e.name == "ah").unwrap();
    assert_eq!((e.x, e.y), (ah.x, ah.y));
    assert!(v.complete);
    assert_eq!(v.source, vec!["ntsc.dump_all.txt", "apply.cli"]);
}

#[test]
fn refusals() {
    let dir = tempfile::tempdir().unwrap();
    let c = core(dir.path());
    let err = |p: OsdParams| format!("{:#}", c.gear_osd(&p).unwrap_err());
    assert!(err(OsdParams::default()).contains("Pass a Betaflight dump"));
    assert!(err(OsdParams {
        device: Some("fc-unknown".into()),
        ..Default::default()
    })
    .starts_with("No device"));
    assert!(err(OsdParams {
        paths: vec![dir.path().to_path_buf()],
        ..Default::default()
    })
    .contains("is not a Betaflight dump"));
    assert!(err(OsdParams {
        paths: vec![fixture("ntsc.dump_all.txt")],
        grid: Some("SECAM".into()),
        ..Default::default()
    })
    .contains("is not a grid"));
}

#[test]
fn the_osd_row_cli_and_mcp_action() {
    let dir = tempfile::tempdir().unwrap();
    let c = core(dir.path());
    let v = c
        .dispatch(
            "gear_osd",
            json!({"paths": [fixture("hd.dump_all.txt")], "grid": "HD"}),
        )
        .unwrap();
    assert_eq!(v["grid"]["width"], 53);
    assert_eq!(v["profiles"].as_array().unwrap().len(), 3);

    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
            .args(args)
            .env("HOME", dir.path())
            .env("QUADCAM_PHOTOS", "dry-run")
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };
    let ntsc = fixture("ntsc.dump_all.txt");
    let ntsc = ntsc.to_str().unwrap();
    let (code, out) = run(&["--json", "gear", "osd", ntsc]);
    assert_eq!(code, 0, "{out}");
    let j: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(j["result"]["grid"]["name"], "NTSC");
    assert_eq!(j["result"]["ok"], true);
    let (code, out) = run(&["gear", "osd", ntsc, "--text", "--grid", "PAL"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("Betaflight"), "{out}");
    assert!(out.contains("Grid PAL (30x16)"));
    let (code, out) = run(&["--json", "gear", "osd", "fc-nothing"]);
    assert_eq!(code, 4, "{out}");

    let mut s = Server::new(LocalBackend(c));
    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "osd", "paths": [fixture("pal-synthetic.dump_all.txt")]}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("FAIL: mah_drawn overlaps current"), "{text}");
    assert_eq!(r["structuredContent"]["ok"], false);
    let r = s.call_tool("quadcam_gear", json!({"action": "osd"}));
    assert_eq!(r["isError"], true);
}
