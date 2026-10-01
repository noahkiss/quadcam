//! The shared core every surface drives: the GUI's Tauri commands, the control socket, the
//! CLI and the MCP server. It owns the session, runs the long steps without holding the
//! lock, saves the session file, and tells the host about changes through `Hooks`.

use crate::disk::{self, Volume};
use crate::media::{self, Encoder, Format};
use crate::photos::{self, PhotosLibrary, ShareReport};
use crate::session::{Editor, PlanPatch, Session, Summary};
use crate::settings::Defaults;
use anyhow::{anyhow, bail, Context, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

mod cuts;
mod files;
mod import;
mod library;
mod setup;
pub use crate::paths::{cache_dir, default_session_file, default_settings_file, support_dir};
pub use files::{Moved, TrashReport};
pub use import::CardStatus;
pub use library::{LibEdit, LibItem, LibraryView, RebuildReport, RenameReport};
pub use setup::{PlaceRemoved, SettingsView};

/// What the host does when the core changes state. The GUI emits events and asks for the
/// format click; a headless host does nothing.
pub trait Hooks: Send + Sync {
    /// The session changed (new clips, plans, results). The GUI re-reads it.
    fn changed(&self) {}
    /// A progress event (`progress`, `import-progress`, `import-result`).
    fn event(&self, _name: &str, _payload: Value) {}
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
}

pub struct NoHooks;
impl Hooks for NoHooks {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportOutcome {
    pub summary: Summary,
    pub photos: Option<Result<ShareReport, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyReport {
    pub id: usize,
    pub output: PathBuf,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub gui: bool,
    pub tools: Result<media::Tools, String>,
    pub defaults: Defaults,
    pub session: Option<SessionBrief>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionBrief {
    pub source: PathBuf,
    pub card: Option<String>,
    pub clips: usize,
    pub analysed: bool,
    pub imported: usize,
    pub failed: usize,
    pub format_ready: Result<(), String>,
}

/// Where the log folder for date planning comes from.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
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
            cache,
            session_file,
            library: Mutex::new(None),
            trash: crate::trash::real_trash(),
            settings_file: None,
        }
    }

    /// Reads and writes settings in `file`, and takes the defaults from it.
    pub fn with_settings(mut self, file: PathBuf) -> Core {
        self.settings_file = Some(file);
        self.reload_settings();
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

    /// Every method `dispatch` answers, in its order.
    pub const METHODS: &'static [&'static str] = &[
        "status",
        "volumes",
        "session",
        "clear",
        "stage",
        "analyse",
        "load",
        "dates",
        "suggest",
        "import",
        "photos",
        "verify",
        "eject",
        "format_plan",
        "format",
        "library",
        "library_rebuild",
        "library_rate",
        "library_edit",
        "library_rename",
        "library_cuts",
        "library_export_cuts",
        "library_trash",
        "library_untrash",
        "library_photos",
        "library_apply_name_format",
        "library_rescan",
        "library_preview",
        "library_strips",
        "card_status",
        "settings",
        "settings_set",
        "places",
        "place_search",
        "place_save",
        "place_delete",
        "profiles",
        "profile_save",
        "profile_delete",
        "profile_default",
        "session_cuts",
    ];

    /// The JSON-RPC surface shared by the control socket and the headless MCP server.
    pub fn dispatch(&self, method: &str, params: Value) -> Result<Value> {
        fn p<T: for<'de> Deserialize<'de> + Default>(v: Value) -> Result<T> {
            if v.is_null() {
                return Ok(T::default());
            }
            serde_json::from_value(v).context("bad params")
        }
        #[derive(Deserialize, Default)]
        struct Source {
            source: Option<PathBuf>,
        }
        #[derive(Deserialize, Default)]
        struct Dates {
            #[serde(default)]
            logs: LogChoice,
            day: Option<NaiveDate>,
        }
        #[derive(Deserialize, Default)]
        struct Suggest {
            patches: Vec<PlanPatch>,
            #[serde(default)]
            editor: Option<Editor>,
        }
        #[derive(Deserialize, Default)]
        struct Photos {
            ids: Option<Vec<usize>>,
            album: Option<String>,
        }
        #[derive(Deserialize, Default)]
        struct Eject {
            target: Option<String>,
        }
        #[derive(Deserialize, Default)]
        struct Label {
            label: Option<String>,
        }
        #[derive(Deserialize, Default)]
        struct Ids {
            #[serde(default)]
            ids: Vec<String>,
            #[serde(default)]
            rating: Option<u8>,
            #[serde(default)]
            flag: Option<crate::library::Flag>,
            #[serde(default)]
            album: Option<String>,
        }
        #[derive(Deserialize, Default)]
        struct One {
            id: String,
            #[serde(default)]
            name: Option<String>,
            #[serde(default)]
            cuts: Vec<crate::moments::Span>,
            #[serde(default)]
            removed_cuts: Option<crate::trim::RemovedCuts>,
            #[serde(flatten)]
            edit: LibEdit,
        }
        #[derive(Deserialize, Default)]
        struct SessionCuts {
            id: usize,
            cuts: Vec<crate::moments::Span>,
            #[serde(default)]
            removed_cuts: Option<crate::trim::RemovedCuts>,
        }
        #[derive(Deserialize, Default)]
        struct Mount {
            mount: PathBuf,
        }
        #[derive(Deserialize, Default)]
        struct SetSettings {
            values: crate::settings::Values,
        }
        #[derive(Deserialize, Default)]
        struct Search {
            query: String,
            #[serde(default)]
            provider: Option<String>,
            #[serde(default)]
            limit: Option<usize>,
        }
        #[derive(Deserialize, Default)]
        struct Named {
            name: String,
            #[serde(default)]
            new_name: Option<String>,
            #[serde(default)]
            lat: Option<f64>,
            #[serde(default)]
            lon: Option<f64>,
            #[serde(default)]
            fields: crate::settings::Values,
        }
        let v = |x: &dyn erased::Ser| x.to_value();
        Ok(match method {
            "status" => v(&self.status()),
            "volumes" => v(&self.volumes()),
            "session" => v(&self.session()),
            "clear" => {
                self.clear()?;
                json!({"cleared": true})
            }
            "stage" => v(&self.stage(p::<Source>(params)?.source.as_deref())?),
            "analyse" => v(&self.analyse()?),
            "load" => v(&self.load(p::<Source>(params)?.source.as_deref())?),
            "dates" => {
                let d: Dates = p(params)?;
                v(&self.plan_dates(d.logs, d.day)?)
            }
            "suggest" => {
                let s: Suggest = p(params)?;
                v(&self.patch(&s.patches, s.editor.unwrap_or(Editor::Agent))?)
            }
            "import" => v(&self.import(&p::<ImportOptions>(params)?)?),
            "photos" => {
                let x: Photos = p(params)?;
                v(&self.add_to_photos(x.ids, x.album)?)
            }
            "verify" => v(&self.verify(p::<Photos>(params)?.ids)?),
            "eject" => {
                self.eject(p::<Eject>(params)?.target.as_deref())?;
                json!({"ejected": true})
            }
            "format_plan" => v(&self.format_plan(p::<Label>(params)?.label.as_deref())?),
            "format" => v(&self.format(&p::<FormatRequest>(params)?, false)?),
            "library" => v(&self.library(&p::<crate::library::Filter>(params)?)?),
            "library_rebuild" => v(&self.library_rebuild()?),
            "library_rate" => {
                let x: Ids = p(params)?;
                v(&self.library_rate(&x.ids, x.rating, x.flag)?)
            }
            "library_edit" => {
                let x: One = p(params)?;
                v(&self.library_edit(&x.id, &x.edit)?)
            }
            "library_rename" => {
                let x: One = p(params)?;
                let name = x.name.context("name is required")?;
                v(&self.library_rename(&x.id, &name)?)
            }
            "library_cuts" => {
                let x: One = p(params)?;
                v(&self.library_set_cuts(&x.id, &x.cuts, x.removed_cuts)?)
            }
            "library_export_cuts" => v(&self.library_export_cuts(&p::<One>(params)?.id)?),
            "library_trash" => v(&self.library_trash(&p::<Ids>(params)?.ids)?),
            "library_untrash" => {
                #[derive(Deserialize, Default)]
                struct Untrash {
                    moved: Vec<Moved>,
                }
                v(&self.library_untrash(&p::<Untrash>(params)?.moved)?)
            }
            "library_photos" => {
                let x: Ids = p(params)?;
                v(&self.library_photos(&x.ids, x.album)?)
            }
            "library_apply_name_format" => {
                let x: Ids = p(params)?;
                v(&self.library_apply_name_format((!x.ids.is_empty()).then_some(x.ids))?)
            }
            "library_rescan" => v(&self.library_rescan(&p::<One>(params)?.id)?),
            "library_preview" => v(&self.library_preview(&p::<One>(params)?.id)?),
            "library_strips" => {
                let x: Ids = p(params)?;
                v(&self.library_strips((!x.ids.is_empty()).then_some(x.ids))?)
            }
            "card_status" => v(&self.card_status(&p::<Mount>(params)?.mount)?),
            "settings" => v(&self.settings()?),
            "settings_set" => v(&self.settings_set(&p::<SetSettings>(params)?.values)?),
            "places" => v(&self.places()?),
            "place_search" => {
                let x: Search = p(params)?;
                v(&self.place_search(&x.query, x.provider.as_deref(), x.limit)?)
            }
            "place_save" => {
                let x: Named = p(params)?;
                v(&self.place_save(&x.name, x.lat, x.lon, x.new_name.as_deref())?)
            }
            "place_delete" => v(&self.place_delete(&p::<Named>(params)?.name)?),
            "profiles" => {
                let (profiles, default) = self.profiles()?;
                json!({"profiles": profiles, "default_profile": default})
            }
            "profile_save" => {
                let x: Named = p(params)?;
                v(&self.profile_save(&x.name, &x.fields, x.new_name.as_deref())?)
            }
            "profile_delete" => v(&self.profile_delete(&p::<Named>(params)?.name)?),
            "profile_default" => v(&self.profile_default(&p::<Named>(params)?.name)?),
            "session_cuts" => {
                let x: SessionCuts = p(params)?;
                v(&self.set_session_cuts(x.id, &x.cuts, x.removed_cuts)?)
            }
            _ => bail!("unknown method {method:?}"),
        })
    }
}

/// Small helper so `dispatch` can serialize any result the same way.
mod erased {
    pub trait Ser {
        fn to_value(&self) -> serde_json::Value;
    }
    impl<T: serde::Serialize> Ser for T {
        fn to_value(&self) -> serde_json::Value {
            serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
        }
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
