//! quadcam: import analog FPV DVR clips (AVI/MJPEG), date and name them, convert, verify, add to
//! Photos, and optionally format the card. The Tauri commands here are thin wrappers over
//! `core::Core`, which the control socket, the CLI and the MCP server share.

pub mod control;
pub mod core;
pub mod disk;
pub mod library;
pub mod logs;
pub mod mcp;
pub mod media;
pub mod metadata;
pub mod moments;
pub mod naming;
pub mod photos;
pub mod pipeline;
pub mod qtmeta;
pub mod scan;
pub mod session;
pub mod trash;
pub mod trim;

use crate::core::{
    Core, FormatPlan, FormatRequest, Hooks, ImportOptions, ImportOutcome, LogChoice,
};
use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use disk::Volume;
use serde::Serialize;
use serde_json::Value;
use session::{Defaults, Editor, PlanPatch, Session};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

/// How long an agent's format request waits for the click in the GUI.
const FORMAT_CONFIRM_TIMEOUT: Duration = Duration::from_secs(180);

/// GUI hooks: events to the webview, and the Erase click for agent-started formats.
struct GuiHooks {
    app: AppHandle,
    pending: Mutex<HashMap<u64, mpsc::Sender<bool>>>,
    next: AtomicU64,
}

impl Hooks for GuiHooks {
    fn changed(&self) {
        let _ = self.app.emit("session-changed", ());
    }
    fn event(&self, name: &str, payload: Value) {
        let _ = self.app.emit(name, payload);
    }
    fn confirm_format(&self, plan: &FormatPlan) -> Result<()> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let _ = self.app.emit(
            "agent-format-request",
            serde_json::json!({"id": id, "plan": plan}),
        );
        let answer = rx.recv_timeout(FORMAT_CONFIRM_TIMEOUT);
        self.pending.lock().unwrap().remove(&id);
        let _ = self.app.emit("agent-format-closed", id);
        match answer {
            Ok(true) => Ok(()),
            Ok(false) => Err(anyhow!("Refused: the user cancelled the erase in quadcam.")),
            Err(_) => Err(anyhow!(
                "Refused: nobody clicked Erase in quadcam within 3 minutes."
            )),
        }
    }
    fn has_gui(&self) -> bool {
        true
    }
}

struct AppState {
    core: Arc<Core>,
    hooks: Arc<GuiHooks>,
}

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

/// Runs a core call off the main thread.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
        .map_err(err)
}

#[derive(Serialize)]
struct EnvCheck {
    tools: Option<media::Tools>,
    error: Option<String>,
    install_hint: &'static str,
    socket: Option<String>,
}

#[tauri::command]
fn env_check(state: State<'_, AppState>) -> EnvCheck {
    let _ = &state.core;
    let socket = Some(control::socket_path().to_string_lossy().to_string());
    match media::find_tools() {
        Ok(t) => EnvCheck {
            tools: Some(t),
            error: None,
            install_hint: media::INSTALL_HINT,
            socket,
        },
        Err(e) => EnvCheck {
            tools: None,
            error: Some(err(e)),
            install_hint: media::INSTALL_HINT,
            socket,
        },
    }
}

/// The output folder used until the user picks one: ~/Movies/quadcam.
#[tauri::command]
fn default_output_dir() -> Option<PathBuf> {
    pipeline::default_output_dir()
}

#[tauri::command]
fn set_defaults(state: State<'_, AppState>, defaults: Defaults) {
    state.core.set_defaults(defaults);
}

#[tauri::command]
async fn list_volumes() -> Vec<Volume> {
    tauri::async_runtime::spawn_blocking(disk::list_volumes)
        .await
        .unwrap_or_default()
}

#[tauri::command]
fn get_session(state: State<'_, AppState>) -> Option<Session> {
    state.core.session()
}

/// Stages every clip from `path` (a card or any folder), analyses them and plans dates.
#[tauri::command]
async fn load_source(state: State<'_, AppState>, path: String) -> Result<Session, String> {
    let core = state.core.clone();
    blocking(move || core.load(Some(&PathBuf::from(path)))).await
}

#[tauri::command]
async fn plan_dates(
    state: State<'_, AppState>,
    log_dir: Option<String>,
    day: Option<NaiveDate>,
) -> Result<Session, String> {
    let core = state.core.clone();
    let logs = match log_dir {
        Some(d) if !d.is_empty() => LogChoice::Dir(PathBuf::from(d)),
        _ => LogChoice::None,
    };
    blocking(move || core.plan_dates(logs, day)).await
}

