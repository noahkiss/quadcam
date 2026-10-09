//! `quadcam-cli gear <command>`: Gear on the same core as the app. One file per area as the
//! Gear packages add them (`backup.rs`, `changes.rs`, ...), each with its subcommands and
//! a `run`; this file holds the `gear` command and sends each subcommand to its area.

use anyhow::Result;
use clap::Subcommand;
use quadcam_lib::api::{self, call};
use quadcam_lib::core::Core;
use serde_json::Value;

mod backup;
mod card;
mod changes;
mod fc;
mod flights;
mod map;
mod osd;
mod packs;
mod rates;
mod sim;

#[derive(Subcommand)]
pub enum GearCmd {
    /// The gear folder, the Gear settings, and the devices plugged in now.
    Status,
    /// Saved devices: list (default), save a name or aircraft, forget.
    Devices {
        #[command(subcommand)]
        cmd: Option<DevicesCmd>,
    },
    /// A Betaflight flight controller: identify, read, check, board notes, USB timer.
    Fc {
        #[command(subcommand)]
        cmd: fc::FcCmd,
    },
    /// An FC's OSD layout per OSD profile, drawn on its grid and checked for overlaps and
    /// cells off screen. Reads a dump or diff file.
    Osd(osd::OsdArgs),
    /// An FC's rate profiles: names, curves per axis (maximum and centre rates), the
    /// throttle curve. Reads a dump or diff file, a backup, or a saved device's latest backup.
    Rates(rates::RatesArgs),
    /// The sims on this Mac with their rate profiles; with a quad, how each differs from it.
    Sims(rates::SimsArgs),
    /// An EdgeTX card: models, the selected model, the radio clock; `preview` checks edits.
    Card(card::CardArgs),
    /// The switch map: what each radio control does on the radio and the FC, per position.
    /// Reads an EdgeTX card or model file and a Betaflight dump; --live marks the
    /// positions now.
    Map(map::MapArgs),
    /// The radio in USB Joystick mode now: buttons, axes, channel values.
    Radio(map::RadioArgs),
    /// The sim: a radio's calibration, the defaults an aircraft gives it.
    Sim {
        #[command(subcommand)]
        cmd: sim::SimCmd,
    },
    /// Flights from the radio logs: measures, pack, place, clip. `set` and `folders`.
    Flights(flights::FlightsArgs),
    /// The session report for a day, else the last import's days.
    Report(flights::ReportArgs),
    /// The "Pack up" check: packs charged, radio model, card space, backups, cards still in.
    Preflight,
    /// Packs with their history, pack types, the charging notes.
    Packs(packs::PacksArgs),
    /// The crash and repair log: list, `save`, `delete`.
    Crashes(packs::CrashesArgs),
    /// Back up a radio card or an FC (the FC reboots); `show`, `diff` and `pin` a backup.
    Backup(backup::BackupArgs),
    /// Backups, newest first.
    Backups {
        /// One device's.
        #[arg(long)]
        device: Option<String>,
    },
    /// The gear folder's size per device; --prune thins backups; --export writes folders.
    Storage(backup::StorageArgs),
    /// Import an old backup folder: radio card copies, FC diff/dump files, LOGS folders.
    ImportBackups {
        folder: std::path::PathBuf,
        /// The saved device that items no id names go to.
        #[arg(long)]
        device: Option<String>,
        /// Report what it would take; write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Check a card's file system (diskutil verifyVolume, read-only); --log lists past checks.
    CardCheck {
        #[arg(long)]
        device: Option<String>,
        #[arg(long)]
        mount: Option<std::path::PathBuf>,
        #[arg(long)]
        log: bool,
    },
    /// Repair a card whose latest check failed: a backup first, the repair, a check after.
    CardRepair {
        /// The failed check's id.
        #[arg(long)]
        check: String,
        #[arg(long)]
        yes: bool,
    },
    /// Staged changes waiting to be applied (--history adds the rest).
    Changes(changes::ChangesArgs),
    /// Stage FC settings: raw CLI lines (--cli FILE) or --set NAME=VALUE. Writes nothing to
    /// the FC.
    Stage(changes::StageArgs),
    /// Discard a staged change. It stays in the history.
    Discard { change: String },
    /// Stage an FC backup's settings back as a change.
    Restore { backup: String },
    /// Apply a staged change to the FC: `--plan` shows the checks, diff and digest;
    /// `--digest D --yes` backs up, writes, saves, reads back and verifies.
    Apply(changes::ApplyArgs),
    /// Stop a running backup or card check (its handle from `gear status`).
    Stop { handle: String },
}

#[derive(Subcommand)]
pub enum DevicesCmd {
    /// List the saved devices.
    List,
    /// Name a device or link it to an aircraft profile. A new device must be plugged in.
    Save {
        /// The device id (from `gear status` or `gear devices`).
        id: String,
        /// The device's name ("" for none).
        #[arg(long)]
        name: Option<String>,
        /// An aircraft profile name ("" to unlink).
        #[arg(long)]
        aircraft: Option<String>,
    },
    /// Remove a device from the list. Its backups stay.
    Forget {
        /// The device id.
        id: String,
    },
}

pub fn run(core: &Core, cmd: GearCmd) -> Result<Value> {
    Ok(match cmd {
        GearCmd::Fc { cmd } => fc::run(core, cmd)?,
        GearCmd::Osd(a) => osd::run(core, a)?,
        GearCmd::Rates(a) => rates::rates(core, a)?,
        GearCmd::Sims(a) => rates::sims(core, a)?,
        GearCmd::Status => serde_json::to_value(call::gear_status(core)?)?,
        GearCmd::Card(a) => card::run(core, a)?,
        GearCmd::Map(a) => map::run(core, a)?,
        GearCmd::Radio(a) => map::radio(core, a)?,
        GearCmd::Sim { cmd } => sim::run(core, cmd)?,
        GearCmd::Flights(a) => flights::flights(core, a)?,
        GearCmd::Report(a) => flights::report(core, a)?,
        GearCmd::Preflight => flights::preflight(core)?,
        GearCmd::Packs(a) => packs::packs(core, a)?,
        GearCmd::Crashes(a) => packs::crashes(core, a)?,
        GearCmd::Backup(a) => backup::backup(core, a)?,
        GearCmd::Backups { device } => backup::backups(core, device)?,
        GearCmd::Storage(a) => backup::storage(core, a)?,
        GearCmd::ImportBackups {
            folder,
            device,
            dry_run,
        } => backup::import(core, folder, device, dry_run)?,
        GearCmd::CardCheck { device, mount, log } => backup::card_check(core, device, mount, log)?,
        GearCmd::CardRepair { check, yes } => backup::card_repair(core, check, yes)?,
        GearCmd::Changes(a) => changes::changes(core, a)?,
        GearCmd::Stage(a) => changes::stage(core, a)?,
        GearCmd::Discard { change } => changes::discard(core, change)?,
        GearCmd::Restore { backup } => changes::restore(core, backup)?,
        GearCmd::Apply(a) => changes::apply(core, a)?,
        GearCmd::Stop { handle } => backup::stop(core, handle)?,
        GearCmd::Devices { cmd } => match cmd.unwrap_or(DevicesCmd::List) {
            DevicesCmd::List => serde_json::to_value(call::gear_devices(core)?)?,
            DevicesCmd::Save { id, name, aircraft } => serde_json::to_value(
                call::gear_device_save(core, api::DeviceSaveParams { id, name, aircraft })?,
            )?,
            DevicesCmd::Forget { id } => {
                serde_json::to_value(call::gear_device_forget(core, api::IdParams { id })?)?
            }
        },
    })
}
