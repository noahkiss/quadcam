//! The Betaflight link (design 6.1, 6.2): the CLI session and its write engine (`cli`),
//! MSP framing and identity (`msp`), `dump all` / `diff all` parsing (`dump`), the
//! simulator (`fake`), and what QuadCam knows about boards (`boards`).
//!
//! Every function here takes a port, opens it, does one thing and drops the link before
//! it returns: QuadCam never holds an FC port idle. The `Core` methods in `core/fc.rs`
//! wrap them as jobs (a hold on the port, one cue at the end).
//!
//! CLI and MSP never run at once on a port: entering the CLI ends MSP, and leaving the
//! CLI reboots the FC. So a job is either MSP only (identify, the battery probe, no
//! reboot) or CLI (read, run; the FC reboots at the end).

pub mod boards;
pub mod cli;
pub mod dump;
pub mod fake;
pub mod msp;

use super::compat::{check_writable, Product};
use super::model::{device_id, DeviceKind, Identity, Refusal};
use super::serial::Ports;
use anyhow::{bail, Result};
use cli::{CliSession, Reply, Timing, BAUD};
use serde::{Deserialize, Serialize};
use specta::Type;

/// Where an FC's device id comes from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum IdSource {
    /// The MCU's unique id (MSP_UID, or `mcu_id` in a dump: the same value).
    McuUid,
    /// The board name and the USB serial number, when the FC gives no UID.
    BoardSerial,
}

/// What QuadCam knows about an FC after talking to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct FcInfo {
    pub port: String,
    /// `fc-<xxh64>`: a hash of the MCU id. None when the FC gave nothing stable.
    pub id: Option<String>,
    pub id_source: Option<IdSource>,
    pub identity: Identity,
    /// The MSP API version (`1.47`), when MSP answered.
    pub msp_api: Option<String>,
    /// Why QuadCam reads this FC but does not write it; None when writes are proven.
    pub read_only: Option<Refusal>,
    /// Known issues of this board and build (`boards`).
    pub notes: Vec<boards::BoardNote>,
    /// Minutes it may run on USB with a battery before "Unplug now" (the board's own, or
    /// the setting).
    pub usb_board_minutes: Option<u32>,
}

/// The raw value an FC's id hashes, and where it came from. The UID wins; all zeros (a
/// scrubbed dump) is no UID.
pub fn id_source(
    uid: Option<&str>,
    board: Option<&str>,
    usb_serial: Option<&str>,
) -> Option<(String, IdSource)> {
    if let Some(u) = uid.filter(|u| !u.is_empty() && u.chars().any(|c| c != '0')) {
        return Some((
            format!("bf-uid:{}", u.to_ascii_lowercase()),
            IdSource::McuUid,
        ));
    }
    match (board, usb_serial.filter(|s| !s.trim().is_empty())) {
        (Some(b), Some(s)) => Some((
            format!("bf-board:{}:{}", b.to_ascii_uppercase(), s.trim()),
            IdSource::BoardSerial,
        )),
        _ => None,
    }
}

/// Fills an `FcInfo` from what was read.
pub fn info(
    port: &str,
    identity: Identity,
    uid: Option<&str>,
    usb_serial: Option<&str>,
    msp_api: Option<String>,
) -> FcInfo {
    let src = id_source(uid, identity.board.as_deref(), usb_serial);
    FcInfo {
        port: port.to_string(),
        id: src.as_ref().map(|(raw, _)| device_id(DeviceKind::Fc, raw)),
        id_source: src.map(|(_, s)| s),
        read_only: writable(&identity).err(),
        notes: boards::notes(identity.board.as_deref(), identity.version.as_deref()),
        usb_board_minutes: boards::usb_minutes(identity.board.as_deref()),
        identity,
        msp_api,
    }
}

/// The write guard for an FC: Betaflight, on a board and build `compat` lists.
pub fn writable(id: &Identity) -> std::result::Result<(), Refusal> {
    if let Some(f) = id.firmware.as_deref().filter(|f| *f != "Betaflight") {
        return Err(Refusal::new(
            super::model::RefusalCode::UnknownVersion,
            format!("{f} is not Betaflight; QuadCam reads it but does not write it."),
        ));
    }
    check_writable(
        Product::Betaflight,
        id.board.as_deref(),
        id.version.as_deref(),
    )
}

