//! `quadcam-cli gear firmware|splash`: the firmware check, the splash preview, and the
//! EdgeTX flash (design 6.5, 7.5, 8). `firmware --plan` runs every guard and prints the
//! digest; `firmware --digest D --yes` flashes. Nothing here flashes without both, and
//! a process started by cargo never reaches a real USB device.

use anyhow::{bail, Context, Result};
use base64::Engine;
use clap::Args;
use quadcam_lib::api::{self, call};
use quadcam_lib::core::Core;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Args)]
pub struct FirmwareArgs {
    /// Read the network now: the newest EdgeTX, Betaflight and ExpressLRS releases.
    /// Without it, the last saved answer (or, with firmwareCheck daily, a fresh one when
    /// the saved one is a day old).
    #[arg(long)]
    check: bool,
    /// Run every guard for a flash and print the checks, the diff and the digest. Needs
    /// --device. Downloads the release on first use; writes no device.
    #[arg(long)]
    plan: bool,
    /// The EdgeTX radio to flash (a saved device id).
    #[arg(long)]
    device: Option<String>,
    /// The EdgeTX version to flash. Default: the version the radio reports.
    #[arg(long)]
    version: Option<String>,
    /// A PNG to make the radio's splash screen.
    #[arg(long)]
    splash: Option<PathBuf>,
    /// Splash: grey values under this (0-255) are dark. Default 128.
    #[arg(long)]
    threshold: Option<u8>,
    /// Splash: swap dark and light.
    #[arg(long)]
    invert: bool,
    /// The digest `--plan` printed. Flashes the radio, which must be in DFU mode.
    #[arg(long)]
    digest: Option<String>,
    #[arg(long)]
    yes: bool,
}

#[derive(Args)]
pub struct SplashArgs {
    /// A PNG file.
    image: PathBuf,
    /// Grey values under this (0-255) are dark. Default 128.
    #[arg(long)]
    threshold: Option<u8>,
    /// Swap dark and light.
    #[arg(long)]
    invert: bool,
    /// The radio's board (pocket); a colour radio is refused.
    #[arg(long)]
    board: Option<String>,
    /// Write the preview (a PNG at 4 times the size) here.
    #[arg(long)]
    out: Option<PathBuf>,
}

fn splash_params(
    image: PathBuf,
    threshold: Option<u8>,
    invert: bool,
    board: Option<String>,
) -> api::SplashParams {
    api::SplashParams {
        image,
        threshold,
        invert,
        board,
    }
}

pub fn firmware(core: &Core, a: FirmwareArgs) -> Result<Value> {
    if a.plan || a.digest.is_some() {
        let device = a
            .device
            .clone()
            .context("--device is required: the radio's saved device id (gear devices)")?;
        let params = api::FlashParams {
            device,
            version: a.version.clone(),
            splash: a
                .splash
                .clone()
                .map(|i| splash_params(i, a.threshold, a.invert, None)),
        };
        let Some(digest) = a.digest else {
            return Ok(serde_json::to_value(call::gear_flash_plan(core, params)?)?);
        };
        if !a.yes {
            bail!("Refused: a flash needs --digest from `--plan` and --yes.");
        }
        return Ok(serde_json::to_value(call::gear_flash(
            core,
            api::FlashRequest {
                params,
                digest,
                confirm: true,
            },
        )?)?);
    }
    Ok(serde_json::to_value(call::gear_firmware(
        core,
        api::FirmwareParams {
            check: a.check.then_some(true),
        },
    )?)?)
}

pub fn splash(core: &Core, a: SplashArgs) -> Result<Value> {
    let mut p = call::gear_splash(core, splash_params(a.image, a.threshold, a.invert, a.board))?;
    if let Some(out) = a.out {
        let png = base64::engine::general_purpose::STANDARD
            .decode(&p.png_base64)
            .context("the preview is not base64")?;
        std::fs::write(&out, png).with_context(|| format!("writing {}", out.display()))?;
    }
    // The PNG goes to --out; the JSON stays small.
    p.png_base64 = String::new();
    Ok(serde_json::to_value(p)?)
}
