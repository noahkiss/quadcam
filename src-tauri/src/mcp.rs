//! MCP server on stdio (`quadcam-cli mcp`). A thin layer over the core: when the quadcam app
//! is running it drives the app's session through the control socket, so the person sees
//! every change live; otherwise it runs a headless core on the shared session file.
//! Only MCP messages go to stdout; logs go to stderr.

use crate::control::{self, Client};
use crate::core::Core;
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

const SUPPORTED_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Where tool calls go: the running app or a local core.
pub trait Backend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value>;
    /// "app" when a person is watching in the GUI, "headless" otherwise.
    fn mode(&mut self) -> &'static str;
}

/// Uses the app when its socket answers, else a headless core. Every call opens a fresh
/// connection, so an app started mid-session takes over, one that quits falls back, and one
/// that quit and relaunched is reached on its new socket instead of a dead connection.
pub struct AutoBackend {
    socket: PathBuf,
    session_file: Option<PathBuf>,
    local: Option<Arc<Core>>,
}

impl AutoBackend {
    pub fn new(socket: PathBuf, session_file: Option<PathBuf>) -> Self {
        Self {
            socket,
            session_file,
            local: None,
        }
    }

    /// A new connection to the app, or None when no app answers the ping.
    fn connect(&self) -> Option<Client> {
        Client::connect(&self.socket).ok()
    }

    fn local(&mut self) -> Arc<Core> {
        self.local
            .get_or_insert_with(|| {
                Arc::new(Core::headless(
                    self.session_file.clone(),
                    Core::real_photos(),
                ))
            })
            .clone()
    }
}

impl Backend for AutoBackend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        // The ping in `connect` proves the app is alive right now. Never retry a call
        // headless that started in app mode: a format the person did not click must not run
        // elsewhere.
        if let Some(mut client) = self.connect() {
            return client.call(method, params);
        }
        self.local().dispatch(method, params)
    }

    fn mode(&mut self) -> &'static str {
        if self.connect().is_some() {
            "app"
        } else {
            "headless"
        }
    }
}

/// A core in this process, with no app. Tests use it with a temp cache and a mock Photos.
pub struct LocalBackend(pub Arc<Core>);

impl Backend for LocalBackend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.0.dispatch(method, params)
    }
    fn mode(&mut self) -> &'static str {
        "headless"
    }
}

/// Reads JSON-RPC lines from stdin and answers on stdout until stdin closes.
pub fn serve_stdio(session_file: Option<PathBuf>) -> Result<()> {
    let mut server = Server::new(AutoBackend::new(control::socket_path(), session_file));
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => server.handle(&msg),
            Err(e) => Some(
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}}),
            ),
        };
        if let Some(r) = reply {
            writeln!(out, "{r}")?;
            out.flush()?;
        }
    }
    Ok(())
}

pub struct Server<B: Backend> {
    pub backend: B,
}

fn text(t: impl Into<String>) -> Value {
    json!({"type": "text", "text": t.into()})
}

