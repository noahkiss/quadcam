//! `FakeFc`: a Betaflight FC simulator for tests and the mock core. Seeded from a
//! `dump all` (a scrubbed fixture or a synthetic one), it answers the CLI (`#`, `set`,
//! `get`, other config lines, `version`, `status`, `diff all`, `dump all`, `profile N`,
//! `save`, `exit`) and the MSP identity and battery messages.
//!
//! - `save` keeps the changes and reboots; `exit` drops them and reboots. A reboot ends
//!   every open link (reads give `PortGone`) and the port refuses opens for
//!   `reboot_opens` tries, then comes back.
//! - Errors on demand: `reject` a line (`###ERROR`), `lose_port_on` a line (an unplug),
//!   `slow_dumps` (a dump in many small reads).
//! - `log` records every CLI line and MSP command received; `opens` counts opens.

use super::dump::{parse_cmd, Cmd, Config};
use super::msp::{self, Direction, Frame, Version};
use crate::gear::model::Section;
use crate::gear::serial::{FakePorts, PortGone, PortInfo, SerialLink};
use anyhow::Result;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Msp,
    Cli,
}

struct State {
    /// The dump's lines as saved, and as changed in this CLI session.
    saved: Vec<String>,
    current: Vec<String>,
    /// The values `diff all` compares with.
    defaults: Config,
    mode: Mode,
    section: Section,
    generation: u64,
    gone_opens: u32,
    reboot_opens: u32,
    uid: [u8; 12],
    /// Centivolts on the battery lead; 0 with no battery.
    vbat_cv: u16,
    reject: Vec<String>,
    lose_port_on: Option<String>,
    dump_chunks: usize,
    log: Vec<String>,
    opens: u32,
    saves: u32,
    exits: u32,
}

/// The simulator. Clones share the FC.
#[derive(Clone)]
pub struct FakeFc {
    state: Arc<Mutex<State>>,
}

impl FakeFc {
    /// An FC whose saved config is `dump` (a `dump all`), with those values as defaults.
    pub fn new(dump: &str) -> Self {
        let lines: Vec<String> = dump
            .lines()
            .map(|l| l.trim_end_matches('\r').to_string())
            .filter(|l| l.trim() != "dump all")
            .collect();
        Self {
            state: Arc::new(Mutex::new(State {
                saved: lines.clone(),
                current: lines,
                defaults: Config::parse(dump),
                mode: Mode::Msp,
                section: Section::Master,
                generation: 0,
                gone_opens: 0,
                reboot_opens: 2,
                uid: [0x11; 12],
                vbat_cv: 0,
                reject: Vec::new(),
                lose_port_on: None,
                dump_chunks: 1,
                log: Vec::new(),
                opens: 0,
                saves: 0,
                exits: 0,
            })),
        }
    }

    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    /// `diff all` compares with these values instead of the seed's.
    pub fn with_defaults(self, dump: &str) -> Self {
        self.st().defaults = Config::parse(dump);
        self
    }
    /// The MCU id MSP_UID reports and `dump all` prints.
    pub fn with_uid(self, uid: [u8; 12]) -> Self {
        self.st().uid = uid;
        self
    }
    /// Opens refused while the FC reboots.
    pub fn with_reboot_opens(self, n: u32) -> Self {
        self.st().reboot_opens = n;
        self
    }
    /// The line gets `###ERROR`.
    pub fn reject(self, line: &str) -> Self {
        self.st().reject.push(line.trim().to_string());
        self
    }
    /// The port goes away when this line arrives, and stays away.
    pub fn lose_port_on(self, line: &str) -> Self {
        self.st().lose_port_on = Some(line.trim().to_string());
        self
    }
    /// `dump all` and `diff all` arrive in this many reads.
    pub fn slow_dumps(self, chunks: usize) -> Self {
        self.st().dump_chunks = chunks.max(1);
        self
    }
    /// The battery: volts on the lead, 0 for none.
    pub fn set_battery(&self, volts: f32) {
        self.st().vbat_cv = (volts * 100.0).round() as u16;
    }

    pub fn log(&self) -> Vec<String> {
        self.st().log.clone()
    }
    pub fn opens(&self) -> u32 {
        self.st().opens
    }
    pub fn saves(&self) -> u32 {
        self.st().saves
    }
    pub fn exits(&self) -> u32 {
        self.st().exits
    }
    /// The saved value of a setting.
    pub fn saved_value(&self, section: Section, name: &str) -> Option<String> {
        Config::parse(&self.st().saved.join("\n"))
            .get(section, name)
            .map(str::to_string)
    }
    /// The saved `dump all` text.
    pub fn saved_dump(&self) -> String {
        let s = self.st();
        render_dump(&s.saved, &s.uid)
    }

