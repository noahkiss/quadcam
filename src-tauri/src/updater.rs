//! The app's own updates through tauri-plugin-updater: a check at launch, once the window has
//! settled, and once an hour while the app runs (unless the `update_check` setting is off), and
//! on request from the app menu. Every check runs while an update is known, and a newer
//! release replaces the known one (`core::weigh`). The banner's Restart to update checks once
//! more, downloads the newest release, and the restart installs it, never while a job runs
//! (`Core::restart_blocker`), and never a version at or below the running one. The state lives in the core
//! (`core/update.rs`), so `status.update` reports it. These are GUI commands only: an agent
//! never updates or restarts the app.

use crate::core::{is_newer, weigh, Core, Found, UpdatePhase, UpdateStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_updater::{Update, UpdaterExt};

/// How long after launch the first automatic check waits.
const FIRST_CHECK: Duration = Duration::from_secs(30);
/// How often the automatic check runs while the app is open.
const EVERY: Duration = Duration::from_secs(60 * 60);
/// How often the scheduler wakes to see whether a check is due (a Mac that slept wakes late).
const WAKE: Duration = Duration::from_secs(15 * 60);
/// How often a ready update looks again whether the job that held the restart ended.
const JOB_POLL: Duration = Duration::from_secs(2);

/// The update found by the last check, and its verified bytes (with their version) once
/// downloaded.
#[derive(Default)]
pub struct Pending {
    update: Mutex<Option<Update>>,
    bytes: Mutex<Option<(String, Vec<u8>)>>,
    /// A check or download runs now.
    working: AtomicBool,
}

/// Whether the automatic check may reach the network. A process started by cargo (`cargo
/// tauri dev`, tests) never checks on its own unless `QUADCAM_UPDATER=real`; the menu's
/// Check for Updates still works.
fn automatic_allowed() -> bool {
    std::env::var("QUADCAM_UPDATER").is_ok_and(|v| v == "real")
        || std::env::var_os("CARGO_MANIFEST_DIR").is_none()
}

/// Starts the launch and hourly checks.
pub fn install<R: Runtime>(app: &AppHandle<R>, core: Arc<Core>) {
    app.manage(Pending::default());
    if !automatic_allowed() {
        eprintln!("quadcam: automatic update checks are off under cargo (QUADCAM_UPDATER=real turns them on)");
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(FIRST_CHECK);
        let mut last: Option<Instant> = None;
        loop {
            let due = last.is_none_or(|t| t.elapsed() >= EVERY);
            if due && core.defaults().update_check {
                tauri::async_runtime::block_on(check(&app, &core));
                last = Some(Instant::now());
            }
            std::thread::sleep(WAKE);
        }
    });
}

