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
    #[schemars(required, extend("enum" = ["status", "devices", "fc_identify", "board_notes", "usb_timers", "osd", "card", "card_preview", "switch_map", "radio"]))]
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
    /// For osd: a saved device id instead of files (needs a backup of it). For card and
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
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct GearEditArgs {
    #[schemars(required, extend("enum" = ["device_save", "device_forget", "fc_read"]))]
    pub action: Option<String>,
    /// For fc_read: the FC's serial port from quadcam_gear status; omit when one FC is plugged in.
    #[schemars(length(max = 200))]
    pub port: Option<String>,
    /// For fc_read: read-only CLI commands (version, status, get NAME, diff all, dump all, diff/dump master|profile|rates|hardware|defaults). Omit for a backup's set: version, status, diff all, dump all.
    #[schemars(length(max = 20))]
    pub commands: Option<Vec<String>>,
    /// For device_save and device_forget: the device id from quadcam_gear status or devices.
    #[schemars(length(max = 80))]
    pub id: Option<String>,
    /// For device_save: the person's name for the device; empty for none.
    #[schemars(length(max = 80))]
    pub name: Option<String>,
    /// For device_save: an aircraft profile name from quadcam_profiles; empty to unlink.
    #[schemars(length(max = 80))]
    pub aircraft: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct GearApplyArgs {
    /// No apply actions in this version.
    #[schemars(required)]
    pub action: Option<String>,
    /// The digest from the plan (quadcam_gear apply_plan).
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
            "Read the FPV gear QuadCam knows: `status` (the gear folder, the Gear settings, and the devices plugged in now: EdgeTX radios in USB Storage mode, goggles and DVR cards, FC and ELRS serial ports, radios in DFU mode; each with its saved name and aircraft when QuadCam knows it; and each FC's USB heat timer), `devices` (every device saved in gear.json: id, kind, name, aircraft, board, firmware, version, last seen, last backup), `fc_identify` (reads a Betaflight FC over MSP: board, firmware, version, device id, whether QuadCam may write it, known issues; no reboot), `board_notes` (known issues of FC boards and builds), `usb_timers` (per FC on USB: battery in, minutes on USB, minutes left before \"Unplug now\"), `osd` (a Betaflight OSD layout from `paths`, dump or diff files read in order: each OSD profile drawn on its grid (NTSC 30x13, PAL 30x16, HD 53x20, from vcd_video_system or `grid`), the elements on in each profile with x and y, and the check for overlaps and cells off screen), `card` (an EdgeTX SD card: board and version, whether QuadCam may write it, its models, the model the radio selects and that model's aircraft, the radio clock check; with `model`, that model in full), `card_preview` (the checks and line diff of EdgeTX card edits, and how long the write would take; writes nothing), `switch_map` (what each radio control does: per switch, trim or stick position, the channel values in microseconds, the Betaflight modes and adjustments they select, and the radio's logical switches, special functions and timers; from `mount` (an EdgeTX card or model file) and `paths` (the FC's dump or diff); conflicts such as two modes on one range, a switch with no effect, a mode no switch reaches, a sound file the card lacks; with `live`, the position each control is in now) or `radio` (the radio in USB Joystick mode now: buttons, axes, channel values). Changes nothing.\n\nBest for: the first Gear call, checking what is plugged in, and checking an OSD layout before or after an edit, reading or planning radio model changes, and answering \"what does this switch do\".\nReturns: one line per device plus the structured records; for osd and switch_map, the drawn view as text plus the structured view.\nFollow up with quadcam_gear_edit device_save to name a device or link it to an aircraft.",
            json!({"openWorldHint": false, "readOnlyHint": true, "title": "Gear"}),
        ),
        tool::<GearEditArgs>(
            "quadcam_gear_edit",
            "Change QuadCam's own gear data, never a device's settings, a sim or a card: `device_save` names a device or links it to an aircraft profile (a device QuadCam does not know yet must be plugged in; use its id from quadcam_gear status or fc_identify), `device_forget` removes a device from QuadCam's list (its backups stay), `fc_read` reads a Betaflight FC through its CLI (read-only commands; the FC reboots when the read ends, so the person should expect it; returns the text, writes nothing).\n\nBest for: naming a radio or quad the person just plugged in, and linking it to its aircraft profile; reading an FC's settings as text.\nReturns: the saved or forgotten device, or the FC's identity and each command's answer.",
            json!({"destructiveHint": false, "idempotentHint": true, "openWorldHint": false, "readOnlyHint": false, "title": "Edit gear data"}),
        ),
        tool::<GearApplyArgs>(
            "quadcam_gear_apply",
            "Write to a device, a sim or the radio firmware: each action needs the digest from its plan and confirm=true, and with the app running the person must also click Apply in the app. No apply actions exist in this version; every call refuses.\n\nBest for: nothing yet.\nReturns: the refusal.",
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
        "quadcam_gear_apply" => gear_apply(a),
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
        other => Err(anyhow!(
            "unknown action {other:?}; use status, devices, fc_identify, board_notes, usb_timers, osd, card, card_preview, switch_map or radio"
        )),
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
        .context("action is required: device_save or device_forget")?;
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
        other => Err(anyhow!(
            "unknown action {other:?}; use device_save, device_forget or fc_read"
        )),
    }
}

fn gear_apply(a: &Value) -> Result<(Vec<Value>, Value)> {
    let x: GearApplyArgs = args(a)?;
    Err(anyhow!(
        "Refused: {} is not an apply action in this version of QuadCam; there are none yet.",
        x.action.as_deref().unwrap_or("(none)")
    ))
}
