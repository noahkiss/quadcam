//! The card health check: `diskutil verifyVolume` before a known card is backed up, and
//! `diskutil repairVolume` when the person asks after a failed check.
//!
//! Pulling a mounted card leaves its FAT file system dirty. macOS checks a dirty volume
//! when it mounts it, but says nothing. QuadCam checks a card it knows before it reads
//! it, shows the result, and logs each check per card in `<gear>/health/<device>.jsonl`.
//!
//! - Neither command needs admin on a removable or image volume (tested on a FAT32 disk
//!   image, 2026-10-07). Verify unmounts the volume, runs `fsck_msdos -n`, and mounts it
//!   again; it takes about 30 s over a radio's USB.
//! - A verify can be stopped: QuadCam ends `diskutil` and mounts the disk again. A repair
//!   is never stopped once it starts (`fsck_msdos -y` is writing).
//! - Every run goes through a `DiskRunner`. A process started by cargo gets one that
//!   refuses unless `QUADCAM_SERIAL=real`; tests pass `FakeDisk`.

use super::store::Store;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::VecDeque;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// `diskutil`.
pub const DISKUTIL: &str = "/usr/sbin/diskutil";

/// A verify or repair gives up after this long and says the card may need a reboot.
pub const TIMEOUT: Duration = Duration::from_secs(600);

/// What a `diskutil` run printed and how it ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOutput {
    /// The exit code; None when it was stopped or timed out.
    pub code: Option<i32>,
    /// stdout and stderr together.
    pub output: String,
    pub stopped: bool,
    pub timed_out: bool,
}

/// Runs `diskutil` with arguments. `stop` ends a run that may be stopped.
pub trait DiskRunner: Send + Sync {
    fn run(
        &self,
        args: &[String],
        stop: Option<&AtomicBool>,
        timeout: Duration,
    ) -> Result<RunOutput>;
}

/// The system's `diskutil`.
pub struct SystemDisk;

impl DiskRunner for SystemDisk {
    fn run(
        &self,
        args: &[String],
        stop: Option<&AtomicBool>,
        timeout: Duration,
    ) -> Result<RunOutput> {
        let mut child = std::process::Command::new(DISKUTIL)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("starting diskutil")?;
        let mut out = child.stdout.take().context("diskutil stdout")?;
        let mut err = child.stderr.take().context("diskutil stderr")?;
        let ro = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = out.read_to_string(&mut s);
            s
        });
        let re = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = err.read_to_string(&mut s);
            s
        });
        let start = Instant::now();
        let (code, stopped, timed_out) = loop {
            if let Some(st) = child.try_wait()? {
                break (st.code(), false, false);
            }
            if stop.is_some_and(|s| s.load(Ordering::SeqCst)) {
                let _ = child.kill();
                let _ = child.wait();
                break (None, true, false);
            }
            if start.elapsed() > timeout {
                let _ = child.kill();
                let _ = child.wait();
                break (None, false, true);
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let mut output = ro.join().unwrap_or_default();
        output.push_str(&re.join().unwrap_or_default());
        Ok(RunOutput {
            code,
            output,
            stopped,
            timed_out,
        })
    }
}

/// Refuses every run: the runner a process started by cargo gets.
pub struct NoDisk;

impl DiskRunner for NoDisk {
    fn run(&self, args: &[String], _: Option<&AtomicBool>, _: Duration) -> Result<RunOutput> {
        bail!(
            "Refused: diskutil {} is not run under cargo (set QUADCAM_SERIAL=real).",
            args.join(" ")
        )
    }
}

/// The runner to use: none under cargo unless `QUADCAM_SERIAL=real`.
pub fn system() -> Arc<dyn DiskRunner> {
    let real = std::env::var("QUADCAM_SERIAL").as_deref() == Ok("real");
    if std::env::var_os("CARGO_MANIFEST_DIR").is_some() && !real {
        Arc::new(NoDisk)
    } else {
        Arc::new(SystemDisk)
    }
}

