//! The EdgeTX CLI over the radio's USB serial port (VCP). The radio answers it when its USB
//! serial port is set to CLI (`serialPort: VCP` in `radio.yml`), at 115200 baud, with the
//! prompt `>`. QuadCam learned the commands from EdgeTX's documentation and a measured
//! radio, and copies no EdgeTX code (EdgeTX is GPL-2.0; QuadCam is MIT).
//!
//! The CLI cannot move file contents, so it is for small jobs only:
//!
//! - `identify`: `ver`, the running firmware's board and version;
//! - `ls`: a folder's files, to check what is on the card;
//! - `play`: a sound file on the radio's speaker (a voice line preview);
//! - `beep`;
//! - `reboot`: a plain restart. QuadCam never sends `reboot bootloader` or any other
//!   command: the commands here are the whole list.
//!
//! A `RadioCli` owns its link. Opening one goes through `Ports::open`, which takes the
//! per-port lock and refuses a port another process holds (`serial::other_holders`), as
//! the FC link does. `FakeRadioCli` stands in for a radio in tests.

use crate::gear::serial::{
    is_gone, read_until, FakePorts, PortGone, PortInfo, Ports, SerialLink, Wait,
};
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The baud rate the radio's CLI uses.
pub const BAUD: u32 = 115_200;

/// What to do when the radio does not answer with a prompt.
pub const NO_PROMPT: &str = "The radio did not answer with a > prompt. On the radio, set the USB serial port (VCP) to CLI in the hardware settings, then plug it in again in USB Serial mode.";

/// How long each step waits.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// The prompt counts once the line is quiet this long.
    pub quiet: Duration,
    /// Opening and each command.
    pub command: Duration,
    /// How long `reboot` waits for a goodbye before it drops the port.
    pub reboot: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            quiet: Duration::from_millis(150),
            command: Duration::from_secs(5),
            reboot: Duration::from_millis(500),
        }
    }
}

impl Timing {
    /// For fakes.
    pub fn fast() -> Self {
        Self {
            quiet: Duration::from_millis(5),
            command: Duration::from_millis(500),
            reboot: Duration::from_millis(20),
        }
    }
}

/// What `ver` says about the running firmware.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RadioInfo {
    /// The board (`pocket`), when `ver` names one.
    #[serde(default)]
    pub board: Option<String>,
    /// The firmware version (`2.12.4`).
    #[serde(default)]
    pub version: Option<String>,
    /// The reply, for the person to read.
    pub text: String,
}

/// A name `ls` printed, with its size when it printed one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CliEntry {
    pub name: String,
    #[serde(default)]
    pub size: Option<u64>,
}

/// True when `buf` ends with the prompt: a `>` alone on the last line.
pub fn at_prompt(buf: &[u8]) -> bool {
    let t = match buf {
        [rest @ .., b' '] => rest,
        b => b,
    };
    match t {
        [.., b'>'] => {
            let before = &t[..t.len() - 1];
            before.is_empty() || matches!(before.last(), Some(b'\n' | b'\r'))
        }
        _ => false,
    }
}

/// A card path the CLI may be given: absolute, plain characters, no `..`.
pub fn check_path(path: &str) -> Result<()> {
    let ok = path.starts_with('/')
        && path.len() <= 200
        && !path.split('/').any(|p| p == "..")
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | ' '));
    if !ok {
        bail!("{path:?} is not a path QuadCam sends to the radio: use an absolute card path such as /SOUNDS/en/hello.wav.");
    }
    Ok(())
}

/// The board and version in a `ver` reply. `ver` prints `key: value` lines; a key naming
/// the board gives `board`, and the first `x.y.z` in the text is the version.
pub fn parse_ver(text: &str) -> RadioInfo {
    let mut board = None;
    for l in text.lines() {
        if let Some((k, v)) = l.split_once(':') {
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim();
            if matches!(k.as_str(), "board" | "brd" | "hw" | "target") && !v.is_empty() {
                board = Some(v.to_string());
            }
        }
    }
    let version = text
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .find(|t| {
            let p: Vec<&str> = t.split('.').collect();
            p.len() == 3 && p.iter().all(|x| !x.is_empty() && x.len() <= 3)
        })
        .map(str::to_string);
    RadioInfo {
        board,
        version,
        text: text.trim().to_string(),
    }
}

