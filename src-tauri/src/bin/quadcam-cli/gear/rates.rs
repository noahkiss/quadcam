//! `quadcam-cli gear rates` and `gear sims`: an FC's rate profiles, and the sims' next to them.

use anyhow::Result;
use clap::Args;
use quadcam_lib::api::{self, call};
use quadcam_lib::core::Core;
use quadcam_lib::gear::{rates, sims};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Args)]
pub struct RatesArgs {
    /// Betaflight `dump all`, `diff all` or CLI-line files, read in order (a later file's
    /// lines win), or one saved device id (its latest backup).
    pub target: Vec<String>,
    /// A backup id from `gear backups`, instead of the latest.
    #[arg(long)]
    pub backup: Option<String>,
    /// Print the profiles as text instead of the structured view.
    #[arg(long)]
    pub text: bool,
}

#[derive(Args)]
pub struct SimsArgs {
    /// The quad to compare with: dump or diff files, or one saved device id. Omit to list
    /// the sims alone.
    pub target: Vec<String>,
    /// A backup id from `gear backups`, instead of the latest.
    #[arg(long)]
    pub backup: Option<String>,
    /// The quad's rate profile to compare with (default: the one it uses).
    #[arg(long)]
    pub profile: Option<u8>,
    /// Print the sims as text instead of the structured view.
    #[arg(long)]
    pub text: bool,
    /// Write the quad's rate profile into sim profiles. Needs the quad and `--to`.
    /// Without `--digest` it only prints the plan (checks, diff, warnings, digest); with
    /// `--digest D --yes` it backs up each file, writes it and reads it back.
    #[arg(long)]
    pub sync: bool,
    /// A sim profile to overwrite, `SIM[:PROFILE][@FILE]`: `liftoff:Freestyle`, `uncrashed:OUT`,
    /// or `all` (each sim's profile named like the quad's). Repeat for several.
    #[arg(long = "to")]
    pub to: Vec<String>,
    /// The digest `--sync` printed.
    #[arg(long)]
    pub digest: Option<String>,
    /// Confirm the write.
    #[arg(long)]
    pub yes: bool,
}

/// One argument that is not a file is a device id.
fn split(target: &[String]) -> (Vec<PathBuf>, Option<String>) {
    match target {
        [one] if !std::path::Path::new(one).exists() => (Vec::new(), Some(one.clone())),
        _ => (target.iter().map(PathBuf::from).collect(), None),
    }
}

pub fn rates(core: &Core, a: RatesArgs) -> Result<Value> {
    let (paths, device) = split(&a.target);
    let view = call::gear_rates(
        core,
        api::RatesParams {
            paths,
            device,
            backup: a.backup,
        },
    )?;
    Ok(if a.text {
        Value::String(rates::render_text(&view))
    } else {
        serde_json::to_value(view)?
    })
}

/// `SIM[:PROFILE][@FILE]` as a target.
fn target(s: &str) -> api::SimTarget {
    let (rest, file) = match s.split_once('@') {
        Some((r, f)) => (r, Some(f.to_string())),
        None => (s, None),
    };
    let (sim, profile) = match rest.split_once(':') {
        Some((sim, p)) => (sim, Some(p.to_string())),
        None => (rest, None),
    };
    api::SimTarget {
        sim: sim.to_string(),
        file,
        profile,
    }
}

fn sync(core: &Core, a: SimsArgs) -> Result<Value> {
    let (paths, device) = split(&a.target);
    let params = api::SimSyncParams {
        sims: a.to.iter().map(|t| target(t)).collect(),
        paths,
        device,
        backup: a.backup,
        profile: a.profile,
    };
    let Some(digest) = a.digest else {
        return Ok(serde_json::to_value(call::gear_sim_sync_plan(core, params)?)?);
    };
    Ok(serde_json::to_value(call::gear_sim_sync(
        core,
        api::SimSyncRequest {
            params,
            digest,
            confirm: a.yes,
        },
    )?)?)
}

pub fn sims(core: &Core, a: SimsArgs) -> Result<Value> {
    if a.sync {
        return sync(core, a);
    }
    let (paths, device) = split(&a.target);
    let list = call::gear_sims(
        core,
        api::SimsParams {
            paths,
            device,
            backup: a.backup,
            profile: a.profile,
        },
    )?;
    Ok(if a.text {
        Value::String(sims::render_text(&list))
    } else {
        serde_json::to_value(list)?
    })
}