/// A scripted `diskutil` for tests: each run takes the next answer (the last one repeats)
/// and records its arguments. With `wait_for_stop`, a run waits until it is stopped.
#[derive(Default)]
pub struct FakeDisk {
    pub answers: Mutex<VecDeque<RunOutput>>,
    pub calls: Mutex<Vec<Vec<String>>>,
    pub wait_for_stop: AtomicBool,
}

impl FakeDisk {
    /// Every verify and repair passes.
    pub fn ok() -> Self {
        Self::with(vec![RunOutput {
            code: Some(0),
            output: VERIFY_OK.into(),
            ..Default::default()
        }])
    }

    pub fn with(answers: Vec<RunOutput>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            ..Default::default()
        }
    }

    pub fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}

impl DiskRunner for FakeDisk {
    fn run(&self, args: &[String], stop: Option<&AtomicBool>, _: Duration) -> Result<RunOutput> {
        self.calls.lock().unwrap().push(args.to_vec());
        if self.wait_for_stop.load(Ordering::SeqCst) && args[0] == "verifyVolume" {
            let start = Instant::now();
            while !stop.is_some_and(|s| s.load(Ordering::SeqCst)) {
                if start.elapsed() > Duration::from_secs(10) {
                    bail!("fake verify never stopped");
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            return Ok(RunOutput {
                stopped: true,
                ..Default::default()
            });
        }
        let mut a = self.answers.lock().unwrap();
        let next = if a.len() > 1 {
            a.pop_front()
        } else {
            a.front().cloned()
        };
        next.context("no fake diskutil answer")
    }
}

/// `diskutil verifyVolume` on a good FAT volume (a disk image, 2026-10-07).
pub const VERIFY_OK: &str = "Started file system verification on disk13s1 (QCTEST)
Verifying file system
Volume was successfully unmounted
Performing fsck_msdos -n /dev/rdisk13s1
** /dev/rdisk13s1
** Phase 1 - Preparing FAT
** Phase 2 - Checking Directories
** Phase 3 - Checking for Orphan Clusters
Warning: 0 files, 65390 KiB free (32695 clusters)
File system check exit code is 0
Restoring the original state found as mounted
Finished file system verification on disk13s1 (QCTEST)
";

/// `diskutil verifyVolume` on the same image with a damaged FAT.
pub const VERIFY_FAILED: &str = "Started file system verification on disk13s1 (QCTEST)
Verifying file system
Volume was successfully unmounted
Performing fsck_msdos -n /dev/rdisk13s1
** /dev/rdisk13s1
** Phase 1 - Preparing FAT
** Phase 2 - Checking Directories
** Phase 3 - Checking for Orphan Clusters
Warning: Found orphan cluster(s)
Fix? no
Warning: Found 2048 orphaned clusters
Warning: 6 files, 61280 KiB free (30640 clusters)
File system check exit code is 206
Restoring the original state found as mounted
Error: -69845: File system verify or repair failed
Underlying error: 206
";

/// `diskutil repairVolume` fixing it.
pub const REPAIR_FIXED: &str = "Started file system repair on disk13s1 (QCTEST)
Checking file system and repairing if necessary and if possible
Volume was successfully unmounted
Performing fsck_msdos -y /dev/rdisk13s1
** /dev/rdisk13s1
** Phase 1 - Preparing FAT
** Phase 2 - Checking Directories
** Phase 3 - Checking for Orphan Clusters
Warning: Found orphan cluster(s)
Fix? yes
Warning: Marked 2048 clusters as free
Warning: 2 files, 65384 KiB free (32692 clusters)
Warning:
***** FILE SYSTEM WAS MODIFIED *****
File system check exit code is 0
Restoring the original state found as mounted
Finished file system repair on disk13s1 (QCTEST)
";

/// Which command a check ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    Verify,
    Repair,
}

/// How a check ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    /// The file system is fine (or the repair fixed it).
    Ok,
    /// The check found damage (or the repair could not fix it).
    Failed,
    /// Stopped before it finished.
    Stopped,
    /// `diskutil` could not run the check (busy, no such volume, timed out).
    Error,
}

