//! `quadcam-cli gear blackbox ...`: an FC's blackbox flash (design 7.12). `pull` reads the
//! used bytes, verifies and stores them, and erases only when the `gear_erase_blackbox`
//! setting allows and the checks pass (`--keep` skips the erase for one run). `list` shows stored pulls with their
//! guessed flights, `export` writes one to a folder, `erase` empties the flash by hand.

use anyhow::Result;
use clap::{Subcommand, ValueEnum};
use quadcam_lib::api::{self, call};
use quadcam_lib::core::{Core, PullMode};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Copy, ValueEnum)]
pub enum Mode {
    /// USB disk mode when the gear_blackbox_msc setting is on and the FC has it, else MSP.
    Auto,
    /// MSP only.
    Msp,
    /// USB disk mode only (unproven on real FCs).
    Msc,
}

#[derive(Subcommand)]
pub enum BlackboxCmd {
    /// Pull the FC's blackbox flash: read the used bytes, verify, store.
    Pull {
        /// The FC's port; omit when one FC is plugged in.
        #[arg(long)]
        port: Option<String>,
        /// Keep the flash this run, even when the gear_erase_blackbox setting is on. No flag
        /// turns the erase on; `erase` is its own command.
        #[arg(long)]
        keep: bool,
        #[arg(long, value_enum)]
        mode: Option<Mode>,
        /// Pull although the USB heat timer is short. An erase that could not finish still
        /// does not start.
        #[arg(long)]
        force: bool,
    },
    /// Stored pulls, newest first, each with its guessed flights.
    List {
        /// One device's.
        #[arg(long)]
        device: Option<String>,
    },
    /// Write a stored pull to a folder as .bbl (--split: each log too).
    Export {
        /// The pull id (<device>/<time>, from `gear blackbox list`).
        id: String,
        dir: PathBuf,
        #[arg(long)]
        split: bool,
    },
    /// Erase the FC's flash. Needs --confirm and a stored pull of exactly what the flash holds.
    Erase {
        #[arg(long)]
        port: Option<String>,
        #[arg(long)]
        confirm: bool,
    },
}

pub fn run(core: &Core, cmd: BlackboxCmd) -> Result<Value> {
    Ok(match cmd {
        BlackboxCmd::Pull {
            port,
            keep,
            mode,
            force,
        } => serde_json::to_value(call::gear_blackbox_pull(
            core,
            api::BlackboxPullParams {
                port,
                keep,
                mode: mode.map(|m| match m {
                    Mode::Auto => PullMode::Auto,
                    Mode::Msp => PullMode::Msp,
                    Mode::Msc => PullMode::Msc,
                }),
                force,
            },
        )?)?,
        BlackboxCmd::List { device } => {
            serde_json::to_value(call::gear_blackbox(core, api::BlackboxFilter { device })?)?
        }
        BlackboxCmd::Export { id, dir, split } => serde_json::to_value(
            call::gear_blackbox_export(core, api::BlackboxExportParams { id, to: dir, split })?,
        )?,
        BlackboxCmd::Erase { port, confirm } => serde_json::to_value(call::gear_blackbox_erase(
            core,
            api::BlackboxEraseParams { port, confirm },
        )?)?,
    })
}