impl<B: Backend> Server<B> {
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    /// Handles one message. Notifications get no reply.
    pub fn handle(&mut self, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned()?;
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => {
                let asked = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let version = if SUPPORTED_VERSIONS.contains(&asked) {
                    asked
                } else {
                    SUPPORTED_VERSIONS[1]
                };
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "quadcam", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": INSTRUCTIONS,
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools()})),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                Ok(self.call_tool(name, args))
            }
            _ => Err(json!({"code": -32601, "message": format!("method not found: {method}")})),
        };
        Some(match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
        })
    }

    /// Runs a tool. Operational failures come back as `isError` results the agent can read.
    pub fn call_tool(&mut self, name: &str, args: Value) -> Value {
        match self.run_tool(name, &args) {
            Ok((content, structured)) => {
                json!({"content": content, "structuredContent": structured, "isError": false})
            }
            Err(e) => json!({"content": [text(format!("{e:#}"))], "isError": true}),
        }
    }

    fn run_tool(&mut self, name: &str, a: &Value) -> Result<(Vec<Value>, Value)> {
        let s = |k: &str| a.get(k).and_then(Value::as_str).map(str::to_string);
        let ids = a.get("ids").cloned().filter(|v| !v.is_null());
        match name {
            "quadcam_status" => {
                let mode = self.backend.mode();
                let status = self.backend.call("status", Value::Null)?;
                let volumes = self.backend.call("volumes", Value::Null)?;
                let cards: Vec<&Value> = volumes
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|v| v["is_card"] == true)
                    .collect();
                let radios: Vec<&Value> = volumes
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|v| v["is_radio"] == true)
                    .collect();
                let sess = &status["session"];
                let line = format!(
                    "Mode: {mode}{}. Cards: {}. Radios: {}. Session: {}.",
                    if mode == "app" {
                        " (the person sees changes live in quadcam)"
                    } else {
                        ""
                    },
                    names(&cards),
                    names(&radios),
                    if sess.is_null() {
                        "none; call quadcam_load_clips".to_string()
                    } else {
                        format!(
                            "{} clips from {}, {} imported",
                            sess["clips"],
                            sess["source"].as_str().unwrap_or("?"),
                            sess["imported"]
                        )
                    }
                );
                Ok((
                    vec![text(line)],
                    json!({"mode": mode, "status": status, "cards": cards, "radios": radios}),
                ))
            }
            "quadcam_library" => {
                let mut filter = json!({});
                for k in ["query", "group", "day", "place", "aircraft"] {
                    if let Some(v) = s(k) {
                        filter[k] = json!(v);
                    }
                }
                if let Some(r) = a.get("min_rating").filter(|v| !v.is_null()) {
                    filter["min_rating"] = r.clone();
                }
                let limit = a.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
                let lib = self.backend.call("library", filter)?;
                let all = lib["clips"].as_array().cloned().unwrap_or_default();
                let clips: Vec<Value> = all
                    .iter()
                    .take(limit)
                    .map(|c| {
                        json!({
                            "id": c["id"], "name": c["name"], "file": c["file"], "date": c["date"],
                            "time": c["time"], "duration_s": c["duration"], "rating": c["rating"],
                            "flag": c["flag"], "place": c["place"], "aircraft": c["aircraft"],
                            "note": c["note"], "keywords": c["keywords"],
                            "moments": c["moments"].as_array().map(|m| m.iter().map(|x| json!({"kind": x["kind"], "start": x["start"], "score": x["score"]})).collect::<Vec<_>>()),
                            "keep": c["keep"], "cuts": c["cuts"], "unsaved_cuts": c["pending_cuts"],
                            "in_photos": c["in_photos"], "last_import": c["last_import"], "dvr": c["dvr"],
                        })
                    })
                    .collect();
                let lines: Vec<String> = clips
                    .iter()
                    .map(|c| {
                        format!(
                            "{} {} {:?} {:.0}s {}★{}{}",
                            c["id"].as_str().unwrap_or("?"),
                            c["date"].as_str().unwrap_or("?"),
                            c["name"].as_str().unwrap_or(""),
                            c["duration_s"].as_f64().unwrap_or(0.0),
                            c["rating"],
                            match c["flag"].as_str() {
                                Some("pick") => " pick",
                                Some("reject") => " rejected",
                                _ => "",
                            },
                            c["moments"]
                                .as_array()
                                .filter(|m| !m.is_empty())
                                .map(|m| format!(" {} moments", m.len()))
                                .unwrap_or_default()
                        )
                    })
                    .collect();
                Ok((
                    vec![text(format!(
                        "Library {}: {} of {} clips{}.\n{}",
                        lib["root"].as_str().unwrap_or("?"),
                        clips.len(),
                        all.len(),
                        if lib["unindexed"].as_u64().unwrap_or(0) > 0 {
                            format!(
                                "; {} files in the folder are not indexed yet (the person can scan them in Settings > Library)",
                                lib["unindexed"]
                            )
                        } else {
                            String::new()
                        },
                        lines.join("\n")
                    ))],
                    json!({"root": lib["root"], "total": all.len(), "has_more": all.len() > clips.len(), "last_import": lib["last_import"], "totals": lib["totals"], "clips": clips}),
                ))
            }
            "quadcam_load_clips" => {
                let session = self.backend.call("load", json!({"source": s("source")}))?;
                let view = clip_views(&session, None);
                Ok((
                    vec![text(format!(
                        "Loaded {} clips from {}.\n{}",
                        view.len(),
                        session["source"].as_str().unwrap_or("?"),
                        table(&view)
                    ))],
                    json!({"clips": view}),
                ))
            }
            "quadcam_read_clips" => {
                let session = self.backend.call("session", Value::Null)?;
                if session.is_null() {
                    return Err(anyhow!("No clips loaded. Call quadcam_load_clips first."));
                }
                let want: Option<Vec<u64>> = ids
                    .as_ref()
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let view = clip_views(&session, want.as_deref());
                let mut content = vec![text(table(&view))];
                if a.get("thumbnails")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    let max = a
                        .get("max_thumbnails")
                        .and_then(Value::as_u64)
                        .unwrap_or(12) as usize;
                    for c in view.iter().take(max) {
                        let Some(path) = c["thumbnail"].as_str() else {
                            continue;
                        };
                        if let Ok(bytes) = std::fs::read(path) {
                            content.push(text(format!(
                                "Thumbnail of clip {} ({})",
                                c["id"],
                                c["name"].as_str().unwrap_or("")
                            )));
                            content.push(json!({"type": "image", "mimeType": "image/jpeg", "data": base64::engine::general_purpose::STANDARD.encode(bytes)}));
                        }
                    }
                }
                let extra = json!({
                    "source": session["source"], "card": session["card"], "log_dir": session["log_dir"], "log_day": session["log_day"],
                    "log_days": session["log_days"], "warnings": session["warnings"], "date_warnings": session["date_warnings"],
                    "output_dir": session["output_dir"],
                });
                Ok((content, json!({"clips": view, "session": extra})))
            }
            "quadcam_match_logs" => {
                let logs = if a.get("no_logs").and_then(Value::as_bool).unwrap_or(false) {
                    json!({"kind": "none"})
                } else if let Some(d) = s("log_dir") {
                    json!({"kind": "dir", "path": d})
                } else {
                    json!({"kind": "keep"})
                };
                let session = self
                    .backend
                    .call("dates", json!({"logs": logs, "day": s("day")}))?;
                let view = clip_views(&session, None);
                let warn = session["date_warnings"]
                    .as_array()
                    .map(|w| {
                        w.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                Ok((
                    vec![text(format!(
                        "Log day: {}. {warn}\n{}",
                        session["log_day"].as_str().unwrap_or("none"),
                        table(&view)
                    ))],
                    json!({"log_day": session["log_day"], "log_days": session["log_days"], "warnings": session["date_warnings"], "clips": view}),
                ))
            }
            "quadcam_suggest" => {
                let patches = a
                    .get("suggestions")
                    .cloned()
                    .context("suggestions is required")?;
                let session = self
                    .backend
                    .call("suggest", json!({"patches": patches, "editor": "agent"}))?;
                let view = clip_views(&session, None);
                Ok((
                    vec![text(format!(
                        "Suggestions saved and marked as agent-suggested{}. Read back final values with quadcam_read_clips before export.\n{}",
                        if self.backend.mode() == "app" { "; the person can edit them in quadcam" } else { "" },
                        table(&view)
                    ))],
                    json!({"clips": view}),
                ))
            }
            "quadcam_export" => {
                let mut opts = json!({});
                for k in ["output_dir", "format", "album"] {
                    if let Some(v) = s(k) {
                        opts[k] = json!(v);
                    }
                }
                for k in ["keep_originals", "add_time", "add_to_photos"] {
                    if let Some(v) = a.get(k).and_then(Value::as_bool) {
                        opts[k] = json!(v);
                    }
                }
                let out = self.backend.call("import", opts)?;
                let sum = &out["summary"];
                let mut line = format!(
                    "Imported {}, skipped {}, failed {} into {}. Format: {}.",
                    sum["imported"],
                    sum["skipped"],
                    sum["failed"],
                    sum["output_dir"].as_str().unwrap_or("?"),
                    if sum["format_ready"].get("Ok").is_some() {
                        "unlocked".to_string()
                    } else {
                        format!(
                            "locked ({})",
                            sum["format_ready"]["Err"].as_str().unwrap_or("?")
                        )
                    }
                );
                for r in sum["results"].as_array().into_iter().flatten() {
                    if r["outcome"] == "failed" {
                        line.push_str(&format!(
                            "\nClip {} failed: {}",
                            r["id"],
                            r["error"].as_str().unwrap_or("")
                        ));
                    }
                    for c in r["cuts"].as_array().into_iter().flatten() {
                        line.push_str(&format!(
                            "\nClip {} cut {}-{} s: {}",
                            r["id"],
                            c["start"],
                            c["end"],
                            if c["outcome"] == "verified" {
                                c["output"].as_str().unwrap_or("?").to_string()
                            } else {
                                format!("failed: {}", c["error"].as_str().unwrap_or(""))
                            }
                        ));
                    }
                }
                if let Some(p) = out.get("photos").filter(|p| !p.is_null()) {
                    line.push_str(&format!("\nPhotos: {}", photos_line(p)));
                }
                Ok((vec![text(line)], out))
            }
            "quadcam_verify" => {
                let reports = self.backend.call("verify", json!({"ids": ids}))?;
                let arr = reports.as_array().cloned().unwrap_or_default();
                let bad: Vec<String> = arr
                    .iter()
                    .filter(|r| r["ok"] != true)
                    .map(|r| format!("clip {}: {}", r["id"], r["error"].as_str().unwrap_or("")))
                    .collect();
                let line = if bad.is_empty() {
                    format!("{} outputs verified.", arr.len())
                } else {
                    format!("{} of {} failed: {}", bad.len(), arr.len(), bad.join("; "))
                };
                Ok((vec![text(line)], json!({"reports": reports})))
            }
            "quadcam_add_to_photos" => {
                let r = self
                    .backend
                    .call("photos", json!({"ids": ids, "album": s("album")}))?;
                Ok((vec![text(photos_line(&json!({"Ok": r.clone()})))], r))
            }
            "quadcam_eject" => {
                self.backend.call("eject", json!({"target": s("target")}))?;
                Ok((vec![text("Ejected.")], json!({"ejected": true})))
            }
            "quadcam_format_card" => {
                if a.get("dry_run").and_then(Value::as_bool).unwrap_or(false) {
                    let plan = self
                        .backend
                        .call("format_plan", json!({"label": s("label")}))?;
                    return Ok((
                        vec![text(format!(
                            "Would erase {} (volume {}, UUID {}, {} bytes, {} clips). To go ahead, call again with device, volume_uuid and confirm=true.",
                            plan["device"].as_str().unwrap_or("?"), plan["volume_name"].as_str().unwrap_or("?"), plan["volume_uuid"].as_str().unwrap_or("?"), plan["size"], plan["clip_count"]
                        ))],
                        plan,
                    ));
                }
                if a.get("confirm").and_then(Value::as_bool) != Some(true) {
                    return Err(anyhow!("Refused: format needs confirm=true, device and volume_uuid. Call with dry_run=true to read them."));
                }
                let req = json!({"device": s("device").unwrap_or_default(), "volume_uuid": s("volume_uuid").unwrap_or_default(), "label": s("label"), "confirm": true});
                let plan = self.backend.call("format", req)?;
                Ok((
                    vec![text(format!(
                        "Erased {} as FAT32 {} and ejected it.",
                        plan["device"].as_str().unwrap_or("?"),
                        plan["label"].as_str().unwrap_or("DVR")
                    ))],
                    plan,
                ))
            }
            _ => Err(anyhow!("unknown tool {name}")),
        }
    }
}

