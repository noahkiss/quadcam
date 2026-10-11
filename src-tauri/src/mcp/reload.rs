//! The server picks up a new QuadCam on disk without a restart of the agent. An update (the
//! app's own updater, or Homebrew) replaces the bundle, and with it the `quadcam-cli` file the
//! agent started. While the server waits for the next message and no job runs, it looks at
//! that file. When a newer build has settled there, the server runs it in its own process
//! (`exec`): the same pid, stdin and stdout, so the agent's session goes on. The new code
//! takes the session's state from `QUADCAM_MCP_HANDOFF` and sends
//! `notifications/tools/list_changed` when its tool list differs.
//!
//! The server reads one message, answers it, then reads the next, so no request is in flight
//! while it waits. It never switches with bytes of a message read but not yet handled.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The session's state for the new code, as JSON. Set only by the `exec`.
pub const HANDOFF_ENV: &str = "QUADCAM_MCP_HANDOFF";
/// Set: `quadcam-cli mcp` prints its `Probe` and exits.
pub const PROBE_ENV: &str = "QUADCAM_MCP_PROBE";
/// `0` or `off`: never switch to a new build.
pub const RELOAD_ENV: &str = "QUADCAM_MCP_RELOAD";
/// How often an idle server looks at the file, in ms (default 2000).
const INTERVAL_ENV: &str = "QUADCAM_MCP_RELOAD_MS";
/// Tests only (a process started by cargo): a JSON file `{version, extra_tool}` that stands
/// for another build of this binary. Read once, at start.
pub const TEST_BUILD_ENV: &str = "QUADCAM_MCP_TEST_BUILD";

/// The handoff format. A build that reads a newer format than the old process writes says so
/// in its probe; the old process switches only to a build that reads its own.
pub const HANDOFF_FORMAT: u32 = 1;

/// How long a new file must stay the same before it is probed: an update may still be
/// writing the bundle.
const SETTLE: Duration = Duration::from_secs(1);
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// What the old process hands the new code.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Handoff {
    pub format: u32,
    /// The protocol version `initialize` agreed on; None before `initialize`.
    pub protocol: Option<String>,
    /// The client sent `notifications/initialized`.
    pub initialized: bool,
    /// `tools_digest` of the tool list the client last saw.
    pub tools: String,
    /// The version that handed over.
    pub from: String,
}

/// What a build on disk says about itself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Probe {
    pub version: String,
    /// The handoff formats it reads.
    pub handoff: Vec<u32>,
    /// The MCP protocol versions it speaks.
    pub protocols: Vec<String>,
}

/// This build: its version, and the test build's stand-ins under cargo.
struct Build {
    version: String,
    extra_tool: bool,
}

fn build() -> &'static Build {
    static BUILD: OnceLock<Build> = OnceLock::new();
    BUILD.get_or_init(|| {
        let real = Build {
            version: env!("CARGO_PKG_VERSION").into(),
            extra_tool: false,
        };
        let Some(path) = std::env::var_os(TEST_BUILD_ENV) else {
            return real;
        };
        if std::env::var_os("CARGO_MANIFEST_DIR").is_none() {
            return real;
        }
        let Ok(v) = std::fs::read(path).map(|b| serde_json::from_slice::<Value>(&b)) else {
            return real;
        };
        let Ok(v) = v else { return real };
        Build {
            version: v["version"].as_str().unwrap_or(&real.version).into(),
            extra_tool: v["extra_tool"] == true,
        }
    })
}

/// The version this process reports.
pub fn version() -> &'static str {
    &build().version
}

/// The tool list this process serves: `tools()`, plus a test tool in a test build.
pub fn listed_tools() -> Value {
    let mut list = super::tools();
    if build().extra_tool {
        if let Some(a) = list.as_array_mut() {
            a.push(json!({
                "name": "quadcam_reload_test",
                "description": "A tool only a test build lists.",
                "inputSchema": {"type": "object"},
            }));
        }
    }
    list
}

/// A short hash of a tool list, to tell whether it changed.
pub fn tools_digest(tools: &Value) -> String {
    format!(
        "{:016x}",
        xxhash_rust::xxh64::xxh64(tools.to_string().as_bytes(), 0)
    )
}

/// This build's probe answer.
pub fn probe(protocols: &[&str]) -> Probe {
    Probe {
        version: version().into(),
        handoff: vec![HANDOFF_FORMAT],
        protocols: protocols.iter().map(|p| p.to_string()).collect(),
    }
}

