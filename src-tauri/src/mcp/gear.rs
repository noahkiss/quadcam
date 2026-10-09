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
    #[schemars(required, extend("enum" = ["status", "devices", "fc_identify", "board_notes", "usb_timers", "osd", "model", "rates", "sims", "card", "card_preview", "storage", "backups", "backup_read", "backup_diff", "card_checks", "flights", "packs", "session_report", "preflight", "crashes", "switch_map", "radio", "sim_calibration", "sim_defaults", "sim_validate", "changes", "apply_plan", "copy_plan"]))]
    pub action: Option<String>,
    /// For copy_plan: the FC to copy from, a device id (its latest backup) or a backup id from backups.
    #[schemars(length(max = 200))]
    pub from: Option<String>,
    /// For copy_plan: the FC that would get the settings (a device id).
    #[schemars(length(max = 80))]
    pub to: Option<String>,
    /// For copy_plan: settings to copy by name, on top of the parts.
    #[schemars(length(max = 200))]
    pub settings: Option<Vec<String>>,
    /// For copy_plan: the parts of the configuration to copy: rates, pid, osd, modes, adjustments, vtx, features.
    #[schemars(length(max = 7))]
    pub parts: Option<Vec<String>>,
    /// For changes: also list applied, failed and discarded changes (the history).
    pub history: Option<bool>,
    /// For changes: only this device's (the device field); for apply_plan: the staged change id from changes.
    #[schemars(length(max = 120))]
    pub change: Option<String>,
    /// For fc_identify and a live switch_map: the FC's serial port (/dev/cu.usbmodem...) from status; omit when one FC is plugged in.
    #[schemars(length(max = 200))]
    pub port: Option<String>,
    /// For board_notes: a board name (BETAFPVG473); omit for every board.
    #[schemars(length(max = 80))]
    pub board: Option<String>,
    /// For board_notes: a firmware version (2025.12.5); omit for every version.
    #[schemars(length(max = 80))]
    pub version: Option<String>,
    /// For osd, rates, sims and switch_map: Betaflight `dump all`, `diff all` or CLI-line files
    /// (absolute paths), read in order; a later file's lines win. For sim_validate: one
    /// folder of decoded blackbox logs (the CSV files blackbox_decode writes).
    #[schemars(length(max = 8))]
    pub paths: Option<Vec<String>>,
    /// For osd, rates, sims, switch_map and model: a saved device id instead of files (needs a backup of it). For card and
    /// card_preview: a connected radio's device id, instead of mount.
    #[schemars(length(max = 80))]
    pub device: Option<String>,
    /// For sims: the quad's rate profile to compare with (0-5); omit for the one in use.
    pub profile: Option<u8>,
    /// For osd: NTSC, PAL, HD or WxH; omit for the files' video system.
    #[schemars(length(max = 8))]
    pub grid: Option<String>,
    /// For osd with a device: show the layout after the device's staged OSD edits, with their check. For model: show the model after its staged edits (default true).
    pub staged: Option<bool>,
    /// For card and card_preview: the card's mount point. Default: the one EdgeTX card mounted. For switch_map: an EdgeTX card's mount or folder, or one model file.
    #[schemars(length(max = 1024))]
    pub mount: Option<String>,
    /// For model: the model file (model01.yml). For card: a model file (model01.yml) to read in full: timers, mixes, logical switches, special functions, switch warnings, sensors, screens. For switch_map: the model on the card; default the radio's selected model.
    #[schemars(length(max = 40))]
    pub model: Option<String>,
    /// For card_preview: the edits, each {"kind": "model", "file": "model01.yml", "name": "<header name>", "ops": [{"op": "set_checklist", "enabled": true}, ...]}, {"kind": "radio", "ops": [{"op": "set_scalar", "key": "hapticMode", "value": "mode_nokeys"}]}, {"kind": "checklist", "model": "model01.yml", "text": "=Props tight"}, {"kind": "model_copy", ...} or {"kind": "model_delete", "file": ...}.
    pub edits: Option<Vec<Value>>,
    /// For switch_map: also mark where each control is now, from the FC's channels (MSP) when an FC is plugged in, else from the radio in USB Joystick mode.
    pub live: Option<bool>,
    /// For rates and sims (a quad's backup, instead of the latest), backup_read and backup_diff: a backup id (<device>/<name>) from backups.
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
    /// For flights and crashes: an aircraft profile name. For switch_map: reads the latest backups of the radio and FC saved with that aircraft. For sim_validate: a built-in sim profile (meteor75, air65ii, five_inch, seven_inch).
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
    /// For sim_calibration: a saved radio's id or a provisional usb-… key; omit for the radio in USB Joystick mode now.
    #[schemars(length(max = 80))]
    pub radio: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct GearEditArgs {
    #[schemars(required, extend("enum" = ["device_save", "device_forget", "fc_read", "backup", "backup_pin", "prune", "export", "import_backups", "card_check", "stop", "poll_pause", "flight_set", "flight_folders", "pack_save", "pack_delete", "pack_type_save", "pack_type_delete", "pack_notes", "crash_save", "crash_delete", "report_save", "sim_calibration_save", "stage", "osd_edit", "model_edit", "update", "discard", "restore_stage", "copy_stage", "keep", "revert_stage", "card_mount", "card_unmount"]))]
    pub action: Option<String>,
    /// For copy_stage: the FC to copy from, a device id (its latest backup) or a backup id.
    #[schemars(length(max = 200))]
    pub from: Option<String>,
    /// For copy_stage: settings to copy by name, on top of the parts.
    #[schemars(length(max = 200))]
    pub settings: Option<Vec<String>>,
    /// For restore_stage on a radio card backup: the files to put back (paths from the card root).
    #[schemars(length(max = 200))]
    pub paths: Option<Vec<String>>,
    /// For card_mount: minutes to keep the card mounted (1-60, default 10).
    #[schemars(range(min = 1, max = 60))]
    pub minutes: Option<u32>,
    /// For update, keep and revert_stage: the staged change id from quadcam_gear changes.
    #[schemars(length(max = 120))]
    pub change: Option<String>,
    /// For stage and update: a short title for the change.
    #[schemars(length(max = 200))]
    pub title: Option<String>,
    /// For stage (with device) and update: raw Betaflight CLI lines, for example `set osd_cap_alarm = 1500`. Never save, exit, defaults or batch: QuadCam saves itself. A `profile N` or `rateprofile N` line selects the section for the lines after it.
    #[schemars(length(max = 200))]
    pub lines: Option<Vec<String>>,
    /// For osd_edit: moves, each {"element": "vbat", "x": 12, "y": 3, "profiles": [1, 2]}. Leave out what stays; empty profiles turn the element off. A position is shared by all profiles.
    pub moves: Option<Vec<Value>>,
    /// For osd_edit: make OSD profile `to` show what profile `from` shows, {"from": 1, "to": 2}.
    pub copy: Option<Value>,
    /// For model_edit: the model file (model01.yml).
    #[schemars(length(max = 40))]
    pub model: Option<String>,
    /// For model_edit: model ops, each {"op": ..., ...}: set_timer {index (0-2), fields: [{key, value}]}, remove_timer {index}, set_screen_values {index (0-3), lines: [["{RxBt}", "Tmr1"], ...]} (4 lines, 3 sources), set_screen {index, script}, set_logging {logging: {swtch, period_ds} | null}, set_sensor_logs {sensors: [{label, logs}]}, set_rf_alarms {warning, critical}, set_callout {callout: {track, when: "below"|"above"|"switch", source, value, delay_ds, swtch, repeat}}, remove_callout {track}, set_checklist {enabled}, rename {name}. A later op on the same setting replaces the earlier one.
    pub ops: Option<Vec<Value>>,
    /// For model_edit: the power-on checklist, one item per line; `=` starts a tick box; 20 characters per line on a 128x64 radio. Turns the checklist on. Empty text clears it.
    #[schemars(length(max = 8000))]
    pub checklist: Option<String>,
    /// For stage and update: typed edits instead of lines (kind fc_set, fc_lines, fc_aux, fc_adjrange, osd_element).
    pub edits: Option<Vec<Value>>,
    /// For update: draft, ready, try (apply, fly, then keep or revert) or read_first (read the real value on the device first).
    #[schemars(length(max = 10))]
    pub status: Option<String>,
    /// For update: the change's place in the device's queue.
    pub order: Option<u32>,
    /// For stage: keep the change as a draft.
    pub draft: Option<bool>,
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
    /// For poll_pause: true pauses QuadCam's own reads of the FC at `port`; false resumes them.
    pub paused: Option<bool>,
    /// For prune and import_backups: report what would happen; change nothing.
    pub dry_run: Option<bool>,
    /// For export: an existing folder (absolute path) to write the backup folders into.
    /// For report_save: the Markdown file (absolute path) to write; its folder must exist.
    /// For copy_stage: the FC that gets the settings (a device id).
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
    /// For report_save: true replaces a file that is already there.
    pub overwrite: Option<bool>,
    /// For crash_save: the day, YYYY-MM-DD (default: the clip's). For report_save: the day (default: the last import's days).
    #[schemars(length(max = 10))]
    pub day: Option<String>,
    /// For crash_save: what broke.
    #[schemars(length(max = 500))]
    pub broke: Option<String>,
    /// For crash_save: parts used for the repair. For copy_plan and copy_stage: the parts of the configuration to copy: rates, pid, osd, modes, adjustments, vtx, features.
    #[schemars(length(max = 40))]
    pub parts: Option<Vec<String>>,
    /// For crash_save: repaired.
    pub repaired: Option<bool>,
    /// For sim_calibration_save: the radio's key, a saved radio's id or the provisional usb-… key quadcam_gear sim_calibration gave.
    #[schemars(length(max = 80))]
    pub radio: Option<String>,
    /// For sim_calibration_save: the calibration, as quadcam_gear sim_calibration returns it (mode, roll, pitch, throttle, yaw, arm, reset).
    pub calibration: Option<Value>,
    /// For sim_calibration_save: the radio's USB product name; given, this radio becomes the answer for it when several saved radios share it.
    #[schemars(length(max = 120))]
    pub product: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct GearApplyArgs {
    #[schemars(required, extend("enum" = ["card_repair", "apply"]))]
    pub action: Option<String>,
    /// For card_repair: the id of the card's latest check, which failed (quadcam_gear card_checks or status). For apply: the digest from quadcam_gear apply_plan.
    #[schemars(length(max = 200))]
    pub digest: Option<String>,
    /// For apply: the staged change id from quadcam_gear changes.
    #[schemars(length(max = 120))]
    pub change: Option<String>,
    /// For apply: the FC's serial port, when several FCs are plugged in.
    #[schemars(length(max = 200))]
    pub port: Option<String>,
    /// Must be true.
    pub confirm: Option<bool>,
}

// ----- tool list -----

/// The Gear tools' descriptors, after the other tools.
pub fn tools() -> Vec<Value> {
    vec![
        tool::<GearArgs>(
            "quadcam_gear",
            "Read the FPV gear QuadCam knows: `status` (the gear folder, the Gear settings, and the devices plugged in now: EdgeTX radios in USB Storage mode, goggles and DVR cards, FC and ELRS serial ports, radios in DFU mode; each with its saved name and aircraft when QuadCam knows it; and each FC's USB heat timer), `devices` (every device saved in gear.json: id, kind, name, aircraft, board, firmware, version, last seen, last backup), `fc_identify` (reads a Betaflight FC over MSP: board, firmware, version, device id, whether QuadCam may write it, known issues; no reboot), `board_notes` (known issues of FC boards and builds), `usb_timers` (per FC on USB: battery in, minutes on USB, minutes left before \"Unplug now\"), `osd` (a Betaflight OSD layout from `paths`, dump or diff files read in order: each OSD profile drawn on its grid (NTSC 30x13, PAL 30x16, HD 53x20, from vcd_video_system or `grid`), the elements on in each profile with x and y, and the check for overlaps and cells off screen; with a `device` and `staged`, the layout after its staged OSD edits), `model` (a saved radio's model for the editors, from `device` and `model` (model01.yml; default the selected one): timers, value screens, logging and which sensors the log records, RSSI alarms, callouts, the checklist, the sounds a callout can name; read from the mounted card, else the latest backup, with the device's staged model edits on top unless `staged` is false), `rates` (an FC's rate profiles from `paths` (dump or diff files), a saved `device`'s latest backup, or a backup `id`: each profile's name and whether it is in use, the rates type, roll, pitch and yaw rc rate, super rate, expo, rate limit, maximum and centre rates in degrees per second, and the throttle curve), `sims` (the sims on this Mac, with the rate profiles QuadCam reads from their files and whether each runs; with `paths`, `device` or `id` for a quad, how each sim profile differs from the quad's rate profile in use, or the one in `profile`), `card` (an EdgeTX SD card: board and version, whether QuadCam may write it, its models, the model the radio selects and that model's aircraft, the radio clock check; with `model`, that model in full) `card_preview` (the checks and line diff of EdgeTX card edits, and how long the write would take; writes nothing), `storage` (the gear folder's size in total and per device: snapshots by kind, logs, blobs only that device uses), `backups` (snapshots newest first, or one `device`'s: id, why it was taken, time, files, size, pinned), `backup_read` (a backup `id`'s file list, or one file's text with `path`), `backup_diff` (what changed from `against`, or the backup before, to `id`: files put and removed, a line diff per text file), `card_checks` (a card `device`'s file-system checks, newest first), `flights` (flights from the radio logs, newest first: hover throttle, sag, resting voltage, mAh, when the pack type's mAh warning was crossed, the worst link, dropouts; each with its aircraft, pack (or a suggested one), place and library clip; filter by `day`, `aircraft`, `pack`, `place`), `packs` (packs with cycles, charge state, resting voltage and a weak mark; pack types with the charging sheet and a suggested mAh warning; the charging notes), `session_report` (one flying day, by default the last import's days: flights, air time, longest flight, worst link, dropouts, pack use, crashes; as Markdown to share), `preflight` (the \"Pack up\" check: packs charged, the radio's model, card space, backups, cards still in the Mac; each pass, warn or unknown) `crashes` (the crash and repair log, by `aircraft` or `clip`), `switch_map` (what each radio control does: per switch, trim or stick position, the channel values in microseconds, the Betaflight modes and adjustments they select, and the radio's logical switches, special functions and timers; from `mount` (an EdgeTX card or model file) and `paths` (the FC's dump or diff), or the latest backups of a saved `device` or of an `aircraft`'s radio and FC; conflicts such as two modes on one range, a switch with no effect, a mode no switch reaches, a sound file the card lacks; with `live`, the position each control is in now), `radio` (the radio in USB Joystick mode now: buttons, axes, channel values), `sim_calibration` (the sim's calibration of a `radio`, or of the one in USB Joystick mode now, matched to a saved radio: the match, or the choices when several saved radios share its model) `sim_defaults` (what the sim pre-fills from an `aircraft`'s radio model and quad modes, or from `mount` and `paths`: stick channels, arm, angle, horizon, turtle and air mode switches, a free reset control, each with its source) `changes` (staged changes waiting to be applied: id, device, title, status, edits; `history` adds applied, failed and discarded ones), `apply_plan` (every check for a staged `change` on an FC or a radio card, its diff and the digest an apply needs; writes nothing, does not reboot the FC, and for an unmounted card mounts it to read and unmounts it again), `copy_plan` (what copying `parts` (rates, pid, osd, modes, adjustments, vtx, features) and named `settings` from the FC `from` (a device id or a backup id) to the FC `to` would stage: compatibility checks, the diff, what is left out and why; writes nothing) or `sim_validate` (a sim profile, `aircraft`, checked against a folder of decoded blackbox logs, `paths`: hover, punch, sag, roll, pitch and yaw response, coast-down and fall recovery, each against its band). Changes nothing.\n\nBest for: the first Gear call, checking what is plugged in, and checking an OSD layout before or after an edit, reading or planning radio model changes, and answering \"what does this switch do\".\nReturns: one line per device plus the structured records; for osd and switch_map, the drawn view as text plus the structured view.\nFollow up with quadcam_gear_edit device_save to name a device or link it to an aircraft, flight_set to put a flight on a pack.",
            json!({"openWorldHint": false, "readOnlyHint": true, "title": "Gear"}),
        ),
        tool::<GearEditArgs>(
            "quadcam_gear_edit",
            "Change QuadCam's own gear data, never a device's settings, a sim or a card: `device_save` names a device or links it to an aircraft profile (a device QuadCam does not know yet must be plugged in; use its id from quadcam_gear status or fc_identify), `device_forget` removes a device from QuadCam's list (its backups stay), `fc_read` reads a Betaflight FC through its CLI (read-only commands; the FC reboots when the read ends, so the person should expect it; returns the text, writes nothing), `backup` backs up a radio card (`device` or `mount`; files whose size and time did not change are not read) or an FC (`port` or `device`; the FC reboots) into QuadCam's gear folder (nothing is written when nothing changed), `backup_pin` keeps a `backup` through pruning (`pinned`), `prune` thins backups by the retention settings and removes stored files nothing uses (`dry_run` to see first), `export` writes a `backup`, or every backup of a `device`, as plain folders into `to`, `import_backups` takes an old backup `folder` (radio card copies, Betaflight diff all and dump all files, LOGS folders, dated by YYYY-MM-DD in folder names; `dry_run` to see first; it never changes the folder), `card_check` checks a card's file system (diskutil verifyVolume, read-only, about 30 s over a radio's USB; the card unmounts and mounts again), `stop` stops a running backup or card check by its `handle`, `poll_pause` pauses (`paused` true) or resumes (false) QuadCam's own background reads of an FC's `port`: the USB timer's battery probe, which then stops counting, and the on-connect FC backup (QuadCam already skips a port another program has open; pause it when you want a tool to have the port without any chance of a probe; jobs you start still run, and the pause lasts until QuadCam quits), `flight_set` (a flight's pack or place), `flight_folders` (add or remove a folder of radio logs the flights read), `pack_save` / `pack_delete` (a pack by `name`, its label; `charged=true` marks it charged now), `pack_type_save` / `pack_type_delete` (a pack type by `name`: chemistry, cells, capacity, connector, charge volts per cell, charge current, mAh warning), `pack_notes` (the charging sheet's notes), `crash_save` / `crash_delete` (a crash on a library clip: the time in the clip, what broke, the parts used; no `id` logs a new one), `report_save` (writes the session report for a `day`, by default the last import's days, as Markdown to the file `to`; `overwrite` replaces an existing file), `sim_calibration_save` (the sim's calibration of a `radio`, keyed by its Gear radio id), `stage` (queues FC edits for a `device`: raw CLI `lines` or typed `edits`; refuses a setting the FC's latest backup does not hold; writes only QuadCam's own data), `osd_edit` (moves, toggles or a profile `copy` of the OSD layout for an FC `device`, joined into its one OSD layout change; check the result with quadcam_gear osd and `staged`), `model_edit` (`ops` and a `checklist` for one `model` of a radio `device`, joined into its one \"Model edits\" change; read the result with quadcam_gear model; a callout is owned by its track name, so a second callout with that track replaces the first), `update` and `discard` (a staged `change`), `restore_stage` (stages a `backup` back as a change: an FC backup's settings, or for a radio card backup the card files named in `paths`), `copy_stage` (stages the settings quadcam_gear copy_plan showed as one change for the FC `to`), `keep` (an applied Try `change` becomes Verified), `revert_stage` (stages a restore of the backup an applied `change` took; the change becomes Reverted when that restore verifies), `card_mount` / `card_unmount` (mounts an unmounted radio card for the person to browse, for `minutes` (default 10), and unmounts it; neither writes the card). `update` also sets a change's status: draft, ready, try or read_first.\n\nBest for: naming a radio or quad the person just plugged in, and linking it to its aircraft profile; reading an FC's settings as text; backing gear up and importing old backups; putting flights on packs; logging a crash and its repair.\nReturns: the saved or forgotten device, the FC's identity and each command's answer, a summary of the backup, prune, export, import or check, or the saved flight, pack, pack type or crash.",
            json!({"destructiveHint": false, "idempotentHint": true, "openWorldHint": false, "readOnlyHint": false, "title": "Edit gear data"}),
        ),
        tool::<GearApplyArgs>(
            "quadcam_gear_apply",
            "Write to a device, a sim or the radio firmware: each action needs a digest and confirm=true. `card_repair` repairs a card whose latest check failed (diskutil repairVolume): `digest` is that check's id (quadcam_gear card_checks or status); QuadCam backs the card up first when it reads, repairs it, and checks it again. A repair cannot be stopped once it starts. `apply` writes a staged FC or radio card `change`: call quadcam_gear apply_plan first, show the person the diff, then pass its `digest` and confirm=true; for an FC QuadCam backs it up (always kept), sends the lines, saves, waits for the reboot and checks every line against `dump all`, and nothing is saved when a line fails; for a radio card it mounts the card if needed, backs it up (always kept), writes file by file, reads each back, puts every file back if one reads wrong, and unmounts it (safe to unplug only after the unmount). With the app running, the person must also click Apply in its sheet (3 minutes, else refused).\n\nBest for: repairing a radio or DVR card after a failed card check, and applying a staged FC or radio card change the person has seen, when they ask.\nReturns: the backup taken first, the repair's result and the check after it.",
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
            for f in s["failures"].as_array().into_iter().flatten() {
                line.push_str(&format!(
                    "\n{} failed on {}: {}",
                    f["step"].as_str().unwrap_or("?"),
                    f["device"].as_str().unwrap_or(f["handle"].as_str().unwrap_or("?")),
                    f["message"].as_str().unwrap_or("")
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
                    "staged": x.staged.unwrap_or(false),
                }),
            )?;
            let view: crate::gear::osd::OsdView =
                serde_json::from_value(v.clone()).context("bad osd answer")?;
            Ok((vec![text(crate::gear::osd::render_text(&view))], v))
        }
        "model" => {
            let device = x
                .device
                .clone()
                .context("device is required for model: a radio's device id from quadcam_gear devices")?;
            let v = backend.call(
                "gear_model",
                json!({"device": device, "model": x.model, "staged": x.staged.unwrap_or(true)}),
            )?;
            let d: crate::core::ModelDetail =
                serde_json::from_value(v.clone()).context("bad model answer")?;
            Ok((vec![text(crate::core::model_edit::render_text(&d))], v))
        }
        "rates" => {
            let v = backend.call(
                "gear_rates",
                json!({
                    "paths": x.paths.unwrap_or_default(),
                    "device": x.device,
                    "backup": x.id,
                }),
            )?;
            let view: crate::gear::rates::RatesView =
                serde_json::from_value(v.clone()).context("bad rates answer")?;
            Ok((vec![text(crate::gear::rates::render_text(&view))], v))
        }
        "sims" => {
            let v = backend.call(
                "gear_sims",
                json!({
                    "paths": x.paths.unwrap_or_default(),
                    "device": x.device,
                    "backup": x.id,
                    "profile": x.profile,
                }),
            )?;
            let list: Vec<crate::gear::sims::SimStatus> =
                serde_json::from_value(v.clone()).context("bad sims answer")?;
            Ok((
                vec![text(crate::gear::sims::render_text(&list))],
                json!({"sims": v}),
            ))
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
        "sim_calibration" => {
            let v = backend.call("gear_sim_calibration", json!({"radio": x.radio}))?;
            Ok((vec![text(sim_calibration_text(&v))], v))
        }
        "sim_validate" => {
            let aircraft = x
                .aircraft
                .clone()
                .context("aircraft is required: a sim profile, such as meteor75")?;
            let logs = x
                .paths
                .clone()
                .and_then(|p| p.into_iter().next())
                .context("paths is required: one folder of decoded blackbox logs")?;
            let v = backend.call(
                "gear_sim_validate",
                json!({"aircraft": aircraft, "logs": logs}),
            )?;
            Ok((vec![text(sim_validate_text(&v))], v))
        }
        "sim_defaults" => {
            let v = backend.call(
                "gear_sim_defaults",
                json!({"aircraft": x.aircraft, "radio": x.mount, "fc": x.paths.unwrap_or_default()}),
            )?;
            Ok((vec![text(sim_defaults_text(&v))], v))
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
        "changes" => {
            let v = backend.call(
                "gear_changes",
                json!({"device": x.device, "history": x.history.unwrap_or(false)}),
            )?;
            let list = v.as_array().cloned().unwrap_or_default();
            let t = if list.is_empty() {
                "No staged changes.".to_string()
            } else {
                list.iter().map(change_line).collect::<Vec<_>>().join("\n")
            };
            Ok((vec![text(t)], v))
        }
        "copy_plan" => {
            let (from, to) = (
                x.from.clone().context("from is required for copy_plan: a device id or a backup id")?,
                x.to.clone().context("to is required for copy_plan: the FC that would get the settings")?,
            );
            let v = backend.call(
                "gear_copy_plan",
                json!({"from": from, "to": to, "parts": x.parts.clone().unwrap_or_default(), "settings": x.settings.clone().unwrap_or_default()}),
            )?;
            Ok((vec![text(copy_text(&v))], v))
        }
        "apply_plan" => {
            let id = x
                .change
                .context("change is required for apply_plan: a staged change id from changes")?;
            let v = backend.call("gear_apply_plan", json!({"id": id, "port": x.port}))?;
            let mut t = preview_text(&v);
            if v["checks"].as_array().is_some_and(|c| c.iter().all(|c| c["ok"] == true)) {
                t.push_str(&format!(
                    "\nTo apply: show the person the diff, then call quadcam_gear_apply apply with change={id}, digest={} and confirm=true.",
                    v["digest"].as_str().unwrap_or("?")
                ));
            }
            Ok((vec![text(t)], v))
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
            "unknown action {other:?}; use status, devices, fc_identify, board_notes, usb_timers, osd, rates, sims, card, card_preview, storage, backups, backup_read, backup_diff, card_checks, switch_map, radio, sim_calibration, sim_defaults, sim_validate, flights, packs, session_report, preflight or crashes"
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

/// One staged change in a line, with what it does.
fn change_line(c: &Value) -> String {
    let edits: Vec<String> = c["edits"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| match e["kind"].as_str() {
            Some("fc_set") => format!(
                "set {} = {}",
                e["name"].as_str().unwrap_or("?"),
                e["value"].as_str().unwrap_or("?")
            ),
            Some("fc_lines") => e["lines"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("; "),
            Some(k) => k.to_string(),
            None => "?".into(),
        })
        .collect();
    format!(
        "{} | {} | {} | {} | {}",
        c["id"].as_str().unwrap_or("?"),
        c["status"].as_str().unwrap_or("?"),
        c["device"].as_str().unwrap_or("?"),
        c["title"].as_str().unwrap_or(""),
        edits.join("; ")
    )
}

/// The edits an agent gave: typed `edits`, else `lines` as one raw CLI edit.
fn change_edits(x: &GearEditArgs) -> Result<Vec<Value>> {
    let mut out: Vec<Value> = x.edits.clone().unwrap_or_default();
    if let Some(l) = &x.lines {
        out.push(json!({"kind": "fc_lines", "lines": l}));
    }
    if out.is_empty() {
        return Err(anyhow!(
            "lines or edits is required: for example lines [\"set osd_cap_alarm = 1500\"]"
        ));
    }
    Ok(out)
}

/// An apply's result in words.
fn copy_text(v: &Value) -> String {
    let mut out = String::new();
    for c in v["checks"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "{} {}{}\n",
            if c["ok"] == true { "pass" } else { "FAIL" },
            c["name"].as_str().unwrap_or("?"),
            c["refusal"]["reason"]
                .as_str()
                .map(|r| format!(": {r}"))
                .unwrap_or_default()
        ));
    }
    for l in v["diff"].as_array().into_iter().flatten() {
        let mark = match l["op"].as_str() {
            Some("add") => "+",
            Some("remove") => "-",
            _ => " ",
        };
        out.push_str(&format!("{mark}{}\n", l["text"].as_str().unwrap_or("")));
    }
    out.push_str(&format!(
        "{} already the same.\n",
        v["same"].as_u64().unwrap_or(0)
    ));
    for s in v["skipped"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        out.push_str(&format!("Left out: {s}\n"));
    }
    for n in v["notes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        out.push_str(&format!("Note: {n}\n"));
    }
    if v["edits"].as_array().is_some_and(|e| !e.is_empty()) {
        out.push_str(
            "To stage it: quadcam_gear_edit copy_stage with the same from, to (device) and parts.",
        );
    }
    out.trim_end().to_string()
}

fn apply_text(v: &Value) -> String {
    let mut out = format!("{}\n", v["message"].as_str().unwrap_or("?"));
    for s in v["steps"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "{}: {}{}\n",
            s["name"].as_str().unwrap_or("?"),
            s["state"].as_str().unwrap_or("?"),
            s["detail"]
                .as_str()
                .map(|d| format!(" ({d})"))
                .unwrap_or_default()
        ));
    }
    if let Some(b) = v["backup"].as_str() {
        out.push_str(&format!("Backup before (kept): {b}\n"));
    }
    for f in v["verify"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "Did not read back: {} (found {})\n",
            f["line"].as_str().unwrap_or("?"),
            f["found"].as_str().unwrap_or("nothing")
        ));
    }
    if v["status"] == "failed" && v["saved"] == true {
        out.push_str("To undo it, stage restore_stage with the backup above.\n");
    }
    for n in v["notes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        out.push_str(&format!("Note: {n}\n"));
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
        "sim_calibration_save" => {
            let radio = x
                .radio
                .context("radio is required: a saved radio's id or the usb-… key from quadcam_gear sim_calibration")?;
            let cal = x
                .calibration
                .context("calibration is required: the calibration from quadcam_gear sim_calibration, changed")?;
            let v = backend.call(
                "gear_sim_calibration_save",
                json!({"radio": radio, "calibration": cal, "product": x.product, "remember": x.product.is_some()}),
            )?;
            Ok((vec![text(format!("Saved the sim calibration of {radio}."))], v))
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
        "stage" => {
            let device = x
                .device
                .clone()
                .context("device is required for stage: an FC's device id from quadcam_gear")?;
            let edits = change_edits(&x)?;
            let v = backend.call(
                "gear_change_stage",
                json!({"device": device, "title": x.title, "edits": edits, "note": x.note, "editor": "agent", "draft": x.draft.unwrap_or(false)}),
            )?;
            Ok((vec![text(format!("Staged. {}\nNothing is written until quadcam_gear apply_plan and quadcam_gear_apply.", change_line(&v)))], v))
        }
        "osd_edit" => {
            let device = x
                .device
                .clone()
                .context("device is required for osd_edit: an FC's device id from quadcam_gear")?;
            let v = backend.call(
                "gear_osd_edit",
                json!({"device": device, "moves": x.moves.clone().unwrap_or_default(), "copy": x.copy, "editor": "agent"}),
            )?;
            Ok((vec![text(format!("Staged. {}\nNothing is written until quadcam_gear apply_plan and quadcam_gear_apply.", change_line(&v)))], v))
        }
        "model_edit" => {
            let device = x
                .device
                .clone()
                .context("device is required for model_edit: a radio's device id from quadcam_gear")?;
            let model = x
                .model
                .clone()
                .context("model is required for model_edit: a model file such as model01.yml (quadcam_gear model lists them)")?;
            let v = backend.call(
                "gear_model_edit",
                json!({"device": device, "model": model, "ops": x.ops.clone().unwrap_or_default(), "checklist": x.checklist, "editor": "agent"}),
            )?;
            Ok((vec![text(format!("Staged. {}\nNothing is written until quadcam_gear apply_plan and quadcam_gear_apply.", change_line(&v)))], v))
        }
        "update" => {
            let id = x.change.clone().context("change is required for update: a staged change id")?;
            let edits = if x.lines.is_some() || x.edits.is_some() {
                Some(change_edits(&x)?)
            } else {
                None
            };
            let v = backend.call(
                "gear_change_update",
                json!({"id": id, "title": x.title, "edits": edits, "status": x.status, "note": x.note, "order": x.order}),
            )?;
            Ok((vec![text(format!("Updated. {}", change_line(&v)))], v))
        }
        "discard" => {
            let id = x.change.clone().context("change is required for discard: a staged change id")?;
            let v = backend.call("gear_change_discard", json!({"id": id}))?;
            Ok((vec![text(format!("Discarded. {}", change_line(&v)))], v))
        }
        "restore_stage" => {
            let b = x.backup.clone().context("backup is required for restore_stage: a backup id from quadcam_gear backups")?;
            let v = backend.call("gear_restore_stage", json!({"backup": b, "paths": x.paths.clone().unwrap_or_default(), "editor": "agent"}))?;
            Ok((vec![text(format!("Staged. {}", change_line(&v)))], v))
        }
        "copy_stage" => {
            let (from, to) = (
                x.from.clone().context("from is required for copy_stage: a device id or a backup id")?,
                x.to.clone().context("to is required for copy_stage: the FC's device id")?,
            );
            let v = backend.call(
                "gear_copy_stage",
                json!({"from": from, "to": to, "parts": x.parts.clone().unwrap_or_default(), "settings": x.settings.clone().unwrap_or_default(), "editor": "agent"}),
            )?;
            Ok((vec![text(format!("Staged. {}\nNothing is written until quadcam_gear apply_plan and quadcam_gear_apply.", change_line(&v)))], v))
        }
        "keep" => {
            let id = x.change.clone().context("change is required for keep: an applied Try change id")?;
            let v = backend.call("gear_change_keep", json!({"id": id}))?;
            Ok((vec![text(format!("Kept. {}", change_line(&v)))], v))
        }
        "revert_stage" => {
            let id = x.change.clone().context("change is required for revert_stage: an applied change id")?;
            let v = backend.call("gear_change_revert", json!({"id": id}))?;
            Ok((vec![text(format!("Staged a revert. {}\nNothing is written until quadcam_gear apply_plan and quadcam_gear_apply.", change_line(&v)))], v))
        }
        "card_mount" => {
            let device = x.device.clone().context("device is required for card_mount: a radio card's device id")?;
            let v = backend.call("gear_card_mount", json!({"device": device, "minutes": x.minutes}))?;
            Ok((vec![text(format!("Mounted at {}. QuadCam unmounts it at {} unless you call card_unmount first.", v["mount"].as_str().unwrap_or("?"), v["until"].as_str().unwrap_or("?")))], v))
        }
        "card_unmount" => {
            let device = x.device.clone().context("device is required for card_unmount: a radio card's device id")?;
            let v = backend.call("gear_card_unmount", json!({"device": device}))?;
            Ok((vec![text("Unmounted: safe to unplug.".to_string())], json!({"unmounted": v})))
        }
        "poll_pause" => {
            let paused = x.paused.unwrap_or(true);
            let v = backend.call("gear_poll_pause", json!({"port": x.port, "paused": paused}))?;
            let list = v.as_array().cloned().unwrap_or_default();
            let line = if paused {
                "Paused: QuadCam will not read that FC until you resume it or quit."
            } else if list.is_empty() {
                "Resumed. No FC is paused."
            } else {
                "Resumed."
            };
            Ok((vec![text(line.to_string())], json!({"paused": v})))
        }
        other => super::gear_flights::edit(backend, other, &x).unwrap_or_else(|| Err(anyhow!(
            "unknown action {other:?}; use device_save, device_forget, fc_read, backup, backup_pin, prune, export, import_backups, card_check, stop, poll_pause, flight_set, flight_folders, pack_save, pack_delete, pack_type_save, pack_type_delete, pack_notes, crash_save, crash_delete, report_save, sim_calibration_save, stage, update, discard, restore_stage, copy_stage, keep, revert_stage, card_mount or card_unmount"
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
        "apply" => {
            let id = x.change.context(
                "change is required for apply: a staged change id from quadcam_gear changes",
            )?;
            let digest = x
                .digest
                .context("digest is required for apply: the digest from quadcam_gear apply_plan")?;
            if x.confirm != Some(true) {
                return Err(anyhow!(
                    "Refused: apply writes the device; it needs the plan's digest and confirm=true."
                ));
            }
            let v = backend.call(
                "gear_apply",
                json!({"id": id, "digest": digest, "confirm": true, "port": x.port}),
            )?;
            Ok((vec![text(apply_text(&v))], v))
        }
        other => Err(anyhow!(
            "Refused: {other:?} is not an apply action; use card_repair or apply."
        )),
    }
}

/// One stick axis: `roll CH1 0-2048, centre 1024, deadzone 0 %`.
fn axis_line(name: &str, a: &Value) -> String {
    let n = |k: &str| a[k].as_u64().unwrap_or(0);
    let end = |e: &str, auto: &str| {
        a[e].as_u64()
            .map(|v| format!("{v} (edited)"))
            .unwrap_or_else(|| n(auto).to_string())
    };
    format!(
        "{name} CH{} {}-{}, centre {}, deadzone {} %{}",
        n("ch"),
        end("edited_low", "auto_low"),
        end("edited_high", "auto_high"),
        n("centre"),
        n("deadzone"),
        if a["reverse"].as_bool() == Some(true) {
            ", reversed"
        } else {
            ""
        }
    )
}

fn control_text(c: &Value) -> String {
    match c["kind"].as_str() {
        Some("channel") => format!("CH{} {}-{} µs", c["ch"], c["min_us"], c["max_us"]),
        Some("button") => format!(
            "button {} {}",
            c["button"],
            if c["pressed"].as_bool() == Some(true) {
                "pressed"
            } else {
                "released"
            }
        ),
        _ => "none".into(),
    }
}

fn sim_calibration_text(v: &Value) -> String {
    let mut out = String::new();
    if let Some(r) = v["resolution"].as_object() {
        let radio = r["radio"].as_str().unwrap_or("not decided");
        out.push_str(&format!(
            "{}: {radio}. {}\n",
            r["product"].as_str().unwrap_or("radio"),
            r["how"].as_str().unwrap_or("")
        ));
        if r["radio"].is_null() {
            for c in r["choices"].as_array().into_iter().flatten() {
                out.push_str(&format!(
                    "  - {} ({})\n",
                    c["name"].as_str().unwrap_or(""),
                    c["id"].as_str().unwrap_or("")
                ));
            }
        }
    }
    match v["calibration"].as_object() {
        Some(s) => {
            let c = &s["calibration"];
            out.push_str(&format!(
                "Mode {}, saved {}\n",
                c["mode"],
                s["saved_at"].as_str().unwrap_or("")
            ));
            for f in ["roll", "pitch", "throttle", "yaw"] {
                out.push_str(&format!("  {}\n", axis_line(f, &c[f])));
            }
            out.push_str(&format!(
                "  arm {}, reset {}",
                control_text(&c["arm"]),
                control_text(&c["reset"])
            ));
        }
        None => out.push_str("No calibration saved: the calibration screen opens on its own."),
    }
    out.trim_end().to_string()
}

fn sim_validate_text(v: &Value) -> String {
    let checks = v["checks"].as_array().cloned().unwrap_or_default();
    let mut out = format!(
        "Sim profile {} on {} log(s):\n",
        v["profile"].as_str().unwrap_or(""),
        v["logs"].as_array().map_or(0, |l| l.len())
    );
    let mut failed = 0;
    for c in &checks {
        let outcome = c["outcome"].as_str().unwrap_or("");
        let line = match outcome {
            "not_in_log" => format!("{}: not in the logs", c["check"].as_str().unwrap_or("")),
            _ => {
                let band = if c["relative"].as_bool() == Some(true) {
                    format!("±{:.0} %", c["band"].as_f64().unwrap_or(0.0) * 100.0)
                } else {
                    format!("±{} {}", c["band"], c["unit"].as_str().unwrap_or(""))
                };
                if outcome == "fail" {
                    failed += 1;
                }
                format!(
                    "{} {} ({}): logged {:.3}, sim {:.3}, band {band}: {}",
                    c["check"].as_str().unwrap_or(""),
                    c["measure"].as_str().unwrap_or(""),
                    c["log"].as_str().unwrap_or(""),
                    c["logged"].as_f64().unwrap_or(0.0),
                    c["sim"].as_f64().unwrap_or(0.0),
                    outcome
                )
            }
        };
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str(&if failed == 0 {
        "Every check that ran passed.".to_string()
    } else {
        format!("{failed} check(s) failed.")
    });
    out
}

fn sim_defaults_text(v: &Value) -> String {
    let m = &v["map"];
    let mut out = format!(
        "Sticks: roll CH{}, pitch CH{}, throttle CH{}, yaw CH{} (from {})\n",
        m["roll"],
        m["pitch"],
        m["throttle"],
        m["yaw"],
        m["source"].as_str().unwrap_or("")
    );
    for (k, name) in [
        ("arm", "Arm"),
        ("angle", "Angle"),
        ("horizon", "Horizon"),
        ("turtle", "Turtle"),
        ("airmode", "Air mode"),
        ("reset", "Reset"),
    ] {
        let s = &v[k];
        if s.is_object() {
            out.push_str(&format!(
                "{name}: {} (from {}; {})\n",
                s["label"].as_str().unwrap_or(""),
                s["source"].as_str().unwrap_or(""),
                control_text(&s["control"])
            ));
        }
    }
    for n in v["notes"].as_array().into_iter().flatten() {
        out.push_str(&format!("Note: {}\n", n.as_str().unwrap_or("")));
    }
    out.trim_end().to_string()
}