    /// The MCU id as the CLI prints it.
    pub fn uid_hex(&self) -> String {
        msp::parse_uid(&self.st().uid).unwrap()
    }

    /// Ports with this FC on `port` (STM32 VCP ids), port locks in `locks`.
    pub fn ports(&self, port: &str, locks: Option<std::path::PathBuf>) -> FakePorts {
        let fc = self.clone();
        let info = PortInfo {
            port: port.into(),
            vid: 0x0483,
            pid: 0x5740,
            serial_number: Some("FAKE0001".into()),
            manufacturer: Some("Betaflight".into()),
            product: Some("STM32 Virtual ComPort".into()),
        };
        let p = FakePorts::new(vec![info]).with_opener(move |port, _| fc.open(port));
        match locks {
            Some(d) => p.with_locks(d),
            None => p,
        }
    }

    /// Opens a link, unless the FC is rebooting.
    pub fn open(&self, port: &str) -> Result<Box<dyn SerialLink>> {
        let mut s = self.st();
        if s.gone_opens > 0 {
            s.gone_opens = s.gone_opens.saturating_sub(1);
            return Err(PortGone(port.to_string()).into());
        }
        s.opens += 1;
        Ok(Box::new(FakeLink {
            port: port.to_string(),
            fc: self.clone(),
            generation: s.generation,
            rx: Vec::new(),
            out: VecDeque::new(),
        }))
    }
}

fn render_dump(lines: &[String], uid: &[u8; 12]) -> String {
    let hex = msp::parse_uid(uid).unwrap();
    let mut out = String::new();
    for l in lines {
        if l.starts_with("mcu_id ") {
            out.push_str(&format!("mcu_id {hex}"));
        } else {
            out.push_str(l);
        }
        out.push_str("\r\n");
    }
    out
}

struct FakeLink {
    port: String,
    fc: FakeFc,
    generation: u64,
    rx: Vec<u8>,
    out: VecDeque<Vec<u8>>,
}

impl FakeLink {
    fn alive(&self) -> bool {
        self.fc.st().generation == self.generation
    }

    fn send(&mut self, s: &str, chunks: usize) {
        let b = s.as_bytes();
        let n = chunks.max(1);
        let size = b.len().div_ceil(n).max(1);
        for c in b.chunks(size) {
            self.out.push_back(c.to_vec());
        }
    }

    /// Ends the session: unsaved changes go, every link dies, opens fail for a while.
    fn reboot(&mut self, keep: bool, opens: Option<u32>) {
        let mut s = self.fc.st();
        if keep {
            s.saved = s.current.clone();
        } else {
            s.current = s.saved.clone();
        }
        s.generation += 1;
        s.mode = Mode::Msp;
        s.section = Section::Master;
        s.gone_opens = opens.unwrap_or(s.reboot_opens);
    }

