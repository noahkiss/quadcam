//! quadcam-cli end to end: every command with --json, exit codes, and the format guards.
//! Each test runs the binary with HOME set to a temp folder, so the cache, session file and
//! default output folder stay inside it. The only disk ever erased is a test disk image.

mod common;

use common::*;
use serde_json::Value;
use std::path::Path;
use std::process::Command;

struct Env {
    home: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        Env {
            home: tempfile::tempdir().unwrap(),
        }
    }

    /// Runs `quadcam-cli --json <args>`; returns (exit code, parsed JSON).
    fn run(&self, args: &[&str]) -> (i32, Value) {
        let out = Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
            .env("HOME", self.home.path())
            // Never reach the real Photos library from a test.
            .env("QUADCAM_PHOTOS", "dry-run")
            .arg("--json")
            .args(args)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let v: Value = serde_json::from_str(text.trim()).unwrap_or_else(|e| {
            panic!(
                "not JSON ({e}): {text} / {}",
                String::from_utf8_lossy(&out.stderr)
            )
        });
        (out.status.code().unwrap_or(-1), v)
    }

    fn ok(&self, args: &[&str]) -> Value {
        let (code, v) = self.run(args);
        assert_eq!(code, 0, "{args:?} -> {v}");
        assert_eq!(v["ok"], true);
        v["result"].clone()
    }
}

fn folder_with_clips() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("DCIM")).unwrap();
    make_clip(&d.path().join("DCIM/PICT0001.AVI"), 2, true);
    make_clip(&d.path().join("DCIM/PICT0002.AVI"), 1, false);
    std::fs::write(d.path().join("DCIM/PICT0003.AVI"), b"").unwrap();
    d
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn full_flow_on_a_folder() {
    let env = Env::new();
    let src = folder_with_clips();

    assert!(env.ok(&["cards"]).is_array());
    let scan = env.ok(&["scan", s(src.path())]);
    assert_eq!(scan["clips"].as_array().unwrap().len(), 3);

    let staged = env.ok(&["stage", s(src.path())]);
    assert_eq!(staged["clips"].as_array().unwrap().len(), 3);
    assert_eq!(staged["analysed"], false);
    let analysed = env.ok(&["analyze"]);
    assert_eq!(analysed["clips"][0]["status"], "ok");
    assert_eq!(analysed["clips"][2]["status"], "empty");
    assert_eq!(
        analysed["plans"][2]["skip"], true,
        "empty clips are skipped"
    );

    let dates = env.ok(&[
        "dates",
        "--no-logs",
        "--set",
        "0=2026-09-28",
        "--set",
        "1=2026-09-28",
    ]);
    assert_eq!(dates["plans"][0]["date"], "2026-09-28");
    assert_eq!(dates["plans"][0]["source"], "edited");

    // Bad input is a clean error, not a crash.
    let (code, v) = env.run(&["dates", "--set", "0=yesterday"]);
    assert_eq!(code, 1);
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("YYYY-MM-DD"));

    // Import to the default folder (~/Movies/quadcam under the temp HOME).
    let out = env.ok(&[
        "import",
        "--name",
        "0=Wake Up",
        "--note",
        "0=two packs",
        "--format",
        "mp4",
        "--add-to-photos",
        "--album",
        "",
    ]);
    let sum = &out["summary"];
    assert_eq!(sum["imported"], 2, "{out}");
    assert_eq!(sum["skipped"], 1);
    // The default layout files each clip under year and day.
    let default_out = env.home.path().join("Movies/quadcam/2026/2026-09-28");
    assert!(default_out.join("2026-09-28_wake_up.mp4").is_file());
    assert!(default_out.join("2026-09-28_flight-1.mp4").is_file());
    assert!(
        sum["format_ready"]["Err"]
            .as_str()
            .unwrap()
            .contains("folder"),
        "a folder never formats"
    );
    // QUADCAM_PHOTOS=dry-run: the recorder reports both files added, and nothing is.
    assert_eq!(
        out["photos"]["Ok"]["added"].as_array().unwrap().len(),
        2,
        "{out}"
    );

    // A second import converts nothing again.
    let again = env.ok(&["import", "--output", s(env.home.path())]);
    assert_eq!(again["summary"]["imported"], 2);
    assert_eq!(std::fs::read_dir(&default_out).unwrap().count(), 2);

    let v = env.ok(&["verify"]);
    assert_eq!(v.as_array().unwrap().len(), 2);
    assert!(v.as_array().unwrap().iter().all(|r| r["ok"] == true));

    let one = default_out.join("2026-09-28_wake_up.mp4");
    let r = env.ok(&[
        "verify",
        s(&one),
        "--source",
        s(&src.path().join("DCIM/PICT0001.AVI")),
    ]);
    assert_eq!(r["frames"], 2 * FPS);
    let (code, _) = env.run(&[
        "verify",
        s(&one),
        "--source",
        s(&src.path().join("DCIM/PICT0002.AVI")),
    ]);
    assert_eq!(code, 1, "a mismatched source fails verify");

    let p = env.ok(&[
        "photos",
        "--dry-run",
        s(&one),
        s(&src.path().join("DCIM/PICT0001.AVI")),
    ]);
    assert_eq!(p["report"]["added"].as_array().unwrap().len(), 1);
    assert_eq!(
        p["report"]["failed"].as_array().unwrap().len(),
        1,
        "an AVI is not offered to Photos"
    );

    let show = env.ok(&["show"]);
    assert_eq!(show["plans"][0]["name"], "Wake Up");

    // Format on a folder session: refused, exit 3.
    let (code, v) = env.run(&[
        "format",
        "--device",
        "/dev/disk99",
        "--volume-uuid",
        "X",
        "--yes",
    ]);
    assert_eq!(code, 3, "{v}");
    assert_eq!(v["error"]["code"], "refused");

    // Start over: `clear` deletes the session; `show` then has nothing to show.
    let gone = env.ok(&["clear"]);
    assert_eq!(gone["cleared"], true);
    let (code, _) = env.run(&["show"]);
    assert_eq!(code, 4);
}