/// The entries in an `ls` reply: per line, a name and a size when the line ends in one.
/// A trailing `/` marks a folder and is kept.
pub fn parse_ls(text: &str) -> Vec<CliEntry> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut parts: Vec<&str> = l.split_whitespace().collect();
            let size = if parts.len() > 1 {
                parts.last().and_then(|s| s.parse::<u64>().ok())
            } else {
                None
            };
            if size.is_some() {
                parts.pop();
            }
            let name = parts.join(" ");
            let name = name
                .rsplit('/')
                .next()
                .filter(|n| !n.is_empty())
                .unwrap_or(&name);
            CliEntry {
                name: name.to_string(),
                size,
            }
        })
        .collect()
}

/// An open CLI on the radio's serial port.
pub struct RadioCli {
    link: Box<dyn SerialLink>,
    timing: Timing,
}

impl RadioCli {
    /// Opens the port and waits for the prompt.
    pub fn open(ports: &dyn Ports, port: &str, timing: Timing) -> Result<Self> {
        let mut link = ports.open(port, BAUD)?;
        link.write_all(b"\n")?;
        let wait = Wait {
            timeout: timing.command,
            quiet: timing.quiet,
            extend: false,
        };
        read_until(link.as_mut(), wait, at_prompt).map_err(|e| {
            if is_gone(&e) {
                e
            } else {
                anyhow!(NO_PROMPT)
            }
        })?;
        Ok(Self { link, timing })
    }

    /// Sends one line and returns the reply without the echo and the prompt.
    fn run(&mut self, line: &str) -> Result<String> {
        if line.contains(['\n', '\r']) {
            bail!("A CLI command is one line.");
        }
        self.link.write_all(format!("{line}\n").as_bytes())?;
        // A long `ls` keeps the wait alive while bytes arrive.
        let wait = Wait {
            timeout: self.timing.command,
            quiet: self.timing.quiet,
            extend: true,
        };
        let raw = read_until(self.link.as_mut(), wait, at_prompt)?;
        let text = String::from_utf8_lossy(&raw)
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        let mut lines: Vec<&str> = text.lines().collect();
        if lines.first().is_some_and(|l| l.trim() == line) {
            lines.remove(0);
        }
        if lines.last().is_some_and(|l| l.trim() == ">") {
            lines.pop();
        }
        Ok(lines.join("\n").trim().to_string())
    }

    /// The running firmware (`ver`).
    pub fn identify(&mut self) -> Result<RadioInfo> {
        Ok(parse_ver(&self.run("ver")?))
    }

    /// A folder's entries (`ls`). An error reply (`ls` of a missing folder) is an error.
    pub fn ls(&mut self, dir: &str) -> Result<Vec<CliEntry>> {
        check_path(dir)?;
        let text = self.run(&format!("ls {dir}"))?;
        if looks_like_error(&text) {
            bail!("The radio answered ls {dir}: {text}");
        }
        Ok(parse_ls(&text))
    }

    /// Plays a sound file on the radio.
    pub fn play(&mut self, file: &str) -> Result<String> {
        check_path(file)?;
        let text = self.run(&format!("play {file}"))?;
        if looks_like_error(&text) {
            bail!("The radio answered play {file}: {text}");
        }
        Ok(text)
    }

    pub fn beep(&mut self) -> Result<()> {
        self.run("beep").map(|_| ())
    }

