//! The voice studio on the core (design 7.4): the ElevenLabs key in the Keychain, the
//! account's voices, models and credits, an estimate before any paid call, A/B samples per
//! voice and model, and a batched render of line sets into a local pack. Every paid call
//! checks the estimate against the credits first and waits for `confirm` with the digest of
//! the plan it priced. A cut that fails a check stays out of the pack as a re-take.
//! The studio always speaks to ElevenLabs; the `tts_provider` setting stays what it was.

use super::voice::{collect_sounds, copy_dir, slug};
use super::{Core, RenderReport, VoiceRenderParams};
use crate::api::{Event, VoiceProgress};
use crate::gear::voice::batch::{self, Batch, BatchCache, BatchSettings, Estimate, Item, Pricing};
use crate::gear::voice::keychain::ELEVENLABS;
use crate::gear::voice::lines::{self, Line, Spelling};
use crate::gear::voice::packs::{self, BuildOpts, HeardLine, PackManifest, Retake, Structure};
use crate::gear::voice::rates;
use crate::gear::voice::render::{self, Plan, RenderSettings};
use crate::gear::voice::reuse::{self, ImportReport, LineIndex, Reuse, Scope, TakeCount};
use crate::gear::voice::sets::{self, SetInfo, SetLine};
use crate::gear::voice::tts::{
    Credits, LibraryPage, LibraryQuery, LibraryVoice, ModelInfo, ProviderConfig, Tts, VoiceInfo,
};
use crate::modules::fetch::Fetch;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// `gear_voice_key`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct KeyParams {
    /// `status` (default), `set` or `delete`.
    #[serde(default)]
    pub action: String,
    /// For `set`: the key. It goes to the Keychain and nowhere else.
    #[serde(default)]
    pub key: Option<String>,
}

/// Whether a key is stored, never the key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct KeyStatus {
    pub set: bool,
    /// What tells keys apart without showing one: `sk_9…8d4d (51 chars)` (`secret::hint`).
    pub hint: String,
    /// The hint's first and last characters and the key's length; empty and 0 with no key.
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub suffix: String,
    #[serde(default)]
    pub length: u32,
    /// `keychain`, or `environment` when QUADCAM_ELEVENLABS_KEY (or the older QUADCAM_TTS_KEY)
    /// holds it for this run.
    pub source: String,
    /// Why the Keychain could not be read, when it could not.
    #[serde(default)]
    pub problem: Option<String>,
}

/// `gear_voice_catalog`: what the account offers. With none of the three set, all of them.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct CatalogParams {
    #[serde(default)]
    pub voices: bool,
    #[serde(default)]
    pub models: bool,
    #[serde(default)]
    pub credits: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct Catalog {
    pub voices: Vec<VoiceInfo>,
    pub models: Vec<ModelInfo>,
    #[serde(default)]
    pub credits: Option<Credits>,
}

/// `gear_voice_sets`. With a voice id and a model, each set says how many of its lines that
/// voice and model already have a take of.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct SetsParams {
    /// A voice id from the account (`gear_voice_catalog`); no name, as this makes no call.
    #[serde(default)]
    pub voice: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub settings: Option<RenderSettings>,
    #[serde(default)]
    pub batch: Option<BatchSettings>,
}

/// `gear_voice_sets`: the line sets and the key, with no network call.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct StudioView {
    pub key: KeyStatus,
    pub sets: Vec<SetInfo>,
    /// The carrier and batch size used when none is given.
    pub batch: BatchSettings,
    /// The aircraft kinds the person flies, from the aircraft profiles (`quad` when no
    /// profile names one).
    #[serde(default)]
    pub aircraft: Vec<String>,
    /// The sets the studio ticks: every set the first time; after that the sets it saved
    /// (`gear_voice_save_sets`), with `quadcam` always ticked.
    #[serde(default)]
    pub picks: Vec<String>,
    /// The lines each voice, model and speed already have a take of, most first.
    #[serde(default)]
    pub rendered: Vec<TakeCount>,
}

/// `gear_voice_save_sets`: the line sets ticked in the studio, kept for the next time.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct SaveSetsParams {
    pub sets: Vec<String>,
}

/// `<gear>/voice-studio.json`: the studio's saved choices.
const CHOICES_FILE: &str = "voice-studio.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StudioChoices {
    #[serde(default)]
    sets: Vec<String>,
}

/// `gear_voice_estimate`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct EstimateParams {
    /// Line set ids (`gear_voice_sets`); they combine.
    pub sets: Vec<String>,
    /// A voice name or id from the account.
    pub voice: String,
    pub model: String,
    /// Only these card paths of the sets; empty for every line.
    #[serde(default)]
    pub lines: Vec<String>,
    #[serde(default)]
    pub settings: Option<RenderSettings>,
    #[serde(default)]
    pub batch: Option<BatchSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct StudioEstimate {
    pub voice: String,
    pub voice_name: String,
    pub model: String,
    /// Lines in the sets after combining.
    pub lines: u32,
    /// Of those, lines with a take that passed its checks: nothing is sent for them.
    #[serde(default)]
    pub rendered: u32,
    /// Lines whose take with this seed failed its checks: nothing is sent; another seed
    /// renders them again.
    #[serde(default)]
    pub retakes: u32,
    /// Lines whose text goes to ElevenLabs.
    #[serde(default)]
    pub to_render: u32,
    pub estimate: Estimate,
    /// The voice's own credit multiplier (a Voice Library voice), already in `estimate`.
    #[serde(default)]
    pub rate: Option<f64>,
    /// What this voice's samples of the risky lines (the `sample` set) showed.
    #[serde(default)]
    pub validation: Validation,
}

/// What a voice's samples of the risky lines, the `sample` set, showed before a pack is
/// rendered: the lines heard wrong, and the ways of saying words a grouped re-take found,
/// which a render of the voice, model and speed carries to the lines with the word.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct Validation {
    /// Lines in the `sample` set.
    pub lines: u32,
    /// Of those, lines this voice, model and speed has a sample of.
    pub sampled: u32,
    /// Sampled lines the speech check heard right (or the person accepted).
    pub heard_right: u32,
    /// Sampled lines the speech check heard wrong, with what it heard.
    #[serde(default)]
    pub heard_wrong: Vec<HeardLine>,
    #[serde(default)]
    pub structures: Vec<Structure>,
}

/// `gear_voice_sample`: short A/B renders of a few lines in every voice and model.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct SampleParams {
    pub voices: Vec<String>,
    pub models: Vec<String>,
    /// Line sets to sample; empty for `sample`.
    #[serde(default)]
    pub sets: Vec<String>,
    #[serde(default)]
    pub lines: Vec<String>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub settings: Option<RenderSettings>,
    #[serde(default)]
    pub batch: Option<BatchSettings>,
    /// The `digest` of the unconfirmed call. A paid `confirm` needs it, and is refused when
    /// the plan changed since.
    #[serde(default)]
    pub digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SampleItem {
    pub voice: String,
    pub voice_name: String,
    pub model: String,
    /// The line's card path.
    pub line: String,
    pub text: String,
    /// The WAV, in the cache.
    pub file: String,
    pub ms: u32,
    /// What the speech check heard; absent when it did not run.
    #[serde(default)]
    pub heard: Option<String>,
    /// The heard text matches the line.
    #[serde(default)]
    pub heard_ok: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SampleReport {
    pub items: Vec<SampleItem>,
    /// The total over every voice and model.
    pub estimate: Estimate,
    pub combos: u32,
    pub needs_confirm: bool,
    pub dry_run: bool,
    pub warnings: Vec<String>,
    /// The digest of this plan, which `confirm` repeats.
    #[serde(default)]
    pub digest: String,
    /// A stop ended the run early: `items` holds the pairs finished before it.
    #[serde(default)]
    pub stopped: bool,
}

/// `gear_voice_import_takes`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct ImportTakesParams {
    /// A folder of takes an earlier line-by-line render left: a `.json` (text, voice, model,
    /// speed, seed) beside a `.pcm` of the same name. QuadCam only reads it.
    pub folder: String,
    #[serde(default)]
    pub dry_run: bool,
}

/// `gear_voice_render_packs`: one local pack per voice and model, in one confirmed run.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct RenderPacksParams {
    /// Voice names or ids from the account.
    pub voices: Vec<String>,
    pub models: Vec<String>,
    /// Line set ids (`gear_voice_sets`); they combine.
    pub sets: Vec<String>,
    /// Only these card paths of the sets; empty for every line.
    #[serde(default)]
    pub lines: Vec<String>,
    /// Price only; render nothing.
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub settings: Option<RenderSettings>,
    #[serde(default)]
    pub batch: Option<BatchSettings>,
    /// The `digest` of the unconfirmed call. A paid `confirm` needs it, and is refused when
    /// the plan changed since.
    #[serde(default)]
    pub digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RenderPacksReport {
    /// Each voice and model: its lines, how many are rendered, and what the rest cost.
    pub pairs: Vec<StudioEstimate>,
    /// The total over every pair.
    pub estimate: Estimate,
    /// Lines in the sets after combining.
    pub lines: u32,
    pub needs_confirm: bool,
    pub dry_run: bool,
    /// The digest of the whole plan, which `confirm` repeats.
    pub digest: String,
    /// The packs made, in order. A failure or a stop keeps those made before it.
    #[serde(default)]
    pub packs: Vec<RenderReport>,
    /// A stop ended the run early.
    #[serde(default)]
    pub stopped: bool,
    /// What ended the run early, when something failed.
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// `gear_voice_library`: a page of ElevenLabs' Voice Library, or one voice by id.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct LibraryParams {
    #[serde(default)]
    pub query: LibraryQuery,
    /// One voice by id instead of a search: the account's own, or a library voice.
    #[serde(default)]
    pub id: String,
}