#[test]
fn import_from_a_plan_file() {
    let env = Env::new();
    let src = folder_with_clips();
    env.ok(&["stage", s(src.path())]);
    env.ok(&["analyze"]);
    let out = tempfile::tempdir().unwrap();
    let plan = env.home.path().join("plan.json");
    std::fs::write(
        &plan,
        serde_json::json!({
            "clips": [{"id": 0, "name": "Park", "date": "2026-09-01"}, {"id": 1, "skip": true}],
            "format": "mov",
            "output_dir": out.path(),
        })
        .to_string(),
    )
    .unwrap();
    let r = env.ok(&["import", "--plan", s(&plan)]);
    assert_eq!(r["summary"]["imported"], 1);
    assert!(out
        .path()
        .join("2026/2026-09-01/2026-09-01_park.mov")
        .is_file());
}

#[test]
fn moments_and_cuts() {
    let env = Env::new();
    let src = folder_with_clips();
    env.ok(&["stage", s(src.path())]);
    env.ok(&["analyze"]);
    let m = env.ok(&["moments", "0"]);
    assert_eq!(m.as_array().unwrap().len(), 1);
    assert_eq!(m[0]["moments"], serde_json::json!([]));
    // No dead air in the clip, so there are no keep ranges to use.
    let (code, v) = env.run(&["cut", "0", "--keep"]);
    assert_eq!(code, 1, "{v}");
    let c = env.ok(&["cut", "0", "0.25-1", "0:01.2-0:02"]);
    assert_eq!(
        c["cuts"],
        serde_json::json!([{"start": 0.25, "end": 1.0}, {"start": 1.2, "end": 2.0}])
    );
    let (code, _) = env.run(&["cut", "0", "1.9-2.3"]);
    assert_eq!(code, 1, "too short once clamped to the clip");
    let out = tempfile::tempdir().unwrap();
    let r = env.ok(&[
        "import",
        "--skip",
        "1",
        "--cut",
        "0=0-0.75",
        "--output",
        s(out.path()),
    ]);
    let cuts = r["summary"]["results"][0]["cuts"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(cuts.len(), 3, "--cut adds to the cuts already set");
    for (i, c) in cuts.iter().enumerate() {
        assert_eq!(c["outcome"], "verified", "{c}");
        let name = format!("_flight-1_cut{}.mp4", i + 1);
        assert!(c["output"].as_str().unwrap().ends_with(&name), "{c}");
    }
    // Dropping exported cuts needs a decision about their files.
    let (code, v) = env.run(&["cut", "0", "--clear"]);
    assert_eq!(code, 1, "{v}");
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("removed_cuts"));
    let c = env.ok(&["cut", "0", "--clear", "--removed", "keep"]);
    assert_eq!(c["cuts"], serde_json::json!([]));
    let kept = cuts[0]["output"].as_str().unwrap();
    assert!(Path::new(kept).is_file(), "kept files stay");
}