fn names(v: &[&Value]) -> String {
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

fn photos_line(p: &Value) -> String {
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

fn table(view: &[Value]) -> String {
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

const INSTRUCTIONS: &str = "quadcam imports analog FPV DVR clips (AVI/MJPEG). Typical flow: quadcam_status -> \
quadcam_load_clips (or read an already loaded session with quadcam_read_clips) -> look at thumbnails \
with quadcam_read_clips(thumbnails=true) -> quadcam_suggest names/dates -> the person may edit them in \
the app -> quadcam_read_clips to read the final values -> quadcam_export -> quadcam_add_to_photos -> \
quadcam_eject. quadcam_format_card erases the card: only on request, after every clip verified. \
Moments (rolls, flips, punch-outs, dives from radio-log sticks; dead air from the video) and \
suggested keep ranges are in quadcam_read_clips; propose trims as cuts with quadcam_suggest.";

/// Tool descriptors: few tools, one per step of the import flow.
pub fn tools() -> Value {
    let ids = json!({"type": "array", "items": {"type": "integer", "minimum": 0}, "description": "Clip ids from quadcam_read_clips. Omit for all clips."});
    json!([
        {
            "name": "quadcam_status",
            "description": "Show whether the quadcam app is running (mode \"app\": the person sees every change live) or not (\"headless\"), the detected DVR cards and radio log sources, the export defaults (including the saved places and aircraft profiles under status.defaults), and a summary of the loaded session.\n\nBest for: the first call, and checking what is inserted.\nReturns: one line of text plus {mode, status, cards, radios}.\nFollow up with quadcam_load_clips to load a card, or quadcam_read_clips when a session is already loaded.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": {"title": "quadcam status", "readOnlyHint": true, "openWorldHint": false}
        },
        {
            "name": "quadcam_library",
            "description": "List and search the clips already imported into the library folder, newest first: name, date, duration, star rating (0-5), pick or reject flag, place, aircraft, note, keywords, radio-log moments, keep ranges, exported and unsaved cuts, whether it is in Photos, and the file path. Read-only. The loaded card session is NOT here; use quadcam_read_clips for that.\n\nBest for: finding earlier flights (\"last week's flips at the field\"), picking the best clips, or checking what the last import added.\nReturns: one line per clip plus structured records with stable `id`s (the DVR content fingerprint), up to `limit` (default 50) with has_more.\nQuery tips: `query` matches all words against name, note, place, aircraft, keywords and file name; `group` narrows to last_import, moments, picks, rejected or not_in_photos; `day` is YYYY-MM-DD.",
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
            "description": "Suggest a short name, date, note, skip or cut ranges for clips. The values are marked agent-suggested, and in the app they appear as editable suggestions the person can accept or change. Names become the filename slug (YYYY-MM-DD_<name>.mp4, lowercased); an empty name uses the default name (\"flight\"), auto-numbered. `cuts` replaces the clip's cut list; each range exports as an extra file <name>_cutN next to the clip (an empty list removes them). `log_offset_s` says where the first armed log row falls in the clip and moves the log moments. Metadata: `profile` (an aircraft profile name from quadcam_status; empty string to fall back to the log's model, then the default), `place` (a saved place name; empty string removes the location) or `location` {lat, lon}, `keywords` (replaces the clip's own; FPV, the profile's and the moment kinds are added at export), `author`. To apply one value to every clip, send one suggestion per clip id.\n\nBest for: proposing names from what the thumbnails show, dates from a clock burned into the video, and cuts from moments or the keep ranges.\nReturns: every clip's current plan.\nFollow up with quadcam_read_clips to read the final values before quadcam_export; the person may have changed them.",
            "inputSchema": {"type": "object", "required": ["suggestions"], "properties": {"suggestions": {"type": "array", "minItems": 1, "items": {"type": "object", "required": ["id"], "properties": {"id": {"type": "integer", "minimum": 0}, "name": {"type": "string", "maxLength": 80}, "date": {"type": "string", "pattern": "^\\d{4}-\\d{2}-\\d{2}$"}, "note": {"type": "string"}, "skip": {"type": "boolean"}, "cuts": {"type": "array", "maxItems": 20, "items": {"type": "object", "required": ["start", "end"], "properties": {"start": {"type": "number", "minimum": 0, "description": "Seconds into the clip."}, "end": {"type": "number", "minimum": 0}}, "additionalProperties": false}, "description": "Ranges to export as extra files, each at least 0.5 s."}, "log_offset_s": {"type": "number", "description": "Seconds into the clip where the radio log's first armed row falls (the DVR usually starts before arming)."}, "profile": {"type": "string", "maxLength": 80}, "place": {"type": "string", "maxLength": 80, "description": "Saved place name; empty string removes the location."}, "location": {"type": "object", "required": ["lat", "lon"], "properties": {"lat": {"type": "number", "minimum": -90, "maximum": 90}, "lon": {"type": "number", "minimum": -180, "maximum": 180}}, "additionalProperties": false}, "keywords": {"type": "array", "maxItems": 30, "items": {"type": "string", "maxLength": 60}}, "author": {"type": "string", "maxLength": 120}, "reason": {"type": "string", "description": "One short line on why, shown to the person."}, "removed_cuts": {"type": "string", "enum": ["keep", "trash"], "description": "Required when `cuts` drops a cut that was already exported: keep its file (it becomes a clip of its own) or move it to the Trash. Ask the person which."}}, "additionalProperties": false}}}, "additionalProperties": false},
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
    ])
}
