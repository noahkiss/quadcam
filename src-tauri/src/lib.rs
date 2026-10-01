//! quadcam: import analog FPV DVR clips (AVI/MJPEG), date and name them, convert, verify, add to
//! Photos, and optionally format the card. The Tauri commands here are thin wrappers over
//! `core::Core`, which the control socket, the CLI and the MCP server share.

pub mod api;
pub mod control;
pub mod core;
pub mod cuts;
pub mod disk;
pub mod geocode;
pub mod library;
pub mod logs;
pub mod mcp;
pub mod media;
mod menu;
pub mod metadata;
pub mod moments;
pub mod naming;
pub mod paths;
pub mod photos;
pub mod pipeline;
pub mod qtmeta;
pub mod scan;
pub mod session;
pub mod settings;
mod share;
pub mod sources;
pub mod trash;
pub mod trim;
pub mod watch;

use crate::core::{
    Core, FormatPlan, FormatRequest, Hooks, ImportOptions, ImportOutcome, LogChoice,
};
use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use disk::Volume;
use serde::Serialize;
use session::{Editor, PlanPatch, Session};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_specta::Event as _;

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
        let _ = self.app.emit(api::SessionChanged::NAME, ());
    }
    fn event(&self, event: api::Event) {
        let (name, payload) = event.name_and_payload();
        let _ = self.app.emit(name, payload);
    }
    fn confirm_format(&self, plan: &FormatPlan) -> Result<()> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let _ = self.app.emit(
            api::AgentFormatRequest::NAME,
            api::AgentFormatRequest {
                id,
                plan: plan.clone(),
            },
        );
        let answer = rx.recv_timeout(FORMAT_CONFIRM_TIMEOUT);
        self.pending.lock().unwrap().remove(&id);
        let _ = self.app.emit(api::AgentFormatClosed::NAME, id);
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
    fn library_changed(&self) {
        let _ = self.app.emit(api::LibraryChanged::NAME, ());
    }
    fn settings_changed(&self) {
        let _ = self.app.emit(api::SettingsChanged::NAME, ());
    }
    fn analysed(&self) {
        let app = self.app.clone();
        std::thread::spawn(move || {
            if let Some(st) = app.try_state::<AppState>() {
                st.core.make_previews();
            }
        });
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

#[derive(Serialize, specta::Type)]
struct EnvCheck {
    tools: Option<media::Tools>,
    error: Option<String>,
    install_hint: &'static str,
    socket: Option<String>,
}

#[tauri::command]
#[specta::specta]
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
#[specta::specta]
fn default_output_dir() -> Option<PathBuf> {
    pipeline::default_output_dir()
}

/// @deprecated Legacy UI; use `volumes`.
#[tauri::command]
#[specta::specta]
async fn list_volumes() -> Vec<Volume> {
    tauri::async_runtime::spawn_blocking(disk::list_volumes)
        .await
        .unwrap_or_default()
}

/// @deprecated Legacy UI; use `session`.
#[tauri::command]
#[specta::specta]
fn get_session(state: State<'_, AppState>) -> Option<Session> {
    state.core.session()
}

/// Stages every clip from `path` (a card or any folder), analyses them and plans dates.
///
/// @deprecated Legacy UI; use `load`.
#[tauri::command]
#[specta::specta]
async fn load_source(state: State<'_, AppState>, path: String) -> Result<Session, String> {
    let core = state.core.clone();
    blocking(move || core.load(Some(&PathBuf::from(path)))).await
}

/// A folder or clip files dropped on the window.
#[tauri::command]
#[specta::specta]
async fn load_dropped(state: State<'_, AppState>, paths: Vec<PathBuf>) -> Result<Session, String> {
    let core = state.core.clone();
    blocking(move || core.load_dropped(&paths)).await
}

/// @deprecated Legacy UI; use `dates`.
#[tauri::command]
#[specta::specta]
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
///
/// @deprecated Legacy UI; use `suggest` with `editor: "user"`.
#[tauri::command]
#[specta::specta]
fn edit_plan(state: State<'_, AppState>, patch: PlanPatch) -> Result<Session, String> {
    state.core.patch(&[patch], Editor::User).map_err(err)
}

/// Several edits at once ("Apply to all", the session bar): one core call, one save.
///
/// @deprecated Legacy UI; use `suggest` with `editor: "user"`.
#[tauri::command]
#[specta::specta]
fn edit_plans(state: State<'_, AppState>, patches: Vec<PlanPatch>) -> Result<Session, String> {
    state.core.patch(&patches, Editor::User).map_err(err)
}

/// @deprecated Legacy UI; use `import`.
#[tauri::command]
#[specta::specta]
async fn import_clips(
    state: State<'_, AppState>,
    options: ImportOptions,
) -> Result<ImportOutcome, String> {
    let core = state.core.clone();
    blocking(move || core.import(&options)).await
}

/// @deprecated Legacy UI; use `photos`.
#[tauri::command]
#[specta::specta]
async fn add_to_photos(
    state: State<'_, AppState>,
    ids: Option<Vec<usize>>,
    album: Option<String>,
) -> Result<photos::ShareReport, String> {
    let core = state.core.clone();
    blocking(move || core.add_to_photos(ids, album)).await
}

/// The GUI's own Erase button: the click in its confirm dialog is the confirmation.
#[tauri::command]
#[specta::specta]
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
#[specta::specta]
fn answer_format_request(state: State<'_, AppState>, id: u64, approve: bool) {
    if let Some(tx) = state.hooks.pending.lock().unwrap().remove(&id) {
        let _ = tx.send(approve);
    }
}

/// "Start over": forgets the session and deletes the session file.
///
/// @deprecated Legacy UI; use `clear`.
#[tauri::command]
#[specta::specta]
fn clear_session(state: State<'_, AppState>) -> Result<(), String> {
    state.core.clear().map_err(err)
}

/// Makes (or reuses) a small H.264 preview of a clip.
#[tauri::command]
#[specta::specta]
async fn preview(state: State<'_, AppState>, id: usize) -> Result<PathBuf, String> {
    let core = state.core.clone();
    blocking(move || core.preview(id)).await
}

/// Any `Core::dispatch` method, off the main thread. The library calls go through here.
///
/// @deprecated Legacy UI; use the typed command of the same name.
#[tauri::command]
#[specta::specta]
async fn core_call(
    state: State<'_, AppState>,
    method: String,
    params: api::Json,
) -> Result<api::Json, String> {
    let core = state.core.clone();
    blocking(move || core.dispatch(&method, params.0).map(api::Json)).await
}

/// Lets the webview load files from the library folder (thumbnails and MP4 playback).
#[tauri::command]
#[specta::specta]
fn library_scope(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let root = state.core.library_root().map_err(err)?;
    app.asset_protocol_scope()
        .allow_directory(&root, true)
        .map_err(|e| e.to_string())
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
                let _ = app.emit(api::VolumesChanged::NAME, ());
            }
        }
    });
}

