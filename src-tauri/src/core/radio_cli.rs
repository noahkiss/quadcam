//! `Core`'s jobs on a radio's USB serial port (the EdgeTX CLI, `gear/edgetx/cli.rs`):
//! identify the running firmware, list a folder, play a sound, beep, restart the radio, and
//! check the card's files against a backup with `ls`. Nothing here writes a file: the CLI
//! cannot move file contents.

use super::Core;
use crate::gear::edgetx::cli::{self, CliEntry, RadioCli, RadioInfo};
use crate::gear::model::{Connected, DeviceKind, Link, Refusal, RefusalCode};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// What `gear_radio_cli` does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum RadioCliAction {
    /// `ver`: the board and version the radio runs now.
    #[default]
    Identify,
    /// `ls` of the folder in `path`.
    Ls,
    /// `play` the sound file in `path` on the radio's speaker.
    Play,
    Beep,
    /// Restart the radio (needs `confirm`).
    Reboot,
    /// `ls` each folder of a saved radio's latest backup and report the files the radio
    /// lacks or holds at another size. `device` names the radio.
    Verify,
}

/// `gear_radio_cli`: the serial port (omit when one radio is on serial), the action and its
/// arguments.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct RadioCliParams {
    #[serde(default)]
    pub port: Option<String>,
    #[serde(default)]
    pub action: RadioCliAction,
    /// For ls and play: a card path (`/SOUNDS/en/hello.wav`).
    #[serde(default)]
    pub path: Option<String>,
    /// For verify: the saved radio whose latest backup to compare with.
    #[serde(default)]
    pub device: Option<String>,
    /// For reboot: must be true.
    #[serde(default)]
    pub confirm: bool,
}

/// A saved radio that a serial radio may be.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct RadioMatch {
    pub id: String,
    pub name: String,
}

/// The `ls` comparison with a backup.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct RadioVerify {
    /// The backup compared with.
    pub backup: String,
    pub checked: u32,
    /// Backup files the radio's `ls` does not show.
    pub missing: Vec<String>,
    /// Files whose size `ls` shows and differs from the backup's.
    pub differ: Vec<String>,
    pub ok: bool,
}

/// What a radio CLI job found.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RadioCliReport {
    pub port: String,
    #[serde(default)]
    pub info: Option<RadioInfo>,
    /// Saved radios of the board `info` names.
    #[serde(default)]
    pub candidates: Vec<RadioMatch>,
    #[serde(default)]
    pub entries: Vec<CliEntry>,
    #[serde(default)]
    pub verify: Option<RadioVerify>,
    pub notes: Vec<String>,
}

/// Paths in a backup that `verify` skips: logs grow, and QuadCam's own and macOS files are
/// not the radio's.
fn verifiable(path: &str) -> bool {
    !path.starts_with("LOGS/")
        && !path.split('/').any(|p| p.starts_with('.'))
        && !path.starts_with("SCREENSHOTS/")
}

impl Core {
    fn radio_cli_timing(&self) -> cli::Timing {
        let t = self.fc_timing();
        cli::Timing {
            quiet: t.quiet.min(std::time::Duration::from_millis(150)),
            command: t.command,
            reboot: t.exit,
        }
    }

    /// The radio serial ports plugged in now.
    fn radio_serials(&self) -> Vec<Connected> {
        self.gear
            .detect()
            .into_iter()
            .filter(|c| c.kind == DeviceKind::Radio && matches!(c.link, Link::Serial { .. }))
            .collect()
    }

    /// The radio port to use: the one asked for, or the only one.
    fn radio_pick(&self, port: Option<&str>) -> Result<String> {
        let list = self.radio_serials();
        let handles: Vec<String> = list.iter().map(|c| super::link_handle(&c.link)).collect();
        if let Some(p) = port.map(str::trim).filter(|p| !p.is_empty()) {
            if handles.iter().any(|h| h == p) {
                return Ok(p.to_string());
            }
            bail!("No device: no radio on serial port {p}. Plug the radio in and pick USB Serial; check `gear status`.");
        }
        match handles.as_slice() {
            [] => bail!("No device: no radio on a USB serial port. Plug it in and pick USB Serial on the radio."),
            [one] => Ok(one.clone()),
            _ => Err(Refusal::new(
                RefusalCode::SeveralDevices,
                format!("{} radios are on serial; pick one with port: {}.", handles.len(), handles.join(", ")),
            )
            .into()),
        }
    }

