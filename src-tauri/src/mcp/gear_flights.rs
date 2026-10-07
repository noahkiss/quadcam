//! The flights, packs, crashes, session report and "Pack up" actions of the Gear tools
//! (WP12). `mcp/gear.rs` holds their arguments and sends these actions here.

use super::gear::{GearArgs, GearEditArgs};
use super::render::text;
use super::server::Backend;
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

/// The read actions this file answers.
pub const READS: &[&str] = &["flights", "packs", "session_report", "preflight", "crashes"];
/// The edit actions this file answers.
pub const EDITS: &[&str] = &[
    "flight_set",
    "flight_folders",
    "pack_save",
    "pack_delete",
    "pack_type_save",
    "pack_type_delete",
    "pack_notes",
    "crash_save",
    "crash_delete",
];

fn num(v: &Value, digits: usize) -> String {
    v.as_f64()
        .map(|x| format!("{x:.digits$}"))
        .unwrap_or_else(|| "?".into())
}

fn mmss(v: &Value) -> String {
    let s = v.as_f64().unwrap_or(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// One flight in a line.
fn flight_line(r: &Value) -> String {
    let f = &r["flight"];
    let mut parts = vec![
        f["id"].as_str().unwrap_or("?").to_string(),
        f["start"].as_str().unwrap_or("?").replace('T', " "),
        mmss(&f["secs"]),
        r["aircraft"]
            .as_str()
            .or(f["model"].as_str())
            .unwrap_or("unknown aircraft")
            .to_string(),
    ];
    match (r["pack"].as_str(), r["suggested_pack"].as_str()) {
        (Some(p), _) => parts.push(format!("pack {p}")),
        (None, Some(s)) => parts.push(format!("no pack (suggest {s})")),
        _ => parts.push("no pack".into()),
    }
    if let Some(p) = r["place"].as_str() {
        parts.push(p.to_string());
    }
    let h = &f["hover"];
    if !h["all"].is_null() {
        parts.push(format!(
            "hover {} % (early {}, late {})",
            num(&h["all"], 0),
            num(&h["early"], 0),
            num(&h["late"], 0)
        ));
    }
    if !f["sag_min_v"].is_null() {
        parts.push(format!(
            "sag p5 {} V, min {} V",
            num(&f["sag_p5_v"], 2),
            num(&f["sag_min_v"], 2)
        ));
    }
    if !f["resting_v"].is_null() {
        parts.push(format!("resting {} V", num(&f["resting_v"], 2)));
    }
    if !f["mah"].is_null() {
        parts.push(format!("{} mAh", num(&f["mah"], 0)));
    }
    if let (Some(t), Some(at)) = (r["threshold_mah"].as_f64(), r["crossed_at_s"].as_f64()) {
        parts.push(format!("{t:.0} mAh at {}", mmss(&json!(at))));
    }
    if !f["worst_lq"].is_null() || !f["worst_rssi_db"].is_null() {
        parts.push(format!(
            "worst LQ {} RSSI {} dB",
            num(&f["worst_lq"], 0),
            num(&f["worst_rssi_db"], 0)
        ));
    }
    let drops = f["dropouts"].as_array().map_or(0, Vec::len);
    if drops > 0 {
        parts.push(format!("dropouts {drops}"));
    }
    parts.join(" | ")
}

fn packs_text(v: &Value) -> String {
    let mut out = Vec::new();
    for p in v["packs"].as_array().into_iter().flatten() {
        let mut l = format!(
            "{} | {} | {} | {} cycles",
            p["pack"]["label"].as_str().unwrap_or("?"),
            p["pack"]["pack_type"].as_str().unwrap_or("no type"),
            p["state"].as_str().unwrap_or("?"),
            p["cycles"]
        );
        if !p["median_resting_v"].is_null() {
            l.push_str(&format!(" | resting {} V", num(&p["median_resting_v"], 2)));
        }
        if p["pack"]["retired"] == true {
            l.push_str(" | retired");
        }
        if let Some(w) = p["weak"].as_str() {
            l.push_str(&format!(" | weak: {w}"));
        }
        out.push(l);
    }
    if out.is_empty() {
        out.push("No packs saved.".into());
    }
    for t in v["types"].as_array().into_iter().flatten() {
        let pt = &t["pack_type"];
        let mut l = format!(
            "Type {} | {} {}S | {} packs, {} flights",
            pt["name"].as_str().unwrap_or("?"),
            pt["chemistry"].as_str().unwrap_or("?"),
            pt["cells"],
            t["packs"],
            t["flights"]
        );
        if !t["full_total_v"].is_null() {
            l.push_str(&format!(" | full {} V", num(&t["full_total_v"], 2)));
        }
        if !t["storage_total_v"].is_null() {
            l.push_str(&format!(" | storage {} V", num(&t["storage_total_v"], 2)));
        }
        if !pt["warn_mah"].is_null() {
            l.push_str(&format!(" | warning {} mAh", num(&pt["warn_mah"], 0)));
        }
        if !t["suggested_warn_mah"].is_null() {
            l.push_str(&format!(
                " | suggested {} mAh for {} V/cell resting",
                num(&t["suggested_warn_mah"], 0),
                num(&v["target_v"], 2)
            ));
        }
        out.push(l);
    }
    if let Some(n) = v["notes"].as_str().filter(|n| !n.is_empty()) {
        out.push(format!("Notes: {n}"));
    }
    out.join("\n")
}

fn crash_line(c: &Value) -> String {
    let mut l = format!(
        "{} | {} | {}",
        c["id"].as_str().unwrap_or("?"),
        c["day"].as_str().unwrap_or("?"),
        c["aircraft"].as_str().unwrap_or("no aircraft")
    );
    if let Some(clip) = c["clip"].as_str() {
        l.push_str(&format!(" | clip {clip}"));
        if let Some(t) = c["time_s"].as_f64() {
            l.push_str(&format!(" at {t:.1} s"));
        }
    }
    if let Some(b) = c["broke"].as_str().filter(|b| !b.is_empty()) {
        l.push_str(&format!(" | broke: {b}"));
    }
    let parts: Vec<&str> = c["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !parts.is_empty() {
        l.push_str(&format!(" | parts: {}", parts.join(", ")));
    }
    if c["repaired"] == true {
        l.push_str(" | repaired");
    }
    l
}

/// Runs a read action; None when it is not one of these.
pub(super) fn read<B: Backend>(
    backend: &mut B,
    action: &str,
    x: &GearArgs,
) -> Option<Result<(Vec<Value>, Value)>> {
    if !READS.contains(&action) {
        return None;
    }
    Some((|| match action {
        "flights" => {
            let v = backend.call(
                "gear_flights",
                json!({"day": x.day, "aircraft": x.aircraft, "pack": x.pack, "place": x.place, "logs": x.logs}),
            )?;
            let list = v["flights"].as_array().cloned().unwrap_or_default();
            let line = if list.is_empty() {
                format!(
                    "No flights in the radio logs. Read: {}. Add a log folder with quadcam_gear_edit flight_folders.",
                    v["sources"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            } else {
                list.iter().map(flight_line).collect::<Vec<_>>().join("\n")
            };
            Ok((vec![text(line)], v))
        }
        "packs" => {
            let v = backend.call("gear_packs", json!({"target_v": x.target_v}))?;
            Ok((vec![text(packs_text(&v))], v))
        }
        "session_report" => {
            let v = backend.call("gear_session_report", json!({"day": x.day}))?;
            Ok((
                vec![text(v["markdown"].as_str().unwrap_or("").to_string())],
                v,
            ))
        }
        "preflight" => {
            let v = backend.call("gear_preflight", Value::Null)?;
            let line = v["rows"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|r| {
                    format!(
                        "{} {}: {}",
                        r["state"].as_str().unwrap_or("?"),
                        r["label"].as_str().unwrap_or("?"),
                        r["detail"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok((vec![text(line)], v))
        }
        "crashes" => {
            let list = backend.call(
                "gear_crashes",
                json!({"aircraft": x.aircraft, "clip": x.clip}),
            )?;
            let arr = list.as_array().cloned().unwrap_or_default();
            let line = if arr.is_empty() {
                "No crashes logged.".to_string()
            } else {
                arr.iter().map(crash_line).collect::<Vec<_>>().join("\n")
            };
            Ok((vec![text(line)], json!({"crashes": list})))
        }
        _ => Err(anyhow!("not a flights action")),
    })())
}

/// Runs an edit action; None when it is not one of these.
pub(super) fn edit<B: Backend>(
    backend: &mut B,
    action: &str,
    x: &GearEditArgs,
) -> Option<Result<(Vec<Value>, Value)>> {
    if !EDITS.contains(&action) {
        return None;
    }
    let name = || {
        x.name
            .clone()
            .context("name is required: the pack label or pack type name")
    };
    Some((|| match action {
        "flight_set" => {
            let flight = x
                .flight
                .clone()
                .context("flight is required: a flight id from quadcam_gear flights")?;
            let r = backend.call(
                "gear_flight_set",
                json!({"flight": flight, "pack": x.pack, "place": x.place}),
            )?;
            Ok((vec![text(format!("Saved. {}", flight_line(&r)))], r))
        }
        "flight_folders" => {
            let v = backend.call(
                "gear_flight_folders",
                json!({"add": x.add_folder, "remove": x.remove_folder}),
            )?;
            let list: Vec<&str> = v
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            Ok((
                vec![text(if list.is_empty() {
                    "No log folders added.".to_string()
                } else {
                    format!("Log folders: {}", list.join(", "))
                })],
                json!({"folders": v}),
            ))
        }
        "pack_save" => {
            let label = name()?;
            let view = backend.call("gear_packs", Value::Null)?;
            let mut p = view["packs"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| v["pack"].clone())
                .find(|p| p["label"] == label.as_str())
                .unwrap_or_else(|| json!({"label": label}));
            for (k, v) in [
                ("pack_type", json!(x.pack_type)),
                ("received", json!(x.received)),
                ("retired", json!(x.retired)),
                ("note", json!(x.note)),
            ] {
                if !v.is_null() {
                    p[k] = v;
                }
            }
            let r = backend.call("gear_pack_save", json!({"pack": p, "charged": x.charged}))?;
            Ok((
                vec![text(format!(
                    "Saved pack {}.",
                    r["label"].as_str().unwrap_or("?")
                ))],
                r,
            ))
        }
        "pack_delete" => {
            let r = backend.call("gear_pack_delete", json!({"name": name()?}))?;
            Ok((
                vec![text(format!(
                    "Deleted pack {}.",
                    r["label"].as_str().unwrap_or("?")
                ))],
                r,
            ))
        }
        "pack_type_save" => {
            let n = name()?;
            let view = backend.call("gear_packs", Value::Null)?;
            let mut t = view["types"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| v["pack_type"].clone())
                .find(|t| t["name"] == n.as_str())
                .unwrap_or_else(|| json!({"name": n, "cells": 1}));
            for (k, v) in [
                (
                    "chemistry",
                    json!(x.chemistry.as_deref().map(str::to_lowercase)),
                ),
                ("cells", json!(x.cells)),
                ("capacity_mah", json!(x.capacity_mah)),
                ("connector", json!(x.connector)),
                ("full_v", json!(x.full_v)),
                ("storage_v", json!(x.storage_v)),
                ("charge_a", json!(x.charge_a)),
                ("warn_mah", json!(x.warn_mah)),
                ("note", json!(x.note)),
            ] {
                if !v.is_null() {
                    t[k] = v;
                }
            }
            let r = backend.call("gear_pack_type_save", t)?;
            Ok((
                vec![text(format!(
                    "Saved pack type {}.",
                    r["name"].as_str().unwrap_or("?")
                ))],
                r,
            ))
        }
        "pack_type_delete" => {
            let r = backend.call("gear_pack_type_delete", json!({"name": name()?}))?;
            Ok((
                vec![text(format!(
                    "Deleted pack type {}.",
                    r["name"].as_str().unwrap_or("?")
                ))],
                r,
            ))
        }
        "pack_notes" => {
            let t = x.text.clone().context("text is required: the notes")?;
            let r = backend.call("gear_pack_notes", json!({"text": t}))?;
            Ok((
                vec![text("Saved the charging notes.")],
                json!({"notes": r}),
            ))
        }
        "crash_save" => {
            let r = backend.call(
                "gear_crash_save",
                json!({
                    "id": x.id, "clip": x.clip, "time_s": x.time_s, "aircraft": x.aircraft,
                    "day": x.day, "broke": x.broke, "parts": x.parts, "note": x.note,
                    "repaired": x.repaired,
                }),
            )?;
            Ok((vec![text(format!("Saved. {}", crash_line(&r)))], r))
        }
        "crash_delete" => {
            let id = x.id.clone().context("id is required: a crash id")?;
            let r = backend.call("gear_crash_delete", json!({"id": id}))?;
            Ok((vec![text(format!("Deleted. {}", crash_line(&r)))], r))
        }
        _ => Err(anyhow!("not a flights action")),
    })())
}
