//! The voice studio on the core (design 7.4): the ElevenLabs key in the Keychain, the
//! account's voices, models and credits, an estimate before any paid call, A/B samples per
//! voice and model, and a batched render of line sets into a local pack. Every paid call
//! checks the estimate against the credits first and waits for `confirm` with the digest of
//! the plan it priced. A cut that fails a check stays out of the pack as a re-take.
//! The studio always speaks to ElevenLabs; the `tts_provider` setting stays what it was.

use super::voice::{collect_sounds, copy_dir, slug};
use super::{Core, RenderReport, VoiceRenderParams};
use crate::gear::voice::batch::{self, Batch, BatchCache, BatchSettings, Estimate, Item, Pricing};
use crate::gear::voice::keychain::ELEVENLABS;
use crate::gear::voice::lines::{self, Line, Spelling};
use crate::gear::voice::packs::{self, BuildOpts, PackManifest, Retake};
use crate::gear::voice::rates;
use crate::gear::voice::render::{self, Plan, RenderSettings};
use crate::gear::voice::reuse::{self, LineIndex, Reuse, Scope, TakeCount};
use crate::gear::voice::sets::{self, SetInfo, SetLine};
use crate::gear::voice::tts::{Credits, ModelInfo, ProviderConfig, Tts, VoiceInfo};
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

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
    /// The sets picked at first: those made only for those kinds, and the person's own lines.
    #[serde(default)]
    pub picks: Vec<String>,
    /// The lines each voice, model and speed already have a take of, most first.
    #[serde(default)]
    pub rendered: Vec<TakeCount>,
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
}

/// The provider id the studio's takes are cached under.
const PROVIDER: &str = "elevenlabs";