    /// Restarts the radio. The port vanishes; that is the answer.
    pub fn reboot(mut self) -> Result<()> {
        self.link.write_all(b"reboot\n")?;
        // A goodbye may arrive, or the port may go at once: both mean it worked.
        match self.link.read_some(self.timing.reboot) {
            Ok(_) => Ok(()),
            Err(e) if is_gone(&e) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

fn looks_like_error(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    [
        "invalid",
        "unknown",
        "not found",
        "error",
        "failed",
        "no such",
    ]
    .iter()
    .any(|w| t.contains(w))
}

/// What `compare_listing` found.
#[derive(Debug, Default, PartialEq)]
pub struct Listing {
    /// Files the radio's `ls` does not show.
    pub missing: Vec<String>,
    /// Files whose size `ls` printed and differs.
    pub differ: Vec<String>,
    /// Files QuadCam could not check: their folder is not one the CLI can list (a name
    /// `check_path` refuses, or a space, which the CLI splits on, or `ls` failed), or their
    /// own name is not plain ASCII, which a listing may print another way.
    pub not_checked: Vec<String>,
}

/// True when `name` holds only the characters the CLI prints as they are.
fn plain_name(name: &str) -> bool {
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '))
}

/// The files `want` (card paths) that the radio's `ls` does not show, and those whose
/// size differs when `ls` printed one. A name matches without regard to case (the card is
/// FAT). Folders are listed once each. A folder the CLI cannot list is "not checked", never
/// "missing".
pub fn compare_listing(cli: &mut RadioCli, want: &[(String, u64)]) -> Result<Listing> {
    let mut by_dir: BTreeMap<String, Vec<&(String, u64)>> = BTreeMap::new();
    for w in want {
        let dir = match w.0.rsplit_once('/') {
            Some((d, _)) => format!("/{d}"),
            None => "/".to_string(),
        };
        by_dir.entry(dir).or_default().push(w);
    }
    let mut out = Listing::default();
    for (dir, files) in by_dir {
        let unlisted =
            |out: &mut Listing| out.not_checked.extend(files.iter().map(|f| f.0.clone()));
        if dir.contains(' ') || check_path(&dir).is_err() {
            unlisted(&mut out);
            continue;
        }
        let listed = match cli.ls(&dir) {
            Ok(l) => l,
            Err(e) if is_gone(&e) => return Err(e),
            Err(_) => {
                // A folder its parent's listing lacks is missing, files and all. Any other
                // failure says nothing about the files.
                let (parent, name) = dir.rsplit_once('/').unwrap_or(("", &dir));
                let parent = if parent.is_empty() { "/" } else { parent };
                match cli.ls(parent) {
                    Ok(up) if !up.iter().any(|e| e.name.eq_ignore_ascii_case(name)) => {
                        out.missing.extend(files.iter().map(|f| f.0.clone()))
                    }
                    Err(e) if is_gone(&e) => return Err(e),
                    _ => unlisted(&mut out),
                }
                continue;
            }
        };
        for (path, size) in files {
            let name = path.rsplit('/').next().unwrap_or(path);
            if !plain_name(name) {
                out.not_checked.push(path.clone());
                continue;
            }
            match listed.iter().find(|e| e.name.eq_ignore_ascii_case(name)) {
                None => out.missing.push(path.clone()),
                Some(e) if e.size.is_some_and(|s| s != *size) => out.differ.push(path.clone()),
                Some(_) => {}
            }
        }
    }
    Ok(out)
}

/// A simulated radio on its serial port: answers `ver`, `ls`, `play`, `beep`, `reboot`,
/// with the echo and the `>` prompt. Clones share the radio.
#[derive(Clone)]
pub struct FakeRadioCli {
    st: Arc<Mutex<FakeState>>,
}

struct FakeState {
    ver: String,
    /// Card paths to sizes (`/SOUNDS/en/hello.wav`).
    files: BTreeMap<String, u64>,
    /// `ls` prints sizes.
    sizes: bool,
    /// The CLI is off: opens work, nothing answers.
    silent: bool,
    log: Vec<String>,
    played: Vec<String>,
    rebooted: bool,
    opens: u32,
}

impl FakeRadioCli {
    /// A radio whose `ver` prints a board and a version.
    pub fn new(board: &str, version: &str) -> Self {
        Self {
            st: Arc::new(Mutex::new(FakeState {
                ver: format!("board: {board}\nvers: edgetx-{board}-{version}\nDATE: 2026-01-01"),
                files: BTreeMap::new(),
                sizes: true,
                silent: false,
                log: Vec::new(),
                played: Vec::new(),
                rebooted: false,
                opens: 0,
            })),
        }
    }

    fn st(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.st.lock().unwrap()
    }

    /// Puts a file on the radio's card.
    pub fn with_file(self, path: &str, size: u64) -> Self {
        self.st().files.insert(path.to_string(), size);
        self
    }

    /// `ls` prints names only.
    pub fn without_sizes(self) -> Self {
        self.st().sizes = false;
        self
    }

    /// The radio's CLI is not on: no prompt ever comes.
    pub fn silent(self) -> Self {
        self.st().silent = true;
        self
    }

    /// Every line received, in order.
    pub fn log(&self) -> Vec<String> {
        self.st().log.clone()
    }

    pub fn played(&self) -> Vec<String> {
        self.st().played.clone()
    }

    pub fn rebooted(&self) -> bool {
        self.st().rebooted
    }

    pub fn opens(&self) -> u32 {
        self.st().opens
    }

    /// Ports with this radio on `port` (the STM32 VCP ids, a radio's product string).
    pub fn ports(&self, port: &str) -> FakePorts {
        let radio = self.clone();
        let info = PortInfo {
            port: port.into(),
            vid: 0x0483,
            pid: 0x5740,
            serial_number: Some("00000000FAKE".into()),
            manufacturer: Some("OpenTX".into()),
            product: Some("Pocket Serial Port".into()),
        };
        FakePorts::new(vec![info]).with_opener(move |port, _| {
            radio.st().opens += 1;
            Ok(Box::new(FakeLink {
                port: port.to_string(),
                radio: radio.clone(),
                rx: Vec::new(),
                out: VecDeque::new(),
                gone: false,
            }))
        })
    }

    fn answer(&self, line: &str) -> Option<String> {
        let mut s = self.st();
        s.log.push(line.to_string());
        let (verb, arg) = line.split_once(' ').unwrap_or((line, ""));
        let arg = arg.trim();
        Some(match verb {
            "" => String::new(),
            "ver" => s.ver.clone(),
            "beep" => String::new(),
            "ls" => {
                let dir = arg.trim_end_matches('/');
                let mut names: BTreeMap<String, Option<u64>> = BTreeMap::new();
                let known =
                    dir.is_empty() || s.files.keys().any(|p| p.starts_with(&format!("{dir}/")));
                for (p, size) in &s.files {
                    let Some(rest) = p.strip_prefix(&format!("{dir}/")) else {
                        continue;
                    };
                    match rest.split_once('/') {
                        Some((sub, _)) => names.insert(format!("{sub}/"), None),
                        None => names.insert(rest.to_string(), Some(*size)),
                    };
                }
                if !known {
                    format!("{arg}: not found")
                } else {
                    names
                        .into_iter()
                        .map(|(n, sz)| match (sz, s.sizes) {
                            (Some(sz), true) => format!("{n} {sz}"),
                            _ => n,
                        })
                        .collect::<Vec<_>>()
                        .join("\r\n")
                }
            }
            "play" => {
                if s.files.keys().any(|p| p.eq_ignore_ascii_case(arg)) {
                    s.played.push(arg.to_string());
                    String::new()
                } else {
                    format!("play: {arg} not found")
                }
            }
            "reboot" => {
                s.rebooted = true;
                return None;
            }
            other => format!("Unknown command: {other}"),
        })
    }
}

struct FakeLink {
    port: String,
    radio: FakeRadioCli,
    rx: Vec<u8>,
    out: VecDeque<Vec<u8>>,
    gone: bool,
}

impl SerialLink for FakeLink {
    fn port(&self) -> &str {
        &self.port
    }
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        if self.gone || self.radio.st().rebooted {
            return Err(PortGone(self.port.clone()).into());
        }
        if self.radio.st().silent {
            return Ok(());
        }
        self.rx.extend_from_slice(data);
        while let Some(i) = self.rx.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.rx.drain(..=i).collect();
            let line = String::from_utf8_lossy(&line).trim().to_string();
            match self.radio.answer(&line) {
                Some(reply) => {
                    let mut text = format!("{line}\r\n");
                    if !reply.is_empty() {
                        text.push_str(&reply);
                        text.push_str("\r\n");
                    }
                    text.push_str("> ");
                    self.out.push_back(text.into_bytes());
                }
                None => self.gone = true,
            }
        }
        Ok(())
    }
    fn read_some(&mut self, _timeout: Duration) -> Result<Vec<u8>> {
        if let Some(b) = self.out.pop_front() {
            return Ok(b);
        }
        if self.gone {
            return Err(PortGone(self.port.clone()).into());
        }
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(radio: &FakeRadioCli) -> Result<RadioCli> {
        RadioCli::open(
            &radio.ports("/dev/cu.usbmodemFAKE2"),
            "/dev/cu.usbmodemFAKE2",
            Timing::fast(),
        )
    }

    #[test]
    fn the_prompt_is_a_lone_angle_bracket() {
        assert!(at_prompt(b"> "));
        assert!(at_prompt(b"ver\r\nboard: x\r\n> "));
        assert!(at_prompt(b"x\n>"));
        assert!(!at_prompt(b"a > b"));
        assert!(!at_prompt(b"play: a>"));
        assert!(!at_prompt(b""));
    }

    #[test]
    fn identify_reads_board_and_version() {
        let radio = FakeRadioCli::new("pocket", "2.12.4");
        let i = cli(&radio).unwrap().identify().unwrap();
        assert_eq!(i.board.as_deref(), Some("pocket"));
        assert_eq!(i.version.as_deref(), Some("2.12.4"));
        assert_eq!(radio.log().last().map(String::as_str), Some("ver"));
    }

    #[test]
    fn ls_lists_files_folders_and_sizes() {
        let radio = FakeRadioCli::new("pocket", "2.12.4")
            .with_file("/MODELS/model01.yml", 1200)
            .with_file("/SOUNDS/en/hello.wav", 5000)
            .with_file("/SOUNDS/en/SYSTEM/a.wav", 10);
        let mut c = cli(&radio).unwrap();
        assert_eq!(
            c.ls("/SOUNDS/en").unwrap(),
            vec![
                CliEntry {
                    name: "SYSTEM/".into(),
                    size: None
                },
                CliEntry {
                    name: "hello.wav".into(),
                    size: Some(5000)
                },
            ]
        );
        assert!(c.ls("/NOPE").is_err());
        assert!(
            c.ls("../etc").is_err(),
            "a path is checked before it is sent"
        );
        assert_eq!(radio.log(), ["", "ls /SOUNDS/en", "ls /NOPE"]);
    }

    #[test]
    fn compare_finds_missing_and_different_files() {
        let radio = FakeRadioCli::new("pocket", "2.12.4")
            .with_file("/MODELS/model01.yml", 1200)
            .with_file("/SOUNDS/en/hello.wav", 4999);
        let mut c = cli(&radio).unwrap();
        let want = vec![
            ("MODELS/model01.yml".to_string(), 1200),
            ("MODELS/model02.yml".to_string(), 800),
            ("SOUNDS/en/hello.wav".to_string(), 5000),
            ("SCRIPTS/x.lua".to_string(), 1),
        ];
        let l = compare_listing(&mut c, &want).unwrap();
        assert_eq!(l.missing, ["MODELS/model02.yml", "SCRIPTS/x.lua"]);
        assert_eq!(l.differ, ["SOUNDS/en/hello.wav"]);
        assert!(l.not_checked.is_empty());
    }

    #[test]
    fn a_folder_the_cli_cannot_list_is_not_checked_not_missing() {
        let radio = FakeRadioCli::new("pocket", "2.12.4")
            .with_file("/MODELS/model01.yml", 1200)
            .with_file("/MODELS/My Model.txt", 30)
            .with_file("/SOUNDS/My Pack/a.wav", 10)
            .with_file("/SOUNDS/fr/\u{e9}t\u{e9}.wav", 10);
        let mut c = cli(&radio).unwrap();
        let want = vec![
            ("MODELS/model01.yml".to_string(), 1200),
            // A space in a name: its folder lists, and `ls` shows the name.
            ("MODELS/My Model.txt".to_string(), 30),
            // A space in the folder: the CLI would split the argument.
            ("SOUNDS/My Pack/a.wav".to_string(), 10),
            // A folder check_path refuses, and a non-ASCII name in a folder that lists.
            ("SOUNDS/x;y/b.wav".to_string(), 1),
            ("SOUNDS/fr/\u{e9}t\u{e9}.wav".to_string(), 10),
            // A folder the card lacks: its parent lists, without it.
            ("NOPE/c.wav".to_string(), 1),
        ];
        let l = compare_listing(&mut c, &want).unwrap();
        assert_eq!(l.missing, ["NOPE/c.wav"], "{l:?}");
        assert!(l.differ.is_empty(), "{l:?}");
        assert_eq!(
            l.not_checked,
            [
                "SOUNDS/My Pack/a.wav",
                "SOUNDS/fr/\u{e9}t\u{e9}.wav",
                "SOUNDS/x;y/b.wav"
            ]
        );
        // Neither the folder with a space nor the refused one went out.
        assert!(!radio
            .log()
            .iter()
            .any(|l| l.contains("My Pack") || l.contains(';')));
    }

    #[test]
    fn play_beep_and_reboot() {
        let radio = FakeRadioCli::new("pocket", "2.12.4").with_file("/SOUNDS/en/hello.wav", 5);
        let mut c = cli(&radio).unwrap();
        c.play("/SOUNDS/en/hello.wav").unwrap();
        assert!(c.play("/SOUNDS/en/none.wav").is_err());
        c.beep().unwrap();
        c.reboot().unwrap();
        assert_eq!(radio.played(), ["/SOUNDS/en/hello.wav"]);
        assert!(radio.rebooted());
        assert!(radio.log().contains(&"reboot".to_string()));
    }

    #[test]
    fn a_radio_with_the_cli_off_gets_the_hint() {
        let radio = FakeRadioCli::new("pocket", "2.12.4").silent();
        let e = cli(&radio).err().unwrap();
        assert!(format!("{e:#}").contains("set the USB serial port (VCP) to CLI"));
    }

    #[test]
    fn only_plain_card_paths_go_out() {
        for bad in ["SOUNDS/x.wav", "/a/../b", "/a;reboot", "/a\nreboot", ""] {
            assert!(check_path(bad).is_err(), "{bad:?}");
        }
        assert!(check_path("/SOUNDS/en/SYSTEM/hello world.wav").is_ok());
    }
}
