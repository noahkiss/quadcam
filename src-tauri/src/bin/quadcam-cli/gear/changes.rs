//! `quadcam-cli gear changes|stage|discard|restore|apply`: staged changes and the FC apply
//! (design 8). `apply --plan` runs every guard and prints the diff and the digest; `apply
//! --digest D --yes` writes. Nothing here writes a device without both.

use anyhow::{bail, Context, Result};
use clap::Args;
use quadcam_lib::api::{self, call};
use quadcam_lib::core::Core;
use quadcam_lib::gear::model::{ChangeStatus, Edit, Section};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Args)]
pub struct ChangesArgs {
    /// One device's changes.
    #[arg(long)]
    device: Option<String>,
    /// Only changes in this status (draft, ready, applied, verified, failed, discarded).
    #[arg(long)]
    status: Option<String>,
    /// Include applied, failed and discarded changes.
    #[arg(long)]
    history: bool,
}

#[derive(Args)]
pub struct StageArgs {
    /// The FC's device id (from `gear devices`).
    #[arg(long)]
    device: String,
    #[arg(long)]
    title: Option<String>,
    /// A file of raw Betaflight CLI lines (no save, exit or defaults: QuadCam saves).
    #[arg(long)]
    cli: Option<PathBuf>,
    /// A JSON file holding an array of edits (`{"kind": "radio", "ops": [...]}`, ...): the
    /// way to stage a radio card change.
    #[arg(long)]
    edits: Option<PathBuf>,
    /// One setting, NAME=VALUE. Repeat for more.
    #[arg(long = "set")]
    sets: Vec<String>,
    /// With --set: the PID profile the setting lives in.
    #[arg(long)]
    profile: Option<u8>,
    /// With --set: the rate profile the setting lives in.
    #[arg(long)]
    rateprofile: Option<u8>,
    #[arg(long)]
    note: Option<String>,
    /// Keep it as a draft.
    #[arg(long)]
    draft: bool,
}

#[derive(Args)]
pub struct ApplyArgs {
    /// The change id (from `gear changes`).
    change: String,
    /// Run every guard and print the diff and the digest. Writes nothing.
    #[arg(long)]
    plan: bool,
    /// The digest `--plan` printed.
    #[arg(long)]
    digest: Option<String>,
    /// Confirm the write.
    #[arg(long)]
    yes: bool,
    /// The FC's port, when several are plugged in.
    #[arg(long)]
    port: Option<String>,
}

pub fn changes(core: &Core, a: ChangesArgs) -> Result<Value> {
    let status: Option<ChangeStatus> = match a.status {
        Some(s) => Some(
            serde_json::from_value(json!(s.to_lowercase()))
                .with_context(|| format!("{s:?} is not a status"))?,
        ),
        None => None,
    };
    Ok(serde_json::to_value(call::gear_changes(
        core,
        api::ChangeFilter {
            device: a.device,
            status,
            history: a.history,
        },
    )?)?)
}

pub fn stage(core: &Core, a: StageArgs) -> Result<Value> {
    let mut edits = Vec::new();
    if let Some(f) = &a.cli {
        let text =
            std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?;
        edits.push(Edit::FcLines {
            lines: text.lines().map(str::to_string).collect(),
        });
    }
    let section = match (a.profile, a.rateprofile) {
        (Some(_), Some(_)) => bail!("Use --profile or --rateprofile, not both."),
        (Some(n), None) => Section::Profile(n),
        (None, Some(n)) => Section::RateProfile(n),
        (None, None) => Section::Master,
    };
    for s in &a.sets {
        let (name, value) = s
            .split_once('=')
            .with_context(|| format!("{s:?} is not NAME=VALUE"))?;
        edits.push(Edit::FcSet {
            section,
            name: name.trim().into(),
            value: value.trim().into(),
        });
    }
    if let Some(f) = &a.edits {
        let text =
            std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?;
        let list: Vec<Edit> = serde_json::from_str(&text)
            .with_context(|| format!("{} is not a JSON array of edits", f.display()))?;
        edits.extend(list);
    }
    if edits.is_empty() {
        bail!("Give --cli FILE, --set NAME=VALUE or --edits FILE.json.");
    }
    Ok(serde_json::to_value(call::gear_change_stage(
        core,
        api::StageParams {
            device: a.device,
            title: a.title,
            edits,
            note: a.note,
            editor: None,
            draft: a.draft,
        },
    )?)?)
}