/// The handoff the old process left, read once and removed, so no child sees it.
pub fn take_handoff() -> Option<Handoff> {
    let raw = std::env::var(HANDOFF_ENV).ok()?;
    std::env::remove_var(HANDOFF_ENV);
    match serde_json::from_str(&raw) {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("quadcam-cli mcp: ignoring a handoff it cannot read: {e}");
            None
        }
    }
}

/// `a` is a newer release than `b`: numeric `X.Y.Z`, compared part by part. Anything else is
/// not newer, so an unreadable version never switches.
pub fn newer(a: &str, b: &str) -> bool {
    fn parts(v: &str) -> Option<Vec<u64>> {
        let core = v.split(['-', '+']).next()?;
        core.split('.').map(|p| p.parse().ok()).collect()
    }
    match (parts(a), parts(b)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// A file's identity: a replaced file has a new inode, a rewritten one a new time or size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Identity {
    dev: u64,
    ino: u64,
    mtime: (i64, i64),
    size: u64,
}

fn identity(path: &std::path::Path) -> Option<Identity> {
    // Follows a symlink: Homebrew's `bin/quadcam-cli` points into the bundle.
    let m = std::fs::metadata(path).ok()?;
    Some(Identity {
        dev: m.dev(),
        ino: m.ino(),
        mtime: (m.mtime(), m.mtime_nsec()),
        size: m.len(),
    })
}

/// Watches the file this process started from.
pub struct Watch {
    path: PathBuf,
    interval: Option<Duration>,
    /// The file as this process found it at start, or after a build it turned down.
    known: Option<Identity>,
    /// A new file, and since when it has stayed the same.
    settling: Option<(Identity, Instant)>,
    /// A settled file that a probe said is newer and can take over.
    ready: Option<(Identity, Probe)>,
}

impl Watch {
    /// Watches `path`, looking every `interval` while idle; None never switches.
    pub fn new(path: PathBuf, interval: Option<Duration>) -> Self {
        let known = identity(&path);
        Self {
            path,
            interval,
            known,
            settling: None,
            ready: None,
        }
    }

    /// The running executable, unless `QUADCAM_MCP_RELOAD` is `0` or `off`.
    pub fn from_env() -> Self {
        let off = std::env::var(RELOAD_ENV)
            .map(|v| v == "0" || v.eq_ignore_ascii_case("off"))
            .unwrap_or(false);
        let path = std::env::current_exe().ok().map(|p| {
            if p.is_absolute() {
                p
            } else {
                std::env::current_dir().unwrap_or_default().join(p)
            }
        });
        let interval = std::env::var(INTERVAL_ENV)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2000);
        match path {
            Some(p) if !off => Self::new(p, Some(Duration::from_millis(interval))),
            _ => Self::new(PathBuf::new(), None),
        }
    }

    /// How long to wait for input before looking again; None: never look.
    pub fn interval(&self) -> Option<Duration> {
        self.interval
    }

    /// Looks at the file. Some: a newer build has settled there, reads this session's
    /// handoff and speaks `protocol` (the agreed version, if any).
    pub fn check(&mut self, protocol: Option<&str>) -> Option<&Probe> {
        self.interval?;
        let now = identity(&self.path)?;
        if Some(now) == self.known {
            self.settling = None;
            self.ready = None;
            return None;
        }
        if self.ready.as_ref().is_some_and(|(id, _)| *id == now) {
            return self.ready.as_ref().map(|(_, p)| p);
        }
        self.ready = None;
        match self.settling {
            Some((id, since)) if id == now && since.elapsed() >= SETTLE => {}
            Some((id, _)) if id == now => return None,
            _ => {
                self.settling = Some((now, Instant::now()));
                return None;
            }
        }
        self.settling = None;
        match self.run_probe() {
            Ok(p) => {
                let why = if !newer(&p.version, version()) {
                    Some(format!("{} is not newer than {}", p.version, version()))
                } else if !p.handoff.contains(&HANDOFF_FORMAT) {
                    Some(format!("{} cannot take over this session", p.version))
                } else if protocol.is_some_and(|v| !p.protocols.iter().any(|x| x == v)) {
                    Some(format!(
                        "{} does not speak protocol {}",
                        p.version,
                        protocol.unwrap_or("")
                    ))
                } else {
                    None
                };
                match why {
                    None => {
                        self.ready = Some((now, p));
                        self.ready.as_ref().map(|(_, p)| p)
                    }
                    Some(why) => {
                        eprintln!("quadcam-cli mcp: staying on {}: {why}", version());
                        self.known = Some(now);
                        None
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "quadcam-cli mcp: staying on {}: the new build did not answer: {e}",
                    version()
                );
                self.known = Some(now);
                None
            }
        }
    }

    /// Asks the build on disk what it is. An older build without the probe starts a server
    /// that reads no input and exits with nothing printed.
    fn run_probe(&self) -> Result<Probe, String> {
        let mut child = Command::new(&self.path)
            .arg("mcp")
            .env(PROBE_ENV, "1")
            .env_remove(HANDOFF_ENV)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if start.elapsed() > PROBE_TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("timed out".into());
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => return Err(e.to_string()),
            }
        }
        let mut out = String::new();
        if let Some(mut s) = child.stdout.take() {
            std::io::Read::read_to_string(&mut s, &mut out).map_err(|e| e.to_string())?;
        }
        serde_json::from_str(out.trim()).map_err(|_| "no probe answer".to_string())
    }

    /// Runs the build on disk in this process with `handoff`. Returns only when that fails;
    /// the file is then left alone until it changes again.
    pub fn exec(&mut self, handoff: &Handoff) -> std::io::Error {
        let err = Command::new(&self.path)
            .args(std::env::args_os().skip(1))
            .env(
                HANDOFF_ENV,
                serde_json::to_string(handoff).unwrap_or_default(),
            )
            .exec();
        self.known = self.ready.take().map(|(id, _)| id).or(self.known);
        err
    }
}

