//! Running a module's tool: only as a child process, by absolute path, with an argv list
//! (never a shell), a fixed working folder, a minimal environment, a timeout, and its output
//! captured. Before a tool runs, its file's hash is checked against `installed.json`.

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Lowercase hex SHA-256 of a file.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// What identifies a file's state without reading it: size, inode, change and modify times.
type Stamp = (u64, u64, i64, i64, i64, i64);

fn stamp(path: &Path) -> Result<Stamp> {
    let m = std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;
    Ok((
        m.len(),
        m.ino(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    ))
}

/// Files whose hash matched, with the state they had then. A file is hashed again only when
/// its state changes, so a check before each run costs a `stat`.
static VERIFIED: Mutex<Option<HashMap<PathBuf, (Stamp, String)>>> = Mutex::new(None);

/// Checks that `path` still has the SHA-256 `expected`. `tool` names it in the refusal.
pub fn verify(tool: &str, path: &Path, expected: &str) -> Result<()> {
    let st = stamp(path).with_context(|| format!("{tool} is missing; reinstall it."))?;
    if let Some((s, h)) = VERIFIED
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .get(path)
    {
        if *s == st && h == expected {
            return Ok(());
        }
    }
    let got = sha256_file(path)?;
    if got != expected {
        bail!("{tool} was changed after install; reinstall it.");
    }
    VERIFIED
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(path.to_path_buf(), (st, got));
    Ok(())
}

/// Runs a program and returns its output. Tests swap it.
pub trait Runner: Send + Sync {
    fn run(
        &self,
        program: &Path,
        args: &[OsString],
        cwd: &Path,
        timeout: Duration,
    ) -> Result<Output>;
}

/// A child process with a minimal environment.
pub struct Process;

/// The environment a module sees: a fixed PATH, HOME, and a UTF-8 locale.
fn minimal_env() -> Vec<(&'static str, OsString)> {
    let mut env = vec![
        ("PATH", OsString::from("/usr/bin:/bin:/usr/sbin:/sbin")),
        ("LANG", OsString::from("en_US.UTF-8")),
    ];
    if let Some(h) = std::env::var_os("HOME") {
        env.push(("HOME", h));
    }
    env
}

impl Runner for Process {
    fn run(
        &self,
        program: &Path,
        args: &[OsString],
        cwd: &Path,
        timeout: Duration,
    ) -> Result<Output> {
        if !program.is_absolute() {
            bail!("{} is not an absolute path", program.display());
        }
        let mut child = Command::new(program)
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .envs(minimal_env())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("starting {}", program.display()))?;
        let pipe = |r: Option<Box<dyn Read + Send>>| {
            std::thread::spawn(move || {
                let mut v = Vec::new();
                if let Some(mut r) = r {
                    let _ = r.read_to_end(&mut v);
                }
                v
            })
        };
        let out = pipe(
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>),
        );
        let err = pipe(
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>),
        );
        let start = Instant::now();
        let status = loop {
            if let Some(s) = child.try_wait()? {
                break s;
            }
            if start.elapsed() > timeout {
                let _ = child.kill();
                let _ = child.wait();
                bail!(
                    "{} did not finish in {} s; it was stopped.",
                    program.display(),
                    timeout.as_secs()
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        Ok(Output {
            status,
            stdout: out.join().unwrap_or_default(),
            stderr: err.join().unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_refuses_a_changed_file() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("tool");
        std::fs::write(&f, b"one").unwrap();
        let h = sha256_file(&f).unwrap();
        verify("tool", &f, &h).unwrap();
        verify("tool", &f, &h).unwrap();
        std::fs::write(&f, b"two").unwrap();
        let e = verify("tool", &f, &h).unwrap_err();
        assert_eq!(
            e.to_string(),
            "tool was changed after install; reinstall it."
        );
    }

    #[test]
    fn process_runs_with_a_minimal_environment_and_a_timeout() {
        let d = tempfile::tempdir().unwrap();
        let out = Process
            .run(
                Path::new("/usr/bin/env"),
                &[],
                d.path(),
                Duration::from_secs(10),
            )
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.contains("PATH=/usr/bin:/bin:/usr/sbin:/sbin"),
            "{text}"
        );
        assert!(!text.contains("CARGO_MANIFEST_DIR"), "{text}");
        let e = Process
            .run(
                Path::new("/bin/sleep"),
                &["5".into()],
                d.path(),
                Duration::from_millis(200),
            )
            .unwrap_err();
        assert!(e.to_string().contains("was stopped"), "{e}");
        assert!(Process
            .run(Path::new("sleep"), &[], d.path(), Duration::from_secs(1))
            .is_err());
    }
}
