//! The tools' text answers and compact records, made from the core's JSON.

use serde_json::{json, Value};

pub(super) fn text(t: impl Into<String>) -> Value {
    json!({"type": "text", "text": t.into()})
}

/// One library clip as the tools return it.
pub(super) fn lib_view(c: &Value) -> Value {
    json!({
        "id": c["id"], "name": c["name"], "file": c["file"], "date": c["date"],
        "time": c["time"], "duration_s": c["duration"], "rating": c["rating"],
        "flag": c["flag"], "place": c["place"], "location": c["location"], "aircraft": c["aircraft"],
        "note": c["note"], "keywords": c["keywords"], "author": c["author"],
        "moments": c["moments"].as_array().map(|m| m.iter().map(|x| json!({"kind": x["kind"], "start": x["start"], "score": x["score"], "source": x["source"]})).collect::<Vec<_>>()),
        "keep": c["keep"], "cuts": c["cuts"], "unsaved_cuts": c["pending_cuts"],
        "in_photos": c["in_photos"], "last_import": c["last_import"], "dvr": c["dvr"],
        "sun": c["sun"], "weather": c["weather"],
    })
}

pub(super) fn lib_line(c: &Value) -> String {
    format!(
        "{} {} {} {:?} {:.0}s {}★{}{}{}{}{}",
        c["id"].as_str().unwrap_or("?"),
        c["date"].as_str().unwrap_or("?"),
        c["time"].as_str().unwrap_or(""),
        c["name"].as_str().unwrap_or(""),
        c["duration_s"].as_f64().unwrap_or(0.0),
        c["rating"],
        match c["flag"].as_str() {
            Some("pick") => " pick",
            Some("reject") => " rejected",
            _ => "",
        },
        c["aircraft"]
            .as_str()
            .map(|a| format!(" {a}"))
            .unwrap_or_default(),
        c["moments"]
            .as_array()
            .filter(|m| !m.is_empty())
            .map(|m| format!(" {} moments", m.len()))
            .unwrap_or_default(),
        c["sun"]
            .as_str()
            .map(|s| format!(" {}", s.replace('_', " ")))
            .unwrap_or_default(),
        weather_phrase(&c["weather"])
            .map(|w| format!(" {w}"))
            .unwrap_or_default()
    )
}

/// A clip's weather as one phrase: `18.2 °C, wind 12 km/h from 270° (gusts 20), rain,
/// cloud 75%, precipitation 1.2 mm, humidity 88%, visibility 9.0 km`. Each sky value only
/// when the record has it; precipitation only above 0.
pub(super) fn weather_phrase(w: &Value) -> Option<String> {
    let t = w["temperature_c"].as_f64()?;
    let wind = w["wind_kmh"].as_f64()?;
    let mut s = format!("{t:.1} °C, wind {wind:.0} km/h");
    if let Some(d) = w["wind_from_deg"].as_f64() {
        s += &format!(" from {d:.0}°");
    }
    if let Some(g) = w["gusts_kmh"].as_f64() {
        s += &format!(" (gusts {g:.0})");
    }
    if let Some(k) = w["sky"].as_str() {
        s += &format!(", {}", k.replace('_', " "));
    }
    if let Some(c) = w["cloud_cover_pct"].as_f64() {
        s += &format!(", cloud {c:.0}%");
    }
    if let Some(p) = w["precipitation_mm"].as_f64().filter(|p| *p > 0.0) {
        s += &format!(", precipitation {p:.1} mm");
    }
    if let Some(h) = w["humidity_pct"].as_f64() {
        s += &format!(", humidity {h:.0}%");
    }
    if let Some(v) = w["visibility_m"].as_f64() {
        s += &format!(", visibility {:.1} km", v / 1000.0);
    }
    Some(s)
}