/// Set for test runs: the app starts behind other windows and never takes focus.
fn no_focus() -> bool {
    std::env::var_os("QUADCAM_NO_FOCUS").is_some()
}

/// Makes the main window from its config. With `QUADCAM_NO_FOCUS` it opens unfocused, below
/// other windows, and keeps rendering there so `screencapture -l` sees it.
fn main_window(app: &AppHandle) -> tauri::Result<()> {
    let config = app.config().app.windows[0].clone();
    let mut builder = tauri::WebviewWindowBuilder::from_config(app, &config)?;
    if no_focus() {
        app.set_activation_policy(tauri::ActivationPolicy::Accessory)?;
        builder = builder
            .focused(false)
            .always_on_bottom(true)
            .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled);
    }
    let window = builder.build()?;
    dev_eval(app, window);
    Ok(())
}

/// Debug builds only: `QUADCAM_DEV_EVAL=<file>` runs the script written to that file in the
/// window, then deletes the file. Scripts report back with `emit("dev-log", text)`, which
/// prints to stderr. Tests drive the UI this way without mouse or keyboard events.
#[cfg(debug_assertions)]
fn dev_eval(app: &AppHandle, window: tauri::WebviewWindow) {
    use tauri::Listener;
    let Some(file) = std::env::var_os("QUADCAM_DEV_EVAL").map(PathBuf::from) else {
        return;
    };
    app.listen_any("dev-log", |e| eprintln!("dev-log: {}", e.payload()));
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(300));
        if let Ok(js) = std::fs::read_to_string(&file) {
            let _ = std::fs::remove_file(&file);
            if let Err(e) = window.eval(&js) {
                eprintln!("dev-eval: {e}");
            }
        }
    });
}

#[cfg(not(debug_assertions))]
fn dev_eval(_: &AppHandle, _: tauri::WebviewWindow) {}

/// Every GUI command with its types, and the events, for tauri-specta: the `api` table's
/// commands, the GUI's own commands, and the legacy UI's commands (marked deprecated).
pub fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    use api::commands as c;
    tauri_specta::Builder::<tauri::Wry>::new()
        .commands(tauri_specta::collect_commands![
            c::status,
            c::volumes,
            c::session,
            c::clear,
            c::stage,
            c::analyse,
            c::load,
            c::dates,
            c::suggest,
            c::import,
            c::photos,
            c::verify,
            c::eject,
            c::format_plan,
            c::format,
            c::library,
            c::library_rebuild,
            c::library_rate,
            c::library_edit,
            c::library_update,
            c::library_rename,
            c::library_cuts,
            c::library_export_cuts,
            c::library_trash,
            c::library_untrash,
            c::library_photos,
            c::library_apply_name_format,
            c::library_rescan,
            c::library_preview,
            c::library_strips,
            c::card_status,
            c::settings,
            c::settings_set,
            c::places,
            c::place_search,
            c::place_save,
            c::place_delete,
            c::profiles,
            c::profile_save,
            c::profile_delete,
            c::profile_default,
            c::session_cuts,
            env_check,
            default_output_dir,
            load_dropped,
            preview,
            format_card,
            answer_format_request,
            library_scope,
            menu::menu_state,
            share::share,
            list_volumes,
            get_session,
            load_source,
            plan_dates,
            edit_plan,
            edit_plans,
            import_clips,
            add_to_photos,
            clear_session,
            core_call,
        ])
        .events(tauri_specta::collect_events![
            api::Progress,
            api::ImportProgress,
            api::ImportResult,
            api::LibraryTask,
            api::SessionChanged,
            api::LibraryChanged,
            api::SettingsChanged,
            api::VolumesChanged,
            api::AgentFormatRequest,
            api::AgentFormatClosed,
            api::Menu,
        ])
        // Sizes and counts fit a JS number.
        .dangerously_cast_bigints_to_number()
}