/// One check of one card, as the log keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct CardCheck {
    /// `<device>-<time>`: a repair names the failed check it answers.
    pub id: String,
    pub device: String,
    pub kind: CheckKind,
    pub state: CheckState,
    pub at: DateTime<Utc>,
    pub seconds: f64,
    /// `fsck_msdos`' exit code, when it got that far.
    #[serde(default)]
    pub fsck_code: Option<i32>,
    /// A repair changed the file system.
    #[serde(default)]
    pub modified: bool,
    /// One line to show: what was found.
    pub summary: String,
    /// The `Warning:` and `Error:` lines.
    #[serde(default)]
    pub findings: Vec<String>,
}

/// Reads `diskutil verifyVolume` or `repairVolume` output.
pub fn parse(
    kind: CheckKind,
    run: &RunOutput,
) -> (CheckState, Option<i32>, bool, String, Vec<String>) {
    let fsck = run.output.lines().find_map(|l| {
        l.trim()
            .strip_prefix("File system check exit code is ")
            .and_then(|c| c.trim().parse::<i32>().ok())
    });
    let modified = run.output.contains("FILE SYSTEM WAS MODIFIED");
    let findings: Vec<String> = run
        .output
        .lines()
        .map(str::trim)
        .filter(|l| {
            (l.starts_with("Warning:") || l.starts_with("Error:"))
                && !l.trim_end_matches(':').eq("Warning")
                && !l.contains(" files, ")
        })
        .map(str::to_string)
        .collect();
    let first_error = run
        .output
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("Error"))
        .map(str::to_string);
    let (state, summary) = if run.stopped {
        (
            CheckState::Stopped,
            "Stopped before it finished.".to_string(),
        )
    } else if run.timed_out {
        (
            CheckState::Error,
            "diskutil did not finish in time; the card may need to be unplugged and the Mac restarted."
                .to_string(),
        )
    } else if run.code == Some(0) && fsck.unwrap_or(0) == 0 {
        (
            CheckState::Ok,
            match (kind, modified) {
                (CheckKind::Repair, true) => "Repaired.".to_string(),
                (CheckKind::Repair, false) => "Nothing to repair.".to_string(),
                (CheckKind::Verify, _) => "The file system is OK.".to_string(),
            },
        )
    } else if fsck.is_some_and(|c| c != 0) {
        (
            CheckState::Failed,
            match kind {
                CheckKind::Verify => "The file system has errors. Repair it.".to_string(),
                CheckKind::Repair => "The repair could not fix every error.".to_string(),
            },
        )
    } else {
        (
            CheckState::Error,
            first_error.unwrap_or_else(|| {
                format!(
                    "diskutil stopped with code {}.",
                    run.code.map(|c| c.to_string()).unwrap_or("none".into())
                )
            }),
        )
    };
    (state, fsck, modified, summary, findings)
}

/// Runs a verify or a repair on `target` (a mount point or `diskNsM`). A verify that is
/// stopped mounts `whole_disk` again (diskutil unmounts the volume while it checks).
pub fn check(
    runner: &dyn DiskRunner,
    device: &str,
    kind: CheckKind,
    target: &str,
    whole_disk: Option<&str>,
    stop: Option<&AtomicBool>,
) -> CardCheck {
    let at = Utc::now();
    let start = Instant::now();
    let verb = match kind {
        CheckKind::Verify => "verifyVolume",
        CheckKind::Repair => "repairVolume",
    };
    // A repair is never stopped once it starts.
    let stop = if kind == CheckKind::Verify {
        stop
    } else {
        None
    };
    let run = runner.run(&[verb.to_string(), target.to_string()], stop, TIMEOUT);
    let run = match run {
        Ok(r) => r,
        Err(e) => RunOutput {
            code: None,
            output: format!("Error: {e:#}"),
            ..Default::default()
        },
    };
    if run.stopped {
        if let Some(d) = whole_disk {
            let _ = runner.run(&["mountDisk".into(), d.to_string()], None, TIMEOUT);
        }
    }
    let (state, fsck_code, modified, summary, findings) = parse(kind, &run);
    CardCheck {
        id: format!("{device}-{}", at.format("%Y%m%dT%H%M%S%3f")),
        device: device.to_string(),
        kind,
        state,
        at,
        seconds: start.elapsed().as_secs_f64(),
        fsck_code,
        modified,
        summary,
        findings,
    }
}

