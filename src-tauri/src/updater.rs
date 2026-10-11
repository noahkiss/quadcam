//! The app's own updates through tauri-plugin-updater: a check at launch, once the window has
//! settled, and once a day while the app runs (unless the `update_check` setting is off), and
//! on request from the app menu. The banner downloads a found update; the restart installs
//! it, never while a job runs (`Core::restart_blocker`). The state lives in the core
//! (`core/update.rs`), so `status.update` reports it. These are GUI commands only: an agent
//! never updates or restarts the app.

use crate::core::{notes_url, Core, UpdatePhase, UpdateStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_updater::{Update, UpdaterExt};

/// How long after launch the first automatic check waits.
const FIRST_CHECK: Duration = Duration::from_secs(30);
/// How often the automatic check runs while the app is open.
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// How often the scheduler wakes to see whether a check is due (a Mac that slept wakes late).
const WAKE: Duration = Duration::from_secs(15 * 60);
/// How often a ready update looks again whether the job that held the restart ended.
const JOB_POLL: Duration = Duration::from_secs(2);

/// The update found by the last check, and its verified bytes once downloaded.
#[derive(Default)]
pub struct Pending {
    update: Mutex<Option<Update>>,
    bytes: Mutex<Option<Vec<u8>>>,
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

/// Starts the launch and daily checks.
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

/// Asks the release feed for a newer version and records the answer. Errors land in the
/// state (`failed`), never in a dialog.
async fn check<R: Runtime>(app: &AppHandle<R>, core: &Core) -> UpdateStatus {
    let pending = app.state::<Pending>();
    // A found update stays found while it downloads or waits for the restart.
    let phase = core.update_status().map(|s| s.phase).unwrap_or_default();
    if matches!(phase, UpdatePhase::Downloading | UpdatePhase::Ready)
        || pending.working.swap(true, Ordering::SeqCst)
    {
        return core.update_status().unwrap_or_default();
    }
    core.set_update(|s| {
        s.phase = UpdatePhase::Checking;
        s.error = None;
    });
    let found = match app.updater() {
        Ok(u) => u.check().await.map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    pending.working.store(false, Ordering::SeqCst);
    let now = chrono::Utc::now();
    match found {
        Ok(Some(update)) => {
            let version = update.version.clone();
            *pending.update.lock().unwrap() = Some(update);
            core.set_update(|s| {
                s.phase = UpdatePhase::Available;
                s.notes_url = Some(notes_url(&version));
                s.version = Some(version);
                s.done = 0;
                s.total = None;
                s.checked_at = Some(now);
            })
        }
        Ok(None) => {
            *pending.update.lock().unwrap() = None;
            core.set_update(|s| {
                s.phase = UpdatePhase::Current;
                s.version = None;
                s.notes_url = None;
                s.checked_at = Some(now);
            })
        }
        Err(e) => core.set_update(|s| {
            s.phase = UpdatePhase::Failed;
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

/// Downloads the found update and checks its signature. It installs only at the restart.
#[tauri::command]
#[specta::specta]
pub async fn update_download(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
) -> Result<UpdateStatus, String> {
    let core = core.inner().clone();
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
            *pending.bytes.lock().unwrap() = Some(bytes);
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
    let (Some(update), Some(bytes)) = (update, bytes) else {
        return Err("Download the update first.".into());
    };
    if let Err(e) = update.install(&bytes) {
        // Keep the bytes, so a second click can try again.
        *pending.bytes.lock().unwrap() = Some(bytes);
        let msg = format!("The update did not install: {e}");
        core.set_update(|s| s.error = Some(msg.clone()));
        return Err(msg);
    }
    app.restart();
}
