//! quadcam: import analog FPV DVR clips (AVI/MJPEG), date and name them, convert, verify, add to
//! Photos, and optionally format the card. The Tauri commands here are thin wrappers over
//! `core::Core`, which the control socket, the CLI and the MCP server share.

pub mod api;
pub mod control;
pub mod core;
pub mod cuts;
pub mod disk;
pub mod gear;
pub mod geocode;
pub mod identity;
pub mod join;
pub mod library;
pub mod logmatch;
pub mod logs;
pub mod mcp;
pub mod media;
mod menu;
pub mod metadata;
pub mod modules;
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

use crate::core::{Core, FormatPlan, FormatRequest, Hooks};
use crate::gear::model::{ApplyPlan, StagedChange};
use anyhow::{anyhow, Result};
use serde::Serialize;
use session::Session;
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
    fn confirm_apply(&self, change: &StagedChange, plan: &ApplyPlan) -> Result<()> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let _ = self.app.emit(
            api::AgentApplyRequest::NAME,
            api::AgentApplyRequest {
                id,
                change: change.clone(),
                plan: plan.clone(),
            },
        );
        let answer = rx.recv_timeout(FORMAT_CONFIRM_TIMEOUT);
        self.pending.lock().unwrap().remove(&id);
        let _ = self.app.emit(api::AgentApplyClosed::NAME, id);
        match answer {
            Ok(true) => Ok(()),
            Ok(false) => Err(anyhow!("Refused: the user cancelled the apply in quadcam.")),
            Err(_) => Err(anyhow!(
                "Refused: nobody clicked Apply in quadcam within 3 minutes."
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
    fn gear_changed(&self) {
        let _ = self.app.emit(api::GearChanged::NAME, ());
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
    /// The app version (`Cargo.toml`).
    version: &'static str,
    /// The commit it was built from (short hash; `unknown` outside a git checkout).
    build: &'static str,
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
            version: env!("CARGO_PKG_VERSION"),
            build: BUILD,
        },
        Err(e) => EnvCheck {
            tools: None,
            error: Some(err(e)),
            install_hint: media::INSTALL_HINT,
            socket,
            version: env!("CARGO_PKG_VERSION"),
            build: BUILD,
        },
    }
}

/// The commit this build came from (`build.rs`).
pub const BUILD: &str = env!("QUADCAM_BUILD");

/// The output folder used until the user picks one: ~/Movies/quadcam.
#[tauri::command]
#[specta::specta]
fn default_output_dir() -> Option<PathBuf> {
    pipeline::default_output_dir()
}

/// A folder or clip files dropped on the window.
#[tauri::command]
#[specta::specta]
async fn load_dropped(state: State<'_, AppState>, paths: Vec<PathBuf>) -> Result<Session, String> {
    let core = state.core.clone();
    blocking(move || core.load_dropped(&paths)).await
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

/// The apply sheet's own Apply button: the click is the confirmation.
#[tauri::command]
#[specta::specta]
async fn gear_apply_click(
    state: State<'_, AppState>,
    params: api::ApplyRequest,
) -> Result<api::ApplyReport, String> {
    let core = state.core.clone();
    blocking(move || core.gear_apply_click(&params)).await
}

/// The person's answer to an agent's apply request.
#[tauri::command]
#[specta::specta]
fn answer_apply_request(state: State<'_, AppState>, id: u64, approve: bool) {
    if let Some(tx) = state.hooks.pending.lock().unwrap().remove(&id) {
        let _ = tx.send(approve);
    }
}

/// The built-in sim profiles the Sim page offers.
#[tauri::command]
#[specta::specta]
fn sim_presets(state: State<'_, AppState>) -> Vec<core::SimPreset> {
    state.core.sim_presets()
}

/// Starts the sim (a GUI command only: a sim is flown by hand, so it is not an `api` row).
#[tauri::command]
#[specta::specta]
async fn sim_start(
    state: State<'_, AppState>,
    params: core::SimStartParams,
) -> Result<core::SimStartInfo, String> {
    let core = state.core.clone();
    blocking(move || core.sim_start(&params)).await
}

/// The newest steps and HUD values, once per frame. Async so it runs off the main thread.
#[tauri::command]
#[specta::specta]
async fn sim_frame(state: State<'_, AppState>) -> Result<core::SimFrame, String> {
    state.core.sim_frame().map_err(err)
}

/// Back to the start pad.
#[tauri::command]
#[specta::specta]
async fn sim_reset(state: State<'_, AppState>) -> Result<(), String> {
    state.core.sim_reset().map_err(err)
}

/// Stops the sim.
#[tauri::command]
#[specta::specta]
async fn sim_stop(state: State<'_, AppState>) -> Result<Option<core::SimStopInfo>, String> {
    let core = state.core.clone();
    blocking(move || Ok(core.sim_stop())).await
}

/// Makes (or reuses) a small H.264 preview of a clip.
#[tauri::command]
#[specta::specta]
async fn preview(state: State<'_, AppState>, id: usize) -> Result<PathBuf, String> {
    let core = state.core.clone();
    blocking(move || core.preview(id)).await
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

/// The third-party notices shipped in the app (`Contents/Resources/THIRD_PARTY_NOTICES.txt`,
/// made by `scripts/notices.mjs` in the build).
#[tauri::command]
#[specta::specta]
fn third_party_notices(app: AppHandle) -> Result<String, String> {
    let dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    std::fs::read_to_string(dir.join("THIRD_PARTY_NOTICES.txt")).map_err(|_| {
        "The notices are in the built app. Run `node scripts/notices.mjs` to see them here."
            .to_string()
    })
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

/// Looks for gear every `gear::detect::POLL`: sends `device-changed` with the events
/// (connected, identified, unmounted but still in, removed), runs the on-connect hooks on
/// their own threads for events QuadCam did not cause, runs the FC USB heat timer
/// (`Core::gear_usb_tick`), plays due reminders, and sends `gear-changed` when another process (the
/// CLI, an MCP server) wrote `gear.json`.
fn poll_gear(app: AppHandle, core: Arc<Core>) {
    std::thread::spawn(move || {
        let mut tracker = gear::events::Tracker::default();
        let mut stamp = core.gear_store().stamp();
        loop {
            match core.gear_poll(&mut tracker) {
                Ok(events) => {
                    if !events.is_empty() {
                        let _ = app.emit(
                            api::DeviceChanged::NAME,
                            api::DeviceChanged {
                                events: events.clone(),
                                connected: tracker.connected.clone(),
                                unmounted: tracker.unmounted.clone(),
                            },
                        );
                    }
                    for e in events.iter().filter(|e| !e.app_initiated) {
                        if matches!(
                            e.kind,
                            gear::events::DeviceEventKind::Connected
                                | gear::events::DeviceEventKind::Identified
                        ) {
                            let (core, device) = (core.clone(), e.device.clone());
                            std::thread::spawn(move || {
                                core.gear_on_connect(&device);
                            });
                        }
                    }
                    // FCs on USB with a battery in: "Unplug now" at their limit.
                    core.gear_usb_tick(std::time::Instant::now());
                    core.gear_play_reminders(
                        &tracker,
                        std::time::Instant::now(),
                        chrono::Local::now().time(),
                    );
                }
                Err(e) => eprintln!("quadcam: gear poll: {e:#}"),
            }
            let now = core.gear_store().stamp();
            if now != stamp {
                stamp = now;
                let _ = app.emit(api::GearChanged::NAME, ());
            }
            std::thread::sleep(gear::detect::POLL);
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
    let mut builder = tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?;
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
/// commands and the GUI's own commands.
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
            c::card_prep_plan,
            c::card_prep,
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
            c::library_match_logs,
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
            c::session_split,
            c::library_split,
            c::gear_status,
            c::gear_devices,
            c::gear_device_save,
            c::gear_device_forget,
            c::gear_fc_identify,
            c::gear_fc_read,
            c::gear_board_notes,
            c::gear_usb_timers,
            c::gear_poll_pause,
            c::gear_dismiss_reminder,
            c::gear_osd,
            c::gear_card,
            c::gear_card_preview,
            c::gear_switch_map,
            c::gear_radio,
            c::gear_radio_watch,
            c::gear_sim_calibration,
            c::gear_sim_calibration_save,
            c::gear_sim_defaults,
            c::gear_sim_validate,
            c::gear_sim_calibrate,
            c::gear_flights,
            c::gear_flight_set,
            c::gear_flight_folders,
            c::gear_packs,
            c::gear_pack_save,
            c::gear_pack_delete,
            c::gear_pack_type_save,
            c::gear_pack_type_delete,
            c::gear_pack_notes,
            c::gear_session_report,
            c::gear_session_report_save,
            c::gear_preflight,
            c::gear_crashes,
            c::gear_crash_save,
            c::gear_crash_delete,
            c::gear_backup,
            c::gear_backups,
            c::gear_backup_read,
            c::gear_backup_diff,
            c::gear_backup_pin,
            c::gear_storage,
            c::gear_prune,
            c::gear_export,
            c::gear_import_backups,
            c::gear_card_check,
            c::gear_card_checks,
            c::gear_card_repair,
            c::gear_stop,
            c::gear_changes,
            c::gear_change_stage,
            c::gear_change_update,
            c::gear_change_discard,
            c::gear_restore_stage,
            c::gear_apply_plan,
            c::gear_apply,
            c::modules,
            c::module_install,
            c::module_remove,
            c::modules_check,
            env_check,
            default_output_dir,
            load_dropped,
            preview,
            sim_presets,
            sim_start,
            sim_frame,
            sim_reset,
            sim_stop,
            format_card,
            answer_format_request,
            gear_apply_click,
            answer_apply_request,
            library_scope,
            third_party_notices,
            menu::menu_state,
            share::share,
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
            api::GearChanged,
            api::DeviceChanged,
            api::AgentFormatRequest,
            api::AgentFormatClosed,
            api::AgentApplyRequest,
            api::AgentApplyClosed,
            api::Menu,
            api::RadioInput,
            api::SimCalibrationEvent,
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
                .with_settings(settings_file),
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
            for h in crate::core::backup_hooks() {
                core.gear_add_hook(h);
            }
            let gear_core = core.clone();
            specta.mount_events(app);
            app.manage(core.clone());
            app.manage(AppState { core, hooks });
            main_window(&handle)?;
            menu::install(&handle)?;
            watch_volumes(handle.clone());
            poll_gear(handle, gear_core);
            Ok(())
        })
        .invoke_handler(typed)
        .run(tauri::generate_context!())
        .expect("error while running quadcam");
}
