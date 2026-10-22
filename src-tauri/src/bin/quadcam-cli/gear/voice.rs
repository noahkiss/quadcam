//! `quadcam-cli gear voice`: the radio's voice lines and packs, rendering with the provider
//! from the settings, installing a pack, a per-line override, Choose voice (stages one card
//! change; nothing is written to the card), `publish` (prepare a pack for the voices repo;
//! `--approve` only at a terminal), and `build-pack`, the maintainer's tool.

use anyhow::{bail, Result};
use clap::{Args, Subcommand};
use quadcam_lib::api::{self, call};
use quadcam_lib::core::{self, Core};
use quadcam_lib::gear::voice::batch::BatchSettings;
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

#[derive(Args, Default)]
pub struct BatchArgs {
    /// The sentence around each line; {line} stands for it ("The word is {line}.").
    #[arg(long)]
    pub carrier: Option<String>,
    /// Sentences in one batch at most (default 30).
    #[arg(long)]
    pub max_lines: Option<u32>,
    /// Each cut edge looks for the quiet at least this many ms either side of its timestamp
    /// (default 40; 0 cuts at the timestamps).
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
    /// The line sets that can be rendered (`--set` takes their ids), the aircraft you fly
    /// with the sets picked at first, and whether a key is stored. With --voice and --model,
    /// how many lines of each set already have a take. Makes no call.
    Sets {
        /// A voice id from the account (`gear voice voices`).
        #[arg(long)]
        voice: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[command(flatten)]
        batch: BatchArgs,
        #[command(flatten)]
        settings: SettingsArgs,
    },
    /// The account's ElevenLabs voices.
    Voices,
    /// The account's ElevenLabs models, with USD per 1K characters and the estimated credits a character.
    Models,
    /// The account's remaining ElevenLabs credits.
    Credits,
    /// Search ElevenLabs' Voice Library (free calls): name, gender, age, accent, languages, use
    /// case, any rate of its own, and whether the account has it. --id looks up one voice.
    Library {
        /// Words in the name, description or labels.
        #[arg(long)]
        search: Option<String>,
        /// One voice by id: the account's own or a library voice.
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        gender: Option<String>,
        /// young, middle_aged or old.
        #[arg(long)]
        age: Option<String>,
        #[arg(long)]
        accent: Option<String>,
        /// A language code such as en.
        #[arg(long)]
        language: Option<String>,
        /// narrative_story, conversational, characters_animation, social_media and so on.
        #[arg(long)]
        use_case: Option<String>,
        /// professional, famous or high_quality.
        #[arg(long)]
        category: Option<String>,
        /// Words such as calm,deep, comma separated.
        #[arg(long, value_delimiter = ',')]
        descriptives: Vec<String>,
        /// created_date, usage_character_count_1y, trending or cloned_by_count.
        #[arg(long)]
        sort: Option<String>,
        /// The page, from 0.
        #[arg(long, default_value_t = 0)]
        page: u32,
        /// Voices a page, 1 to 100.
        #[arg(long, default_value_t = 30)]
        page_size: u32,
    },
    /// Add a Voice Library voice to the account's My Voices. It changes the account and takes a
    /// voice slot (no credits): without --yes it only shows what would be added.
    Add {
        /// The library voice's id (`gear voice library`).
        voice: String,
        /// The name in the account (default the library's).
        #[arg(long)]
        name: Option<String>,
        /// Add it.
        #[arg(long)]
        yes: bool,
    },
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
    /// Copy earlier ElevenLabs takes of single lines from a folder into the cache, so the
    /// studio counts them as rendered and never sends their lines again at that voice, model
    /// and speed. A take is a .json (text, voice, model, speed, seed) beside a .pcm of the same
    /// name (16-bit mono, 32000 Hz unless the JSON names a rate). Reads the folder only.
    ImportTakes {
        folder: PathBuf,
        /// Report what would be copied; copy nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Render a few lines in every voice and model, cut out of carrier sentences, and keep
    /// the WAVs in the sample gallery (`gear voice samples`). Bills ElevenLabs: it needs --confirm.
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
    /// The sample gallery: each sampled voice with its takes, what the speech check heard and
    /// any flag, then the packs on this Mac. --delete or --all with --yes removes samples (the
    /// takes stay in the cache).
    Samples {
        /// Sample ids to delete, comma separated (needs --yes).
        #[arg(long, value_delimiter = ',')]
        delete: Vec<String>,
        /// Delete every sample (needs --yes).
        #[arg(long)]
        all: bool,
        /// Delete them.
        #[arg(long)]
        yes: bool,
        /// Print as text.
        #[arg(long)]
        text: bool,
    },
    /// Every flagged take: voice, model, speed, seed, line, text, heard text, file, reason, note.
    Flags {
        /// Print as text.
        #[arg(long)]
        text: bool,
    },
    /// Flag a take of a sample or a pack with a reason and a note, or clear its flag.
    Flag {
        /// The take's card path, such as SOUNDS/en/armed.wav.
        line: String,
        /// A sample id (`gear voice samples`).
        #[arg(long, conflicts_with = "pack", required_unless_present = "pack")]
        sample: Option<String>,
        /// A pack on this Mac.
        #[arg(long)]
        pack: Option<String>,
        /// mispronounced, clipped, wrong_tone, noise or other.
        #[arg(long, required_unless_present = "clear")]
        reason: Option<String>,
        /// What is wrong with the take.
        #[arg(long)]
        note: Option<String>,
        /// Remove the flag instead.
        #[arg(long)]
        clear: bool,
    },
    /// Make the flagged takes of one sample voice or one studio pack again with another seed,
    /// or a studio pack's re-take lines named in --lines. Bills ElevenLabs: without --confirm it prices them and prints the digest.
    Retake {
        /// A sample id (`gear voice samples`).
        #[arg(long, conflicts_with = "pack", required_unless_present = "pack")]
        sample: Option<String>,
        /// A pack the Voice studio rendered.
        #[arg(long)]
        pack: Option<String>,
        /// Only these card paths, comma separated: flagged lines, or a pack's re-take lines
        /// (default every flagged line).
        #[arg(long, value_delimiter = ',')]
        lines: Vec<String>,
        /// Price it; render nothing.
        #[arg(long)]
        dry_run: bool,
        /// Allow the paid call.
        #[arg(long)]
        confirm: bool,
        /// With --confirm: the digest the run without --confirm printed. Refused when the
        /// plan changed since.
        #[arg(long)]
        digest: Option<String>,
        /// A pack's re-take lines, or a sample voice's lines heard wrong, grouped by the word
        /// each got wrong: one line per group is tried in other ways (seed, carrier, spelling)
        /// until the speech check hears it right, then the rest of the group is made that way.
        /// The price is the most it can cost.
        #[arg(long)]
        grouped: bool,
        /// With --grouped: the ways one group's line is tried at most (default 6, at most 12).
        #[arg(long, requires = "grouped")]
        tries: Option<u32>,
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
    /// Render line sets with ElevenLabs into one local pack per voice and model, priced as one
    /// plan. Without --confirm it prints each pair's cost, the total and the digest. Bills
    /// ElevenLabs: it needs --confirm and --digest. A failure keeps the packs made before it.
    RenderPacks {
        /// Voice names or ids, comma separated.
        #[arg(long, value_delimiter = ',', required = true)]
        voices: Vec<String>,
        /// Model ids, comma separated.
        #[arg(long, value_delimiter = ',', required = true)]
        models: Vec<String>,
        /// Line set ids, comma separated.
        #[arg(long = "set", value_delimiter = ',', required = true)]
        sets: Vec<String>,
        /// Only these card paths of the sets, comma separated.
        #[arg(long, value_delimiter = ',')]
        lines: Vec<String>,
        /// Price it; render nothing.
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
    /// Hear each line of a pack with the on-device speech recognizer (macOS 26 or later) and
    /// compare it with the line's text. In a pack the studio rendered, a line heard wrong
    /// leaves the pack as a re-take. Calls no provider.
    Check {
        /// The pack id.
        pack: String,
        /// Only these card paths, comma separated. A re-take line is heard again from its held
        /// take and goes back into the pack when heard right.
        #[arg(long, value_delimiter = ',')]
        lines: Vec<String>,
        /// Keep what was heard for these card paths, comma separated: each passes, and goes
        /// back into the pack. Hears nothing.
        #[arg(long, value_delimiter = ',')]
        accept: Vec<String>,
        /// First install the recognizer's assets for the pack's language (macOS downloads them).
        #[arg(long)]
        install: bool,
    },
    /// Fetch an index pack's preview lines into the cache, each checked against its hash.
    Preview {
        /// The pack id from the index.
        pack: String,
        /// The index (a path or an address) when the voice_index setting names none.
        #[arg(long)]
        source: Option<String>,
    },
    /// Prepare a pack on this Mac for the QuadCam voices repo and print the review: every line
    /// with what the speech check heard, and a spot check to listen to. Publishes nothing.
    /// With --approve, at a terminal only: publishes the prepared pack the digest names.
    Publish {
        /// The pack on this Mac (local-...).
        pack: String,
        /// The published pack's id (default <lang>-<voice name>).
        #[arg(long)]
        id: Option<String>,
        /// The pack's version (default 1).
        #[arg(long)]
        version: Option<String>,
        /// The voice's name in the index (default the pack's).
        #[arg(long)]
        voice_name: Option<String>,
        /// The pack's license (ElevenLabs and Kokoro packs get theirs filled in).
        #[arg(long)]
        license: Option<String>,
        #[arg(long)]
        attribution: Option<String>,
        /// The GitHub repo, owner/name (default the QuadCam voices repo).
        #[arg(long)]
        repo: Option<String>,
        /// Publish the prepared pack: creates the release and the index pull request with gh,
        /// or prints the commands. Asks you to type the pack's id at the terminal.
        #[arg(long, requires = "digest")]
        approve: bool,
        /// The digest the review printed.
        #[arg(long)]
        digest: Option<String>,
        /// Print the review as text.
        #[arg(long)]
        text: bool,
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

/// The key from one line of stdin (`read_secret`).
fn read_key() -> Result<String> {
    let key = crate::read_secret("ElevenLabs API key")?.trim().to_string();
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
        Some(VoiceCmd::Sets {
            voice,
            model,
            batch,
            settings,
        }) => serde_json::to_value(call::gear_voice_sets(
            core,
            api::SetsParams {
                voice: voice.unwrap_or_default(),
                model: model.unwrap_or_default(),
                settings: settings.get(),
                batch: batch.get(),
            },
        )?)?,
        Some(VoiceCmd::Voices) => catalog(core, true, false, false)?,
        Some(VoiceCmd::Models) => catalog(core, false, true, false)?,
        Some(VoiceCmd::Credits) => catalog(core, false, false, true)?,
        Some(VoiceCmd::Library {
            search,
            id,
            gender,
            age,
            accent,
            language,
            use_case,
            category,
            descriptives,
            sort,
            page,
            page_size,
        }) => serde_json::to_value(call::gear_voice_library(
            core,
            api::LibraryParams {
                query: quadcam_lib::gear::voice::tts::LibraryQuery {
                    search: search.unwrap_or_default(),
                    gender: gender.unwrap_or_default(),
                    age: age.unwrap_or_default(),
                    accent: accent.unwrap_or_default(),
                    language: language.unwrap_or_default(),
                    use_case: use_case.unwrap_or_default(),
                    category: category.unwrap_or_default(),
                    descriptives,
                    sort: sort.unwrap_or_default(),
                    page,
                    page_size,
                },
                id: id.unwrap_or_default(),
            },
        )?)?,
        Some(VoiceCmd::Add { voice, name, yes }) => serde_json::to_value(call::gear_voice_add(
            core,
            api::VoiceAddParams {
                voice,
                owner: String::new(),
                name: name.unwrap_or_default(),
                confirm: yes,
            },
        )?)?,
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
        Some(VoiceCmd::Check {
            pack,
            lines,
            accept,
            install,
        }) => serde_json::to_value(call::gear_voice_check(
            core,
            api::VoiceCheckParams {
                pack,
                lines,
                accept,
                install,
            },
        )?)?,
        Some(VoiceCmd::ImportTakes { folder, dry_run }) => {
            serde_json::to_value(call::gear_voice_import_takes(
                core,
                api::ImportTakesParams {
                    folder: folder.display().to_string(),
                    dry_run,
                },
            )?)?
        }
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
        Some(VoiceCmd::Samples {
            delete,
            all,
            yes,
            text,
        }) => {
            let g = if all || !delete.is_empty() {
                if !yes {
                    bail!(
                        "Not deleted: {} would be removed (the takes stay in the cache). Run again with --yes.",
                        if all { "every sample".to_string() } else { delete.join(", ") }
                    );
                }
                call::gear_voice_samples_delete(
                    core,
                    api::SamplesDeleteParams { ids: delete, all },
                )?
            } else {
                call::gear_voice_samples(core)?
            };
            if text {
                Value::String(core::voice_samples_text(&g))
            } else {
                serde_json::to_value(g)?
            }
        }
        Some(VoiceCmd::Flags { text }) => {
            let l = call::gear_voice_flags(core)?;
            if text {
                Value::String(core::voice_flags_text(&l))
            } else {
                serde_json::to_value(l)?
            }
        }
        Some(VoiceCmd::Flag {
            line,
            sample,
            pack,
            reason,
            note,
            clear,
        }) => serde_json::to_value(call::gear_voice_flag(
            core,
            api::FlagParams {
                sample: sample.unwrap_or_default(),
                pack: pack.unwrap_or_default(),
                line,
                reason: reason.unwrap_or_default(),
                note: note.unwrap_or_default(),
                clear,
            },
        )?)?,
        Some(VoiceCmd::Retake {
            sample,
            pack,
            lines,
            dry_run,
            confirm,
            digest,
            grouped,
            tries,
        }) => serde_json::to_value(call::gear_voice_retake(
            core,
            api::RetakeParams {
                sample: sample.unwrap_or_default(),
                pack: pack.unwrap_or_default(),
                lines,
                dry_run,
                confirm,
                digest,
                grouped,
                tries,
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
        Some(VoiceCmd::RenderPacks {
            voices,
            models,
            sets,
            lines,
            dry_run,
            confirm,
            digest,
            batch,
            settings,
        }) => serde_json::to_value(call::gear_voice_render_packs(
            core,
            api::RenderPacksParams {
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
        Some(VoiceCmd::Preview { pack, source }) => serde_json::to_value(
            call::gear_voice_pack_preview(core, api::PackPreviewParams { pack, source })?,
        )?,
        Some(VoiceCmd::Publish {
            approve: true,
            digest,
            ..
        }) => {
            let digest = digest.unwrap_or_default();
            let typed = approve_at_terminal(&digest)?;
            let r = core.voice_publish_approve(&api::PublishApproveParams {
                digest,
                id: Some(typed),
            })?;
            eprint!("{}", core::voice_approve_text(&r));
            serde_json::to_value(r)?
        }
        Some(VoiceCmd::Publish {
            pack,
            id,
            version,
            voice_name,
            license,
            attribution,
            repo,
            text,
            ..
        }) => {
            let r = call::gear_voice_publish(
                core,
                api::PublishParams {
                    pack,
                    id,
                    version,
                    voice_name,
                    license,
                    attribution,
                    repo,
                },
            )?;
            if text {
                Value::String(core::voice_review_text(&r))
            } else {
                serde_json::to_value(r)?
            }
        }
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

/// The person's Approve at a terminal: refuses without one, then reads the pack id they type.
/// An agent's shell has no terminal, so it cannot approve.
fn approve_at_terminal(digest: &str) -> Result<String> {
    use std::io::{IsTerminal, Write};
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        bail!("Approve publishes a pack, so it runs only at a terminal, where you type the pack's id. In the app: Gear > Voices > Publish….");
    }
    eprint!("Approve publishes the pack prepared with digest {digest}.\nType the pack's id to approve: ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let typed = line.trim().to_string();
    if typed.is_empty() {
        bail!("Nothing was published.");
    }
    Ok(typed)
}
