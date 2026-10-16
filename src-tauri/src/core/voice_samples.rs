//! The sample gallery (design 7.4): every sample the Voice studio rendered, kept by voice,
//! model and speed in the gear folder (`voice-samples/`), and the voices of the packs on this
//! Mac. Samples from 0.19 and earlier sat in the cache, which macOS may empty; they move to
//! the gear folder once, the first time the gallery or a sample runs.
//!
//! A take the person does not like gets a flag: a reason and an optional note, kept beside
//! the take (`flags.json` in the sample folder or the pack). A flag on a studio take also marks
//! that take in the line index (`reuse::Entry::rejected`), so a render with the same seed
//! keeps it out at no cost and a render with another seed makes it again. `gear_voice_retake`
//! is that render for the flagged lines only: priced first, paid only with `confirm` and the
//! digest of the plan it priced. The seed comes from the flags, so the confirm prices the same
//! plan, and each re-take of a line that is still flagged picks another.

use super::voice::{copy_dir, slug};
use super::voice_check::{holds_lines, read_manifest, write_manifest};
use super::voice_studio::{kept_key, SampleParams, SampleReport, PROVIDER};
use super::{Core, RenderReport, VoiceRenderParams};
use crate::gear::voice::batch::{BatchCache, Estimate};
use crate::gear::voice::lines::Spelling;
use crate::gear::voice::packs::{self, PackManifest};
use crate::gear::voice::render::RenderSettings;
use crate::gear::voice::reuse::{LineIndex, Scope};
use crate::gear::voice::sets;
use crate::gear::voice::tts::VoiceInfo;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Why a take is flagged.
pub const FLAG_REASONS: &[&str] = &["mispronounced", "clipped", "wrong_tone", "noise", "other"];

/// The sample manifest in each sample folder.
const SAMPLES_FILE: &str = "samples.json";
/// The flags beside the takes, in a sample folder or a pack.
const FLAGS_FILE: &str = "flags.json";
/// The longest note a flag keeps.
const NOTE_MAX: usize = 500;
/// A pack's card shows these many lines when it holds none of the `sample` set.
const PACK_LINES: usize = 12;

/// The person's flag on one take.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct TakeFlag {
    /// The card path.
    pub line: String,
    /// `mispronounced`, `clipped`, `wrong_tone`, `noise` or `other`.
    pub reason: String,
    #[serde(default)]
    pub note: String,
    /// When it was flagged (RFC 3339).
    pub at: String,
    /// Re-takes asked for while it stayed flagged; each picks another seed.
    #[serde(default)]
    pub retakes: u32,
}

/// One take on a card: a sample, or a line of a pack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct SampleTake {
    /// The card path.
    pub line: String,
    pub text: String,
    /// The WAV. In `samples.json` it is relative to the folder; in an answer, absolute.
    pub file: String,
    pub ms: u32,
    /// The seed the take was asked for; absent when not known.
    #[serde(default)]
    pub seed: Option<u64>,
    /// What the speech check heard; absent when it did not run.
    #[serde(default)]
    pub heard: Option<String>,
    /// The heard text matches the line (or the person accepted it).
    #[serde(default)]
    pub heard_ok: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flag: Option<TakeFlag>,
}

/// One voice's card in the gallery.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct SampleVoice {
    /// The sample folder's name, or the pack id.
    pub id: String,
    /// `sample` (the Voice studio's samples) or `pack` (a pack on this Mac).
    pub kind: String,
    /// The provider's voice id; empty when not known (samples from 0.19 and earlier).
    pub voice: String,
    pub voice_name: String,
    pub model: String,
    pub provider: String,
    pub speed: f64,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub accent: String,
    #[serde(default)]
    pub languages: Vec<String>,
    /// When a sample last changed (RFC 3339).
    #[serde(default)]
    pub updated: Option<String>,
    /// The line sets the samples came from.
    #[serde(default)]
    pub sets: Vec<String>,
    pub takes: Vec<SampleTake>,
}

/// `gear_voice_samples`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct SampleGallery {
    /// Sample voices first (newest first), then the packs on this Mac.
    pub voices: Vec<SampleVoice>,
    /// Where the samples are.
    pub dir: String,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// `samples.json`: one voice, model and speed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SampleSet {
    #[serde(default)]
    voice: String,
    #[serde(default)]
    voice_name: String,
    #[serde(default)]
    model: String,
    #[serde(default = "one")]
    speed: f64,
    #[serde(default)]
    description: String,
    #[serde(default)]
    accent: String,
    #[serde(default)]
    languages: Vec<String>,
    #[serde(default)]
    updated: Option<String>,
    #[serde(default)]
    sets: Vec<String>,
    #[serde(default)]
    takes: Vec<SampleTake>,
}

fn one() -> f64 {
    1.0
}

/// `gear_voice_samples_delete`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct SamplesDeleteParams {
    /// Sample folder ids (`gear_voice_samples`). Packs are deleted on the Voices page.
    #[serde(default)]
    pub ids: Vec<String>,
    /// Every sample.
    #[serde(default)]
    pub all: bool,
}

/// `gear_voice_flag`: flag one take, or clear its flag.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct FlagParams {
    /// A sample folder id (`gear_voice_samples`), or `pack` instead.
    #[serde(default)]
    pub sample: String,
    /// A pack on this Mac.
    #[serde(default)]
    pub pack: String,
    /// The take's card path.
    pub line: String,
    /// `mispronounced`, `clipped`, `wrong_tone`, `noise` or `other`; not needed to clear.
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub note: String,
    /// Removes the flag instead.
    #[serde(default)]
    pub clear: bool,
}

