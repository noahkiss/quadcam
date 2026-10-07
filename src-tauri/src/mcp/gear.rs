//! The three Gear tools, split by what they can change, so a harness can allow the read
//! tool freely and gate the other two (design 5.3):
//!
//! - `quadcam_gear` changes nothing;
//! - `quadcam_gear_edit` changes QuadCam's own data only, never a device, a sim or a card;
//! - `quadcam_gear_apply` writes a device, a sim or the radio firmware, behind a plan's
//!   digest and `confirm=true`.
//!
//! Each tool takes an `action`. A package adds its action to the tool's `enum`, a field to
//! its args if it needs one, and a match arm here; it does not add a tool. Gear's argument
//! types live here rather than in `params.rs`, so Gear packages edit one file.

use super::render::text;
use super::server::Backend;
use super::tools::tool;
use anyhow::{anyhow, Context, Result};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

/// What Gear says when nothing is plugged in. On Apple silicon, macOS keeps a new USB
/// accessory off the bus until the person allows it, and no app can see that prompt.
pub const NOTHING_FOUND: &str = "Nothing found. If macOS asked to allow an accessory, click Allow.";

// ----- arguments -----

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct GearArgs {
    #[schemars(required, extend("enum" = ["status", "devices", "fc_identify", "board_notes", "usb_timers", "osd", "card", "card_preview", "storage", "backups", "backup_read", "backup_diff", "card_checks", "flights", "packs", "session_report", "preflight", "crashes", "switch_map", "radio"]))]
    pub action: Option<String>,
    /// For fc_identify and a live switch_map: the FC's serial port (/dev/cu.usbmodem...) from status; omit when one FC is plugged in.
    #[schemars(length(max = 200))]
    pub port: Option<String>,
    /// For board_notes: a board name (BETAFPVG473); omit for every board.
    #[schemars(length(max = 80))]
    pub board: Option<String>,
    /// For board_notes: a firmware version (2025.12.5); omit for every version.
    #[schemars(length(max = 80))]
    pub version: Option<String>,
    /// For osd and switch_map: Betaflight `dump all`, `diff all` or CLI-line files
    /// (absolute paths), read in order; a later file's lines win.
    #[schemars(length(max = 8))]
    pub paths: Option<Vec<String>>,
    /// For osd and switch_map: a saved device id instead of files (needs a backup of it). For card and
    /// card_preview: a connected radio's device id, instead of mount.
    #[schemars(length(max = 80))]
    pub device: Option<String>,
    /// For osd: NTSC, PAL, HD or WxH; omit for the files' video system.
    #[schemars(length(max = 8))]
    pub grid: Option<String>,
    /// For card and card_preview: the card's mount point. Default: the one EdgeTX card mounted. For switch_map: an EdgeTX card's mount or folder, or one model file.
    #[schemars(length(max = 1024))]
    pub mount: Option<String>,
    /// For card: a model file (model01.yml) to read in full: timers, mixes, logical switches, special functions, switch warnings, sensors, screens. For switch_map: the model on the card; default the radio's selected model.
    #[schemars(length(max = 40))]
    pub model: Option<String>,
    /// For card_preview: the edits, each {"kind": "model", "file": "model01.yml", "name": "<header name>", "ops": [{"op": "set_checklist", "enabled": true}, ...]}, {"kind": "radio", "ops": [{"op": "set_scalar", "key": "hapticMode", "value": "mode_nokeys"}]}, {"kind": "checklist", "model": "model01.yml", "text": "=Props tight"}, {"kind": "model_copy", ...} or {"kind": "model_delete", "file": ...}.
    pub edits: Option<Vec<Value>>,
    /// For switch_map: also mark where each control is now, from the FC's channels (MSP) when an FC is plugged in, else from the radio in USB Joystick mode.
    pub live: Option<bool>,
    /// For backup_read and backup_diff: a backup id (<device>/<name>) from backups.
    #[schemars(length(max = 120))]
    pub id: Option<String>,
    /// For backup_diff: the older backup to compare with; omit for the one before id.
    #[schemars(length(max = 120))]
    pub against: Option<String>,
    /// For backup_read: one file of the backup (MODELS/model01.yml, diff all); omit for the file list. For backup_diff: one file only.
    #[schemars(length(max = 1024))]
    pub path: Option<String>,
    /// For flights and session_report: a day, YYYY-MM-DD. session_report without it: the days of the last import.
    #[schemars(length(max = 10))]
    pub day: Option<String>,
    /// For flights and crashes: an aircraft profile name. For switch_map: reads the latest backups of the radio and FC saved with that aircraft.
    #[schemars(length(max = 80))]
    pub aircraft: Option<String>,
    /// For flights: a pack label.
    #[schemars(length(max = 80))]
    pub pack: Option<String>,
    /// For flights: a place name.
    #[schemars(length(max = 200))]
    pub place: Option<String>,
    /// For flights: one more folder of radio logs (absolute path), read for this call only.
    #[schemars(length(max = 1024))]
    pub logs: Option<String>,
    /// For packs: the resting volts per cell the mAh warning suggestion aims for (default 3.7).
    pub target_v: Option<f64>,
    /// For crashes: a library clip id.
    #[schemars(length(max = 80))]
    pub clip: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct GearEditArgs {
    #[schemars(required, extend("enum" = ["device_save", "device_forget", "fc_read", "backup", "backup_pin", "prune", "export", "import_backups", "card_check", "stop", "flight_set", "flight_folders", "pack_save", "pack_delete", "pack_type_save", "pack_type_delete", "pack_notes", "crash_save", "crash_delete"]))]
    pub action: Option<String>,
    /// For fc_read and backup: the FC's serial port from quadcam_gear status; omit when one FC is plugged in.
    #[schemars(length(max = 200))]
    pub port: Option<String>,
    /// For fc_read: read-only CLI commands (version, status, get NAME, diff all, dump all, diff/dump master|profile|rates|hardware|defaults). Omit for a backup's set: version, status, diff all, dump all.
    #[schemars(length(max = 20))]
    pub commands: Option<Vec<String>>,
    /// For device_save and device_forget: the device id from quadcam_gear status or devices.
    /// For crash_save (to change one) and crash_delete: the crash id.
    #[schemars(length(max = 80))]
    pub id: Option<String>,
    /// For device_save: the person's name for the device; empty for none. For pack_save and
    /// pack_delete: the pack label. For pack_type_save and pack_type_delete: the type name.
    #[schemars(length(max = 80))]
    pub name: Option<String>,
    /// For device_save: an aircraft profile name from quadcam_profiles; empty to unlink.
    /// For crash_save: the aircraft (default: the clip's).
    #[schemars(length(max = 80))]
    pub aircraft: Option<String>,
    /// For backup, card_check and export: a device id; for import_backups: the saved device that items no id names go to.
    #[schemars(length(max = 80))]
    pub device: Option<String>,
    /// For backup and card_check: a card's mount point instead of device.
    #[schemars(length(max = 1024))]
    pub mount: Option<String>,
    /// For backup_pin and export: a backup id (<device>/<name>) from quadcam_gear backups.
    #[schemars(length(max = 120))]
    pub backup: Option<String>,
    /// For backup_pin: true keeps the backup through pruning; false lets retention thin it.
    pub pinned: Option<bool>,
    /// For prune and import_backups: report what would happen; change nothing.
    pub dry_run: Option<bool>,
    /// For export: an existing folder (absolute path) to write the backup folders into.
    #[schemars(length(max = 1024))]
    pub to: Option<String>,
    /// For import_backups: the old backup folder (absolute path).
    #[schemars(length(max = 1024))]
    pub folder: Option<String>,
    /// For stop: the running job's handle from quadcam_gear status (jobs).
    #[schemars(length(max = 200))]
    pub handle: Option<String>,
    /// For flight_set: a flight id from quadcam_gear flights.
    #[schemars(length(max = 120))]
    pub flight: Option<String>,
    /// For flight_set: the pack label ("" clears it).
    #[schemars(length(max = 80))]
    pub pack: Option<String>,
    /// For flight_set: the place ("" clears it).
    #[schemars(length(max = 200))]
    pub place: Option<String>,
    /// For flight_folders: a folder of radio logs to add (absolute path).
    #[schemars(length(max = 1024))]
    pub add_folder: Option<String>,
    /// For flight_folders: a folder to remove.
    #[schemars(length(max = 1024))]
    pub remove_folder: Option<String>,
    /// For pack_save: its pack type name.
    #[schemars(length(max = 80))]
    pub pack_type: Option<String>,
    /// For pack_save: the day it arrived, YYYY-MM-DD.
    #[schemars(length(max = 10))]
    pub received: Option<String>,
    /// For pack_save: retired from use.
    pub retired: Option<bool>,
    /// For pack_save: true marks it charged now, false clears the mark.
    pub charged: Option<bool>,
    /// For pack_save, pack_type_save and crash_save: a free note.
    #[schemars(length(max = 2000))]
    pub note: Option<String>,
    /// For pack_type_save: lipo, lihv or liion.
    #[schemars(length(max = 10))]
    pub chemistry: Option<String>,
    /// For pack_type_save: cells in series.
    #[schemars(range(min = 1, max = 14))]
    pub cells: Option<u8>,
    /// For pack_type_save: capacity in mAh.
    pub capacity_mah: Option<f64>,
    /// For pack_type_save: the connector (XT30, BT2.0, ...).
    #[schemars(length(max = 40))]
    pub connector: Option<String>,
    /// For pack_type_save: full charge, volts per cell.
    pub full_v: Option<f64>,
    /// For pack_type_save: storage charge, volts per cell.
    pub storage_v: Option<f64>,
    /// For pack_type_save: charge current in amps.
    pub charge_a: Option<f64>,
    /// For pack_type_save: the radio's mAh warning for this type (the flight threshold).
    pub warn_mah: Option<f64>,
    /// For pack_notes: the charging sheet's notes.
    #[schemars(length(max = 10000))]
    pub text: Option<String>,
    /// For crash_save: the library clip id.
    #[schemars(length(max = 80))]
    pub clip: Option<String>,
    /// For crash_save: seconds into the clip.
    pub time_s: Option<f64>,
    /// For crash_save: the day, YYYY-MM-DD (default: the clip's).
    #[schemars(length(max = 10))]
    pub day: Option<String>,
    /// For crash_save: what broke.
    #[schemars(length(max = 500))]
    pub broke: Option<String>,
    /// For crash_save: parts used for the repair.
    #[schemars(length(max = 40))]
    pub parts: Option<Vec<String>>,
    /// For crash_save: repaired.
    pub repaired: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct GearApplyArgs {
    #[schemars(required, extend("enum" = ["card_repair"]))]
    pub action: Option<String>,
    /// For card_repair: the id of the card's latest check, which failed (quadcam_gear card_checks or status).
    #[schemars(length(max = 200))]
    pub digest: Option<String>,
    /// Must be true.
    pub confirm: Option<bool>,
}

// ----- tool list -----

/// The Gear tools' descriptors, after the other tools.
pub fn tools() -> Vec<Value> {
    vec![
        tool::<GearArgs>(
            "quadcam_gear",
            "Read the FPV gear QuadCam knows: `status` (the gear folder, the Gear settings, and the devices plugged in now: EdgeTX radios in USB Storage mode, goggles and DVR cards, FC and ELRS serial ports, radios in DFU mode; each with its saved name and aircraft when QuadCam knows it; and each FC's USB heat timer), `devices` (every device saved in gear.json: id, kind, name, aircraft, board, firmware, version, last seen, last backup), `fc_identify` (reads a Betaflight FC over MSP: board, firmware, version, device id, whether QuadCam may write it, known issues; no reboot), `board_notes` (known issues of FC boards and builds), `usb_timers` (per FC on USB: battery in, minutes on USB, minutes left before \"Unplug now\"), `osd` (a Betaflight OSD layout from `paths`, dump or diff files read in order: each OSD profile drawn on its grid (NTSC 30x13, PAL 30x16, HD 53x20, from vcd_video_system or `grid`), the elements on in each profile with x and y, and the check for overlaps and cells off screen), `card` (an EdgeTX SD card: board and version, whether QuadCam may write it, its models, the model the radio selects and that model's aircraft, the radio clock check; with `model`, that model in full) `card_preview` (the checks and line diff of EdgeTX card edits, and how long the write would take; writes nothing), `storage` (the gear folder's size in total and per device: snapshots by kind, logs, blobs only that device uses), `backups` (snapshots newest first, or one `device`'s: id, why it was taken, time, files, size, pinned), `backup_read` (a backup `id`'s file list, or one file's text with `path`), `backup_diff` (what changed from `against`, or the backup before, to `id`: files put and removed, a line diff per text file), `card_checks` (a card `device`'s file-system checks, newest first), `flights` (flights from the radio logs, newest first: hover throttle, sag, resting voltage, mAh, when the pack type's mAh warning was crossed, the worst link, dropouts; each with its aircraft, pack (or a suggested one), place and library clip; filter by `day`, `aircraft`, `pack`, `place`), `packs` (packs with cycles, charge state, resting voltage and a weak mark; pack types with the charging sheet and a suggested mAh warning; the charging notes), `session_report` (one flying day, by default the last import's days: flights, air time, longest flight, worst link, dropouts, pack use, crashes; as Markdown to share), `preflight` (the \"Pack up\" check: packs charged, the radio's model, card space, backups, cards still in the Mac; each pass, warn or unknown) `crashes` (the crash and repair log, by `aircraft` or `clip`), `switch_map` (what each radio control does: per switch, trim or stick position, the channel values in microseconds, the Betaflight modes and adjustments they select, and the radio's logical switches, special functions and timers; from `mount` (an EdgeTX card or model file) and `paths` (the FC's dump or diff), or the latest backups of a saved `device` or of an `aircraft`'s radio and FC; conflicts such as two modes on one range, a switch with no effect, a mode no switch reaches, a sound file the card lacks; with `live`, the position each control is in now) or `radio` (the radio in USB Joystick mode now: buttons, axes, channel values). Changes nothing.\n\nBest for: the first Gear call, checking what is plugged in, and checking an OSD layout before or after an edit, reading or planning radio model changes, and answering \"what does this switch do\".\nReturns: one line per device plus the structured records; for osd and switch_map, the drawn view as text plus the structured view.\nFollow up with quadcam_gear_edit device_save to name a device or link it to an aircraft, flight_set to put a flight on a pack.",
            json!({"openWorldHint": false, "readOnlyHint": true, "title": "Gear"}),
        ),
        tool::<GearEditArgs>(
            "quadcam_gear_edit",
            "Change QuadCam's own gear data, never a device's settings, a sim or a card: `device_save` names a device or links it to an aircraft profile (a device QuadCam does not know yet must be plugged in; use its id from quadcam_gear status or fc_identify), `device_forget` removes a device from QuadCam's list (its backups stay), `fc_read` reads a Betaflight FC through its CLI (read-only commands; the FC reboots when the read ends, so the person should expect it; returns the text, writes nothing), `backup` backs up a radio card (`device` or `mount`; files whose size and time did not change are not read) or an FC (`port` or `device`; the FC reboots) into QuadCam's gear folder (nothing is written when nothing changed), `backup_pin` keeps a `backup` through pruning (`pinned`), `prune` thins backups by the retention settings and removes stored files nothing uses (`dry_run` to see first), `export` writes a `backup`, or every backup of a `device`, as plain folders into `to`, `import_backups` takes an old backup `folder` (radio card copies, Betaflight diff all and dump all files, LOGS folders, dated by YYYY-MM-DD in folder names; `dry_run` to see first; it never changes the folder), `card_check` checks a card's file system (diskutil verifyVolume, read-only, about 30 s over a radio's USB; the card unmounts and mounts again), `stop` stops a running backup or card check by its `handle`, `flight_set` (a flight's pack or place), `flight_folders` (add or remove a folder of radio logs the flights read), `pack_save` / `pack_delete` (a pack by `name`, its label; `charged=true` marks it charged now), `pack_type_save` / `pack_type_delete` (a pack type by `name`: chemistry, cells, capacity, connector, charge volts per cell, charge current, mAh warning), `pack_notes` (the charging sheet's notes), `crash_save` / `crash_delete` (a crash on a library clip: the time in the clip, what broke, the parts used; no `id` logs a new one).\n\nBest for: naming a radio or quad the person just plugged in, and linking it to its aircraft profile; reading an FC's settings as text; backing gear up and importing old backups; putting flights on packs; logging a crash and its repair.\nReturns: the saved or forgotten device, the FC's identity and each command's answer, a summary of the backup, prune, export, import or check, or the saved flight, pack, pack type or crash.",
            json!({"destructiveHint": false, "idempotentHint": true, "openWorldHint": false, "readOnlyHint": false, "title": "Edit gear data"}),
        ),
        tool::<GearApplyArgs>(
            "quadcam_gear_apply",
            "Write to a device, a sim or the radio firmware: each action needs a digest and confirm=true. `card_repair` repairs a card whose latest check failed (diskutil repairVolume): `digest` is that check's id (quadcam_gear card_checks or status); QuadCam backs the card up first when it reads, repairs it, and checks it again. A repair cannot be stopped once it starts.\n\nBest for: repairing a radio or DVR card after a failed card check, when the person asks.\nReturns: the backup taken first, the repair's result and the check after it.",
            json!({"destructiveHint": true, "idempotentHint": false, "openWorldHint": false, "readOnlyHint": false, "title": "Apply to gear"}),
        ),
    ]
}

// ----- handlers -----

/// A tool's arguments as its type; no arguments are the defaults.
fn args<T: serde::de::DeserializeOwned + Default>(a: &Value) -> Result<T> {
    if a.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(a.clone()).context("bad arguments")
}

/// One line per device.
fn device_line(d: &Value) -> String {
    let name = d["name"].as_str().filter(|n| !n.trim().is_empty());
    let kind = d["kind"].as_str().unwrap_or("?");
    let ident = &d["identity"];
    let fw = [&ident["firmware"], &ident["version"], &ident["board"]]
        .iter()
        .filter_map(|v| v.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{} | {} | {} | aircraft {} | {}",
        d["id"].as_str().unwrap_or("?"),
        kind,
        name.map(str::to_string)
            .unwrap_or_else(|| format!("Unnamed {kind}")),
        d["aircraft"].as_str().unwrap_or("none"),
        if fw.is_empty() { "unidentified" } else { &fw }
    )
}

/// One line per connected device.
fn connected_line(c: &Value) -> String {
    let link = &c["link"];
    let at = match link["kind"].as_str() {
        Some("volume") => link["mount"].as_str().unwrap_or("?").to_string(),
        Some("serial") => link["port"].as_str().unwrap_or("?").to_string(),
        Some("dfu") => "DFU".to_string(),
        _ => "?".into(),
    };
    let saved = c["device"]
        .as_object()
        .and_then(|d| d.get("name"))
        .and_then(Value::as_str)
        .filter(|n| !n.trim().is_empty())
        .map(|n| format!("{n:?}"))
        .unwrap_or_else(|| "not saved".into());
    format!(
        "{} at {} | id {} | {}",
        c["kind"].as_str().unwrap_or("?"),
        at,
        c["id"].as_str().unwrap_or("none yet"),
        saved
    )
}

/// An FC's identity, write status and job notes in words.
fn fc_info_text(i: &Value, notes: &Value) -> String {
    let id = &i["identity"];
    let mut s = format!(
        "FC on {} | id {} | {} {} | board {}",
        i["port"].as_str().unwrap_or("?"),
        i["id"].as_str().unwrap_or("none"),
        id["firmware"].as_str().unwrap_or("?"),
        id["version"].as_str().unwrap_or("?"),
        id["board"].as_str().unwrap_or("?"),
    );
    match i["read_only"]["reason"].as_str() {
        Some(r) => s.push_str(&format!("\nRead only: {r}")),
        None => s.push_str("\nWritable: this board and build are proven."),
    }
    for n in notes.as_array().into_iter().flatten() {
        if let Some(t) = n.as_str() {
            s.push_str(&format!("\nNote: {t}"));
        }
    }
    s.push_str("\nThe port is released: safe to unplug.");
    s
}

/// One FC's USB timer in words.
fn usb_line(t: &Value) -> String {
    let port = t["port"].as_str().unwrap_or("?");
    if t["battery"].as_bool() != Some(true) {
        return format!("{port}: no battery");
    }
    let min = |v: &Value| v.as_u64().map(|s| format!("{}:{:02}", s / 60, s % 60));
    match min(&t["remaining_s"]) {
        Some(left) => format!(
            "{port}: battery in for {}, {left} left{}",
            min(&t["elapsed_s"]).unwrap_or_default(),
            if t["warned"].as_bool() == Some(true) {
                " (unplug now played)"
            } else {
                ""
            }
        ),
        None => format!("{port}: battery in, timer off"),
    }
}

/// Runs a Gear tool; None when `name` is not one.
pub(super) fn run<B: Backend>(
    backend: &mut B,
    name: &str,
    a: &Value,
) -> Option<Result<(Vec<Value>, Value)>> {
    Some(match name {
        "quadcam_gear" => gear(backend, a),
        "quadcam_gear_edit" => gear_edit(backend, a),
        "quadcam_gear_apply" => gear_apply(backend, a),
        _ => return None,
    })
}

fn gear<B: Backend>(backend: &mut B, a: &Value) -> Result<(Vec<Value>, Value)> {
    let x: GearArgs = args(a)?;
    match x.action.as_deref().unwrap_or("status") {
        "status" => {
            let s = backend.call("gear_status", Value::Null)?;
            let connected = s["connected"].as_array().cloned().unwrap_or_default();
            let mut line = format!(
                "Gear folder {}. {} saved devices, {} staged changes.\n",
                s["gear_dir"].as_str().unwrap_or("?"),
                s["devices"],
                s["staged"]
            );
            line.push_str(&if connected.is_empty() {
                NOTHING_FOUND.to_string()
            } else {
                connected
                    .iter()
                    .map(connected_line)
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            for j in s["jobs"].as_array().into_iter().flatten() {
                let p = &j["progress"];
                line.push_str(&format!(
                    "\n{} on {} (handle {}): {} of {} files{}",
                    j["step"].as_str().unwrap_or("?"),
                    j["device"].as_str().unwrap_or("?"),
                    j["handle"].as_str().unwrap_or("?"),
                    p["files_done"],
                    p["files_total"],
                    if j["stopping"].as_bool() == Some(true) {
                        ", stopping"
                    } else {
                        ""
                    }
                ));
            }
            for c in s["card_checks"].as_array().into_iter().flatten() {
                if c["state"] != "ok" {
                    line.push_str(&format!(
                        "\nCard check {} on {}: {} (check id {})",
                        c["state"].as_str().unwrap_or("?"),
                        c["device"].as_str().unwrap_or("?"),
                        c["summary"].as_str().unwrap_or(""),
                        c["id"].as_str().unwrap_or("?")
                    ));
                }
            }
            Ok((vec![text(line)], s))
        }
        "fc_identify" => {
            let j = backend.call("gear_fc_identify", json!({"port": x.port}))?;
            Ok((vec![text(fc_info_text(&j["result"], &j["notes"]))], j))
        }
        "board_notes" => {
            let list = backend.call(
                "gear_board_notes",
                json!({"board": x.board, "version": x.version}),
            )?;
            let arr = list.as_array().cloned().unwrap_or_default();
            let line = if arr.is_empty() {
                "No known issues.".to_string()
            } else {
                arr.iter()
                    .map(|n| {
                        format!(
                            "{} {}: {}",
                            n["board"].as_str().unwrap_or("?"),
                            n["version"].as_str().unwrap_or("(every version)"),
                            n["text"].as_str().unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            Ok((vec![text(line)], json!({"notes": list})))
        }
        "usb_timers" => {
            let list = backend.call("gear_usb_timers", Value::Null)?;
            let arr = list.as_array().cloned().unwrap_or_default();
            let line = if arr.is_empty() {
                "No FC on USB.".to_string()
            } else {
                arr.iter().map(usb_line).collect::<Vec<_>>().join("\n")
            };
            Ok((vec![text(line)], json!({"timers": list})))
        }
        "devices" => {
            let list = backend.call("gear_devices", Value::Null)?;
            let arr = list.as_array().cloned().unwrap_or_default();
            let line = if arr.is_empty() {
                "No saved devices.".to_string()
            } else {
                arr.iter().map(device_line).collect::<Vec<_>>().join("\n")
            };
            Ok((vec![text(line)], json!({"devices": list})))
        }
        "osd" => {
            let v = backend.call(
                "gear_osd",
                json!({
                    "paths": x.paths.unwrap_or_default(),
                    "device": x.device,
                    "grid": x.grid,
                }),
            )?;
            let view: crate::gear::osd::OsdView =
                serde_json::from_value(v.clone()).context("bad osd answer")?;
            Ok((vec![text(crate::gear::osd::render_text(&view))], v))
        }
        "card" => {
            let v = backend.call(
                "gear_card",
                json!({"mount": x.mount, "device": x.device, "model": x.model}),
            )?;
            Ok((vec![text(card_text(&v))], v))
        }
        "card_preview" => {
            let edits = x
                .edits
                .context("edits is required for card_preview: a list of card edits")?;
            let v = backend.call(
                "gear_card_preview",
                json!({"mount": x.mount, "device": x.device, "edits": edits}),
            )?;
            Ok((vec![text(preview_text(&v))], v))
        }
        "switch_map" => {
            let v = backend.call(
                "gear_switch_map",
                json!({
                    "radio": x.mount,
                    "model": x.model,
                    "aircraft": x.aircraft,
                    "devices": x.device.into_iter().collect::<Vec<_>>(),
                    "fc": x.paths.unwrap_or_default(),
                    "live": x.live.unwrap_or(false),
                    "port": x.port,
                }),
            )?;
            let map: crate::gear::switchmap::SwitchMap =
                serde_json::from_value(v.clone()).context("bad switch_map answer")?;
            Ok((vec![text(crate::gear::switchmap::render_text(&map))], v))
        }
        "radio" => {
            let v = backend.call("gear_radio", json!({}))?;
            let line = match v["frame"].as_object() {
                Some(f) => format!(
                    "{} | channels {} | buttons on {}",
                    v["product"].as_str().unwrap_or("radio"),
                    f["channels"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .enumerate()
                        .map(|(i, c)| format!("CH{} {c}", i + 1))
                        .collect::<Vec<_>>()
                        .join(", "),
                    {
                        let b = f["buttons"].as_u64().unwrap_or(0);
                        let on: Vec<String> =
                            (0..24).filter(|i| b >> i & 1 == 1).map(|i| (i + 1).to_string()).collect();
                        if on.is_empty() { "none".to_string() } else { on.join(", ") }
                    }
                ),
                None => v["message"].as_str().unwrap_or("No radio.").to_string(),
            };
            Ok((vec![text(line)], v))
        }
        "storage" => {
            let v = backend.call("gear_storage", Value::Null)?;
            Ok((vec![text(storage_text(&v))], v))
        }
        "backups" => {
            let list = backend.call("gear_backups", json!({"device": x.device}))?;
            let arr = list.as_array().cloned().unwrap_or_default();
            let line = if arr.is_empty() {
                "No backups.".to_string()
            } else {
                arr.iter().map(backup_line).collect::<Vec<_>>().join("\n")
            };
            Ok((vec![text(line)], json!({"backups": list})))
        }
        "backup_read" => {
            let id = x.id.context("id is required for backup_read: a backup id from backups")?;
            let v = backend.call("gear_backup_read", json!({"id": id, "path": x.path}))?;
            Ok((vec![text(backup_read_text(&v))], v))
        }
        "backup_diff" => {
            let id = x.id.context("id is required for backup_diff: a backup id from backups")?;
            let v = backend.call(
                "gear_backup_diff",
                json!({"a": x.against.clone().unwrap_or_else(|| id.clone()), "b": x.against.as_ref().map(|_| id.clone()), "path": x.path}),
            )?;
            let t = diff_text(&v);
            Ok((
                vec![text(if t.is_empty() { "No changes.".into() } else { t })],
                json!({"diff": v}),
            ))
        }
        "card_checks" => {
            let device = x
                .device
                .context("device is required for card_checks: a card's device id")?;
            let list = backend.call("gear_card_checks", json!({"device": device}))?;
            let arr = list.as_array().cloned().unwrap_or_default();
            let line = if arr.is_empty() {
                "No checks of this card yet.".to_string()
            } else {
                arr.iter().map(check_line).collect::<Vec<_>>().join("\n")
            };
            Ok((vec![text(line)], json!({"checks": list})))
        }
        other => super::gear_flights::read(backend, other, &x).unwrap_or_else(|| Err(anyhow!(
            "unknown action {other:?}; use status, devices, fc_identify, board_notes, usb_timers, osd, card, card_preview, storage, backups, backup_read, backup_diff, card_checks, switch_map, radio, flights, packs, session_report, preflight or crashes"
        ))),
    }
}

/// The card answer in lines: identity, write state, models, clock.
fn card_text(v: &Value) -> String {
    let c = &v["card"];
    let id = &c["identity"];
    let mut out = format!(
        "EdgeTX {} on {} at {}{}.\n",
        id["version"].as_str().unwrap_or("?"),
        id["board"].as_str().unwrap_or("?"),
        c["root"].as_str().unwrap_or("?"),
        if v["radio_usb"].as_bool() == Some(true) {
            " (the radio over USB: writes are slow)"
        } else {
            ""
        }
    );
    match c["read_only"]["reason"].as_str() {
        Some(r) => out.push_str(&format!("Read only: {r}\n")),
        None => out.push_str("QuadCam may write this card.\n"),
    }
    for m in c["models"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "{} {:?}{}{}\n",
            m["file"].as_str().unwrap_or("?"),
            m["name"].as_str().unwrap_or(""),
            if m["selected"].as_bool() == Some(true) {
                " (selected)"
            } else {
                ""
            },
            m["problem"]
                .as_str()
                .map(|p| format!(" | {p}"))
                .unwrap_or_default()
        ));
    }
    if let Some(a) = v["selected_aircraft"].as_str() {
        out.push_str(&format!("The selected model is aircraft {a}.\n"));
    }
    if let Some(m) = c["clock"]["message"].as_str() {
        out.push_str(m);
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// Bytes as KB or MB.
fn size(v: &Value) -> String {
    let b = v.as_u64().unwrap_or(0);
    if b >= 1 << 20 {
        format!("{:.1} MB", b as f64 / (1u64 << 20) as f64)
    } else {
        format!("{} KB", b.div_ceil(1024))
    }
}

/// The date part of a time.
fn day(v: &Value) -> String {
    v.as_str()
        .map(|t| t.get(..16).unwrap_or(t).replace('T', " "))
        .unwrap_or_else(|| "never".into())
}

fn storage_text(v: &Value) -> String {
    let mut out = format!(
        "Gear folder {}: {} in total ({} snapshots; stored files {}, of them {} shared by devices; logs {}).",
        v["gear_dir"].as_str().unwrap_or("?"),
        size(&v["total_bytes"]),
        v["snapshots"],
        size(&v["blob_bytes"]),
        size(&v["shared_blob_bytes"]),
        size(&v["log_bytes"]),
    );
    for d in v["devices"].as_array().into_iter().flatten() {
        let kinds = d["by_trigger"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, n)| format!("{n} {k}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        out.push_str(&format!(
            "\n{} ({}): {} snapshots ({}; {} pinned), {} logs, {} of its own, latest {}",
            d["name"].as_str().unwrap_or("not in the device list"),
            d["device"].as_str().unwrap_or("?"),
            d["snapshots"],
            if kinds.is_empty() {
                "none".into()
            } else {
                kinds
            },
            d["pinned"],
            d["logs"],
            size(&d["total_bytes"]),
            day(&d["latest"]),
        ));
    }
    out
}

fn backup_line(b: &Value) -> String {
    format!(
        "{} | {} | {} | {} files, {}{}",
        b["id"].as_str().unwrap_or("?"),
        b["trigger"].as_str().unwrap_or("?"),
        day(&b["taken_at"]),
        b["files"],
        size(&b["bytes"]),
        if b["pinned"].as_bool() == Some(true) {
            " | pinned"
        } else {
            ""
        }
    )
}

fn backup_read_text(v: &Value) -> String {
    let b = &v["backup"];
    match v["path"].as_str() {
        None => {
            let mut out = format!(
                "Backup {} ({}, {}):",
                b["id"].as_str().unwrap_or("?"),
                b["trigger"].as_str().unwrap_or("?"),
                day(&b["taken_at"])
            );
            for f in b["files"].as_array().into_iter().flatten() {
                out.push_str(&format!(
                    "\n{} ({})",
                    f["path"].as_str().unwrap_or("?"),
                    size(&f["size"])
                ));
            }
            out
        }
        Some(p) => match v["text"].as_str() {
            Some(t) => format!("# {p}\n{t}"),
            None => format!("{p} is a binary file ({}).", size(&v["size"])),
        },
    }
}

fn check_line(c: &Value) -> String {
    let mut s = format!(
        "{} | {} {} | {} | {}",
        c["id"].as_str().unwrap_or("?"),
        c["kind"].as_str().unwrap_or("?"),
        c["state"].as_str().unwrap_or("?"),
        day(&c["at"]),
        c["summary"].as_str().unwrap_or("")
    );
    for f in c["findings"].as_array().into_iter().flatten() {
        if let Some(t) = f.as_str() {
            s.push_str(&format!("\n  {t}"));
        }
    }
    s
}

/// Diff items as text: files put and removed, then each line diff.
fn diff_text(items: &Value) -> String {
    let mut out = String::new();
    for d in items.as_array().into_iter().flatten() {
        match d["kind"].as_str() {
            Some("files") => {
                for (k, sign) in [("put", "changed or added"), ("delete", "removed")] {
                    let l: Vec<&str> = d[k]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect();
                    if !l.is_empty() {
                        out.push_str(&format!("{sign}: {}\n", l.join(", ")));
                    }
                }
            }
            Some("lines") => {
                out.push_str(&format!("--- {}\n", d["label"].as_str().unwrap_or("?")));
                for l in d["lines"].as_array().into_iter().flatten() {
                    let sign = match l["op"].as_str() {
                        Some("add") => '+',
                        Some("remove") => '-',
                        _ => ' ',
                    };
                    out.push_str(&format!("{sign}{}\n", l["text"].as_str().unwrap_or("")));
                }
            }
            _ => {}
        }
    }
    out.trim_end().to_string()
}

fn backup_result_text(v: &Value) -> String {
    let r = &v["report"];
    let b = &r["backup"];
    let mut out = if r["new"].as_bool() == Some(true) {
        format!(
            "Backed up {} ({}): new backup {}. Read {} files ({}); {} unchanged files not read.",
            v["name"].as_str().unwrap_or("?"),
            v["device"].as_str().unwrap_or("?"),
            b["id"].as_str().unwrap_or("?"),
            r["read"],
            size(&r["bytes_read"]),
            r["skipped"]
        )
    } else {
        format!(
            "{} ({}): no changes since {} (backup {}). Nothing written.",
            v["name"].as_str().unwrap_or("?"),
            v["device"].as_str().unwrap_or("?"),
            day(&b["taken_at"]),
            b["id"].as_str().unwrap_or("?")
        )
    };
    let l = &r["logs"];
    if l["added"].as_u64().unwrap_or(0)
        + l["grown"].as_u64().unwrap_or(0)
        + l["kept_both"].as_u64().unwrap_or(0)
        > 0
    {
        out.push_str(&format!(
            "\nLogs: {} new, {} grown, {} kept as a second copy.",
            l["added"], l["grown"], l["kept_both"]
        ));
    }
    if let Some(p) = v["pruned"].as_object() {
        let n = p["dropped"].as_array().map_or(0, Vec::len);
        if n > 0 {
            out.push_str(&format!("\nPruned {n} older backups."));
        }
    }
    for n in v["notes"].as_array().into_iter().flatten() {
        if let Some(t) = n.as_str() {
            out.push_str(&format!("\nNote: {t}"));
        }
    }
    out
}

fn import_text(v: &Value) -> String {
    let mut out = format!(
        "{}{} new backups, {} the same as one QuadCam has, {} skipped; logs: {} new, {} grown.",
        if v["dry_run"].as_bool() == Some(true) {
            "Dry run, nothing written: "
        } else {
            ""
        },
        v["imported"],
        v["same"],
        v["skipped"],
        v["logs"]["added"],
        v["logs"]["grown"]
    );
    for i in v["items"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "\n{} | {} | {} | {}{}",
            i["path"].as_str().unwrap_or("?"),
            i["kind"].as_str().unwrap_or("?"),
            i["outcome"].as_str().unwrap_or("?"),
            i["device"].as_str().unwrap_or("no device"),
            i["reason"]
                .as_str()
                .map(|r| format!(" | {r}"))
                .unwrap_or_default()
        ));
    }
    out
}

/// The preview in lines: checks, files, and the diff.
fn preview_text(v: &Value) -> String {
    let mut out = String::new();
    for c in v["checks"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "{} {}{}\n",
            if c["ok"].as_bool() == Some(true) {
                "ok"
            } else {
                "REFUSED"
            },
            c["name"].as_str().unwrap_or("?"),
            c["refusal"]["reason"]
                .as_str()
                .map(|r| format!(": {r}"))
                .unwrap_or_default()
        ));
    }
    for w in v["warnings"].as_array().into_iter().flatten() {
        out.push_str(&format!("warning: {}\n", w.as_str().unwrap_or("")));
    }
    let files: Vec<&str> = v["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if files.is_empty() {
        out.push_str("Nothing to change.\n");
    } else {
        out.push_str(&format!(
            "Would write {} ({} bytes, about {} s).\n",
            files.join(", "),
            v["bytes"],
            v["eta_s"]
        ));
    }
    for d in v["diff"].as_array().into_iter().flatten() {
        if d["kind"] != "lines" {
            continue;
        }
        out.push_str(&format!("--- {}\n", d["label"].as_str().unwrap_or("?")));
        for l in d["lines"].as_array().into_iter().flatten() {
            let sign = match l["op"].as_str() {
                Some("add") => '+',
                Some("remove") => '-',
                _ => ' ',
            };
            out.push_str(&format!("{sign}{}\n", l["text"].as_str().unwrap_or("")));
        }
    }
    out.trim_end().to_string()
}

fn gear_edit<B: Backend>(backend: &mut B, a: &Value) -> Result<(Vec<Value>, Value)> {
    let x: GearEditArgs = args(a)?;
    let action = x
        .action
        .clone()
        .context("action is required, for example device_save")?;
    match action.as_str() {
        "device_save" => {
            let id =
                x.id.context("id is required: a device id from quadcam_gear")?;
            let d = backend.call(
                "gear_device_save",
                json!({"id": id, "name": x.name, "aircraft": x.aircraft}),
            )?;
            Ok((vec![text(format!("Saved. {}", device_line(&d)))], d))
        }
        "device_forget" => {
            let id =
                x.id.context("id is required: a device id from quadcam_gear")?;
            let d = backend.call("gear_device_forget", json!({"id": id}))?;
            Ok((
                vec![text(format!(
                    "Forgot {}. Its backups stay.",
                    device_line(&d)
                ))],
                d,
            ))
        }
        "fc_read" => {
            let j = backend.call(
                "gear_fc_read",
                json!({"port": x.port, "commands": x.commands.unwrap_or_default()}),
            )?;
            let r = &j["result"];
            let mut out = fc_info_text(&r["info"], &j["notes"]);
            for rep in r["replies"].as_array().into_iter().flatten() {
                out.push_str(&format!(
                    "\n\n# {}\n{}",
                    rep["line"].as_str().unwrap_or("?"),
                    rep["text"].as_str().unwrap_or("")
                ));
            }
            Ok((vec![text(out)], j))
        }
        "backup" => {
            let v = backend.call(
                "gear_backup",
                json!({"device": x.device, "port": x.port, "mount": x.mount}),
            )?;
            Ok((vec![text(backup_result_text(&v))], v))
        }
        "backup_pin" => {
            let id = x
                .backup
                .context("backup is required for backup_pin: a backup id from quadcam_gear backups")?;
            let pinned = x.pinned.context("pinned is required for backup_pin: true or false")?;
            let v = backend.call("gear_backup_pin", json!({"id": id, "pinned": pinned}))?;
            Ok((vec![text(backup_line(&v))], v))
        }
        "prune" => {
            let v = backend.call("gear_prune", json!({"dry_run": x.dry_run.unwrap_or(false)}))?;
            let c = &v["collected"];
            Ok((
                vec![text(format!(
                    "{}{} backups dropped, {} kept; {} stored files removed ({}).",
                    if v["dry_run"].as_bool() == Some(true) {
                        "Dry run, nothing deleted: "
                    } else {
                        ""
                    },
                    v["dropped"].as_array().map_or(0, Vec::len),
                    v["kept"],
                    c["blobs"],
                    size(&c["bytes"])
                ))],
                v,
            ))
        }
        "export" => {
            let to = x.to.context("to is required for export: an existing folder")?;
            let v = backend.call(
                "gear_export",
                json!({"device": x.device, "snapshot": x.backup, "to": to}),
            )?;
            let folders: Vec<&str> = v["folders"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            Ok((
                vec![text(format!(
                    "Wrote {} files ({}) into:\n{}",
                    v["files"],
                    size(&v["bytes"]),
                    folders.join("\n")
                ))],
                v,
            ))
        }
        "import_backups" => {
            let folder = x
                .folder
                .context("folder is required for import_backups: the old backup folder")?;
            let v = backend.call(
                "gear_import_backups",
                json!({"folder": folder, "device": x.device, "dry_run": x.dry_run.unwrap_or(false)}),
            )?;
            Ok((vec![text(import_text(&v))], v))
        }
        "card_check" => {
            let v = backend.call(
                "gear_card_check",
                json!({"device": x.device, "mount": x.mount}),
            )?;
            let mut t = check_line(&v);
            if v["state"] == "failed" {
                t.push_str("\nTo repair it, ask the person, then call quadcam_gear_apply card_repair with this check's id as digest and confirm=true.");
            }
            Ok((vec![text(t)], v))
        }
        "stop" => {
            let handle = x
                .handle
                .context("handle is required for stop: a job's handle from quadcam_gear status")?;
            let v = backend.call("gear_stop", json!({"handle": handle}))?;
            Ok((
                vec![text(if v.as_bool() == Some(true) {
                    "Stopping: it ends after the file being read.".to_string()
                } else {
                    "Nothing is running on that device.".to_string()
                })],
                json!({"stopping": v}),
            ))
        }
        other => super::gear_flights::edit(backend, other, &x).unwrap_or_else(|| Err(anyhow!(
            "unknown action {other:?}; use device_save, device_forget, fc_read, backup, backup_pin, prune, export, import_backups, card_check, stop, flight_set, flight_folders, pack_save, pack_delete, pack_type_save, pack_type_delete, pack_notes, crash_save or crash_delete"
        ))),
    }
}

fn gear_apply<B: Backend>(backend: &mut B, a: &Value) -> Result<(Vec<Value>, Value)> {
    let x: GearApplyArgs = args(a)?;
    match x.action.as_deref().unwrap_or("") {
        "card_repair" => {
            let check = x.digest.context(
                "digest is required for card_repair: the id of the card's latest check, which failed",
            )?;
            if x.confirm != Some(true) {
                return Err(anyhow!(
                    "Refused: card_repair writes the card's file system; it needs confirm=true."
                ));
            }
            let v = backend.call("gear_card_repair", json!({"check": check, "confirm": true}))?;
            let mut t = match v["backup"].as_str() {
                Some(b) => format!("Backed up first: {b} (always kept)."),
                None => format!(
                    "No backup first: {}",
                    v["backup_error"]
                        .as_str()
                        .unwrap_or("this card is not backed up")
                ),
            };
            t.push_str(&format!(
                "\nRepair: {}\nCheck after: {}",
                check_line(&v["repair"]),
                check_line(&v["verify"])
            ));
            Ok((vec![text(t)], v))
        }
        other => Err(anyhow!(
            "Refused: {other:?} is not an apply action; use card_repair."
        )),
    }
}
