//! MCP server: protocol, the agent flow headless, the same flow against a "running app"
//! (a core served on a temp control socket with mock GUI hooks), and format through MCP on
//! a disk image with the GUI click denied and then approved. Photos is always a recorder.

mod common;

use anyhow::Result;
use common::*;
use quadcam_lib::control::{self, Client};
use quadcam_lib::core::{Core, FormatPlan, Hooks, NoHooks};
use quadcam_lib::mcp::{AutoBackend, Backend, LocalBackend, Server};
use quadcam_lib::photos::Recorder;
use quadcam_lib::session::{Editor, PlanPatch};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Stands in for the GUI: counts change events and answers the Erase dialog.
#[derive(Default)]
struct MockGui {
    approve: AtomicBool,
    asked: Mutex<Vec<FormatPlan>>,
    changes: AtomicUsize,
}

impl Hooks for MockGui {
    fn changed(&self) {
        self.changes.fetch_add(1, Ordering::SeqCst);
    }
    fn confirm_format(&self, plan: &FormatPlan) -> Result<()> {
        self.asked.lock().unwrap().push(plan.clone());
        if self.approve.load(Ordering::SeqCst) {
            Ok(())
        } else {
            anyhow::bail!("Refused: the user cancelled the erase in quadcam.")
        }
    }
    fn has_gui(&self) -> bool {
        true
    }
}

fn core(dir: &Path, hooks: Arc<dyn Hooks>, photos: Arc<Recorder>) -> Arc<Core> {
    let c = Core::new(dir.join("cache"), None, hooks, photos);
    let mut d = c.defaults();
    d.output_dir = Some(dir.join("out"));
    std::fs::create_dir_all(dir.join("out")).unwrap();
    c.set_defaults(d);
    Arc::new(c)
}

fn clips_folder() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    make_clip(&d.path().join("PICT0001.AVI"), 2, true);
    make_clip(&d.path().join("PICT0002.AVI"), 1, true);
    d
}

fn call<B: Backend>(s: &mut Server<B>, tool: &str, args: Value) -> Value {
    let r = s.call_tool(tool, args);
    assert_eq!(r["isError"], false, "{tool}: {r}");
    r
}

