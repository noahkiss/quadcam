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
    assert_eq!(tools.len(), 16);
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
        json!({"suggestions": [{"id": 0, "name": "Backyard Loops", "date": "2026-09-28", "reason": "trees in frame", "cuts": [{"start": 0.5, "end": 1.5}], "location": {"lat": 51.5, "lon": -0.1}, "keywords": ["park"]}, {"id": 1, "skip": true}]}),
    );
    let read = call(&mut s, "quadcam_read_clips", json!({"ids": [0]}));
    let c = &read["structuredContent"]["clips"][0];
    assert_eq!(c["cuts"], json!([{"start": 0.5, "end": 1.5}]));
    assert_eq!(c["agent_suggested"]["cuts"], true);
    assert_eq!(c["agent_suggested"]["meta"], true);
    assert_eq!(c["metadata"]["keywords"], json!(["park"]));
    assert_eq!(
        c["moments"],
        json!([]),
        "a clean clip with no log has no moments"
    );
    assert!(read["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("1 cuts"));
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
    let out = dir
        .path()
        .join("out/2026/2026-09-28/2026-09-28_backyard_loops.mp4");
    assert!(out.is_file());
    let tags = format_tags(&out);
    assert_eq!(
        tags["com.apple.quicktime.location.ISO6709"],
        "+51.5000-000.1000/"
    );
    assert_eq!(tags["com.apple.quicktime.keywords"], "FPV,park");
    let cut = dir
        .path()
        .join("out/2026/2026-09-28/2026-09-28_backyard_loops_cut1.mp4");
    assert!(cut.is_file());
    assert!(exp["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("_cut1.mp4"));
    let calls = rec.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec![(vec![out.clone(), cut.clone()], Some("Drone".to_string()))],
        "recorder only; nothing real; cuts go to Photos with their clip"
    );

    let v = call(&mut s, "quadcam_verify", json!({}));
    assert_eq!(v["structuredContent"]["reports"][0]["ok"], true);

    // The import is in the library, as the last import, with its cut.
    let lib = call(&mut s, "quadcam_library", json!({"group": "last_import"}));
    let clips = lib["structuredContent"]["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 1, "{lib}");
    assert_eq!(clips[0]["name"], "Backyard Loops");
    assert_eq!(clips[0]["cuts"].as_array().unwrap().len(), 1);
    let none = call(
        &mut s,
        "quadcam_library",
        json!({"query": "nothing-like-this"}),
    );
    assert_eq!(none["structuredContent"]["clips"], json!([]));

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
            .join("out/2026/2026-09-27/2026-09-27_dive_over_the_pond.mp4")
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

/// The GUI's core keeps its session in the session file: a relaunch restores it while its
/// staged clips exist, forgets it when they are gone, and Start over (`clear`) deletes it.
#[test]
fn session_survives_a_relaunch() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("cache/session.json");
    let open = || {
        let c = Core::new(
            dir.path().join("cache"),
            Some(file.clone()),
            Arc::new(MockGui::default()),
            Arc::new(Recorder::default()),
        );
        c.forget_unrestorable();
        c
    };
    let src = clips_folder();
    let first = open();
    assert!(first.session().is_none(), "nothing saved yet");
    first.load(Some(src.path())).unwrap();
    first
        .patch(
            &[PlanPatch {
                id: 0,
                name: Some("Backyard".into()),
                ..Default::default()
            }],
            Editor::User,
        )
        .unwrap();
    drop(first);

    // Relaunch: the session and the person's edits come back.
    let second = open();
    let s = second.session().expect("restored");
    assert_eq!(s.clips.len(), 2);
    assert_eq!(s.plans[0].name, "Backyard");

    // Start over: the session and its file are gone, and the next launch starts empty.
    second.dispatch("clear", Value::Null).unwrap();
    assert!(second.session().is_none());
    assert!(!file.exists());
    assert!(open().session().is_none());
    second.clear().unwrap();

    // Staged clips removed from the cache: the next launch starts empty, not broken.
    let third = open();
    third.load(Some(src.path())).unwrap();
    let staging = third.session().unwrap().staging;
    std::fs::remove_dir_all(&staging).unwrap();
    assert!(file.is_file());
    assert!(open().session().is_none());
}

/// The app quits and relaunches while the MCP server keeps running: the server falls back to
/// headless while the app is gone, then reaches the new app, never a dead connection.
#[test]
fn survives_an_app_restart() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("support/control.sock");
    std::fs::create_dir_all(sock.parent().unwrap()).unwrap();

    // The first app answers one connection, then quits: its socket file stays behind.
    let first = core(dir.path(), Arc::new(MockGui::default()), Arc::default());
    let listener = UnixListener::bind(&sock).unwrap();
    let app1 = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut out = stream.try_clone().unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        writeln!(out, "{}", control::respond(&first, &line)).unwrap();
    });
    let mut b = AutoBackend::new(sock.clone(), Some(dir.path().join("headless.json")));
    assert_eq!(b.mode(), "app");
    app1.join().unwrap();

    // App gone: the call runs headless instead of failing with a broken pipe.
    let st = b.call("status", Value::Null).unwrap();
    assert_eq!(st["gui"], false);

    // App relaunched on the same path: the next call reaches it.
    let gui = Arc::new(MockGui::default());
    let second = core(dir.path(), gui, Arc::default());
    control::serve(second.clone(), &sock).unwrap();
    let st = b.call("status", Value::Null).unwrap();
    assert_eq!(st["gui"], true);
    assert_eq!(b.mode(), "app");
    let mut s = Server::new(b);
    let st = call(&mut s, "quadcam_status", json!({}));
    assert_eq!(st["structuredContent"]["mode"], "app");
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
    assert!(card.is_attached() && !card.is_mounted());
}