/// `gear_voice_library_preview`: a voice's preview sample, fetched into the cache to play.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct LibraryPreviewParams {
    /// The voice's `preview_url` (HTTPS).
    pub url: String,
}

/// `gear_voice_add`: a Voice Library voice into the account's My Voices.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct VoiceAddParams {
    /// The library voice's id.
    pub voice: String,
    /// The owner's public id (`public_owner_id`); looked up by the voice id when empty.
    #[serde(default)]
    pub owner: String,
    /// The name it gets in the account; the library's name when empty.
    #[serde(default)]
    pub name: String,
    /// Adds it. Without it, the answer only says what would be added.
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct VoiceAddReport {
    /// The voice as the library shows it.
    pub voice: LibraryVoice,
    /// The name in the account.
    pub name: String,
    /// The voice's id in the account, once added.
    #[serde(default)]
    pub added_id: Option<String>,
    /// Nothing was added: run again with `confirm`.
    pub needs_confirm: bool,
    /// The account had the voice already; nothing was sent.
    #[serde(default)]
    pub already: bool,
}

/// The gear job a paid studio run registers: `gear_stop` with this handle stops it after the
/// current batch.
pub const STUDIO_JOB: &str = "voice-studio";

/// The account's voices, models and credits, read once per call.
struct Account {
    voices: Vec<VoiceInfo>,
    models: Vec<ModelInfo>,
    credits: Credits,
}

impl Account {
    fn of(tts: &dyn Tts) -> Result<Self> {
        Ok(Account {
            voices: tts.voices()?,
            models: tts.models()?,
            credits: tts.credits()?,
        })
    }
}

/// One voice and model of a render, priced and not yet paid for.
struct Prepared {
    /// The voice's own credit multiplier, when it has one.
    rate: Option<f64>,
    voice: String,
    voice_name: String,
    model: String,
    id: String,
    old_id: Option<String>,
    old: Option<PackManifest>,
    plan: StudioPlan,
    est: Estimate,
    /// (rendered, re-takes, to render), as `StudioPlan::counts`.
    counts: (u32, u32, u32),
    report: RenderReport,
    /// The lines as the pack speaks them (`structured`), and the batch settings with the
    /// carriers its structures name.
    lines: Vec<(SetLine, String)>,
    bs: BatchSettings,
    /// The structures the pack speaks its lines with (`adopt`).
    structures: Vec<Structure>,
    validation: Validation,
}

impl Prepared {
    fn studio_estimate(&self, lines: usize) -> StudioEstimate {
        StudioEstimate {
            voice: self.voice.clone(),
            voice_name: self.voice_name.clone(),
            model: self.model.clone(),
            lines: lines as u32,
            rendered: self.counts.0,
            retakes: self.counts.1,
            to_render: self.counts.2,
            estimate: self.est.clone(),
            rate: self.rate,
            validation: self.validation.clone(),
        }
    }
}

/// Adds one pair's estimate to a total.
fn add_estimate(total: &mut Estimate, e: &Estimate) {
    total.batches += e.batches;
    total.cached_batches += e.cached_batches;
    total.lines += e.lines;
    total.chars += e.chars;
    total.credits += e.credits;
    total.usd += e.usd;
    if total.credits_basis.is_empty() {
        total.credits_basis = e.credits_basis.clone();
    } else if total.credits_basis != e.credits_basis {
        total.credits_basis = "estimated".into();
    }
}

/// The one USD rate and promo every pair shares; none (0) when they differ.
fn shared_rate(total: &mut Estimate, each: &[&Estimate]) {
    let Some(first) = each.first() else {
        return;
    };
    if each.iter().all(|e| {
        (e.usd_per_1k - first.usd_per_1k).abs() < 1e-9 && e.promo_until == first.promo_until
    }) {
        total.usd_per_1k = first.usd_per_1k;
        total.promo_until = first.promo_until.clone();
        total.cost_per_char = first.cost_per_char;
    }
}

/// A paid run's progress, sent as `VoiceProgress` events, and its stop: the gear job
/// `STUDIO_JOB` while it lives, and the studio lock, which one paid run on this Mac holds at a
/// time (the app and `quadcam-cli` each have their own jobs, but share the batch cache).
/// Dropping it ends all three.
struct Progress<'a> {
    core: &'a Core,
    stop: Arc<AtomicBool>,
    _job: super::backup::JobGuard<'a>,
    _lock: std::fs::File,
    ev: VoiceProgress,
}

/// Takes the machine-wide studio lock, `<cache>/voice/studio.lock`, and writes who holds it
/// into the file. Refused, naming the holder, while another process holds it.
fn studio_lock(core: &Core) -> Result<std::fs::File> {
    use std::io::{Seek, Write};
    let dir = core.cache.join("voice");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("studio.lock");
    let mut f = std::fs::File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .context("opening the voice studio lock")?;
    match f.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            let who = std::fs::read_to_string(&path).unwrap_or_default();
            let who = who.trim();
            bail!(
                "Refused: {} is rendering voices on this Mac now. Wait for it to finish, or stop it there.",
                if who.is_empty() { "another QuadCam" } else { who }
            );
        }
        Err(std::fs::TryLockError::Error(e)) => {
            return Err(e).context("locking the voice studio lock")
        }
    }
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "QuadCam".into());
    f.set_len(0)?;
    f.rewind()?;
    write!(f, "{exe} (process {})", std::process::id())?;
    Ok(f)
}

impl<'a> Progress<'a> {
    fn start(core: &'a Core, kind: &str, total: usize) -> Result<Self> {
        let (job, stop) = core.job_start(
            STUDIO_JOB,
            None,
            if kind == "sample" {
                "Sampling voices"
            } else {
                "Rendering voices"
            },
        )?;
        let lock = studio_lock(core)?;
        let p = Progress {
            core,
            stop,
            _job: job,
            _lock: lock,
            ev: VoiceProgress {
                kind: kind.into(),
                running: true,
                total: total as u32,
                step: "Rendering".into(),
                ..Default::default()
            },
        };
        p.emit();
        Ok(p)
    }

    fn emit(&self) {
        self.core.hooks.event(Event::VoiceProgress(self.ev.clone()));
    }

    fn pair(&mut self, voice: &str, voice_name: &str, model: &str) {
        self.ev.voice = voice.into();
        self.ev.voice_name = voice_name.into();
        self.ev.model = model.into();
        self.ev.step = "Rendering".into();
        self.emit();
    }

    fn step(&mut self, step: &str) {
        self.ev.step = step.into();
        self.emit();
    }

    /// One batch done; false when a stop was asked for.
    fn batch(&mut self) -> bool {
        self.ev.done = (self.ev.done + 1).min(self.ev.total);
        self.emit();
        if self.stopped() && !self.ev.stopping {
            self.ev.stopping = true;
            self.emit();
        }
        !self.ev.stopping
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

impl Drop for Progress<'_> {
    fn drop(&mut self) {
        self.ev.running = false;
        self.emit();
    }
}

/// The provider id the studio's takes are cached under.
pub(super) const PROVIDER: &str = "elevenlabs";

/// A studio plan for one voice and model: the lines earlier takes hold, and the batches still
/// to make for the rest.
struct StudioPlan {
    reuse: Reuse,
    /// Batches of the other lines, less those the pack keeps whole.
    batches: Vec<Batch>,
    /// Batches the pack keeps whole.
    kept: u32,
    /// The cache keys of the batches still to make (`batch::priced`): what the estimate
    /// prices, what the confirm digest names, and the only batches the render may send.
    priced: Vec<String>,
}

impl StudioPlan {
    fn new(
        s: &Scope,
        index: &LineIndex,
        lines: &[(SetLine, String)],
        bs: &BatchSettings,
        limit: usize,
        pack: Option<&PackManifest>,
    ) -> Self {
        let (fresh, mut reuse) = reuse::split(s, index, &Core::studio_items(lines), bs);
        let all = batch::plan_within(&fresh, bs, limit);
        let (batches, mut kept) = still_to_do(all, pack, s.voice, s.model, s.settings);
        // A take the pack holds whole, with every path of its wanted lines, needs no new cut.
        if let Some(p) = pack {
            let mut paths: HashMap<&str, Vec<&str>> = HashMap::new();
            for (l, spoken) in lines {
                paths.entry(spoken).or_default().push(&l.line.path);
            }
            reuse.takes.retain(|(e, want)| {
                let whole =
                    p.kept_batches
                        .contains(&kept_key(s.voice, s.model, s.settings, &e.batch()))
                        && want.iter().all(|w| {
                            paths
                                .get(w.line.as_str())
                                .into_iter()
                                .flatten()
                                .all(|path| p.files.iter().any(|f| f == path))
                        });
                kept += u32::from(whole);
                !whole
            });
        }
        let priced = batches
            .iter()
            .filter(|b| s.to_send(b))
            .map(|b| s.key(b))
            .collect();
        StudioPlan {
            reuse,
            batches,
            kept,
            priced,
        }
    }

