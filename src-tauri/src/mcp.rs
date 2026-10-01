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

/// One library clip as the tools return it.
fn lib_view(c: &Value) -> Value {
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

fn lib_line(c: &Value) -> String {
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

fn places_text(places: &Value) -> String {
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

const INSTRUCTIONS: &str = "quadcam imports analog FPV DVR clips (AVI/MJPEG) into a library folder. \
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
            "description": "Show whether the quadcam app is running (mode \"app\": the person sees every change live) or not (\"headless\"), the detected DVR cards and radio log sources, the export defaults (including the saved places and aircraft profiles under status.defaults), and a summary of the loaded session.\n\nBest for: the first call, and checking what is inserted.\nReturns: one line of text plus {mode, status, cards, radios}.\nFollow up with quadcam_load_clips to load a card, or quadcam_read_clips when a session is already loaded.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": {"title": "quadcam status", "readOnlyHint": true, "openWorldHint": false}
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
