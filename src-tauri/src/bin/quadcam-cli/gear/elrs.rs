//! `quadcam-cli gear elrs`: ExpressLRS (design 6.4), a preview behind the `elrs_preview`
//! setting. `read` hands a saved radio's or FC's port to the module or receiver behind it
//! and saves what it finds; `set` stages option changes (apply them with `gear apply`);
//! `flash` prints a plan, and `--digest D --yes` flashes. Nothing here writes a device
//! without the plan's digest and `--yes`.

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use quadcam_lib::api::{self, call};
use quadcam_lib::core::Core;
use quadcam_lib::gear::elrs::ElrsSet;
use quadcam_lib::gear::model::Edit;
use serde_json::Value;

#[derive(Subcommand)]
pub enum ElrsCmd {
    /// The ExpressLRS devices read so far, the radios and FCs to read through, and what a
    /// flash needs. `--check` reads the newest release from the network.
    Status {
        #[arg(long)]
        check: bool,
    },
    /// Read the device behind a saved radio (its internal module) or FC (its receiver):
    /// version, target, options. The radio or FC stays in passthrough until it is restarted
    /// or unplugged.
    Read {
        /// The saved radio or FC (from `gear devices`).
        host: String,
        /// The host's serial port; omit when one is plugged in.
        #[arg(long)]
        port: Option<String>,
    },
    /// Stage option changes for a read ELRS device, as one change (apply it with `gear
    /// apply`). Options: packet_rate, telemetry_ratio, power, dynamic_power, switch_mode,
    /// model_match. The binding phrase is never an option.
    Set {
        /// The ELRS device id (from `gear elrs`).
        device: String,
        /// OPTION=VALUE, for example packet_rate=250Hz or telemetry_ratio=1:16.
        #[arg(required = true)]
        options: Vec<String>,
        #[arg(long)]
        note: Option<String>,
        /// Keep it as a draft.
        #[arg(long)]
        draft: bool,
    },
    /// Plan an ExpressLRS flash: the official release, the device's target, the image with
    /// the binding UID's fingerprint, every guard and a digest. `--digest D --yes` flashes
    /// with the esptool module.
    Flash(FlashArgs),
}

#[derive(Args)]
pub struct FlashArgs {
    /// The ELRS device id (from `gear elrs`).
    device: String,
    /// The ExpressLRS version. Default: the newest release the last check found.
    #[arg(long)]
    version: Option<String>,
    /// The release zip's SHA-256, if you have it from a trusted place.
    #[arg(long)]
    sha256: Option<String>,
    /// The host's serial port; omit when one is plugged in.
    #[arg(long)]
    port: Option<String>,
    /// The digest the plan printed.
    #[arg(long)]
    digest: Option<String>,
    #[arg(long)]
    yes: bool,
}

fn parse_set(s: &str) -> Result<ElrsSet> {
    let (option, value) = s
        .split_once('=')
        .with_context(|| format!("`{s}` is not OPTION=VALUE"))?;
    if option.trim().is_empty() || value.trim().is_empty() {
        bail!("`{s}` is not OPTION=VALUE");
    }
    Ok(ElrsSet {
        option: option.trim().into(),
        value: value.trim().into(),
    })
}

pub fn run(core: &Core, cmd: Option<ElrsCmd>) -> Result<Value> {
    Ok(match cmd.unwrap_or(ElrsCmd::Status { check: false }) {
        ElrsCmd::Status { check } => serde_json::to_value(call::gear_elrs(
            core,
            api::ElrsParams {
                check: check.then_some(true),
            },
        )?)?,
        ElrsCmd::Read { host, port } => {
            serde_json::to_value(call::gear_elrs_read(core, api::ElrsReadParams { host, port })?)?
        }
        ElrsCmd::Set {
            device,
            options,
            note,
            draft,
        } => {
            let options = options
                .iter()
                .map(|s| parse_set(s))
                .collect::<Result<Vec<_>>>()?;
            serde_json::to_value(call::gear_change_stage(
                core,
                quadcam_lib::core::StageParams {
                    device,
                    title: None,
                    edits: vec![Edit::ElrsOptions { options }],
                    note,
                    editor: None,
                    draft,
                },
            )?)?
        }
        ElrsCmd::Flash(a) => {
            let params = api::ElrsFlashParams {
                device: a.device,
                version: a.version,
                sha256: a.sha256,
                port: a.port,
            };
            let Some(digest) = a.digest else {
                return Ok(serde_json::to_value(call::gear_elrs_flash_plan(
                    core, params,
                )?)?);
            };
            if !a.yes {
                bail!("Refused: a flash needs --digest from the plan and --yes.");
            }
            serde_json::to_value(call::gear_elrs_flash(
                core,
                api::ElrsFlashRequest {
                    params,
                    digest,
                    confirm: true,
                },
            )?)?
        }
    })
}