    fn cli_line(&mut self, line: &str) {
        let line = line.trim().to_string();
        let mut s = self.fc.st();
        s.log.push(line.clone());
        if s.lose_port_on.as_deref() == Some(line.as_str()) {
            drop(s);
            self.reboot(false, Some(u32::MAX));
            return;
        }
        let chunks = s.dump_chunks;
        let echo = format!("{line}\r\n");
        if s.reject.contains(&line) {
            drop(s);
            self.send(
                &format!("{echo}###ERROR IN {line}: INVALID###\r\n\r\n# "),
                1,
            );
            return;
        }
        let version = s
            .saved
            .iter()
            .find(|l| l.starts_with("# Betaflight"))
            .cloned()
            .unwrap_or_default();
        let board = Config::parse(&s.saved.join("\n"))
            .board_name()
            .unwrap_or_default();
        match line.as_str() {
            "save" => {
                s.saves += 1;
                drop(s);
                self.send(&format!("{echo}Saving\r\nRebooting"), 1);
                self.reboot(true, None);
                return;
            }
            "exit" => {
                s.exits += 1;
                drop(s);
                self.send(
                    &format!("{echo}\r\nLeaving CLI mode, unsaved changes lost.\r\n"),
                    1,
                );
                self.reboot(false, None);
                return;
            }
            "version" => {
                let t = format!("{echo}{version}\r\n# board: manufacturer_id: BEFH, board_name: {board}\r\n\r\n# ");
                drop(s);
                self.send(&t, 1);
                return;
            }
            "status" => {
                let t = format!(
                    "{echo}MCU G473 Clock=170MHz, Vref=3.30V, Core temp=40degC\r\nVoltage: {} * 0.01V\r\nCPU:2%, cycle time: 125\r\n\r\n# ",
                    s.vbat_cv
                );
                drop(s);
                self.send(&t, 1);
                return;
            }
            "mcu_id" => {
                let t = format!("{echo}mcu_id {}\r\n\r\n# ", msp::parse_uid(&s.uid).unwrap());
                drop(s);
                self.send(&t, 1);
                return;
            }
            "dump all" => {
                let t = format!("{echo}{}\r\n# ", render_dump(&s.current, &s.uid));
                drop(s);
                self.send(&t, chunks);
                return;
            }
            "diff all" => {
                let t = format!("{echo}{}\r\n# ", diff_all(&s, &version, &board));
                drop(s);
                self.send(&t, chunks);
                return;
            }
            _ => {}
        }
        let answer = match parse_cmd(&line) {
            Cmd::Select(sec) => {
                s.section = sec;
                String::new()
            }
            Cmd::Set { name, value } => {
                let sec = s.section;
                match find_set(&s.current, sec, &name) {
                    Some(i) => {
                        s.current[i] = format!("set {name} = {value}");
                        format!("{name} set to {value}\r\n")
                    }
                    None => format!("###ERROR IN set: INVALID NAME: {name}###\r\n"),
                }
            }
            Cmd::Other { verb, key } if verb == "get" => {
                let name = key.trim_start_matches("get").trim().to_string();
                let name = line.split_whitespace().nth(1).unwrap_or(&name).to_string();
                let c = Config::parse(&s.current.join("\n"));
                match c.get(s.section, &name) {
                    Some(v) => format!("{name} = {v}\r\nAllowed range: 0 - 65535\r\n"),
                    None => format!("###ERROR IN get: INVALID NAME: {name}###\r\n"),
                }
            }
            Cmd::Other { key, .. } => {
                let c = Config::parse(&s.current.join("\n"));
                let idx = c
                    .lines
                    .iter()
                    .position(|l| matches!(&l.cmd, Some(Cmd::Other { key: k, .. }) if *k == key));
                match idx {
                    Some(i) => {
                        s.current[i] = line.clone();
                        String::new()
                    }
                    None => format!("###ERROR IN {line}: INVALID NAME###\r\n"),
                }
            }
            Cmd::Control => String::new(),
        };
        drop(s);
        self.send(&format!("{echo}{answer}\r\n# "), 1);
    }

    fn msp_frame(&mut self, f: Frame) {
        let s = self.fc.st();
        let payload: Option<Vec<u8>> = match f.cmd {
            msp::MSP_API_VERSION => Some(vec![0, 1, 47]),
            msp::MSP_FC_VARIANT => Some(b"BTFL".to_vec()),
            msp::MSP_FC_VERSION => {
                let c = Config::parse(&s.saved.join("\n"));
                let v = c.version.map(|v| v.version).unwrap_or_default();
                let n: Vec<u8> = v
                    .split(['.', '-'])
                    .take(3)
                    .map(|x| (x.parse::<u32>().unwrap_or(0) % 256) as u8)
                    .collect();
                let mut p = n;
                p.resize(3, 0);
                p.push(v.len() as u8);
                p.extend_from_slice(v.as_bytes());
                Some(p)
            }
            msp::MSP_BOARD_INFO => {
                let c = Config::parse(&s.saved.join("\n"));
                let board = c.board_name().unwrap_or_default();
                let target = c.version.and_then(|v| v.target).unwrap_or_default();
                let mut p = format!("{target:<4}").into_bytes()[..4].to_vec();
                p.extend_from_slice(&[0, 0, 0, 0]);
                for x in [target.as_str(), board.as_str(), "BEFH"] {
                    p.push(x.len() as u8);
                    p.extend_from_slice(x.as_bytes());
                }
                Some(p)
            }
            msp::MSP_BUILD_INFO => Some(b"Jan  1 202600:00:00abcdef0".to_vec()),
            msp::MSP_UID => Some(s.uid.to_vec()),
            msp::MSP_ANALOG => {
                let mut p = vec![(s.vbat_cv / 10).min(255) as u8, 0, 0, 0, 0, 0, 0];
                p.extend_from_slice(&s.vbat_cv.to_le_bytes());
                Some(p)
            }
            _ => None,
        };
        drop(s);
        self.fc.st().log.push(format!("msp {}", f.cmd));
        let reply = Frame {
            version: f.version,
            direction: if payload.is_some() {
                Direction::Reply
            } else {
                Direction::Error
            },
            cmd: f.cmd,
            payload: payload.unwrap_or_default(),
        };
        let bytes = msp::encode(&reply).unwrap_or_else(|_| {
            msp::encode(&Frame {
                version: Version::V2,
                ..reply
            })
            .unwrap()
        });
        self.out.push_back(bytes);
    }
}

