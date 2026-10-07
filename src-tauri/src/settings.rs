//! The app's settings file, `settings.json` in the support folder: one JSON object with
//! camelCase keys (`outputDir`, `profiles`, `places`, ...). It is the one reader and writer
//! the GUI, the CLI and the MCP server share.
//!
//! Every write reads the file fresh, changes only the keys it was given, keeps every other
//! key (also ones this version does not know), and replaces the file atomically while it
//! holds a lock. Nobody keeps a full copy in memory and writes it back, so a CLI write while
//! the app runs is never lost.

use crate::logs::Tunables;
use crate::media::{Encoder, Format};
use crate::metadata::{Place, Profile};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

pub type Values = Map<String, Value>;

/// One setting: its key in the file, its name on the CLI and in MCP, and the values it
/// takes (as words, for error messages).
pub struct Key {
    pub file: &'static str,
    /// snake_case name for the CLI and MCP; None for keys only the GUI sets.
    pub name: Option<&'static str>,
    pub about: &'static str,
    check: fn(&Value) -> Result<()>,
}

fn any(_: &Value) -> Result<()> {
    Ok(())
}

fn path_or_null(v: &Value) -> Result<()> {
    match v {
        Value::Null => Ok(()),
        Value::String(s) if Path::new(s).is_absolute() => Ok(()),
        _ => bail!("an absolute folder path"),
    }
}

fn string(v: &Value) -> Result<()> {
    v.as_str().map(|_| ()).context("a string")
}

fn yes() -> bool {
    true
}

fn boolean(v: &Value) -> Result<()> {
    v.as_bool().map(|_| ()).context("true or false")
}

fn one_of(v: &Value, allowed: &[&str]) -> Result<()> {
    match v.as_str() {
        Some(s) if allowed.contains(&s) => Ok(()),
        _ => bail!("one of {}", allowed.join(", ")),
    }
}

fn typed<T: serde::de::DeserializeOwned>(v: &Value, what: &str) -> Result<()> {
    serde_json::from_value::<T>(v.clone())
        .map(|_| ())
        .with_context(|| what.to_string())
}

/// `modules`: a tool name (`ffmpeg`, `ffprobe`, `esptool`) to the absolute path of an
/// executable that QuadCam uses instead of its own module or Homebrew.
fn tool_paths(v: &Value) -> Result<()> {
    let m = v.as_object().context("an object")?;
    for p in m.values() {
        match p.as_str() {
            Some(s) if Path::new(s).is_absolute() => {}
            _ => bail!("an absolute file path per tool"),
        }
    }
    Ok(())
}

pub const GEOCODERS: &[&str] = &["apple", "nominatim", "census", "google"];

