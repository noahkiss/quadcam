//! The MCP server: JSON-RPC over stdio, the backend that reaches the running app or a local
//! core, and one handler per tool.

use super::params::*;
use super::reload::{self, Handoff, Watch};
use super::render::{
    clip_views, deletion_lines, lib_line, lib_view, names, on_device_text, photos_line,
    places_text, source_label, table, text, update_note, video_text, weather_text,
};
use super::tools::INSTRUCTIONS;
use crate::control::{self, Client};
use crate::core::Core;
use crate::schema::{mcp_deprecations, Deprecation, DEPRECATIONS, SCHEMA_VERSION};
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

pub(super) const SUPPORTED_VERSIONS: &[&str] =
    &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Where tool calls go: the running app or a local core.
pub trait Backend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value>;
    /// "app" when a person is watching in the GUI, "headless" otherwise.
    fn mode(&mut self) -> &'static str;
    /// The job running in this process that a switch to a new build would cut off, as
    /// words. None: free to switch.
    fn blocker(&mut self) -> Option<String> {
        None
    }
}

/// Uses the app when its socket answers, else a headless core. Every call opens a fresh
/// connection, so an app started mid-session takes over, one that quits falls back, and one
/// that quit and relaunched is reached on its new socket instead of a dead connection.
pub struct AutoBackend {
    socket: PathBuf,
    session_file: Option<PathBuf>,
    /// The headless core's settings file; None for the shared one.
    settings_file: Option<PathBuf>,
    local: Option<Arc<Core>>,
}

impl AutoBackend {
    pub fn new(socket: PathBuf, session_file: Option<PathBuf>) -> Self {
        Self {
            socket,
            session_file,
            settings_file: None,
            local: None,
        }
    }

    /// The headless core reads and writes this settings file (tests pass a temp one).
    pub fn with_settings(mut self, file: PathBuf) -> Self {
        self.settings_file = Some(file);
        self
    }

    /// A new connection to the app, or None when no app answers the ping.
    fn connect(&self) -> Option<Client> {
        Client::connect(&self.socket).ok()
    }

