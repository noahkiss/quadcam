//! The tool list: names, descriptions and annotations written here, and input schemas
//! derived from the argument types in `params.rs`.

use super::params::*;
use schemars::JsonSchema;
use serde_json::{json, Value};

pub(super) const INSTRUCTIONS: &str = "QuadCam imports FPV clips (analog DVR AVI/MJPEG, DJI MP4) into a library folder. \
Import flow: quadcam_status -> quadcam_load_clips (or quadcam_read_clips for a loaded session) -> \
quadcam_read_clips(thumbnails=true) -> quadcam_suggest names, dates, times, places, cuts -> the person may \
edit them in the app -> quadcam_read_clips for the final values -> quadcam_export (when the \
delete_clips_after_import setting is on, it then deletes the clip files that verified from the card; \
keep_clips=true keeps them this run) -> quadcam_add_to_photos -> quadcam_eject. quadcam_format_card erases the card: only on request, after every clip verified; never a DJI card. \
With prep=true it erases a card with no session whose clips are all in the library. \
Library: quadcam_library finds imported clips; quadcam_library_edit changes ratings, names, notes, \
places, aircraft, dates and times; quadcam_library_files handles cuts, Trash and Photos. Setup: \
quadcam_places (search an address or landmark, save it), quadcam_profiles (aircraft gear), \
quadcam_settings (library folder and export defaults). With the app running, every change shows there live.";

/// One tool: its name, its description, its arguments' type, and its annotations.
fn tool<T: JsonSchema>(name: &str, description: &str, annotations: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema::<T>(),
        "annotations": annotations,
    })
}

/// The JSON schema of `T`, inlined, in the plain form the tools have always had: no
/// `$schema`, titles, formats or root description, and null only where a field is marked
/// `x-nullable`.
pub fn input_schema<T: JsonSchema>() -> Value {
    let settings = schemars::generate::SchemaSettings::draft2020_12().with(|s| {
        s.inline_subschemas = true;
        s.meta_schema = None;
    });
    let mut v = settings
        .into_generator()
        .into_root_schema_for::<T>()
        .to_value();
    if let Some(o) = v.as_object_mut() {
        o.remove("title");
        o.remove("description");
        o.entry("properties").or_insert_with(|| json!({}));
    }
    clean(&mut v);
    v
}

/// Tidies one schema and every schema inside it.
fn clean(v: &mut Value) {
    let Some(o) = v.as_object_mut() else { return };
    o.remove("$schema");
    o.remove("title");
    o.remove("format");
    let nullable = o.remove("x-nullable").is_some();
    // `Option<T>` as `anyOf: [T, null]`: T itself, keeping this schema's own keys.
    if let Some(Value::Array(any)) = o.get("anyOf").cloned() {
        let null = any
            .iter()
            .position(|x| x.get("type") == Some(&json!("null")));
        if let (2, Some(n)) = (any.len(), null) {
            o.remove("anyOf");
            if let Some(i) = any[1 - n].as_object() {
                for (k, x) in i {
                    o.entry(k.clone()).or_insert_with(|| x.clone());
                }
            }
            if nullable {
                if let Some(t) = o.get("type").cloned() {
                    o.insert("type".into(), json!([t, "null"]));
                }
            }
        }
    }
    // `Option<T>` as `type: [T, null]`.
    if !nullable {
        if let Some(Value::Array(types)) = o.get("type").cloned() {
            let rest: Vec<Value> = types.into_iter().filter(|t| t != "null").collect();
            o.insert(
                "type".into(),
                match &rest[..] {
                    [one] => one.clone(),
                    _ => Value::Array(rest),
                },
            );
        }
    }
    if let Some(Value::Object(props)) = o.get_mut("properties") {
        for p in props.values_mut() {
            clean(p);
        }
    }
    if let Some(items) = o.get_mut("items") {
        clean(items);
    }
}