/// Identity over MSP: no CLI, no reboot. Opens the port, reads, drops it.
pub fn identify(ports: &dyn Ports, port: &str, timing: Timing) -> Result<FcInfo> {
    let usb_serial = ports
        .list()
        .into_iter()
        .find(|p| p.port == port)
        .and_then(|p| p.serial_number);
    let mut link = ports.open(port, BAUD)?;
    let m = msp::read_identity(link.as_mut(), timing.msp)?;
    drop(link);
    let identity = Identity {
        board: m
            .board_name
            .clone()
            .or(Some(m.board_id.clone()))
            .filter(|b| !b.is_empty()),
        firmware: Some(msp::variant_name(&m.variant).to_string()),
        version: Some(m.version.clone()),
        build: m.build.clone(),
        target: Some(m.board_id.clone()).filter(|b| !b.is_empty()),
    };
    Ok(info(
        port,
        identity,
        m.uid.as_deref(),
        usb_serial.as_deref(),
        Some(m.api),
    ))
}

/// The battery voltage over MSP (0 with none). Opens the port, reads, drops it.
pub fn battery_volts(ports: &dyn Ports, port: &str, timing: Timing) -> Result<f32> {
    let mut link = ports.open(port, BAUD)?;
    let p = msp::call(link.as_mut(), msp::MSP_ANALOG, timing.msp)?;
    msp::parse_analog_volts(&p).ok_or_else(|| anyhow::anyhow!("short MSP_ANALOG reply"))
}

/// The channel values the FC receives (µs, CH1 first) over MSP. Opens the port, reads,
/// drops it.
pub fn rc_channels(ports: &dyn Ports, port: &str, timing: Timing) -> Result<Vec<u16>> {
    let mut link = ports.open(port, BAUD)?;
    let p = msp::call(link.as_mut(), msp::MSP_RC, timing.msp)?;
    Ok(msp::parse_rc(&p))
}

/// A battery counts as in above this voltage (USB alone reads near 0).
pub const BATTERY_IN_VOLTS: f32 = 1.0;

/// CLI commands a read may send: none of them changes the FC. `exit` always follows.
pub fn read_only_command(c: &str) -> bool {
    let w: Vec<&str> = c.split_whitespace().collect();
    match w.as_slice() {
        ["version"] | ["status"] | ["get", _] => true,
        ["diff" | "dump"] => true,
        ["diff" | "dump", what] => {
            matches!(
                *what,
                "all" | "master" | "profile" | "rates" | "hardware" | "defaults"
            )
        }
        _ => false,
    }
}

/// The commands of a backup, in order (design 6.1).
pub const BACKUP: &[&str] = &["version", "status", "diff all", "dump all"];

/// What a CLI read gave back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct FcRead {
    pub info: FcInfo,
    /// Each command and its answer.
    pub replies: Vec<Reply>,
}

impl FcRead {
    /// The answer to `command`, in the file form a backup stores: the command, then the
    /// answer (`diff all\n# version\n...`).
    pub fn file(&self, command: &str) -> Option<String> {
        self.replies
            .iter()
            .find(|r| r.line == command)
            .map(|r| format!("{command}\n{}\n", r.text))
    }
}

