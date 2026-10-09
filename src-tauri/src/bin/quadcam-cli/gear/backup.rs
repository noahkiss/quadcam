//! `quadcam-cli gear backup|backups|storage|import-backups|card-check|card-repair|stop`:
//! the backup store (design 7.1) and the card check.

use anyhow::{bail, Result};
use clap::{Args, Subcommand};
use quadcam_lib::api::{self, call};
use quadcam_lib::core::Core;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Args)]
pub struct BackupArgs {
    /// A connected device's id.
    #[arg(long)]
    device: Option<String>,
    /// An FC's port (/dev/cu.usbmodem...).
    #[arg(long)]
    port: Option<String>,
    /// A radio card's mount point.
    #[arg(long)]
    mount: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Option<BackupCmd>,
}

#[derive(Subcommand)]
pub enum BackupCmd {
    /// A backup's file list, or one file's content.
    Show {
        /// The backup id (<device>/<name>, from `gear backups`).
        id: String,
        /// A file of the backup (MODELS/model01.yml, "diff all").
        path: Option<String>,
    },
    /// What changed from backup A to backup B (one argument: from the backup before it).
    Diff {
        a: String,
        b: Option<String>,
        /// One file only.
        #[arg(long)]
        path: Option<String>,
    },
    /// Keep a backup through pruning (--off to stop keeping it).
    Pin {
        id: String,
        #[arg(long)]
        off: bool,
    },
}

#[derive(Args)]
pub struct StorageArgs {
    /// Thin backups by the retention settings and remove stored files nothing uses.
    #[arg(long)]
    prune: bool,
    /// With --prune: report what it would remove.
    #[arg(long)]
    dry_run: bool,
    /// Write a backup (an id) or every backup of a device (a device id) as plain folders
    /// into DIR.
    #[arg(long, num_args = 2, value_names = ["BACKUP_OR_DEVICE", "DIR"])]
    export: Option<Vec<String>>,
}

pub fn backup(core: &Core, a: BackupArgs) -> Result<Value> {
    Ok(match a.cmd {
        None => serde_json::to_value(call::gear_backup(
            core,
            api::BackupParams {
                device: a.device,
                port: a.port,
                mount: a.mount,
            },
        )?)?,
        Some(BackupCmd::Show { id, path }) => serde_json::to_value(call::gear_backup_read(
            core,
            api::BackupReadParams { id, path },
        )?)?,
        Some(BackupCmd::Diff { a, b, path }) => serde_json::to_value(call::gear_backup_diff(
            core,
            api::BackupDiffParams { a, b, path },
        )?)?,
        Some(BackupCmd::Pin { id, off }) => serde_json::to_value(call::gear_backup_pin(
            core,
            api::BackupPinParams { id, pinned: !off },
        )?)?,
    })
}

pub fn backups(core: &Core, device: Option<String>) -> Result<Value> {
    Ok(serde_json::to_value(call::gear_backups(
        core,
        api::BackupFilter { device },
    )?)?)
}

pub fn storage(core: &Core, a: StorageArgs) -> Result<Value> {
    if a.dry_run && !a.prune {
        bail!("--dry-run goes with --prune.");
    }
    if a.prune {
        return Ok(serde_json::to_value(call::gear_prune(
            core,
            api::PruneParams { dry_run: a.dry_run },
        )?)?);
    }
    if let Some(e) = a.export {
        let (what, to) = (e[0].clone(), PathBuf::from(&e[1]));
        // A backup id has a slash; a device id does not.
        let (device, snapshot) = if what.contains('/') {
            (None, Some(what))
        } else {
            (Some(what), None)
        };
        return Ok(serde_json::to_value(call::gear_export(
            core,
            api::ExportParams {
                device,
                snapshot,
                to,
            },
        )?)?);
    }
    Ok(serde_json::to_value(call::gear_storage(core)?)?)
}

pub fn import(
    core: &Core,
    folder: PathBuf,
    device: Option<String>,
    dry_run: bool,
) -> Result<Value> {
    Ok(serde_json::to_value(call::gear_import_backups(
        core,
        api::ImportBackupsParams {
            folder,
            device,
            dry_run,
        },
    )?)?)
}

pub fn card_check(
    core: &Core,
    device: Option<String>,
    mount: Option<PathBuf>,
    log: bool,
) -> Result<Value> {
    if log {
        let Some(device) = device else {
            bail!("--log needs --device.");
        };
        return Ok(serde_json::to_value(call::gear_card_checks(
            core,
            api::CardChecksParams { device },
        )?)?);
    }
    Ok(serde_json::to_value(call::gear_card_check(
        core,
        api::CardCheckParams { device, mount },
    )?)?)
}

pub fn card_clean(
    core: &Core,
    device: Option<String>,
    mount: Option<PathBuf>,
    remove: bool,
    yes: bool,
) -> Result<Value> {
    Ok(serde_json::to_value(call::gear_card_clean(
        core,
        api::CardCleanParams {
            mount,
            device,
            remove,
            confirm: yes,
        },
    )?)?)
}

pub fn radio_cli(
    core: &Core,
    action: &str,
    port: Option<String>,
    path: Option<String>,
    device: Option<String>,
    yes: bool,
) -> Result<Value> {
    let action = serde_json::from_value(Value::String(action.to_string())).map_err(|_| {
        anyhow::anyhow!("unknown radio action {action:?}; use identify, ls, play, beep, reboot or verify")
    })?;
    Ok(serde_json::to_value(call::gear_radio_cli(
        core,
        api::RadioCliParams {
            port,
            action,
            path,
            device,
            confirm: yes,
        },
    )?)?)
}

pub fn card_repair(core: &Core, check: String, yes: bool) -> Result<Value> {
    Ok(serde_json::to_value(call::gear_card_repair(
        core,
        api::CardRepairParams {
            check,
            confirm: yes,
        },
    )?)?)
}

pub fn stop(core: &Core, handle: String) -> Result<Value> {
    Ok(serde_json::to_value(call::gear_stop(
        core,
        api::StopParams { handle },
    )?)?)
}
