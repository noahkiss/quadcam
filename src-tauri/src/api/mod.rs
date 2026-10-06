//! The one table of core methods. Each row names a method, its params type, its result
//! type, and the `Core` call that answers it. `api!` turns the table into:
//!
//! - `Core::dispatch` and `Core::METHODS`, for the control socket and the headless MCP server;
//! - one typed Tauri command per method in `commands`, for the GUI, which tauri-specta
//!   exports to `app/src/bindings.ts` with every param and result type;
//! - one typed function per method in `call`, which the CLI and both of the above use.
//!
//! A method with params takes one struct. `dispatch` reads JSON null as its default.

pub mod events;
pub mod library;
pub mod session;
pub mod setup;

pub use events::*;
pub use library::*;
pub use session::*;
pub use setup::*;

use crate::core::{
    CardStatus, Core, FormatPlan, FormatRequest, ImportOptions, ImportOutcome, LibMatchParams,
    LibMatchReport, LibraryView, PlaceRemoved, RebuildReport, RenameReport, SettingsView, Status,
    TrashReport, VerifyReport,
};
use crate::disk::Volume;
use crate::geocode::GeoResult;
use crate::library::{Filter, LibClip, LibCut};
use crate::metadata::{Place, Profile};
use crate::photos::ShareReport;
use crate::session::{Editor, Session};
use crate::trim::CutChange;
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;