#[test]
fn metadata_from_settings_profiles_and_places() {
    let env = Env::new();
    let support = env
        .home
        .path()
        .join("Library/Application Support/app.quadcam");
    std::fs::create_dir_all(&support).unwrap();
    std::fs::write(
        support.join("settings.json"),
        serde_json::json!({
            "places": [{"name": "Field", "lat": 40.68919, "lon": -74.04449}],
            "profiles": [{"name": "Whoop", "camera_make": "Maker", "author": "Pilot", "keywords": ["tinywhoop"]}],
            "defaultProfile": "Whoop",
        })
        .to_string(),
    )
    .unwrap();
    let src = folder_with_clips();
    env.ok(&["stage", s(src.path())]);
    env.ok(&["analyze"]);
    let p = env.ok(&["profiles"]);
    assert_eq!(p["default_profile"], "Whoop");
    assert_eq!(p["places"][0]["name"], "Field");
    let (code, v) = env.run(&["meta", "all", "--place", "Nope"]);
    assert_eq!(code, 1, "{v}");
    let m = env.ok(&[
        "meta",
        "all",
        "--place",
        "Field",
        "--keywords",
        "park, , park",
    ]);
    assert_eq!(m.as_array().unwrap().len(), 3);
    assert_eq!(m[0]["metadata"]["location"]["name"], "Field");
    assert_eq!(m[0]["metadata"]["keywords"], serde_json::json!(["park"]));
    env.ok(&[
        "meta",
        "1",
        "--location",
        "-33.8568,151.2153",
        "--author",
        "Guest",
    ]);
    let out = tempfile::tempdir().unwrap();
    let r = env.ok(&["import", "--output", s(out.path())]);
    let res = &r["summary"]["results"];
    let tags = format_tags(Path::new(res[0]["output"].as_str().unwrap()));
    assert_eq!(
        tags["com.apple.quicktime.location.ISO6709"],
        "+40.6892-074.0445/"
    );
    assert_eq!(tags["com.apple.quicktime.make"], "Maker");
    assert_eq!(tags["com.apple.quicktime.author"], "Pilot");
    assert_eq!(tags["com.apple.quicktime.keywords"], "FPV,tinywhoop,park");
    let tags = format_tags(Path::new(res[1]["output"].as_str().unwrap()));
    assert_eq!(
        tags["com.apple.quicktime.location.ISO6709"],
        "-33.8568+151.2153/"
    );
    assert_eq!(tags["com.apple.quicktime.author"], "Guest");
}

#[test]
fn errors_and_exit_codes() {
    let env = Env::new();
    let (code, v) = env.run(&["show"]);
    assert_eq!((code, v["error"]["code"].as_str()), (4, Some("no_session")));
    let (code, v) = env.run(&["import", "--format", "avi"]);
    assert_eq!((code, v["error"]["code"].as_str()), (2, Some("usage")));
    let (code, v) = env.run(&["format", "--yes"]);
    assert_eq!(code, 3, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("--device"));
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

#[test]
fn format_a_disk_image_only_with_every_flag() {
    let env = Env::new();
    let card = Image::create("64m", "QCCLI", false);
    std::fs::create_dir(card.mount.join("DCIM")).unwrap();
    make_clip(&card.mount.join("DCIM/PICT0001.AVI"), 1, true);

    let staged = env.ok(&["stage", s(&card.mount)]);
    assert_eq!(staged["card"]["whole_disk"], card.disk.as_str());
    env.ok(&["analyze"]);

    // Before import: locked.
    let (code, _) = env.run(&["format", "--plan"]);
    assert_eq!(code, 1);

    let out = tempfile::tempdir().unwrap();
    env.ok(&["import", "--output", s(out.path())]);
    let plan = env.ok(&["format", "--plan"]);
    let device = plan["device"].as_str().unwrap().to_string();
    let uuid = plan["volume_uuid"].as_str().unwrap().to_string();
    assert_eq!(device, format!("/dev/{}", card.disk));

    for args in [
        vec!["format", "--device", &device, "--volume-uuid", &uuid],
        vec!["format", "--device", &device, "--yes"],
        vec!["format", "--volume-uuid", &uuid, "--yes"],
        vec![
            "format",
            "--device",
            &device,
            "--volume-uuid",
            "00000000-0000-0000-0000-000000000000",
            "--yes",
        ],
        vec![
            "format",
            "--device",
            "/dev/disk0",
            "--volume-uuid",
            &uuid,
            "--yes",
        ],
        vec![
            "format",
            "--device",
            &format!("{device}s1"),
            "--volume-uuid",
            &uuid,
            "--yes",
        ],
    ] {
        let (code, v) = env.run(&args);
        assert_eq!(code, 3, "{args:?} -> {v}");
        assert!(card.is_attached(), "refused formats leave the card alone");
        assert!(card.mount.join("DCIM/PICT0001.AVI").is_file());
    }

    assert_is_test_image(&card);
    let done = env.ok(&[
        "format",
        "--device",
        &device,
        "--volume-uuid",
        &uuid,
        "--yes",
        "--label",
        "fpvcard",
    ]);
    assert_eq!(done["label"], "FPVCARD");
    assert!(
        card.is_attached() && !card.is_mounted(),
        "unmounted after the erase"
    );
}

/// `quadcam-cli mcp` over real stdio: only JSON-RPC on stdout, one reply per request.
#[test]
fn mcp_over_stdio() {
    use std::io::Write;
    let env = Env::new();
    let mut child = Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
        .env("HOME", env.home.path())
        .env("QUADCAM_PHOTOS", "dry-run")
        .arg("mcp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for m in [
            serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}}),
            serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "quadcam_status", "arguments": {}}}),
        ] {
            writeln!(stdin, "{m}").unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    let lines: Vec<Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).expect("stdout is JSON-RPC only"))
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(lines[1]["result"]["tools"].as_array().unwrap().len(), 19);
    assert_eq!(lines[2]["result"]["structuredContent"]["mode"], "headless");
}

