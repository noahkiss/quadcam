//! The Betaflight CLI session (design 6.1): enter with `#`, one line at a time with a wait
//! for the `# ` prompt, stop at the first error, `save` (writes and reboots: the port
//! vanishes), `exit` (leaves the CLI and reboots; unsaved changes are discarded).
//!
//! A session owns its link. `save` and `exit` consume the session and drop the link, so
//! the port (and its lock) is released at once. `wait_for_port` waits for the FC to come
//! back after a reboot.
//!
//! `run_lines` is the write engine the apply engine (`gear/apply/fc.rs`) calls: send each
//! line, stop at the first error and discard with `exit`, else `save`, wait through the
//! reboot, read `dump all` and `exit`.

use super::dump::{self, Config, VerifyFail};
use crate::gear::serial::{is_gone, read_until, Ports, SerialLink, Wait};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::time::{Duration, Instant};

/// The CLI's prompt at the end of every answer.
pub const PROMPT: &[u8] = b"\r\n# ";

/// The baud rate of the USB port (the CDC port ignores it; the API wants one).
pub const BAUD: u32 = 115_200;

/// How long each step waits. `Timing::default()` is the bench-proven set; tests use
/// `Timing::fast()`.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// The prompt counts only once the line is quiet this long.
    pub quiet: Duration,
    /// Entering the CLI and each plain command.
    pub command: Duration,
    /// `dump all` and `diff all`, extended while bytes arrive.
    pub dump: Duration,
    /// How long the port takes to vanish after `save`.
    pub save: Duration,
    /// How long `exit` waits for the goodbye before it drops the port.
    pub exit: Duration,
    /// How long the port may stay gone after a reboot.
    pub reboot: Duration,
    /// How often to look for the port while it is gone.
    pub poll: Duration,
    /// MSP replies.
    pub msp: Duration,
    /// The most a blackbox erase may be given to finish, whatever its estimate says.
    pub erase_max: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            quiet: Duration::from_millis(300),
            command: Duration::from_secs(5),
            dump: Duration::from_secs(30),
            save: Duration::from_secs(8),
            exit: Duration::from_millis(500),
            reboot: Duration::from_secs(30),
            poll: Duration::from_millis(250),
            msp: Duration::from_secs(1),
            erase_max: Duration::from_secs(300),
        }
    }
}

impl Timing {
    /// For fakes: a short quiet wait (a fake's bytes are all there at once), short
    /// timeouts.
    pub fn fast() -> Self {
        Self {
            quiet: Duration::from_millis(5),
            command: Duration::from_millis(500),
            dump: Duration::from_millis(500),
            save: Duration::from_millis(500),
            exit: Duration::from_millis(20),
            reboot: Duration::from_secs(2),
            poll: Duration::from_millis(1),
            msp: Duration::from_millis(500),
            erase_max: Duration::from_secs(1),
        }
    }
}

/// True when a reply reports an error.
pub fn is_error(reply: &str) -> bool {
    reply.contains("###ERROR") || reply.contains("Invalid") || reply.contains("Parse error")
}

/// Lines `run_lines` never sends: the engine saves and exits itself, and these would reset
/// the config, enter a bootloader, erase flash or hand the port to another device.
pub const FORBIDDEN: &[&str] = &[
    "save",
    "exit",
    "defaults",
    "bl",
    "dfu",
    "flash_erase",
    "msc",
    "serialpassthrough",
    "batch",
    "#",
];

/// The refusal for a line `run_lines` will not send, if any.
pub fn forbidden(line: &str) -> Option<&'static str> {
    let verb = line.split_whitespace().next().unwrap_or("");
    FORBIDDEN.iter().copied().find(|f| *f == verb)
}

/// One command and what it answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Reply {
    pub line: String,
    /// The answer without the echo and the prompt, `\n` line endings.
    pub text: String,
    pub error: bool,
}

/// An open CLI session on one port.
pub struct CliSession {
    link: Box<dyn SerialLink>,
    timing: Timing,
}

impl CliSession {
    /// Sends `#` and waits for the prompt. Gives back the banner.
    pub fn enter(mut link: Box<dyn SerialLink>, timing: Timing) -> Result<(CliSession, String)> {
        link.write_all(b"#")?;
        let buf = read_until(link.as_mut(), wait(timing, timing.command, false), |b| {
            b.ends_with(PROMPT)
        })
        .context("No CLI prompt after '#': is this a Betaflight FC?")?;
        let banner = clean(&String::from_utf8_lossy(&buf), "");
        Ok((CliSession { link, timing }, banner))
    }

