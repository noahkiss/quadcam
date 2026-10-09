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
mod firmware;
mod flights;
mod map;
mod model;
mod osd;
mod packs;
mod rates;
mod sim;
mod voice;

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
    /// Stages OSD moves, toggles or a profile copy for an FC (one "OSD layout" change).
    OsdEdit(osd::OsdEditArgs),
    /// An FC's rate profiles: names, curves per axis (maximum and centre rates), the
    /// throttle curve. Reads a dump or diff file, a backup, or a saved device's latest backup.
    Rates(rates::RatesArgs),
    /// The sims on this Mac with their rate profiles; with a quad, how each differs from it.
    Sims(rates::SimsArgs),
    /// A radio's model for the editors: timers, value screens, logging, alarms, callouts and
    /// the checklist, from the mounted card or the latest backup, with staged edits on top.
    Model(model::ModelArgs),
    /// Stages model edits for a radio (one "Model edits" change); nothing is written to the card.
    ModelEdit(model::ModelEditArgs),
    /// The radio's voice: lines and packs. `render`, `install`, `edit`, `choose` and
    /// `build-pack` are its subcommands.
    Voice(voice::VoiceArgs),
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
    /// Stage a backup's settings back as a change. An FC backup restores its `dump all`; a
    /// radio card backup needs the files to put back (`--path`, repeat for more).
    Restore {
        backup: String,
        #[arg(long = "path")]
        paths: Vec<String>,
    },
    /// Change a staged change's status (draft, ready, try, read_first), title, note or order.
    Update(changes::UpdateArgs),
    /// Keep an applied Try change.
    Keep { change: String },
    /// Stage a restore of the backup an applied change took. The change becomes Reverted
    /// when the restore verifies.
    Revert { change: String },
    /// Copy settings between quads: `--plan` shows the checks and the diff; without it the
    /// settings are staged as one change for the target FC.
    Copy(changes::CopyArgs),
    /// Mount an unmounted radio card to browse it; it unmounts after --minutes (10).
    CardMount {
        /// The card's device id.
        device: String,
        #[arg(long)]
        minutes: Option<u32>,
    },
    /// Unmount a radio card (Done).
    CardUnmount {
        /// The card's device id.
        device: String,
    },
    /// Apply a staged change to the FC: `--plan` shows the checks, diff and digest;
    /// `--digest D --yes` backs up, writes, saves, reads back and verifies.
    Apply(changes::ApplyArgs),
    /// Stop a running backup or card check (its handle from `gear status`).
    Stop { handle: String },
    /// Firmware: each saved device against the newest release (--check reads the network).
    /// `--plan --device ID [--version V] [--splash IMAGE]` shows an EdgeTX flash with its
    /// checks and digest; `--digest D --yes` flashes the radio in DFU mode.
    Firmware(firmware::FirmwareArgs),
    /// A PNG as the radio's splash screen: 128 x 64, one bit; --out writes the preview.
    Splash(firmware::SplashArgs),
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
        GearCmd::OsdEdit(a) => osd::edit(core, a)?,
        GearCmd::Voice(a) => voice::run(core, a)?,
        GearCmd::Model(a) => model::run(core, a)?,
        GearCmd::ModelEdit(a) => model::edit(core, a)?,
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
        GearCmd::Restore { backup, paths } => changes::restore(core, backup, paths)?,
        GearCmd::Update(a) => changes::update(core, a)?,
        GearCmd::Keep { change } => changes::keep(core, change)?,
        GearCmd::Revert { change } => changes::revert(core, change)?,
        GearCmd::Copy(a) => changes::copy(core, a)?,
        GearCmd::CardMount { device, minutes } => serde_json::to_value(call::gear_card_mount(
            core,
            api::CardMountParams { device, minutes },
        )?)?,
        GearCmd::CardUnmount { device } => serde_json::to_value(call::gear_card_unmount(
            core,
            api::CardMountParams {
                device,
                minutes: None,
            },
        )?)?,
        GearCmd::Apply(a) => changes::apply(core, a)?,
        GearCmd::Stop { handle } => backup::stop(core, handle)?,
        GearCmd::Firmware(a) => firmware::firmware(core, a)?,
        GearCmd::Splash(a) => firmware::splash(core, a)?,
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