/// One flag, with what an agent needs to judge it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct FlagRow {
    /// `sample` or `pack`.
    pub kind: String,
    /// The sample folder id or the pack id.
    pub id: String,
    pub voice: String,
    pub voice_name: String,
    pub model: String,
    pub speed: f64,
    #[serde(default)]
    pub seed: Option<u64>,
    /// The card path.
    pub line: String,
    pub text: String,
    #[serde(default)]
    pub heard: Option<String>,
    /// The WAV.
    pub file: String,
    pub reason: String,
    #[serde(default)]
    pub note: String,
    pub at: String,
    #[serde(default)]
    pub retakes: u32,
    /// `gear_voice_retake` can make this take again (a studio take of a known voice).
    pub retake: bool,
}

/// `gear_voice_flags`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct FlagList {
    pub flags: Vec<FlagRow>,
}

/// `gear_voice_retake`: the flagged takes of one sample voice or one pack, made again with
/// another seed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct RetakeParams {
    #[serde(default)]
    pub sample: String,
    #[serde(default)]
    pub pack: String,
    /// Flagged card paths, or a pack's re-take lines (`retakes`); empty for every flagged line.
    #[serde(default)]
    pub lines: Vec<String>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub confirm: bool,
    /// The `digest` of the unconfirmed call. A paid `confirm` needs it.
    #[serde(default)]
    pub digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RetakeReport {
    pub kind: String,
    pub id: String,
    /// The lines asked for again.
    pub lines: Vec<String>,
    /// The seed the re-take uses.
    pub seed: u64,
    pub estimate: Option<Estimate>,
    pub needs_confirm: bool,
    pub dry_run: bool,
    pub digest: String,
    /// Lines with a new take now: their flags are gone.
    #[serde(default)]
    pub cleared: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// Voice id and name by the slug of the name.
type PackNames = HashMap<String, (String, String)>;

/// A voice name back from its slug, for a folder no pack names: " - " and ", " come back from
/// the runs of hyphens `slug` made of them, and each word starts with a capital.
fn unslug(s: &str) -> String {
    let words = |part: &str| -> String {
        part.split('-')
            .filter(|w| !w.is_empty())
            .map(|w| {
                let mut c = w.chars();
                c.next()
                    .map(|f| f.to_uppercase().chain(c).collect::<String>())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    s.split("---")
        .map(|part| {
            part.split("--")
                .map(words)
                .filter(|w| !w.is_empty())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" - ")
}

/// A sample folder's name: the voice's name, a short hash of its id (two voices with one
/// name stay apart), the model, and the speed when it is not 1.
pub(super) fn sample_dir_name(voice: &str, voice_name: &str, model: &str, speed: f64) -> String {
    let name: String = slug(voice_name).chars().take(40).collect();
    let mut s = format!(
        "{}-{}-{}",
        name.trim_end_matches('-'),
        &crate::gear::voice::sha256_hex(voice.as_bytes())[..6],
        slug(model).chars().take(40).collect::<String>()
    );
    if (speed - 1.0).abs() > 1e-9 {
        s.push_str(&format!("-x{}", format!("{speed}").replace('.', "_")));
    }
    s
}

/// A sample folder id or a pack id: letters, digits, `-` and `_`.
fn check_folder_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 200
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("{id:?} is not a sample id: gear_voice_samples lists them.");
    }
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(p: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(p).ok()?).ok()
}

fn write_json<T: Serialize>(p: &Path, v: &T) -> Result<()> {
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v)?)?;
    std::fs::rename(&tmp, p)?;
    Ok(())
}

fn read_flags(dir: &Path) -> Vec<TakeFlag> {
    read_json(&dir.join(FLAGS_FILE)).unwrap_or_default()
}

/// The flags beside a pack's or a sample's takes.
pub(super) fn flags_of(dir: &Path) -> Vec<TakeFlag> {
    read_flags(dir)
}

fn write_flags(dir: &Path, flags: &[TakeFlag]) -> Result<()> {
    if flags.is_empty() {
        let _ = std::fs::remove_file(dir.join(FLAGS_FILE));
        return Ok(());
    }
    write_json(&dir.join(FLAGS_FILE), &flags)
}

/// A WAV's length from its size (32 kHz, 16-bit mono, a 44-byte header).
fn wav_ms(p: &Path) -> u32 {
    let len = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    (len.saturating_sub(44) * 1000 / (32_000 * 2)) as u32
}

/// The WAVs under a folder, as card paths.
fn wavs(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    super::voice::collect_sounds(dir, dir, &mut out);
    out.sort();
    out
}

/// Moves `from` to `to`; when `to` exists, copies in what it lacks. `from` goes only after
/// every file is in `to`.
fn move_dir(from: &Path, to: &Path) -> Result<()> {
    if !to.exists() {
        if let Some(p) = to.parent() {
            std::fs::create_dir_all(p)?;
        }
        if std::fs::rename(from, to).is_ok() {
            return Ok(());
        }
        std::fs::create_dir_all(to)?;
        copy_dir(from, to)?;
    } else {
        merge_dir(from, to)?;
    }
    std::fs::remove_dir_all(from)?;
    Ok(())
}

/// Copies the files of `from` that `to` lacks.
fn merge_dir(from: &Path, to: &Path) -> Result<()> {
    for e in std::fs::read_dir(from)?.flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            std::fs::create_dir_all(&dst)?;
            merge_dir(&src, &dst)?;
        } else if !dst.exists() {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// The seed of a re-take: from the target, its flags and its unflagged re-take lines (`held`,
/// one key each), so a confirm repeats the plan its estimate priced, and a line re-taken again
/// (its count up) gets another.
fn retake_seed(id: &str, flags: &[&TakeFlag], held: &[String]) -> u64 {
    let mut key = id.to_string();
    let mut f: Vec<String> = flags
        .iter()
        .map(|f| format!("{}|{}|{}", f.line, f.at, f.retakes))
        .chain(held.iter().cloned())
        .collect();
    f.sort();
    for x in f {
        key.push('\n');
        key.push_str(&x);
    }
    let h = crate::gear::voice::sha256_hex(key.as_bytes());
    1 + u64::from_str_radix(&h[..12], 16).unwrap_or(0) % 2_000_000_000
}

/// A flag target: a sample folder or a pack.
enum Target {
    Sample {
        id: String,
        dir: PathBuf,
        set: SampleSet,
    },
    Pack {
        id: String,
        dir: PathBuf,
        m: PackManifest,
    },
}

impl Target {
    fn dir(&self) -> &Path {
        match self {
            Target::Sample { dir, .. } | Target::Pack { dir, .. } => dir,
        }
    }

    fn id(&self) -> &str {
        match self {
            Target::Sample { id, .. } | Target::Pack { id, .. } => id,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Target::Sample { .. } => "sample",
            Target::Pack { .. } => "pack",
        }
    }

    /// The provider's voice id, model and render settings of a studio take of `line`; none
    /// when the takes are not the studio's (no line index) or the voice is not known.
    fn studio(&self, line: &str) -> Option<(String, String, RenderSettings)> {
        match self {
            Target::Sample { set, .. } if !set.voice.is_empty() => {
                let seed = set
                    .takes
                    .iter()
                    .find(|t| t.line == line)
                    .and_then(|t| t.seed)
                    .unwrap_or(0);
                Some((
                    set.voice.clone(),
                    set.model.clone(),
                    RenderSettings {
                        speed: set.speed,
                        seed,
                        ..Default::default()
                    },
                ))
            }
            Target::Pack { m, .. } if holds_lines(m) => {
                Some((m.voice_id.clone(), m.model.clone(), m.settings.clone()))
            }
            _ => None,
        }
    }
}

impl Core {
    /// `<gear>/voice-samples`.
    pub(super) fn samples_root(&self) -> PathBuf {
        self.gear_store().voice_samples_dir()
    }

    /// The folders outside the cache whose sounds the app plays: the samples
    /// (`<gear>/voice-samples`) and the installed packs (`<gear>/voices`). The app grants the
    /// web view's asset scope these two, not the whole gear folder.
    pub fn voice_media_dirs(&self) -> Vec<PathBuf> {
        vec![self.samples_root(), self.voices_dir()]
    }

    /// Moves the samples 0.19 and earlier left in the cache into the gear folder. Never
    /// removes a file before it is in the new place. A failure is a note.
    pub(super) fn move_cached_samples(&self) -> Vec<String> {
        let old = self.cache.join("voice").join("samples");
        let Ok(rd) = std::fs::read_dir(&old) else {
            return Vec::new();
        };
        let root = self.samples_root();
        let mut notes = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            if let Err(err) = move_dir(&p, &root.join(e.file_name())) {
                notes.push(format!(
                    "The samples in {} did not move to {}: {err:#}",
                    p.display(),
                    root.display()
                ));
            }
        }
        let _ = std::fs::remove_dir(&old);
        notes
    }

    /// The folder a sample of this voice, model and speed goes into. A folder from 0.19 and
    /// earlier (named for the voice's name and the model) becomes this voice's at speed 1.
    pub(super) fn sample_dir(
        &self,
        voice: &str,
        voice_name: &str,
        model: &str,
        speed: f64,
    ) -> PathBuf {
        let root = self.samples_root();
        let dir = root.join(sample_dir_name(voice, voice_name, model, speed));
        let legacy = root.join(format!("{}-{}", slug(voice_name), slug(model)));
        if !dir.exists() && (speed - 1.0).abs() < 1e-9 && legacy.is_dir() {
            let known = read_json::<SampleSet>(&legacy.join(SAMPLES_FILE))
                .is_some_and(|s| !s.voice.is_empty());
            if !known {
                let _ = std::fs::rename(&legacy, &dir);
            }
        }
        dir
    }

    /// Records the samples a run wrote, by folder: the voice, its description, accent and
    /// languages, and each take with its seed and what the speech check heard. A take made
    /// again replaces the one before it, and its flag goes.
    pub(super) fn save_samples(
        &self,
        r: &SampleReport,
        voices: &[VoiceInfo],
        sets: &[String],
        settings: &RenderSettings,
    ) -> Vec<String> {
        let mut notes = Vec::new();
        let mut by_dir: HashMap<PathBuf, Vec<&super::SampleItem>> = HashMap::new();
        for i in &r.items {
            let dir = self.sample_dir(&i.voice, &i.voice_name, &i.model, settings.speed);
            by_dir.entry(dir).or_default().push(i);
        }
        let now = chrono::Utc::now().to_rfc3339();
        for (dir, items) in by_dir {
            let mut set: SampleSet = read_json(&dir.join(SAMPLES_FILE)).unwrap_or_default();
            let first = items[0];
            let info = voices.iter().find(|v| v.id == first.voice);
            set.voice = first.voice.clone();
            set.voice_name = first.voice_name.clone();
            set.model = first.model.clone();
            set.speed = settings.speed;
            if let Some(v) = info {
                set.description = v.description.clone();
                set.accent = v.accent.clone();
                set.languages = v.languages.clone();
            }
            set.updated = Some(now.clone());
            for s in sets {
                if !set.sets.contains(s) {
                    set.sets.push(s.clone());
                }
            }
            let mut flags = read_flags(&dir);
            for i in items {
                let file = Path::new(&i.file)
                    .strip_prefix(&dir)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| i.line.clone());
                set.takes.retain(|t| t.line != i.line);
                set.takes.push(SampleTake {
                    line: i.line.clone(),
                    text: i.text.clone(),
                    file,
                    ms: i.ms,
                    seed: Some(settings.seed),
                    heard: i.heard.clone(),
                    heard_ok: i.heard_ok,
                    flag: None,
                });
                flags.retain(|f| f.line != i.line);
            }
            set.takes.sort_by(|a, b| a.line.cmp(&b.line));
            if let Err(e) = std::fs::create_dir_all(&dir)
                .map_err(anyhow::Error::from)
                .and_then(|_| write_json(&dir.join(SAMPLES_FILE), &set))
                .and_then(|_| write_flags(&dir, &flags))
            {
                notes.push(format!(
                    "The sample list in {} was not saved: {e:#}",
                    dir.display()
                ));
            }
        }
        notes
    }

    /// A sample folder as a card. A folder without `samples.json` (0.19 and earlier) is read
    /// from its files: the voice's name and the model from the folder's name, each line's
    /// text from QuadCam's lines.
    fn sample_card(
        &self,
        dir: &Path,
        texts: &HashMap<String, String>,
        names: &PackNames,
    ) -> Option<SampleVoice> {
        let id = dir.file_name()?.to_string_lossy().into_owned();
        let set = match read_json::<SampleSet>(&dir.join(SAMPLES_FILE)) {
            Some(s) => s,
            None => {
                let files = wavs(dir);
                if files.is_empty() {
                    return None;
                }
                let model = crate::gear::voice::rates::RATES
                    .iter()
                    .map(|r| r.model)
                    .find(|m| id.ends_with(&format!("-{}", slug(m))))
                    .unwrap_or("");
                let name = if model.is_empty() {
                    id.clone()
                } else {
                    id[..id.len() - slug(model).len() - 1].to_string()
                };
                let (voice, voice_name) = names
                    .get(&name)
                    .cloned()
                    .unwrap_or_else(|| (String::new(), unslug(&name)));
                SampleSet {
                    voice,
                    voice_name,
                    model: model.to_string(),
                    speed: 1.0,
                    takes: files
                        .into_iter()
                        .map(|f| SampleTake {
                            text: texts.get(&f).cloned().unwrap_or_else(|| f.clone()),
                            ms: wav_ms(&dir.join(&f)),
                            line: f.clone(),
                            file: f,
                            seed: None,
                            heard: None,
                            heard_ok: None,
                            flag: None,
                        })
                        .collect(),
                    ..Default::default()
                }
            }
        };
        let flags = read_flags(dir);
        let takes = set
            .takes
            .into_iter()
            .filter(|t| dir.join(&t.file).is_file())
            .map(|mut t| {
                t.flag = flags.iter().find(|f| f.line == t.line).cloned();
                t.file = dir.join(&t.file).display().to_string();
                t
            })
            .collect::<Vec<_>>();
        if takes.is_empty() {
            return None;
        }
        Some(SampleVoice {
            id,
            kind: "sample".into(),
            voice: set.voice,
            voice_name: set.voice_name,
            model: set.model,
            provider: PROVIDER.into(),
            speed: set.speed,
            description: set.description,
            accent: set.accent,
            languages: set.languages,
            updated: set.updated,
            sets: set.sets,
            takes,
        })
    }

    /// The voices of the packs on this Mac by the slug of their name, as a folder from 0.19
    /// and earlier names them: the voice id and the name.
    fn pack_names(&self) -> PackNames {
        packs::installed(&self.voices_dir())
            .into_iter()
            .map(|i| {
                (
                    slug(&i.manifest.voice),
                    (i.manifest.voice_id, i.manifest.voice),
                )
            })
            .collect()
    }

    /// A pack on this Mac as a card: its lines of the `sample` set (or its first lines), and
    /// every line with a flag.
    fn pack_card(
        &self,
        dir: &Path,
        m: &PackManifest,
        texts: &HashMap<String, String>,
        sample_lines: &[String],
    ) -> Option<SampleVoice> {
        let flags = read_flags(dir);
        let mut lines: Vec<&String> = m
            .files
            .iter()
            .filter(|f| sample_lines.contains(f))
            .collect();
        if lines.is_empty() {
            lines = m.files.iter().take(PACK_LINES).collect();
        }
        for f in &flags {
            if m.files.contains(&f.line) && !lines.contains(&&f.line) {
                lines.push(&f.line);
            }
        }
        let studio = holds_lines(m);
        let takes: Vec<SampleTake> = lines
            .into_iter()
            .filter(|f| dir.join(f).is_file())
            .map(|f| {
                let heard = m.heard.iter().find(|h| h.path == *f);
                SampleTake {
                    line: f.clone(),
                    text: texts.get(f).cloned().unwrap_or_else(|| f.clone()),
                    file: dir.join(f).display().to_string(),
                    ms: wav_ms(&dir.join(f)),
                    seed: studio.then_some(m.settings.seed),
                    heard: heard.map(|h| h.heard.clone()),
                    heard_ok: heard.map(|h| h.ok || h.accepted),
                    flag: flags.iter().find(|x| x.line == *f).cloned(),
                }
            })
            .collect();
        if takes.is_empty() {
            return None;
        }
        Some(SampleVoice {
            id: m.id.clone(),
            kind: "pack".into(),
            voice: m.voice_id.clone(),
            voice_name: m.voice.clone(),
            model: m.model.clone(),
            provider: m.provider.clone(),
            speed: m.settings.speed,
            description: String::new(),
            accent: String::new(),
            languages: Vec::new(),
            updated: None,
            sets: Vec::new(),
            takes,
        })
    }

    /// The sample gallery: every sample voice with its takes (and their flags and what the
    /// speech check heard), then the packs on this Mac. Reads files only.
    pub fn gear_voice_samples(&self) -> Result<SampleGallery> {
        let notes = self.move_cached_samples();
        let root = self.samples_root();
        let texts = self.line_texts()?;
        let names = self.pack_names();
        let mut samples: Vec<SampleVoice> = std::fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| self.sample_card(&e.path(), &texts, &names))
            .collect();
        samples.sort_by(|a, b| {
            b.updated
                .cmp(&a.updated)
                .then(
                    a.voice_name
                        .to_lowercase()
                        .cmp(&b.voice_name.to_lowercase()),
                )
                .then(a.id.cmp(&b.id))
        });
        let sample_lines: Vec<String> = sets::lines_of("sample")?
            .into_iter()
            .map(|l| l.line.path)
            .collect();
        for i in packs::installed(&self.voices_dir()) {
            let dir = PathBuf::from(&i.dir);
            if let Some(mut card) = self.pack_card(&dir, &i.manifest, &texts, &sample_lines) {
                // What a sample of the same voice knows about it.
                if let Some(s) = samples
                    .iter()
                    .find(|s| !s.voice.is_empty() && s.voice == card.voice)
                {
                    card.description = s.description.clone();
                    card.accent = s.accent.clone();
                    card.languages = s.languages.clone();
                }
                samples.push(card);
            }
        }
        Ok(SampleGallery {
            voices: samples,
            dir: root.display().to_string(),
            notes,
        })
    }

    /// Deletes sample folders (or every sample). The takes they were cut from stay in the
    /// cache, so sampling again costs nothing.
    pub fn gear_voice_samples_delete(&self, p: &SamplesDeleteParams) -> Result<SampleGallery> {
        let _ = self.move_cached_samples();
        let root = self.samples_root();
        if p.all {
            if root.is_dir() {
                std::fs::remove_dir_all(&root)
                    .with_context(|| format!("deleting {}", root.display()))?;
            }
        } else {
            if p.ids.is_empty() {
                bail!("Name the samples to delete (ids), or all.");
            }
            for id in &p.ids {
                check_folder_id(id)?;
                let dir = root.join(id);
                if !dir.is_dir() {
                    bail!("No sample {id:?}: gear_voice_samples lists them. A pack is deleted on the Voices page.");
                }
            }
            for id in &p.ids {
                std::fs::remove_dir_all(root.join(id))?;
            }
        }
        self.gear_voice_samples()
    }

    fn flag_target(&self, sample: &str, pack: &str) -> Result<Target> {
        match (sample.trim(), pack.trim()) {
            (s, "") if !s.is_empty() => {
                check_folder_id(s)?;
                let _ = self.move_cached_samples();
                let dir = self.samples_root().join(s);
                if !dir.is_dir() {
                    bail!("No sample {s:?}: gear_voice_samples lists them.");
                }
                let texts = self.line_texts()?;
                let card = self
                    .sample_card(&dir, &texts, &self.pack_names())
                    .with_context(|| format!("The sample {s:?} holds no take."))?;
                let mut set: SampleSet = read_json(&dir.join(SAMPLES_FILE)).unwrap_or_default();
                if set.takes.is_empty() {
                    // A folder from 0.19 and earlier: its takes as the card reads them.
                    set.voice_name = card.voice_name;
                    set.model = card.model;
                    set.speed = 1.0;
                    set.takes = card
                        .takes
                        .into_iter()
                        .map(|mut t| {
                            t.file = t.line.clone();
                            t.flag = None;
                            t
                        })
                        .collect();
                }
                Ok(Target::Sample {
                    id: s.to_string(),
                    dir,
                    set,
                })
            }
            ("", k) if !k.is_empty() => {
                packs::check_id(k)?;
                let dir = self.voices_dir().join(k);
                let m = read_manifest(&dir)
                    .map_err(|_| anyhow::anyhow!("Pack {k:?} is not on this Mac."))?;
                Ok(Target::Pack {
                    id: k.to_string(),
                    dir,
                    m,
                })
            }
            _ => bail!("Name one sample (gear_voice_samples) or one pack."),
        }
    }

    /// A seed key per unflagged re-take line of a studio pack: its path, its reason, and how
    /// many takes of it the line index holds. A re-take adds a take, so the next one gets
    /// another seed; the estimate and its confirm see the same count.
    fn retake_keys(&self, m: &PackManifest, held: &[&packs::Retake]) -> Result<Vec<String>> {
        if held.is_empty() {
            return Ok(Vec::new());
        }
        let cache = BatchCache::new(&self.cache);
        let index = LineIndex::load(&Scope {
            cache: &cache,
            provider: PROVIDER,
            voice: &m.voice_id,
            model: &m.model,
            settings: &m.settings,
        });
        let spelling = Spelling::builtin()?;
        Ok(held
            .iter()
            .map(|r| {
                let spoken = spelling.apply(&r.text);
                let takes = index
                    .entries()
                    .iter()
                    .filter(|e| {
                        e.items
                            .iter()
                            .any(|i| i.spoken.eq_ignore_ascii_case(&spoken))
                    })
                    .count();
                format!("{}|retake|{}|{takes}", r.path, r.reason)
            })
            .collect())
    }

    /// The text of a line of a target, when the target holds it.
    fn target_line(&self, t: &Target, line: &str) -> Result<String> {
        match t {
            Target::Sample { set, id, .. } => set
                .takes
                .iter()
                .find(|x| x.line == line)
                .map(|x| x.text.clone())
                .with_context(|| format!("The sample {id} has no take of {line:?}.")),
            Target::Pack { m, id, .. } => {
                if !m.files.iter().any(|f| f == line) {
                    bail!("The pack {id} has no line {line:?}.");
                }
                Ok(self
                    .line_texts()?
                    .get(line)
                    .cloned()
                    .unwrap_or_else(|| line.to_string()))
            }
        }
    }

    /// Marks (or unmarks) the studio take of a line as rejected in the line index. A take the
    /// cache lost is a note: a re-take then renders the line new.
    fn reject_take(&self, t: &mut Target, line: &str, text: &str, on: bool) -> Option<String> {
        let (voice, model, settings) = t.studio(line)?;
        let cache = BatchCache::new(&self.cache);
        let scope = Scope {
            cache: &cache,
            provider: PROVIDER,
            voice: &voice,
            model: &model,
            settings: &settings,
        };
        let spoken = Spelling::builtin().ok()?.apply(text);
        let index = LineIndex::load(&scope);
        let mut changed = Vec::new();
        if on {
            let Some(n) = index.take_of(&scope, &spoken) else {
                return Some(format!(
                    "No take of {line:?} is in the cache: a re-take renders it new."
                ));
            };
            let mut e = index.entries()[n].clone();
            if !e.is_rejected(&spoken) {
                e.rejected.push(spoken.clone());
            }
            // The take's batch is no longer whole in the pack: a re-take cuts it again.
            if let Target::Pack { m, dir, .. } = t {
                let k = kept_key(&voice, &model, &settings, &e.batch());
                if m.kept_batches.contains(&k) {
                    m.kept_batches.retain(|x| *x != k);
                    if let Err(err) = write_manifest(dir, m) {
                        return Some(format!("The pack was not updated: {err:#}"));
                    }
                }
            }
            changed.push(e);
        } else {
            for e in index.entries().iter().filter(|e| e.is_rejected(&spoken)) {
                let mut e = e.clone();
                e.rejected.retain(|r| !r.eq_ignore_ascii_case(&spoken));
                changed.push(e);
            }
        }
        if changed.is_empty() {
            return None;
        }
        LineIndex::record(&scope, changed).err().map(|e| {
            format!("The line index was not saved ({e:#}): a re-take may not know the flag.")
        })
    }

    /// Flags one take with a reason and a note, or clears its flag. Changes only QuadCam's own
    /// files. Answers the take's card.
    pub fn gear_voice_flag(&self, p: &FlagParams) -> Result<SampleVoice> {
        let mut t = self.flag_target(&p.sample, &p.pack)?;
        let line = p.line.trim();
        if line.is_empty() {
            bail!("Name the take's line: its card path, such as SOUNDS/en/armed.wav.");
        }
        let text = self.target_line(&t, line)?;
        let mut flags = read_flags(t.dir());
        if p.clear {
            flags.retain(|f| f.line != line);
        } else {
            let reason = p.reason.trim();
            if !FLAG_REASONS.contains(&reason) {
                bail!(
                    "{reason:?} is not a reason: use {}.",
                    FLAG_REASONS.join(", ")
                );
            }
            if p.note.chars().count() > NOTE_MAX {
                bail!("A note holds {NOTE_MAX} characters at most.");
            }
            flags.retain(|f| f.line != line);
            flags.push(TakeFlag {
                line: line.to_string(),
                reason: reason.to_string(),
                note: p.note.trim().to_string(),
                at: chrono::Utc::now().to_rfc3339(),
                retakes: 0,
            });
            flags.sort_by(|a, b| a.line.cmp(&b.line));
        }
        write_flags(t.dir(), &flags)?;
        let _ = self.reject_take(&mut t, line, &text, !p.clear);
        self.card_of(&t)
    }

    /// A target's card, read again.
    fn card_of(&self, t: &Target) -> Result<SampleVoice> {
        let texts = self.line_texts()?;
        let card = match t {
            Target::Sample { dir, .. } => self.sample_card(dir, &texts, &self.pack_names()),
            Target::Pack { dir, m, .. } => {
                let sample_lines: Vec<String> = sets::lines_of("sample")?
                    .into_iter()
                    .map(|l| l.line.path)
                    .collect();
                self.pack_card(dir, m, &texts, &sample_lines)
            }
        };
        card.with_context(|| format!("{} holds no take.", t.id()))
    }

    /// Every flag, sample and pack, with the voice, model, speed, seed, text, what the speech
    /// check heard and the file.
    pub fn gear_voice_flags(&self) -> Result<FlagList> {
        let g = self.gear_voice_samples()?;
        let mut flags = Vec::new();
        let packs: HashMap<String, PackManifest> = packs::installed(&self.voices_dir())
            .into_iter()
            .map(|i| (i.manifest.id.clone(), i.manifest))
            .collect();
        for v in &g.voices {
            let retake = match v.kind.as_str() {
                "pack" => packs.get(&v.id).is_some_and(holds_lines),
                _ => !v.voice.is_empty(),
            };
            for t in &v.takes {
                let Some(f) = &t.flag else { continue };
                flags.push(FlagRow {
                    kind: v.kind.clone(),
                    id: v.id.clone(),
                    voice: v.voice.clone(),
                    voice_name: v.voice_name.clone(),
                    model: v.model.clone(),
                    speed: v.speed,
                    seed: t.seed,
                    line: t.line.clone(),
                    text: t.text.clone(),
                    heard: t.heard.clone(),
                    file: t.file.clone(),
                    reason: f.reason.clone(),
                    note: f.note.clone(),
                    at: f.at.clone(),
                    retakes: f.retakes,
                    retake,
                });
            }
        }
        flags.sort_by(|a, b| b.at.cmp(&a.at).then(a.line.cmp(&b.line)));
        Ok(FlagList { flags })
    }

    /// Makes the flagged takes of one sample voice or one pack again with another seed. A pack
    /// also takes its re-take lines by name (a line the speech check heard wrong, or whose cut
    /// failed its checks) without a flag. A paid call: without `confirm` (or with `dry_run`) it
    /// prices the lines and answers the digest.
    pub fn gear_voice_retake(&self, p: &RetakeParams) -> Result<RetakeReport> {
        let t = self.flag_target(&p.sample, &p.pack)?;
        let flags = read_flags(t.dir());
        let picked: Vec<&TakeFlag> = flags
            .iter()
            .filter(|f| p.lines.is_empty() || p.lines.contains(&f.line))
            .collect();
        // The named re-take lines of a pack that carry no flag.
        let held: Vec<&packs::Retake> = match &t {
            Target::Pack { m, .. } => m
                .retakes
                .iter()
                .filter(|r| p.lines.contains(&r.path) && !flags.iter().any(|f| f.line == r.path))
                .collect(),
            Target::Sample { .. } => Vec::new(),
        };
        for l in &p.lines {
            if !flags.iter().any(|f| f.line == *l) && !held.iter().any(|r| r.path == *l) {
                bail!(
                    "{l:?} has no flag in {} and needs no re-take: flag it first.",
                    t.id()
                );
            }
        }
        if picked.is_empty() && held.is_empty() {
            bail!("{} has no flagged take.", t.id());
        }
        let lines: Vec<String> = picked
            .iter()
            .map(|f| f.line.clone())
            .chain(held.iter().map(|r| r.path.clone()))
            .collect();
        let held_keys = match &t {
            Target::Pack { m, .. } => self.retake_keys(m, &held)?,
            Target::Sample { .. } => Vec::new(),
        };
        let seed = retake_seed(t.id(), &picked, &held_keys);
        let mut report = RetakeReport {
            kind: t.kind().into(),
            id: t.id().into(),
            lines: lines.clone(),
            seed,
            estimate: None,
            needs_confirm: false,
            dry_run: p.dry_run,
            digest: String::new(),
            cleared: Vec::new(),
            notes: Vec::new(),
        };
        // The lines with a new take after the run.
        let made: Vec<String> = match &t {
            Target::Sample { set, id, .. } => {
                if set.voice.is_empty() {
                    bail!("The sample {id} was made before QuadCam kept its voice's id: sample the voice again in the Voice studio.");
                }
                let r = self.gear_voice_sample(&SampleParams {
                    voices: vec![set.voice.clone()],
                    models: vec![set.model.clone()],
                    sets: if set.sets.is_empty() {
                        vec!["sample".into()]
                    } else {
                        set.sets.clone()
                    },
                    lines: lines.clone(),
                    dry_run: p.dry_run,
                    confirm: p.confirm,
                    settings: Some(RenderSettings {
                        speed: set.speed,
                        seed,
                        ..Default::default()
                    }),
                    batch: None,
                    digest: p.digest.clone(),
                })?;
                report.estimate = Some(r.estimate.clone());
                report.needs_confirm = r.needs_confirm;
                report.digest = r.digest.clone();
                report.notes.extend(r.warnings.iter().cloned());
                r.items.iter().map(|i| i.line.clone()).collect()
            }
            Target::Pack { m, id, .. } => {
                if !holds_lines(m) {
                    bail!("Only a pack the Voice studio rendered can make a re-take; a flag on {id} is a note.");
                }
                let r: RenderReport = self.gear_voice_render(&VoiceRenderParams {
                    voice: m.voice_id.clone(),
                    lines: lines.clone(),
                    dry_run: p.dry_run,
                    confirm: p.confirm,
                    settings: Some(RenderSettings {
                        seed,
                        ..m.settings.clone()
                    }),
                    sets: sets::ids().into_iter().map(String::from).collect(),
                    model: m.model.clone(),
                    batch: None,
                    digest: p.digest.clone(),
                })?;
                if r.pack != *id {
                    bail!(
                        "The pack {id} is not the studio pack of its voice and model ({}).",
                        r.pack
                    );
                }
                report.estimate = r.estimate.clone();
                report.needs_confirm = r.needs_confirm;
                report.digest = r.digest.clone();
                report.notes.extend(r.notes.iter().cloned());
                if r.needs_confirm || r.dry_run {
                    Vec::new()
                } else {
                    let now = read_manifest(t.dir())?;
                    lines
                        .iter()
                        .filter(|l| {
                            now.files.contains(l) && !now.retakes.iter().any(|x| x.path == **l)
                        })
                        .cloned()
                        .collect()
                }
            }
        };
        if p.dry_run || report.needs_confirm {
            return Ok(report);
        }
        // A flag goes with its take; a line still without a new take re-takes with another
        // seed next time.
        let mut flags = read_flags(t.dir());
        flags.retain(|f| !made.contains(&f.line));
        for f in flags.iter_mut().filter(|f| lines.contains(&f.line)) {
            f.retakes += 1;
        }
        write_flags(t.dir(), &flags)?;
        let left = lines.len() - made.len();
        if left > 0 {
            report.notes.push(format!(
                "{left} lines have no new take yet: they stay flagged or re-takes, and the next re-take uses another seed."
            ));
        }
        report.cleared = made;
        Ok(report)
    }
}