    pub fn port(&self) -> &str {
        self.link.port()
    }

    /// Sends one line and waits for the prompt.
    pub fn command(&mut self, line: &str) -> Result<Reply> {
        let line = line.trim();
        let long = line.starts_with("dump") || line.starts_with("diff");
        let limit = if long {
            self.timing.dump
        } else {
            self.timing.command
        };
        self.link.write_all(format!("{line}\n").as_bytes())?;
        let buf = read_until(self.link.as_mut(), wait(self.timing, limit, long), |b| {
            b.ends_with(PROMPT)
        })
        .with_context(|| format!("No prompt after {line:?}"))?;
        let text = clean(&String::from_utf8_lossy(&buf), line);
        Ok(Reply {
            error: !long && is_error(&text),
            line: line.to_string(),
            text,
        })
    }

    /// Sends `save`. The FC writes, reboots and the port vanishes; the link is dropped.
    /// Fails when the port is still there after `Timing::save`.
    pub fn save(self) -> Result<String> {
        self.leave("save")
    }

    /// Sends `msc`: the FC reboots as a USB disk and the port vanishes; the link is dropped.
    /// Fails when the port is still there after `Timing::save`.
    pub fn msc(self) -> Result<String> {
        self.leave("msc")
    }

    fn leave(mut self, line: &str) -> Result<String> {
        self.link.write_all(format!("{line}\n").as_bytes())?;
        let port = self.link.port().to_string();
        let mut out = Vec::new();
        let deadline = Instant::now() + self.timing.save;
        while Instant::now() < deadline {
            match self.link.read_some(Duration::from_millis(100)) {
                Ok(b) => out.extend_from_slice(&b),
                Err(e) if is_gone(&e) => {
                    return Ok(clean(&String::from_utf8_lossy(&out), line));
                }
                Err(e) => return Err(e),
            }
        }
        bail!("After {line} the port {port} is still there; check the FC.")
    }

