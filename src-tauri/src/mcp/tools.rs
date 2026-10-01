//! The tool list: names, descriptions, input schemas and annotations.

use serde_json::{json, Value};

pub(super) const INSTRUCTIONS: &str = "QuadCam imports analog FPV DVR clips (AVI/MJPEG) into a library folder. \
Import flow: quadcam_status -> quadcam_load_clips (or quadcam_read_clips for a loaded session) -> \
quadcam_read_clips(thumbnails=true) -> quadcam_suggest names, dates, times, places, cuts -> the person may \
edit them in the app -> quadcam_read_clips for the final values -> quadcam_export -> quadcam_add_to_photos \
-> quadcam_eject. quadcam_format_card erases the card: only on request, after every clip verified. \
Library: quadcam_library finds imported clips; quadcam_library_edit changes ratings, names, notes, \
places, aircraft, dates and times; quadcam_library_files handles cuts, Trash and Photos. Setup: \
quadcam_places (search an address or landmark, save it), quadcam_profiles (aircraft gear), \
quadcam_settings (library folder and export defaults). With the app running, every change shows there live.";

/// Tool descriptors: one per step of the import flow, plus the library and setup tools.
pub fn tools() -> Value {
    let ids = json!({"type": "array", "items": {"type": "integer", "minimum": 0}, "description": "Clip ids from quadcam_read_clips. Omit for all clips."});
    let mut flow = json!([
        {
            "name": "quadcam_status",
            "description": "Show whether the QuadCam app is running (mode \"app\": the person sees every change live) or not (\"headless\"), the detected DVR cards and radio log sources, the export defaults (including the saved places and aircraft profiles under status.defaults), and a summary of the loaded session.\n\nBest for: the first call, and checking what is inserted.\nReturns: one line of text plus {mode, status, cards, radios}.\nFollow up with quadcam_load_clips to load a card, or quadcam_read_clips when a session is already loaded.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": {"title": "QuadCam status", "readOnlyHint": true, "openWorldHint": false}
        },
        {
            "name": "quadcam_library",
            "description": "List and search the clips already imported into the library folder, newest first: name, date, duration, star rating (0-5), pick or reject flag, place, aircraft, note, keywords, radio-log moments, keep ranges, exported and unsaved cuts, whether it is in Photos, and the file path. Read-only; change clips with quadcam_library_edit and quadcam_library_files. The loaded card session is NOT here; use quadcam_read_clips for that.\n\nBest for: finding earlier flights (\"last week's flips at the field\"), picking the best clips, or checking what the last import added.\nReturns: one line per clip plus structured records with stable `id`s (the DVR content fingerprint), up to `limit` (default 50) with has_more.\nFollow up with quadcam_library_edit to rate, rename or change details, or quadcam_library_files for cuts, Trash and Photos.\nQuery tips: `query` matches all words against name, note, place, aircraft, keywords and file name; `group` narrows to last_import, moments, picks, rejected or not_in_photos; `day` is YYYY-MM-DD.",
            "inputSchema": {"type": "object", "properties": {"query": {"type": "string", "maxLength": 200}, "group": {"type": "string", "enum": ["all", "last_import", "moments", "picks", "rejected", "not_in_photos"]}, "day": {"type": "string", "pattern": "^\\d{4}-\\d{2}-\\d{2}$"}, "place": {"type": "string", "description": "Saved place name."}, "aircraft": {"type": "string", "description": "Aircraft profile name."}, "min_rating": {"type": "integer", "minimum": 0, "maximum": 5}, "limit": {"type": "integer", "minimum": 1, "maximum": 500}}, "additionalProperties": false},
            "annotations": {"title": "Library", "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        },
        {
            "name": "quadcam_load_clips",
            "description": "Copy every DVR clip off a card or folder to local staging, probe them (recovering half-written files), make thumbnails, and date them from radio logs when a log folder is set. Starts a new session and replaces the old one.\n\nBest for: starting an import. Do not call it to re-read a loaded session; use quadcam_read_clips.\nReturns: a line per clip (id, name, duration, status, date, name).\nFollow up with quadcam_read_clips(thumbnails=true) to see the clips.",
            "inputSchema": {"type": "object", "properties": {"source": {"type": "string", "description": "Card mount point (e.g. /Volumes/NO NAME) or a folder of AVI files. Omit to use the first detected card."}}, "additionalProperties": false},
            "annotations": {"title": "Load clips", "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false}
        },
        {
            "name": "quadcam_read_clips",
            "description": "Read the loaded clips: duration, frames, status (ok / incomplete / empty), the planned date with its source and radio-log match (matched / likely / unmatched), short name, note, skip, which values an agent suggested, and import results. Also each clip's moments, in clip seconds: rolls, flips, punch-outs, dives and possible crashes from the radio log's sticks (scored 0..1; a 0.5 s log interval scores lower and its times are rough), and dead air from the video (blue no-signal screen, static, test pattern, black, 3 s or longer). `keep` holds the suggested ranges without dead air, and `cuts` the ranges that will export as extra files. `metadata` holds the clip's own profile, location, keywords and author; `log_model` is the EdgeTX model of its log (it picks the profile when the clip has none) and `flight` the log's numbers (armed time, packs, min RxBt, LQ, RSSI, max throttle). Optionally returns each clip's thumbnail as an image.\n\nBest for: looking at the footage before suggesting names or cuts, and reading back the values the person settled on before export.\nReturns: a line per clip plus structured records; with thumbnails=true, one JPEG per clip (up to max_thumbnails).\nFollow up with quadcam_suggest to propose names or dates, or quadcam_export when the values are final.",
            "inputSchema": {"type": "object", "properties": {"ids": ids, "thumbnails": {"type": "boolean", "default": false, "description": "Attach the first-frame thumbnail of each clip as an image."}, "max_thumbnails": {"type": "integer", "minimum": 1, "maximum": 50, "default": 12}}, "additionalProperties": false},
            "annotations": {"title": "Read clips", "readOnlyHint": true, "openWorldHint": false}
        },
        {
            "name": "quadcam_match_logs",
            "description": "Date the clips from EdgeTX radio logs: rows split into armed segments and sessions, walked against the clips in PICT order. A log day before 2020 or over 60 days from today counts as a radio clock reset and falls back to the import date. Dates the person or an agent edited are kept.\n\nBest for: after loading, when a radio or a copy of its LOGS folder is available.\nReturns: the log day used, the days available, warnings, and each clip's date and match badge.",
            "inputSchema": {"type": "object", "properties": {"log_dir": {"type": "string", "description": "EdgeTX LOGS folder, or the radio's root when it is mounted in USB storage mode."}, "day": {"type": "string", "pattern": "^\\d{4}-\\d{2}-\\d{2}$", "description": "Log day to match. Omit for the newest plausible day."}, "no_logs": {"type": "boolean", "description": "Stop using logs; every clip gets the import date."}}, "additionalProperties": false},
            "annotations": {"title": "Match radio logs", "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        },
        {
            "name": "quadcam_suggest",
            "description": "Suggest a short name, date, time of day, note, skip or cut ranges for clips. The values are marked agent-suggested, and in the app they appear as editable suggestions the person can accept or change. Names become the filename slug (YYYY-MM-DD_<name>.mp4, lowercased); an empty name uses the default name (\"flight\"), auto-numbered. `cuts` replaces the clip's cut list; each range exports as an extra file <name>_cutN next to the clip (an empty list removes them). `log_offset_s` says where the first armed log row falls in the clip and moves the log moments. Metadata: `profile` (an aircraft profile name from quadcam_status; empty string to fall back to the log's model, then the default), `place` (a saved place name; empty string removes the location) or `location` {lat, lon}, `keywords` (replaces the clip's own; FPV, the profile's and the moment kinds are added at export), `author`. To apply one value to every clip, send one suggestion per clip id.\n\nBest for: proposing names from what the thumbnails show, dates and times from a clock burned into the video, and cuts from moments or the keep ranges.\nReturns: every clip's current plan.\nFollow up with quadcam_read_clips to read the final values before quadcam_export; the person may have changed them.",
            "inputSchema": {"type": "object", "required": ["suggestions"], "properties": {"suggestions": {"type": "array", "minItems": 1, "items": {"type": "object", "required": ["id"], "properties": {"id": {"type": "integer", "minimum": 0}, "name": {"type": "string", "maxLength": 80}, "date": {"type": "string", "pattern": "^\\d{4}-\\d{2}-\\d{2}$"}, "time": {"type": "string", "pattern": "^(\\d{2}:\\d{2}(:\\d{2})?)?$", "description": "Time of day, HH:MM 24-hour; empty string for local noon. A new date without a time resets it to noon."}, "note": {"type": "string"}, "skip": {"type": "boolean"}, "cuts": {"type": "array", "maxItems": 20, "items": {"type": "object", "required": ["start", "end"], "properties": {"start": {"type": "number", "minimum": 0, "description": "Seconds into the clip."}, "end": {"type": "number", "minimum": 0}}, "additionalProperties": false}, "description": "Ranges to export as extra files, each at least 0.5 s."}, "log_offset_s": {"type": "number", "description": "Seconds into the clip where the radio log's first armed row falls (the DVR usually starts before arming)."}, "profile": {"type": "string", "maxLength": 80}, "place": {"type": "string", "maxLength": 80, "description": "Saved place name; empty string removes the location."}, "location": {"type": "object", "required": ["lat", "lon"], "properties": {"lat": {"type": "number", "minimum": -90, "maximum": 90}, "lon": {"type": "number", "minimum": -180, "maximum": 180}}, "additionalProperties": false}, "keywords": {"type": "array", "maxItems": 30, "items": {"type": "string", "maxLength": 60}}, "author": {"type": "string", "maxLength": 120}, "reason": {"type": "string", "description": "One short line on why, shown to the person."}, "removed_cuts": {"type": "string", "enum": ["keep", "trash"], "description": "Required when `cuts` drops a cut that was already exported: keep its file (it becomes a clip of its own) or move it to the Trash. Ask the person which."}}, "additionalProperties": false}}}, "additionalProperties": false},
            "annotations": {"title": "Suggest names and dates", "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        },
        {
            "name": "quadcam_export",
            "description": "Convert every non-skipped clip (MP4 H.264 by default, or a lossless MOV remux), write metadata, and verify each output (frame count, duration, streams, metadata) before it counts. Never overwrites: duplicate names get -2, -3. Each cut range also exports as <name>_cutN (re-encoded from the source, frame-exact) and is verified. Clips and cuts that already verified are not written again, so call it again after adding cuts. Options left out use the app's settings (in app mode) or the defaults (output ~/Movies/quadcam).\n\nBest for: after the names and dates are final.\nReturns: imported / skipped / failed counts, each clip's output path, and whether the card format step is unlocked.\nFollow up with quadcam_add_to_photos, then quadcam_eject.",
            "inputSchema": {"type": "object", "properties": {"output_dir": {"type": "string"}, "format": {"type": "string", "enum": ["mp4", "mov"]}, "keep_originals": {"type": "boolean", "description": "Also copy each source AVI into <output>/originals/."}, "add_time": {"type": "boolean", "description": "Add HHMM to names of clips dated from a radio log."}, "add_to_photos": {"type": "boolean", "description": "Add the verified outputs to Photos afterwards."}, "album": {"type": "string", "description": "Photos album; empty string for the library only."}}, "additionalProperties": false},
            "annotations": {"title": "Export clips", "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        },
        {
            "name": "quadcam_verify",
            "description": "Re-check exported files against their sources: frame count, duration within 0.1 s, one video stream, audio when the source had it, and the metadata written.\n\nBest for: confirming outputs are intact later on. quadcam_export already verifies every file it writes.\nReturns: one report per output, ok or the reason it failed.",
            "inputSchema": {"type": "object", "properties": {"ids": ids}, "additionalProperties": false},
            "annotations": {"title": "Verify outputs", "readOnlyHint": true, "openWorldHint": false}
        },
        {
            "name": "quadcam_add_to_photos",
            "description": "Add verified outputs to the macOS Photos library, into an album (default \"Drone\"). macOS asks the person for permission the first time.\n\nBest for: after quadcam_export, unless it already ran with add_to_photos=true.\nReturns: the files added and any that failed, with the reason.",
            "inputSchema": {"type": "object", "properties": {"ids": ids, "album": {"type": "string", "description": "Album name; empty string for the library only. Omit for the default."}}, "additionalProperties": false},
            "annotations": {"title": "Add to Photos", "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false}
        },
        {
            "name": "quadcam_eject",
            "description": "Eject the session's card (or another mount point or /dev/diskN) with diskutil, so it can be pulled safely.\n\nBest for: the last step after export.\nReturns: {ejected: true}.",
            "inputSchema": {"type": "object", "properties": {"target": {"type": "string", "description": "Mount point or /dev/diskN. Omit for the session's card."}}, "additionalProperties": false},
            "annotations": {"title": "Eject card", "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        },
        {
            "name": "quadcam_format_card",
            "description": "ERASE the session's card as FAT32 and eject it. Only when the person asked for it. It refuses unless every non-skipped clip verified, the card is removable, not internal, not the boot disk, 64 GB or smaller, and still the same card (device and volume UUID). It also needs device, volume_uuid and confirm=true, and when the app is running the person must click Erase in the app.\n\nBest for: clearing the card after a verified export. Call with dry_run=true first to read the device and volume UUID.\nReturns: the disk that was erased, or the reason it refused.",
            "inputSchema": {"type": "object", "properties": {"dry_run": {"type": "boolean", "description": "Run every guard and return the plan; erase nothing."}, "device": {"type": "string", "pattern": "^/dev/disk[0-9]+$", "description": "Whole-disk device from the dry run, e.g. /dev/disk4."}, "volume_uuid": {"type": "string", "description": "Volume UUID from the dry run."}, "label": {"type": "string", "maxLength": 11, "description": "FAT32 volume name, default DVR."}, "confirm": {"type": "boolean", "description": "Must be true to erase."}}, "additionalProperties": false},
            "annotations": {"title": "Format card", "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": false}
        }
    ]);
    // The library and setup tools.
    let more = json!([
        {
            "name": "quadcam_library_edit",
            "description": "Change library clips already imported: star rating, pick/reject flag, short name, note, keywords, author, place, aircraft profile, date and time of day. Every change is written into the clip's file (and its cut files) and the index. Several fields can change in one call; each applies to every id, except `name`, which needs exactly one id.\n\nBest for: rating and flagging after a review, fixing a wrong date or time, moving clips to the right aircraft after export, renaming.\nNot for: the loaded card session (use quadcam_suggest) or cuts, Trash and Photos (use quadcam_library_files).\nEffects: `name` renames the file, its cuts and its original. `date` moves the clip, its cuts and its original to that day's folder (renaming them when the file name starts with the date) and keeps the time unless `time` is given. `time` rewrites the QuickTime creation date and the file times. `profile` rewrites make, model, aircraft, video system and swaps the old profile's keywords for the new one's.\nReturns: the changed clips.",
            "inputSchema": {"type": "object", "required": ["ids"], "properties": {
                "ids": {"type": "array", "minItems": 1, "maxItems": 500, "items": {"type": "string"}, "description": "Library clip ids from quadcam_library."},
                "rating": {"type": "integer", "minimum": 0, "maximum": 5, "description": "Stars; 0 clears."},
                "flag": {"type": "string", "enum": ["pick", "reject", "none"]},
                "name": {"type": "string", "maxLength": 80, "description": "New short name; one id only."},
                "note": {"type": "string"},
                "keywords": {"type": "array", "maxItems": 30, "items": {"type": "string", "maxLength": 60}, "description": "Replaces the clip's keywords."},
                "author": {"type": "string", "maxLength": 120},
                "place": {"type": "string", "maxLength": 80, "description": "Saved place name (quadcam_places); empty string removes the location."},
                "location": {"type": "object", "required": ["lat", "lon"], "properties": {"lat": {"type": "number", "minimum": -90, "maximum": 90}, "lon": {"type": "number", "minimum": -180, "maximum": 180}}, "additionalProperties": false},
                "profile": {"type": "string", "maxLength": 80, "description": "Aircraft profile name (quadcam_profiles); empty string removes the profile's details."},
                "date": {"type": "string", "pattern": "^\\d{4}-\\d{2}-\\d{2}$", "description": "New flying day."},
                "time": {"type": "string", "pattern": "^(\\d{2}:\\d{2}(:\\d{2})?)?$", "description": "Time of day, HH:MM 24-hour; empty string for local noon."}
            }, "additionalProperties": false},
            "annotations": {"title": "Edit library clips", "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        },
        {
            "name": "quadcam_library_files",
            "description": "Work on library clip files: set a clip's cut ranges (`cuts`), write unsaved cuts as files (`export_cuts`), move clips with their cuts and originals to the Trash (`trash`), add clips and their cuts to Photos (`photos`), rebuild the index from the files (`rebuild`, after files were changed outside quadcam), or rename clips so their file names start with the date in the name_date_format setting (`apply_name_format`; ids, or none for every clip).\n\nBest for: trimming a library clip into keeper cuts, clearing out rejects (only when the person asked), sharing to Photos.\nNot for: names, ratings, dates or other details (use quadcam_library_edit).\nQuery tips: `cuts` replaces the clip's cut list in clip seconds (an empty list removes every cut). Dropping a cut that is already a file needs `removed_cuts` (\"keep\": the file stays as a clip of its own; \"trash\"); ask the person which. Set `export` true to write the new cuts in the same call.\nReturns: what changed, with file paths.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["cuts", "export_cuts", "trash", "photos", "rebuild", "apply_name_format"]},
                "ids": {"type": "array", "maxItems": 500, "items": {"type": "string"}, "description": "Library clip ids from quadcam_library. cuts and export_cuts take exactly one; rebuild takes none; apply_name_format takes some or none (every clip)."},
                "cuts": {"type": "array", "maxItems": 20, "items": {"type": "object", "required": ["start", "end"], "properties": {"start": {"type": "number", "minimum": 0}, "end": {"type": "number", "minimum": 0}}, "additionalProperties": false}, "description": "For cuts: ranges in clip seconds, each at least 0.5 s."},
                "removed_cuts": {"type": "string", "enum": ["keep", "trash"]},
                "export": {"type": "boolean", "description": "For cuts: also write the new cuts as files."},
                "album": {"type": "string", "description": "For photos: album name; empty string for the library only. Omit for the default."}
            }, "additionalProperties": false},
            "annotations": {"title": "Library files", "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": false}
        },
        {
            "name": "quadcam_places",
            "description": "Saved places (name, latitude, longitude) that clips and aircraft profiles use as their location. `list` them, `search` for an address or a named place (a landmark, park or field) to get its coordinates, `save` one (creates it, or updates the place with that name; `new_name` renames it and profiles follow), or `delete` one (profiles that used it lose their default place; clips keep the location written into them).\n\nBest for: adding the field the person flies at before tagging clips with `place`.\nQuery tips: search with a full address or a well-known name; pick the right result from the list (name, address, lat, lon), then save it with the person's name for it. The search uses the geocoder setting (apple: Apple Maps, the default; nominatim: OpenStreetMap; census: US Census, US street addresses only; google: Google Places, needs an API key). When apple or nominatim finds nothing, the US Census geocoder is tried. Search only on request, never per keystroke.\nReturns: the places, or the search results.\nFollow up with quadcam_suggest or quadcam_library_edit with `place` to tag clips.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["list", "search", "save", "delete"]},
                "query": {"type": "string", "maxLength": 200, "description": "For search: an address or a place name."},
                "provider": {"type": "string", "enum": ["apple", "nominatim", "census", "google"], "description": "For search: overrides the geocoder setting. census: US street addresses; google needs an API key."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 10, "default": 5},
                "name": {"type": "string", "maxLength": 80, "description": "For save and delete: the place's name (any case)."},
                "lat": {"type": "number", "minimum": -90, "maximum": 90, "description": "For save: required for a new place."},
                "lon": {"type": "number", "minimum": -180, "maximum": 180},
                "new_name": {"type": "string", "maxLength": 80, "description": "For save: rename the place."}
            }, "additionalProperties": false},
            "annotations": {"title": "Saved places", "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true}
        },
        {
            "name": "quadcam_profiles",
            "description": "Aircraft profiles: the gear written into each clip (aircraft, camera make and model from the goggles or DVR, video system, keywords, author), a default place, and the EdgeTX model names that pick the profile when a radio log matches. `list` them, `save` one (creates it, or changes only the given fields of the profile with that name; `new_name` renames it), `delete` one, or `set_default` (the profile for clips without a log match; empty name for none).\n\nBest for: setting up a new quad, or fixing gear details before an import.\nNot for: changing the profile of clips already imported (use quadcam_library_edit with `profile`) or of the loaded session (quadcam_suggest).\nReturns: every profile and the default.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["list", "save", "delete", "set_default"]},
                "name": {"type": "string", "maxLength": 80},
                "new_name": {"type": "string", "maxLength": 80, "description": "For save: rename the profile; the default follows."},
                "fields": {"type": "object", "description": "For save: the fields to set.", "properties": {
                    "aircraft": {"type": "string", "description": "For example \"65 mm whoop\"."},
                    "camera_make": {"type": "string"}, "camera_model": {"type": "string"},
                    "video_system": {"type": "string", "description": "Analog, DJI O4, Walksnail or HDZero."},
                    "keywords": {"type": "array", "items": {"type": "string"}},
                    "author": {"type": "string"},
                    "place": {"type": ["string", "null"], "description": "A saved place name, or null for none."},
                    "edgetx_models": {"type": "array", "items": {"type": "string"}, "description": "EdgeTX model names (the start of the radio's log file names)."}
                }, "additionalProperties": false},
                "default": {"type": "boolean", "description": "For save: also make it the default."}
            }, "additionalProperties": false},
            "annotations": {"title": "Aircraft profiles", "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": false}
        },
        {
            "name": "quadcam_settings",
            "description": "Read or write the app's settings, the same file the app's Settings window uses: library folder (`output_dir`) and its `layout` (year_day, day, flat) and `place_folders`, export `format` (mp4, mov), `encoder` (videotoolbox, x264), `keep_originals`, `add_time` (HHMM in names of clips with a time), `default_name`, `photos_album` (empty: library only), card `format_label`, radio `log_dir`, log matching `tunables`, place search `geocoder` (apple, nominatim, census, google) and its `google_places_key` (write-only), file-name `name_date_format` (YYYY-MM-DD, YY.MM.DD) and `default_profile`. A write changes only the given settings; null resets one to its default.\n\nBest for: pointing the library somewhere else, or changing export defaults the person asked for.\nNot for: places and profiles (quadcam_places, quadcam_profiles).\nReturns: the settings file's path and every effective setting.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["read", "write"]},
                "values": {"type": "object", "description": "For write: {setting: value}.", "properties": {
                    "output_dir": {"type": ["string", "null"], "description": "Absolute folder path."},
                    "layout": {"type": ["string", "null"], "enum": ["year_day", "day", "flat", null]},
                    "place_folders": {"type": ["boolean", "null"]},
                    "format": {"type": ["string", "null"], "enum": ["mp4", "mov", null]},
                    "encoder": {"type": ["string", "null"], "enum": ["videotoolbox", "x264", null]},
                    "keep_originals": {"type": ["boolean", "null"]},
                    "add_time": {"type": ["boolean", "null"]},
                    "default_name": {"type": ["string", "null"]},
                    "photos_album": {"type": ["string", "null"]},
                    "format_label": {"type": ["string", "null"], "maxLength": 11},
                    "log_dir": {"type": ["string", "null"]},
                    "tunables": {"type": ["object", "null"], "properties": {"segment_gap_s": {"type": "number"}, "session_gap_min": {"type": "number"}, "tolerance_s": {"type": "number"}, "max_log_age_days": {"type": "integer"}}, "required": ["segment_gap_s", "session_gap_min", "tolerance_s", "max_log_age_days"], "additionalProperties": false},
                    "geocoder": {"type": ["string", "null"], "enum": ["apple", "nominatim", "census", "google", null]},
                    "google_places_key": {"type": ["string", "null"], "description": "Google Places API (New) key, for geocoder google. Never read back."},
                    "name_date_format": {"type": ["string", "null"], "enum": ["YYYY-MM-DD", "YY.MM.DD", null], "description": "How the date starts new file names; rename existing clips with quadcam_library_files apply_name_format."},
                    "default_profile": {"type": ["string", "null"]}
                }, "additionalProperties": false}
            }, "additionalProperties": false},
            "annotations": {"title": "Settings", "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        }
    ]);
    if let (Some(a), Value::Array(b)) = (flow.as_array_mut(), more) {
        a.extend(b);
    }
    flow
}