/// A gallery as text.
pub fn samples_text(g: &SampleGallery) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    if g.voices.is_empty() {
        let _ = write!(
            s,
            "No samples. The Voice studio's Sample makes them ({}).",
            g.dir
        );
    }
    for v in &g.voices {
        let what = if v.kind == "pack" { "pack" } else { "sample" };
        let _ = write!(
            s,
            "{} ({what} {}): {}, speed {}{}",
            v.voice_name,
            v.id,
            if v.model.is_empty() {
                &v.provider
            } else {
                &v.model
            },
            v.speed,
            if v.accent.is_empty() {
                String::new()
            } else {
                format!(", {}", v.accent)
            }
        );
        for t in &v.takes {
            let _ = write!(s, "\n  {}: {} ms {}", t.text, t.ms, t.file);
            if t.heard_ok == Some(false) {
                let _ = write!(s, " (heard {:?})", t.heard.clone().unwrap_or_default());
            }
            if let Some(f) = &t.flag {
                let _ = write!(
                    s,
                    " FLAGGED {}{}",
                    f.reason,
                    if f.note.is_empty() {
                        String::new()
                    } else {
                        format!(": {}", f.note)
                    }
                );
            }
        }
        s.push('\n');
    }
    for n in &g.notes {
        let _ = writeln!(s, "{n}");
    }
    s.trim_end().to_string()
}

