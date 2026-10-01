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
        "moments": c["moments"].as_array().map(|m| m.iter().map(|x| json!({"kind": x["kind"], "start": x["start"], "score": x["score"]})).collect::<Vec<_>>()),
        "keep": c["keep"], "cuts": c["cuts"], "unsaved_cuts": c["pending_cuts"],
        "in_photos": c["in_photos"], "last_import": c["last_import"], "dvr": c["dvr"],
    })
}

pub(super) fn lib_line(c: &Value) -> String {
    format!(
        "{} {} {} {:?} {:.0}s {}★{}{}{}",
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
            .unwrap_or_default()
    )
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

pub(super) fn names(v: &[&Value]) -> String {
    if v.is_empty() {
        return "none".into();
    }
    v.iter()
        .map(|x| {
            format!(
                "{} ({})",
                x["info"]["volume_name"].as_str().unwrap_or("?"),
                x["mount"].as_str().unwrap_or("?")
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
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
            json!({
                "id": id,
                "name": c["name"],
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
                "packs": p["segments"],
                "short_name": p["name"],
                "note": p["note"],
                "skip": p["skip"],
                "agent_suggested": p["suggested"],
                "reason": p["reason"],
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
            format!(
                "#{} {} {}s {} date={} ({}, {}) name={:?} note={:?}{}",
                c["id"],
                c["name"].as_str().unwrap_or(""),
                c["duration_s"],
                c["status"].as_str().unwrap_or(""),
                c["date"].as_str().unwrap_or("?"),
                c["date_source"].as_str().unwrap_or("?"),
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
