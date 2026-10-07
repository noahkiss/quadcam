//! `quadcam-cli gear card`: read an EdgeTX card, and preview card edits (nothing written).

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use quadcam_lib::api::{self, call};
use quadcam_lib::core::Core;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Args)]
pub struct CardArgs {
    /// The card's mount point. Default: the one EdgeTX card mounted.
    #[arg(long, global = true)]
    mount: Option<PathBuf>,
    /// A connected radio's device id, instead of --mount.
    #[arg(long, global = true)]
    device: Option<String>,
    /// A model file (model01.yml) to show in full.
    #[arg(long)]
    model: Option<String>,
    #[command(subcommand)]
    cmd: Option<CardCmd>,
}

#[derive(Subcommand)]
pub enum CardCmd {
    /// Check and diff card edits from a JSON file (a list of edits). Writes nothing.
    Preview {
        /// The edits file.
        #[arg(long)]
        edits: PathBuf,
    },
}

pub fn run(core: &Core, a: CardArgs) -> Result<Value> {
    Ok(match a.cmd {
        None => serde_json::to_value(call::gear_card(
            core,
            api::CardParams {
                mount: a.mount,
                device: a.device,
                model: a.model,
            },
        )?)?,
        Some(CardCmd::Preview { edits }) => {
            let text = std::fs::read_to_string(&edits)
                .with_context(|| format!("reading {}", edits.display()))?;
            let edits =
                serde_json::from_str(&text).context("the edits file is not a list of edits")?;
            serde_json::to_value(call::gear_card_preview(
                core,
                api::CardPreviewParams {
                    mount: a.mount,
                    device: a.device,
                    edits,
                },
            )?)?
        }
    })
}