/// Asks the release feed for a newer version and records the answer. It runs while an
/// update is known too: the feed names the newest release, which replaces an older known one.
/// Errors land in the state, never in a dialog; a failed check keeps a known update.
async fn check<R: Runtime>(app: &AppHandle<R>, core: &Core) -> UpdateStatus {
    let pending = app.state::<Pending>();
    let before = core.update_status().unwrap_or_default();
    // A download runs now: it checked just before it started.
    if before.phase == UpdatePhase::Downloading || pending.working.swap(true, Ordering::SeqCst) {
        return before;
    }
    let known = matches!(before.phase, UpdatePhase::Available | UpdatePhase::Ready)
        .then_some(before.version.clone())
        .flatten();
    if known.is_none() {
        core.set_update(|s| {
            s.phase = UpdatePhase::Checking;
            s.error = None;
        });
    }
    let found = match app.updater() {
        Ok(u) => u.check().await.map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    pending.working.store(false, Ordering::SeqCst);
    let now = chrono::Utc::now();
    match found {
        Ok(update) => {
            let version = update.as_ref().map(|u| u.version.clone());
            match weigh(known.as_deref(), version.as_deref(), &before.current) {
                Found::Same => {
                    *pending.update.lock().unwrap() = update;
                    core.set_update(|s| {
                        s.error = None;
                        s.checked_at = Some(now);
                    })
                }
                Found::Take(v) => {
                    *pending.update.lock().unwrap() = update;
                    *pending.bytes.lock().unwrap() = None;
                    core.set_update(|s| {
                        s.offer(&v);
                        s.checked_at = Some(now);
                    })
                }
                Found::Nothing => {
                    *pending.update.lock().unwrap() = None;
                    *pending.bytes.lock().unwrap() = None;
                    core.set_update(|s| {
                        s.phase = UpdatePhase::Current;
                        s.version = None;
                        s.notes_url = None;
                        s.error = None;
                        s.checked_at = Some(now);
                    })
                }
            }
        }
        Err(e) => core.set_update(|s| {
            if known.is_none() {
                s.phase = UpdatePhase::Failed;
            }
            s.error = Some(format!("Could not check for updates: {e}"));
            s.checked_at = Some(now);
        }),
    }
}

/// The update state now (None never happens in the app; it has a GUI).
#[tauri::command]
#[specta::specta]
pub fn update_status(core: State<'_, Arc<Core>>) -> Option<UpdateStatus> {
    core.update_status()
}

/// Checks now (the app menu's Check for Updates, or Settings > Advanced).
#[tauri::command]
#[specta::specta]
pub async fn update_check(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
) -> Result<UpdateStatus, String> {
    let core = core.inner().clone();
    Ok(check(&app, &core).await)
}

/// Checks once more, then downloads the newest update and checks its signature. A download
/// of that same version already done is kept. It installs only at the restart.
#[tauri::command]
#[specta::specta]
pub async fn update_download(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
) -> Result<UpdateStatus, String> {
    let core = core.inner().clone();
    let checked = check(&app, &core).await;
    if checked.phase != UpdatePhase::Available {
        // Ready (this version is downloaded), Current, or a failed check.
        return Ok(checked);
    }
    let pending = app.state::<Pending>();
    let Some(update) = pending.update.lock().unwrap().clone() else {
        return Err("No update to download. Check for updates first.".into());
    };
    if pending.working.swap(true, Ordering::SeqCst) {
        return Ok(core.update_status().unwrap_or_default());
    }
    core.set_update(|s| {
        s.phase = UpdatePhase::Downloading;
        s.done = 0;
        s.total = None;
        s.error = None;
    });
    let mut done: u64 = 0;
    let mut shown = Instant::now();
    let got = update
        .download(
            |chunk, total| {
                done += chunk as u64;
                // A few updates a second are enough for the banner.
                if shown.elapsed() >= Duration::from_millis(200) {
                    shown = Instant::now();
                    core.set_update(|s| {
                        s.done = done;
                        s.total = total;
                    });
                }
            },
            || {},
        )
        .await;
    pending.working.store(false, Ordering::SeqCst);
    match got {
        Ok(bytes) => {
            let size = bytes.len() as u64;
            *pending.bytes.lock().unwrap() = Some((update.version.clone(), bytes));
            let now = core.set_update(|s| {
                s.phase = UpdatePhase::Ready;
                s.done = size;
                s.total = Some(size);
            });
            if now.waiting_for.is_some() {
                tell_when_free(core.clone());
            }
            Ok(now)
        }
        Err(e) => Ok(core.set_update(|s| {
            s.phase = UpdatePhase::Failed;
            s.error = Some(format!("The update did not download: {e}"));
        })),
    }
}

/// Tells the banner once the job that held the restart back has ended.
fn tell_when_free(core: Arc<Core>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(JOB_POLL);
        let Some(s) = core.update_status() else {
            return;
        };
        if s.phase != UpdatePhase::Ready {
            return;
        }
        if s.waiting_for.is_none() {
            core.set_update(|_| {});
            return;
        }
    });
}

/// Installs the downloaded update and restarts the app. Refuses while a job runs, so an
/// import, a card or device write, or a flash is never cut off.
#[tauri::command]
#[specta::specta]
pub async fn update_restart(app: AppHandle, core: State<'_, Arc<Core>>) -> Result<(), String> {
    if let Some(job) = core.restart_blocker() {
        // The banner shows the wait and offers the restart again when the job ends.
        core.set_update(|_| {});
        tell_when_free(core.inner().clone());
        return Err(format!("QuadCam restarts after {job} ends."));
    }
    let pending = app.state::<Pending>();
    let update = pending.update.lock().unwrap().clone();
    let bytes = pending.bytes.lock().unwrap().take();
    let (Some(update), Some((version, bytes))) = (update, bytes) else {
        return Err("Download the update first.".into());
    };
    if version != update.version || !is_newer(&update.version, &update.current_version) {
        return Err("Download the update first.".into());
    }
    if let Err(e) = update.install(&bytes) {
        // Keep the bytes, so a second click can try again.
        *pending.bytes.lock().unwrap() = Some((version, bytes));
        let msg = format!("The update did not install: {e}");
        core.set_update(|s| s.error = Some(msg.clone()));
        return Err(msg);
    }
    app.restart();
}
