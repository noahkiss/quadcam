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
    /// A model from another radio (a card's mount or folder, a model file, or a radio
    /// backup id) as a new model on this card: its controls mapped onto this radio's, and
    /// the preview. Writes nothing; stage the printed edit with `gear stage --edits`.
    Import {
        /// Where the model comes from.
        #[arg(long)]
        from: String,
        /// The model file in --from (model01.yml); default its selected model.
        #[arg(long = "from-model")]
        from_model: Option<String>,
        /// The source radio's board, for a model file with no radio.yml beside it.
        #[arg(long)]
        from_board: Option<String>,
        /// The new model file (model03.yml); default the first free one.
        #[arg(long)]
        to: Option<String>,
        /// The new model's name; default the source's.
        #[arg(long)]
        name: Option<String>,
        /// A link in place of the default, SOURCE=TARGET (SE=SH); repeat for more.
        #[arg(long = "link")]
        links: Vec<String>,
        /// The model on this card whose custom-switch setup the new model takes; default
        /// the selected model.
        #[arg(long)]
        template: Option<String>,
    },
}

/// `SE=SH` as a link.
pub fn parse_link(s: &str) -> Result<quadcam_lib::gear::edgetx::board::ControlLink> {
    let (from, to) = s
        .split_once('=')
        .with_context(|| format!("{s:?} is not a link: SOURCE=TARGET, such as SE=SH"))?;
    Ok(quadcam_lib::gear::edgetx::board::ControlLink {
        from: from.trim().to_string(),
        to: to.trim().to_string(),
    })
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
        Some(CardCmd::Import {
            from,
            from_model,
            from_board,
            to,
            name,
            links,
            template,
        }) => serde_json::to_value(call::gear_model_import(
            core,
            api::ModelImportParams {
                mount: a.mount,
                device: a.device,
                from,
                model: from_model,
                from_board,
                to,
                name,
                links: links.iter().map(|l| parse_link(l)).collect::<Result<_>>()?,
                template,
            },
        )?)?,
    })
}