pub const KEYS: &[Key] = &[
    Key {
        file: "outputDir",
        name: Some("output_dir"),
        about: "an absolute folder path or null",
        check: path_or_null,
    },
    Key {
        file: "format",
        name: Some("format"),
        about: "mp4 or mov",
        check: |v| one_of(v, &["mp4", "mov"]),
    },
    Key {
        file: "encoder",
        name: Some("encoder"),
        about: "videotoolbox or x264",
        check: |v| one_of(v, &["videotoolbox", "x264"]),
    },
    Key {
        file: "keepOriginals",
        name: Some("keep_originals"),
        about: "true or false",
        check: boolean,
    },
    Key {
        file: "addTime",
        name: Some("add_time"),
        about: "true or false",
        check: boolean,
    },
    Key {
        file: "deleteClipsAfterImport",
        name: Some("delete_clips_after_import"),
        about: "true or false",
        check: boolean,
    },
    Key {
        file: "joinSplitRecordings",
        name: Some("join_split_recordings"),
        about: "true or false",
        check: boolean,
    },
    Key {
        file: "defaultName",
        name: Some("default_name"),
        about: "text",
        check: string,
    },
    Key {
        file: "photosAlbum",
        name: Some("photos_album"),
        about: "text (empty: the library only)",
        check: string,
    },
    Key {
        file: "formatLabel",
        name: Some("format_label"),
        about: "a card volume name",
        check: |v| {
            crate::disk::fat_label(v.as_str().context("a string")?)?;
            Ok(())
        },
    },
    Key {
        file: "logDir",
        name: Some("log_dir"),
        about: "an absolute folder path or null",
        check: path_or_null,
    },
    Key {
        file: "libraryLayout",
        name: Some("layout"),
        about: "year_day, day or flat",
        check: |v| one_of(v, &["year_day", "day", "flat"]),
    },
    Key {
        file: "placeFolders",
        name: Some("place_folders"),
        about: "true or false",
        check: boolean,
    },
    Key {
        file: "tunables",
        name: Some("tunables"),
        about: "an object with segment_gap_s, session_gap_min, tolerance_s, max_log_age_days and clock_skew_s",
        check: |v| {
            typed::<crate::logs::Tunables>(
                v,
                "an object with segment_gap_s, session_gap_min, tolerance_s, max_log_age_days and clock_skew_s",
            )
        },
    },
    Key {
        file: "nameDateFormat",
        name: Some("name_date_format"),
        about: "YYYY-MM-DD or YY.MM.DD",
        check: |v| one_of(v, crate::naming::DateFormat::NAMES),
    },
    Key {
        file: "geocoder",
        name: Some("geocoder"),
        about: "apple, nominatim, census or google",
        check: |v| one_of(v, GEOCODERS),
    },
    Key {
        file: "googlePlacesKey",
        name: Some("google_places_key"),
        about: "a Google Places API key",
        check: string,
    },
    // Gear (`gear::GearSettings` reads them).
    Key {
        file: "gearDir",
        name: Some("gear_dir"),
        about: "an absolute folder path or null",
        check: path_or_null,
    },
    Key {
        file: "gearAutoBackup",
        name: Some("gear_auto_backup"),
        about: "true or false",
        check: boolean,
    },
    Key {
        file: "gearKeepRecent",
        name: Some("gear_keep_recent"),
        about: "a whole number from 1 to 1000",
        check: |v| crate::gear::check_count(v, 1, 1000),
    },
    Key {
        file: "gearKeepWeeks",
        name: Some("gear_keep_weeks"),
        about: "a whole number from 0 to 520",
        check: |v| crate::gear::check_count(v, 0, 520),
    },
    Key {
        file: "gearKeepMonthly",
        name: Some("gear_keep_monthly"),
        about: "true or false",
        check: boolean,
    },
    Key {
        file: "gearUsbMinutes",
        name: Some("gear_usb_minutes"),
        about: "a whole number from 0 to 240 (0: no warning)",
        check: |v| crate::gear::check_count(v, 0, 240),
    },
    Key {
        file: "gearOnConnect",
        name: Some("gear_on_connect"),
        about: "an object of device kind (fc, radio, elrs_tx, elrs_rx, goggles, dvr_card) to a list of backup, import, apply_ready",
        check: |v| {
            typed::<std::collections::BTreeMap<crate::gear::model::DeviceKind, Vec<crate::gear::Automation>>>(
                v,
                "an object of device kind to a list of backup, import, apply_ready",
            )
        },
    },
    Key {
        file: "gearCues",
        name: Some("gear_cues"),
        about: "an object with mute, speech, sound, notification, safe_to_unplug, still_inserted, step_failed, unplug_now, debounce_s, reminder_grace_s, still_inserted_every_s, reminder_max, quiet_hours and voice",
        check: |v| {
            if !v.is_object() {
                bail!("an object");
            }
            typed::<crate::gear::cues::CueSettings>(v, "cue settings")
        },
    },
    Key {
        file: "firmwareCheck",
        name: Some("firmware_check"),
        about: "manual or daily",
        check: |v| one_of(v, crate::gear::FIRMWARE_CHECKS),
    },
    Key {
        file: "ttsProvider",
        name: Some("tts_provider"),
        about: "a voice provider name",
        check: string,
    },
    Key {
        file: "ttsKey",
        name: Some("tts_key"),
        about: "a voice provider API key",
        check: string,
    },
    Key {
        file: "ffmpegSource",
        name: Some("ffmpeg_source"),
        about: "module or homebrew",
        check: |v| one_of(v, &["module", "homebrew"]),
    },
    Key {
        file: "modules",
        name: Some("modules"),
        about: "an object of tool name to an absolute file path",
        check: tool_paths,
    },
    Key {
        file: "places",
        name: None,
        about: "a list of {name, lat, lon}",
        check: |v| typed::<Vec<Place>>(v, "a list of {name, lat, lon}"),
    },
    Key {
        file: "profiles",
        name: None,
        about: "a list of profiles",
        check: |v| typed::<Vec<Profile>>(v, "a list of profiles"),
    },
    Key {
        file: "defaultProfile",
        name: Some("default_profile"),
        about: "text (a profile name)",
        check: string,
    },
    Key {
        file: "recents",
        name: None,
        about: "any value",
        check: any,
    },
    Key {
        file: "libView",
        name: None,
        about: "any value",
        check: any,
    },
    Key {
        file: "thumbSize",
        name: None,
        about: "any value",
        check: any,
    },
];