#[test]
fn protocol_and_tool_list() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Server::new(LocalBackend(core(
        dir.path(),
        Arc::new(NoHooks),
        Arc::default(),
    )));
    let init = s.handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}})).unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "quadcam");
    assert!(s
        .handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .is_none());

    let list = s
        .handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
        .unwrap();
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 10);
    for t in tools {
        assert!(t["name"].as_str().unwrap().starts_with("quadcam_"));
        assert_eq!(t["inputSchema"]["type"], "object");
        assert!(
            t["description"].as_str().unwrap().contains("Returns:"),
            "{}",
            t["name"]
        );
    }
    let fmt = tools
        .iter()
        .find(|t| t["name"] == "quadcam_format_card")
        .unwrap();
    assert_eq!(fmt["annotations"]["destructiveHint"], true);
    let read = tools
        .iter()
        .find(|t| t["name"] == "quadcam_read_clips")
        .unwrap();
    assert_eq!(read["annotations"]["readOnlyHint"], true);

    let bad = s.handle(&json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "quadcam_read_clips", "arguments": {}}})).unwrap();
    assert_eq!(
        bad["result"]["isError"], true,
        "no session yet is a tool error, not a protocol error"
    );
    assert!(bad["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("quadcam_load_clips"));
    let unknown = s
        .handle(&json!({"jsonrpc": "2.0", "id": 4, "method": "nope"}))
        .unwrap();
    assert_eq!(unknown["error"]["code"], -32601);
}

#[test]
fn headless_agent_flow() {
    let dir = tempfile::tempdir().unwrap();
    let rec = Arc::new(Recorder::default());
    let mut s = Server::new(LocalBackend(core(
        dir.path(),
        Arc::new(NoHooks),
        rec.clone(),
    )));
    let src = clips_folder();

    let st = call(&mut s, "quadcam_status", json!({}));
    assert_eq!(st["structuredContent"]["mode"], "headless");

    let loaded = call(&mut s, "quadcam_load_clips", json!({"source": src.path()}));
    assert_eq!(
        loaded["structuredContent"]["clips"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // Thumbnails come back as MCP image content the agent can look at.
    let read = call(&mut s, "quadcam_read_clips", json!({"thumbnails": true}));
    let images: Vec<&Value> = read["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["type"] == "image")
        .collect();
    assert_eq!(images.len(), 2);
    assert_eq!(images[0]["mimeType"], "image/jpeg");
    assert!(images[0]["data"].as_str().unwrap().len() > 1000);
    assert_eq!(read["structuredContent"]["clips"][0]["duration_s"], 2.0);

    call(
        &mut s,
        "quadcam_suggest",
        json!({"suggestions": [{"id": 0, "name": "Backyard Loops", "date": "2026-09-28", "reason": "trees in frame"}, {"id": 1, "skip": true}]}),
    );
    let read = call(&mut s, "quadcam_read_clips", json!({"ids": [0]}));
    let c = &read["structuredContent"]["clips"][0];
    assert_eq!(c["short_name"], "Backyard Loops");
    assert_eq!(c["agent_suggested"]["name"], true);
    assert_eq!(c["reason"], "trees in frame");
    assert!(read["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("agent-suggested"));

    let bad = s.call_tool(
        "quadcam_suggest",
        json!({"suggestions": [{"id": 7, "name": "x"}]}),
    );
    assert_eq!(bad["isError"], true);

    let exp = call(
        &mut s,
        "quadcam_export",
        json!({"format": "mp4", "add_to_photos": true, "album": "Drone"}),
    );
    let sum = &exp["structuredContent"]["summary"];
    assert_eq!(
        (sum["imported"].as_u64(), sum["skipped"].as_u64()),
        (Some(1), Some(1)),
        "{exp}"
    );
    let out = dir.path().join("out/2026-09-28_backyard_loops.mp4");
    assert!(out.is_file());
    let calls = rec.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec![(vec![out.clone()], Some("Drone".to_string()))],
        "recorder only; nothing real"
    );

    let v = call(&mut s, "quadcam_verify", json!({}));
    assert_eq!(v["structuredContent"]["reports"][0]["ok"], true);

    // Format on a folder session refuses, even with every argument.
    let f = s.call_tool(
        "quadcam_format_card",
        json!({"device": "/dev/disk99", "volume_uuid": "X", "confirm": true}),
    );
    assert_eq!(f["isError"], true);
}

/// The MCP server drives a "running app": suggestions land in the app's own session, the
/// person's edits win, and the agent reads the final values back.
#[test]
fn app_mode_shares_the_gui_session() {
    let dir = tempfile::tempdir().unwrap();
    let gui = Arc::new(MockGui::default());
    let app = core(dir.path(), gui.clone(), Arc::default());
    let sock = dir.path().join("support/control.sock");
    control::serve(app.clone(), &sock).unwrap();
    // A second app on the same socket refuses to take it over.
    assert!(control::serve(app.clone(), &sock).is_err());
    let mode = std::fs::metadata(&sock).unwrap().permissions();
    assert_eq!(
        std::os::unix::fs::PermissionsExt::mode(&mode) & 0o777,
        0o600
    );
    let dmode = std::fs::metadata(sock.parent().unwrap())
        .unwrap()
        .permissions();
    assert_eq!(
        std::os::unix::fs::PermissionsExt::mode(&dmode) & 0o777,
        0o700
    );

    let mut s = Server::new(AutoBackend::new(
        sock.clone(),
        Some(dir.path().join("headless.json")),
    ));
    let st = call(&mut s, "quadcam_status", json!({}));
    assert_eq!(st["structuredContent"]["mode"], "app");
    assert_eq!(st["structuredContent"]["status"]["gui"], true);

    let src = clips_folder();
    call(&mut s, "quadcam_load_clips", json!({"source": src.path()}));
    assert!(
        app.session().is_some(),
        "the app's session, not a separate one"
    );
    let before = gui.changes.load(Ordering::SeqCst);

    call(
        &mut s,
        "quadcam_suggest",
        json!({"suggestions": [{"id": 0, "name": "Dive", "date": "2026-09-27"}]}),
    );
    assert!(
        gui.changes.load(Ordering::SeqCst) > before,
        "the GUI is told to re-render"
    );
    let p = app.session().unwrap().plans[0].clone();
    assert!(p.suggested.name && p.suggested.date);

    // The person edits the name in the GUI.
    app.patch(
        &[PlanPatch {
            id: 0,
            name: Some("Dive over the pond".into()),
            ..Default::default()
        }],
        Editor::User,
    )
    .unwrap();
    let read = call(&mut s, "quadcam_read_clips", json!({}));
    let c = &read["structuredContent"]["clips"][0];
    assert_eq!(c["short_name"], "Dive over the pond");
    assert_eq!(c["agent_suggested"]["name"], false);
    assert_eq!(c["agent_suggested"]["date"], true);

    let exp = call(&mut s, "quadcam_export", json!({}));
    assert_eq!(exp["structuredContent"]["summary"]["imported"], 2);
    assert!(
        dir.path()
            .join("out/2026-09-27_dive_over_the_pond.mp4")
            .is_file(),
        "app defaults (output folder) apply"
    );
    assert!(
        !dir.path().join("headless.json").exists(),
        "nothing ran headless"
    );

    // Raw client: bad method is an error, ping answers.
    let mut c = Client::connect(&sock).unwrap();
    assert!(c.call("nope", Value::Null).is_err());
    assert_eq!(c.call("ping", Value::Null).unwrap()["gui"], true);
}

#[test]
fn falls_back_to_headless_without_an_app() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = AutoBackend::new(
        dir.path().join("none.sock"),
        Some(dir.path().join("s.json")),
    );
    assert_eq!(b.mode(), "headless");
}

/// Test-only check before any erase: the target is a disk image this test attached.
fn assert_is_test_image(img: &Image) {
    let whole = quadcam_lib::disk::info(&img.disk).unwrap();
    assert_eq!(
        whole.bus_protocol.as_deref(),
        Some("Disk Image"),
        "refusing to format a non-image disk"
    );
}

/// Format through MCP in app mode: every guard, explicit device + UUID + confirm, and the
/// person's click in the GUI.
#[test]
fn format_through_mcp_needs_the_gui_click() {
    let dir = tempfile::tempdir().unwrap();
    let gui = Arc::new(MockGui::default());
    let app = core(dir.path(), gui.clone(), Arc::default());
    let sock = dir.path().join("support/control.sock");
    control::serve(app.clone(), &sock).unwrap();
    let mut s = Server::new(AutoBackend::new(sock, None));

    let card = Image::create("64m", "QCMCP", false);
    make_clip(&card.mount.join("PICT0001.AVI"), 1, true);
    call(&mut s, "quadcam_load_clips", json!({"source": card.mount}));

    let early = s.call_tool("quadcam_format_card", json!({"dry_run": true}));
    assert_eq!(early["isError"], true, "locked before export");
    call(&mut s, "quadcam_export", json!({}));

    let plan =
        call(&mut s, "quadcam_format_card", json!({"dry_run": true}))["structuredContent"].clone();
    let (device, uuid) = (
        plan["device"].as_str().unwrap().to_string(),
        plan["volume_uuid"].as_str().unwrap().to_string(),
    );
    assert_eq!(device, format!("/dev/{}", card.disk));

    // Missing confirm, wrong UUID, wrong device: refused before the GUI is even asked.
    for args in [
        json!({"device": device, "volume_uuid": uuid}),
        json!({"device": device, "volume_uuid": "WRONG", "confirm": true}),
        json!({"device": "/dev/disk0", "volume_uuid": uuid, "confirm": true}),
    ] {
        let r = s.call_tool("quadcam_format_card", args.clone());
        assert_eq!(r["isError"], true, "{args}");
    }
    assert!(gui.asked.lock().unwrap().is_empty());

    // The person cancels in the GUI.
    let r = s.call_tool(
        "quadcam_format_card",
        json!({"device": device, "volume_uuid": uuid, "confirm": true}),
    );
    assert_eq!(r["isError"], true);
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("cancelled"));
    assert_eq!(gui.asked.lock().unwrap().len(), 1);
    assert!(card.is_attached() && card.mount.join("PICT0001.AVI").is_file());

    // The person clicks Erase.
    gui.approve.store(true, Ordering::SeqCst);
    assert_is_test_image(&card);
    let r = call(
        &mut s,
        "quadcam_format_card",
        json!({"device": device, "volume_uuid": uuid, "confirm": true}),
    );
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with("Erased"));
    assert!(!card.is_attached());
}
