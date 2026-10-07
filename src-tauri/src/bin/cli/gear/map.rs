//! `quadcam-cli gear map`: the switch map, and `gear radio`: the radio in USB Joystick mode.

use anyhow::Result;
use clap::Args;
use quadcam_lib::api::{self, call};
use quadcam_lib::core::Core;
use quadcam_lib::gear::switchmap;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Args)]
pub struct MapArgs {
    /// An EdgeTX card (mount or folder) or one model file.
    #[arg(long)]
    pub radio: Option<PathBuf>,
    /// The model file on the card (model01.yml); default: the radio's selected model.
    #[arg(long)]
    pub model: Option<String>,
    /// Betaflight dump, diff or CLI files, read in order (a later file's lines win).
    #[arg(long = "fc")]
    pub fc: Vec<PathBuf>,
    /// An aircraft profile: the latest backups of its saved radio and FC.
    #[arg(long)]
    pub aircraft: Option<String>,
    /// A saved device whose latest backup to read (an FC or a radio); repeat for both.
    #[arg(long = "device")]
    pub devices: Vec<String>,
    /// Mark where each control is now: the FC's channels, else the radio's joystick.
    #[arg(long)]
    pub live: bool,
    /// The FC's port for --live; omit when one FC is plugged in.
    #[arg(long)]
    pub port: Option<String>,
    /// Channel values to mark instead, µs, CH1 first (1500,1500,988,...).
    #[arg(long, value_delimiter = ',')]
    pub channels: Vec<u16>,
    /// Print the map as text instead of the structured view.
    #[arg(long)]
    pub text: bool,
}

#[derive(Args)]
pub struct RadioArgs {
    /// Milliseconds to wait for a report (default 500).
    #[arg(long)]
    pub wait_ms: Option<u32>,
}

pub fn run(core: &Core, a: MapArgs) -> Result<Value> {
    let map = call::gear_switch_map(
        core,
        api::SwitchMapParams {
            radio: a.radio,
            model: a.model,
            fc: a.fc,
            aircraft: a.aircraft,
            devices: a.devices,
            live: a.live,
            port: a.port,
            channels: a.channels,
        },
    )?;
    Ok(if a.text {
        Value::String(switchmap::render_text(&map))
    } else {
        serde_json::to_value(map)?
    })
}

pub fn radio(core: &Core, a: RadioArgs) -> Result<Value> {
    Ok(serde_json::to_value(call::gear_radio(
        core,
        api::RadioParams { wait_ms: a.wait_ms },
    )?)?)
}