/// A flag list as text.
pub fn flags_text(l: &FlagList) -> String {
    use std::fmt::Write;
    if l.flags.is_empty() {
        return "No flagged takes.".into();
    }
    let mut s = format!("{} flagged takes:", l.flags.len());
    for f in &l.flags {
        let _ = write!(
            s,
            "\n  {} {} ({}, {}, speed {}, seed {}): {} {:?}: {}{}; heard {:?}; {}{}",
            f.kind,
            f.id,
            f.voice_name,
            f.model,
            f.speed,
            f.seed.map(|x| x.to_string()).unwrap_or_else(|| "?".into()),
            f.line,
            f.text,
            f.reason,
            if f.note.is_empty() {
                String::new()
            } else {
                format!(" ({})", f.note)
            },
            f.heard.clone().unwrap_or_default(),
            f.file,
            if f.retake {
                ""
            } else {
                " (no re-take: not a studio take)"
            },
        );
    }
    s
}

/// A re-take report as text.
pub fn retake_text(r: &RetakeReport) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    if r.needs_confirm {
        let _ = write!(s, "Not rendered: this sends {} lines of {} to ElevenLabs, which bills it. Ask the person, then run again with confirm and digest {}.", r.lines.len(), r.id, r.digest);
    } else if r.dry_run {
        let _ = write!(
            s,
            "Dry run: nothing was rendered. A paid run confirms with digest {}.",
            r.digest
        );
    } else {
        let _ = write!(
            s,
            "Re-took {} of {} lines of {} with seed {}.",
            r.cleared.len(),
            r.lines.len(),
            r.id,
            r.seed
        );
    }
    if let Some(e) = &r.estimate {
        let _ = write!(s, " {}", super::voice_studio::estimate_text(e));
    }
    for n in &r.notes {
        let _ = write!(s, "\n{n}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_folder_name_reads_back_as_a_voice_name() {
        assert_eq!(unslug("adam---dominant--firm"), "Adam - Dominant, Firm");
        assert_eq!(
            unslug("johnny-dynamite---vintage-radio-dj"),
            "Johnny Dynamite - Vintage Radio Dj"
        );
        assert_eq!(unslug("callum"), "Callum");
        assert_eq!(
            unslug(&slug("Xavier - Dominating, Metalic Announcer")),
            "Xavier - Dominating, Metalic Announcer"
        );
    }

    #[test]
    fn a_sample_folder_names_the_voice_model_and_speed() {
        let a = sample_dir_name("v-1", "Callum Smith", "eleven_v3", 1.0);
        let b = sample_dir_name("v-2", "Callum Smith", "eleven_v3", 1.0);
        assert!(
            a.starts_with("callum-smith-") && a.ends_with("-eleven-v3"),
            "{a}"
        );
        assert_ne!(a, b);
        let c = sample_dir_name("v-1", "Callum Smith", "eleven_v3", 1.1);
        assert!(c.ends_with("-eleven-v3-x1_1"), "{c}");
        check_folder_id(&c).unwrap();
        assert!(check_folder_id("../x").is_err());
    }

    #[test]
    fn a_retake_seed_follows_the_flags() {
        let f = TakeFlag {
            line: "SOUNDS/en/armed.wav".into(),
            reason: "clipped".into(),
            note: String::new(),
            at: "2026-10-11T00:00:00Z".into(),
            retakes: 0,
        };
        let s1 = retake_seed("x", &[&f], &[]);
        assert_eq!(s1, retake_seed("x", &[&f], &[]));
        assert!(s1 > 0);
        let again = TakeFlag {
            retakes: 1,
            ..f.clone()
        };
        assert_ne!(s1, retake_seed("x", &[&again], &[]));
    }
}