fn find_set(lines: &[String], section: Section, name: &str) -> Option<usize> {
    let c = Config::parse(&lines.join("\n"));
    let hit = |want: Section| {
        c.lines.iter().position(|l| {
            l.section == want && matches!(&l.cmd, Some(Cmd::Set { name: n, .. }) if n == name)
        })
    };
    hit(section).or_else(|| hit(Section::Master))
}

fn diff_all(s: &State, version: &str, board: &str) -> String {
    let c = Config::parse(&s.current.join("\n"));
    let mut out = format!(
        "# version\r\n{version}\r\n\r\nbatch start\r\n\r\ndefaults nosave\r\n\r\nboard_name {board}\r\n"
    );
    let mut section = Section::Master;
    out.push_str("\r\n# master\r\n");
    for (sec, name, value) in c.sets() {
        if s.defaults.get(sec, name) == Some(value) {
            continue;
        }
        if sec != section {
            if let Some(l) = sec.select_line() {
                out.push_str(&format!("\r\n{l}\r\n"));
            }
            section = sec;
        }
        out.push_str(&format!("set {name} = {value}\r\n"));
    }
    out.push_str("\r\nbatch end");
    out
}

impl SerialLink for FakeLink {
    fn port(&self) -> &str {
        &self.port
    }
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        if !self.alive() {
            return Err(PortGone(self.port.clone()).into());
        }
        let mode = self.fc.st().mode;
        match mode {
            Mode::Msp => {
                if data == b"#" {
                    self.fc.st().mode = Mode::Cli;
                    self.send(
                        "\r\nEntering CLI Mode, type 'exit' to return, or 'help'\r\n\r\n# ",
                        1,
                    );
                    return Ok(());
                }
                self.rx.extend_from_slice(data);
                let (frames, rest) = msp::decode_all(&self.rx);
                self.rx = rest;
                for f in frames {
                    if f.direction == Direction::Request {
                        self.msp_frame(f);
                    }
                }
            }
            Mode::Cli => {
                self.rx.extend_from_slice(data);
                while let Some(i) = self.rx.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = self.rx.drain(..=i).collect();
                    let line = String::from_utf8_lossy(&line).to_string();
                    self.cli_line(&line);
                    if !self.alive() {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
    fn read_some(&mut self, _timeout: Duration) -> Result<Vec<u8>> {
        if let Some(c) = self.out.pop_front() {
            return Ok(c);
        }
        if !self.alive() {
            return Err(PortGone(self.port.clone()).into());
        }
        std::thread::yield_now();
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::super::cli::{run_lines, CliSession, Timing};
    use super::*;
    use crate::gear::serial::Ports;

    const DUMP: &str = include_str!("../../../tests/fixtures/bf/g473-2025.12.5.dump_all.txt");
    const AFTER: &str =
        include_str!("../../../tests/fixtures/bf/g473-2025.12.5.after.dump_all.txt");
    const PORT: &str = "/dev/cu.usbmodemFAKE1";

    fn battery_lines() -> Vec<String> {
        // The difference between the two fixtures, as a CLI file.
        let before = Config::parse(DUMP);
        let after = Config::parse(AFTER);
        after
            .sets()
            .filter(|(s, n, v)| before.get(*s, n) != Some(*v))
            .map(|(_, n, v)| format!("set {n} = {v}"))
            .collect()
    }

    #[test]
    fn a_run_with_save_waits_through_the_reboot_and_matches_the_real_after_dump() {
        let fc = FakeFc::new(DUMP).with_reboot_opens(3);
        let ports = fc.ports(PORT, None);
        let lines = battery_lines();
        assert_eq!(lines.len(), 9, "{lines:?}");
        let (s, _) = CliSession::enter(ports.open(PORT, 115200).unwrap(), Timing::fast()).unwrap();
        let r = run_lines(s, &ports, &lines, Timing::fast()).unwrap();
        assert!(r.saved && r.failed.is_none());
        assert!(r.verify.is_empty(), "{:?}", r.verify);
        assert_eq!(fc.saves(), 1);
        assert_eq!(fc.exits(), 1, "the read back exits");
        // The FC's saved dump is the real after dump, line for line.
        let got = fc.saved_dump().replace("\r\n", "\n");
        let want = AFTER.replace(&"0".repeat(24), &fc.uid_hex());
        let want: Vec<&str> = want.lines().filter(|l| *l != "dump all").collect();
        assert_eq!(got.lines().collect::<Vec<_>>(), want);
    }

    #[test]
    fn a_run_that_hits_an_error_discards() {
        let fc = FakeFc::new(DUMP).reject("set osd_cap_alarm = 400");
        let ports = fc.ports(PORT, None);
        let lines = battery_lines();
        let (s, _) = CliSession::enter(ports.open(PORT, 115200).unwrap(), Timing::fast()).unwrap();
        let r = run_lines(s, &ports, &lines, Timing::fast()).unwrap();
        assert!(!r.saved);
        assert_eq!(r.failed.as_ref().unwrap().line, "set osd_cap_alarm = 400");
        assert!(r.sent.len() < lines.len());
        assert_eq!((fc.saves(), fc.exits()), (0, 1));
        // Nothing changed on the FC, including the lines before the error.
        assert_eq!(
            fc.saved_value(Section::Master, "vbat_min_cell_voltage")
                .as_deref(),
            Some("330")
        );
        // A name the FC does not know is an error too.
        let fc = FakeFc::new(DUMP);
        let ports = fc.ports(PORT, None);
        let (s, _) = CliSession::enter(ports.open(PORT, 115200).unwrap(), Timing::fast()).unwrap();
        let r = run_lines(s, &ports, &["set no_such = 1".into()], Timing::fast()).unwrap();
        assert!(r.failed.is_some() && !r.saved);
    }

    #[test]
    fn a_lost_port_saves_nothing_and_forbidden_lines_refuse() {
        let fc = FakeFc::new(DUMP).lose_port_on("set osd_ah_pos = 2233");
        let ports = fc.ports(PORT, None);
        let (s, _) = CliSession::enter(ports.open(PORT, 115200).unwrap(), Timing::fast()).unwrap();
        let e = run_lines(s, &ports, &battery_lines(), Timing::fast()).unwrap_err();
        assert!(format!("{e:#}").contains("Nothing was saved"), "{e:#}");
        assert_eq!(fc.saves(), 0);
        let fc = FakeFc::new(DUMP);
        let ports = fc.ports(PORT, None);
        let (s, _) = CliSession::enter(ports.open(PORT, 115200).unwrap(), Timing::fast()).unwrap();
        let e = run_lines(s, &ports, &["defaults nosave".into()], Timing::fast()).unwrap_err();
        assert!(format!("{e}").starts_with("Refused"));
        assert!(fc.log().is_empty(), "nothing was sent");
    }

    #[test]
    fn a_slow_dump_arrives_whole() {
        let fc = FakeFc::new(DUMP).slow_dumps(200);
        let ports = fc.ports(PORT, None);
        let (mut s, _) =
            CliSession::enter(ports.open(PORT, 115200).unwrap(), Timing::fast()).unwrap();
        let d = s.command("dump all").unwrap();
        s.exit();
        assert_eq!(
            Config::parse(&d.text).sets().count(),
            Config::parse(DUMP).sets().count()
        );
        assert!(d.text.starts_with("# version"), "{}", &d.text[..40]);
    }

    #[test]
    fn diff_all_shows_only_changed_values_and_get_reads() {
        let fc = FakeFc::new(DUMP);
        let ports = fc.ports(PORT, None);
        let (mut s, _) =
            CliSession::enter(ports.open(PORT, 115200).unwrap(), Timing::fast()).unwrap();
        s.command("rateprofile 2").unwrap();
        s.command("set roll_rc_rate = 7").unwrap();
        let d = s.command("diff all").unwrap();
        assert!(
            d.text.contains("rateprofile 2\nset roll_rc_rate = 7"),
            "{}",
            d.text
        );
        assert_eq!(Config::parse(&d.text).sets().count(), 1);
        let g = s.command("get osd_ah_pos").unwrap();
        assert!(g.text.starts_with("osd_ah_pos = 4281"), "{}", g.text);
        s.exit();
        // exit dropped the change.
        assert_eq!(
            fc.saved_value(Section::RateProfile(2), "roll_rc_rate"),
            Config::parse(DUMP)
                .get(Section::RateProfile(2), "roll_rc_rate")
                .map(str::to_string)
        );
    }
}