#[test]
fn places_profiles_settings_and_times_from_the_cli() {
    let env = Env::new();
    let settings = env
        .home
        .path()
        .join("Library/Application Support/app.quadcam/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(&settings, r#"{"formatLabel":"ECHO","libView":"list"}"#).unwrap();

    let p = env.ok(&[
        "places",
        "save",
        "Statue of Liberty",
        "--location",
        "40.6892,-74.0445",
    ]);
    assert_eq!(p["place"]["name"], "Statue of Liberty");
    let (code, v) = env.run(&["places", "save", "Nowhere"]);
    assert_eq!(code, 1, "{v}");
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("lat and lon"));
    env.ok(&[
        "profiles",
        "save",
        "Whoop",
        "--aircraft",
        "65 mm whoop",
        "--camera-make",
        "Maker",
        "--keywords",
        "tinywhoop, fpv",
        "--place",
        "statue of liberty",
        "--models",
        "WHOOP A",
        "--default",
    ]);
    let pr = env.ok(&["profiles"]);
    assert_eq!(pr["default_profile"], "Whoop");
    assert_eq!(pr["profiles"][0]["place"], "Statue of Liberty");
    env.ok(&[
        "places",
        "save",
        "Statue of Liberty",
        "--rename",
        "Liberty Island",
    ]);
    assert_eq!(
        env.ok(&["profiles", "list"])["profiles"][0]["place"],
        "Liberty Island"
    );

    let out = tempfile::tempdir().unwrap();
    let st = env.ok(&[
        "settings",
        "set",
        &format!("output_dir={}", s(out.path())),
        "place_folders=true",
        "format=mp4",
    ]);
    assert_eq!(st["effective"]["place_folders"], true);
    let (code, v) = env.run(&["settings", "set", "format=avi"]);
    assert_eq!(code, 1, "{v}");
    let (code, v) = env.run(&["settings", "set", "colour=red"]);
    assert_eq!(code, 1, "{v}");
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unknown setting"));
    let file: Value = serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();
    assert_eq!(
        file["formatLabel"], "ECHO",
        "keys the CLI did not set survive"
    );
    assert_eq!(file["libView"], "list");
    assert_eq!(file["places"][0]["name"], "Liberty Island");

    // A manual time at import, then a new date from the library.
    let src = folder_with_clips();
    env.ok(&["stage", s(src.path())]);
    env.ok(&["analyze"]);
    env.ok(&[
        "dates",
        "--no-logs",
        "--set",
        "0=2026-09-27",
        "--set",
        "1=2026-09-27",
    ]);
    let (code, v) = env.run(&["import", "--time", "0=7:5pm"]);
    assert_eq!(code, 1, "{v}");
    let r = env.ok(&[
        "import", "--name", "0=loops", "--time", "0=18:30", "--skip", "1",
    ]);
    assert_eq!(r["summary"]["imported"], 1, "{r}");
    let lib = env.ok(&["library", "list"]);
    let c = &lib["clips"][0];
    assert_eq!(c["time"], "18:30");
    assert!(
        c["file"]
            .as_str()
            .unwrap()
            .contains("2026-09-27 Liberty Island"),
        "{c}"
    );
    let id = c["id"].as_str().unwrap().to_string();
    let e = env.ok(&[
        "library",
        "edit",
        &id,
        "--date",
        "2026-09-28",
        "--time",
        "09:15",
    ]);
    assert_eq!(e["date"], "2026-09-28");
    assert_eq!(e["time"], "09:15");
    assert!(out
        .path()
        .join("2026/2026-09-28 Liberty Island/2026-09-28_loops.mp4")
        .is_file());
    let (code, _) = env.run(&["library", "edit", &id]);
    assert_eq!(code, 1, "an edit with nothing to change is refused");

    let gone = env.ok(&["places", "delete", "liberty island"]);
    assert_eq!(gone["profiles_cleared"][0], "Whoop");
    env.ok(&["profiles", "delete", "Whoop"]);
    assert_eq!(env.ok(&["profiles"])["default_profile"], Value::Null);
}

