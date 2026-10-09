//! `quadcam-cli gear voice`: the radio's voice lines and packs, rendering with the provider
//! from the settings, installing a pack, a per-line override, Choose voice (stages one card
//! change; nothing is written to the card), and `build-pack`, the maintainer's tool.

use anyhow::Result;
use clap::{Args, Subcommand};
use quadcam_lib::api::{self, call};
use quadcam_lib::core::{self, Core};
use quadcam_lib::gear::voice::render::RenderSettings;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Args)]
pub struct VoiceArgs {
    /// Read the pack index again from the voice_index setting.
    #[arg(long)]
    pub refresh: bool,
    /// A saved radio's device id: shows its overrides and the voice chosen for it.
    #[arg(long)]
    pub radio: Option<String>,
    /// Print as text instead of the structured view.
    #[arg(long)]
    pub text: bool,
    #[command(subcommand)]
    pub cmd: Option<VoiceCmd>,
}

#[derive(Args, Default)]
pub struct SettingsArgs {
    /// The provider's speaking speed (default 1).
    #[arg(long)]
    pub speed: Option<f64>,
    /// atempo after the take, 0.5 to 2 (default 1).
    #[arg(long)]
    pub tempo: Option<f64>,
    /// Speech is every 5 ms window within this many dB of the peak (default -55).
    #[arg(long, allow_hyphen_values = true)]
    pub trim_db: Option<f64>,
    /// Silence after the speech, in ms (default 150).
    #[arg(long)]
    pub tail_ms: Option<u32>,
    /// Fade out at the end of the speech, in ms (default 30).
    #[arg(long)]
    pub fade_out_ms: Option<u32>,
    /// Fixes a take where the provider can (default 0).
    #[arg(long)]
    pub seed: Option<u64>,
}

impl SettingsArgs {
    fn get(&self) -> Option<RenderSettings> {
        let none = self.speed.is_none()
            && self.tempo.is_none()
            && self.trim_db.is_none()
            && self.tail_ms.is_none()
            && self.fade_out_ms.is_none()
            && self.seed.is_none();
        if none {
            return None;
        }
        let d = RenderSettings::default();
        Some(RenderSettings {
            speed: self.speed.unwrap_or(d.speed),
            tempo: self.tempo.unwrap_or(d.tempo),
            trim_db: self.trim_db.unwrap_or(d.trim_db),
            tail_ms: self.tail_ms.unwrap_or(d.tail_ms),
            fade_out_ms: self.fade_out_ms.unwrap_or(d.fade_out_ms),
            seed: self.seed.unwrap_or(d.seed),
            ..d
        })
    }
}

#[derive(Subcommand)]
pub enum VoiceCmd {
    /// Render QuadCam's lines (and your own) with the provider from the settings into a local
    /// pack. A provider that may charge needs --confirm.
    Render {
        /// The provider's voice (default: the tts_voice setting).
        #[arg(long)]
        voice: Option<String>,
        /// Card paths to render, comma separated (default: every line).
        #[arg(long, value_delimiter = ',')]
        lines: Vec<String>,
        /// Report the plan and the characters; render nothing.
        #[arg(long)]
        dry_run: bool,
        /// Allow a render that sends text to a provider that may charge.
        #[arg(long)]
        confirm: bool,
        #[command(flatten)]
        settings: SettingsArgs,
    },
    /// Install a pack from the index after a hash check.
    Install {
        /// The pack id.
        pack: String,
        /// The index (a path or an address) when the voice_index setting names none.
        #[arg(long)]
        source: Option<String>,
    },
    /// Override one line on one radio: another pack's take (--pack), or your own text
    /// rendered with your provider (--text); neither clears it.
    Edit {
        /// The radio's device id.
        #[arg(long)]
        radio: String,
        /// The sound's card path, SOUNDS/en/armed.wav.
        #[arg(long)]
        line: String,
        #[arg(long)]
        text: Option<String>,
        #[arg(long)]
        pack: Option<String>,
        /// Allow a render that sends text to a provider that may charge.
        #[arg(long)]
        confirm: bool,
    },
    /// Stage one card change that puts a pack's sounds on a radio.
    Choose {
        #[arg(long)]
        radio: String,
        #[arg(long)]
        pack: String,
        /// Clear the lines you overrode instead of keeping them.
        #[arg(long)]
        drop_overrides: bool,
    },
    /// Maintainer tool: render QuadCam's lines into a pack zip and a voices.json entry.
    BuildPack {
        /// The provider's voice.
        #[arg(long)]
        voice: String,
        /// Where the zip and voices.json go.
        #[arg(long)]
        out: PathBuf,
        /// The pack id (default <lang>-<voice>-v1).
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        version: Option<String>,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long)]
        license: Option<String>,
        #[arg(long)]
        attribution: Option<String>,
        /// Report the plan and the characters; build nothing.
        #[arg(long)]
        dry_run: bool,
        /// Allow a render that sends text to a provider that may charge.
        #[arg(long)]
        confirm: bool,
        #[command(flatten)]
        settings: SettingsArgs,
    },
}

pub fn run(core: &Core, a: VoiceArgs) -> Result<Value> {
    Ok(match a.cmd {
        None => {
            let v = call::gear_voice(
                core,
                api::VoiceParams {
                    radio: a.radio,
                    refresh_index: a.refresh,
                },
            )?;
            if a.text {
                Value::String(core::voice_view_text(&v))
            } else {
                serde_json::to_value(v)?
            }
        }
        Some(VoiceCmd::Render {
            voice,
            lines,
            dry_run,
            confirm,
            settings,
        }) => serde_json::to_value(call::gear_voice_render(
            core,
            api::VoiceRenderParams {
                voice: voice.unwrap_or_default(),
                lines,
                dry_run,
                confirm,
                settings: settings.get(),
            },
        )?)?,
        Some(VoiceCmd::Install { pack, source }) => serde_json::to_value(
            call::gear_voice_pack_install(core, api::PackInstallParams { pack, source })?,
        )?,
        Some(VoiceCmd::Edit {
            radio,
            line,
            text,
            pack,
            confirm,
        }) => serde_json::to_value(call::gear_voice_edit(
            core,
            api::VoiceEditParams {
                radio,
                line,
                text,
                pack,
                confirm,
            },
        )?)?,
        Some(VoiceCmd::Choose {
            radio,
            pack,
            drop_overrides,
        }) => serde_json::to_value(call::gear_voice_choose(
            core,
            api::VoiceChooseParams {
                radio,
                pack,
                keep_overrides: !drop_overrides,
                editor: None,
            },
        )?)?,
        Some(VoiceCmd::BuildPack {
            voice,
            out,
            id,
            version,
            lang,
            license,
            attribution,
            dry_run,
            confirm,
            settings,
        }) => {
            let (zip, entry, plan) = core.voice_build_pack(&core::BuildPackParams {
                voice,
                id,
                version,
                lang,
                out,
                license,
                attribution,
                settings: settings.get(),
                confirm,
                dry_run,
            })?;
            serde_json::json!({
                "dry_run": dry_run,
                "zip": if dry_run { Value::Null } else { Value::String(zip.display().to_string()) },
                "entry": if dry_run { Value::Null } else { serde_json::to_value(entry)? },
                "plan": plan,
            })
        }
    })
}