/// `library_weather`'s report: one line per clip.
pub(super) fn weather_text(r: &Value) -> String {
    let clips = r["clips"].as_array().cloned().unwrap_or_default();
    if clips.is_empty() {
        return "Every clip has its weather already.".into();
    }
    clips
        .iter()
        .map(|c| {
            let what = if let Some(e) = c["error"].as_str() {
                format!("failed: {e}")
            } else if let Some(why) = c["skipped"].as_str() {
                format!("skipped: {why}")
            } else {
                let w = weather_phrase(&c["weather"]).unwrap_or_default();
                if c["kept"].as_bool() == Some(true) {
                    format!("{w} (kept)")
                } else {
                    w
                }
            };
            format!("{} {}", c["id"].as_str().unwrap_or("?"), what)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `library_overlay`'s report: what was written and from where.
pub(super) fn overlay_text(r: &Value) -> String {
    let list = |v: &Value| {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut lines = vec![format!(
        "{} cues from the {} ({}), {:.0}-{:.0} s, showing {}.",
        r["cues"],
        if r["source"] == "log" {
            "radio log"
        } else {
            "sidecar"
        },
        r["from"].as_str().unwrap_or(""),
        r["first_s"].as_f64().unwrap_or(0.0),
        r["last_s"].as_f64().unwrap_or(0.0),
        list(&r["fields"])
    )];
    lines.push(format!("Subtitles: {}", r["srt"].as_str().unwrap_or("")));
    if let Some(v) = r["video"].as_str() {
        lines.push(format!("Burned-in copy: {v}"));
    }
    if r["unknown_keys"].as_array().is_some_and(|a| !a.is_empty()) {
        lines.push(format!(
            "Sidecar keys not shown: {}",
            list(&r["unknown_keys"])
        ));
    }
    for w in r["warnings"].as_array().into_iter().flatten() {
        lines.push(format!("Warning: {}", w.as_str().unwrap_or("")));
    }
    lines.join("\n")
}

/// `library_analyze_video`'s report: one line per clip, or why nothing ran.
pub(super) fn video_text(r: &Value) -> String {
    let mut lines: Vec<String> = Vec::new();
    if let Some(why) = r["unavailable"].as_str() {
        lines.push(format!("Video analysis is unavailable: {why}"));
    }
    let clips = r["clips"].as_array().cloned().unwrap_or_default();
    if clips.is_empty() && lines.is_empty() {
        lines.push(
            "Nothing to analyse: every clip was analysed already (again=true analyses them again)."
                .into(),
        );
    }
    for c in &clips {
        lines.push(match c["error"].as_str() {
            Some(e) => format!(
                "{} {}: failed: {e}",
                c["id"].as_str().unwrap_or(""),
                c["path"].as_str().unwrap_or("")
            ),
            None => format!(
                "{} {}: {} highlights, key frame {}{}",
                c["id"].as_str().unwrap_or(""),
                c["path"].as_str().unwrap_or(""),
                c["highlights"],
                c["key_frame"]
                    .as_f64()
                    .map(|k| format!("{k:.1} s"))
                    .unwrap_or_else(|| "none".into()),
                if c["cached"] == true { " (cached)" } else { "" }
            ),
        });
    }
    lines.join("\n")
}

pub(super) fn places_text(places: &Value) -> String {
    let arr = places.as_array().cloned().unwrap_or_default();
    if arr.is_empty() {
        return "No saved places. Find one with action search, then save it.".into();
    }
    arr.iter()
        .map(|p| {
            format!(
                "{}: {}, {}",
                p["name"].as_str().unwrap_or("?"),
                p["lat"],
                p["lon"]
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A source kind as people say it: `analog`, `DJI`.
pub(super) fn source_label(kind: &Value) -> String {
    serde_json::from_value::<crate::sources::SourceKind>(kind.clone())
        .map(|k| k.label().to_string())
        .unwrap_or_else(|_| kind.as_str().unwrap_or("?").to_string())
}

/// A date source as people say it: `radio log`, `clip clock`.
pub(super) fn date_source_label(source: &Value) -> String {
    serde_json::from_value::<crate::pipeline::DateSource>(source.clone())
        .map(|s| s.label().to_string())
        .unwrap_or_else(|_| source.as_str().unwrap_or("?").to_string())
}

/// The status line's note on the app's own update: empty unless a newer QuadCam is found.
pub(super) fn update_note(u: &Value) -> String {
    let v = u["version"].as_str().unwrap_or("?");
    match u["phase"].as_str() {
        Some("available") => {
            format!(" Update: QuadCam {v} is available (the person installs it in the app).")
        }
        Some("downloading") => format!(" Update: QuadCam {v} is downloading."),
        Some("ready") => format!(" Update: QuadCam {v} installs when the app restarts."),
        _ => String::new(),
    }
}

pub(super) fn names(v: &[&Value]) -> String {
    if v.is_empty() {
        return "none".into();
    }
    v.iter()
        .map(|x| {
            let source = if x["source"].is_null() {
                String::new()
            } else {
                format!(", {}", source_label(&x["source"]))
            };
            format!(
                "{} ({}{source})",
                x["info"]["volume_name"].as_str().unwrap_or("?"),
                x["mount"].as_str().unwrap_or("?")
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The answer of `suggest_on_device` in a line: how many clips got a name, or why none.
pub(super) fn on_device_text(r: &Value) -> String {
    if r["available"] != true {
        return format!(
            "On-device suggestions skipped: {}. ",
            r["reason"].as_str().unwrap_or("unavailable")
        );
    }
    let clips = r["clips"].as_array().cloned().unwrap_or_default();
    let named = clips.iter().filter(|c| !c["suggestion"].is_null()).count();
    let filled = clips
        .iter()
        .filter(|c| c["filled"].as_array().is_some_and(|f| !f.is_empty()))
        .count();
    format!(
        "On-device model suggested names for {named} of {} clips in {} s; it filled empty fields on {filled} (values the person or an agent set stay). ",
        clips.len(),
        r["seconds"]
    )
}

pub(super) fn photos_line(p: &Value) -> String {
    match (p.get("Ok"), p.get("Err")) {
        (Some(r), _) => {
            let added = r["added"].as_array().map(Vec::len).unwrap_or(0);
            let failed = r["failed"].as_array().cloned().unwrap_or_default();
            let where_ = r["album"]
                .as_str()
                .map(|a| format!("album \"{a}\""))
                .unwrap_or_else(|| "the library".into());
            if failed.is_empty() {
                format!("{added} added to {where_}.")
            } else {
                format!(
                    "{added} added to {where_}; {} failed: {}",
                    failed.len(),
                    failed[0][1].as_str().unwrap_or("")
                )
            }
        }
        (_, Some(e)) => format!("failed: {}", e.as_str().unwrap_or("?")),
        _ => "not requested".into(),
    }
}

/// What "Delete clips after import" did: a count, then one line per clip.
pub(super) fn deletion_lines(d: &[Value]) -> String {
    let deleted = d.iter().filter(|x| x["state"] == "deleted").count();
    let mut out = format!(
        "Delete clips after import: {deleted} deleted, {} kept.",
        d.len() - deleted
    );
    for x in d {
        let path = x["path"].as_str().unwrap_or("?");
        if x["state"] == "deleted" {
            out.push_str(&format!("\nClip {} deleted: {path}", x["id"]));
        } else {
            out.push_str(&format!(
                "\nClip {} kept: {path} ({})",
                x["id"],
                x["reason"].as_str().unwrap_or("")
            ));
        }
    }
    out
}

/// Flattens session clips + plans + results into one compact record per clip.
pub fn clip_views(session: &Value, ids: Option<&[u64]>) -> Vec<Value> {
    let plans = session["plans"].as_array().cloned().unwrap_or_default();
    let results = session["results"].as_array().cloned().unwrap_or_default();
    let in_photos = session["in_photos"].as_array().cloned().unwrap_or_default();
    session["clips"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| ids.is_none_or(|ids| c["id"].as_u64().is_some_and(|id| ids.contains(&id))))
        .map(|c| {
            let id = &c["id"];
            let p = plans
                .iter()
                .find(|p| &p["id"] == id)
                .cloned()
                .unwrap_or(Value::Null);
            let r = results
                .iter()
                .rev()
                .find(|r| &r["id"] == id)
                .cloned()
                .unwrap_or(Value::Null);
            let duration = c["duration"].as_f64().unwrap_or(0.0);
            // Log moments the offset moved outside the clip are left out.
            let mut moments: Vec<Value> = p["moments"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(c["signal"]["dead_air"].as_array().into_iter().flatten())
                .filter(|m| {
                    m["end"].as_f64().unwrap_or(0.0) > 0.0
                        && m["start"].as_f64().unwrap_or(0.0) < duration
                })
                .cloned()
                .collect();
            moments.sort_by(|a, b| {
                a["start"]
                    .as_f64()
                    .unwrap_or(0.0)
                    .total_cmp(&b["start"].as_f64().unwrap_or(0.0))
            });
            // A recording the DVR split: its files, by name, and whether they import as one.
            let all = session["clips"].as_array().cloned().unwrap_or_default();
            let files: Vec<Value> = std::iter::once(c["name"].clone())
                .chain(
                    c["join"]["parts"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|pid| all.iter().find(|x| &x["id"] == pid))
                        .map(|x| x["name"].clone()),
                )
                .collect();
            json!({
                "id": id,
                "name": c["name"],
                "joined": if c["join"].is_null() { Value::Null } else { c["join"]["on"].clone() },
                "files": if c["join"].is_null() { Value::Null } else { json!(files) },
                "part_of": c["part_of"],
                "source": c["kind"],
                "path": c["rel"],
                "duration_s": (c["duration"].as_f64().unwrap_or(0.0) * 10.0).round() / 10.0,
                "size": c["size"],
                "status": c["status"],
                "detail": c["detail"],
                "frames": c["probe"]["video_packets"],
                "thumbnail": c["thumb"],
                "date": p["date"],
                "time": p["time"],
                "date_source": p["source"],
                "log_match": p["badge"],
                "log_match_reason": p["match_reason"],
                "flights": p["segments"],
                "short_name": p["name"],
                "note": p["note"],
                "skip": p["skip"],
                "agent_suggested": p["suggested"],
                "reason": p["reason"],
                "on_device": p["ondevice"],
                "outcome": r["outcome"],
                "output": r["output"],
                "error": r["error"],
                "in_photos": in_photos.contains(id),
                "moments": moments,
                "keep": c["signal"]["keep"],
                "log_interval_s": p["log_interval_s"],
                "log_offset_s": p["log_offset_s"],
                "cuts": p["cuts"],
                "cut_results": r["cuts"],
                "metadata": p["meta"],
                "log_model": p["log_model"],
                "flight": p["flight"],
            })
        })
        .collect()
}

pub(super) fn table(view: &[Value]) -> String {
    view.iter()
        .map(|c| {
            let mut marks = Vec::new();
            if let Some(h) = c["part_of"].as_u64() {
                marks.push(format!("part of #{h}, imports with it"));
            }
            if let Some(files) = c["files"].as_array() {
                marks.push(format!(
                    "{} files of one recording, {}",
                    files.len(),
                    if c["joined"] == true {
                        "joined (joined=false keeps them apart)"
                    } else {
                        "kept apart (joined=true joins them)"
                    }
                ));
            }
            if c["skip"] == true {
                marks.push("skip".to_string());
            }
            if let Some(o) = c["outcome"].as_str() {
                marks.push(o.to_string());
            }
            let n = |k: &str| c[k].as_array().map(Vec::len).unwrap_or(0);
            if n("moments") > 0 {
                marks.push(format!("{} moments", n("moments")));
            }
            if n("keep") > 0 {
                marks.push(format!("{} keep ranges", n("keep")));
            }
            if n("cuts") > 0 {
                marks.push(format!("{} cuts", n("cuts")));
            }
            let sug = &c["agent_suggested"];
            if let Some(l) = c["metadata"]["location"].as_object() {
                marks.push(match l.get("name").and_then(Value::as_str) {
                    Some(n) => format!("at {n}"),
                    None => "located".into(),
                });
            }
            if ["date", "name", "note", "skip", "cuts", "meta"]
                .iter()
                .any(|k| sug[k] == true)
            {
                marks.push("agent-suggested".into());
            }
            if let Some(n) = c["on_device"]["name"].as_str() {
                marks.push(format!("on-device suggestion {n:?}"));
            }
            format!(
                "#{} {} ({}) {}s {} date={} ({}, {}) name={:?} note={:?}{}",
                c["id"],
                c["name"].as_str().unwrap_or(""),
                source_label(&c["source"]),
                c["duration_s"],
                c["status"].as_str().unwrap_or(""),
                c["date"].as_str().unwrap_or("?"),
                date_source_label(&c["date_source"]),
                c["log_match"].as_str().unwrap_or("?"),
                c["short_name"].as_str().unwrap_or(""),
                c["note"].as_str().unwrap_or(""),
                if marks.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", marks.join(", "))
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