pub fn discard(core: &Core, id: String) -> Result<Value> {
    Ok(serde_json::to_value(call::gear_change_discard(
        core,
        api::IdParams { id },
    )?)?)
}

pub fn restore(core: &Core, backup: String, paths: Vec<String>) -> Result<Value> {
    Ok(serde_json::to_value(call::gear_restore_stage(
        core,
        api::RestoreParams {
            backup,
            paths,
            editor: None,
        },
    )?)?)
}

#[derive(Args)]
pub struct UpdateArgs {
    /// The change id.
    change: String,
    /// draft, ready, try (apply, fly, then keep or revert) or read_first (read the real
    /// value on the device before changing anything).
    #[arg(long)]
    status: Option<String>,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    note: Option<String>,
    /// Its place in the device's queue (0 first).
    #[arg(long)]
    order: Option<u32>,
}

pub fn update(core: &Core, a: UpdateArgs) -> Result<Value> {
    let status: Option<ChangeStatus> = match a.status {
        Some(s) => Some(
            serde_json::from_value(json!(s.to_lowercase()))
                .with_context(|| format!("{s:?} is not a status"))?,
        ),
        None => None,
    };
    Ok(serde_json::to_value(call::gear_change_update(
        core,
        api::ChangeUpdateParams {
            id: a.change,
            title: a.title,
            edits: None,
            status,
            note: a.note,
            order: a.order,
        },
    )?)?)
}

pub fn keep(core: &Core, id: String) -> Result<Value> {
    Ok(serde_json::to_value(call::gear_change_keep(
        core,
        api::IdParams { id },
    )?)?)
}

pub fn revert(core: &Core, id: String) -> Result<Value> {
    Ok(serde_json::to_value(call::gear_change_revert(
        core,
        api::IdParams { id },
    )?)?)
}

#[derive(Args)]
pub struct CopyArgs {
    /// The FC to copy from: its device id (latest backup) or a backup id.
    #[arg(long)]
    from: String,
    /// The FC to copy to (a device id).
    #[arg(long)]
    to: String,
    /// A part to copy: rates, pid, osd, modes, adjustments, vtx, features. Repeat for more.
    #[arg(long = "part")]
    parts: Vec<String>,
    /// A setting to copy by name. Repeat for more.
    #[arg(long = "setting")]
    settings: Vec<String>,
    /// Show the checks and the diff; stage nothing.
    #[arg(long)]
    plan: bool,
}

pub fn copy(core: &Core, a: CopyArgs) -> Result<Value> {
    let mut parts = Vec::new();
    for p in &a.parts {
        parts.push(
            serde_json::from_value(json!(p.to_lowercase())).with_context(|| {
                format!("{p:?} is not a part (rates, pid, osd, modes, adjustments, vtx, features)")
            })?,
        );
    }
    let params = api::CopyParams {
        from: a.from,
        to: a.to,
        select: quadcam_lib::gear::copy::CopySelect {
            parts,
            settings: a.settings,
        },
        editor: None,
    };
    if a.plan {
        return Ok(serde_json::to_value(call::gear_copy_plan(core, params)?)?);
    }
    Ok(serde_json::to_value(call::gear_copy_stage(core, params)?)?)
}

pub fn apply(core: &Core, a: ApplyArgs) -> Result<Value> {
    if a.plan {
        return Ok(serde_json::to_value(call::gear_apply_plan(
            core,
            api::ApplyPlanParams {
                id: a.change,
                port: a.port,
            },
        )?)?);
    }
    let Some(digest) = a.digest else {
        bail!("Refused: apply needs --digest from `--plan` and --yes.");
    };
    Ok(serde_json::to_value(call::gear_apply(
        core,
        api::ApplyRequest {
            id: a.change,
            digest,
            confirm: a.yes,
            port: a.port,
        },
    )?)?)
}