/// The setting with this file key or CLI/MCP name.
pub fn key(name: &str) -> Result<&'static Key> {
    KEYS.iter()
        .find(|k| k.file == name || k.name == Some(name))
        .with_context(|| {
            format!(
                "unknown setting {name:?}; settings: {}",
                KEYS.iter()
                    .filter_map(|k| k.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

/// Checks a value for a setting. Null always passes: it removes the key, so the default
/// applies.
pub fn check(name: &str, v: &Value) -> Result<&'static Key> {
    let k = key(name)?;
    if !v.is_null() {
        if let Err(e) = (k.check)(v) {
            // The detail helps only where the rule is not already in `about`.
            match k.file {
                "formatLabel" | "tunables" | "places" | "profiles" => {
                    bail!("{name} must be {}: {e:#}", k.about)
                }
                _ => bail!("{name} must be {}", k.about),
            }
        }
    }
    Ok(k)
}

/// Keys an older version wrote that mean nothing now (`ui` picked the legacy UI before
/// 0.5.0). Reads drop them, so the next write removes them from the file.
const RETIRED: &[&str] = &["ui"];

/// The file's values. A missing file is empty. A file that does not parse is an error, so a
/// write never replaces settings it could not read.
pub fn read(path: &Path) -> Result<Values> {
    match std::fs::read(path) {
        Ok(b) if b.iter().all(u8::is_ascii_whitespace) => Ok(Values::new()),
        Ok(b) => match serde_json::from_slice::<Value>(&b)
            .with_context(|| format!("{} is not valid JSON; fix or remove it", path.display()))?
        {
            Value::Object(mut m) => {
                m.retain(|k, _| !RETIRED.contains(&k.as_str()));
                Ok(m)
            }
            _ => bail!("{} is not a JSON object; fix or remove it", path.display()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Values::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    path.with_file_name(name)
}

/// Reads the file fresh, lets `edit` change the values, and writes them back when they
/// changed. Holds an exclusive lock on `<file>.lock` throughout, so two processes never
/// interleave. Returns what `edit` returned and the values as written.
pub fn update<T>(path: &Path, edit: impl FnOnce(&mut Values) -> Result<T>) -> Result<(T, Values)> {
    let dir = path.parent().context("settings file has no folder")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let lock = std::fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path(path))
        .context("opening the settings lock")?;
    lock.lock().context("locking the settings file")?;
    let before = read(path)?;
    let mut values = before.clone();
    let out = edit(&mut values)?;
    if values != before {
        let tmp = path.with_extension("json.tmp");
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(&values)?)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
    }
    drop(lock);
    Ok((out, values))
}

/// Settings that hold secrets: reads show only whether they are set.
pub const SECRET_KEYS: &[&str] = &["googlePlacesKey", "ttsKey"];

/// The values with secrets replaced by `"(set)"`.
pub fn redacted(mut values: Values) -> Values {
    for k in SECRET_KEYS {
        if let Some(v) = values.get_mut(*k) {
            *v = Value::String("(set)".into());
        }
    }
    values
}

/// Sets keys (CLI, MCP or file names). Null removes a key, so its default applies. Every
/// value is checked first; one bad value changes nothing.
pub fn set(path: &Path, changes: &Values) -> Result<Values> {
    let mut checked = Vec::new();
    for (name, v) in changes {
        checked.push((check(name, v)?.file, v.clone()));
    }
    Ok(update(path, |values| {
        for (k, v) in checked {
            if v.is_null() {
                values.remove(k);
            } else {
                values.insert(k.to_string(), v);
            }
        }
        Ok(())
    })?
    .1)
}

/// A list setting (`places`, `profiles`), or empty.
pub fn list<T: serde::de::DeserializeOwned>(values: &Values, key: &str) -> Vec<T> {
    values
        .get(key)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

/// Settings every surface starts from. The GUI keeps them in sync with its settings store.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct Defaults {
    pub output_dir: Option<PathBuf>,
    pub format: Format,
    pub encoder: Encoder,
    pub keep_originals: bool,
    pub add_time: bool,
    /// After an import, delete the clip files that verified from the card or folder they
    /// came from. Every other file stays. Off by default; an import may turn it off for
    /// one run, never on.
    #[serde(default)]
    pub delete_clips_after_import: bool,
    /// Import a recording the DVR split into several files as one clip (see `join`). On by
    /// default; a run or a clip may keep the files separate.
    #[serde(default = "yes")]
    pub join_split_recordings: bool,
    pub default_name: String,
    pub photos_album: String,
    pub log_dir: Option<PathBuf>,
    pub tunables: Tunables,
    /// Volume name for the format step.
    #[serde(default = "default_label")]
    pub format_label: String,
    #[serde(default)]
    pub places: Vec<Place>,
    #[serde(default)]
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub default_profile: Option<String>,
    /// How imports are filed in the library folder (`output_dir`).
    #[serde(default)]
    pub layout: crate::library::Layout,
    /// Add the place name to day folders.
    #[serde(default)]
    pub place_folders: bool,
    /// Place search provider: `apple` or `nominatim`.
    #[serde(default = "default_geocoder")]
    pub geocoder: String,
    /// How the date starts file names.
    #[serde(default)]
    pub name_date_format: crate::naming::DateFormat,
    /// Where ffmpeg and ffprobe come from: QuadCam's module when it is installed, else
    /// Homebrew (`module`, the default), or Homebrew only (`homebrew`).
    #[serde(default)]
    pub ffmpeg_source: crate::media::FfmpegSource,
    /// Google Places API key from the settings file. Never serialized.
    #[serde(default, skip_serializing)]
    pub google_places_key: Option<String>,
}

fn default_geocoder() -> String {
    "apple".into()
}

fn default_label() -> String {
    crate::disk::DEFAULT_LABEL.into()
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            output_dir: crate::paths::default_output_dir(),
            format: Format::Mp4,
            encoder: Encoder::Videotoolbox,
            keep_originals: false,
            add_time: false,
            delete_clips_after_import: false,
            join_split_recordings: true,
            default_name: crate::naming::DEFAULT_NAME.into(),
            photos_album: crate::photos::DEFAULT_ALBUM.into(),
            log_dir: None,
            tunables: Tunables::default(),
            format_label: default_label(),
            places: Vec::new(),
            profiles: Vec::new(),
            default_profile: None,
            layout: crate::library::Layout::default(),
            place_folders: false,
            geocoder: default_geocoder(),
            name_date_format: Default::default(),
            ffmpeg_source: Default::default(),
            google_places_key: None,
        }
    }
}

impl Defaults {
    /// Defaults with the app's saved settings on top (the Tauri store file the GUI writes,
    /// `settings.json` in the app's support folder), so a headless CLI or MCP run exports
    /// where the person told the app to. Missing or unreadable keys keep the default.
    pub fn with_app_settings(path: &Path) -> Defaults {
        Defaults::from_values(&read(path).unwrap_or_default())
    }

    /// Defaults with the settings file's values on top.
    pub fn from_values(v: &Values) -> Defaults {
        let mut d = Defaults::default();
        fn get<T: serde::de::DeserializeOwned>(v: &Values, k: &str) -> Option<T> {
            v.get(k)
                .filter(|x| !x.is_null())
                .and_then(|x| serde_json::from_value(x.clone()).ok())
        }
        if let Some(p) = get(v, "outputDir") {
            d.output_dir = Some(p);
        }
        if let Some(f) = get(v, "format") {
            d.format = f;
        }
        if let Some(e) = get(v, "encoder") {
            d.encoder = e;
        }
        if let Some(b) = get(v, "keepOriginals") {
            d.keep_originals = b;
        }
        if let Some(b) = get(v, "addTime") {
            d.add_time = b;
        }
        if let Some(b) = get(v, "deleteClipsAfterImport") {
            d.delete_clips_after_import = b;
        }
        if let Some(b) = get(v, "joinSplitRecordings") {
            d.join_split_recordings = b;
        }
        if let Some(n) = get::<String>(v, "defaultName").filter(|n| !n.trim().is_empty()) {
            d.default_name = n;
        }
        if let Some(a) = get(v, "photosAlbum") {
            d.photos_album = a;
        }
        if let Some(l) = get::<String>(v, "formatLabel").filter(|l| !l.trim().is_empty()) {
            d.format_label = l;
        }
        if let Some(p) = get(v, "logDir") {
            d.log_dir = Some(p);
        }
        if let Some(t) = get(v, "tunables") {
            d.tunables = t;
        }
        if let Some(p) = get(v, "places") {
            d.places = p;
        }
        if let Some(p) = get(v, "profiles") {
            d.profiles = p;
        }
        if let Some(p) = get::<String>(v, "defaultProfile").filter(|p| !p.trim().is_empty()) {
            d.default_profile = Some(p);
        }
        if let Some(l) = get(v, "libraryLayout") {
            d.layout = l;
        }
        if let Some(b) = get(v, "placeFolders") {
            d.place_folders = b;
        }
        d.google_places_key = get::<String>(v, "googlePlacesKey").filter(|k| !k.trim().is_empty());
        if let Some(f) = get(v, "nameDateFormat") {
            d.name_date_format = f;
        }
        if let Some(f) = get(v, "ffmpegSource") {
            d.ffmpeg_source = f;
        }
        if let Some(g) = get::<String>(v, "geocoder").filter(|g| GEOCODERS.contains(&g.as_str())) {
            d.geocoder = g;
        }
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A settings file as quadcam 0.3.0 wrote it.
    const V030: &str = r#"{"profiles":[{"name":"Whoop A","aircraft":"65 mm whoop","camera_make":"Maker","camera_model":"Goggle","video_system":"Analog","keywords":["fpv"],"author":"","place":null,"edgetx_models":["WHOOP A"]}],"defaultProfile":"Whoop A","places":[],"outputDir":"/tmp/x","encoder":"videotoolbox","photosAlbum":"Drone","defaultName":"flight","tunables":{"max_log_age_days":60,"segment_gap_s":5,"session_gap_min":20,"tolerance_s":30},"formatLabel":"ECHO"}"#;

    #[test]
    fn settings_from_0_3_0_carry_over() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("settings.json");
        std::fs::write(&f, V030).unwrap();
        let old: Value = serde_json::from_str(V030).unwrap();

        // The new code reads every value.
        let x = Defaults::with_app_settings(&f);
        assert_eq!(x.output_dir, Some(PathBuf::from("/tmp/x")));
        assert_eq!(x.photos_album, "Drone");
        assert_eq!(x.format_label, "ECHO");
        assert_eq!(x.default_profile.as_deref(), Some("Whoop A"));
        assert_eq!(x.profiles.len(), 1);
        assert_eq!(x.profiles[0].edgetx_models, vec!["WHOOP A"]);
        assert_eq!(x.tunables.max_log_age_days, 60);
        // New settings get their defaults.
        assert_eq!(x.geocoder, "apple");
        assert!(!x.delete_clips_after_import);
        assert!(x.join_split_recordings);

        // A write of one key keeps every other key, byte for byte in value.
        let after = set(
            &f,
            &serde_json::from_value(serde_json::json!({"place_folders": true})).unwrap(),
        )
        .unwrap();
        for (k, v) in old.as_object().unwrap() {
            assert_eq!(after.get(k), Some(v), "{k} survived the write");
        }
        assert_eq!(after["placeFolders"], true);
        let on_disk: Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
        assert_eq!(on_disk, Value::Object(after));

        // Every 0.3.0 value passes the checks a write applies.
        for (k, v) in old.as_object().unwrap() {
            check(k, v).unwrap_or_else(|e| panic!("{k}: {e:#}"));
        }
    }

    #[test]
    fn the_retired_ui_key_loads_and_goes() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("settings.json");
        std::fs::write(&f, r#"{"ui":"next","format":"mov","someFutureKey":1}"#).unwrap();

        // It loads, and the key is gone from every read.
        let values = read(&f).unwrap();
        assert!(!values.contains_key("ui"));
        assert_eq!(Defaults::from_values(&values).format, Format::Mov);
        assert!(check("ui", &Value::String("next".into())).is_err());

        // The next write drops it from the file and keeps the rest.
        set(
            &f,
            &serde_json::from_value(serde_json::json!({"place_folders": true})).unwrap(),
        )
        .unwrap();
        let on_disk: Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
        assert_eq!(
            on_disk,
            serde_json::json!({"format":"mov","someFutureKey":1,"placeFolders":true})
        );
    }

    #[test]
    fn unknown_keys_survive_and_bad_values_change_nothing() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("settings.json");
        std::fs::write(&f, r#"{"fromTheFuture":{"a":1},"format":"mov"}"#).unwrap();
        let bad: Values =
            serde_json::from_value(serde_json::json!({"encoder": "x264", "format": "avi"}))
                .unwrap();
        let e = set(&f, &bad).unwrap_err();
        assert!(
            format!("{e:#}").contains("format must be mp4 or mov"),
            "{e:#}"
        );
        let v = read(&f).unwrap();
        assert_eq!(v["format"], "mov");
        assert!(v.get("encoder").is_none(), "nothing was written");
        let e = set(
            &f,
            &serde_json::from_value(serde_json::json!({"colour": "red"})).unwrap(),
        )
        .unwrap_err();
        assert!(format!("{e:#}").contains("unknown setting"), "{e:#}");
        let v = set(
            &f,
            &serde_json::from_value(serde_json::json!({"format": null, "output_dir": "/tmp/lib"}))
                .unwrap(),
        )
        .unwrap();
        assert!(v.get("format").is_none(), "null removes a key");
        assert_eq!(v["outputDir"], "/tmp/lib");
        assert_eq!(v["fromTheFuture"]["a"], 1);
    }

    #[test]
    fn a_file_that_does_not_parse_is_never_replaced() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("settings.json");
        std::fs::write(&f, "{not json").unwrap();
        assert!(set(
            &f,
            &serde_json::from_value(serde_json::json!({"format": "mov"})).unwrap()
        )
        .is_err());
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "{not json");
    }

    #[test]
    fn concurrent_writers_keep_every_key() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("settings.json");
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let f = f.clone();
                std::thread::spawn(move || {
                    update(&f, |v| {
                        v.insert(format!("k{i}"), Value::from(i));
                        Ok(())
                    })
                    .unwrap();
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let v = read(&f).unwrap();
        assert_eq!(v.len(), 8);
    }

    #[test]
    fn app_settings_override_defaults() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("settings.json");
        assert_eq!(
            Defaults::with_app_settings(&f),
            Defaults::default(),
            "no file: defaults"
        );
        std::fs::write(&f, r#"{"outputDir":"/tmp/out","format":"mov","formatLabel":"FPVCARD","defaultName":"","logDir":null,"tunables":"junk"}"#).unwrap();
        let x = Defaults::with_app_settings(&f);
        assert_eq!(x.output_dir, Some(PathBuf::from("/tmp/out")));
        assert_eq!(x.format, Format::Mov);
        assert_eq!(x.format_label, "FPVCARD");
        assert_eq!(x.default_name, "flight", "empty keeps the default");
        assert_eq!(
            x.tunables,
            Tunables::default(),
            "bad values keep the default"
        );
    }
}
