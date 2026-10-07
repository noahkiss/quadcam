//! `quadcam-cli gear osd`: an FC's OSD layout per OSD profile, drawn and checked.

use anyhow::Result;
use clap::Args;
use quadcam_lib::api::{self, call};
use quadcam_lib::core::Core;
use quadcam_lib::gear::osd;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Args)]
pub struct OsdArgs {
    /// Betaflight `dump all`, `diff all` or CLI-line files, read in order (a later file's
    /// lines win), or one saved device id.
    #[arg(required = true)]
    pub target: Vec<String>,
    /// The grid: NTSC, PAL, HD or WxH (default: the files' vcd_video_system).
    #[arg(long)]
    pub grid: Option<String>,
    /// Print the drawn profiles as text instead of the structured view.
    #[arg(long)]
    pub text: bool,
}

pub fn run(core: &Core, a: OsdArgs) -> Result<Value> {
    // One argument that is not a file is a device id.
    let params = match a.target.as_slice() {
        [one] if !std::path::Path::new(one).exists() => api::OsdParams {
            paths: Vec::new(),
            device: Some(one.clone()),
            grid: a.grid,
        },
        _ => api::OsdParams {
            paths: a.target.iter().map(PathBuf::from).collect(),
            device: None,
            grid: a.grid,
        },
    };
    let view = call::gear_osd(core, params)?;
    Ok(if a.text {
        Value::String(osd::render_text(&view))
    } else {
        serde_json::to_value(view)?
    })
}