/// The check log of a gear folder.
pub struct HealthLog {
    store: Store,
}

impl HealthLog {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// `<gear>/health/<device>.jsonl`.
    pub fn path(&self, device: &str) -> PathBuf {
        self.store
            .root()
            .join("health")
            .join(format!("{}.jsonl", super::store::safe(device)))
    }

    /// Adds a check to the device's log.
    pub fn append(&self, c: &CardCheck) -> Result<()> {
        use std::io::Write;
        let p = self.path(&c.device);
        std::fs::create_dir_all(p.parent().context("health log has no folder")?)?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&p)
            .with_context(|| format!("opening {}", p.display()))?;
        let mut line = serde_json::to_vec(c)?;
        line.push(b'\n');
        f.write_all(&line)?;
        f.sync_all()?;
        Ok(())
    }

    /// The device's checks, newest first. A line that does not parse is skipped.
    pub fn list(&self, device: &str) -> Vec<CardCheck> {
        let text = std::fs::read_to_string(self.path(device)).unwrap_or_default();
        let mut v: Vec<CardCheck> = text
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        v.reverse();
        v
    }

    /// The newest check.
    pub fn latest(&self, device: &str) -> Option<CardCheck> {
        self.list(device).into_iter().next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(code: i32, text: &str) -> RunOutput {
        RunOutput {
            code: Some(code),
            output: text.into(),
            ..Default::default()
        }
    }

    #[test]
    fn parses_real_diskutil_output() {
        let (s, f, m, sum, find) = parse(CheckKind::Verify, &out(0, VERIFY_OK));
        assert_eq!((s, f, m), (CheckState::Ok, Some(0), false));
        assert_eq!(sum, "The file system is OK.");
        assert!(find.is_empty(), "{find:?}");
        let (s, f, _, _, find) = parse(CheckKind::Verify, &out(1, VERIFY_FAILED));
        assert_eq!((s, f), (CheckState::Failed, Some(206)));
        assert!(
            find.iter().any(|l| l.contains("2048 orphaned clusters")),
            "{find:?}"
        );
        assert!(find.iter().any(|l| l.starts_with("Error: -69845")));
        let (s, _, m, sum, _) = parse(CheckKind::Repair, &out(0, REPAIR_FIXED));
        assert_eq!((s, m, sum.as_str()), (CheckState::Ok, true, "Repaired."));
        let (s, _, _, sum, _) = parse(
            CheckKind::Verify,
            &out(1, "Error: -69673: Unable to unmount volume for repair\n"),
        );
        assert_eq!(s, CheckState::Error);
        assert!(sum.contains("-69673"));
    }

    #[test]
    fn a_stopped_verify_mounts_the_disk_again_and_logs() {
        let fake = FakeDisk::ok();
        fake.wait_for_stop.store(true, Ordering::SeqCst);
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            s2.store(true, Ordering::SeqCst);
        });
        let c = check(
            &fake,
            "radio-1",
            CheckKind::Verify,
            "/Volumes/CARD",
            Some("disk9"),
            Some(&stop),
        );
        t.join().unwrap();
        assert_eq!(c.state, CheckState::Stopped);
        assert_eq!(
            fake.calls()[1],
            vec!["mountDisk".to_string(), "disk9".into()]
        );
        let d = tempfile::tempdir().unwrap();
        let log = HealthLog::new(Store::new(d.path()));
        log.append(&c).unwrap();
        let c2 = check(
            &FakeDisk::ok(),
            "radio-1",
            CheckKind::Verify,
            "/Volumes/CARD",
            None,
            None,
        );
        log.append(&c2).unwrap();
        assert_eq!(log.list("radio-1").len(), 2);
        assert_eq!(log.latest("radio-1").unwrap().state, CheckState::Ok);
    }

    #[test]
    fn the_cargo_runner_refuses() {
        if std::env::var("QUADCAM_SERIAL").as_deref() == Ok("real") {
            return;
        }
        let e = system().run(&["list".into()], None, TIMEOUT).unwrap_err();
        assert!(e.to_string().starts_with("Refused"));
    }
}
