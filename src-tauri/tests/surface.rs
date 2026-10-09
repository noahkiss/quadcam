//! The 1.0 surface guard. The MCP tools (names, `action` values, parameters) and the CLI
//! (commands, flags, positional arguments) are compared with the baselines in `tests/surface/`.
//! A removal or a rename fails unless `SCHEMA_VERSION` went up and `schema::DEPRECATIONS` lists
//! the item. An addition passes, and the test prints a reminder to update the baseline:
//!
//! ```text
//! QUADCAM_UPDATE_SURFACE=1 cargo test --test surface
//! ```

use quadcam_lib::schema::{self, Deprecation, Item, SCHEMA_VERSION};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

fn baseline_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/surface")
        .join(name)
}

// ----- the CLI surface, read from `--help` -----

fn help(path: &[String], home: &std::path::Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
        .env("HOME", home)
        .args(path)
        .arg("--help")
        .output()
        .unwrap();
    assert!(out.status.success(), "{path:?} --help");
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// One command's help, split into its subcommands, flags (with the values they take) and
/// positional arguments.
#[derive(Default, Debug)]
struct Parsed {
    subcommands: Vec<String>,
    /// `--flag`, `-s,--flag`, or `--flag VALUE`, and the values clap lists for it.
    flags: Vec<(String, Vec<String>)>,
    args: Vec<String>,
}

fn parse_help(text: &str) -> Parsed {
    let mut p = Parsed::default();
    let mut section = "";
    for line in text.lines() {
        if let Some(head) = line.strip_suffix(':').filter(|h| !h.contains(' ')) {
            section = match head {
                "Commands" => "commands",
                "Options" => "options",
                "Arguments" => "arguments",
                _ => "",
            };
            continue;
        }
        match section {
            "commands" => {
                if let Some(name) = line
                    .strip_prefix("  ")
                    .and_then(|l| l.split_whitespace().next())
                {
                    if name != "help" {
                        p.subcommands.push(name.to_string());
                    }
                }
            }
            "arguments" => {
                if let Some(a) = line
                    .strip_prefix("  ")
                    .and_then(|l| l.split_whitespace().next())
                {
                    p.args.push(a.to_string());
                }
            }
            "options" => {
                let trimmed = line.trim_start();
                // An option line starts with `-x, --long` or `--long`; its notes sit deeper.
                if !trimmed.starts_with('-') || line.len() - trimmed.len() > 6 {
                    continue;
                }
                let head = trimmed.split("  ").next().unwrap_or("");
                let mut parts = head.split(", ");
                let first = parts.next().unwrap_or("");
                let (short, long) = match parts.next() {
                    Some(l) => (Some(first), l),
                    None => (None, first),
                };
                let mut words = long.split_whitespace();
                let name = words.next().unwrap_or("").to_string();
                let takes_value = words.next().is_some();
                let mut flag = match short {
                    Some(s) => format!("{s},{name}"),
                    None => name,
                };
                if takes_value {
                    flag.push_str(" VALUE");
                }
                let values = trimmed
                    .split("[possible values: ")
                    .nth(1)
                    .and_then(|v| v.split(']').next())
                    .map(|v| v.split(", ").map(str::to_string).collect())
                    .unwrap_or_default();
                p.flags.push((flag, values));
            }
            _ => {}
        }
    }
    p
}

/// Every command's help, walked from the root. `--json` and `--session` are global: they are
/// listed once, on the root.
fn cli_surface() -> BTreeSet<String> {
    let home = tempfile::tempdir().unwrap();
    let mut out = BTreeSet::new();
    let mut todo: Vec<Vec<String>> = vec![vec![]];
    while let Some(path) = todo.pop() {
        let p = parse_help(&help(&path, home.path()));
        let name = path.join(" ");
        if !path.is_empty() {
            out.insert(format!("cli.command {name}"));
        }
        for (flag, values) in &p.flags {
            let long = flag.split(',').next_back().unwrap_or(flag);
            let long = long.split(' ').next().unwrap_or(long);
            if (long == "--json" || long == "--session") && !path.is_empty() {
                continue;
            }
            if matches!(long, "--help") || (path.is_empty() && long == "--version") {
                continue;
            }
            out.insert(format!("cli.flag {name} {flag}"));
            for v in values {
                out.insert(format!("cli.value {name} {long}={v}"));
            }
        }
        for a in &p.args {
            out.insert(format!("cli.arg {name} {a}"));
        }
        for s in &p.subcommands {
            let mut next = path.clone();
            next.push(s.clone());
            todo.push(next);
        }
    }
    out
}

// ----- the guard -----

/// Compares one surface with its baseline file, or writes the baseline when asked.
fn guard(file: &str, current: BTreeSet<String>, table: &[Deprecation]) {
    let path = baseline_path(file);
    let update = std::env::var_os("QUADCAM_UPDATE_SURFACE").is_some();
    let (base_version, base) = match std::fs::read_to_string(&path) {
        Ok(t) => schema::parse_baseline(&t),
        Err(_) if update => {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, schema::render(SCHEMA_VERSION, &current)).unwrap();
            return;
        }
        Err(e) => panic!(
            "{}: {e}; run with QUADCAM_UPDATE_SURFACE=1 to create it",
            path.display()
        ),
    };
    let report = schema::check((base_version, &base), &current, SCHEMA_VERSION, table);
    for item in &report.removed_allowed {
        eprintln!("surface: removed after deprecation: {item}");
    }
    if !report.added.is_empty() {
        eprintln!(
            "surface: {} new item(s) in {file}. Update the baseline: QUADCAM_UPDATE_SURFACE=1 cargo test --test surface\n  {}",
            report.added.len(),
            report.added.join("\n  ")
        );
    }
    assert!(
        report.ok(),
        "the {file} surface broke the 1.0 contract.\n\
         Removed or renamed with no schema bump and deprecation:\n  {}\n\
         Newly required:\n  {}\n\
         Keep the old item working and add the new one, or deprecate the old one in \
         schema::DEPRECATIONS, bump SCHEMA_VERSION in a later release, then update the baseline.",
        report.removed.join("\n  "),
        report.newly_required.join("\n  "),
    );
    if update {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, schema::render(SCHEMA_VERSION, &current)).unwrap();
    } else if report.added.is_empty() && report.removed_allowed.is_empty() {
        assert_eq!(
            base_version, SCHEMA_VERSION,
            "the baseline names another schema version"
        );
    }
}