/// A person's edit in the GUI. It clears the agent-suggested mark on the fields it touches.
#[tauri::command]
fn edit_plan(state: State<'_, AppState>, patch: PlanPatch) -> Result<Session, String> {
    state.core.patch(&[patch], Editor::User).map_err(err)
}

#[tauri::command]
async fn import_clips(
    state: State<'_, AppState>,
    options: ImportOptions,
) -> Result<ImportOutcome, String> {
    let core = state.core.clone();
    blocking(move || core.import(&options)).await
}

#[tauri::command]
async fn add_to_photos(
    state: State<'_, AppState>,
    ids: Option<Vec<usize>>,
    album: Option<String>,
) -> Result<photos::ShareReport, String> {
    let core = state.core.clone();
    blocking(move || core.add_to_photos(ids, album)).await
}

#[tauri::command]
async fn format_plan(
    state: State<'_, AppState>,
    label: Option<String>,
) -> Result<FormatPlan, String> {
    let core = state.core.clone();
    blocking(move || core.format_plan(label.as_deref())).await
}

/// The GUI's own Erase button: the click in its confirm dialog is the confirmation.
#[tauri::command]
async fn format_card(state: State<'_, AppState>, label: String) -> Result<FormatPlan, String> {
    let core = state.core.clone();
    blocking(move || {
        let plan = core.format_plan(Some(&label))?;
        let req = FormatRequest {
            device: plan.device.clone(),
            volume_uuid: plan.volume_uuid.clone(),
            label: Some(label),
            confirm: true,
        };
        core.format(&req, true)
    })
    .await
}

/// The person's answer to an agent's format request.
#[tauri::command]
fn answer_format_request(state: State<'_, AppState>, id: u64, approve: bool) {
    if let Some(tx) = state.hooks.pending.lock().unwrap().remove(&id) {
        let _ = tx.send(approve);
    }
}

/// "Start over": forgets the session and deletes the session file.
#[tauri::command]
fn clear_session(state: State<'_, AppState>) -> Result<(), String> {
    state.core.clear().map_err(err)
}

#[tauri::command]
async fn eject(state: State<'_, AppState>, path: Option<String>) -> Result<(), String> {
    let core = state.core.clone();
    blocking(move || core.eject(path.as_deref())).await
}

/// Makes (or reuses) a small H.264 preview of a clip.
#[tauri::command]
async fn preview(state: State<'_, AppState>, id: usize) -> Result<PathBuf, String> {
    let core = state.core.clone();
    blocking(move || core.preview(id)).await
}

/// Emits `volumes-changed` whenever something mounts or unmounts under /Volumes.
fn watch_volumes(app: AppHandle) {
    use notify::{RecursiveMode, Watcher};
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let Ok(mut w) = notify::recommended_watcher(tx) else {
            return;
        };
        if w.watch(
            std::path::Path::new("/Volumes"),
            RecursiveMode::NonRecursive,
        )
        .is_err()
        {
            return;
        }
        while let Ok(ev) = rx.recv() {
            if ev.is_ok() {
                // Let the mount settle before the UI asks diskutil about it.
                std::thread::sleep(Duration::from_millis(800));
                while rx.try_recv().is_ok() {}
                let _ = app.emit("volumes-changed", ());
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let hooks = Arc::new(GuiHooks {
                app: handle.clone(),
                pending: Mutex::new(HashMap::new()),
                next: AtomicU64::new(1),
            });
            let cache = app.path().app_cache_dir()?;
            // The GUI shares the CLI's session file, so a relaunch shows the last session
            // again while its staged clips are still in the cache.
            let session_file = cache.join("session.json");
            let core = Arc::new(Core::new(
                cache,
                Some(session_file),
                hooks.clone(),
                Core::real_photos(),
            ));
            core.forget_unrestorable();
            if let Err(e) = control::serve(core.clone(), &control::socket_path()) {
                eprintln!("quadcam: control socket not started: {e:#}");
            }
            app.manage(AppState { core, hooks });
            watch_volumes(handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            env_check,
            default_output_dir,
            set_defaults,
            list_volumes,
            get_session,
            load_source,
            plan_dates,
            edit_plan,
            import_clips,
            add_to_photos,
            format_plan,
            format_card,
            answer_format_request,
            eject,
            clear_session,
            preview
        ])
        .run(tauri::generate_context!())
        .expect("error while running quadcam");
}
