//! `quadcam-cli gear setup`: a radio setup as data (jobs, the controls that do them, the
//! model template, the aircraft) read against a radio's card or a card folder;
//! `setup-edit`, which saves a radio's setup and stages it as one "Radio setup" change; and
//! `post-flash`, the steps after a flash. Nothing is written to the card; the apply sheet
//! writes it.

use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use quadcam_lib::api::{self, call};
use quadcam_lib::core::{radio_setup, Core, PostFlashStep};
use quadcam_lib::gear::edgetx::setup::{Assignment, RadioSetup};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Args)]
pub struct SetupArgs {
    /// A saved radio's device id: its saved setup against its mounted card or latest backup.
    pub device: Option<String>,
    /// A card folder (a mounted card, or a copy) to read instead of the radio's card.
    #[arg(long, value_name = "DIR")]
    pub mount: Option<PathBuf>,
    /// A setup file (JSON) to read instead of the saved one.
    #[arg(long, value_name = "FILE")]
    pub setup: Option<PathBuf>,
    /// Print the jobs and what would change as text.
    #[arg(long)]
    pub text: bool,
}

#[derive(Args)]
pub struct SetupEditArgs {
    /// A saved radio's device id.
    pub device: String,
    /// A setup file (JSON) that replaces the saved one.
    #[arg(long, value_name = "FILE")]
    pub setup: Option<PathBuf>,
    /// Start again from the board's default setup.
    #[arg(long)]
    pub reset: bool,
    /// A job's control: `JOB=CONTROL` for a pot, slider or trim, `JOB=CONTROL:HIGH` for a
    /// switch, HIGH its position (0 or 2) that sends 2000 µs (`rates=SC:0`). Repeat for more.
    #[arg(long, value_name = "JOB=CONTROL[:HIGH]")]
    pub assign: Vec<String>,
    /// A job no control does any more. Repeat for more.
    #[arg(long, value_name = "JOB")]
    pub free: Vec<String>,
    /// Stage the generated edits as the radio's "Radio setup" change.
    #[arg(long)]
    pub stage: bool,
}

#[derive(Args)]
pub struct PostFlashArgs {
    /// A saved radio's device id.
    pub device: String,
    /// A step: storage-pack, elrs-lua or volume-label. Repeat for more; every step when left
    /// out.
    #[arg(long, value_name = "STEP")]
    pub step: Vec<String>,
    /// The storage pack release (2.12.3); default the newest QuadCam checked for the radio's
    /// EdgeTX series.
    #[arg(long, value_name = "VERSION")]
    pub pack_version: Option<String>,
    /// The ExpressLRS release whose Lua to put on (4.1.0).
    #[arg(long, value_name = "VERSION")]
    pub elrs_version: Option<String>,
    /// Stage the storage pack and the Lua as card changes.
    #[arg(long)]
    pub stage: bool,
}

#[derive(Args)]
pub struct VolumeLabelArgs {
    /// A saved radio's device id; its volume must be mounted.
    pub device: String,
    /// The label (upper case letters, digits, _ and -, 11 at most); default one from the
    /// radio's saved name.
    #[arg(long)]
    pub label: Option<String>,
    /// Rename it.
    #[arg(long)]
    pub yes: bool,
}

fn read_setup(f: &PathBuf) -> Result<RadioSetup> {
    let text = std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?;
    serde_json::from_str(&text).context("the file is not a radio setup")
}

/// `JOB=CONTROL[:HIGH]` as an assignment.
pub fn assignment_of(s: &str) -> Result<Assignment> {
    let (job, rest) = s
        .split_once('=')
        .ok_or_else(|| anyhow!("{s:?} is not JOB=CONTROL[:HIGH]"))?;
    let (control, high) = match rest.split_once(':') {
        Some((c, h)) => (
            c,
            Some(
                h.trim()
                    .parse::<u8>()
                    .map_err(|_| anyhow!("{s:?}: HIGH is 0 or 2"))?,
            ),
        ),
        None => (rest, None),
    };
    if job.trim().is_empty() || control.trim().is_empty() {
        bail!("{s:?} is not JOB=CONTROL[:HIGH]");
    }
    Ok(Assignment {
        job: job.trim().into(),
        control: control.trim().into(),
        high,
    })
}

fn step_of(s: &str) -> Result<PostFlashStep> {
    Ok(match s.trim() {
        "storage-pack" | "storage_pack" => PostFlashStep::StoragePack,
        "elrs-lua" | "elrs_lua" => PostFlashStep::ElrsLua,
        "volume-label" | "volume_label" => PostFlashStep::VolumeLabel,
        other => bail!("{other:?} is not a step (storage-pack, elrs-lua, volume-label)"),
    })
}

pub fn run(core: &Core, a: SetupArgs) -> Result<Value> {
    let d = call::gear_radio_setup(
        core,
        api::RadioSetupParams {
            device: a.device,
            mount: a.mount.map(|m| m.display().to_string()),
            setup: a.setup.as_ref().map(read_setup).transpose()?,
        },
    )?;
    Ok(if a.text {
        Value::String(radio_setup::render_text(&d))
    } else {
        serde_json::to_value(d)?
    })
}

pub fn edit(core: &Core, a: SetupEditArgs) -> Result<Value> {
    let assign = a
        .assign
        .iter()
        .map(|s| assignment_of(s))
        .collect::<Result<Vec<_>>>()?;
    let r = call::gear_radio_setup_edit(
        core,
        api::RadioSetupEditParams {
            device: a.device,
            setup: a.setup.as_ref().map(read_setup).transpose()?,
            reset: a.reset,
            assign,
            free: a.free,
            stage: a.stage,
            editor: None,
        },
    )?;
    Ok(serde_json::to_value(r)?)
}

pub fn post_flash(core: &Core, a: PostFlashArgs) -> Result<Value> {
    let steps = a
        .step
        .iter()
        .map(|s| step_of(s))
        .collect::<Result<Vec<_>>>()?;
    let r = call::gear_radio_post_flash(
        core,
        api::PostFlashParams {
            device: a.device,
            steps,
            pack_version: a.pack_version,
            elrs_version: a.elrs_version,
            stage: a.stage,
            editor: None,
        },
    )?;
    Ok(serde_json::to_value(r)?)
}

pub fn volume_label(core: &Core, a: VolumeLabelArgs) -> Result<Value> {
    let r = call::gear_volume_label(
        core,
        api::VolumeLabelParams {
            device: a.device,
            label: a.label,
            confirm: a.yes,
        },
    )?;
    Ok(serde_json::to_value(r)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_become_assignments_and_steps() {
        assert_eq!(
            assignment_of("rates=SC:0").unwrap(),
            Assignment {
                job: "rates".into(),
                control: "SC".into(),
                high: Some(0)
            }
        );
        assert_eq!(assignment_of("vtx=P2").unwrap().high, None);
        assert!(assignment_of("rates").is_err());
        assert!(assignment_of("rates=SC:x").is_err());
        assert_eq!(step_of("elrs-lua").unwrap(), PostFlashStep::ElrsLua);
        assert!(step_of("splash").is_err());
    }
}
