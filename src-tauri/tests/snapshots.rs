//! Shape snapshots: the MCP tool list, the `dispatch` methods, the CLI help, and the JSON of
//! a session and of the library after an import of synthetic clips. A refactor leaves them
//! unchanged; a deliberate shape change shows in review as a snapshot diff. The session and
//! library JSON also seed the UI's mocked IPC.

mod common;

use common::make_clip;
use quadcam_lib::core::{Core, ImportOptions, NoHooks};
use quadcam_lib::library::Filter;
use quadcam_lib::media::Format;
use quadcam_lib::metadata::Place;
use quadcam_lib::moments::Span;
use quadcam_lib::photos::Recorder;
use quadcam_lib::session::{Defaults, Editor, PlanPatch};
use quadcam_lib::trash::DirTrash;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

#[test]
fn mcp_tool_list() {
    insta::assert_json_snapshot!("mcp_tools", quadcam_lib::mcp::tools());
}

#[test]
fn dispatch_methods() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::new(
        dir.path().join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(dir.path().join("settings.json"));
    core.set_defaults(Defaults {
        output_dir: Some(dir.path().join("library")),
        ..Defaults::default()
    });
    // Params of the wrong type: every method either ignores them or refuses them, and none
    // of them is unknown.
    for m in Core::METHODS {
        if let Err(e) = core.dispatch(m, json!(42)) {
            assert!(!format!("{e:#}").contains("unknown method"), "{m}: {e:#}");
        }
    }
    let e = core.dispatch("nope", Value::Null).unwrap_err();
    assert!(format!("{e:#}").contains("unknown method"));
    insta::assert_json_snapshot!("dispatch_methods", Core::METHODS);
}

/// Every command's `--help`, in one text.
#[test]
fn cli_help() {
    let commands: &[&[&str]] = &[
        &[],
        &["cards"],
        &["scan"],
        &["stage"],
        &["analyze"],
        &["dates"],
        &["show"],
        &["clear"],
        &["moments"],
        &["profiles"],
        &["profiles", "list"],
        &["profiles", "save"],
        &["profiles", "delete"],
        &["profiles", "default"],
        &["places"],
        &["places", "list"],
        &["places", "search"],
        &["places", "save"],
        &["places", "delete"],
        &["settings"],
        &["settings", "show"],
        &["settings", "set"],
        &["meta"],
        &["cut"],
        &["import"],
        &["verify"],
        &["photos"],
        &["eject"],
        &["format"],
        &["library"],
        &["library", "list"],
        &["library", "rate"],
        &["library", "rebuild"],
        &["library", "rename"],
        &["library", "edit"],
        &["library", "cut"],
        &["library", "apply-name-format"],
        &["library", "trash"],
        &["library", "photos"],
        &["gear"],
        &["gear", "status"],
        &["gear", "devices"],
        &["gear", "devices", "list"],
        &["gear", "devices", "save"],
        &["gear", "devices", "forget"],
        &["gear", "fc"],
        &["gear", "fc", "identify"],
        &["gear", "fc", "read"],
        &["gear", "fc", "check"],
        &["gear", "fc", "notes"],
        &["gear", "fc", "usb"],
        &["mcp"],
    ];
    let home = tempfile::tempdir().unwrap();
    let mut text = String::new();
    for c in commands {
        let out = Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
            .env("HOME", home.path())
            .args(*c)
            .arg("--help")
            .output()
            .unwrap();
        assert!(out.status.success(), "{c:?}");
        text.push_str(&format!(
            "===== quadcam-cli {} --help\n{}\n",
            c.join(" "),
            String::from_utf8_lossy(&out.stdout).trim_end()
        ));
    }
    insta::assert_snapshot!("cli_help", text);
}

/// Replaces what changes from run to run: the temp folder, today's date, times, sizes,
/// import ids and content hashes. Shapes and every other value stay.
fn normalize(v: &mut Value, tmp: &[String], today: &str) {
    match v {
        Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                match k.as_str() {
                    "size" | "total_bytes" | "bytes" | "free" if x.is_number() => *x = json!(0),
                    "import" | "last_import" | "import_id" if x.is_string() => {
                        *x = json!("$IMPORT")
                    }
                    "creation_time" | "mtime" if x.is_string() => *x = json!("$TIME"),
                    // ffmpeg's version, in the probe's tags.
                    "software" if x.is_string() => *x = json!("$LAVF"),
                    _ => normalize(x, tmp, today),
                }
            }
        }
        Value::Array(a) => {
            // A QuickTime item: [key, value].
            if let [Value::String(k), Value::String(val)] = &mut a[..] {
                match k.as_str() {
                    "com.apple.quicktime.creationdate" => *val = "$TIME".into(),
                    "app.quadcam.import" => *val = "$IMPORT".into(),
                    _ => {}
                }
            }
            for x in a {
                normalize(x, tmp, today);
            }
        }
        Value::String(s) => *s = normalize_text(s, tmp, today),
        _ => {}
    }
}

