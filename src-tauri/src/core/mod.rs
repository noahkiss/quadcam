//! The shared core every surface drives: the GUI's Tauri commands, the control socket, the
//! CLI and the MCP server. It owns the session, runs the long steps without holding the
//! lock, saves the session file, and tells the host about changes through `Hooks`.

use crate::disk::{self, Volume};
use crate::media::{self, Encoder, Format};
use crate::photos::{self, PhotosLibrary, ShareReport};
use crate::session::{Session, Summary};
use crate::settings::Defaults;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

mod cuts;
mod fc;
mod files;
mod gear;
mod import;
mod library;
mod modules;
mod osd;
mod prep;
mod rematch;
mod setup;
pub use crate::paths::{cache_dir, default_session_file, default_settings_file, support_dir};
pub use fc::{BoardNotesParams, FcJob, FcPortParams, FcReadParams, UsbTimer, USB_PROBE};
pub use files::{Moved, TrashReport};
pub use gear::{
    connected_name, link_handle, DeviceSaveParams, GearStatus, Hold, HookFn, HookOutcome, HookRun,
    OnConnectHook, HOLD_GRACE,
};
pub use import::CardStatus;
pub use library::{LibEdit, LibItem, LibUpdate, LibraryView, RebuildReport, RenameReport};
pub use osd::OsdParams;
pub use rematch::{LibMatch, LibMatchParams, LibMatchReport};
pub use setup::{PlaceRemoved, SettingsView};

/// What the host does when the core changes state. The GUI emits events and asks for the
/// format click; a headless host does nothing.
pub trait Hooks: Send + Sync {
    /// The session changed (new clips, plans, results). The GUI re-reads it.
    fn changed(&self) {}
    /// A progress event (`progress`, `import-progress`, `import-result`, `library-task`).
    fn event(&self, _event: crate::api::Event) {}
    /// Called before a format that did not start from the GUI's own button. The GUI shows
    /// its confirm dialog and returns Ok only after the user clicks Erase.
    fn confirm_format(&self, _plan: &FormatPlan) -> Result<()> {
        Ok(())
    }
    /// True when a person is watching (the GUI).
    fn has_gui(&self) -> bool {
        false
    }
    /// The library index changed. The GUI re-reads it.
    fn library_changed(&self) {}
    /// The settings file changed (profiles, places, preferences). The GUI re-reads it.
    fn settings_changed(&self) {}
    /// The session's clips were analysed. The GUI starts making their previews.
    fn analysed(&self) {}
    /// `gear.json` changed (devices, links). The GUI re-reads Gear.
    fn gear_changed(&self) {}
}

pub struct NoHooks;
impl Hooks for NoHooks {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct FormatPlan {
    /// Whole disk, for example `disk4`.
    pub disk: String,
    pub device: String,
    pub volume_uuid: String,
    pub volume_name: String,
    pub size: u64,
    pub media_name: String,
    pub clip_count: usize,
    pub label: String,
}

/// An explicit format request from the CLI or an agent. Every field must match the card
/// the clips were read from.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct FormatRequest {
    /// Whole-disk device node, for example `/dev/disk4`.
    pub device: String,
    pub volume_uuid: String,
    #[serde(default)]
    pub label: Option<String>,
    /// Must be true.
    #[serde(default)]
    pub confirm: bool,
}

/// Per-run overrides for an import. Anything missing comes from `Defaults`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct ImportOptions {
    #[serde(default)]
    pub output_dir: Option<PathBuf>,
    #[serde(default)]
    pub format: Option<Format>,
    #[serde(default)]
    pub encoder: Option<Encoder>,
    #[serde(default)]
    pub keep_originals: Option<bool>,
    #[serde(default)]
    pub add_time: Option<bool>,
    #[serde(default)]
    pub add_to_photos: bool,
    #[serde(default)]
    pub album: Option<String>,
    /// Keep every clip file on the card or folder this run, even when the
    /// `delete_clips_after_import` setting is on. Nothing here turns deletion on: the
    /// setting is the only consent.
    #[serde(default)]
    pub keep_clips: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ImportOutcome {
    pub summary: Summary,
    #[specta(type = Option<crate::api::SerdeResult<ShareReport, String>>)]
    pub photos: Option<Result<ShareReport, String>>,
    /// What "Delete clips after import" did with each clip. None when it did not run: the
    /// setting is off, or this run kept the clips.
    #[serde(default)]
    pub clip_deletion: Option<Vec<ClipDeletion>>,
}

