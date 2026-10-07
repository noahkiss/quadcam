//! `quadcam-cli gear sim`: the sim's radio calibration, the defaults an aircraft gives it,
//! and the validation of a sim profile against a quad's logs.

use anyhow::{Context, Result};
use clap::Subcommand;
use quadcam_lib::api::{self, call};
use quadcam_lib::core::Core;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum SimCmd {
    /// A radio's sim calibration; with no radio, the one in USB Joystick mode now, matched to
    /// a saved radio. --set saves one from a JSON file.
    Calibration {
        /// A saved radio's id, or a provisional usb-… key.
        radio: Option<String>,
        /// A calibration JSON file (the `calibration` field of this command's answer) to save.
        #[arg(long, requires = "radio")]
        set: Option<PathBuf>,
        /// With --set: remember this radio for its USB product name.
        #[arg(long)]
        product: Option<String>,
    },
    /// What the sim pre-fills: stick channels, arm, angle, horizon, turtle and air mode
    /// switches, a reset control, each with its source.
    Defaults {
        /// An aircraft profile: the latest backups of its saved radio and FC.
        #[arg(long)]
        aircraft: Option<String>,
        /// An EdgeTX card (mount or folder) or one model file.
        #[arg(long)]
        radio: Option<PathBuf>,
        /// Betaflight dump, diff or CLI files.
        #[arg(long = "fc")]
        fc: Vec<PathBuf>,
    },
    /// Check a sim profile against a folder of decoded blackbox logs (the CSV files
    /// `blackbox_decode` writes): hover, punch, sag, rate response, coast-down and fall
    /// recovery, each with its band.
    Validate {
        /// A built-in sim profile: meteor75, air65ii, five_inch, seven_inch.
        aircraft: String,
        /// The folder of decoded logs.
        #[arg(long)]
        logs: PathBuf,
        /// The motors' pole count (default: the profile's).
        #[arg(long)]
        poles: Option<u32>,
        /// Print the result table instead of JSON.
        #[arg(long)]
        text: bool,
    },
}

pub fn run(core: &Core, cmd: SimCmd) -> Result<Value> {
    Ok(match cmd {
        SimCmd::Calibration {
            radio,
            set: Some(file),
            product,
        } => {
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("cannot read {}", file.display()))?;
            let calibration = serde_json::from_str(&text)
                .with_context(|| format!("{} is not a calibration", file.display()))?;
            serde_json::to_value(call::gear_sim_calibration_save(
                core,
                api::SimCalibrationSaveParams {
                    radio: radio.unwrap_or_default(),
                    calibration,
                    remember: product.is_some(),
                    product,
                    ..Default::default()
                },
            )?)?
        }
        SimCmd::Calibration { radio, .. } => serde_json::to_value(call::gear_sim_calibration(
            core,
            api::SimCalibrationParams { radio },
        )?)?,
        SimCmd::Validate {
            aircraft,
            logs,
            poles,
            text,
        } => {
            let report = call::gear_sim_validate(
                core,
                api::SimValidateParams {
                    aircraft,
                    logs,
                    motor_poles: poles,
                },
            )?;
            if text {
                Value::String(report.table())
            } else {
                serde_json::to_value(report)?
            }
        }
        SimCmd::Defaults {
            aircraft,
            radio,
            fc,
        } => serde_json::to_value(call::gear_sim_defaults(
            core,
            api::SimDefaultsParams {
                aircraft,
                radio,
                fc,
            },
        )?)?,
    })
}
