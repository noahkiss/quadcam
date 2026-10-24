//! Local control channel: newline-delimited JSON-RPC 2.0 over a Unix domain socket in
//! `~/Library/Application Support/app.quadcam/`. The running GUI serves it so an agent (the
//! MCP server or the CLI) drives the same session the person sees. The folder is 0700 and
//! the socket 0600, so only the owner can connect.

use crate::core::Core;
use crate::paths::support_dir;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub fn socket_path() -> PathBuf {
    support_dir().join("control.sock")
}

/// Binds the socket and serves `core` on background threads. Refuses to take over a socket
/// another live instance is serving.
pub fn serve(core: Arc<Core>, path: &Path) -> Result<()> {
    let dir = path.parent().context("socket path has no folder")?;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            bail!("another quadcam is already serving {}", path.display());
        }
        std::fs::remove_file(path)?;
    }
    let listener =
        UnixListener::bind(path).with_context(|| format!("binding {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let core = core.clone();
            std::thread::spawn(move || handle(core, stream));
        }
    });
    Ok(())
}

/// The longest request line the app reads. A longer one closes the connection.
const MAX_LINE: u64 = 16 << 20;

/// How long a connection may sit without sending its next request. A client sends each
/// request as soon as it connects; a long job runs while the app is not reading.
const IDLE: Duration = Duration::from_secs(60);

fn handle(core: Arc<Core>, stream: UnixStream) {
    let Ok(mut out) = stream.try_clone() else {
        return;
    };
    if stream.set_read_timeout(Some(IDLE)).is_err() {
        return;
    }
    let mut reader = BufReader::new(stream);
    loop {
        let mut buf = Vec::new();
        match (&mut reader).take(MAX_LINE + 1).read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if buf.last() != Some(&b'\n') && buf.len() as u64 > MAX_LINE {
            let reply = json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600, "message": format!("request longer than {MAX_LINE} bytes")}});
            let _ = writeln!(out, "{reply}");
            return;
        }
        let Ok(line) = String::from_utf8(buf) else {
            return;
        };
        if line.trim().is_empty() {
            continue;
        }
        let reply = respond(&core, line.trim_end_matches(['\r', '\n']));
        if writeln!(out, "{reply}").is_err() {
            return;
        }
    }
}

/// Handles one JSON-RPC request line and returns the response line.
pub fn respond(core: &Core, line: &str) -> Value {
    let req: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}})
        }
    };
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    let result = if method == "ping" {
        Ok(json!({"app": "quadcam", "gui": core.has_gui(), "pid": std::process::id()}))
    } else {
        core.dispatch(method, params)
    };
    match result {
        Ok(v) => json!({"jsonrpc": "2.0", "id": id, "result": v}),
        Err(e) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": format!("{e:#}")}})
        }
    }
}

/// A connection to a running app.
pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
}

impl Client {
    /// Connects and pings. Err means no app is serving this socket.
    pub fn connect(path: &Path) -> Result<Client> {
        let s = UnixStream::connect(path)
            .with_context(|| format!("no QuadCam app at {}", path.display()))?;
        let mut c = Client {
            reader: BufReader::new(s.try_clone()?),
            writer: s,
            next_id: 1,
        };
        c.writer.set_write_timeout(Some(Duration::from_secs(10)))?;
        c.call("ping", Value::Null)?;
        Ok(c)
    }

    /// One call. Format may wait for a person to click in the GUI, so reads do not time out.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.writer,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        )?;
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            bail!("the QuadCam app closed the connection");
        }
        let v: Value = serde_json::from_str(&line)?;
        if let Some(e) = v.get("error") {
            bail!(
                "{}",
                e.get("message").and_then(Value::as_str).unwrap_or("error")
            );
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }
}