fn normalize_text(s: &str, tmp: &[String], today: &str) -> String {
    let mut s = s.to_string();
    for t in tmp {
        s = s.replace(t.as_str(), "$TMP");
    }
    s = s
        .replace(&format!("{}/{today}", &today[..4]), "$YEAR/$TODAY")
        .replace(today, "$TODAY")
        .replace(env!("CARGO_PKG_VERSION"), "$VERSION");
    stamps(&hashes(&s))
}

/// Staging folder stamps (`YYYYMMDD-HHMMSS`) become `$STAMP`.
fn stamps(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        let w = &b[i..(i + 15).min(b.len())];
        if w.len() == 15
            && w[..8].iter().all(u8::is_ascii_digit)
            && w[8] == b'-'
            && w[9..].iter().all(u8::is_ascii_digit)
        {
            out.push_str("$STAMP");
            i += 15;
        } else {
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Content hashes (16 hex digits, after an optional `h` or `x` prefix) become `$HASH`.
fn hashes(s: &str) -> String {
    let is_hash = |w: &str| {
        w.len() >= 16
            && w.len() <= 18
            && w[w.len() - 16..].bytes().all(|c| c.is_ascii_hexdigit())
            && w[..w.len() - 16].bytes().all(|c| c == b'h' || c == b'x')
    };
    let mut out = String::new();
    let mut word = String::new();
    for ch in s.chars().chain(std::iter::once('\0')) {
        if ch.is_ascii_alphanumeric() {
            word.push(ch);
            continue;
        }
        if is_hash(&word) {
            out.push_str("$HASH");
        } else {
            out.push_str(&word);
        }
        word.clear();
        if ch != '\0' {
            out.push(ch);
        }
    }
    out
}

#[test]
fn session_and_library_json() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("library");
    std::fs::create_dir(&root).unwrap();
    let src = dir.path().join("card");
    std::fs::create_dir_all(src.join("DCIM")).unwrap();
    make_clip(&src.join("DCIM/PICT0001.AVI"), 3, true);
    make_clip(&src.join("DCIM/PICT0002.AVI"), 2, false);
    std::fs::write(src.join("DCIM/PICT0003.AVI"), b"").unwrap();
    let core = Core::new(
        dir.path().join("cache"),
        Some(dir.path().join("session.json")),
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_trash(Arc::new(DirTrash(dir.path().join("trash"))));
    core.set_defaults(Defaults {
        output_dir: Some(root.clone()),
        format: Format::Mp4,
        keep_originals: true,
        places: vec![Place {
            name: "Home field".into(),
            lat: 40.6892,
            lon: -74.0445,
        }],
        ..Defaults::default()
    });
    core.load(Some(&src)).unwrap();
    core.patch(
        &[
            PlanPatch {
                id: 0,
                name: Some("loops".into()),
                place: Some("Home field".into()),
                time: Some("18:30".into()),
                cuts: Some(vec![Span {
                    start: 0.5,
                    end: 2.0,
                }]),
                ..Default::default()
            },
            PlanPatch {
                id: 1,
                note: Some("windy".into()),
                keywords: Some(vec!["practice".into()]),
                ..Default::default()
            },
        ],
        Editor::Agent,
    )
    .unwrap();
    core.import(&ImportOptions::default()).unwrap();
    let id = core.library(&Filter::default()).unwrap().clips[0]
        .clip
        .id
        .clone();
    core.library_rate(std::slice::from_ref(&id), Some(4), None)
        .unwrap();

    let tmp: Vec<String> = [dir.path().to_path_buf()]
        .into_iter()
        .flat_map(|p| {
            let canon = std::fs::canonicalize(&p).unwrap_or(p.clone());
            [canon, p]
        })
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut session = serde_json::to_value(core.session().unwrap()).unwrap();
    normalize(&mut session, &tmp, &today);
    let mut library = serde_json::to_value(core.library(&Filter::default()).unwrap()).unwrap();
    normalize(&mut library, &tmp, &today);
    insta::with_settings!({sort_maps => true}, {
        insta::assert_json_snapshot!("session", session);
        insta::assert_json_snapshot!("library_view", library);
    });
    assert!(Path::new(&root).is_dir());
}

/// The tool list as it was before the schemas were derived from types. The derived list
/// must equal it; a deliberate change updates both on review.
#[test]
fn mcp_tool_list_matches_the_hand_written_one() {
    let before: Value =
        serde_json::from_str(include_str!("fixtures/mcp_tools_before.json")).unwrap();
    let now = quadcam_lib::mcp::tools();
    let (b, n) = (before.as_array().unwrap(), now.as_array().unwrap());
    assert_eq!(b.len(), n.len());
    let mut bad = Vec::new();
    for (b, n) in b.iter().zip(n) {
        for k in ["name", "description", "annotations", "inputSchema"] {
            if b[k] != n[k] {
                bad.push(format!(
                    "{} {k}:\nbefore {}\nnow    {}",
                    b["name"], b[k], n[k]
                ));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