/// Any JSON value, typed `unknown` in TypeScript (specta cannot export a `serde_json::Value`
/// field yet).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(transparent)]
pub struct Json(#[specta(type = specta_typescript::Unknown)] pub Value);

/// How serde writes a `Result`: `{"Ok": value}` or `{"Err": error}`. Fields of type
/// `Result` take this as their TypeScript type (specta's own `Result` type has another
/// shape).
#[derive(specta::Type)]
#[allow(dead_code)]
pub enum SerdeResult<T, E> {
    Ok(T),
    Err(E),
}

/// Params from JSON: null is the default (no params).
fn from_json<T: serde::de::DeserializeOwned + Default>(v: Value) -> Result<T> {
    if v.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(v).context("bad params")
}

macro_rules! api {
    ($(
        $(#[doc = $doc:literal])*
        $name:ident($($p:ident: $pty:ty)?) -> $ret:ty = |$c:ident| $body:expr;
    )*) => {
        /// One typed function per method: the params type in, the result type out. The CLI,
        /// `dispatch` and the Tauri commands all call these.
        pub mod call {
            #[allow(unused_imports)]
            use super::*;

            $(
                $(#[doc = $doc])*
                pub fn $name($c: &Core $(, $p: $pty)?) -> Result<$ret> {
                    $body
                }
            )*
        }

        impl Core {
            /// Every method `dispatch` answers, in table order.
            pub const METHODS: &'static [&'static str] = &[$(stringify!($name)),*];

            /// The JSON-RPC surface shared by the control socket and the headless MCP server.
            #[allow(unused_variables)]
            pub fn dispatch(&self, method: &str, raw: Value) -> Result<Value> {
                match method {
                    $(stringify!($name) => {
                        $(let $p: $pty = from_json(raw)?;)?
                        let out = call::$name(self $(, $p)?)?;
                        Ok(serde_json::to_value(&out).unwrap_or(Value::Null))
                    })*
                    _ => anyhow::bail!("unknown method {method:?}"),
                }
            }
        }

        /// One Tauri command per method, run off the main thread.
        pub mod commands {
            #[allow(unused_imports)]
            use super::*;
            use std::sync::Arc;

            $(
                $(#[doc = $doc])*
                #[tauri::command]
                #[specta::specta]
                pub async fn $name(
                    core: tauri::State<'_, Arc<Core>>,
                    $($p: $pty)?
                ) -> std::result::Result<$ret, String> {
                    let core = core.inner().clone();
                    tauri::async_runtime::spawn_blocking(move || call::$name(&core $(, $p)?))
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| format!("{e:#}"))
                }
            )*
        }
    };
}

api! {
    /// Whether the GUI runs, the tools, the effective settings, and the session in brief.
    status() -> Status = |c| Ok(c.status());
    /// Mounted volumes: DVR cards and radio log sources.
    volumes() -> Vec<Volume> = |c| Ok(c.volumes());
    /// The import session, if one is loaded.
    session() -> Option<Session> = |c| Ok(c.session());
    /// Forgets the session and deletes the session file.
    clear() -> Cleared = |c| c.clear().map(|_| Cleared { cleared: true });
    /// Copies the clips off a card or folder; a new session.
    stage(params: SourceParams) -> Session = |c| c.stage(params.source.as_deref());
    /// Probes, recovers and thumbnails the staged clips.
    analyse() -> Session = |c| c.analyse();
    /// Stage, analyse and date in one go.
    load(params: SourceParams) -> Session = |c| c.load(params.source.as_deref());
    /// Dates the clips from radio logs.
    dates(params: DatesParams) -> Session = |c| c.plan_dates(params.logs, params.day);
    /// Changes clip plans (an agent's suggestions unless `editor` is `user`).
    suggest(params: SuggestParams) -> Session =
        |c| c.patch(&params.patches, params.editor.unwrap_or(Editor::Agent));
    /// Converts and verifies the clips, and optionally adds them to Photos.
    import(params: ImportOptions) -> ImportOutcome = |c| c.import(&params);
    /// Adds verified outputs to Photos.
    photos(params: PhotosParams) -> ShareReport = |c| c.add_to_photos(params.ids, params.album);
    /// Checks verified outputs again.
    verify(params: VerifyParams) -> Vec<VerifyReport> = |c| c.verify(params.ids);
    /// Ejects a card.
    eject(params: EjectParams) -> Ejected =
        |c| c.eject(params.target.as_deref()).map(|_| Ejected { ejected: true });
    /// Runs every format guard and names the card that would be erased.
    format_plan(params: LabelParams) -> FormatPlan = |c| c.format_plan(params.label.as_deref());
    /// Erases the card. With the GUI running, the person must click Erase too.
    format(params: FormatRequest) -> FormatPlan = |c| c.format(&params, false);
    /// The library, narrowed by a filter.
    library(params: Filter) -> LibraryView = |c| c.library(&params);
    /// Makes the index again from the files.
    library_rebuild() -> RebuildReport = |c| c.library_rebuild();
    /// Sets stars and flags.
    library_rate(params: RateParams) -> Vec<LibClip> =
        |c| c.library_rate(&params.ids, params.rating, params.flag);
    /// Changes one clip's details, date or time.
    library_edit(params: LibraryEditParams) -> LibClip = |c| c.library_edit(&params.id, &params.edit);
    /// Changes stars, flag, name and details of clips in one call; checks everything first.
    library_update(params: LibraryUpdateParams) -> Vec<LibClip> =
        |c| c.library_update(&params.ids, &params.update);
    /// Renames a clip, its cuts and its original.
    library_rename(params: RenameParams) -> LibClip = |c| {
        let name = params.name.context("name is required")?;
        c.library_rename(&params.id, &name)
    };
    /// Sets a clip's cut list.
    library_cuts(params: LibraryCutsParams) -> CutChange =
        |c| c.library_set_cuts(&params.id, &params.cuts, params.removed_cuts);
    /// Writes a clip's unsaved cuts as files.
    library_export_cuts(params: ClipIdParams) -> Vec<LibCut> = |c| c.library_export_cuts(&params.id);
    /// Moves clips, with their cuts and originals, to the Trash.
    library_trash(params: IdsParams) -> TrashReport = |c| c.library_trash(&params.ids);
    /// Puts trashed clips back.
    library_untrash(params: UntrashParams) -> Vec<String> = |c| c.library_untrash(&params.moved);
    /// Adds clips and their cuts to Photos.
    library_photos(params: LibraryPhotosParams) -> ShareReport =
        |c| c.library_photos(&params.ids, params.album);
    /// Renames clips to the file-name date format (every clip when `ids` is empty).
    library_apply_name_format(params: IdsParams) -> RenameReport =
        |c| c.library_apply_name_format((!params.ids.is_empty()).then_some(params.ids));
    /// Matches radio logs to library clips by shape; writes flight numbers and moments only
    /// with `apply`.
    library_match_logs(params: LibMatchParams) -> LibMatchReport = |c| c.library_match_logs(&params);
    /// Finds dead air again in a clip.
    library_rescan(params: ClipIdParams) -> LibClip = |c| c.library_rescan(&params.id);
    /// A file the web view can play.
    library_preview(params: ClipIdParams) -> PathBuf = |c| c.library_preview(&params.id);
    /// Makes missing hover-scrub strips (for every clip when `ids` is empty).
    library_strips(params: IdsParams) -> usize =
        |c| c.library_strips((!params.ids.is_empty()).then_some(params.ids));
    /// Clips on a card and how many are new to the library.
    card_status(params: MountParams) -> CardStatus = |c| c.card_status(&params.mount);
    /// The settings file and the effective settings.
    settings() -> SettingsView = |c| c.settings();
    /// Changes settings.
    settings_set(params: SetSettingsParams) -> SettingsView = |c| c.settings_set(&params.values);
    /// Saved places.
    places() -> Vec<Place> = |c| c.places();
    /// Searches for an address or a named place.
    place_search(params: SearchParams) -> Vec<GeoResult> =
        |c| c.place_search(&params.query, params.provider.as_deref(), params.limit);
    /// Creates or changes a saved place.
    place_save(params: PlaceSaveParams) -> Place =
        |c| c.place_save(&params.name, params.lat, params.lon, params.new_name.as_deref());
    /// Deletes a saved place.
    place_delete(params: NameParams) -> PlaceRemoved = |c| c.place_delete(&params.name);
    /// Aircraft profiles and the default.
    profiles() -> ProfilesView = |c| {
        let (profiles, default_profile) = c.profiles()?;
        Ok(ProfilesView {
            profiles,
            default_profile,
        })
    };
    /// Creates or changes an aircraft profile.
    profile_save(params: ProfileSaveParams) -> Profile =
        |c| c.profile_save(&params.name, &params.fields, params.new_name.as_deref());
    /// Deletes an aircraft profile.
    profile_delete(params: NameParams) -> Profile = |c| c.profile_delete(&params.name);
    /// Sets the default profile; an empty name means none.
    profile_default(params: NameParams) -> Option<String> = |c| c.profile_default(&params.name);
    /// Sets a session clip's cut list.
    session_cuts(params: SessionCutsParams) -> CutChange =
        |c| c.set_session_cuts(params.id, &params.cuts, params.removed_cuts);
}
