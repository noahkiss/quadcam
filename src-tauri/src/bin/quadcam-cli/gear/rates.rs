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

pub fn sims(core: &Core, a: SimsArgs) -> Result<Value> {
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