#[test]
fn voice_studio_commands_list_sets_and_keep_the_key_out_of_every_answer() {
    let env = Env::new();
    let sets = env.ok(&["gear", "voice", "sets"]);
    let ids: Vec<&str> = sets["sets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    for want in [
        "edgetx", "quad", "heli", "plane", "glider", "extras", "easter", "sample",
    ] {
        assert!(ids.contains(&want), "{want} in {ids:?}");
    }
    // Under cargo the real Keychain is closed; the view says so instead of failing.
    assert_eq!(sets["key"]["set"], false);
    assert!(sets["key"]["problem"]
        .as_str()
        .unwrap()
        .contains("off in tests"));

    // `key set` reads the key from stdin, and no stream shows it.
    let key = "fake-key-0123456789abcdef";
    let mut child = Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
        .env("HOME", env.home.path())
        .args(["--json", "gear", "voice", "key", "set"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(
        &mut child.stdin.take().unwrap(),
        format!("{key}\n").as_bytes(),
    )
    .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(!out.status.success());
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(all.contains("off in tests"), "{all}");
    assert!(!all.contains(key), "{all}");

    // A render of sets without a reachable key says what to do.
    let (code, v) = env.run(&[
        "gear",
        "voice",
        "render",
        "--set",
        "quad",
        "--voice",
        "x",
        "--model",
        "m",
        "--dry-run",
    ]);
    assert_ne!(code, 0, "{v}");
}

#[test]
fn voice_delete_lists_the_radios_that_chose_the_pack_and_clears_their_choice() {
    let env = Env::new();
    let support = env
        .home
        .path()
        .join("Library/Application Support/app.quadcam");
    let pack = support.join("gear/voices/local-say-test");
    std::fs::create_dir_all(pack.join("SOUNDS/en")).unwrap();
    std::fs::write(pack.join("SOUNDS/en/armed.wav"), b"RIFF").unwrap();
    std::fs::write(
        pack.join("pack.json"),
        serde_json::to_vec(&serde_json::json!({
            "id": "local-say-test", "voice": "Test", "lang": "en", "provider": "say",
            "settings": {}, "lines": 1, "license": "", "attribution": "", "lines_csv_sha": "",
            "files": ["SOUNDS/en/armed.wav"]
        }))
        .unwrap(),
    )
    .unwrap();
    let gear = support.join("gear/gear.json");
    std::fs::write(
        &gear,
        serde_json::to_vec(&serde_json::json!({
            "devices": [
                {"id": "radio-a", "kind": "radio", "name": "Field radio"},
                {"id": "radio-b", "kind": "radio"},
                {"id": "radio-c", "kind": "radio", "name": "Spare"}
            ],
            "voice": {"chosen": {"radio-a": "local-say-test", "radio-b": "local-say-test", "radio-c": "other"}}
        }))
        .unwrap(),
    )
    .unwrap();

    // Without --yes: refused, naming the radios whose choice the delete clears.
    let (code, v) = env.run(&["gear", "voice", "delete", "local-say-test"]);
    assert_ne!(code, 0, "{v}");
    let e = v.to_string();
    assert!(
        e.contains("Radios that chose it: Field radio (radio-a), radio-b.")
            && e.contains("clears their choice"),
        "{e}"
    );
    assert!(pack.is_dir());

    let r = env.ok(&["gear", "voice", "delete", "local-say-test", "--yes"]);
    assert_eq!(
        r["radios"],
        serde_json::json!(["radio-a", "radio-b"]),
        "{r}"
    );
    assert!(!pack.exists());
    let g: Value = serde_json::from_slice(&std::fs::read(&gear).unwrap()).unwrap();
    assert_eq!(
        g["voice"]["chosen"],
        serde_json::json!({"radio-c": "other"}),
        "only the deleted pack's choices go"
    );
}
