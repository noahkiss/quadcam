//! `quadcam-cli gear sim`: the sim's radio calibration, the defaults an aircraft gives it,
//! its profiles, and fitting and validating a profile against a quad's logs. `fit` prints
//! the fit and its digest; `fit ... --digest D --yes` saves it.

use anyhow::{bail, Context, Result};
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
    /// Check a sim profile against a quad's blackbox logs: hover, punch, sag, rate
    /// response, coast-down, fall recovery and attitude replay, each with its band.
    Validate {
        /// A sim profile: meteor75, air65ii, five_inch, seven_inch, or a fitted one's id.
        aircraft: String,
        /// A blackbox file (.bbl), a flash image, a decoded CSV, or a folder of them.
        #[arg(long)]
        logs: PathBuf,
        /// The motors' pole count (default: the profile's).
        #[arg(long)]
        poles: Option<u32>,
        /// Print the result table instead of JSON.
        #[arg(long)]
        text: bool,
    },
    /// The sim's profiles: the built-in ones, then the ones fitted from logs. --delete ID
    /// --yes removes a fitted one.
    Profiles {
        #[arg(long, requires = "yes")]
        delete: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    /// Fit a sim profile from a quad's blackbox logs, its diff all and its weight; print the
    /// fit, its validation and the base's on the same logs, and the digest. With --digest
    /// and --yes, save it.
    Fit {
        /// The built-in profile to start from: meteor75, air65ii, five_inch, seven_inch.
        #[arg(long)]
        base: String,
        /// An aircraft profile: its FC's blackbox pulls and latest backup.
        #[arg(long)]
        aircraft: Option<String>,
        /// A blackbox file, a flash image, a decoded CSV, or a folder of them (repeatable).
        #[arg(long = "logs")]
        logs: Vec<PathBuf>,
        /// A Betaflight diff or dump file (repeatable; default: the aircraft's FC backup).
        #[arg(long = "fc")]
        fc: Vec<PathBuf>,
        /// The quad weighed with a pack, in grams.
        #[arg(long)]
        mass_g: Option<f64>,
        /// The pack's weight, in grams.
        #[arg(long)]
        pack_g: Option<f64>,
        /// The new profile's id (default: from the aircraft's name).
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        label: Option<String>,
        /// The digest a fit printed: with --yes, save that fit.
        #[arg(long)]
        digest: Option<String>,
        #[arg(long)]
        yes: bool,
        /// Print the fit's table instead of JSON.
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
        SimCmd::Profiles {
            delete: Some(id),
            yes,
        } => serde_json::to_value(call::gear_sim_profile_delete(
            core,
            api::SimProfileDeleteParams { id, confirm: yes },
        )?)?,
        SimCmd::Profiles { .. } => serde_json::to_value(call::gear_sim_profiles(core)?)?,
        SimCmd::Fit {
            base,
            aircraft,
            logs,
            fc,
            mass_g,
            pack_g,
            id,
            label,
            digest,
            yes,
            text,
        } => {
            let fit = api::SimFitParams {
                base,
                aircraft,
                logs,
                fc,
                mass_g,
                pack_g,
                id,
                label,
            };
            match digest {
                Some(digest) => {
                    if !yes {
                        bail!("Refused: a save needs --digest from the fit and --yes.");
                    }
                    serde_json::to_value(call::gear_sim_fit_save(
                        core,
                        api::SimFitSaveParams {
                            fit,
                            digest,
                            confirm: true,
                        },
                    )?)?
                }
                None => {
                    let f = call::gear_sim_fit(core, fit)?;
                    if text {
                        Value::String(format!(
                            "{}\nFitted:\n{}\nBase:\n{}\nDigest: {}\n",
                            f.report.table(),
                            f.validation.table(),
                            f.base_validation.table(),
                            f.digest
                        ))
                    } else {
                        serde_json::to_value(f)?
                    }
                }
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