    /// Sends `exit`: the FC leaves the CLI, drops unsaved changes and reboots. The link is
    /// dropped either way; a port already gone is fine.
    pub fn exit(mut self) {
        if self.link.write_all(b"exit\n").is_ok() {
            let deadline = Instant::now() + self.timing.exit;
            while Instant::now() < deadline {
                match self.link.read_some(Duration::from_millis(50)) {
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        }
    }
}

fn wait(t: Timing, timeout: Duration, extend: bool) -> Wait {
    Wait {
        timeout,
        quiet: t.quiet,
        extend,
    }
}

/// An answer without its echoed command and trailing prompt, with `\n` endings.
fn clean(s: &str, echo: &str) -> String {
    let mut t = s.replace("\r\n", "\n");
    if let Some(r) = t.strip_suffix("\n# ") {
        t = r.to_string();
    } else if let Some(r) = t.strip_suffix("# ") {
        t = r.to_string();
    }
    let t = t.trim_start_matches('\n');
    let t = t.strip_prefix(echo).unwrap_or(t);
    t.trim_matches('\n').to_string()
}

/// Waits for `port` to come back after a reboot and opens it.
pub fn wait_for_port(ports: &dyn Ports, port: &str, timing: Timing) -> Result<Box<dyn SerialLink>> {
    let deadline = Instant::now() + timing.reboot;
    let mut last = None;
    loop {
        if ports.list().iter().any(|p| p.port == port) {
            match ports.open(port, BAUD) {
                Ok(l) => return Ok(l),
                Err(e) if e.downcast_ref::<crate::gear::model::Refusal>().is_some() => {
                    return Err(e)
                }
                Err(e) => last = Some(e),
            }
        }
        if Instant::now() >= deadline {
            let why = last.map(|e| format!(" ({e:#})")).unwrap_or_default();
            bail!(
                "The FC did not come back on {port} within {} s{why}.",
                timing.reboot.as_secs()
            );
        }
        std::thread::sleep(timing.poll);
    }
}

/// What `run_lines` did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct RunReport {
    /// Each line sent, with its answer, up to and including a failed one.
    pub sent: Vec<Reply>,
    /// The line that failed; nothing was saved.
    pub failed: Option<Reply>,
    /// Every line passed and `save` ran.
    pub saved: bool,
    /// `dump all` read after the reboot.
    #[serde(skip)]
    pub after_dump: Option<String>,
    /// Lines the after dump does not show as written.
    pub verify: Vec<VerifyFail>,
    /// The answers to the `after` commands `run_lines_with` read in the same session as
    /// the after dump (`version`, `status`, `diff all`).
    #[serde(skip)]
    pub after_replies: Vec<Reply>,
}

/// Sends `lines` in an open session. On the first error: `exit` (nothing saved). Else
/// `save`, wait through the reboot, read `dump all`, `exit`, and verify the lines against
/// that dump. Every link is dropped before it returns.
pub fn run_lines(
    session: CliSession,
    ports: &dyn Ports,
    lines: &[String],
    timing: Timing,
) -> Result<RunReport> {
    run_lines_with(session, ports, lines, timing, &[])
}

/// `run_lines`, reading the read-only commands in `after` too (in order, before the
/// session ends), so the caller can store a whole backup of the saved state.
pub fn run_lines_with(
    session: CliSession,
    ports: &dyn Ports,
    lines: &[String],
    timing: Timing,
    after: &[&str],
) -> Result<RunReport> {
    let lines: Vec<String> = dump::cli_lines(&lines.join("\n"));
    if let Some((l, f)) = lines.iter().find_map(|l| forbidden(l).map(|f| (l, f))) {
        bail!("Refused: the line {l:?} ({f}) is not sent; QuadCam saves and exits itself.");
    }
    let mut session = session;
    let port = session.port().to_string();
    let mut report = RunReport {
        sent: Vec::new(),
        failed: None,
        saved: false,
        after_dump: None,
        verify: Vec::new(),
        after_replies: Vec::new(),
    };
    for l in &lines {
        let r = match session.command(l) {
            Ok(r) => r,
            Err(e) => {
                session.exit();
                return Err(e.context("Nothing was saved"));
            }
        };
        report.sent.push(r.clone());
        if r.error {
            report.failed = Some(r);
            session.exit();
            return Ok(report);
        }
    }
    session.save()?;
    report.saved = true;
    let link = wait_for_port(ports, &port, timing)?;
    let (mut s, _) = CliSession::enter(link, timing)?;
    let mut extra = Vec::new();
    for c in after {
        match s.command(c) {
            Ok(r) => extra.push(r),
            Err(_) => break,
        }
    }
    let d = s.command("dump all");
    s.exit();
    let d = d.map_err(|e| anyhow!("Saved, but the read back failed: {e:#}"))?;
    report.after_replies = extra;
    report.verify = dump::verify_lines(&Config::parse(&d.text), &lines);
    report.after_dump = Some(d.text);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gear::serial::{ScriptLink, Step};

    #[test]
    fn clean_strips_echo_and_prompt() {
        assert_eq!(
            clean("version\r\n# Betaflight / X\r\n\r\n# ", "version"),
            "# Betaflight / X"
        );
        assert_eq!(
            clean("set x = 1\r\nx set to 1\r\n\r\n# ", "set x = 1"),
            "x set to 1"
        );
        assert_eq!(
            clean("\r\nEntering CLI Mode\r\n# ", ""),
            "Entering CLI Mode"
        );
    }

    #[test]
    fn errors() {
        assert!(is_error("###ERROR IN set: INVALID NAME: nope###"));
        assert!(is_error("Invalid value"));
        assert!(!is_error("osd_ah_pos set to 2233"));
        assert_eq!(forbidden("save"), Some("save"));
        assert_eq!(forbidden("defaults nosave"), Some("defaults"));
        assert_eq!(forbidden("set beeper_dshot_beacon_tone = 1"), None);
    }

    #[test]
    fn a_scripted_session() {
        let link = ScriptLink::new(
            "/dev/cu.fake",
            vec![
                Step::expect("#"),
                Step::reply("\r\nEntering CLI Mode, type 'exit' to return, or 'help'\r\n\r\n# "),
                Step::expect("version\n"),
                Step::reply("version\r\n# Betaflight / STM32G47X (G473) 2026.6.0-alpha May 15 2026 / 06:14:55 (e92c10887) MSP API: 1.48\r\n\r\n# "),
                Step::expect("save\n"),
                Step::reply("save\r\nSaving\r\nRebooting"),
                Step::Gone,
            ],
        );
        let (mut s, banner) = CliSession::enter(Box::new(link), Timing::fast()).unwrap();
        assert!(banner.starts_with("Entering CLI Mode"));
        let r = s.command("version").unwrap();
        assert!(!r.error);
        assert!(r.text.contains("2026.6.0-alpha"));
        let out = s.save().unwrap();
        assert!(out.contains("Rebooting"), "{out}");
    }
}
