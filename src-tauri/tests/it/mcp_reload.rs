//! `quadcam-cli mcp` switches to a newer build on disk without ending the agent's session.
//! Each test runs a copy of the binary from a temp folder, then replaces that copy the way an
//! update does (a new file renamed over it). A test build file (`QUADCAM_MCP_TEST_BUILD`,
//! honored only under cargo) gives the new copy another version and one more tool.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

struct Mcp {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<Value>,
}

impl Mcp {
    /// Starts `bin mcp` with HOME `home`, looking at its file every 100 ms.
    fn start(bin: &Path, home: &Path, build: &Path) -> Mcp {
        let mut child = Command::new(bin)
            .env("HOME", home)
            .env("QUADCAM_PHOTOS", "dry-run")
            .env("CARGO_MANIFEST_DIR", env!("CARGO_MANIFEST_DIR"))
            .env("QUADCAM_MCP_TEST_BUILD", build)
            .env("QUADCAM_MCP_RELOAD_MS", "100")
            .env_remove("QUADCAM_MCP_RELOAD")
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let v: Value = serde_json::from_str(&line).expect("stdout is JSON-RPC only");
                if tx.send(v).is_err() {
                    break;
                }
            }
        });
        Mcp {
            child,
            stdin,
            lines,
        }
    }

    fn send(&mut self, m: Value) {
        writeln!(self.stdin, "{m}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn next(&self, within: Duration) -> Option<Value> {
        self.lines.recv_timeout(within).ok()
    }

    /// Sends a request and returns its answer.
    fn ask(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let r = self.next(Duration::from_secs(30)).expect("an answer");
        assert_eq!(r["id"], id, "{r}");
        r
    }

    fn handshake(&mut self) -> Value {
        let init = self.ask(
            1,
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}),
        );
        self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        init
    }

    fn tool_names(&mut self, id: u64) -> Vec<String> {
        self.ask(id, "tools/list", json!({}))["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A temp folder with a copy of the CLI in `bin/`, and a short path (the control socket's
/// path must fit in 104 bytes).
fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
    std::fs::create_dir(dir.path().join("bin")).unwrap();
    let bin = dir.path().join("bin/quadcam-cli");
    std::fs::copy(env!("CARGO_BIN_EXE_quadcam-cli"), &bin).unwrap();
    let build = dir.path().join("build.json");
    (dir, bin, build)
}

/// Puts a "new build" in place: the test build file says `version`, and a fresh copy of the
/// binary is renamed over the old one, as the updater replaces the bundle.
fn update(bin: &Path, build: &Path, version: &str) {
    std::fs::write(
        build,
        json!({"version": version, "extra_tool": true}).to_string(),
    )
    .unwrap();
    let next = bin.with_extension("new");
    std::fs::copy(env!("CARGO_BIN_EXE_quadcam-cli"), &next).unwrap();
    std::fs::rename(&next, bin).unwrap();
}

#[test]
fn the_session_survives_a_switch_to_a_newer_build() {
    let (dir, bin, build) = setup();
    let mut mcp = Mcp::start(&bin, dir.path(), &build);
    let init = mcp.handshake();
    assert_eq!(
        init["result"]["capabilities"]["tools"]["listChanged"], true,
        "{init}"
    );
    assert_eq!(
        init["result"]["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    let before = mcp.tool_names(2);
    assert!(!before.contains(&"quadcam_reload_test".to_string()));

    update(&bin, &build, "99.0.0");
    let note = mcp
        .next(Duration::from_secs(20))
        .expect("the new build says its tools changed");
    assert_eq!(note["method"], "notifications/tools/list_changed", "{note}");
    assert!(note.get("id").is_none());

    // The same process and pipes, now the new code: no second initialize.
    assert!(mcp.child.try_wait().unwrap().is_none());
    let after = mcp.tool_names(3);
    assert!(after.contains(&"quadcam_reload_test".to_string()));
    assert_eq!(after.len(), before.len() + 1);
    let status = mcp.ask(
        4,
        "tools/call",
        json!({"name": "quadcam_status", "arguments": {}}),
    );
    assert_eq!(status["result"]["isError"], false, "{status}");
    assert_eq!(status["result"]["structuredContent"]["mode"], "headless");

    // It sent the notification once, and nothing more comes unasked.
    assert!(mcp.next(Duration::from_millis(1500)).is_none());
}

#[test]
fn an_older_or_equal_build_never_takes_over() {
    let (dir, bin, build) = setup();
    let mut mcp = Mcp::start(&bin, dir.path(), &build);
    mcp.handshake();
    update(&bin, &build, "0.0.1");
    assert!(
        mcp.next(Duration::from_secs(3)).is_none(),
        "no notification"
    );
    let names = mcp.tool_names(2);
    assert!(!names.contains(&"quadcam_reload_test".to_string()));
}

/// A fake app on the control socket that answers `ping` at once and holds `places` until
/// the test lets it go.
fn fake_app(home: &Path) -> Arc<(Mutex<(bool, bool)>, Condvar)> {
    let dir = home.join("Library/Application Support/app.quadcam");
    std::fs::create_dir_all(&dir).unwrap();
    let listener = UnixListener::bind(dir.join("control.sock")).unwrap();
    // (a call is held, the test let it go)
    let gate = Arc::new((Mutex::new((false, false)), Condvar::new()));
    let g = gate.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let g = g.clone();
            std::thread::spawn(move || {
                let mut w = stream.try_clone().unwrap();
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    let m: Value = serde_json::from_str(&line).unwrap();
                    let result = if m["method"] == "places" {
                        let (lock, cv) = &*g;
                        let mut s = lock.lock().unwrap();
                        s.0 = true;
                        cv.notify_all();
                        while !s.1 {
                            s = cv.wait(s).unwrap();
                        }
                        json!([])
                    } else {
                        json!({})
                    };
                    let _ = writeln!(
                        w,
                        "{}",
                        json!({"jsonrpc": "2.0", "id": m["id"], "result": result})
                    );
                }
            });
        }
    });
    gate
}

#[test]
fn no_switch_while_a_request_is_in_flight() {
    let (dir, bin, build) = setup();
    let gate = fake_app(dir.path());
    let mut mcp = Mcp::start(&bin, dir.path(), &build);
    mcp.handshake();

    // A call the app holds: the server waits inside it.
    mcp.send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "quadcam_places", "arguments": {"action": "list"}}}));
    {
        let (lock, cv) = &*gate;
        let mut s = lock.lock().unwrap();
        while !s.0 {
            s = cv.wait_timeout(s, Duration::from_secs(30)).unwrap().0;
        }
    }
    update(&bin, &build, "99.0.0");
    // Well past the look interval and the settle time: still nothing, the old code is busy.
    assert!(mcp.next(Duration::from_secs(3)).is_none());

    {
        let (lock, cv) = &*gate;
        lock.lock().unwrap().1 = true;
        cv.notify_all();
    }
    // The answer comes first, from the old code, then the switch.
    let answer = mcp.next(Duration::from_secs(30)).expect("the held answer");
    assert_eq!(answer["id"], 2, "{answer}");
    assert_eq!(answer["result"]["isError"], false, "{answer}");
    let note = mcp
        .next(Duration::from_secs(20))
        .expect("the switch after the answer");
    assert_eq!(note["method"], "notifications/tools/list_changed", "{note}");
    assert!(mcp
        .tool_names(3)
        .contains(&"quadcam_reload_test".to_string()));
}