/// The TypeScript the bindings are written with.
pub fn bindings_language() -> specta_typescript::Typescript {
    specta_typescript::Typescript::default().header(
        "// Generated by tauri-specta from src-tauri/src/api (quadcam_lib::specta_builder).\n// Do not edit. Regenerate: QUADCAM_UPDATE_BINDINGS=1 cargo test --test bindings\n",
    )
}

/// The legacy UI's `eject` and `format_plan` take their own arguments, while the typed
/// commands of the same names take `params`. A call without `params` comes from the legacy
/// UI and is answered here, the way the legacy commands answered it. Goes with the legacy UI.
mod legacy {
    use crate::core::Core;
    use serde_json::Value;
    use std::sync::Arc;
    use tauri::ipc::{Invoke, InvokeBody, InvokeError};
    use tauri::Manager;

    pub fn wants(invoke: &Invoke) -> bool {
        matches!(invoke.message.command(), "eject" | "format_plan")
            && match invoke.message.payload() {
                InvokeBody::Json(v) => v.get("params").is_none(),
                _ => false,
            }
    }

    pub fn handle(invoke: Invoke) {
        let core = invoke
            .message
            .webview()
            .state::<Arc<Core>>()
            .inner()
            .clone();
        let command = invoke.message.command().to_string();
        let args = match invoke.message.payload() {
            InvokeBody::Json(v) => v.clone(),
            _ => Value::Null,
        };
        invoke.resolver.respond_async(async move {
            tauri::async_runtime::spawn_blocking(move || {
                let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);
                match command.as_str() {
                    "eject" => core.eject(text("path").as_deref()).map(|_| Value::Null),
                    _ => core
                        .format_plan(text("label").as_deref())
                        .map(|p| serde_json::to_value(p).unwrap_or(Value::Null)),
                }
                .map_err(|e| format!("{e:#}"))
            })
            .await
            .map_err(|e| InvokeError::from(e.to_string()))?
            .map_err(InvokeError::from)
        });
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let specta = specta_builder();
    // Debug builds keep the new UI's bindings current, once it exists.
    #[cfg(debug_assertions)]
    {
        let app_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../app/src");
        if app_dir.is_dir() {
            if let Err(e) = specta.export(bindings_language(), app_dir.join("bindings.ts")) {
                eprintln!("quadcam: bindings.ts not written: {e}");
            }
        }
    }
    let typed = specta.invoke_handler();
    tauri::Builder::default()
        // Launching never takes focus from the app in front; a person's launch from the Dock
        // or Finder still brings the app forward.
        .activate_ignoring_other_apps(false)
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
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
            // The same settings file the CLI and the MCP server use.
            let settings_file = app.path().app_data_dir()?.join("settings.json");
            let core = Arc::new(
                Core::new(
                    cache,
                    Some(session_file),
                    hooks.clone(),
                    Core::real_photos(),
                )
                .with_settings(settings_file.clone()),
            );
            core.forget_unrestorable();
            // The CLI and a headless MCP server write these files directly.
            let (a, b) = (handle.clone(), handle.clone());
            watch::spawn(
                core.clone(),
                move || {
                    let _ = a.emit(api::SettingsChanged::NAME, ());
                },
                move || {
                    let _ = b.emit(api::LibraryChanged::NAME, ());
                },
            );
            // A restored session's previews may be gone from the cache; make them again.
            let warm = core.clone();
            std::thread::spawn(move || warm.make_previews());
            if let Err(e) = control::serve(core.clone(), &control::socket_path()) {
                eprintln!("quadcam: control socket not started: {e:#}");
            }
            specta.mount_events(app);
            app.manage(core.clone());
            app.manage(AppState { core, hooks });
            main_window(&handle)?;
            menu::install(&handle)?;
            watch_volumes(handle);
            Ok(())
        })
        .invoke_handler(move |invoke| {
            if legacy::wants(&invoke) {
                legacy::handle(invoke);
                return true;
            }
            typed(invoke)
        })
        .run(tauri::generate_context!())
        .expect("error while running quadcam");
}