#[test]
fn the_mcp_surface_keeps_the_baseline() {
    let tools = quadcam_lib::mcp::tools();
    guard("mcp.txt", schema::mcp_surface(&tools), schema::DEPRECATIONS);
}

#[test]
fn the_cli_surface_keeps_the_baseline() {
    guard("cli.txt", cli_surface(), schema::DEPRECATIONS);
}

#[test]
fn every_deprecation_names_a_live_item_until_its_removal() {
    // A deprecation points at something that still works, or at something the baseline
    // lists because a later version removed it.
    let live: BTreeSet<String> = schema::mcp_surface(&quadcam_lib::mcp::tools())
        .into_iter()
        .chain(cli_surface())
        .collect();
    for d in schema::DEPRECATIONS {
        assert!(
            d.since <= SCHEMA_VERSION,
            "{:?} is deprecated in the future",
            d.item
        );
        let named = live.iter().any(|l| d.item.covers(l));
        assert!(
            named || d.since < SCHEMA_VERSION,
            "{} is deprecated but not on the surface",
            d.item.describe()
        );
    }
}

// ----- the mechanism, on a made-up table -----

const TABLE: &[Deprecation] = &[
    Deprecation {
        item: Item::McpAction {
            tool: "quadcam_settings",
            action: "modules",
        },
        replacement: "quadcam_settings action=read",
        since: 1,
    },
    Deprecation {
        item: Item::CliCommand("clear"),
        replacement: "`quadcam-cli stage`, which starts a new session",
        since: 1,
    },
];

#[test]
fn a_removal_passes_only_after_a_bump_and_a_deprecation() {
    let current = schema::mcp_surface(&quadcam_lib::mcp::tools());
    let (v, base) =
        schema::parse_baseline(&std::fs::read_to_string(baseline_path("mcp.txt")).unwrap());
    let mut without = current.clone();
    without.remove("mcp.action quadcam_settings.modules");
    // Not deprecated: fails at any version.
    assert!(!schema::check((v, &base), &without, v + 1, &[]).ok());
    // Deprecated, but the version did not move: fails.
    assert!(!schema::check((v, &base), &without, v, TABLE).ok());
    // Deprecated at the baseline's own version, and a later version removes it: passes.
    assert!(schema::check((v, &base), &without, v + 1, TABLE).ok());
}

#[test]
fn the_server_warns_on_a_deprecated_action() {
    use quadcam_lib::core::{Core, NoHooks};
    use quadcam_lib::mcp::{LocalBackend, Server};
    use quadcam_lib::photos::Recorder;
    use serde_json::json;
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let core = Core::new(
        dir.path().join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(dir.path().join("settings.json"));
    let mut s = Server::new(LocalBackend(Arc::new(core))).with_deprecations(TABLE);
    let r = s.call_tool("quadcam_settings", json!({"action": "modules"}));
    assert_eq!(r["isError"], false, "{r}");
    let w = r["structuredContent"]["deprecations"][0].as_str().unwrap();
    assert!(
        w.contains("quadcam_settings action=modules") && w.contains("action=read"),
        "{w}"
    );
    let r = s.call_tool("quadcam_settings", json!({"action": "read"}));
    assert!(r["structuredContent"].get("deprecations").is_none());
}

#[test]
fn the_server_and_status_report_the_schema_version() {
    use quadcam_lib::core::{Core, NoHooks};
    use quadcam_lib::mcp::{LocalBackend, Server};
    use quadcam_lib::photos::Recorder;
    use serde_json::json;
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let core = Core::new(
        dir.path().join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(dir.path().join("settings.json"));
    let mut s = Server::new(LocalBackend(Arc::new(core)));
    let init = s
        .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}))
        .unwrap();
    assert_eq!(
        init["result"]["serverInfo"]["schemaVersion"],
        SCHEMA_VERSION
    );
    let status = s.call_tool("quadcam_status", json!({}));
    assert_eq!(
        status["structuredContent"]["schema_version"],
        SCHEMA_VERSION
    );
}

#[test]
fn the_cli_prints_the_schema_version_and_warns_on_a_deprecated_command() {
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
            .args(args)
            .output()
            .unwrap()
    };
    let out = run(&["--schema-version"]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        SCHEMA_VERSION.to_string()
    );
    let out = run(&["--json", "--schema-version"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["result"]["schema_version"], SCHEMA_VERSION);
    // The CLI's own table is empty at 1; the matcher is tested on TABLE.
    let argv: Vec<String> = ["--json", "clear"].iter().map(|s| s.to_string()).collect();
    assert_eq!(schema::cli_deprecations(TABLE, &argv).len(), 1);
    assert!(schema::cli_deprecations(TABLE, &["stage".to_string()]).is_empty());
}