/// Tool descriptors: one per step of the import flow, plus the library and setup tools.
pub fn tools() -> Value {
    json!([
        tool::<StatusArgs>(
            "quadcam_status",
            "Show whether the QuadCam app is running (mode \"app\": the person sees every change live) or not (\"headless\"), the detected cards (each with its source, analog or DJI) and radio log sources, the export defaults (including the saved places and aircraft profiles under status.defaults), and a summary of the loaded session.\n\nBest for: the first call, and checking what is inserted.\nReturns: one line of text plus {mode, status, cards, radios}.\nFollow up with quadcam_load_clips to load a card, or quadcam_read_clips when a session is already loaded.",
            json!({"openWorldHint": false, "readOnlyHint": true, "title": "QuadCam status"}),
        ),
        tool::<LibraryArgs>(
            "quadcam_library",
            "List and search the clips already imported into the library folder, newest first: name, date, duration, star rating (0-5), pick or reject flag, place, aircraft, note, keywords, radio-log moments, keep ranges, exported and unsaved cuts, whether it is in Photos, and the file path. Read-only; change clips with quadcam_library_edit and quadcam_library_files. The loaded card session is NOT here; use quadcam_read_clips for that.\n\nBest for: finding earlier flights (\"last week's flips at the field\"), picking the best clips, or checking what the last import added.\nReturns: one line per clip plus structured records with stable `id`s (the DVR content fingerprint), up to `limit` (default 50) with has_more.\nFollow up with quadcam_library_edit to rate, rename or change details, or quadcam_library_files for cuts, Trash and Photos.\nQuery tips: `query` matches all words against name, note, place, aircraft, keywords and file name; `group` narrows to last_import, moments, picks, rejected or not_in_photos; `day` is YYYY-MM-DD.",
            json!({"destructiveHint": false, "idempotentHint": true, "openWorldHint": false, "readOnlyHint": true, "title": "Library"}),
        ),
        tool::<LoadClipsArgs>(
            "quadcam_load_clips",
            "Copy every clip (analog DVR or DJI) off a card or folder to local staging, probe them (recovering half-written files), make thumbnails, and date them from radio logs when a log folder is set. Starts a new session and replaces the old one. A recording an analog DVR split into files of about 600 s (PICT0001.AVI, PICT0002.AVI, ...) loads as one clip made of those files (`files`, `joined`), unless `join` is false or the join_split_recordings setting is off; the later files show `part_of` and import with the first.\n\nBest for: starting an import. Do not call it to re-read a loaded session; use quadcam_read_clips.\nReturns: a line per clip (id, name, duration, status, date, name).\nFollow up with quadcam_read_clips(thumbnails=true) to see the clips.",
            json!({"destructiveHint": false, "idempotentHint": false, "openWorldHint": false, "readOnlyHint": false, "title": "Load clips"}),
        ),
        tool::<ReadClipsArgs>(
            "quadcam_read_clips",
            "Read the loaded clips: duration, frames, status (ok / incomplete / empty), the planned date with its source and radio-log match (matched / likely / unmatched), short name, note, skip, which values an agent suggested, and import results. Also each clip's moments, in clip seconds: rolls, flips, punch-outs, dives and possible crashes from the radio log's sticks (scored 0..1; a 0.5 s log interval scores lower and its times are rough), and dead air from the video (blue no-signal screen, static, test pattern, black, 3 s or longer). `keep` holds the suggested ranges without dead air, and `cuts` the ranges that will export as extra files. `metadata` holds the clip's own profile, location, keywords and author; `log_model` is the EdgeTX model of its log (it picks the profile when the clip has none) and `flight` the log's numbers (armed time, packs, min RxBt, LQ, RSSI, max throttle). Optionally returns each clip's thumbnail as an image.\n\nBest for: looking at the footage before suggesting names or cuts, and reading back the values the person settled on before export.\nReturns: a line per clip plus structured records; with thumbnails=true, one JPEG per clip (up to max_thumbnails).\nFollow up with quadcam_suggest to propose names or dates, or quadcam_export when the values are final.",
            json!({"openWorldHint": false, "readOnlyHint": true, "title": "Read clips"}),
        ),
        tool::<MatchLogsArgs>(
            "quadcam_match_logs",
            "Date the clips from EdgeTX radio logs: rows split into armed segments and sessions, walked against the clips in PICT order. A log day before 2020 or over 60 days from today counts as a radio clock reset and falls back to the import date. Dates the person or an agent edited are kept.\n\nBest for: after loading, when a radio or a copy of its LOGS folder is available.\nReturns: the log day used, the days available, warnings, and each clip's date and match badge.",
            json!({"destructiveHint": false, "idempotentHint": true, "openWorldHint": false, "readOnlyHint": false, "title": "Match radio logs"}),
        ),
        tool::<SuggestArgs>(
            "quadcam_suggest",
            "Suggest a short name, date, time of day, note, skip or cut ranges for clips. The values are marked agent-suggested, and in the app they appear as editable suggestions the person can accept or change. Names become the filename slug (YYYY-MM-DD_<name>.mp4, lowercased); an empty name uses the default name (\"flight\"), auto-numbered. `cuts` replaces the clip's cut list; each range exports as an extra file <name>_cutN next to the clip (an empty list removes them). `split_by_flight` true adds one cut per radio-log pack (each pack's armed range plus up to 2 s each side). `joined` on the first file of a split recording (a clip with `files`) imports its files as one clip (true) or as clips of their own (false); the clips are matched to the radio logs again. `log_offset_s` says where the first armed log row falls in the clip and moves the log moments. Metadata: `profile` (an aircraft profile name from quadcam_status; empty string to fall back to the log's model, then the default), `place` (a saved place name; empty string removes the location) or `location` {lat, lon}, `keywords` (replaces the clip's own; FPV, the profile's and the moment kinds are added at export), `author`. To apply one value to every clip, send one suggestion per clip id.\n\nBest for: proposing names from what the thumbnails show, dates and times from a clock burned into the video, and cuts from moments or the keep ranges.\nReturns: every clip's current plan.\nFollow up with quadcam_read_clips to read the final values before quadcam_export; the person may have changed them.",
            json!({"destructiveHint": false, "idempotentHint": true, "openWorldHint": false, "readOnlyHint": false, "title": "Suggest names and dates"}),
        ),
        tool::<ExportArgs>(
            "quadcam_export",
            "Convert every non-skipped clip (MP4 H.264 by default, or a lossless MOV remux), write metadata, and verify each output (frame count, duration, streams, metadata) before it counts. Never overwrites: duplicate names get -2, -3. Each cut range also exports as <name>_cutN (re-encoded from the source, frame-exact) and is verified. Clips and cuts that already verified are not written again, so call it again after adding cuts. Options left out use the app's settings (in app mode) or the defaults (output ~/Movies/quadcam). When the delete_clips_after_import setting is on, it then deletes each clip file whose output verified from the card or folder it came from; skipped and failed clips, sidecars and every other file stay. keep_clips=true keeps every clip this run; nothing here turns deletion on.\n\nBest for: after the names and dates are final.\nReturns: imported / skipped / failed counts, each clip's output path, whether the card format step is unlocked, and, when deletion ran, each clip deleted or kept with the reason.\nFollow up with quadcam_add_to_photos, then quadcam_eject.",
            json!({"destructiveHint": true, "idempotentHint": true, "openWorldHint": false, "readOnlyHint": false, "title": "Export clips"}),
        ),
        tool::<VerifyArgs>(
            "quadcam_verify",
            "Re-check exported files against their sources: frame count, duration within 0.1 s, one video stream, audio when the source had it, and the metadata written.\n\nBest for: confirming outputs are intact later on. quadcam_export already verifies every file it writes.\nReturns: one report per output, ok or the reason it failed.",
            json!({"openWorldHint": false, "readOnlyHint": true, "title": "Verify outputs"}),
        ),
        tool::<AddToPhotosArgs>(
            "quadcam_add_to_photos",
            "Add verified outputs to the macOS Photos library, into an album (default \"Drone\"). macOS asks the person for permission the first time.\n\nBest for: after quadcam_export, unless it already ran with add_to_photos=true.\nReturns: the files added and any that failed, with the reason.",
            json!({"destructiveHint": false, "idempotentHint": false, "openWorldHint": false, "readOnlyHint": false, "title": "Add to Photos"}),
        ),
        tool::<EjectArgs>(
            "quadcam_eject",
            "Eject the session's card (or another mount point or /dev/diskN) with diskutil, so it can be pulled safely.\n\nBest for: the last step after export.\nReturns: {ejected: true}.",
            json!({"destructiveHint": false, "idempotentHint": true, "openWorldHint": false, "readOnlyHint": false, "title": "Eject card"}),
        ),
        tool::<FormatCardArgs>(
            "quadcam_format_card",
            "ERASE the session's card as FAT32 and eject it. Only when the person asked for it. It refuses a DJI card (goggles format their own) and a radio's SD card, and refuses unless every non-skipped clip verified, the card is removable, not internal, not the boot disk, 64 GB or smaller, and still the same card (device and volume UUID). It also needs device, volume_uuid and confirm=true, and when the app is running the person must click Erase in the app. With prep=true it erases a card with no session instead (card prep: a new card, or one whose clips are all in the library; a clip missing from the library refuses), found by `mount` in the dry run and by device and volume_uuid to erase.\n\nBest for: clearing the card after a verified export, or preparing a spare DVR card (prep=true). Call with dry_run=true first to read the device and volume UUID.\nReturns: the disk that was erased, or the reason it refused.",
            json!({"destructiveHint": true, "idempotentHint": false, "openWorldHint": false, "readOnlyHint": false, "title": "Format card"}),
        ),
        tool::<LibraryEditArgs>(
            "quadcam_library_edit",
            "Change library clips already imported: star rating, pick/reject flag, short name, note, keywords, author, place, aircraft profile, date and time of day. Every change is written into the clip's file (and its cut files) and the index. Several fields can change in one call; each applies to every id, except `name`, which needs exactly one id.\n\nBest for: rating and flagging after a review, fixing a wrong date or time, moving clips to the right aircraft after export, renaming.\nNot for: the loaded card session (use quadcam_suggest) or cuts, Trash and Photos (use quadcam_library_files).\nEffects: `name` renames the file, its cuts and its original. `date` moves the clip, its cuts and its original to that day's folder (renaming them when the file name starts with the date) and keeps the time unless `time` is given. `time` rewrites the QuickTime creation date and the file times. `profile` rewrites make, model, aircraft, video system and swaps the old profile's keywords for the new one's.\nReturns: the changed clips.",
            json!({"destructiveHint": false, "idempotentHint": true, "openWorldHint": false, "readOnlyHint": false, "title": "Edit library clips"}),
        ),
        tool::<LibraryFilesArgs>(
            "quadcam_library_files",
            "Work on library clip files: set a clip's cut ranges (`cuts`), add one cut per radio-log pack (`split_by_flight`: each pack's armed range plus up to 2 s each side; fails when the clip has no packs, or one pack that covers nearly all of it; clips imported before 0.6.3 need `match_logs` with `apply` first), write unsaved cuts as files (`export_cuts`), move clips with their cuts and originals to the Trash (`trash`), add clips and their cuts to Photos (`photos`), rebuild the index from the files (`rebuild`, after files were changed outside quadcam), rename clips so their file names start with the date in the name_date_format setting (`apply_name_format`; ids, or none for every clip), or match radio logs to clips already imported (`match_logs`; ids, or none for every clip: reports matched / likely / unmatched with a reason, and writes flight numbers and moments only with `apply` true; dates never change).\n\nBest for: trimming a library clip into keeper cuts, clearing out rejects (only when the person asked), sharing to Photos.\nNot for: names, ratings, dates or other details (use quadcam_library_edit).\nQuery tips: `cuts` replaces the clip's cut list in clip seconds (an empty list removes every cut). Dropping a cut that is already a file needs `removed_cuts` (\"keep\": the file stays as a clip of its own; \"trash\"); ask the person which. Set `export` true with `cuts` or `split_by_flight` to write the new cuts in the same call.\nReturns: what changed, with file paths.",
            json!({"destructiveHint": true, "idempotentHint": false, "openWorldHint": false, "readOnlyHint": false, "title": "Library files"}),
        ),
        tool::<PlacesArgs>(
            "quadcam_places",
            "Saved places (name, latitude, longitude) that clips and aircraft profiles use as their location. `list` them, `search` for an address or a named place (a landmark, park or field) to get its coordinates, `save` one (creates it, or updates the place with that name; `new_name` renames it and profiles follow), or `delete` one (profiles that used it lose their default place; clips keep the location written into them).\n\nBest for: adding the field the person flies at before tagging clips with `place`.\nQuery tips: search with a full address or a well-known name; pick the right result from the list (name, address, lat, lon), then save it with the person's name for it. The search uses the geocoder setting (apple: Apple Maps, the default; nominatim: OpenStreetMap; census: US Census, US street addresses only; google: Google Places, needs an API key). When apple or nominatim finds nothing, the US Census geocoder is tried. Search only on request, never per keystroke.\nReturns: the places, or the search results.\nFollow up with quadcam_suggest or quadcam_library_edit with `place` to tag clips.",
            json!({"destructiveHint": true, "idempotentHint": false, "openWorldHint": true, "readOnlyHint": false, "title": "Saved places"}),
        ),
        tool::<ProfilesArgs>(
            "quadcam_profiles",
            "Aircraft profiles: the gear written into each clip (aircraft, camera make and model from the goggles or DVR, video system, keywords, author), a default place, and the EdgeTX model names that pick the profile when a radio log matches. `list` them, `save` one (creates it, or changes only the given fields of the profile with that name; `new_name` renames it), `delete` one, or `set_default` (the profile for clips without a log match; empty name for none).\n\nBest for: setting up a new quad, or fixing gear details before an import.\nNot for: changing the profile of clips already imported (use quadcam_library_edit with `profile`) or of the loaded session (quadcam_suggest).\nReturns: every profile and the default.",
            json!({"destructiveHint": true, "idempotentHint": false, "openWorldHint": false, "readOnlyHint": false, "title": "Aircraft profiles"}),
        ),
        tool::<SettingsArgs>(
            "quadcam_settings",
            "Read or write the app's settings, the same file the app's Settings window uses: library folder (`output_dir`) and its `layout` (year_day, day, flat) and `place_folders`, export `format` (mp4, mov), `encoder` (videotoolbox, x264), `keep_originals`, `add_time` (HHMM in names of clips with a time), `delete_clips_after_import` (after each export, delete the clip files that verified from the card or folder; other files stay; off by default), `join_split_recordings` (import a recording the DVR split into files as one clip; on by default), `default_name`, `photos_album` (empty: library only), card `format_label`, radio `log_dir`, log matching `tunables`, place search `geocoder` (apple, nominatim, census, google) and its `google_places_key` (write-only), file-name `name_date_format` (YYYY-MM-DD, YY.MM.DD) and `default_profile`. A write changes only the given settings; null resets one to its default.\n\nBest for: pointing the library somewhere else, or changing export defaults the person asked for.\nNot for: places and profiles (quadcam_places, quadcam_profiles).\nReturns: the settings file's path and every effective setting.",
            json!({"destructiveHint": false, "idempotentHint": true, "openWorldHint": false, "readOnlyHint": false, "title": "Settings"}),
        ),
    ])
}
