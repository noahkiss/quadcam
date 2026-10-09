//! Each tool's arguments as a type. `tools.rs` derives the tool's `inputSchema` from it, and
//! the handler deserializes the arguments into it once. Field docs are the schema's
//! descriptions; a field without a doc comment has none. `x-nullable` marks the fields
//! whose schema also allows null (see `tools::clean`).

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

// ----- shared shapes -----

// A location in decimal degrees.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct LatLon {
    #[schemars(range(min = -90, max = 90))]
    pub lat: f64,
    #[schemars(range(min = -180, max = 180))]
    pub lon: f64,
}

// A cut range in a session clip.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct SuggestCut {
    /// Seconds into the clip.
    #[schemars(range(min = 0))]
    pub start: f64,
    #[schemars(range(min = 0))]
    pub end: f64,
}

// A cut range in a library clip.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct LibraryCut {
    #[schemars(range(min = 0))]
    pub start: f64,
    #[schemars(range(min = 0))]
    pub end: f64,
}

// ----- the import flow -----

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct StatusArgs {}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct LoadClipsArgs {
    /// Card mount point (e.g. /Volumes/NO NAME) or a folder of AVI files. Omit to use the first detected card.
    pub source: Option<String>,
    /// false keeps recordings the DVR split into files (PICT0001.AVI, PICT0002.AVI, ...) as separate clips this run. Omit to follow the join_split_recordings setting (on by default).
    pub join: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ReadClipsArgs {
    /// Clip ids from quadcam_read_clips. Omit for all clips.
    pub ids: Option<Vec<u64>>,
    /// Attach the first-frame thumbnail of each clip as an image.
    #[schemars(extend("default" = false))]
    pub thumbnails: Option<bool>,
    #[schemars(range(min = 1, max = 50), extend("default" = 12))]
    pub max_thumbnails: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct MatchLogsArgs {
    /// EdgeTX LOGS folder, or the radio's root when it is mounted in USB storage mode.
    pub log_dir: Option<String>,
    /// Log day to match. Omit for the newest plausible day.
    #[schemars(pattern(r"^\d{4}-\d{2}-\d{2}$"))]
    pub day: Option<String>,
    /// Stop using logs; every clip gets the import date.
    pub no_logs: Option<bool>,
}

// One suggestion for one clip.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct Suggestion {
    pub id: u64,
    #[schemars(length(max = 80))]
    pub name: Option<String>,
    #[schemars(pattern(r"^\d{4}-\d{2}-\d{2}$"))]
    pub date: Option<String>,
    /// Time of day, HH:MM 24-hour; empty string for local noon. A new date without a time resets it to noon.
    #[schemars(pattern(r"^(\d{2}:\d{2}(:\d{2})?)?$"))]
    pub time: Option<String>,
    pub note: Option<String>,
    pub skip: Option<bool>,
    /// Ranges to export as extra files, each at least 0.5 s.
    #[schemars(length(max = 20))]
    pub cuts: Option<Vec<SuggestCut>>,
    /// Seconds into the clip where the radio log's first armed row falls (the DVR usually starts before arming).
    pub log_offset_s: Option<f64>,
    #[schemars(length(max = 80))]
    pub profile: Option<String>,
    /// Saved place name; empty string removes the location.
    #[schemars(length(max = 80))]
    pub place: Option<String>,
    pub location: Option<LatLon>,
    #[schemars(length(max = 30), inner(length(max = 60)))]
    pub keywords: Option<Vec<String>>,
    #[schemars(length(max = 120))]
    pub author: Option<String>,
    /// One short line on why, shown to the person.
    pub reason: Option<String>,
    /// For the first file of a recording the DVR split into files (it has `files`): true imports them as one clip, false as clips of their own.
    pub joined: Option<bool>,
    /// true adds one cut per radio-log flight (armed range plus up to 2 s each side), after cuts. Fails when the clip has no flights, or one flight that covers nearly all of it.
    pub split_by_flight: Option<bool>,
    /// Required when `cuts` drops a cut that was already exported: keep its file (it becomes a clip of its own) or move it to the Trash. Ask the person which.
    #[schemars(extend("enum" = ["keep", "trash"]))]
    pub removed_cuts: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct SuggestArgs {
    // Passed on to the core's `suggest`, which reads each as a `PlanPatch`.
    #[schemars(required, length(min = 1), with = "Vec<Suggestion>")]
    pub suggestions: Option<Value>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ExportArgs {
    pub output_dir: Option<String>,
    #[schemars(extend("enum" = ["mp4", "mov"]))]
    pub format: Option<String>,
    /// Also copy each source AVI into <output>/originals/.
    pub keep_originals: Option<bool>,
    /// Add HHMM to names of clips dated from a radio log.
    pub add_time: Option<bool>,
    /// Add the verified outputs to Photos afterwards.
    pub add_to_photos: Option<bool>,
    /// true: keep every clip on the card or folder this run, even when the delete_clips_after_import setting is on. There is no way to turn deletion on here.
    pub keep_clips: Option<bool>,
    /// Photos album; empty string for the library only.
    pub album: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct VerifyArgs {
    /// Clip ids from quadcam_read_clips. Omit for all clips.
    pub ids: Option<Vec<u64>>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct AddToPhotosArgs {
    /// Clip ids from quadcam_read_clips. Omit for all clips.
    pub ids: Option<Vec<u64>>,
    /// Album name; empty string for the library only. Omit for the default.
    pub album: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct EjectArgs {
    /// Mount point or /dev/diskN. Omit for the session's card.
    pub target: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct FormatCardArgs {
    /// Run every guard and return the plan; erase nothing.
    pub dry_run: Option<bool>,
    /// Whole-disk device from the dry run, e.g. /dev/disk4.
    #[schemars(pattern(r"^/dev/disk[0-9]+$"))]
    pub device: Option<String>,
    /// Volume UUID from the dry run.
    pub volume_uuid: Option<String>,
    /// Volume name, default DVR.
    #[schemars(length(max = 11))]
    pub label: Option<String>,
    /// Must be true to erase.
    pub confirm: Option<bool>,
    /// Card prep: erase a card with no session (new, or all its clips in the library).
    pub prep: Option<bool>,
    /// Card prep: the card's mount point, for the dry run.
    pub mount: Option<String>,
}

// ----- the library -----

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct LibraryArgs {
    #[schemars(length(max = 200))]
    pub query: Option<String>,
    #[schemars(extend("enum" = ["all", "last_import", "moments", "picks", "rejected", "not_in_photos"]))]
    pub group: Option<String>,
    #[schemars(pattern(r"^\d{4}-\d{2}-\d{2}$"))]
    pub day: Option<String>,
    /// Saved place name.
    pub place: Option<String>,
    /// Aircraft profile name.
    pub aircraft: Option<String>,
    #[schemars(range(min = 0, max = 5))]
    pub min_rating: Option<u8>,
    #[schemars(range(min = 1, max = 500))]
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct LibraryEditArgs {
    /// Library clip ids from quadcam_library.
    #[schemars(required, length(min = 1, max = 500))]
    pub ids: Option<Vec<String>>,
    /// Stars; 0 clears.
    #[schemars(range(min = 0, max = 5))]
    pub rating: Option<u8>,
    #[schemars(extend("enum" = ["pick", "reject", "none"]))]
    pub flag: Option<String>,
    /// New short name; one id only.
    #[schemars(length(max = 80))]
    pub name: Option<String>,
    pub note: Option<String>,
    /// Replaces the clip's keywords.
    #[schemars(length(max = 30), inner(length(max = 60)))]
    pub keywords: Option<Vec<String>>,
    #[schemars(length(max = 120))]
    pub author: Option<String>,
    /// Saved place name (quadcam_places); empty string removes the location.
    #[schemars(length(max = 80))]
    pub place: Option<String>,
    pub location: Option<LatLon>,
    /// Aircraft profile name (quadcam_profiles); empty string removes the profile's details.
    #[schemars(length(max = 80))]
    pub profile: Option<String>,
    /// New flying day.
    #[schemars(pattern(r"^\d{4}-\d{2}-\d{2}$"))]
    pub date: Option<String>,
    /// Time of day, HH:MM 24-hour; empty string for local noon.
    #[schemars(pattern(r"^(\d{2}:\d{2}(:\d{2})?)?$"))]
    pub time: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct LibraryFilesArgs {
    #[schemars(required, extend("enum" = ["cuts", "split_by_flight", "export_cuts", "trash", "photos", "rebuild", "apply_name_format", "match_logs"]))]
    pub action: Option<String>,
    /// Library clip ids from quadcam_library. cuts, split_by_flight and export_cuts take exactly one; rebuild takes none; apply_name_format takes some or none (every clip).
    #[schemars(length(max = 500))]
    pub ids: Option<Vec<String>>,
    /// For cuts: ranges in clip seconds, each at least 0.5 s.
    #[schemars(length(max = 20))]
    pub cuts: Option<Vec<LibraryCut>>,
    #[schemars(extend("enum" = ["keep", "trash"]))]
    pub removed_cuts: Option<String>,
    /// For cuts and split_by_flight: also write the new cuts as files.
    pub export: Option<bool>,
    /// For photos: album name; empty string for the library only. Omit for the default.
    pub album: Option<String>,
    /// For match_logs: EdgeTX LOGS folder; omit for the log folder setting.
    pub log_dir: Option<String>,
    /// For match_logs: one log day for every clip; omit to match each clip to its own day's logs, else a reset-clock (2000-01-01) log.
    #[schemars(pattern(r"^\d{4}-\d{2}-\d{2}$"))]
    pub day: Option<String>,
    /// For match_logs: write flight numbers and moments into "matched" clips (and "likely" ones named in ids). Omit to only report.
    pub apply: Option<bool>,
}

// ----- setup -----

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct PlacesArgs {
    #[schemars(required, extend("enum" = ["list", "search", "save", "delete"]))]
    pub action: Option<String>,
    /// For search: an address or a place name.
    #[schemars(length(max = 200))]
    pub query: Option<String>,
    /// For search: overrides the geocoder setting. census: US street addresses; google needs an API key.
    #[schemars(extend("enum" = ["apple", "nominatim", "census", "google"]))]
    pub provider: Option<String>,
    #[schemars(range(min = 1, max = 10), extend("default" = 5))]
    pub limit: Option<u64>,
    /// For save and delete: the place's name (any case).
    #[schemars(length(max = 80))]
    pub name: Option<String>,
    /// For save: required for a new place.
    #[schemars(range(min = -90, max = 90))]
    pub lat: Option<f64>,
    #[schemars(range(min = -180, max = 180))]
    pub lon: Option<f64>,
    /// For save: rename the place.
    #[schemars(length(max = 80))]
    pub new_name: Option<String>,
}

// The profile fields a save may set.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ProfileFields {
    /// For example "65 mm whoop".
    pub aircraft: Option<String>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    /// Analog, DJI O4, Walksnail or HDZero.
    pub video_system: Option<String>,
    pub keywords: Option<Vec<String>>,
    pub author: Option<String>,
    /// A saved place name, or null for none.
    #[schemars(extend("x-nullable" = true))]
    pub place: Option<String>,
    /// EdgeTX model names (the start of the radio's log file names).
    pub edgetx_models: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ProfilesArgs {
    #[schemars(required, extend("enum" = ["list", "save", "delete", "set_default"]))]
    pub action: Option<String>,
    #[schemars(length(max = 80))]
    pub name: Option<String>,
    /// For save: rename the profile; the default follows.
    #[schemars(length(max = 80))]
    pub new_name: Option<String>,
    /// For save: the fields to set.
    // Passed on as they are: the core names an unknown field in its error.
    #[schemars(with = "Option<ProfileFields>")]
    pub fields: Option<Value>,
    /// For save: also make it the default.
    pub default: Option<bool>,
}

// The log matching tunables, all required but the clip clock skew (default 300 s).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct TunablesValue {
    pub segment_gap_s: f64,
    pub session_gap_min: f64,
    pub tolerance_s: f64,
    pub max_log_age_days: i64,
    pub clock_skew_s: Option<f64>,
}

// The settings a write may set; null resets one.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct SettingsValues {
    /// Absolute folder path.
    #[schemars(extend("x-nullable" = true))]
    pub output_dir: Option<String>,
    #[schemars(extend("x-nullable" = true, "enum" = ["year_day", "day", "flat", null]))]
    pub layout: Option<String>,
    #[schemars(extend("x-nullable" = true))]
    pub place_folders: Option<bool>,
    #[schemars(extend("x-nullable" = true, "enum" = ["mp4", "mov", null]))]
    pub format: Option<String>,
    #[schemars(extend("x-nullable" = true, "enum" = ["videotoolbox", "x264", null]))]
    pub encoder: Option<String>,
    #[schemars(extend("x-nullable" = true))]
    pub keep_originals: Option<bool>,
    #[schemars(extend("x-nullable" = true))]
    pub add_time: Option<bool>,
    /// After each export, delete the clip files that verified from the card or folder they came from. Other files stay.
    #[schemars(extend("x-nullable" = true))]
    pub delete_clips_after_import: Option<bool>,
    /// Import a recording the DVR split into files (about 600 s each) as one clip. On by default.
    #[schemars(extend("x-nullable" = true))]
    pub join_split_recordings: Option<bool>,
    #[schemars(extend("x-nullable" = true))]
    pub default_name: Option<String>,
    #[schemars(extend("x-nullable" = true))]
    pub photos_album: Option<String>,
    #[schemars(length(max = 11), extend("x-nullable" = true))]
    pub format_label: Option<String>,
    #[schemars(extend("x-nullable" = true))]
    pub log_dir: Option<String>,
    #[schemars(extend("x-nullable" = true))]
    pub tunables: Option<TunablesValue>,
    #[schemars(extend("x-nullable" = true, "enum" = ["apple", "nominatim", "census", "google", null]))]
    pub geocoder: Option<String>,
    /// Google Places API (New) key, for geocoder google. Never read back.
    #[schemars(extend("x-nullable" = true))]
    pub google_places_key: Option<String>,
    /// How the date starts new file names; rename existing clips with quadcam_library_files apply_name_format.
    #[schemars(extend("x-nullable" = true, "enum" = ["YYYY-MM-DD", "YY.MM.DD", null]))]
    pub name_date_format: Option<String>,
    #[schemars(extend("x-nullable" = true))]
    pub default_profile: Option<String>,
    /// Gear: the gear folder (absolute path); null for the default in the support folder.
    #[schemars(extend("x-nullable" = true))]
    pub gear_dir: Option<String>,
    /// Show the Sim page in the Gear sidebar (a preview). Off by default.
    #[schemars(extend("x-nullable" = true))]
    pub sim_preview: Option<bool>,
    /// Gear: back up a device when it is plugged in. On by default.
    #[schemars(extend("x-nullable" = true))]
    pub gear_auto_backup: Option<bool>,
    /// Gear: plug-in and manual backups kept per device before thinning (default 10).
    #[schemars(range(min = 1, max = 1000), extend("x-nullable" = true))]
    pub gear_keep_recent: Option<u64>,
    /// Gear: then one backup a week for this many weeks (default 8).
    #[schemars(range(min = 0, max = 520), extend("x-nullable" = true))]
    pub gear_keep_weeks: Option<u64>,
    /// Gear: then one backup a month, with no limit (default true).
    #[schemars(extend("x-nullable" = true))]
    pub gear_keep_monthly: Option<bool>,
    /// Gear: minutes an FC may run on USB power before a warning; 0 for none (default 20).
    #[schemars(range(min = 0, max = 240), extend("x-nullable" = true))]
    pub gear_usb_minutes: Option<u64>,
    /// Gear: per device kind (fc, radio, elrs_tx, elrs_rx, goggles, dvr_card), the steps that run when it is plugged in: backup, import, apply_ready. Only backup is on by default.
    #[schemars(extend("x-nullable" = true))]
    pub gear_on_connect: Option<std::collections::BTreeMap<String, Vec<String>>>,
    /// Gear: cues, one per job at its end. mute; channels speech, sound, notification; cues safe_to_unplug, still_inserted, step_failed; debounce_s; the reminder reminder_grace_s, still_inserted_every_s, reminder_max; quiet_hours {start, end} (HH:MM, no speech or sound); voice (a say voice name).
    #[schemars(extend("x-nullable" = true))]
    pub gear_cues: Option<std::collections::BTreeMap<String, Value>>,
    #[schemars(extend("x-nullable" = true, "enum" = ["manual", "daily", null]))]
    pub firmware_check: Option<String>,
    /// Gear: the voice provider (default say, macOS).
    #[schemars(extend("x-nullable" = true))]
    pub tts_provider: Option<String>,
    /// Gear: the address of an OpenAI-compatible voice server (for tts_provider openai), such as http://127.0.0.1:8880.
    #[schemars(extend("x-nullable" = true))]
    pub tts_base_url: Option<String>,
    /// Gear: the voice server's model name.
    #[schemars(extend("x-nullable" = true))]
    pub tts_model: Option<String>,
    /// Gear: the voice your own lines render with: a say voice name, or the server's voice id.
    #[schemars(extend("x-nullable" = true))]
    pub tts_voice: Option<String>,
    /// Gear: the address or path of the voice pack index (voices.json).
    #[schemars(extend("x-nullable" = true))]
    pub voice_index: Option<String>,
    /// Gear: the voice provider's API key. Never read back.
    #[schemars(extend("x-nullable" = true))]
    pub tts_key: Option<String>,
    /// Where ffmpeg comes from: QuadCam's module when installed, else Homebrew (module), or Homebrew only.
    #[schemars(extend("x-nullable" = true, "enum" = ["module", "homebrew", null]))]
    pub ffmpeg_source: Option<String>,
    /// {tool: absolute path}: an executable that replaces a module's tool (ffmpeg, ffprobe, esptool).
    #[schemars(extend("x-nullable" = true))]
    pub modules: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct SettingsArgs {
    #[schemars(required, extend("enum" = ["read", "write", "modules", "module_install", "module_remove"]))]
    pub action: Option<String>,
    /// For write: {setting: value}.
    // Passed on as they are: null resets a setting, and the settings file checks each value.
    #[schemars(with = "Option<SettingsValues>")]
    pub values: Option<Value>,
    /// For module_install and module_remove: the module (ffmpeg, esptool).
    pub module: Option<String>,
    /// For module_install: true after the person saw the module's license and agreed.
    pub confirm: Option<bool>,
}