/// A studio plan for one voice and model: the lines earlier takes hold, and the batches still
/// to make for the rest.
struct StudioPlan {
    reuse: Reuse,
    /// Batches of the other lines, less those the pack keeps whole.
    batches: Vec<Batch>,
    /// Batches the pack keeps whole.
    kept: u32,
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
        StudioPlan {
            reuse,
            batches,
            kept,
        }
    }

    /// The card paths of `lines` as (rendered, re-takes, to render). A text in two paths
    /// counts in both.
    fn counts(&self, s: &Scope, lines: &[(SetLine, String)]) -> (u32, u32, u32) {
        let billed: HashSet<&str> = self
            .batches
            .iter()
            .filter(|b| !s.cached(b))
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

    /// Makes the batches, cuts the reused lines from their takes, and records both in the
    /// line index. A failed index write is a note, not an error: the takes are paid for.
    fn render(
        &self,
        ctx: &batch::Ctx<'_>,
        bs: &BatchSettings,
        notes: &mut Vec<String>,
    ) -> Result<(batch::Rendered, Vec<Batch>)> {
        let s = Scope::of(ctx);
        let mut done = batch::render_all(ctx, &self.batches, bs.snap_ms, &mut |_, _| {})?;
        let mut entries = reuse::entries(&s, &self.batches, bs, &done);
        let reused = reuse::cut_reused(&s, &self.reuse, bs.snap_ms, &mut done)?;
        // The reused takes whose lines all passed: the pack keeps them whole.
        let whole: Vec<Batch> = reused
            .iter()
            .filter(|e| e.flagged.is_empty())
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
        let mut picks = sets::default_picks(&aircraft);
        if custom > 0 {
            picks.push(sets::CUSTOM.into());
        }
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

    fn pricing(&self, models: &[ModelInfo], model: &str, credits: &Credits) -> Result<Pricing> {
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
        let recorded = rates::Recorded::new(&self.cache).get(model);
        let basis = if recorded.is_some() {
            "recorded"
        } else if rates::rate(model).is_some() {
            "estimated"
        } else {
            "account"
        };
        Ok(Pricing {
            cost_per_char: recorded.unwrap_or(m.cost_per_char),
            basis,
            usd_per_1k: m.usd_per_1k,
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
        let (voice, voice_name) = Self::resolve_voice(&tts.voices()?, &p.voice)?;
        let model = self.studio_model(&p.model)?;
        let price = self.pricing(&tts.models()?, &model, &tts.credits()?)?;
        let lines = self.studio_lines(&p.sets, &p.lines)?;
        let (_, old) = self.studio_pack(&voice, &voice_name, &model);
        let old = old.map(|(_, m)| m);
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
        Ok(StudioEstimate {
            voice: voice.clone(),
            voice_name,
            model: model.clone(),
            lines: lines.len() as u32,
            rendered,
            retakes,
            to_render,
            estimate,
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
        let mut total = Estimate {
            remaining: Some(credits.remaining),
            affordable: true,
            ..Default::default()
        };
        for v in &p.voices {
            let (vid, vname) = Self::resolve_voice(&voices, v)?;
            for m in &p.models {
                let price = self.pricing(&models, m, &credits)?;
                let ctx = batch::Ctx {
                    tts: tts.as_ref(),
                    cache: &cache,
                    voice: &vid,
                    model: m,
                    settings: &settings,
                };
                let s = Scope::of(&ctx);
                let plan = StudioPlan::new(
                    &s,
                    &LineIndex::load(&s),
                    &lines,
                    &bs,
                    price.batch_limit,
                    None,
                );
                let e = batch::estimate(&ctx, &plan.batches, &price);
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
                priced.push(serde_json::json!({
                    "voice": vid, "model": m, "to_render": to_render(&ctx, &plan.batches),
                }));
                combos.push((vid.clone(), vname.clone(), m.clone(), plan));
            }
        }
        total.affordable = credits.remaining >= total.credits;
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
        let out_root = self.cache.join("voice").join("samples");
        for (vid, vname, model, plan) in combos {
            let ctx = batch::Ctx {
                tts: tts.as_ref(),
                cache: &cache,
                voice: &vid,
                model: &model,
                settings: &settings,
            };
            let mut notes = Vec::new();
            let (done, _) = plan.render(&ctx, &bs, &mut notes)?;
            report.warnings.extend(notes);
            report.warnings.extend(
                done.warnings
                    .iter()
                    .map(|w| format!("{vname}, {model}: {w}")),
            );
            let dir = out_root.join(format!("{}-{}", slug(&vname), slug(&model)));
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
                std::fs::write(&file, &wav)?;
                report.items.push(SampleItem {
                    voice: vid.clone(),
                    voice_name: vname.clone(),
                    model: model.clone(),
                    line: l.line.path.clone(),
                    text: l.line.text.clone(),
                    file: file.display().to_string(),
                    ms: ((wav.len().saturating_sub(44)) as u64 * 1000 / (32000 * 2)) as u32,
                });
            }
        }
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
        let (voice, voice_name) = Self::resolve_voice(&tts.voices()?, &want)?;
        let model = self.studio_model(&p.model)?;
        let price = self.pricing(&tts.models()?, &model, &tts.credits()?)?;
        let lines = self.studio_lines(&p.sets, &p.lines)?;
        let (id, found) = self.studio_pack(&voice, &voice_name, &model);
        let (old_id, old) = match found {
            Some((k, m)) => (Some(k), Some(m)),
            None => (None, None),
        };
        let cache = BatchCache::new(&self.cache);
        let bctx = batch::Ctx {
            tts: tts.as_ref(),
            cache: &cache,
            voice: &voice,
            model: &model,
            settings: &settings,
        };
        let scope = Scope::of(&bctx);
        let plan = StudioPlan::new(
            &scope,
            &LineIndex::load(&scope),
            &lines,
            &bs,
            price.batch_limit,
            old.as_ref(),
        );
        let (batches, kept) = (&plan.batches, plan.kept);
        let est = batch::estimate(&bctx, batches, &price);
        let (_, _, paths_to_render) = plan.counts(&scope, &lines);
        let digest = plan_digest(&serde_json::json!({
            "kind": "render", "voice": voice, "model": model, "sets": p.sets, "lines": p.lines,
            "settings": settings, "batch": bs, "to_render": to_render(&bctx, batches),
            "chars": est.chars, "credits": est.credits,
        }));
        let unique: std::collections::HashSet<&String> = lines.iter().map(|(_, s)| s).collect();
        let mut report = RenderReport {
            pack: id.clone(),
            provider: "elevenlabs".into(),
            voice: voice_name.clone(),
            plan: Plan {
                lines: lines.len() as u32,
                cached: (lines.len() as u32).saturating_sub(paths_to_render),
                to_render: paths_to_render,
                chars: est.chars,
            },
            rendered: 0,
            from_cache: 0,
            paid: true,
            needs_confirm: false,
            dry_run: p.dry_run,
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
            digest: digest.clone(),
            retakes: Vec::new(),
        };
        let reused = plan.reuse.lines().count();
        if reused > 0 {
            report.notes.push(format!(
                "{reused} lines come from earlier takes of this voice and model: cut again, not sent."
            ));
        }
        if kept > 0 {
            report.notes.push(format!(
                "{kept} batches already in the pack with every line passing its checks: kept, not rendered again."
            ));
        }
        if p.dry_run {
            return Ok(report);
        }
        if est.chars > 0 {
            est.check()?;
            if !p.confirm {
                report.needs_confirm = true;
                return Ok(report);
            }
            same_plan(p.digest.as_deref(), &digest)?;
        }
        let (done, whole) = plan.render(&bctx, &bs, &mut report.notes)?;
        report.warnings = done.warnings.clone();
        let tools = crate::media::find_tools().ok();
        let rcache = self.voice_cache();
        let mut ctx = self.render_ctx(
            tts.as_ref(),
            &rcache,
            &voice,
            &model,
            &settings,
            tools.as_ref(),
        );
        ctx.cuts = Some(&done.cuts);
        // Only lines with a cut that passed its checks: a line missing from `cuts` would go
        // to the provider one at a time, unpriced. Kept batches' lines are in the pack already.
        let chosen: Vec<Line> = lines
            .iter()
            .filter(|(_, spoken)| done.cuts.contains_key(spoken))
            .map(|(l, _)| l.line.clone())
            .collect();
        let spelling = Spelling::builtin()?;
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
        if !chosen.is_empty() {
            if let Err(e) =
                packs::render_to(&opts, &ctx, &chosen, &spelling, &stage, &mut |_, _| {})
            {
                let _ = std::fs::remove_dir_all(&stage);
                return Err(e);
            }
        }
        // A flagged line has no take in the pack, not even an older one.
        let mut retakes: Vec<Retake> = Vec::new();
        for (l, spoken) in &lines {
            if let Some(reason) = done.flagged.get(spoken) {
                let _ = std::fs::remove_file(stage.join(&l.line.path));
                retakes.push(Retake {
                    path: l.line.path.clone(),
                    text: l.line.text.clone(),
                    reason: reason.clone(),
                });
            }
        }
        let rendered_now: std::collections::HashSet<&str> =
            lines.iter().map(|(l, _)| l.line.path.as_str()).collect();
        let mut kept_batches: Vec<String> = Vec::new();
        if let Some(o) = &old {
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
                let k = kept_key(&voice, &model, &settings, b);
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
        self.cache.join("voice").join("samples")
    }
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
fn kept_key(voice: &str, model: &str, settings: &RenderSettings, b: &Batch) -> String {
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

/// The cache keys of the batches the provider would make: what a paid call pays for.
fn to_render(ctx: &batch::Ctx<'_>, batches: &[Batch]) -> Vec<String> {
    batches
        .iter()
        .filter(|b| !ctx.cached(b))
        .map(|b| ctx.key(b))
        .collect()
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
        "{} lines, {} with {}: {} rendered{retakes}, {} to render. {}",
        e.lines,
        e.voice_name,
        e.model,
        e.rendered,
        e.to_render,
        estimate_text(&e.estimate)
    )
}

/// The line sets as text: the aircraft flown, the first picks, and per set what it holds.
pub fn sets_text(v: &StudioView) -> String {
    use std::fmt::Write;
    let mut s = key_text(&v.key);
    let _ = write!(
        s,
        "\nAircraft flown (from the profiles): {}. Picked at first: {}.",
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
