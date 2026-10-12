//! `quadcam-cli gear radio-settings`: a radio's own settings (`RADIO/radio.yml`) for the
//! Settings editor, and `radio-settings-edit`, which stages them as one "Radio settings"
//! change. Nothing is written to the card; the apply sheet writes it.

use anyhow::{Context, Result};
use clap::Args;
use quadcam_lib::api::{self, call};
use quadcam_lib::core::{radio_settings, Core};
use quadcam_lib::gear::edgetx::card::RadioOp;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Args)]
pub struct RadioSettingsArgs {
    /// A saved radio's device id.
    pub device: String,
    /// Without the device's staged radio edits on top.
    #[arg(long)]
    pub no_staged: bool,
    /// Print the settings as text instead of the structured view.
    #[arg(long)]
    pub text: bool,
}

#[derive(Args)]
pub struct RadioSettingsEditArgs {
    /// A saved radio's device id.
    pub device: String,
    /// A setting in the file's own units: `KEY=VALUE` (`beepVolume=3`, `vBatWarn=66`,
    /// `backlightMode=backlight_mode_on`). `radio-settings` lists the keys and values.
    /// Repeat for more.
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub set: Vec<String>,
    /// The model the radio selects (model01.yml).
    #[arg(long, value_name = "FILE")]
    pub select_model: Option<String>,
    /// Radio ops as JSON: a file holding a list of ops.
    #[arg(long, value_name = "FILE")]
    pub ops: Option<PathBuf>,
}

/// Turns the flags into radio ops.
pub fn ops_of(a: &RadioSettingsEditArgs) -> Result<Vec<RadioOp>> {
    let mut ops = Vec::new();
    for kv in &a.set {
        let (k, v) = kv
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("{kv:?} is not KEY=VALUE"))?;
        ops.push(RadioOp::SetScalar {
            key: k.trim().to_string(),
            value: v.to_string(),
        });
    }
    if let Some(f) = &a.select_model {
        ops.push(RadioOp::SelectModel { file: f.clone() });
    }
    if let Some(f) = &a.ops {
        let text =
            std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?;
        let more: Vec<RadioOp> =
            serde_json::from_str(&text).context("the ops file is not a list of radio ops")?;
        ops.extend(more);
    }
    Ok(ops)
}

pub fn run(core: &Core, a: RadioSettingsArgs) -> Result<Value> {
    let d = call::gear_radio_settings(
        core,
        api::RadioSettingsParams {
            device: a.device,
            staged: !a.no_staged,
        },
    )?;
    Ok(if a.text {
        Value::String(radio_settings::render_text(&d))
    } else {
        serde_json::to_value(d)?
    })
}

pub fn edit(core: &Core, a: RadioSettingsEditArgs) -> Result<Value> {
    let ops = ops_of(&a)?;
    let change = call::gear_radio_settings_edit(
        core,
        api::RadioSettingsEditParams {
            device: a.device,
            ops,
            editor: None,
        },
    )?;
    Ok(serde_json::to_value(change)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_become_ops() {
        let a = RadioSettingsEditArgs {
            device: "d".into(),
            set: vec!["beepVolume=3".into(), "ownerRegistrationID=ab c".into()],
            select_model: Some("model00.yml".into()),
            ops: None,
        };
        assert_eq!(
            ops_of(&a).unwrap(),
            vec![
                RadioOp::SetScalar {
                    key: "beepVolume".into(),
                    value: "3".into()
                },
                RadioOp::SetScalar {
                    key: "ownerRegistrationID".into(),
                    value: "ab c".into()
                },
                RadioOp::SelectModel {
                    file: "model00.yml".into()
                },
            ]
        );
        let a = RadioSettingsEditArgs {
            device: "d".into(),
            set: vec!["beepVolume".into()],
            select_model: None,
            ops: None,
        };
        assert!(ops_of(&a).unwrap_err().to_string().contains("KEY=VALUE"));
    }
}