/// Reads through the CLI: enter, each command, `exit` (the FC reboots). The port is
/// dropped before it returns. Refuses a command that is not read-only.
pub fn read(ports: &dyn Ports, port: &str, commands: &[String], timing: Timing) -> Result<FcRead> {
    if let Some(c) = commands.iter().find(|c| !read_only_command(c)) {
        bail!(
            "Refused: {c:?} is not a read; a read sends version, status, get, diff or dump only."
        );
    }
    let usb_serial = ports
        .list()
        .into_iter()
        .find(|p| p.port == port)
        .and_then(|p| p.serial_number);
    let (mut s, _) = CliSession::enter(ports.open(port, BAUD)?, timing)?;
    let mut replies = Vec::new();
    let mut fail = None;
    for c in commands {
        match s.command(c) {
            Ok(r) => replies.push(r),
            Err(e) => {
                fail = Some(e);
                break;
            }
        }
    }
    s.exit();
    if let Some(e) = fail {
        return Err(e);
    }
    let all: String = replies
        .iter()
        .map(|r| r.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let c = dump::Config::parse(&all);
    let info = info(
        port,
        c.identity(),
        c.mcu_id().as_deref(),
        usb_serial.as_deref(),
        c.version.and_then(|v| v.msp_api),
    );
    Ok(FcRead { info, replies })
}

/// Runs CLI lines with the write engine (`cli::run_lines`). Before the first write it
/// enters the CLI, reads `version` and runs the write guard again on what the FC says
/// now, and checks it is the FC the caller planned for (`expect_id`). The apply engine
/// calls this after its own plan, backup and confirm; nothing else writes an FC.
pub fn run(
    ports: &dyn Ports,
    port: &str,
    expect_id: Option<&str>,
    lines: &[String],
    timing: Timing,
) -> Result<(FcInfo, cli::RunReport)> {
    run_with(ports, port, expect_id, lines, timing, &[], &mut |_| Ok(()))
}

/// `run`, with two additions for the apply engine. `precheck` runs in the open session
/// after the identity guard and before the first line is sent: an `Err` ends the session
/// with `exit` (nothing is written). `after` names read-only commands read next to the
/// after dump (`cli::run_lines_with`). The port may still be rebooting from an earlier
/// job: this waits for it.
pub fn run_with(
    ports: &dyn Ports,
    port: &str,
    expect_id: Option<&str>,
    lines: &[String],
    timing: Timing,
    after: &[&str],
    precheck: &mut dyn FnMut(&mut CliSession) -> Result<()>,
) -> Result<(FcInfo, cli::RunReport)> {
    let usb_serial = ports
        .list()
        .into_iter()
        .find(|p| p.port == port)
        .and_then(|p| p.serial_number);
    let (mut s, _) = CliSession::enter(cli::wait_for_port(ports, port, timing)?, timing)?;
    let checked = (|| -> Result<FcInfo> {
        let v = s.command("version")?;
        // `mcu_id` prints the MCU id, the value MSP_UID gives (and the id hashes).
        let mut text = v.text.clone();
        if let Some(m) = s.command("mcu_id").ok().filter(|m| !m.error) {
            text.push('\n');
            text.push_str(&m.text);
        }
        let c = dump::Config::parse(&text);
        let uid = c.mcu_id();
        let fc = info(
            port,
            c.identity(),
            uid.as_deref(),
            usb_serial.as_deref(),
            None,
        );
        writable(&fc.identity)?;
        if let Some(want) = expect_id {
            if fc.id.as_deref() != Some(want) {
                return Err(Refusal::new(
                    super::model::RefusalCode::DeviceChanged,
                    "This is not the FC the change was planned for.",
                )
                .into());
            }
        }
        precheck(&mut s)?;
        Ok(fc)
    })();
    match checked {
        Ok(fc) => Ok((fc, cli::run_lines_with(s, ports, lines, timing, after)?)),
        Err(e) => {
            s.exit();
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        let (raw, s) = id_source(Some("002F00383435"), None, None).unwrap();
        assert_eq!((raw.as_str(), s), ("bf-uid:002f00383435", IdSource::McuUid));
        assert!(id_source(Some("000000"), None, None).is_none());
        let (raw, s) = id_source(Some("0000"), Some("betafpvg473"), Some("3A0F")).unwrap();
        assert_eq!(
            (raw.as_str(), s),
            ("bf-board:BETAFPVG473:3A0F", IdSource::BoardSerial)
        );
        assert!(id_source(None, Some("X"), None).is_none());
    }

    #[test]
    fn read_only_commands() {
        for c in [
            "version",
            "status",
            "diff all",
            "dump all",
            "get osd_ah_pos",
            "dump",
            "diff rates",
        ] {
            assert!(read_only_command(c), "{c}");
        }
        for c in [
            "save",
            "set x = 1",
            "defaults",
            "dump all now",
            "exit",
            "diff something",
        ] {
            assert!(!read_only_command(c), "{c}");
        }
    }

    #[test]
    fn the_guard_is_board_and_build() {
        let id = |b: &str, v: &str| Identity {
            board: Some(b.into()),
            firmware: Some("Betaflight".into()),
            version: Some(v.into()),
            ..Default::default()
        };
        assert!(writable(&id("BETAFPVG473_V2", "2026.6.0-alpha")).is_ok());
        assert!(writable(&id("BETAFPVG473", "2025.12.5-alpha")).is_ok());
        // The right build on the other board, and another board: read only.
        let e = writable(&id("BETAFPVG473", "2026.6.0-alpha")).unwrap_err();
        assert_eq!(e.code, super::super::model::RefusalCode::UnknownVersion);
        let e = writable(&id("BETAFPVF411", "2026.6.0-alpha")).unwrap_err();
        assert_eq!(e.code, super::super::model::RefusalCode::UnknownBoard);
        assert_eq!(e.reason, "Board BETAFPVF411 is not proven.");
        let mut inav = id("BETAFPVG473", "2025.12.5");
        inav.firmware = Some("INAV".into());
        assert!(writable(&inav).is_err());
    }
}