    fn local(&mut self) -> Arc<Core> {
        self.local
            .get_or_insert_with(|| {
                let session = self.session_file.clone();
                Arc::new(match self.settings_file.clone() {
                    Some(f) => Core::headless_with(session, Core::real_photos(), f),
                    None => Core::headless(session, Core::real_photos()),
                })
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

    /// A job of the headless core (`Core::restart_blocker`). The app's own jobs run in the
    /// app and outlive this process.
    fn blocker(&mut self) -> Option<String> {
        self.local.as_ref().and_then(|c| c.restart_blocker())
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

/// Reads JSON-RPC lines from stdin and answers on stdout until stdin closes. While it waits
/// for a line and nothing runs, it switches to a newer build on disk (`reload`).
pub fn serve_stdio(session_file: Option<PathBuf>) -> Result<()> {
    let mut out = std::io::stdout().lock();
    if std::env::var_os(reload::PROBE_ENV).is_some() {
        writeln!(out, "{}", json!(reload::probe(SUPPORTED_VERSIONS)))?;
        return Ok(());
    }
    let handoff = reload::take_handoff();
    let mut server = Server::new(AutoBackend::new(control::socket_path(), session_file));
    if let Some(h) = handoff {
        if let Some(note) = server.resume(&h) {
            writeln!(out, "{note}")?;
            out.flush()?;
        }
    }
    let mut watch = Watch::from_env();
    // Stdin as a plain file under this process's own buffer: before a switch the buffer
    // must hold no unread bytes, which `std::io::stdin()`'s hidden buffer would not show.
    // SAFETY: fd 0 stays open for the life of the process, and nothing else reads it.
    let fd0 = std::mem::ManuallyDrop::new(unsafe {
        <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(0)
    });
    let mut input = std::io::BufReader::new(&*fd0);
    let mut line = String::new();
    loop {
        if input.buffer().is_empty() {
            // Idle: every message read is answered, and no part of the next is read.
            if let Some(every) = watch.interval() {
                switch_if_ready(&mut server, &mut watch, &mut out)?;
                if !reload::readable(0, every)? {
                    continue;
                }
            }
        }
        line.clear();
        if input.read_line(&mut line)? == 0 {
            break;
        }
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

/// Runs a newer build that settled on disk in this process, when no job would be cut off.
/// Returns only when it stays: nothing newer, a job runs, or the switch failed.
fn switch_if_ready<B: Backend>(
    server: &mut Server<B>,
    watch: &mut Watch,
    out: &mut impl Write,
) -> Result<()> {
    let Some(next) = watch
        .check(server.protocol.as_deref())
        .map(|p| p.version.clone())
    else {
        return Ok(());
    };
    if let Some(job) = server.backend.blocker() {
        if server.held_by.as_deref() != Some(job.as_str()) {
            eprintln!("quadcam-cli mcp: {next} waits for {job} to finish");
            server.held_by = Some(job);
        }
        return Ok(());
    }
    out.flush()?;
    eprintln!("quadcam-cli mcp: switching to {next}");
    let e = watch.exec(&server.handoff());
    eprintln!("quadcam-cli mcp: staying on {}: {e}", reload::version());
    Ok(())
}

/// A tool's arguments as its type; no arguments are the defaults.
fn args<T: serde::de::DeserializeOwned + Default>(a: &Value) -> Result<T> {
    if a.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(a.clone()).context("bad arguments")
}

/// Refuses arguments past a limit the tool's schema states (sizes, counts, number ranges)
/// before anything runs. The schema's limits reach the client only as words, and arguments
/// some tools pass on as JSON (`suggestions`, `fields`, `values`) skip a typed check: 5,000
/// cuts for one clip would be 5,000 encodes.
fn within_limits(tool: &str, a: &Value) -> Result<()> {
    static SCHEMAS: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    let tools = SCHEMAS.get_or_init(super::tools::tools);
    let Some(schema) = tools
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["name"] == tool)
        .map(|t| &t["inputSchema"])
    else {
        return Ok(());
    };
    check_limits(schema, a, "arguments").context("bad arguments")
}

fn check_limits(schema: &Value, v: &Value, at: &str) -> Result<()> {
    let n = |k: &str| schema.get(k).and_then(Value::as_f64);
    match v {
        Value::String(s) => {
            let len = s.chars().count() as f64;
            if n("maxLength").is_some_and(|m| len > m) {
                anyhow::bail!("{at} is longer than {} characters", n("maxLength").unwrap());
            }
            if n("minLength").is_some_and(|m| len < m) {
                anyhow::bail!("{at} needs at least {} characters", n("minLength").unwrap());
            }
        }
        Value::Number(x) => {
            let x = x.as_f64().unwrap_or_default();
            if n("maximum").is_some_and(|m| x > m) {
                anyhow::bail!("{at} is above {}", n("maximum").unwrap());
            }
            if n("minimum").is_some_and(|m| x < m) {
                anyhow::bail!("{at} is below {}", n("minimum").unwrap());
            }
        }
        Value::Array(items) => {
            let len = items.len() as f64;
            if n("maxItems").is_some_and(|m| len > m) {
                anyhow::bail!("{at} has more than {} items", n("maxItems").unwrap());
            }
            if n("minItems").is_some_and(|m| len < m) {
                anyhow::bail!("{at} needs at least {} items", n("minItems").unwrap());
            }
            if let Some(item) = schema.get("items") {
                for (i, x) in items.iter().enumerate() {
                    check_limits(item, x, &format!("{at}[{i}]"))?;
                }
            }
        }
        Value::Object(m) => {
            if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                for (k, x) in m {
                    if let Some(p) = props.get(k) {
                        let at = if at == "arguments" {
                            k.clone()
                        } else {
                            format!("{at}.{k}")
                        };
                        check_limits(p, x, &at)?;
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub struct Server<B: Backend> {
    pub backend: B,
    /// What warns on use (`schema::DEPRECATIONS`; tests give their own).
    deprecations: &'static [Deprecation],
    /// The protocol version `initialize` agreed on.
    protocol: Option<String>,
    /// The client sent `notifications/initialized`.
    initialized: bool,
    /// The job a switch to a new build last waited for, so the log says it once.
    held_by: Option<String>,
}

impl<B: Backend> Server<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            deprecations: DEPRECATIONS,
            protocol: None,
            initialized: false,
            held_by: None,
        }
    }

    /// The session's state for a newer build of this server.
    pub fn handoff(&self) -> Handoff {
        Handoff {
            format: reload::HANDOFF_FORMAT,
            protocol: self.protocol.clone(),
            initialized: self.initialized,
            tools: reload::tools_digest(&reload::listed_tools()),
            from: reload::version().into(),
        }
    }

    /// Takes over a session an older build started. Some: the notification to send, when
    /// the client was told of another tool list.
    pub fn resume(&mut self, h: &Handoff) -> Option<Value> {
        self.protocol = h.protocol.clone();
        self.initialized = h.initialized;
        eprintln!(
            "quadcam-cli mcp: {} took over from {}",
            reload::version(),
            h.from
        );
        (self.initialized && reload::tools_digest(&reload::listed_tools()) != h.tools)
            .then(|| json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}))
    }

    /// The same server with another deprecation table, to test the warnings.
    pub fn with_deprecations(mut self, table: &'static [Deprecation]) -> Self {
        self.deprecations = table;
        self
    }

    /// Handles one message. Notifications get no reply.
    pub fn handle(&mut self, msg: &Value) -> Option<Value> {
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        if method == "notifications/initialized" {
            self.initialized = true;
        }
        let id = msg.get("id").cloned()?;
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
                self.protocol = Some(version.to_string());
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": true}},
                    "serverInfo": {
                        "name": "quadcam",
                        "version": reload::version(),
                        "schemaVersion": SCHEMA_VERSION,
                    },
                    "instructions": INSTRUCTIONS,
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": reload::listed_tools()})),
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
            Ok((mut content, mut structured)) => {
                // A deprecated tool, action or parameter works and says what replaces it.
                let warnings: Vec<String> = mcp_deprecations(self.deprecations, name, &args)
                    .iter()
                    .map(|d| d.warning())
                    .collect();
                if !warnings.is_empty() {
                    content.push(text(warnings.join("\n")));
                    if let Some(o) = structured.as_object_mut() {
                        o.insert("deprecations".into(), json!(warnings));
                    }
                }
                json!({"content": content, "structuredContent": structured, "isError": false})
            }
            Err(e) => json!({"content": [text(format!("{e:#}"))], "isError": true}),
        }
    }

    fn run_tool(&mut self, name: &str, a: &Value) -> Result<(Vec<Value>, Value)> {
        within_limits(name, a)?;
        if let Some(r) = super::gear::run(&mut self.backend, name, a) {
            return r;
        }
        match name {
            "quadcam_status" => {
                let _: StatusArgs = args(a)?;
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
                    "Schema {SCHEMA_VERSION}. Mode: {mode}{}. Cards: {}. Radios: {}. Session: {}.{}",
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
                            "{} {} clips from {}, {} imported",
                            sess["clips"],
                            source_label(&sess["kind"]),
                            sess["source"].as_str().unwrap_or("?"),
                            sess["imported"]
                        )
                    },
                    update_note(&status["update"])
                );
                Ok((
                    vec![text(line)],
                    json!({"schema_version": SCHEMA_VERSION, "mode": mode, "status": status, "cards": cards, "radios": radios}),
                ))
            }
            "quadcam_library" => {
                let x: LibraryArgs = args(a)?;
                let filter = json!({
                    "query": x.query, "group": x.group, "day": x.day, "place": x.place,
                    "aircraft": x.aircraft, "min_rating": x.min_rating,
                });
                let limit = x.limit.unwrap_or(50) as usize;
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
                let x: LibraryEditArgs = args(a)?;
                let ids = x.ids.clone().unwrap_or_default();
                if ids.is_empty() {
                    return Err(anyhow!(
                        "ids is required: library clip ids from quadcam_library."
                    ));
                }
                if let Some(sh) = &x.shift {
                    return self.library_shift(&x, sh, ids);
                }
                let edit = json!({
                    "note": x.note, "author": x.author, "place": x.place, "date": x.date,
                    "time": x.time, "profile": x.profile, "keywords": x.keywords,
                    "location": x.location.map(|l| json!({"lat": l.lat, "lon": l.lon})),
                });
                let mut changed: Vec<String> = Vec::new();
                if x.rating.is_some() {
                    changed.push("rating".into());
                }
                if x.flag.is_some() {
                    changed.push("flag".into());
                }
                if x.name.is_some() {
                    changed.push("name".into());
                }
                // Keys in name order, as a JSON object lists them.
                let mut keys: Vec<&String> = edit
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter(|(_, v)| !v.is_null())
                    .map(|(k, _)| k)
                    .collect();
                keys.sort();
                changed.extend(keys.into_iter().cloned());
                let mut update = edit.clone();
                update["ids"] = json!(ids);
                update["rating"] = json!(x.rating);
                update["flag"] = json!(x.flag);
                update["name"] = json!(x.name);
                // One call: every value and id is checked before any file changes.
                self.backend.call("library_update", update)?;
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
                let x: LibraryFilesArgs = args(a)?;
                let action = x.action.with_context(|| {
                    format!("action is required: {}", one_of(LIBRARY_FILES_ACTIONS))
                })?;
                let ids: Vec<String> = x.ids.unwrap_or_default();
                let one = || -> Result<String> {
                    match ids.as_slice() {
                        [id] => Ok(id.clone()),
                        _ => Err(anyhow!("{action} works on one clip; give exactly one id.")),
                    }
                };
                match action.as_str() {
                    "cuts" => {
                        let id = one()?;
                        let cuts: Vec<Value> = x
                            .cuts
                            .clone()
                            .context("cuts is required (an empty list removes every cut)")?
                            .iter()
                            .map(|c| json!({"start": c.start, "end": c.end}))
                            .collect();
                        let change = self.backend.call(
                            "library_cuts",
                            json!({"id": id, "cuts": cuts, "removed_cuts": x.removed_cuts}),
                        )?;
                        if change["status"] == "confirm" {
                            let files = &change["files"];
                            return Err(anyhow!(
                                "These cuts were exported already: {}. Ask the person, then call again with removed_cuts \"keep\" (the files stay as clips of their own) or \"trash\".",
                                files.as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")
                            ));
                        }
                        let mut line = "Cut list saved. New ranges are not files yet; call action export_cuts to write them.".to_string();
                        if x.export == Some(true) {
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
                    "split_by_flight" => {
                        let id = one()?;
                        let change = self.backend.call("library_split", json!({"id": id}))?;
                        let n = change["cuts"].as_array().map(Vec::len).unwrap_or(0);
                        let mut line = format!("Cut list saved: {n} cuts, one per radio-log flight added. New ranges are not files yet; call action export_cuts to write them.");
                        if x.export == Some(true) {
                            let made = self
                                .backend
                                .call("library_export_cuts", json!({"id": id}))?;
                            line = format!(
                                "Cut list saved: {n} cuts; {} cut files written.",
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
                            .call("library_photos", json!({"ids": ids, "album": x.album}))?;
                        Ok((vec![text(photos_line(&json!({"Ok": r.clone()})))], r))
                    }
                    "match_logs" => {
                        let r = self.backend.call(
                            "library_match_logs",
                            json!({"ids": ids, "logs": x.log_dir, "day": x.day, "apply": x.apply.unwrap_or(false)}),
                        )?;
                        let mut lines: Vec<String> = r["warnings"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(|w| format!("Warning: {w}"))
                            .collect();
                        for c in r["clips"].as_array().into_iter().flatten() {
                            lines.push(format!(
                                "{} {}: {}{}{}",
                                c["id"].as_str().unwrap_or(""),
                                c["path"].as_str().unwrap_or(""),
                                c["badge"].as_str().unwrap_or(""),
                                c["reason"]
                                    .as_str()
                                    .map(|w| format!(" ({w})"))
                                    .unwrap_or_default(),
                                if c["applied"] == true {
                                    "; written"
                                } else {
                                    ""
                                }
                            ));
                        }
                        Ok((vec![text(lines.join("\n"))], r))
                    }
                    "weather" => {
                        let r = self.backend.call("library_weather", json!({"ids": ids}))?;
                        Ok((vec![text(weather_text(&r))], r))
                    }
                    "analyze_video" => {
                        let r = self.backend.call(
                            "library_analyze_video",
                            json!({"ids": ids, "again": x.again.unwrap_or(false)}),
                        )?;
                        Ok((vec![text(video_text(&r))], r))
                    }
                    "apply_name_format" => {
                        let r = self
                            .backend
                            .call("library_apply_name_format", json!({"ids": ids}))?;
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
                        "unknown action {other:?}; use {}",
                        one_of(LIBRARY_FILES_ACTIONS)
                    )),
                }
            }
            "quadcam_places" => {
                let x: PlacesArgs = args(a)?;
                let action = x.action.clone().unwrap_or_else(|| "list".into());
                match action.as_str() {
                    "list" => {
                        let places = self.backend.call("places", Value::Null)?;
                        Ok((vec![text(places_text(&places))], json!({"places": places})))
                    }
                    "search" => {
                        let query = x
                            .query
                            .clone()
                            .context("query is required: an address or a place name")?;
                        let hits = self.backend.call(
                            "place_search",
                            json!({"query": query, "provider": x.provider, "limit": x.limit}),
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
                    "here" | "reverse" => {
                        let hit = if action == "here" {
                            self.backend.call("place_here", Value::Null)?
                        } else {
                            let (Some(lat), Some(lon)) = (x.lat, x.lon) else {
                                return Err(anyhow!("lat and lon are required for reverse"));
                            };
                            self.backend
                                .call("place_reverse", json!({"lat": lat, "lon": lon}))?
                        };
                        Ok((
                            vec![text(format!(
                                "{} | {} | {}, {}. Save it with action save, name and lat/lon.",
                                hit["name"]
                                    .as_str()
                                    .filter(|n| !n.is_empty())
                                    .unwrap_or("(no name)"),
                                hit["address"].as_str().unwrap_or(""),
                                hit["lat"],
                                hit["lon"]
                            ))],
                            json!({"result": hit}),
                        ))
                    }
                    "save" => {
                        let name = x.name.clone().context("name is required")?;
                        let p = self.backend.call(
                            "place_save",
                            json!({"name": name, "lat": x.lat, "lon": x.lon, "new_name": x.new_name}),
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
                        let name = x.name.clone().context("name is required")?;
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
                        "unknown action {other:?}; use {}",
                        one_of(PLACES_ACTIONS)
                    )),
                }
            }
            "quadcam_profiles" => {
                let x: ProfilesArgs = args(a)?;
                let action = x.action.clone().unwrap_or_else(|| "list".into());
                let out = match action.as_str() {
                    "list" => None,
                    "save" => {
                        let name = x.name.clone().context("name is required")?;
                        let fields = x
                            .fields
                            .clone()
                            .filter(|v| !v.is_null())
                            .unwrap_or(json!({}));
                        let p = self.backend.call(
                            "profile_save",
                            json!({"name": name, "fields": fields, "new_name": x.new_name}),
                        )?;
                        if x.default == Some(true) {
                            self.backend
                                .call("profile_default", json!({"name": p["name"]}))?;
                        }
                        Some(format!(
                            "Saved profile {}.",
                            p["name"].as_str().unwrap_or("?")
                        ))
                    }
                    "delete" => {
                        let name = x.name.clone().context("name is required")?;
                        let p = self.backend.call("profile_delete", json!({"name": name}))?;
                        Some(format!(
                            "Deleted profile {}.",
                            p["name"].as_str().unwrap_or("?")
                        ))
                    }
                    "set_default" => {
                        let name = x.name.clone().unwrap_or_default();
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
                            "unknown action {other:?}; use {}",
                            one_of(PROFILES_ACTIONS)
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
                let x: SettingsArgs = args(a)?;
                let action = x.action.clone().unwrap_or_else(|| "read".into());
                if action.starts_with("module") {
                    return self.modules_action(&action, &x);
                }
                let view = match action.as_str() {
                    "read" => self.backend.call("settings", Value::Null)?,
                    "write" => {
                        let values = x
                            .values
                            .clone()
                            .filter(|v| v.as_object().is_some_and(|o| !o.is_empty()))
                            .context(
                                "values is required: {setting: value}; null resets a setting",
                            )?;
                        if let Some(k) = PERSON_ONLY_SETTINGS
                            .iter()
                            .find(|k| values.get(**k).is_some())
                        {
                            anyhow::bail!(
                                "{k} is set by the person, not over MCP: in the app's Settings window, or `quadcam-cli settings set {k}=...`"
                            );
                        }
                        self.backend
                            .call("settings_set", json!({"values": values}))?
                    }
                    other => {
                        return Err(anyhow!(
                            "unknown action {other:?}; use {}",
                            one_of(SETTINGS_ACTIONS)
                        ))
                    }
                };
                let (settings, secrets) = settings_read(&view);
                let line = settings_line(&view, &settings, &secrets, action == "write");
                let map = |l: Vec<(&str, Value)>| -> serde_json::Map<String, Value> {
                    l.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
                };
                Ok((
                    vec![text(line)],
                    json!({"path": view["path"], "settings": map(settings), "secrets": map(secrets)}),
                ))
            }
            "quadcam_load_clips" => {
                let x: LoadClipsArgs = args(a)?;
                let session = self.backend.call(
                    "load",
                    json!({"source": x.source, "device": x.card, "join": x.join}),
                )?;
                let view = clip_views(&session, None);
                Ok((
                    vec![text(format!(
                        "Loaded {} {} clips from {}.\n{}",
                        view.len(),
                        source_label(&session["kind"]),
                        session["source"].as_str().unwrap_or("?"),
                        table(&view)
                    ))],
                    json!({"clips": view}),
                ))
            }
            "quadcam_read_clips" => {
                let x: ReadClipsArgs = args(a)?;
                let session = self.backend.call("session", Value::Null)?;
                if session.is_null() {
                    return Err(anyhow!("No clips loaded. Call quadcam_load_clips first."));
                }
                let want: Option<Vec<u64>> = x.ids;
                let view = clip_views(&session, want.as_deref());
                let mut content = vec![text(table(&view))];
                if x.thumbnails.unwrap_or(false) {
                    let max = x.max_thumbnails.unwrap_or(12) as usize;
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
                let x: MatchLogsArgs = args(a)?;
                let logs = if x.no_logs.unwrap_or(false) {
                    json!({"kind": "none"})
                } else if let Some(d) = &x.log_dir {
                    json!({"kind": "dir", "path": d})
                } else {
                    json!({"kind": "keep"})
                };
                let session = self
                    .backend
                    .call("dates", json!({"logs": logs, "day": x.day}))?;
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
                let x: SuggestArgs = args(a)?;
                if x.suggestions.is_none() && x.on_device != Some(true) {
                    anyhow::bail!("send suggestions, on_device=true, or both");
                }
                let mut head = String::new();
                let mut on_device = Value::Null;
                if x.on_device == Some(true) {
                    let r = self.backend.call("suggest_on_device", json!({}))?;
                    head = on_device_text(&r);
                    on_device = r;
                }
                let session = match x.suggestions {
                    Some(patches) => {
                        let s = self
                            .backend
                            .call("suggest", json!({"patches": patches, "editor": "agent"}))?;
                        head.push_str(&format!(
                            "Suggestions saved and marked as agent-suggested{}. ",
                            if self.backend.mode() == "app" {
                                "; the person can edit them in quadcam"
                            } else {
                                ""
                            },
                        ));
                        s
                    }
                    None => self.backend.call("session", json!({}))?,
                };
                let view = clip_views(&session, None);
                Ok((
                    vec![text(format!(
                        "{head}Read back final values with quadcam_read_clips before export.\n{}",
                        table(&view)
                    ))],
                    json!({"clips": view, "on_device": on_device}),
                ))
            }
            "quadcam_export" => {
                let x: ExportArgs = args(a)?;
                let mut opts = json!({});
                for (k, v) in [
                    ("output_dir", x.output_dir),
                    ("format", x.format),
                    ("album", x.album),
                ] {
                    if let Some(v) = v {
                        opts[k] = json!(v);
                    }
                }
                for (k, v) in [
                    ("keep_originals", x.keep_originals),
                    ("add_time", x.add_time),
                    ("add_to_photos", x.add_to_photos),
                    ("keep_clips", x.keep_clips),
                ] {
                    if let Some(v) = v {
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
                if let Some(d) = out["clip_deletion"].as_array() {
                    line.push_str(&format!("\n{}", deletion_lines(d)));
                }
                Ok((vec![text(line)], out))
            }
            "quadcam_verify" => {
                let x: VerifyArgs = args(a)?;
                let reports = self.backend.call("verify", json!({"ids": x.ids}))?;
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
                let x: AddToPhotosArgs = args(a)?;
                let r = self
                    .backend
                    .call("photos", json!({"ids": x.ids, "album": x.album}))?;
                Ok((vec![text(photos_line(&json!({"Ok": r.clone()})))], r))
            }
            "quadcam_eject" => {
                let x: EjectArgs = args(a)?;
                self.backend.call("eject", json!({"target": x.target}))?;
                Ok((vec![text("Safe to remove.")], json!({"ejected": true})))
            }
            "quadcam_format_card" => {
                let x: FormatCardArgs = args(a)?;
                let prep = x.prep.unwrap_or(false);
                if x.dry_run.unwrap_or(false) {
                    let plan = if prep {
                        if x.mount.is_none() && x.card.is_none() {
                            return Err(anyhow!(
                                "Card prep's dry run needs mount (the card's mount point) or card (its device id)."
                            ));
                        }
                        self.backend.call(
                            "card_prep_plan",
                            json!({"mount": x.mount, "device": x.card, "label": x.label}),
                        )?
                    } else {
                        self.backend
                            .call("format_plan", json!({"label": x.label}))?
                    };
                    let advice: String = plan["warnings"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|w| w.as_str())
                        .map(|w| format!(" Note: {w}"))
                        .collect();
                    return Ok((
                        vec![text(format!(
                            "Would erase {} as {} (volume {}, UUID {}, {} bytes, {} clips). To go ahead, call again with device, volume_uuid{} and confirm=true.{advice}",
                            plan["device"].as_str().unwrap_or("?"), plan["filesystem"].as_str().unwrap_or("FAT32"), plan["volume_name"].as_str().unwrap_or("?"), plan["volume_uuid"].as_str().unwrap_or("?"), plan["size"], plan["clip_count"],
                            if prep { ", prep=true" } else { "" }
                        ))],
                        plan,
                    ));
                }
                if x.confirm != Some(true) {
                    return Err(anyhow!("Refused: format needs confirm=true, device and volume_uuid. Call with dry_run=true to read them."));
                }
                let req = json!({"device": x.device.unwrap_or_default(), "volume_uuid": x.volume_uuid.unwrap_or_default(), "label": x.label, "confirm": true});
                let plan = self
                    .backend
                    .call(if prep { "card_prep" } else { "format" }, req)?;
                Ok((
                    vec![text(format!(
                        "Erased {} as {} {}. It is safe to remove.",
                        plan["device"].as_str().unwrap_or("?"),
                        plan["filesystem"].as_str().unwrap_or("FAT32"),
                        plan["label"].as_str().unwrap_or("DVR")
                    ))],
                    plan,
                ))
            }
            _ => Err(anyhow!("unknown tool {name}")),
        }
    }
}

impl<B: Backend> Server<B> {
    /// `quadcam_library_edit` with `shift`: the plan, written only with `confirm`.
    fn library_shift(
        &mut self,
        x: &LibraryEditArgs,
        sh: &TimeShift,
        ids: Vec<String>,
    ) -> Result<(Vec<Value>, Value)> {
        let others = [
            x.rating.is_some(),
            x.flag.is_some(),
            x.name.is_some(),
            x.note.is_some(),
            x.keywords.is_some(),
            x.author.is_some(),
            x.place.is_some(),
            x.location.is_some(),
            x.profile.is_some(),
            x.date.is_some(),
            x.time.is_some(),
        ];
        if others.into_iter().any(|o| o) {
            return Err(anyhow!(
                "shift takes no other change; send the other fields in a call of their own."
            ));
        }
        let apply = x.confirm.unwrap_or(false);
        let r = self.backend.call(
            "library_time_shift",
            json!({"ids": ids, "from": sh.from, "date": sh.date, "time": sh.time,
                   "logs": sh.log_dir, "apply": apply}),
        )?;
        let rows = r["clips"].as_array().cloned().unwrap_or_default();
        let when = |d: &Value, t: &Value| match (d.as_str(), t.as_str()) {
            (Some(d), Some(t)) => format!("{d} {t}"),
            (Some(d), None) => d.to_string(),
            _ => "-".to_string(),
        };
        let lines: Vec<String> = rows
            .iter()
            .map(|c| {
                let how = match c["source"].as_str() {
                    Some("set") => "given".to_string(),
                    Some("log") => format!(
                        "radio log{}",
                        c["reason"]
                            .as_str()
                            .map(|r| format!(": {r}"))
                            .unwrap_or_default()
                    ),
                    Some("skipped") => format!(
                        "skipped: {}",
                        c["reason"].as_str().unwrap_or("no time of day")
                    ),
                    _ => "offset".to_string(),
                };
                format!(
                    "- {} [{}]: {} -> {} ({how}){}",
                    c["name"].as_str().unwrap_or("?"),
                    c["id"].as_str().unwrap_or("?"),
                    when(&c["old_date"], &c["old_time"]),
                    when(&c["date"], &c["time"]),
                    if c["changed"].as_bool() == Some(false) {
                        ", unchanged"
                    } else {
                        ""
                    }
                )
            })
            .collect();
        let changed = rows.iter().filter(|c| c["changed"] == true).count();
        let offset = r["offset_s"].as_i64().unwrap_or(0);
        let sign = if offset < 0 { "-" } else { "+" };
        let (h, m) = (offset.abs() / 3600, offset.abs() % 3600 / 60);
        let lead = if apply {
            format!(
                "Shifted {changed} clip{}",
                if changed == 1 { "" } else { "s" }
            )
        } else {
            format!(
                "Plan: {changed} clip{} would change. Nothing was written; call again with confirm=true to write",
                if changed == 1 { "" } else { "s" }
            )
        };
        let warnings: Vec<String> = r["warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|w| w.as_str().map(|w| format!("Warning: {w}")))
            .collect();
        Ok((
            vec![text(format!(
                "{lead} (offset {sign}{h}:{m:02}).\n{}{}",
                lines.join("\n"),
                if warnings.is_empty() {
                    String::new()
                } else {
                    format!("\n{}", warnings.join("\n"))
                }
            ))],
            json!({"applied": apply, "offset_s": offset, "clips": rows, "warnings": r["warnings"]}),
        ))
    }

    /// `quadcam_settings` actions `modules`, `module_install` and `module_remove`.
    fn modules_action(&mut self, action: &str, x: &SettingsArgs) -> Result<(Vec<Value>, Value)> {
        let name = || {
            x.module
                .clone()
                .context("module is required: the module's name (see action modules)")
        };
        let one = |v: Value| Value::Array(vec![v]);
        let (lead, list) = match action {
            "modules" => (String::new(), self.backend.call("modules", Value::Null)?),
            "module_install" => {
                let v = self.backend.call(
                    "module_install",
                    json!({"name": name()?, "confirm": x.confirm.unwrap_or(false)}),
                )?;
                ("Installed. ".to_string(), one(v))
            }
            "module_remove" => {
                let v = self
                    .backend
                    .call("module_remove", json!({"name": name()?}))?;
                ("Removed. ".to_string(), one(v))
            }
            other => {
                return Err(anyhow!(
                    "unknown action {other:?}; use {}",
                    one_of(SETTINGS_ACTIONS)
                ))
            }
        };
        let rows = list.as_array().cloned().unwrap_or_default();
        let lines = rows
            .iter()
            .map(|m| {
                let pin = if m["newest"].is_null() {
                    &m["pinned"]
                } else {
                    &m["newest"]
                };
                let installed = match m["installed"]["version"].as_str() {
                    Some(v) => format!("installed {v}"),
                    None => "not installed".into(),
                };
                format!(
                    "{}: {} | {}{} | license {} ({}) | {:.1} MB from {} | source {}{}",
                    m["name"].as_str().unwrap_or("?"),
                    pin["title"].as_str().unwrap_or("?"),
                    installed,
                    if m["update"] == json!(true) {
                        format!(" | update to {}", pin["version"].as_str().unwrap_or("?"))
                    } else if m["installed"].is_null() {
                        format!(
                            " | would install {}",
                            pin["version"].as_str().unwrap_or("?")
                        )
                    } else {
                        String::new()
                    },
                    pin["license"].as_str().unwrap_or("?"),
                    pin["license_url"].as_str().unwrap_or("?"),
                    pin["assets"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|x| x["size"].as_u64()).sum::<u64>())
                        .unwrap_or(0) as f64
                        / 1e6,
                    pin["homepage"].as_str().unwrap_or("?"),
                    pin["source"].as_str().unwrap_or("?"),
                    m["problem"]
                        .as_str()
                        .map(|p| format!(" | problem: {p}"))
                        .unwrap_or_default(),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok((
            vec![text(format!("{lead}{lines}"))],
            json!({"modules": rows}),
        ))
    }
}

/// Settings by name, in `settings::KEYS` order.
type Named = Vec<(&'static str, Value)>;

/// Settings MCP never writes: `modules` names a program the app runs, `tts_base_url` the
/// server the stored `tts_key` is sent to.
const PERSON_ONLY_SETTINGS: &[&str] = &["modules", "tts_base_url"];

/// The settings a `quadcam_settings` read shows: every named setting of `settings::KEYS`
/// with its value in effect (`named`; an older app without it gives `effective`, then the
/// file's value), and each secret's hint by name.
fn settings_read(view: &Value) -> (Named, Named) {
    use crate::settings::{KEYS, SECRET_KEYS};
    let settings = KEYS
        .iter()
        .filter(|k| !SECRET_KEYS.contains(&k.file))
        .filter_map(|k| {
            let name = k.name?;
            let v = view["named"]
                .get(name)
                .or_else(|| view["effective"].get(name))
                .or_else(|| view["values"].get(k.file))
                .cloned()
                .unwrap_or(Value::Null);
            Some((name, v))
        })
        .collect();
    let secrets = KEYS
        .iter()
        .filter(|k| SECRET_KEYS.contains(&k.file))
        .filter_map(|k| Some((k.name?, view["secrets"].get(k.file)?.clone())))
        .collect();
    (settings, secrets)
}

/// The text answer of a settings read or write: every setting, one line per group, and
/// each secret as its hint (`secret::hint`), never its value.
fn settings_line(
    view: &Value,
    settings: &[(&str, Value)],
    secrets: &[(&str, Value)],
    saved: bool,
) -> String {
    fn shown(v: &Value) -> String {
        match v {
            Value::Null => "none".into(),
            Value::String(s) if s.is_empty() => "\"\"".into(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    }
    fn group(name: &str) -> usize {
        if name.starts_with("gear_") || name == "firmware_check" || name.starts_with("betaflight_")
        {
            1
        } else if name.starts_with("tts_") || name.starts_with("voice_") {
            2
        } else if name.starts_with("elrs_") {
            3
        } else {
            0
        }
    }
    let mut groups: [Vec<String>; 4] = Default::default();
    for (name, v) in settings {
        groups[group(name)].push(format!("{name} {}", shown(v)));
    }
    let mut out = format!(
        "{}Settings file {}.",
        if saved { "Saved. " } else { "" },
        view["path"].as_str().unwrap_or("?")
    );
    for (label, items) in ["General", "Gear", "Voice", "ExpressLRS"]
        .iter()
        .zip(&groups)
    {
        if !items.is_empty() {
            out.push_str(&format!("\n{label}: {}", items.join(" | ")));
        }
    }
    let hints: Vec<String> = crate::settings::KEYS
        .iter()
        .filter(|k| crate::settings::SECRET_KEYS.contains(&k.file))
        .filter_map(|k| {
            let name = k.name?;
            let hint = secrets
                .iter()
                .find(|(n, _)| *n == name)
                .and_then(|(_, h)| h["hint"].as_str())
                .unwrap_or("not set");
            Some(format!("{name} {hint}"))
        })
        .collect();
    out.push_str(&format!("\nSecrets: {}", hints.join(" | ")));
    out
}