/// What happened to one clip's file on the card or folder after an import.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum DeletionState {
    /// The file was deleted.
    Deleted,
    /// The file stays; `reason` says why.
    Kept,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct ClipDeletion {
    pub id: usize,
    /// The clip's file on the card or folder.
    pub path: PathBuf,
    pub state: DeletionState,
    /// Why the file stays. None when it was deleted.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct VerifyReport {
    pub id: usize,
    pub output: PathBuf,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct Status {
    pub gui: bool,
    #[specta(type = crate::api::SerdeResult<media::Tools, String>)]
    pub tools: Result<media::Tools, String>,
    pub defaults: Defaults,
    pub session: Option<SessionBrief>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct SessionBrief {
    pub source: PathBuf,
    /// The video system of the clips.
    pub kind: crate::sources::SourceKind,
    pub card: Option<String>,
    pub clips: usize,
    pub analysed: bool,
    pub imported: usize,
    pub failed: usize,
    #[specta(type = crate::api::SerdeResult<(), String>)]
    pub format_ready: Result<(), String>,
}

/// Where the log folder for date planning comes from.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, specta::Type)]
#[serde(rename_all = "lowercase", tag = "kind", content = "path")]
pub enum LogChoice {
    /// Keep the session's current folder (or the default).
    #[default]
    Keep,
    None,
    Dir(PathBuf),
}

pub struct Core {
    session: Mutex<Option<Session>>,
    defaults: Mutex<Defaults>,
    busy: AtomicBool,
    hooks: Arc<dyn Hooks>,
    photos: Arc<dyn PhotosLibrary>,
    cache: PathBuf,
    session_file: Option<PathBuf>,
    /// The library index, loaded on first use: (library folder, index).
    library: Mutex<Option<library::Loaded>>,
    trash: Arc<dyn crate::trash::Trash>,
    /// The app's settings file; the defaults are read from it.
    settings_file: Option<PathBuf>,
    /// What Gear reaches outside the process: serial ports, volumes, DFU devices.
    gear: crate::gear::Env,
    /// Steps that may run when a device is plugged in (`gear_add_hook`).
    gear_hooks: Mutex<Vec<OnConnectHook>>,
    /// Links QuadCam holds for a job: None while held, else held until then.
    gear_holds: Mutex<std::collections::HashMap<String, Option<std::time::Instant>>>,
    /// What each FC port said last, and its USB timer (`core/fc.rs`).
    fc_state: Mutex<fc::FcState>,
    /// CLI and MSP waits (`with_fc_timing`).
    fc_timing: Mutex<crate::gear::bf::cli::Timing>,
    /// Downloaded tools (ffmpeg, esptool).
    modules: crate::modules::Modules,
}

struct Busy<'a>(&'a AtomicBool);
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Whether to use the real Photos library: see `Core::real_photos`.
pub fn photos_enabled(setting: Option<&str>, under_cargo: bool) -> bool {
    match setting {
        Some("real") => true,
        Some(_) => false,
        None => !under_cargo,
    }
}

impl Core {
    pub fn new(
        cache: PathBuf,
        session_file: Option<PathBuf>,
        hooks: Arc<dyn Hooks>,
        photos: Arc<dyn PhotosLibrary>,
    ) -> Core {
        let session = session_file
            .as_deref()
            .filter(|f| f.is_file())
            .and_then(|f| Session::load(f).ok());
        Core {
            session: Mutex::new(session),
            defaults: Mutex::new(Defaults::default()),
            busy: AtomicBool::new(false),
            hooks,
            photos,
            session_file,
            library: Mutex::new(None),
            trash: crate::trash::real_trash(),
            settings_file: None,
            gear: crate::gear::Env::system(&cache),
            gear_hooks: Mutex::new(Vec::new()),
            gear_holds: Mutex::default(),
            fc_state: Mutex::default(),
            fc_timing: Mutex::default(),
            cache,
            modules: crate::modules::Modules::default(),
        }
    }

    /// Reads and writes settings in `file`, and takes the defaults from it.
    pub fn with_settings(mut self, file: PathBuf) -> Core {
        self.settings_file = Some(file);
        self.reload_settings();
        self
    }

    /// Replaces the module manager (tests pass a temp folder, a manifest and a fetcher).
    pub fn with_modules(mut self, modules: crate::modules::Modules) -> Core {
        self.modules = modules;
        self
    }

    /// Replaces the Trash (tests pass a folder).
    pub fn with_trash(mut self, trash: Arc<dyn crate::trash::Trash>) -> Core {
        self.trash = trash;
        self
    }

    /// A core for the CLI or a headless MCP server: shared cache, session file, PhotoKit.
    /// Export defaults come from the app's saved settings when there are any.
    pub fn headless(session_file: Option<PathBuf>, photos: Arc<dyn PhotosLibrary>) -> Core {
        Core::new(
            cache_dir(),
            Some(session_file.unwrap_or_else(default_session_file)),
            Arc::new(NoHooks),
            photos,
        )
        .with_settings(default_settings_file())
    }

    /// The Photos library to use. `QUADCAM_PHOTOS=real` forces PhotoKit and
    /// `QUADCAM_PHOTOS=dry-run` forces a recorder that adds nothing. When it is unset, a
    /// process started by cargo (`cargo test`, `cargo run`, `cargo tauri dev`, and anything
    /// they spawn, which all carry `CARGO_MANIFEST_DIR`) gets the recorder too. So a test can
    /// never reach a real Photos library, even if it forgets to set the variable.
    pub fn real_photos() -> Arc<dyn PhotosLibrary> {
        if !photos_enabled(
            std::env::var("QUADCAM_PHOTOS").ok().as_deref(),
            std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
        ) {
            eprintln!(
                "quadcam: Photos is in dry-run mode (set QUADCAM_PHOTOS=real to use PhotoKit)"
            );
            return Arc::new(photos::Recorder::default());
        }
        #[cfg(target_os = "macos")]
        return Arc::new(photos::PhotoKit);
        #[cfg(not(target_os = "macos"))]
        return Arc::new(photos::Recorder {
            fail: true,
            ..Default::default()
        });
    }

    fn claim(&self) -> Result<Busy<'_>> {
        if self.busy.swap(true, Ordering::SeqCst) {
            bail!("Another step is still running. Try again when it finishes.");
        }
        Ok(Busy(&self.busy))
    }

    fn commit(&self, s: Option<Session>) -> Result<()> {
        if let (Some(f), Some(s)) = (&self.session_file, &s) {
            s.save(f)?;
        }
        *self.session.lock().unwrap() = s;
        self.hooks.changed();
        Ok(())
    }

    fn current(&self) -> Result<Session> {
        self.session
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| anyhow!("No clips loaded. Load a card or folder first."))
    }

    pub fn session(&self) -> Option<Session> {
        self.session.lock().unwrap().clone()
    }

    /// Drops a session restored from the session file when the GUI cannot show it: its
    /// staged clips are gone (the cache was cleared), or it was never analysed (the app quit
    /// while loading). The GUI then starts empty. The file stays until the next commit.
    pub fn forget_unrestorable(&self) {
        let mut guard = self.session.lock().unwrap();
        if guard
            .as_ref()
            .is_some_and(|s| !s.analysed || !s.staged_files_exist())
        {
            *guard = None;
        }
    }

    /// Forgets the session and deletes the session file. Staged copies stay in the cache.
    pub fn clear(&self) -> Result<()> {
        let _b = self.claim()?;
        if let Some(f) = &self.session_file {
            match std::fs::remove_file(f) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(e).with_context(|| format!("removing {}", f.display()))
                }
                _ => {}
            }
        }
        *self.session.lock().unwrap() = None;
        self.hooks.changed();
        Ok(())
    }

    pub fn defaults(&self) -> Defaults {
        self.defaults.lock().unwrap().clone()
    }

    pub fn set_defaults(&self, d: Defaults) {
        *self.defaults.lock().unwrap() = d;
    }

    pub fn has_gui(&self) -> bool {
        self.hooks.has_gui()
    }

    pub fn status(&self) -> Status {
        let session = self.session().map(|s| {
            let sum = s.summary();
            SessionBrief {
                source: s.source.clone(),
                kind: s.kind,
                card: s.card.as_ref().map(|c| c.whole_disk.clone()),
                clips: s.clips.len(),
                analysed: s.analysed,
                imported: sum.imported,
                failed: sum.failed,
                format_ready: sum.format_ready,
            }
        });
        Status {
            gui: self.has_gui(),
            tools: media::find_tools().map_err(|e| format!("{e:#}")),
            defaults: self.defaults(),
            session,
        }
    }

    pub fn volumes(&self) -> Vec<Volume> {
        disk::list_volumes()
    }
}

#[cfg(test)]
mod tests {
    use super::photos_enabled;

    #[test]
    fn photos_fail_safe() {
        assert!(photos_enabled(None, false), "a normal launch uses PhotoKit");
        assert!(
            !photos_enabled(None, true),
            "anything started by cargo gets the recorder"
        );
        assert!(!photos_enabled(Some("dry-run"), false));
        assert!(
            !photos_enabled(Some("anything-else"), false),
            "unknown values are safe"
        );
        assert!(
            photos_enabled(Some("real"), true),
            "explicit opt-in for cargo tauri dev"
        );
    }

    /// This test binary runs under cargo, so the default must be the recorder.
    #[test]
    fn under_cargo_test_the_default_is_the_recorder() {
        assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some());
        if std::env::var("QUADCAM_PHOTOS").as_deref() != Ok("real") {
            let lib = super::Core::real_photos();
            let d = tempfile::tempdir().unwrap();
            let f = d.path().join("x.mp4");
            std::fs::write(&f, b"x").unwrap();
            // A recorder "adds" instantly; PhotoKit would ask for permission.
            crate::photos::share(lib.as_ref(), &[f], None).unwrap();
        }
    }
}