    /// The card paths of `lines` as (rendered, re-takes, to render). A text in two paths
    /// counts in both.
    fn counts(&self, s: &Scope, lines: &[(SetLine, String)]) -> (u32, u32, u32) {
        let billed: HashSet<&str> = self
            .batches
            .iter()
            .filter(|b| self.priced.contains(&s.key(b)))
            .flat_map(|b| b.items.iter().map(|(i, _)| i.spoken.as_str()))
            .collect();
        let retake: HashSet<&str> = self
            .reuse
            .lines()
            .filter(|(_, flagged)| *flagged)
            .map(|(l, _)| l)
            .collect();
        let (mut done, mut again, mut todo) = (0, 0, 0);
        for (_, spoken) in lines {
            if billed.contains(spoken.as_str()) {
                todo += 1;
            } else if retake.contains(spoken.as_str()) {
                again += 1;
            } else {
                done += 1;
            }
        }
        (done, again, todo)
    }

    /// Cuts the reused lines from their takes, makes the batches the estimate priced, and
    /// records both in the line index. The reused lines go first, as they cost nothing: a
    /// take that no longer cuts is removed before anything is sent. A failed index write is
    /// a note, not an error: the takes are paid for.
    fn render(
        &self,
        ctx: &batch::Ctx<'_>,
        bs: &BatchSettings,
        notes: &mut Vec<String>,
        each: &mut dyn FnMut(usize, usize) -> bool,
    ) -> Result<(batch::Rendered, Vec<Batch>)> {
        let s = Scope::of(ctx);
        let mut from_takes = batch::Rendered::default();
        let reused = match reuse::cut_reused(&s, &self.reuse, bs.snap_ms, &mut from_takes) {
            Ok(r) => r,
            Err(e) => {
                // Drops the removed takes from the index.
                let _ = LineIndex::record(&s, Vec::new());
                return Err(e);
            }
        };
        let mut done = match batch::render_all(ctx, &self.batches, bs.snap_ms, &self.priced, each) {
            Ok(d) => d,
            Err(e) => {
                // The batches made before the error are paid for: index them, unchecked
                // (cut 0), so a later plan reuses their lines instead of sending them again.
                let made = reuse::entries(&s, &self.batches, bs, &Default::default())
                    .into_iter()
                    .map(|e| reuse::Entry { cut: 0, ..e })
                    .collect();
                let _ = LineIndex::record(&s, made);
                return Err(e);
            }
        };
        let mut entries = reuse::entries(&s, &self.batches, bs, &done);
        done.cuts.extend(from_takes.cuts);
        done.flagged.extend(from_takes.flagged);
        done.warnings.extend(from_takes.warnings);
        done.batches_cached += from_takes.batches_cached;
        // The reused takes whose lines all passed: the pack keeps them whole.
        let whole: Vec<Batch> = reused
            .iter()
            .filter(|e| e.items.iter().all(|i| !e.needs_retake(&i.spoken)))
            .map(|e| e.batch())
            .collect();
        entries.extend(reused);
        if let Err(e) = LineIndex::record(&s, entries) {
            notes.push(format!(
                "The line index was not saved ({e:#}): the next estimate may price these lines again."
            ));
        }
        Ok((done, whole))
    }
}

/// The env var that holds the ElevenLabs key for one run, ahead of the Keychain.
pub const KEY_ENV: &str = "QUADCAM_ELEVENLABS_KEY";
/// The older name of `KEY_ENV`, still read for ElevenLabs only: no other provider gets it.
pub const OLD_KEY_ENV: &str = "QUADCAM_TTS_KEY";

impl Core {
    pub(super) fn eleven_key(&self) -> Result<Option<(String, &'static str)>> {
        for var in [KEY_ENV, OLD_KEY_ENV] {
            if let Some(k) = std::env::var(var).ok().filter(|k| !k.trim().is_empty()) {
                return Ok(Some((k.trim().to_string(), "environment")));
            }
        }
        Ok(self.gear.keys.get(ELEVENLABS)?.map(|k| (k, "keychain")))
    }

    fn studio_tts(&self) -> Result<Box<dyn Tts>> {
        let Some((key, _)) = self.eleven_key()? else {
            bail!("No ElevenLabs key is stored: run `quadcam-cli gear voice key set`, or paste it in the Voice studio.");
        };
        self.gear.tts.make(&ProviderConfig {
            provider: "elevenlabs".into(),
            base_url: String::new(),
            key: Some(key),
        })
    }

    fn key_status(&self) -> KeyStatus {
        let none = |problem| KeyStatus {
            set: false,
            hint: String::new(),
            prefix: String::new(),
            suffix: String::new(),
            length: 0,
            source: String::new(),
            problem,
        };
        match self.eleven_key() {
            Ok(Some((k, source))) => {
                let h = crate::secret::hint(&k);
                KeyStatus {
                    set: true,
                    hint: h.hint,
                    prefix: h.prefix,
                    suffix: h.suffix,
                    length: h.length,
                    source: source.into(),
                    problem: None,
                }
            }
            Ok(None) => none(None),
            Err(e) => none(Some(format!("{e:#}"))),
        }
    }

    /// Stores, deletes or reports the ElevenLabs key. The key never comes back.
    pub fn gear_voice_key(&self, p: &KeyParams) -> Result<KeyStatus> {
        match p.action.as_str() {
            "" | "status" => {}
            "set" => {
                let key = p.key.as_deref().map(str::trim).unwrap_or("");
                if key.is_empty() {
                    bail!("The key is empty.");
                }
                if key.chars().any(|c| c.is_whitespace() || c.is_control()) {
                    bail!("A key has no spaces or line breaks: paste only the key.");
                }
                self.gear.keys.set(ELEVENLABS, key)?;
            }
            "delete" => {
                self.gear.keys.delete(ELEVENLABS)?;
            }
            other => bail!("{other:?} is not a key action: use status, set or delete."),
        }
        Ok(self.key_status())
    }

    /// The key and the line sets, with the default picks for the aircraft the person flies;
    /// with a voice id and a model, how many lines of each set have a take. No network.
    pub fn gear_voice_sets(&self, p: &SetsParams) -> Result<StudioView> {
        let custom = self.custom_lines()?.len();
        let mut list = sets::infos(custom)?;
        let aircraft = self.flown_kinds();
        let picks = self.studio_picks(&list);
        let (voice, model) = (p.voice.trim(), p.model.trim());
        if !voice.is_empty() && !model.is_empty() {
            let settings = p.settings.clone().unwrap_or_default();
            settings.check()?;
            let bs = p.batch.clone().unwrap_or_default();
            bs.check()?;
            let cache = BatchCache::new(&self.cache);
            let s = Scope {
                cache: &cache,
                provider: PROVIDER,
                voice,
                model,
                settings: &settings,
            };
            let index = LineIndex::load(&s);
            let pack = self.local_pack(&local_pack_id(voice, model));
            // The account's request limit needs a call; the rate table knows the usual models.
            let limit = rates::batch_limit(model, 5000);
            for info in &mut list {
                let lines = if info.lines == 0 {
                    Vec::new()
                } else {
                    self.studio_lines(std::slice::from_ref(&info.id), &[])?
                };
                let plan = StudioPlan::new(&s, &index, &lines, &bs, limit, pack.as_ref());
                info.rendered = Some(plan.counts(&s, &lines).0);
            }
        }
        Ok(StudioView {
            key: self.key_status(),
            sets: list,
            batch: BatchSettings::default(),
            aircraft,
            picks,
            rendered: reuse::rendered_counts(&BatchCache::new(&self.cache), PROVIDER),
        })
    }

    /// Copies earlier ElevenLabs takes of single lines into the cache, so the studio counts
    /// them as rendered and never sends their lines again at that voice, model and speed. No
    /// network; the folder is only read.
    pub fn gear_voice_import_takes(&self, p: &ImportTakesParams) -> Result<ImportReport> {
        let folder = p.folder.trim();
        if folder.is_empty() {
            bail!("Name the folder of takes.");
        }
        reuse::import_takes(
            &BatchCache::new(&self.cache),
            PROVIDER,
            std::path::Path::new(folder),
            p.dry_run,
        )
    }

    /// The sets the studio ticks: every set it shows the first time, then the saved sets.
    /// `quadcam` is ticked each time, as the app's own cues need it; an untick lasts one run.
    fn studio_picks(&self, list: &[SetInfo]) -> Vec<String> {
        let shown = list
            .iter()
            .filter(|x| x.id != "sample" && !(x.id == sets::CUSTOM && x.lines == 0))
            .map(|x| x.id.clone());
        match self.studio_choices() {
            None => shown.collect(),
            Some(c) => shown
                .filter(|id| id == sets::QUADCAM || c.sets.contains(id))
                .collect(),
        }
    }

    fn studio_choices(&self) -> Option<StudioChoices> {
        let b = std::fs::read(self.gear_store().root().join(CHOICES_FILE)).ok()?;
        serde_json::from_slice(&b).ok()
    }

