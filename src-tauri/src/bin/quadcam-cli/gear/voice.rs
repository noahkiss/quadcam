//! `quadcam-cli gear voice`: the radio's voice lines and packs, rendering with the provider
//! from the settings, installing a pack, a per-line override, Choose voice (stages one card
//! change; nothing is written to the card), and `build-pack`, the maintainer's tool.

use anyhow::{bail, Result};
use clap::{Args, Subcommand};
use quadcam_lib::api::{self, call};
use quadcam_lib::core::{self, Core};
use quadcam_lib::gear::voice::batch::BatchSettings;
use quadcam_lib::gear::voice::render::RenderSettings;
use serde_json::Value;
use std::io::IsTerminal;
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

#[derive(Args, Default)]
pub struct BatchArgs {
    /// The sentence around each line; {line} stands for it ("The word is {line}.").
    #[arg(long)]
    pub carrier: Option<String>,
    /// Sentences in one batch at most (default 30).
    #[arg(long)]
    pub max_lines: Option<u32>,
    /// Each cut edge moves to the quietest point within this many ms (default 40).
    #[arg(long)]
    pub snap_ms: Option<u32>,
}

impl BatchArgs {
    fn get(&self) -> Option<BatchSettings> {
        if self.carrier.is_none() && self.max_lines.is_none() && self.snap_ms.is_none() {
            return None;
        }
        let d = BatchSettings::default();
        Some(BatchSettings {
            carrier: self.carrier.clone().unwrap_or(d.carrier),
            max_lines: self.max_lines.unwrap_or(d.max_lines),
            snap_ms: self.snap_ms.unwrap_or(d.snap_ms),
            ..d
        })
    }
}

#[derive(Subcommand)]
pub enum KeyCmd {
    /// Store the ElevenLabs key in the Keychain. Reads one line from stdin (hidden when
    /// stdin is a terminal), so the key never sits in argv or shell history.
    Set,
    /// Remove the stored key.
    Delete,
    /// Say whether a key is stored. Never prints the key.
    Status,
}

