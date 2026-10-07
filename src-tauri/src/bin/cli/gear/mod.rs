//! `quadcam-cli gear <command>`: Gear on the same core as the app. One file per area as the
//! Gear packages add them (`backup.rs`, `changes.rs`, ...), each with its subcommands and
//! a `run`; this file holds the `gear` command and sends each subcommand to its area.

use anyhow::Result;
use clap::Subcommand;
use quadcam_lib::api::{self, call};
use quadcam_lib::core::Core;
use serde_json::Value;

mod card;
mod osd;

#[derive(Subcommand)]
pub enum GearCmd {
    /// The gear folder, the Gear settings, and the devices plugged in now.
    Status,
    /// Saved devices: list (default), save a name or aircraft, forget.
    Devices {
        #[command(subcommand)]
        cmd: Option<DevicesCmd>,
    },
    /// An FC's OSD layout per OSD profile, drawn on its grid and checked for overlaps and
    /// cells off screen. Reads a dump or diff file.
    Osd(osd::OsdArgs),
    /// An EdgeTX card: models, the selected model, the radio clock; `preview` checks edits.
    Card(card::CardArgs),
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
        GearCmd::Osd(a) => osd::run(core, a)?,
        GearCmd::Status => serde_json::to_value(call::gear_status(core)?)?,
        GearCmd::Card(a) => card::run(core, a)?,
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