    /// Saves the line sets ticked in the studio into the gear folder. No network.
    pub fn gear_voice_save_sets(&self, p: &SaveSetsParams) -> Result<()> {
        let root = self.gear_store().root().to_path_buf();
        std::fs::create_dir_all(&root)?;
        let c = StudioChoices {
            sets: p.sets.clone(),
        };
        let tmp = root.join(format!(".{CHOICES_FILE}.part"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(&c)?)?;
        std::fs::rename(&tmp, root.join(CHOICES_FILE))?;
        Ok(())
    }

    /// The aircraft kinds the person flies: a word in a profile's name or aircraft
    /// (`sets::kind_of`), else a profile linked to an FC is a quad. `quad` when none says.
    fn flown_kinds(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in self.profiles().map(|(p, _)| p).unwrap_or_default() {
            let kind = sets::kind_of(&format!("{} {}", p.name, p.aircraft)).or(p
                .gear
                .fc
                .as_ref()
                .map(|_| "quad"));
            if let Some(k) = kind {
                if !out.iter().any(|o| o == k) {
                    out.push(k.into());
                }
            }
        }
        if out.is_empty() {
            out.push("quad".into());
        }
        out
    }

    /// The account's voices, models and credits (free calls).
    pub fn gear_voice_catalog(&self, p: &CatalogParams) -> Result<Catalog> {
        let all = !(p.voices || p.models || p.credits);
        let tts = self.studio_tts()?;
        let mut c = Catalog::default();
        if all || p.voices {
            c.voices = tts.voices()?;
        }
        if all || p.models {
            c.models = tts.models()?;
        }
        if all || p.credits {
            c.credits = Some(tts.credits()?);
        }
        Ok(c)
    }

    /// A page of ElevenLabs' Voice Library (`query`), or one voice (`id`): free calls. Each
    /// voice says whether the account has it already.
    pub fn gear_voice_library(&self, p: &LibraryParams) -> Result<LibraryPage> {
        let tts = self.studio_tts()?;
        let mut page = if p.id.trim().is_empty() {
            tts.library(&p.query)?
        } else {
            LibraryPage {
                voices: vec![tts.library_voice(p.id.trim())?],
                page: 0,
                has_more: false,
                total: 1,
            }
        };
        let mine: HashSet<String> = tts.voices()?.into_iter().map(|v| v.id).collect();
        for v in &mut page.voices {
            v.added |= mine.contains(&v.voice_id);
        }
        Ok(page)
    }

    /// Fetches a voice's preview sample (its `preview_url`) into the cache once, and answers
    /// the file to play. No credits.
    pub fn gear_voice_library_preview(&self, p: &LibraryPreviewParams) -> Result<String> {
        let url = p.url.trim();
        if !url.starts_with("https://") || url.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            bail!("A preview is fetched only from an https:// address.");
        }
        let path = url.split(['?', '#']).next().unwrap_or(url);
        let ext = path
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .filter(|e| matches!(e.as_str(), "mp3" | "wav" | "m4a" | "ogg"))
            .unwrap_or_else(|| "mp3".into());
        let dir = self.cache.join("voice").join("library-previews");
        std::fs::create_dir_all(&dir)?;
        let file = dir.join(format!(
            "{}.{ext}",
            &crate::gear::voice::sha256_hex(url.as_bytes())[..16]
        ));
        if !file.is_file() {
            let part = file.with_extension(format!("{ext}.part"));
            self.voice_fetch()
                .download(url, &part)
                .context("fetching the preview")?;
            std::fs::rename(&part, &file)?;
        }
        Ok(file.display().to_string())
    }

    /// Adds a Voice Library voice to the account's My Voices. It changes the account and takes
    /// a voice slot, so it waits for `confirm`; it costs no credits.
    pub fn gear_voice_add(&self, p: &VoiceAddParams) -> Result<VoiceAddReport> {
        let tts = self.studio_tts()?;
        let id = p.voice.trim();
        if id.is_empty() {
            bail!("Name the voice id to add.");
        }
        let mut voice = tts.library_voice(id)?;
        if !p.owner.trim().is_empty() {
            voice.public_owner_id = p.owner.trim().to_string();
        }
        let name = if p.name.trim().is_empty() {
            voice.name.clone()
        } else {
            p.name.trim().to_string()
        };
        let mine = tts.voices()?;
        let already = mine.iter().any(|v| v.id == voice.voice_id);
        voice.added |= already;
        let mut r = VoiceAddReport {
            voice,
            name,
            added_id: None,
            needs_confirm: false,
            already,
        };
        if already {
            r.added_id = Some(r.voice.voice_id.clone());
            return Ok(r);
        }
        if r.voice.public_owner_id.is_empty() {
            bail!("ElevenLabs does not say who shares this voice, so it cannot be added: add it on elevenlabs.io instead.");
        }
        if !p.confirm {
            r.needs_confirm = true;
            return Ok(r);
        }
        r.added_id = Some(tts.add_voice(&r.voice.public_owner_id, &r.voice.voice_id, &r.name)?);
        r.voice.added = true;
        Ok(r)
    }