    /// The saved radios of a board.
    fn radios_of_board(&self, board: Option<&str>) -> Vec<RadioMatch> {
        let Some(board) = board.filter(|b| !b.is_empty()) else {
            return Vec::new();
        };
        self.gear_store()
            .devices()
            .unwrap_or_default()
            .into_iter()
            .filter(|d| d.kind == DeviceKind::Radio)
            .filter(|d| {
                d.identity
                    .board
                    .as_deref()
                    .is_some_and(|b| b.eq_ignore_ascii_case(board))
            })
            .map(|d| RadioMatch {
                name: d.display_name(),
                id: d.id,
            })
            .collect()
    }

    /// One job on a radio's serial port: identify, ls, play, beep, reboot or verify.
    pub fn gear_radio_cli(&self, p: &RadioCliParams) -> Result<RadioCliReport> {
        if p.action == RadioCliAction::Reboot && !p.confirm {
            bail!("Refused: restarting the radio needs confirm=true.");
        }
        let port = self.radio_pick(p.port.as_deref())?;
        if let Some((_, name)) = (self.gear.holders)(&port).into_iter().next() {
            return Err(Refusal::new(
                RefusalCode::PortBusy,
                format!("{port} is open in {name}. Close it there; QuadCam does not share a port."),
            )
            .into());
        }
        let _hold = self.gear_hold(&port);
        let mut cli = RadioCli::open(self.gear.ports.as_ref(), &port, self.radio_cli_timing())?;
        let mut report = RadioCliReport {
            port: port.clone(),
            info: None,
            candidates: Vec::new(),
            entries: Vec::new(),
            verify: None,
            notes: Vec::new(),
        };
        let path = || {
            p.path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .context("path is required: a card path such as /SOUNDS/en/hello.wav")
        };
        match p.action {
            RadioCliAction::Identify => {
                let info = cli.identify()?;
                report.candidates = self.radios_of_board(info.board.as_deref());
                self.fc_state
                    .lock()
                    .unwrap()
                    .radio
                    .insert(port.clone(), info.clone());
                self.hooks.gear_changed();
                report.info = Some(info);
            }
            RadioCliAction::Ls => report.entries = cli.ls(path()?)?,
            RadioCliAction::Play => {
                let file = path()?;
                cli.play(file)?;
                report.notes.push(format!("Played {file} on the radio."));
            }
            RadioCliAction::Beep => {
                cli.beep()?;
                report.notes.push("The radio beeped.".into());
            }
            RadioCliAction::Reboot => {
                cli.reboot()?;
                self.fc_state.lock().unwrap().radio.remove(&port);
                report
                    .notes
                    .push("The radio restarted. Its serial port is gone until it is back.".into());
            }
            RadioCliAction::Verify => {
                let info = cli.identify()?;
                let device = self.verify_device(p.device.as_deref(), info.board.as_deref())?;
                let backup = self.snapshots().latest(&device).with_context(|| {
                    format!(
                        "The radio {device} has no backup to compare with. Back its card up first."
                    )
                })?;
                let want: Vec<(String, u64)> = backup
                    .files
                    .iter()
                    .filter(|f| verifiable(&f.path))
                    .map(|f| (f.path.clone(), f.size))
                    .collect();
                let (missing, differ) = cli::compare_listing(&mut cli, &want)?;
                report.verify = Some(RadioVerify {
                    backup: backup.id.clone(),
                    checked: want.len() as u32,
                    ok: missing.is_empty() && differ.is_empty(),
                    missing,
                    differ,
                });
                report.info = Some(info);
            }
        }
        Ok(report)
    }

    /// The saved radio `verify` compares with: the one named, else the only saved radio of
    /// the running radio's board.
    fn verify_device(&self, device: Option<&str>, board: Option<&str>) -> Result<String> {
        if let Some(d) = device.map(str::trim).filter(|d| !d.is_empty()) {
            return Ok(d.to_string());
        }
        let c = self.radios_of_board(board);
        match c.as_slice() {
            [one] => Ok(one.id.clone()),
            [] => bail!("No saved radio of this board. Name the radio with device."),
            many => bail!(
                "Several saved radios share this board ({}); name one with device.",
                many.iter()
                    .map(|m| format!("{} ({})", m.name, m.id))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}
