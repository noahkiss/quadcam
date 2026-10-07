//! `quadcam-cli gear fc ...`: a Betaflight flight controller over USB. Each command opens
//! the port, works and releases it. `read` reboots the FC when it ends (the CLI `exit`).

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use quadcam_lib::api::{self, call};
use quadcam_lib::core::Core;
use quadcam_lib::gear::bf::dump;
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum FcCmd {
    /// Read the FC's identity over MSP: board, firmware, version, device id. No reboot.
    Identify {
        /// The FC's port (/dev/cu.usbmodem...); omit when one FC is plugged in.
        #[arg(long)]
        port: Option<String>,
    },
    /// Read through the CLI (read-only commands). The FC reboots when the read ends.
    Read {
        #[arg(long)]
        port: Option<String>,
        /// A read-only command (repeat it): version, status, get NAME, diff all, dump all.
        /// Default: version, status, diff all, dump all.
        #[arg(long = "cmd")]
        commands: Vec<String>,
        /// Also write each `diff all` and `dump all` to <OUT>.diff_all.txt and
        /// <OUT>.dump_all.txt. Refuses to overwrite a file.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Offline: every CLI line of EXPECTED must appear verbatim in the `diff all` file.
    Check {
        /// A `diff all` file.
        diff: PathBuf,
        /// A CLI file (comments and blank lines are skipped).
        expected: PathBuf,
    },
    /// Known issues of FC boards and builds.
    Notes {
        #[arg(long)]
        board: Option<String>,
        #[arg(long)]
        version: Option<String>,
    },
    /// Each FC's USB heat timer, as the app last read it.
    Usb,
}

pub fn run(core: &Core, cmd: FcCmd) -> Result<Value> {
    Ok(match cmd {
        FcCmd::Identify { port } => {
            serde_json::to_value(call::gear_fc_identify(core, api::FcPortParams { port })?)?
        }
        FcCmd::Read {
            port,
            commands,
            out,
        } => {
            if let Some(o) = &out {
                for k in ["diff_all", "dump_all"] {
                    let p = with_suffix(o, k);
                    if p.exists() {
                        bail!("Refused: {} exists; pick another --out.", p.display());
                    }
                }
            }
            let j = call::gear_fc_read(core, api::FcReadParams { port, commands })?;
            let mut written = Vec::new();
            if let Some(o) = &out {
                for (cmd, k) in [("diff all", "diff_all"), ("dump all", "dump_all")] {
                    if let Some(text) = j.result.file(cmd) {
                        let p = with_suffix(o, k);
                        std::fs::write(&p, text)
                            .with_context(|| format!("writing {}", p.display()))?;
                        written.push(p.display().to_string());
                    }
                }
            }
            let mut v = serde_json::to_value(&j)?;
            v["written"] = json!(written);
            v
        }
        FcCmd::Check { diff, expected } => {
            let d = std::fs::read_to_string(&diff)
                .with_context(|| format!("reading {}", diff.display()))?;
            let e = std::fs::read_to_string(&expected)
                .with_context(|| format!("reading {}", expected.display()))?;
            let missing = dump::missing_lines(&d, &e);
            if !missing.is_empty() {
                bail!(
                    "{} expected lines missing from {}: {}",
                    missing.len(),
                    diff.display(),
                    missing.join(" | ")
                );
            }
            json!({"ok": true, "missing": missing})
        }
        FcCmd::Notes { board, version } => serde_json::to_value(call::gear_board_notes(
            core,
            api::BoardNotesParams { board, version },
        )?)?,
        FcCmd::Usb => serde_json::to_value(call::gear_usb_timers(core)?)?,
    })
}

/// `<out>.<kind>.txt`.
fn with_suffix(out: &std::path::Path, kind: &str) -> PathBuf {
    let s = out.to_string_lossy();
    let stem = s.strip_suffix(".txt").unwrap_or(&s);
    PathBuf::from(format!("{stem}.{kind}.txt"))
}