    /// A voice by id or by name (any case), as `(id, name)`.
    fn resolve_voice(voices: &[VoiceInfo], want: &str) -> Result<(String, String)> {
        let want = want.trim();
        if want.is_empty() {
            bail!("Name a voice (quadcam-cli gear voice voices lists them).");
        }
        if let Some(v) = voices.iter().find(|v| v.id == want) {
            return Ok((v.id.clone(), v.name.clone()));
        }
        let hits: Vec<&VoiceInfo> = voices
            .iter()
            .filter(|v| v.name.eq_ignore_ascii_case(want))
            .collect();
        match hits.as_slice() {
            [v] => Ok((v.id.clone(), v.name.clone())),
            [] => bail!(
                "The account has no voice {want:?}. It has: {}.",
                voices
                    .iter()
                    .take(12)
                    .map(|v| v.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            _ => bail!("{want:?} names {} voices: use the voice id.", hits.len()),
        }
    }

    /// What a character costs on `model` in a voice billed at `rate` times the usual credits
    /// (a Voice Library voice with a rate of its own).
    fn pricing(
        &self,
        models: &[ModelInfo],
        model: &str,
        credits: &Credits,
        rate: Option<f64>,
    ) -> Result<Pricing> {
        let m = models.iter().find(|m| m.id == model).ok_or_else(|| {
            anyhow!(
                "The account lists no model {model:?}. It lists: {}.",
                models
                    .iter()
                    .map(|m| m.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
        let recorded = rates::Recorded::new(&self.cache).get(model, &rates::today());
        let basis = if recorded.is_some() {
            "recorded"
        } else if rates::rate(model).is_some() {
            "estimated"
        } else {
            "account"
        };
        let x = rate.unwrap_or(1.0);
        Ok(Pricing {
            cost_per_char: recorded.unwrap_or(m.cost_per_char) * x,
            basis,
            usd_per_1k: m.usd_per_1k * x,
            promo_until: m.promo_until.clone(),
            batch_limit: rates::batch_limit(model, m.max_chars),
            remaining: Some(credits.remaining),
        })
    }

    /// The sets as lines to speak: the spelling rules applied, each with its tone.
    fn studio_lines(&self, ids: &[String], only: &[String]) -> Result<Vec<(SetLine, String)>> {
        if ids.is_empty() {
            bail!("Name at least one line set: {}.", sets::ids().join(", "));
        }
        let custom: Vec<Line> = self
            .custom_lines()?
            .into_iter()
            .map(|c| Line {
                path: c.path,
                text: c.text,
                group: "extras".into(),
                why: "Your own line.".into(),
            })
            .collect();
        let mut all = sets::combine(ids, &custom)?;
        if !only.is_empty() {
            for o in only {
                if !all.iter().any(|l| &l.line.path == o) {
                    bail!("No line {o:?} in those sets.");
                }
            }
            all.retain(|l| only.contains(&l.line.path));
        }
        let sp = Spelling::builtin()?;
        Ok(all
            .into_iter()
            .map(|l| {
                let spoken = sp.apply(&l.line.text);
                (l, spoken)
            })
            .collect())
    }

    fn studio_items(lines: &[(SetLine, String)]) -> Vec<Item> {
        lines
            .iter()
            .map(|(l, spoken)| Item {
                spoken: spoken.clone(),
                tone: l.tone.clone(),
            })
            .collect()
    }

    /// What a batched render of the sets would cost, and whether the credits cover it.
    pub fn gear_voice_estimate(&self, p: &EstimateParams) -> Result<StudioEstimate> {
        let tts = self.studio_tts()?;
        let settings = p.settings.clone().unwrap_or_default();
        settings.check()?;
        let bs = p.batch.clone().unwrap_or_default();
        bs.check()?;
        let voices = tts.voices()?;
        let (voice, voice_name) = Self::resolve_voice(&voices, &p.voice)?;
        let rate = voice_rate(&voices, &voice);
        let model = self.studio_model(&p.model)?;
        let price = self.pricing(&tts.models()?, &model, &tts.credits()?, rate)?;
        let lines = self.studio_lines(&p.sets, &p.lines)?;
        let (_, old) = self.studio_pack(&voice, &voice_name, &model);
        let old = old.map(|(_, m)| m);
        let structures = adopt(
            old.as_ref(),
            &self.sample_structures(&voice, &voice_name, &model, settings.speed),
            &lines,
        );
        let (lines, bs) = structured(&lines, &bs, &structures)?;
        let cache = BatchCache::new(&self.cache);
        let ctx = batch::Ctx {
            tts: tts.as_ref(),
            cache: &cache,
            voice: &voice,
            model: &model,
            settings: &settings,
        };
        let s = Scope::of(&ctx);
        let plan = StudioPlan::new(
            &s,
            &LineIndex::load(&s),
            &lines,
            &bs,
            price.batch_limit,
            old.as_ref(),
        );
        let estimate = batch::estimate(&ctx, &plan.batches, &price);
        let (rendered, retakes, to_render) = plan.counts(&s, &lines);
        let validation = self.validation(&voice, &voice_name, &model, settings.speed);
        Ok(StudioEstimate {
            voice: voice.clone(),
            voice_name,
            model: model.clone(),
            lines: lines.len() as u32,
            rendered,
            retakes,
            to_render,
            estimate,
            rate,
            validation,
        })
    }

    fn studio_model(&self, asked: &str) -> Result<String> {
        let m = asked.trim();
        if !m.is_empty() {
            return Ok(m.to_string());
        }
        let (_, setting, _) = self.voice_config();
        if setting.is_empty() {
            bail!("Name a model (quadcam-cli gear voice models lists them).");
        }
        Ok(setting)
    }

    /// Short renders of the sample lines (or the sets named) in every voice and model, cut
    /// out of carrier sentences and written as WAVs to compare. Needs `confirm` when any
    /// batch is not in the cache yet; refuses when the credits do not cover it.
    pub fn gear_voice_sample(&self, p: &SampleParams) -> Result<SampleReport> {
        if p.voices.is_empty() || p.models.is_empty() {
            bail!("Name at least one voice and one model.");
        }
        let tts = self.studio_tts()?;
        let settings = p.settings.clone().unwrap_or_default();
        settings.check()?;
        let bs = p.batch.clone().unwrap_or_default();
        bs.check()?;
        let voices = tts.voices()?;
        let models = tts.models()?;
        let credits = tts.credits()?;
        let ids = if p.sets.is_empty() {
            vec!["sample".to_string()]
        } else {
            p.sets.clone()
        };
        let lines = self.studio_lines(&ids, &p.lines)?;
        let cache = BatchCache::new(&self.cache);
        let mut combos = Vec::new();
        let mut priced = Vec::new();
        let mut each = Vec::new();
        let mut total = Estimate {
            remaining: Some(credits.remaining),
            affordable: true,
            ..Default::default()
        };
        for v in &p.voices {
            let (vid, vname) = Self::resolve_voice(&voices, v)?;
            let rate = voice_rate(&voices, &vid);
            for m in &p.models {
                let price = self.pricing(&models, m, &credits, rate)?;
                let ctx = batch::Ctx {
                    tts: tts.as_ref(),
                    cache: &cache,
                    voice: &vid,
                    model: m,
                    settings: &settings,
                };
                let s = Scope::of(&ctx);
                // The voice's own ways of saying words, from an earlier grouped re-take.
                let found = self.sample_structures(&vid, &vname, m, settings.speed);
                let (lines, bs) = structured(&lines, &bs, &found)?;
                let plan = StudioPlan::new(
                    &s,
                    &LineIndex::load(&s),
                    &lines,
                    &bs,
                    price.batch_limit,
                    None,
                );
                let e = batch::estimate(&ctx, &plan.batches, &price);
                add_estimate(&mut total, &e);
                each.push(e);
                priced.push(serde_json::json!({
                    "voice": vid, "model": m, "to_render": plan.priced,
                }));
                combos.push((vid.clone(), vname.clone(), m.clone(), plan, lines, bs));
            }
        }
        total.affordable = credits.remaining >= total.credits;
        shared_rate(&mut total, &each.iter().collect::<Vec<_>>());
        let digest = plan_digest(&serde_json::json!({
            "kind": "sample", "combos": priced, "sets": ids, "lines": p.lines,
            "settings": settings, "batch": bs, "chars": total.chars, "credits": total.credits,
        }));
        let mut report = SampleReport {
            items: Vec::new(),
            estimate: total.clone(),
            combos: combos.len() as u32,
            needs_confirm: false,
            dry_run: p.dry_run,
            warnings: Vec::new(),
            digest: digest.clone(),
            stopped: false,
        };
        if p.dry_run {
            return Ok(report);
        }
        if total.chars > 0 {
            total.check()?;
            if !p.confirm {
                report.needs_confirm = true;
                return Ok(report);
            }
            same_plan(p.digest.as_deref(), &digest)?;
        }
        let tools = crate::media::find_tools().ok();
        // Samples from 0.19 and earlier move out of the cache first; the gallery shows a failure.
        let _ = self.move_cached_samples();
        let batches = combos
            .iter()
            .map(|(_, _, _, plan, ..)| plan.batches.len())
            .sum();
        let mut run = Progress::start(self, "sample", batches)?;
        for (vid, vname, model, plan, lines, bs) in combos {
            if run.stopped() {
                report.stopped = true;
                break;
            }
            run.pair(&vid, &vname, &model);
            let ctx = batch::Ctx {
                tts: tts.as_ref(),
                cache: &cache,
                voice: &vid,
                model: &model,
                settings: &settings,
            };
            let mut notes = Vec::new();
            let done = match plan.render(&ctx, &bs, &mut notes, &mut |_, _| run.batch()) {
                Ok((d, _)) => d,
                Err(e) if e.is::<batch::Stopped>() => {
                    report.stopped = true;
                    break;
                }
                Err(e) => return Err(e),
            };
            report.warnings.extend(notes);
            report.warnings.extend(
                done.warnings
                    .iter()
                    .map(|w| format!("{vname}, {model}: {w}")),
            );
            let dir = self.sample_dir(&vid, &vname, &model, settings.speed);
            std::fs::create_dir_all(&dir)?;
            for (l, spoken) in &lines {
                let Some(cut) = done.cuts.get(spoken) else {
                    continue;
                };
                let wav = render::normalise(cut, &settings, tools.as_ref())?;
                // The whole card path: SOUNDS/en/armed.wav and a script's armed.wav differ.
                lines::check_path(&l.line.path)?;
                let file = dir.join(&l.line.path);
                std::fs::create_dir_all(file.parent().unwrap_or(&dir))?;
                // Through a temporary file: a crash never leaves a half WAV in the gallery.
                let tmp = file.with_extension("tmp");
                std::fs::write(&tmp, &wav)?;
                std::fs::rename(&tmp, &file)?;
                report.items.push(SampleItem {
                    voice: vid.clone(),
                    voice_name: vname.clone(),
                    model: model.clone(),
                    line: l.line.path.clone(),
                    text: l.line.text.clone(),
                    file: file.display().to_string(),
                    ms: ((wav.len().saturating_sub(44)) as u64 * 1000 / (32000 * 2)) as u32,
                    heard: None,
                    heard_ok: None,
                });
            }
        }
        run.step("Checking speech");
        self.check_samples(&mut report, &settings);
        let notes = self.save_samples(&report, &voices, &ids, &settings);
        report.warnings.extend(notes);
        Ok(report)
    }

    /// `gear_voice_render` with `sets`: the sets as batched carrier sentences into the local
    /// pack for that voice and model. A paid render waits for `confirm`.
    pub(super) fn voice_studio_render(&self, p: &VoiceRenderParams) -> Result<RenderReport> {
        let tts = self.studio_tts()?;
        let settings = p.settings.clone().unwrap_or_default();
        settings.check()?;
        let bs = p.batch.clone().unwrap_or_default();
        bs.check()?;
        let (_, _, setting_voice) = self.voice_config();
        let want = if p.voice.trim().is_empty() {
            setting_voice
        } else {
            p.voice.clone()
        };
        let model = self.studio_model(&p.model)?;
        let lines = self.studio_lines(&p.sets, &p.lines)?;
        let account = Account::of(tts.as_ref())?;
        let prep = self.studio_prepare(
            tts.as_ref(),
            &account,
            &want,
            &model,
            &lines,
            &settings,
            &bs,
        )?;
        let digest = plan_digest(&serde_json::json!({
            "kind": "render", "voice": prep.voice, "model": prep.model, "sets": p.sets, "lines": p.lines,
            "settings": settings, "batch": bs, "to_render": prep.plan.priced,
            "chars": prep.est.chars, "credits": prep.est.credits,
        }));
        let mut report = prep.report.clone();
        report.dry_run = p.dry_run;
        report.digest = digest.clone();
        if p.dry_run {
            return Ok(report);
        }
        if prep.est.chars > 0 {
            prep.est.check()?;
            if !p.confirm {
                report.needs_confirm = true;
                return Ok(report);
            }
            same_plan(p.digest.as_deref(), &digest)?;
        }
        let mut run = Progress::start(self, "render", prep.plan.batches.len())?;
        let mut done = self.studio_finish(tts.as_ref(), prep, &settings, &mut run)?;
        done.digest = digest;
        Ok(done)
    }

    /// `gear_voice_render_packs`: the sets into one local pack per voice and model, priced as
    /// one plan with one digest. Without `confirm` (or with `dry_run`) it only prices: each
    /// pair's lines, what they cost, and the total. A failure keeps the packs made before it.
    pub fn gear_voice_render_packs(&self, p: &RenderPacksParams) -> Result<RenderPacksReport> {
        if p.voices.is_empty() || p.models.is_empty() {
            bail!("Name at least one voice and one model.");
        }
        let tts = self.studio_tts()?;
        let settings = p.settings.clone().unwrap_or_default();
        settings.check()?;
        let bs = p.batch.clone().unwrap_or_default();
        bs.check()?;
        let lines = self.studio_lines(&p.sets, &p.lines)?;
        let account = Account::of(tts.as_ref())?;
        let mut preps = Vec::new();
        for v in &p.voices {
            for m in &p.models {
                let model = self.studio_model(m)?;
                preps.push(self.studio_prepare(
                    tts.as_ref(),
                    &account,
                    v,
                    &model,
                    &lines,
                    &settings,
                    &bs,
                )?);
            }
        }
        let mut total = Estimate {
            remaining: Some(account.credits.remaining),
            affordable: true,
            ..Default::default()
        };
        for x in &preps {
            add_estimate(&mut total, &x.est);
        }
        shared_rate(
            &mut total,
            &preps.iter().map(|x| &x.est).collect::<Vec<_>>(),
        );
        total.affordable = account.credits.remaining >= total.credits;
        let digest = plan_digest(&serde_json::json!({
            "kind": "render_packs",
            "pairs": preps.iter().map(|x| serde_json::json!({
                "voice": x.voice, "model": x.model, "to_render": x.plan.priced,
            })).collect::<Vec<_>>(),
            "sets": p.sets, "lines": p.lines, "settings": settings, "batch": bs,
            "chars": total.chars, "credits": total.credits,
        }));
        let mut report = RenderPacksReport {
            pairs: preps
                .iter()
                .map(|x| x.studio_estimate(lines.len()))
                .collect(),
            estimate: total.clone(),
            lines: lines.len() as u32,
            needs_confirm: false,
            dry_run: p.dry_run,
            digest: digest.clone(),
            packs: Vec::new(),
            stopped: false,
            error: None,
            notes: Vec::new(),
        };
        if p.dry_run {
            return Ok(report);
        }
        if total.chars > 0 {
            total.check()?;
            if !p.confirm {
                report.needs_confirm = true;
                return Ok(report);
            }
            same_plan(p.digest.as_deref(), &digest)?;
        }
        let batches = preps.iter().map(|x| x.plan.batches.len()).sum();
        let mut run = Progress::start(self, "render", batches)?;
        for prep in preps {
            if run.stopped() {
                report.stopped = true;
                break;
            }
            let name = format!("{}, {}", prep.voice_name, prep.model);
            match self.studio_finish(tts.as_ref(), prep, &settings, &mut run) {
                Ok(r) => report.packs.push(r),
                Err(e) if e.is::<batch::Stopped>() => {
                    report.stopped = true;
                    report.notes.push(format!(
                        "Stopped during {name}: its batches made so far are in the cache, and the next render cuts them from there instead of paying again."
                    ));
                    break;
                }
                Err(e) => {
                    report.error = Some(format!("{name}: {e:#}"));
                    break;
                }
            }
        }
        if report.stopped && report.notes.is_empty() {
            report.notes.push("Stopped between packs.".into());
        }
        Ok(report)
    }

    /// One voice and model of a studio render, priced: the pack it goes into and the batches
    /// still to make. Makes no paid call.
    #[allow(clippy::too_many_arguments)]
    fn studio_prepare(
        &self,
        tts: &dyn Tts,
        account: &Account,
        want: &str,
        model: &str,
        lines: &[(SetLine, String)],
        settings: &RenderSettings,
        bs: &BatchSettings,
    ) -> Result<Prepared> {
        let (voice, voice_name) = Self::resolve_voice(&account.voices, want)?;
        let rate = voice_rate(&account.voices, &voice);
        let price = self.pricing(&account.models, model, &account.credits, rate)?;
        let (id, found) = self.studio_pack(&voice, &voice_name, model);
        let (old_id, old) = match found {
            Some((k, m)) => (Some(k), Some(m)),
            None => (None, None),
        };
        // The pack's structures, and the ones the voice's samples found for lines the pack does
        // not hold yet.
        let structures = adopt(
            old.as_ref(),
            &self.sample_structures(&voice, &voice_name, model, settings.speed),
            lines,
        );
        let (lines, bs) = structured(lines, bs, &structures)?;
        let (lines, bs) = (lines.as_slice(), &bs);
        let cache = BatchCache::new(&self.cache);
        let bctx = batch::Ctx {
            tts,
            cache: &cache,
            voice: &voice,
            model,
            settings,
        };
        let scope = Scope::of(&bctx);
        let plan = StudioPlan::new(
            &scope,
            &LineIndex::load(&scope),
            lines,
            bs,
            price.batch_limit,
            old.as_ref(),
        );
        let est = batch::estimate(&bctx, &plan.batches, &price);
        let counts = plan.counts(&scope, lines);
        let unique: HashSet<&String> = lines.iter().map(|(_, s)| s).collect();
        let mut report = RenderReport {
            pack: id.clone(),
            provider: "elevenlabs".into(),
            voice: voice_name.clone(),
            plan: Plan {
                lines: lines.len() as u32,
                cached: (lines.len() as u32).saturating_sub(counts.2),
                to_render: counts.2,
                chars: est.chars,
            },
            rendered: 0,
            from_cache: 0,
            paid: true,
            needs_confirm: false,
            dry_run: false,
            notes: vec![format!(
                "{} batches ({} cached), {} characters with carriers, {}; {} credits left; {} distinct lines.",
                est.batches,
                est.cached_batches,
                est.chars,
                cost_text(&est),
                est.remaining.unwrap_or(0),
                unique.len()
            )],
            estimate: Some(est.clone()),
            warnings: Vec::new(),
            digest: String::new(),
            retakes: Vec::new(),
            heard: Vec::new(),
        };
        let reused = plan.reuse.lines().count();
        if reused > 0 {
            report.notes.push(format!(
                "{reused} lines come from earlier takes of this voice and model: cut again, not sent."
            ));
        }
        if plan.kept > 0 {
            report.notes.push(format!(
                "{} batches already in the pack with every line passing its checks: kept, not rendered again.",
                plan.kept
            ));
        }
        let validation = self.validation(&voice, &voice_name, model, settings.speed);
        Ok(Prepared {
            rate,
            voice,
            voice_name,
            model: model.to_string(),
            id,
            old_id,
            old,
            plan,
            est,
            counts,
            report,
            lines: lines.to_vec(),
            bs: bs.clone(),
            structures,
            validation,
        })
    }

    /// Makes the batches a prepared render priced and builds its pack. `run` hears each batch
    /// and may stop the render between two of them (`batch::Stopped`).
    fn studio_finish(
        &self,
        tts: &dyn Tts,
        prep: Prepared,
        settings: &RenderSettings,
        run: &mut Progress<'_>,
    ) -> Result<RenderReport> {
        let Prepared {
            voice,
            voice_name,
            model,
            id,
            old_id,
            old,
            plan,
            mut report,
            lines,
            bs,
            structures,
            ..
        } = prep;
        let (lines, bs) = (lines.as_slice(), &bs);
        let cache = BatchCache::new(&self.cache);
        let bctx = batch::Ctx {
            tts,
            cache: &cache,
            voice: &voice,
            model: &model,
            settings,
        };
        run.pair(&voice, &voice_name, &model);
        let batches = &plan.batches;
        let (done, whole) = plan.render(&bctx, bs, &mut report.notes, &mut |_, _| run.batch())?;
        run.step("Building the pack");
        report.warnings = done.warnings.clone();
        let tools = crate::media::find_tools().ok();
        let rcache = self.voice_cache();
        let mut ctx = self.render_ctx(tts, &rcache, &voice, &model, settings, tools.as_ref());
        ctx.cuts = Some(&done.cuts);
        // Only lines with a cut that passed its checks: `render::take` refuses a line missing
        // from `cuts` rather than send it unpriced. Kept batches' lines are in the pack already.
        let chosen: Vec<Line> = lines
            .iter()
            .filter(|(_, spoken)| done.cuts.contains_key(spoken))
            .map(|(l, _)| l.line.clone())
            .collect();
        let base = Spelling::builtin()?;
        let spelling_for = |path: &str| packs::spelling_for(&structures, &base, path);
        let dir = self.voices_dir().join(&id);
        let stage = self.voices_dir().join(format!(".rendering-{id}"));
        let _ = std::fs::remove_dir_all(&stage);
        std::fs::create_dir_all(&stage)?;
        // A local pack keeps what an earlier render made: a second set adds to it. A pack
        // from 0.12.0 or earlier, named for the voice's name, moves to the new id here.
        let old_dir = self.voices_dir().join(old_id.as_deref().unwrap_or(&id));
        if old_dir.is_dir() {
            copy_dir(&old_dir, &stage)?;
        }
        let opts = BuildOpts {
            id: id.clone(),
            version: String::new(),
            voice: voice_name.clone(),
            lang: "en".into(),
            license: "Rendered by you; yours to use.".into(),
            attribution: String::new(),
            lines_csv_sha: lines::builtin_sha(),
        };
        // Lines a structure of the pack names are spoken their own way: each spelling renders
        // its own lines.
        let mut by_spelling: Vec<(Spelling, Vec<Line>)> = Vec::new();
        for l in &chosen {
            let sp = spelling_for(&l.path);
            match by_spelling.iter_mut().find(|(s, _)| *s == sp) {
                Some((_, ls)) => ls.push(l.clone()),
                None => by_spelling.push((sp, vec![l.clone()])),
            }
        }
        for (sp, ls) in &by_spelling {
            if let Err(e) = packs::render_to(&opts, &ctx, ls, sp, &stage, &mut |_, _| {}) {
                let _ = std::fs::remove_dir_all(&stage);
                return Err(e);
            }
        }
        // A flagged line has no take in the pack, not even an older one.
        let mut retakes: Vec<Retake> = Vec::new();
        for (l, spoken) in lines {
            if let Some(reason) = done.flagged.get(spoken) {
                let _ = std::fs::remove_file(stage.join(&l.line.path));
                retakes.push(Retake {
                    path: l.line.path.clone(),
                    text: l.line.text.clone(),
                    reason: reason.clone(),
                });
            }
        }
        let rendered_now: HashSet<&str> = lines.iter().map(|(l, _)| l.line.path.as_str()).collect();
        let mut kept_batches: Vec<String> = Vec::new();
        let mut heard = Vec::new();
        if let Some(o) = &old {
            // What the check heard still holds for a line not rendered into the pack now: one
            // the render left alone, or a re-take whose take is the one heard.
            heard = o
                .heard
                .iter()
                .filter(|h| {
                    !rendered_now.contains(h.path.as_str())
                        || !done
                            .cuts
                            .contains_key(&spelling_for(&h.path).apply(&h.text))
                })
                .cloned()
                .collect();
            retakes.extend(
                o.retakes
                    .iter()
                    .filter(|r| !rendered_now.contains(r.path.as_str()))
                    .cloned(),
            );
            kept_batches = o.kept_batches.clone();
        }
        for b in batches.iter().chain(&whole) {
            if b.items
                .iter()
                .all(|(i, _)| !done.flagged.contains_key(&i.spoken))
            {
                let k = kept_key(&voice, &model, settings, b);
                if !kept_batches.contains(&k) {
                    kept_batches.push(k);
                }
            }
        }
        let mut files: Vec<String> = Vec::new();
        collect_sounds(&stage, &stage, &mut files);
        files.sort();
        let manifest = PackManifest {
            id: id.clone(),
            voice: voice_name,
            lang: opts.lang,
            provider: "elevenlabs".into(),
            model: model.clone(),
            settings: settings.clone(),
            lines: files.len() as u32,
            license: opts.license,
            attribution: opts.attribution,
            lines_csv_sha: opts.lines_csv_sha,
            version: String::new(),
            files,
            retakes: retakes.clone(),
            kept_batches,
            firmware: packs::EDGETX.into(),
            voice_id: voice.clone(),
            heard,
            structures,
        };
        std::fs::write(
            stage.join("pack.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::rename(&stage, &dir)?;
        if let Some(from) = old_id.filter(|k| *k != id) {
            self.move_pack_refs(&from, &id)?;
            let _ = std::fs::remove_dir_all(&old_dir);
            report.notes.push(format!(
                "The pack {from} is now {id}; radios that chose it keep it."
            ));
        }
        report.rendered = done.batches_rendered;
        report.from_cache = done.batches_cached;
        // The lines rendered into the pack now, heard by the speech check.
        let into_pack: Vec<String> = chosen
            .iter()
            .map(|l| l.path.clone())
            .filter(|p| manifest.files.contains(p))
            .collect();
        run.step("Checking speech");
        self.check_after_render(&id, &into_pack, &mut report);
        let retakes = self.local_pack(&id).map(|m| m.retakes).unwrap_or(retakes);
        if !retakes.is_empty() {
            report.notes.push(format!(
                "{} lines need a re-take and are not in the pack: render again with another seed to redo only their batches.",
                retakes.len()
            ));
        }
        report.retakes = retakes;
        Ok(report)
    }

    /// The manifest of a local pack, when there is one.
    fn local_pack(&self, id: &str) -> Option<PackManifest> {
        let b = std::fs::read(self.voices_dir().join(id).join("pack.json")).ok()?;
        serde_json::from_slice(&b).ok()
    }

    /// The local pack id of a voice and model, and the pack a render adds to, with its id:
    /// the pack under that id, or a pack from 0.12.0 or earlier under the voice's name that this
    /// voice made (or that recorded no voice id).
    fn studio_pack(
        &self,
        voice: &str,
        voice_name: &str,
        model: &str,
    ) -> (String, Option<(String, PackManifest)>) {
        let id = local_pack_id(voice, model);
        if let Some(m) = self.local_pack(&id) {
            return (id.clone(), Some((id, m)));
        }
        let legacy = legacy_pack_id(voice_name, model);
        let found = self
            .local_pack(&legacy)
            .filter(|m| m.voice_id.is_empty() || m.voice_id == voice)
            .map(|m| (legacy, m));
        (id, found)
    }

    /// Where the samples go (for the CLI to print).
    pub fn voice_samples_dir(&self) -> PathBuf {
        self.samples_root()
    }
}

/// A voice's own credit multiplier, from the account's voice list.
fn voice_rate(voices: &[VoiceInfo], id: &str) -> Option<f64> {
    voices.iter().find(|v| v.id == id).and_then(|v| v.rate)
}

/// The local pack of a voice and model. A short hash of the voice id names it, so two voices
/// with the same name never share a pack.
fn local_pack_id(voice: &str, model: &str) -> String {
    format!(
        "local-elevenlabs-{}-{}",
        &crate::gear::voice::sha256_hex(voice.as_bytes())[..12],
        slug(model).chars().take(24).collect::<String>()
    )
}

/// The id a local pack had in 0.12.0 and earlier: the voice's name and the model.
fn legacy_pack_id(voice_name: &str, model: &str) -> String {
    format!(
        "local-elevenlabs-{}-{}",
        slug(voice_name).chars().take(24).collect::<String>(),
        slug(model).chars().take(24).collect::<String>()
    )
}

/// A batch as the pack remembers it: voice, model, settings, text and the cut's version,
/// without the seed, so a re-render with a new seed knows which batches the pack holds whole.
/// A new `batch::CUT_VERSION` makes every batch new again: a re-render cuts it once more (from
/// the batch cache, at no cost).
pub(super) fn kept_key(voice: &str, model: &str, settings: &RenderSettings, b: &Batch) -> String {
    let settings = RenderSettings {
        seed: 0,
        ..settings.clone()
    };
    crate::gear::voice::sha256_hex(
        serde_json::json!({
            "voice": voice, "model": model, "settings": settings, "text": b.text,
            "cut": batch::CUT_VERSION,
        })
        .to_string()
        .as_bytes(),
    )
}

/// The lines as a pack speaks them: each line's text with the pack's structures applied
/// (`PackManifest::spelling`), and a line whose structure names a carrier in a tone of its own
/// that carries it. Without a pack, the lines and settings as they are.
pub(super) fn structured(
    lines: &[(SetLine, String)],
    bs: &BatchSettings,
    structures: &[Structure],
) -> Result<(Vec<(SetLine, String)>, BatchSettings)> {
    if structures.is_empty() {
        return Ok((lines.to_vec(), bs.clone()));
    }
    let base = Spelling::builtin()?;
    let mut bs = bs.clone();
    let out = lines
        .iter()
        .map(|(l, _)| {
            let mut l = l.clone();
            if let Some(c) = packs::carrier_for(structures, &l.line.path) {
                let tone = format!("{}|{c}", l.tone);
                bs.tone_carriers.insert(tone.clone(), c.to_string());
                l.tone = tone;
            }
            let spoken = packs::spelling_for(structures, &base, &l.line.path).apply(&l.line.text);
            (l, spoken)
        })
        .collect();
    bs.check()?;
    Ok((out, bs))
}

/// The structures a pack render speaks its lines with: the pack's own, and each structure the
/// voice's samples found (`sample_structures`) carried to every line of the render with its
/// word that the pack does not hold yet and no structure of the pack covers. A line in the
/// pack keeps the way it was said.
fn adopt(
    pack: Option<&PackManifest>,
    found: &[Structure],
    lines: &[(SetLine, String)],
) -> Vec<Structure> {
    let mut out: Vec<Structure> = pack.map(|p| p.structures.clone()).unwrap_or_default();
    for s in found {
        let carried: Vec<String> = lines
            .iter()
            .map(|(l, _)| &l.line)
            .filter(|l| packs::has_word(&l.text, &s.word))
            .filter(|l| !pack.is_some_and(|p| p.files.contains(&l.path)))
            .filter(|l| {
                !out.iter()
                    .any(|o| o.word.eq_ignore_ascii_case(&s.word) && o.lines.contains(&l.path))
            })
            .map(|l| l.path.clone())
            .collect();
        if !carried.is_empty() {
            out.push(Structure {
                lines: carried,
                ..s.clone()
            });
        }
    }
    out
}

/// The batches a render still has to make: those the pack does not already hold with every
/// line passing its checks. Returns them and how many were kept.
fn still_to_do(
    all: Vec<Batch>,
    pack: Option<&PackManifest>,
    voice: &str,
    model: &str,
    settings: &RenderSettings,
) -> (Vec<Batch>, u32) {
    let Some(pack) = pack else {
        return (all, 0);
    };
    let n = all.len();
    let todo: Vec<Batch> = all
        .into_iter()
        .filter(|b| {
            !pack
                .kept_batches
                .contains(&kept_key(voice, model, settings, b))
        })
        .collect();
    let kept = (n - todo.len()) as u32;
    (todo, kept)
}

/// The digest a paid call's `confirm` repeats.
pub(super) fn plan_digest(plan: &serde_json::Value) -> String {
    crate::gear::voice::sha256_hex(plan.to_string().as_bytes())
}

/// A paid `confirm` pays only for the plan its estimate showed.
pub(super) fn same_plan(given: Option<&str>, want: &str) -> Result<()> {
    match given.map(str::trim).filter(|d| !d.is_empty()) {
        None => bail!("A paid call needs the digest of its estimate: run it without confirm, show the person the cost, then confirm with that digest."),
        Some(d) if d == want => Ok(()),
        Some(_) => bail!("The plan changed since its estimate (voices, models, sets, seed or characters): estimate again."),
    }
}

/// The key status as text.
pub fn key_text(k: &KeyStatus) -> String {
    if k.set {
        format!("An ElevenLabs key is stored ({}, {}).", k.hint, k.source)
    } else if let Some(p) = &k.problem {
        format!("No ElevenLabs key could be read: {p}")
    } else {
        "No ElevenLabs key is stored.".into()
    }
}

/// The catalogue as text: voices, models, credits.
pub fn catalog_text(c: &Catalog) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    if let Some(cr) = &c.credits {
        let _ = writeln!(
            s,
            "Credits: {} left of {} ({} plan, {} used).",
            cr.remaining, cr.limit, cr.tier, cr.used
        );
    }
    if !c.models.is_empty() {
        let _ = writeln!(s, "Models:");
        for m in &c.models {
            let _ = writeln!(
                s,
                "  {}  {}: ${} per 1K characters{}, about {} credits a character (estimated), {} characters a request",
                m.id,
                m.name,
                m.usd_per_1k,
                m.promo_until
                    .as_ref()
                    .map(|u| format!(" (promo until {u})"))
                    .unwrap_or_default(),
                m.cost_per_char,
                m.max_chars
            );
        }
    }
    if !c.voices.is_empty() {
        let _ = writeln!(s, "Voices:");
        for v in &c.voices {
            let _ = writeln!(
                s,
                "  {}  {} ({}{}{})",
                v.id,
                v.name,
                v.category,
                if v.labels.is_empty() { "" } else { ", " },
                v.labels
            );
        }
    }
    s
}

/// What an estimate costs: USD first, then credits, which are an estimate unless an earlier
/// call recorded the real count.
fn cost_text(e: &Estimate) -> String {
    let basis = if e.credits_basis == "recorded" {
        "recorded rate"
    } else {
        "estimated"
    };
    let rate = if e.usd_per_1k > 0.0 {
        format!(
            " at ${} per 1K{}",
            e.usd_per_1k,
            e.promo_until
                .as_ref()
                .map(|u| format!(", promo until {u}"))
                .unwrap_or_default()
        )
    } else {
        String::new()
    };
    format!("${:.2}{rate}, {} credits ({basis})", e.usd, e.credits)
}

/// The estimate as text.
pub fn estimate_text(e: &Estimate) -> String {
    format!(
        "{} batches ({} cached), {} lines to render, {} characters with carriers, {}; {} left{}.",
        e.batches,
        e.cached_batches,
        e.lines,
        e.chars,
        cost_text(e),
        e.remaining
            .map(|r| r.to_string())
            .unwrap_or_else(|| "unknown".into()),
        if e.affordable { "" } else { " (not enough)" }
    )
}

/// A studio estimate as text: the lines, how many are rendered, and the cost of the rest.
pub fn studio_estimate_text(e: &StudioEstimate) -> String {
    let retakes = if e.retakes > 0 {
        format!(", {} need a re-take", e.retakes)
    } else {
        String::new()
    };
    format!(
        "{} lines, {} with {}: {} rendered{retakes}, {} to render. {}\n{}",
        e.lines,
        e.voice_name,
        e.model,
        e.rendered,
        e.to_render,
        estimate_text(&e.estimate),
        validation_text(&e.validation)
    )
}

/// What a voice's samples of the risky lines showed, as text.
pub fn validation_text(v: &Validation) -> String {
    use std::fmt::Write;
    if v.sampled == 0 {
        return format!(
            "Risky lines: none sampled yet. Sample this voice first: the sample set holds the {} lines voices got wrong before.",
            v.lines
        );
    }
    let mut s = format!(
        "Risky lines: {} of {} sampled, {} heard right, {} heard wrong",
        v.sampled,
        v.lines,
        v.heard_right,
        v.heard_wrong.len()
    );
    if !v.structures.is_empty() {
        let _ = write!(
            s,
            "; {} ways of saying words found, carried to the pack",
            v.structures.len()
        );
    }
    s.push('.');
    for h in &v.heard_wrong {
        let _ = write!(s, "\n  {:?} heard {:?}", h.text, h.heard);
    }
    for st in &v.structures {
        let _ = write!(
            s,
            "\n  {:?}: say {:?}{}",
            st.word,
            st.say,
            st.carrier
                .as_deref()
                .map(|c| format!(" in {c:?}"))
                .unwrap_or_default()
        );
    }
    s
}

/// The line sets as text: the aircraft flown, the sets ticked, and per set what it holds.
pub fn sets_text(v: &StudioView) -> String {
    use std::fmt::Write;
    let mut s = key_text(&v.key);
    let _ = write!(
        s,
        "\nAircraft flown (from the profiles): {}. Ticked in the studio: {}.",
        v.aircraft.join(", "),
        v.picks.join(", ")
    );
    for t in &v.rendered {
        let _ = write!(
            s,
            "\nRendered: {} lines in voice {} with {} at speed {}.",
            t.lines, t.voice, t.model, t.speed
        );
    }
    for x in &v.sets {
        let rendered = x
            .rendered
            .map(|r| format!(", {r} rendered"))
            .unwrap_or_default();
        let kinds = if x.kinds.is_empty() {
            String::new()
        } else {
            format!(" For: {}.", x.kinds.join(", "))
        };
        let _ = write!(
            s,
            "\n{}: {} ({} lines{rendered}).{kinds} {}",
            x.id, x.title, x.lines, x.about
        );
    }
    s
}

/// A render of several packs as text: the question or the outcome, each pair, the total.
pub fn render_packs_text(r: &RenderPacksReport) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    if r.needs_confirm {
        let _ = writeln!(s, "Not rendered: this sends text to ElevenLabs, which bills it. Ask the person, then run again with confirm and digest {}.", r.digest);
    } else if r.dry_run {
        let _ = writeln!(
            s,
            "Dry run: nothing was rendered. A paid run confirms with digest {}.",
            r.digest
        );
    }
    for e in &r.pairs {
        let _ = writeln!(s, "{}", studio_estimate_text(e));
    }
    let _ = writeln!(
        s,
        "Total for {} voice and model pairs: {}",
        r.pairs.len(),
        estimate_text(&r.estimate)
    );
    for p in &r.packs {
        let _ = writeln!(
            s,
            "Rendered {} into {}: {} batches made, {} from the cache, {} re-takes.",
            p.voice,
            p.pack,
            p.rendered,
            p.from_cache,
            p.retakes.len()
        );
    }
    if r.stopped {
        let _ = writeln!(s, "Stopped after {} packs.", r.packs.len());
    }
    if let Some(e) = &r.error {
        let _ = writeln!(s, "Failed after {} packs: {e}", r.packs.len());
    }
    for n in &r.notes {
        let _ = writeln!(s, "{n}");
    }
    s
}

/// A library voice as one line: name, labels, languages, rate, and whether the account has it.
fn library_line(v: &LibraryVoice) -> String {
    let labels: Vec<&str> = [
        v.gender.as_str(),
        v.age.as_str(),
        v.accent.as_str(),
        v.use_case.as_str(),
        v.descriptive.as_str(),
    ]
    .into_iter()
    .filter(|x| !x.is_empty())
    .collect();
    let mut s = format!("{}  {} ({})", v.voice_id, v.name, labels.join(", "));
    if !v.languages.is_empty() {
        s.push_str(&format!("; languages {}", v.languages.join(", ")));
    } else if !v.language.is_empty() {
        s.push_str(&format!("; language {}", v.language));
    }
    if let Some(r) = v.rate {
        s.push_str(&format!("; bills {r}x credits"));
    }
    if let Some(d) = v.notice_days {
        s.push_str(&format!("; {d} days' notice before it leaves"));
    }
    if v.added {
        s.push_str("; in My Voices");
    }
    if !v.description.is_empty() {
        s.push_str(&format!(". {}", v.description));
    }
    s
}

/// A page of the Voice Library as text.
pub fn library_text(p: &LibraryPage) -> String {
    use std::fmt::Write;
    let mut s = format!(
        "{} voices on page {}{}{}.",
        p.voices.len(),
        p.page,
        if p.total > 0 {
            format!(" of {} matching", p.total)
        } else {
            String::new()
        },
        if p.has_more {
            "; more pages follow"
        } else {
            ""
        }
    );
    for v in &p.voices {
        let _ = write!(s, "\n  {}", library_line(v));
    }
    s
}

/// An add as text.
pub fn voice_add_text(r: &VoiceAddReport) -> String {
    if r.already {
        format!("The account has {} already.", r.voice.name)
    } else if r.needs_confirm {
        format!(
            "Not added: this adds {} to the ElevenLabs account as {:?} and takes a voice slot. Ask the person, then run again with confirm.\n  {}",
            r.voice.name,
            r.name,
            library_line(&r.voice)
        )
    } else {
        format!(
            "Added {} to My Voices as {:?} (id {}).",
            r.voice.name,
            r.name,
            r.added_id.as_deref().unwrap_or("")
        )
    }
}

/// An import of takes as text.
pub fn import_text(r: &ImportReport) -> String {
    use std::fmt::Write;
    let mut s = format!(
        "{} {} takes found: {} {}, {} in the cache already.",
        if r.dry_run { "Dry run:" } else { "" },
        r.found,
        r.imported,
        if r.dry_run {
            "would be imported"
        } else {
            "imported"
        },
        r.already
    )
    .trim_start()
    .to_string();
    for k in &r.skipped {
        let _ = write!(s, "\nLeft out: {k}");
    }
    for t in &r.counts {
        let _ = write!(
            s,
            "\n{} lines in voice {} with {} at speed {}.",
            t.lines, t.voice, t.model, t.speed
        );
    }
    s
}

/// A sample report as text.
pub fn sample_text(r: &SampleReport) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    if r.needs_confirm {
        let _ = writeln!(s, "Not rendered: this sends text to ElevenLabs, which bills it. Ask the person, then run again with confirm and digest {}.", r.digest);
    } else if r.dry_run {
        let _ = writeln!(
            s,
            "Dry run: nothing was rendered. A paid run confirms with digest {}.",
            r.digest
        );
    }
    let _ = writeln!(
        s,
        "{} voice and model pairs. {}",
        r.combos,
        estimate_text(&r.estimate)
    );
    for i in &r.items {
        let _ = writeln!(
            s,
            "  {} / {} / {}: {}ms {}",
            i.voice_name, i.model, i.text, i.ms, i.file
        );
    }
    for w in &r.warnings {
        let _ = writeln!(s, "Needs a re-take: {w}");
    }
    s
}