/// The library and setup tools, headless: a time at suggest, then rating, renaming, a new
/// day, a profile, cuts, Photos and the Trash through MCP alone.
#[test]
fn library_and_setup_tools() {
    let dir = tempfile::tempdir().unwrap();
    let rec = Arc::new(Recorder::default());
    let settings = dir.path().join("settings.json");
    let trash = dir.path().join("trash");
    let c = Core::new(
        dir.path().join("cache"),
        None,
        Arc::new(NoHooks),
        rec.clone(),
    )
    .with_settings(settings.clone())
    .with_trash(Arc::new(quadcam_lib::trash::DirTrash(trash.clone())));
    let mut s = Server::new(LocalBackend(Arc::new(c)));
    let lib = dir.path().join("lib");
    std::fs::create_dir(&lib).unwrap();

    call(
        &mut s,
        "quadcam_settings",
        json!({"action": "write", "values": {"output_dir": lib, "layout": "day"}}),
    );
    let bad = s.call_tool(
        "quadcam_settings",
        json!({"action": "write", "values": {"layout": "monthly"}}),
    );
    assert_eq!(bad["isError"], true);
    assert!(bad["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("year_day, day or flat"));
    let r = call(&mut s, "quadcam_settings", json!({"action": "read"}));
    assert_eq!(r["structuredContent"]["settings"]["layout"], "day");
    call(
        &mut s,
        "quadcam_places",
        json!({"action": "save", "name": "Golden Gate Park", "lat": 37.7694, "lon": -122.4862}),
    );
    call(
        &mut s,
        "quadcam_profiles",
        json!({"action": "save", "name": "Whoop", "fields": {"aircraft": "65 mm whoop", "camera_make": "Maker", "keywords": ["tinywhoop"]}, "default": true}),
    );
    call(
        &mut s,
        "quadcam_profiles",
        json!({"action": "save", "name": "Five", "fields": {"aircraft": "5-inch", "camera_make": "Other"}}),
    );
    let p = call(&mut s, "quadcam_profiles", json!({"action": "list"}));
    assert_eq!(p["structuredContent"]["default_profile"], "Whoop");
    let pl = call(&mut s, "quadcam_places", json!({"action": "list"}));
    assert!(pl["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Golden Gate Park"));

    let src = clips_folder();
    call(&mut s, "quadcam_load_clips", json!({"source": src.path()}));
    call(
        &mut s,
        "quadcam_suggest",
        json!({"suggestions": [
        {"id": 0, "name": "loops", "date": "2026-09-27", "time": "16:20", "place": "Golden Gate Park", "cuts": [{"start": 0.5, "end": 1.5}]},
        {"id": 1, "name": "gap", "date": "2026-09-27"}]}),
    );
    let read = call(&mut s, "quadcam_read_clips", json!({"ids": [0]}));
    assert_eq!(read["structuredContent"]["clips"][0]["time"], "16:20:00");
    call(&mut s, "quadcam_export", json!({}));
    let l = call(&mut s, "quadcam_library", json!({}));
    let clips = l["structuredContent"]["clips"].as_array().unwrap().clone();
    assert_eq!(clips.len(), 2);
    let loops = clips.iter().find(|c| c["name"] == "loops").unwrap();
    let gap = clips.iter().find(|c| c["name"] == "gap").unwrap();
    assert_eq!(loops["time"], "16:20");
    assert_eq!(loops["aircraft"], "Whoop");
    let id = loops["id"].as_str().unwrap();

    let e = call(
        &mut s,
        "quadcam_library_edit",
        json!({"ids": [id], "rating": 4, "flag": "pick", "name": "fence loops", "date": "2026-09-29", "profile": "Five", "note": "best pack"}),
    );
    let c = &e["structuredContent"]["clips"][0];
    assert_eq!(c["rating"], 4);
    assert_eq!(c["flag"], "pick");
    assert_eq!(c["date"], "2026-09-29");
    assert_eq!(c["time"], "16:20");
    assert_eq!(c["aircraft"], "Five");
    assert_eq!(c["note"], "best pack");
    let file = c["file"].as_str().unwrap();
    assert!(
        file.ends_with("lib/2026-09-29/2026-09-29_fence_loops.mp4"),
        "{file}"
    );
    let two = s.call_tool(
        "quadcam_library_edit",
        json!({"ids": [id, gap["id"]], "name": "x"}),
    );
    assert_eq!(two["isError"], true, "a name needs one id");

    let ask = s.call_tool(
        "quadcam_library_files",
        json!({"action": "cuts", "ids": [id], "cuts": []}),
    );
    assert_eq!(ask["isError"], true);
    assert!(ask["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("removed_cuts"));
    call(
        &mut s,
        "quadcam_library_files",
        json!({"action": "cuts", "ids": [id], "cuts": [], "removed_cuts": "trash"}),
    );
    let ph = call(
        &mut s,
        "quadcam_library_files",
        json!({"action": "photos", "ids": [id]}),
    );
    assert_eq!(
        ph["structuredContent"]["added"].as_array().unwrap().len(),
        1
    );
    call(
        &mut s,
        "quadcam_library_files",
        json!({"action": "trash", "ids": [gap["id"]]}),
    );
    let l = call(&mut s, "quadcam_library", json!({}));
    assert_eq!(l["structuredContent"]["total"], 1);
    call(
        &mut s,
        "quadcam_library_files",
        json!({"action": "rebuild"}),
    );
    assert!(
        std::fs::read_dir(&trash).unwrap().count() >= 2,
        "the cut and the clip"
    );
}