/// Waits up to `timeout` for input on fd `fd`. True: input (or end of input) is there.
pub fn readable(fd: i32, timeout: Duration) -> std::io::Result<bool> {
    let mut p = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    // SAFETY: one valid pollfd, and its count.
    let n = unsafe { libc::poll(&mut p, 1, ms) };
    match n {
        n if n > 0 => Ok(true),
        0 => Ok(false),
        _ => {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                Ok(false)
            } else {
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_newer_release_is_newer() {
        assert!(newer("0.18.0", "0.17.0"));
        assert!(newer("0.17.10", "0.17.9"));
        assert!(newer("1.0.0", "0.99.99"));
        assert!(!newer("0.17.0", "0.17.0"));
        assert!(!newer("0.16.2", "0.17.0"), "a downgrade never switches");
        assert!(!newer("garbage", "0.17.0"));
        assert!(!newer("0.18.0", ""));
        assert!(newer("0.18.0-beta.1", "0.17.0"));
    }

    #[test]
    fn a_handoff_round_trips() {
        let h = Handoff {
            format: HANDOFF_FORMAT,
            protocol: Some("2025-06-18".into()),
            initialized: true,
            tools: tools_digest(&json!([])),
            from: "0.17.0".into(),
        };
        let s = serde_json::to_string(&h).unwrap();
        assert_eq!(serde_json::from_str::<Handoff>(&s).unwrap(), h);
    }

    #[test]
    fn the_digest_follows_the_list() {
        let a = tools_digest(&super::super::tools());
        assert_eq!(a, tools_digest(&super::super::tools()));
        assert_ne!(a, tools_digest(&json!([])));
    }

    #[test]
    fn a_file_that_does_not_change_never_switches() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("quadcam-cli");
        std::fs::write(&f, "x").unwrap();
        let mut w = Watch::new(f.clone(), Some(Duration::from_millis(1)));
        assert!(w.check(None).is_none());
        // Replaced: it settles before anything runs, and a file that does not run is
        // turned down once, not probed on every look.
        std::fs::write(dir.path().join("new"), "y").unwrap();
        std::fs::rename(dir.path().join("new"), &f).unwrap();
        assert!(w.check(None).is_none(), "settling");
        assert!(w.settling.is_some());
        w.settling = w.settling.map(|(id, t)| (id, t - SETTLE));
        assert!(w.check(None).is_none(), "not a build");
        assert_eq!(w.known, identity(&f));
        assert!(w.check(None).is_none());
        assert!(w.settling.is_none());
    }

    #[test]
    fn off_never_looks() {
        let mut w = Watch::new(PathBuf::from("/nonexistent"), None);
        assert!(w.interval().is_none());
        assert!(w.check(None).is_none());
    }
}
