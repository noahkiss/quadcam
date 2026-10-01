//! The MCP server: JSON-RPC over stdio, the backend that reaches the running app or a local
//! core, and one handler per tool.

use super::render::{clip_views, lib_line, lib_view, names, photos_line, places_text, table, text};
use super::tools::{tools, INSTRUCTIONS};
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
                let clips: Vec<Value> = all.iter().take(limit).map(lib_view).collect();
                let lines: Vec<String> = clips.iter().map(lib_line).collect();
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
            "quadcam_library_edit" => {
                let ids: Vec<String> = a
                    .get("ids")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()
                    .context("ids is a list of library clip ids")?
                    .unwrap_or_default();
                if ids.is_empty() {
                    return Err(anyhow!(
                        "ids is required: library clip ids from quadcam_library."
                    ));
                }
                let mut changed: Vec<String> = Vec::new();
                let rating = a.get("rating").filter(|v| !v.is_null()).cloned();
                let flag = s("flag");
                if rating.is_some() || flag.is_some() {
                    if rating.is_some() {
                        changed.push("rating".into());
                    }
                    if flag.is_some() {
                        changed.push("flag".into());
                    }
                    self.backend.call(
                        "library_rate",
                        json!({"ids": ids, "rating": rating, "flag": flag}),
                    )?;
                }
                if let Some(name) = s("name") {
                    if ids.len() != 1 {
                        return Err(anyhow!("name renames one clip; give exactly one id."));
                    }
                    self.backend
                        .call("library_rename", json!({"id": ids[0], "name": name}))?;
                    changed.push("name".into());
                }
                let mut edit = json!({});
                for k in ["note", "author", "place", "date", "time", "profile"] {
                    if let Some(v) = s(k) {
                        edit[k] = json!(v);
                    }
                }
                for k in ["keywords", "location"] {
                    if let Some(v) = a.get(k).filter(|v| !v.is_null()) {
                        edit[k] = v.clone();
                    }
                }
                if edit.as_object().is_some_and(|o| !o.is_empty()) {
                    for id in &ids {
                        let mut x = edit.clone();
                        x["id"] = json!(id);
                        self.backend.call("library_edit", x)?;
                    }
                    changed.extend(edit.as_object().into_iter().flat_map(|o| o.keys().cloned()));
                }
                if changed.is_empty() {
                    return Err(anyhow!("Nothing to change: give rating, flag, name, note, keywords, author, place, location, profile, date or time."));
                }
                let lib = self.backend.call("library", json!({}))?;
                let clips: Vec<Value> = lib["clips"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|c| c["id"].as_str().is_some_and(|i| ids.iter().any(|x| x == i)))
                    .map(lib_view)
                    .collect();
                Ok((
                    vec![text(format!(
                        "Changed {} on {} clip{}.\n{}",
                        changed.join(", "),
                        ids.len(),
                        if ids.len() == 1 { "" } else { "s" },
                        clips.iter().map(lib_line).collect::<Vec<_>>().join("\n")
                    ))],
                    json!({"clips": clips}),
                ))
            }
            "quadcam_library_files" => {
                let action = s("action")
                    .context("action is required: cuts, export_cuts, trash, photos, rebuild or apply_name_format")?;
                let ids: Vec<String> = a
                    .get("ids")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()
                    .context("ids is a list of library clip ids")?
                    .unwrap_or_default();
                let one = || -> Result<String> {
                    match ids.as_slice() {
                        [id] => Ok(id.clone()),
                        _ => Err(anyhow!("{action} works on one clip; give exactly one id.")),
                    }
                };
                match action.as_str() {
                    "cuts" => {
                        let id = one()?;
                        let cuts = a
                            .get("cuts")
                            .cloned()
                            .context("cuts is required (an empty list removes every cut)")?;
                        let change = self.backend.call(
                            "library_cuts",
                            json!({"id": id, "cuts": cuts, "removed_cuts": s("removed_cuts")}),
                        )?;
                        if change["status"] == "confirm" {
                            let files = &change["files"];
                            return Err(anyhow!(
                                "These cuts were exported already: {}. Ask the person, then call again with removed_cuts \"keep\" (the files stay as clips of their own) or \"trash\".",
                                files.as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")
                            ));
                        }
                        let mut line = "Cut list saved. New ranges are not files yet; call action export_cuts to write them.".to_string();
                        if a.get("export").and_then(Value::as_bool) == Some(true) {
                            let made = self
                                .backend
                                .call("library_export_cuts", json!({"id": id}))?;
                            line = format!(
                                "Cut list saved; {} cut files written.",
                                made.as_array().map(Vec::len).unwrap_or(0)
                            );
                        }
                        Ok((vec![text(line)], change))
                    }
                    "export_cuts" => {
                        let made = self
                            .backend
                            .call("library_export_cuts", json!({"id": one()?}))?;
                        let n = made.as_array().map(Vec::len).unwrap_or(0);
                        Ok((
                            vec![text(format!("{n} cut files written."))],
                            json!({"cuts": made}),
                        ))
                    }
                    "trash" => {
                        if ids.is_empty() {
                            return Err(anyhow!("ids is required."));
                        }
                        let r = self.backend.call("library_trash", json!({"ids": ids}))?;
                        Ok((
                            vec![text(format!(
                                "Moved {} files to the Trash{}.",
                                r["trashed"].as_array().map(Vec::len).unwrap_or(0),
                                match r["failed"].as_array().map(Vec::len).unwrap_or(0) {
                                    0 => String::new(),
                                    n => format!("; {n} failed"),
                                }
                            ))],
                            r,
                        ))
                    }
                    "photos" => {
                        if ids.is_empty() {
                            return Err(anyhow!("ids is required."));
                        }
                        let r = self
                            .backend
                            .call("library_photos", json!({"ids": ids, "album": s("album")}))?;
                        Ok((vec![text(photos_line(&json!({"Ok": r.clone()})))], r))
                    }
                    "apply_name_format" => {
                        let r = self.backend.call(
                            "library_apply_name_format",
                            json!({"ids": ids}),
                        )?;
                        Ok((
                            vec![text(format!(
                                "Renamed {} clips with their cuts and originals; {} already matched; {} do not start with a date{}.",
                                r["renamed"].as_array().map(Vec::len).unwrap_or(0),
                                r["unchanged"],
                                r["skipped"].as_array().map(Vec::len).unwrap_or(0),
                                match r["failed"].as_array().map(Vec::len).unwrap_or(0) {
                                    0 => String::new(),
                                    n => format!("; {n} failed"),
                                }
                            ))],
                            r,
                        ))
                    }
                    "rebuild" => {
                        let r = self.backend.call("library_rebuild", Value::Null)?;
                        Ok((
                            vec![text(format!(
                                "Index rebuilt from the files: {} clips, {} cuts{}.",
                                r["clips"],
                                r["cuts"],
                                match r["problems"].as_array().map(Vec::len).unwrap_or(0) {
                                    0 => String::new(),
                                    n => format!("; {n} files could not be read"),
                                }
                            ))],
                            r,
                        ))
                    }
                    other => Err(anyhow!(
                        "unknown action {other:?}; use cuts, export_cuts, trash, photos, rebuild or apply_name_format"
                    )),
                }
            }
            "quadcam_places" => {
                let action = s("action").unwrap_or_else(|| "list".into());
                match action.as_str() {
                    "list" => {
                        let places = self.backend.call("places", Value::Null)?;
                        Ok((vec![text(places_text(&places))], json!({"places": places})))
                    }
                    "search" => {
                        let query =
                            s("query").context("query is required: an address or a place name")?;
                        let hits = self.backend.call(
                            "place_search",
                            json!({"query": query, "provider": s("provider"), "limit": a.get("limit")}),
                        )?;
                        let arr = hits.as_array().cloned().unwrap_or_default();
                        let line = if arr.is_empty() {
                            format!("No places found for {query:?}. Try a fuller address, or another provider.")
                        } else {
                            arr.iter()
                                .enumerate()
                                .map(|(i, h)| {
                                    format!(
                                        "{}. {} | {} | {}, {}",
                                        i + 1,
                                        h["name"].as_str().unwrap_or(""),
                                        h["address"].as_str().unwrap_or(""),
                                        h["lat"],
                                        h["lon"]
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        };
                        Ok((vec![text(line)], json!({"results": hits})))
                    }
                    "save" => {
                        let name = s("name").context("name is required")?;
                        let p = self.backend.call(
                            "place_save",
                            json!({"name": name, "lat": a.get("lat"), "lon": a.get("lon"), "new_name": s("new_name")}),
                        )?;
                        Ok((
                            vec![text(format!(
                                "Saved place {} at {}, {}.",
                                p["name"].as_str().unwrap_or("?"),
                                p["lat"],
                                p["lon"]
                            ))],
                            json!({"place": p}),
                        ))
                    }
                    "delete" => {
                        let name = s("name").context("name is required")?;
                        let r = self.backend.call("place_delete", json!({"name": name}))?;
                        let cleared = r["profiles_cleared"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default();
                        Ok((
                            vec![text(format!(
                                "Deleted place {}.{}",
                                r["name"].as_str().unwrap_or("?"),
                                if cleared.is_empty() {
                                    String::new()
                                } else {
                                    format!(
                                        " These profiles no longer have a default place: {}.",
                                        cleared
                                            .iter()
                                            .filter_map(Value::as_str)
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    )
                                }
                            ))],
                            r,
                        ))
                    }
                    other => Err(anyhow!(
                        "unknown action {other:?}; use list, search, save or delete"
                    )),
                }
            }
            "quadcam_profiles" => {
                let action = s("action").unwrap_or_else(|| "list".into());
                let out = match action.as_str() {
                    "list" => None,
                    "save" => {
                        let name = s("name").context("name is required")?;
                        let fields = a
                            .get("fields")
                            .cloned()
                            .filter(|v| !v.is_null())
                            .unwrap_or(json!({}));
                        let p = self.backend.call(
                            "profile_save",
                            json!({"name": name, "fields": fields, "new_name": s("new_name")}),
                        )?;
                        if a.get("default").and_then(Value::as_bool) == Some(true) {
                            self.backend
                                .call("profile_default", json!({"name": p["name"]}))?;
                        }
                        Some(format!(
                            "Saved profile {}.",
                            p["name"].as_str().unwrap_or("?")
                        ))
                    }
                    "delete" => {
                        let name = s("name").context("name is required")?;
                        let p = self.backend.call("profile_delete", json!({"name": name}))?;
                        Some(format!(
                            "Deleted profile {}.",
                            p["name"].as_str().unwrap_or("?")
                        ))
                    }
                    "set_default" => {
                        let name = s("name").unwrap_or_default();
                        self.backend
                            .call("profile_default", json!({"name": name}))?;
                        Some(if name.is_empty() {
                            "No default profile now.".into()
                        } else {
                            format!("{name} is the default profile.")
                        })
                    }
                    other => {
                        return Err(anyhow!(
                            "unknown action {other:?}; use list, save, delete or set_default"
                        ))
                    }
                };
                let r = self.backend.call("profiles", Value::Null)?;
                let list = r["profiles"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|p| {
                        format!(
                            "{}{}: {} | {} {} | {} | place {} | radio models {}",
                            p["name"].as_str().unwrap_or("?"),
                            if r["default_profile"] == p["name"] {
                                " (default)"
                            } else {
                                ""
                            },
                            p["aircraft"].as_str().unwrap_or(""),
                            p["camera_make"].as_str().unwrap_or(""),
                            p["camera_model"].as_str().unwrap_or(""),
                            p["video_system"].as_str().unwrap_or(""),
                            p["place"].as_str().unwrap_or("none"),
                            p["edgetx_models"]
                                .as_array()
                                .map(|m| m
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join(", "))
                                .filter(|m| !m.is_empty())
                                .unwrap_or_else(|| "none".into()),
                        )
                    })
                    .collect::<Vec<_>>();
                let mut line = out.map(|o| format!("{o}\n")).unwrap_or_default();
                line.push_str(&if list.is_empty() {
                    "No aircraft profiles.".to_string()
                } else {
                    list.join("\n")
                });
                Ok((vec![text(line)], r))
            }
            "quadcam_settings" => {
                let action = s("action").unwrap_or_else(|| "read".into());
                let view = match action.as_str() {
                    "read" => self.backend.call("settings", Value::Null)?,
                    "write" => {
                        let values = a
                            .get("values")
                            .filter(|v| v.as_object().is_some_and(|o| !o.is_empty()))
                            .cloned()
                            .context(
                                "values is required: {setting: value}; null resets a setting",
                            )?;
                        self.backend
                            .call("settings_set", json!({"values": values}))?
                    }
                    other => return Err(anyhow!("unknown action {other:?}; use read or write")),
                };
                let e = &view["effective"];
                let line = format!(
                    "{}Settings file {}.\noutput_dir {} | layout {} | place_folders {} | format {} | encoder {} | keep_originals {} | add_time {} | default_name {:?} | photos_album {:?} | format_label {} | log_dir {} | geocoder {} | name_date_format {} | default_profile {} | tunables {} | google_places_key {}",
                    if action == "write" { "Saved. " } else { "" },
                    view["path"].as_str().unwrap_or("?"),
                    e["output_dir"].as_str().unwrap_or("none"), e["layout"].as_str().unwrap_or("?"), e["place_folders"],
                    e["format"].as_str().unwrap_or("?"), e["encoder"].as_str().unwrap_or("?"), e["keep_originals"], e["add_time"],
                    e["default_name"].as_str().unwrap_or(""), e["photos_album"].as_str().unwrap_or(""), e["format_label"].as_str().unwrap_or(""),
                    e["log_dir"].as_str().unwrap_or("none"), e["geocoder"].as_str().unwrap_or("?"), e["name_date_format"].as_str().unwrap_or("?"), e["default_profile"].as_str().unwrap_or("none"), e["tunables"],
                    if view["values"]["googlePlacesKey"].is_null() { "not set" } else { "set" },
                );
                let settings: serde_json::Map<String, Value> = [
                    "output_dir",
                    "layout",
                    "place_folders",
                    "format",
                    "encoder",
                    "keep_originals",
                    "add_time",
                    "default_name",
                    "photos_album",
                    "format_label",
                    "log_dir",
                    "geocoder",
                    "name_date_format",
                    "default_profile",
                    "tunables",
                ]
                .iter()
                .map(|k| (k.to_string(), e[*k].clone()))
                .collect();
                Ok((
                    vec![text(line)],
                    json!({"path": view["path"], "settings": settings}),
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