#[derive(Subcommand)]
pub enum VoiceCmd {
    /// The ElevenLabs API key: set, delete, status.
    Key {
        #[command(subcommand)]
        cmd: KeyCmd,
    },
    /// The line sets that can be rendered (`--set` takes their ids), and whether a key is stored.
    Sets,
    /// The account's ElevenLabs voices.
    Voices,
    /// The account's ElevenLabs models, with USD per 1K characters and the estimated credits a character.
    Models,
    /// The account's remaining ElevenLabs credits.
    Credits,
    /// What rendering line sets would cost, and whether the credits cover it. Makes no paid call.
    Estimate {
        /// Line set ids, comma separated.
        #[arg(long = "set", value_delimiter = ',', required = true)]
        sets: Vec<String>,
        /// A voice name or id from the account.
        #[arg(long)]
        voice: String,
        #[arg(long)]
        model: String,
        /// Only these card paths of the sets, comma separated.
        #[arg(long, value_delimiter = ',')]
        lines: Vec<String>,
        #[command(flatten)]
        batch: BatchArgs,
        #[command(flatten)]
        settings: SettingsArgs,
    },
    /// Render a few lines in every voice and model, cut out of carrier sentences, and write
    /// the WAVs to the cache for listening. Bills ElevenLabs: it needs --confirm.
    Sample {
        /// Voice names or ids, comma separated.
        #[arg(long, value_delimiter = ',', required = true)]
        voices: Vec<String>,
        /// Model ids, comma separated.
        #[arg(long, value_delimiter = ',', required = true)]
        models: Vec<String>,
        /// Line set ids, comma separated (default: sample, the hard lines).
        #[arg(long = "set", value_delimiter = ',')]
        sets: Vec<String>,
        /// Only these card paths of the sets, comma separated.
        #[arg(long, value_delimiter = ',')]
        lines: Vec<String>,
        /// Price it and list nothing else; render nothing.
        #[arg(long)]
        dry_run: bool,
        /// Allow the paid call.
        #[arg(long)]
        confirm: bool,
        /// With --confirm: the digest the run without --confirm printed. Refused when the
        /// plan changed since.
        #[arg(long)]
        digest: Option<String>,
        #[command(flatten)]
        batch: BatchArgs,
        #[command(flatten)]
        settings: SettingsArgs,
    },
    /// Render QuadCam's lines (and your own) with the provider from the settings into a local
    /// pack. A provider that may charge needs --confirm. With --set, render line sets as
    /// carrier-sentence batches with ElevenLabs instead.
    Render {
        /// The provider's voice (default: the tts_voice setting).
        #[arg(long)]
        voice: Option<String>,
        /// Card paths to render, comma separated (default: every line).
        #[arg(long, value_delimiter = ',')]
        lines: Vec<String>,
        /// Line set ids, comma separated: a batched ElevenLabs render of those sets.
        #[arg(long = "set", value_delimiter = ',')]
        sets: Vec<String>,
        /// The model for --set (default: the tts_model setting).
        #[arg(long)]
        model: Option<String>,
        #[command(flatten)]
        batch: BatchArgs,
        /// Report the plan and the characters; render nothing.
        #[arg(long)]
        dry_run: bool,
        /// Allow a render that sends text to a provider that may charge.
        #[arg(long)]
        confirm: bool,
        /// With --set and --confirm: the digest the run without --confirm printed. Refused
        /// when the plan changed since.
        #[arg(long)]
        digest: Option<String>,
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
        /// With --text: report the characters and the digest; change nothing.
        #[arg(long)]
        dry_run: bool,
        /// Allow a render that sends text to a provider that may charge (needs --digest).
        #[arg(long)]
        confirm: bool,
        /// With --text and --confirm: the digest the --dry-run printed. Refused when the
        /// text, voice or provider changed since.
        #[arg(long)]
        digest: Option<String>,
    },
    /// Stage one card change that puts a pack's sounds on a radio, or on each of several
    /// radios (--radio again, or --all-radios).
    Choose {
        /// A saved radio's device id; repeat it, or separate ids with commas, for several.
        #[arg(long, value_delimiter = ',')]
        radio: Vec<String>,
        /// Every saved EdgeTX radio.
        #[arg(long)]
        all_radios: bool,
        #[arg(long)]
        pack: String,
        /// Clear the lines you overrode instead of keeping them.
        #[arg(long)]
        drop_overrides: bool,
    },
    /// Remove an installed or rendered pack from this Mac and clear the radios' choice of it.
    /// Cards keep its sounds. Needs --yes.
    Delete {
        /// The pack id.
        pack: String,
        /// Also remove the raw takes it was made from; a later render of that voice and model
        /// then calls the provider again.
        #[arg(long)]
        takes: bool,
        /// Delete it.
        #[arg(long)]
        yes: bool,
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

fn catalog(core: &Core, voices: bool, models: bool, credits: bool) -> Result<Value> {
    Ok(serde_json::to_value(call::gear_voice_catalog(
        core,
        api::CatalogParams {
            voices,
            models,
            credits,
        },
    )?)?)
}

/// One line from stdin. A terminal gets a prompt and no echo.
fn read_key() -> Result<String> {
    let tty = std::io::stdin().is_terminal();
    let stty = |on: bool| {
        let _ = std::process::Command::new("/bin/stty")
            .arg(if on { "echo" } else { "-echo" })
            .stdin(std::process::Stdio::inherit())
            .status();
    };
    if tty {
        eprint!("ElevenLabs API key (not shown): ");
        stty(false);
    }
    let mut line = String::new();
    let r = std::io::stdin().read_line(&mut line);
    if tty {
        stty(true);
        eprintln!();
    }
    r?;
    let key = line.trim().to_string();
    if key.is_empty() {
        bail!("No key was given: pipe it in, or type it at the prompt.");
    }
    Ok(key)
}

/// Saved devices' ids as "name (id)", for a refusal that lists them.
fn radio_names(core: &Core, ids: &[String]) -> Result<Vec<String>> {
    let devices = call::gear_devices(core)?;
    Ok(ids
        .iter()
        .map(|id| match devices.iter().find(|d| &d.id == id) {
            Some(d) if !d.name.is_empty() => format!("{} ({id})", d.name),
            _ => id.clone(),
        })
        .collect())
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
        Some(VoiceCmd::Key { cmd }) => {
            let (action, key) = match cmd {
                KeyCmd::Set => ("set", Some(read_key()?)),
                KeyCmd::Delete => ("delete", None),
                KeyCmd::Status => ("status", None),
            };
            serde_json::to_value(call::gear_voice_key(
                core,
                api::KeyParams {
                    action: action.into(),
                    key,
                },
            )?)?
        }
        Some(VoiceCmd::Sets) => serde_json::to_value(call::gear_voice_sets(core)?)?,
        Some(VoiceCmd::Voices) => catalog(core, true, false, false)?,
        Some(VoiceCmd::Models) => catalog(core, false, true, false)?,
        Some(VoiceCmd::Credits) => catalog(core, false, false, true)?,
        Some(VoiceCmd::Estimate {
            sets,
            voice,
            model,
            lines,
            batch,
            settings,
        }) => serde_json::to_value(call::gear_voice_estimate(
            core,
            api::EstimateParams {
                sets,
                voice,
                model,
                lines,
                settings: settings.get(),
                batch: batch.get(),
            },
        )?)?,
        Some(VoiceCmd::Sample {
            voices,
            models,
            sets,
            lines,
            dry_run,
            confirm,
            digest,
            batch,
            settings,
        }) => serde_json::to_value(call::gear_voice_sample(
            core,
            api::SampleParams {
                voices,
                models,
                sets,
                lines,
                dry_run,
                confirm,
                settings: settings.get(),
                batch: batch.get(),
                digest,
            },
        )?)?,
        Some(VoiceCmd::Render {
            voice,
            lines,
            sets,
            model,
            batch,
            dry_run,
            confirm,
            digest,
            settings,
        }) => serde_json::to_value(call::gear_voice_render(
            core,
            api::VoiceRenderParams {
                voice: voice.unwrap_or_default(),
                lines,
                dry_run,
                confirm,
                settings: settings.get(),
                sets,
                model: model.unwrap_or_default(),
                batch: batch.get(),
                digest,
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
            dry_run,
            confirm,
            digest,
        }) => serde_json::to_value(call::gear_voice_edit(
            core,
            api::VoiceEditParams {
                radio,
                line,
                text,
                pack,
                confirm,
                dry_run,
                digest,
            },
        )?)?,
        Some(VoiceCmd::Choose {
            mut radio,
            all_radios,
            pack,
            drop_overrides,
        }) => {
            if radio.len() == 1 && !all_radios {
                serde_json::to_value(call::gear_voice_choose(
                    core,
                    api::VoiceChooseParams {
                        radio: radio.remove(0),
                        pack,
                        keep_overrides: !drop_overrides,
                        editor: None,
                    },
                )?)?
            } else {
                serde_json::to_value(call::gear_voice_choose_radios(
                    core,
                    api::VoiceChooseRadiosParams {
                        pack,
                        radios: radio,
                        all: all_radios,
                        keep_overrides: !drop_overrides,
                        editor: None,
                    },
                )?)?
            }
        }
        Some(VoiceCmd::Delete { pack, takes, yes }) => {
            if !yes {
                let view = call::gear_voice(core, api::VoiceParams::default())?;
                let radios = view
                    .packs
                    .iter()
                    .find(|k| k.id == pack && k.installed)
                    .map(|k| radio_names(core, &k.radios))
                    .transpose()?
                    .unwrap_or_default();
                bail!(
                    "Refused: delete needs --yes. It removes pack {pack} from this Mac.{}",
                    if radios.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " Radios that chose it: {}. The delete clears their choice; their cards keep its sounds.",
                            radios.join(", ")
                        )
                    }
                );
            }
            serde_json::to_value(call::gear_voice_pack_delete(
                core,
                api::PackDeleteParams { pack, takes },
            )?)?
        }
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
