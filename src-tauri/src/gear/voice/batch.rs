//! Batched carrier-sentence rendering (design 7.4). A lone word sounds wrong from a voice
//! (breathy edges, no sentence around it), so each line is spoken inside a carrier
//! ("The word is six.") and cut out afterwards:
//!
//! 1. `plan` groups lines by tone, wraps each in the carrier and joins up to `max_lines`
//!    sentences into one batch text, remembering where each line sits in it.
//! 2. `fetch` renders a batch with character timestamps, or finds it in the `BatchCache`
//!    (raw audio and alignment, by provider, voice, model, speed, batch text and seed).
//! 3. `cut` takes each line from its first character's start to its last character's end,
//!    each edge moved into the quiet between the line and the carrier (`snap_edge`): the
//!    timestamps can run 0.1 s off, so the audio decides. A line that opens with a fricative
//!    keeps up to 120 ms of its hiss when no quiet divides it from the carrier (`keep_hiss`).
//!    A fresh take is cached only after it cuts; a cached take that no longer cuts is removed.
//! 4. `check` flags cuts that are silent, whose length is off for their text, or that start
//!    with the carrier's last sound (`cut` drops a hiss it finds there first). A flagged line stays out of the pack and needs a re-take;
//!    its batch stays cached (it was paid).
//! 5. The cut goes on through `render::normalise` (trim, fades, loudness, tempo) like any take.
//!
//! Re-cutting from the cache costs nothing; `estimate` prices only the batches the cache
//! lacks, and `Pricing::check` refuses a render the credits do not cover.

use super::rates;
use super::render::RenderSettings;
use super::tts::{Aligned, Alignment, Tts, TtsRequest};
use super::wav::{self, Pcm};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// The placeholder of the carrier that stands for the line.
pub const SLOT: &str = "{line}";

fn carrier() -> String {
    format!("The word is {SLOT}.")
}
fn max_lines() -> u32 {
    30
}
fn snap_ms() -> u32 {
    40
}

/// The version of the way `cut` cuts and `check` judges. It changes whenever a cut comes out
/// differently or a check changes, so a pack cuts its kept batches again (from the batch
/// cache, at no cost). 5: a cut drops the carrier's last hiss instead of failing on it, and an
/// initialism's pace counts its letters' names.
pub const CUT_VERSION: u32 = 5;

/// How lines are batched and cut. Part of a batch's cache key only through the batch text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct BatchSettings {
    /// The sentence around a line; `{line}` stands for it, and the line ends the sentence.
    #[serde(default = "carrier")]
    pub carrier: String,
    /// A carrier for one tone (`alert`, `number`, `fun`, `calm`) instead of `carrier`.
    #[serde(default)]
    pub tone_carriers: BTreeMap<String, String>,
    /// Sentences in one batch at most.
    #[serde(default = "max_lines")]
    pub max_lines: u32,
    /// Each cut edge looks for the quiet at least this many milliseconds either side of its
    /// timestamp, and also back to the carrier's last letter or on to the next sentence's
    /// first. 0 cuts at the timestamps.
    #[serde(default = "snap_ms")]
    pub snap_ms: u32,
}

impl Default for BatchSettings {
    fn default() -> Self {
        Self {
            carrier: carrier(),
            tone_carriers: BTreeMap::new(),
            max_lines: max_lines(),
            snap_ms: snap_ms(),
        }
    }
}

impl BatchSettings {
    pub fn check(&self) -> Result<()> {
        check_carrier(&self.carrier)?;
        for (tone, c) in &self.tone_carriers {
            check_carrier(c).with_context(|| format!("the carrier for {tone}"))?;
        }
        if !(1..=100).contains(&self.max_lines) {
            bail!("max_lines takes 1 to 100, not {}", self.max_lines);
        }
        if self.snap_ms > 200 {
            bail!("snap_ms takes 0 to 200, not {}", self.snap_ms);
        }
        Ok(())
    }

    fn carrier_for(&self, tone: &str) -> &str {
        self.tone_carriers
            .get(tone)
            .map(String::as_str)
            .unwrap_or(&self.carrier)
    }
}

/// A carrier names its slot once, and the slot ends the sentence (a full stop may follow).
pub fn check_carrier(c: &str) -> Result<()> {
    if c.matches(SLOT).count() != 1 {
        bail!("a carrier holds {SLOT} exactly once, like \"The word is {SLOT}.\"");
    }
    let after = c.split(SLOT).nth(1).unwrap_or("");
    if !after.chars().all(|ch| matches!(ch, '.' | '!' | '?')) {
        bail!("nothing may follow {SLOT} in a carrier but its end mark: {c:?}");
    }
    Ok(())
}

/// One line to speak, with the tone that batches it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub spoken: String,
    pub tone: String,
}

/// One request's worth of sentences.
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    pub tone: String,
    pub text: String,
    /// Each item with its character range in `text`.
    pub items: Vec<(Item, std::ops::Range<usize>)>,
}

/// The text of one sentence, and where its line sits in it (character offsets).
fn sentence(carrier: &str, line: &str) -> (String, std::ops::Range<usize>) {
    let (pre, post) = carrier.split_once(SLOT).expect("checked carrier");
    let line = line.trim();
    // A line that ends its own sentence ("Beast mode!") takes no second full stop.
    let post = if line.ends_with(['.', '!', '?']) {
        ""
    } else {
        post
    };
    let start = pre.chars().count();
    let end = start + line.chars().count();
    (format!("{pre}{line}{post}"), start..end)
}

/// The sentence an item is spoken in: its tone's carrier around it.
pub fn sentence_of(s: &BatchSettings, i: &Item) -> String {
    sentence(s.carrier_for(&i.tone), &i.spoken).0
}

/// Groups items by tone (the order tones first appear in), drops repeats of the same spoken
/// text, and cuts each tone's list into batches of `max_lines`.
pub fn plan(items: &[Item], s: &BatchSettings) -> Vec<Batch> {
    plan_within(items, s, 0)
}

/// `plan`, with no batch longer than `max_chars` characters (0 for no limit). A batch holds
/// at least one sentence.
pub fn plan_within(items: &[Item], s: &BatchSettings, max_chars: usize) -> Vec<Batch> {
    let mut tones: Vec<&str> = Vec::new();
    for i in items {
        if !tones.contains(&i.tone.as_str()) {
            tones.push(&i.tone);
        }
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for tone in tones {
        let mine: Vec<&Item> = items
            .iter()
            .filter(|i| i.tone == tone && seen.insert(i.spoken.clone()))
            .collect();
        let (mut text, mut spans): (String, Vec<(Item, std::ops::Range<usize>)>) =
            (String::new(), Vec::new());
        let mut len = 0;
        for i in mine {
            let (sent, range) = sentence(s.carrier_for(tone), &i.spoken);
            let sent_len = sent.chars().count();
            let full = spans.len() >= s.max_lines as usize
                || (max_chars > 0 && !spans.is_empty() && len + 1 + sent_len > max_chars);
            if full {
                out.push(Batch {
                    tone: tone.to_string(),
                    text: std::mem::take(&mut text),
                    items: std::mem::take(&mut spans),
                });
            }
            if !text.is_empty() {
                text.push(' ');
            }
            let base = text.chars().count();
            spans.push((i.clone(), base + range.start..base + range.end));
            text.push_str(&sent);
            len = text.chars().count();
        }
        if !spans.is_empty() {
            out.push(Batch {
                tone: tone.to_string(),
                text,
                items: spans,
            });
        }
    }
    out
}

// ----- the batch cache -----

/// Raw batch audio and alignment: `<cache>/voice/batch/<provider>/<key>.wav` and `.json`. A
/// paid take that did not cut is kept as `<key>.failed.wav` and `.failed.json`: never cut
/// from, never sent again.
#[derive(Debug, Clone)]
pub struct BatchCache {
    root: PathBuf,
    cache_dir: PathBuf,
}

/// What the cache keeps beside the audio.
#[derive(Debug, Serialize, Deserialize)]
struct Stored {
    voice: String,
    model: String,
    speed: f64,
    seed: u64,
    text: String,
    alignment: Alignment,
    /// Why the take did not cut; only in a `.failed.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failed: Option<String>,
}

impl BatchCache {
    pub fn new(cache_dir: &Path) -> Self {
        Self {
            root: cache_dir.join("voice").join("batch"),
            cache_dir: cache_dir.to_path_buf(),
        }
    }

    /// The cache folder this was made on.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// The key of a batch: provider, voice, model, provider speed, batch text and seed.
    pub fn key(
        provider: &str,
        voice: &str,
        model: &str,
        speed: f64,
        text: &str,
        seed: u64,
    ) -> String {
        let canon = serde_json::json!({
            "provider": provider, "voice": voice, "model": model,
            "speed": format!("{speed:.4}"), "text": text, "seed": seed,
        });
        super::sha256_hex(canon.to_string().as_bytes())
    }

    /// The key of a take of one line spoken alone (`reuse::import_takes`). It never equals a
    /// batch's key, even a batch of one line whose carrier is only `{line}`.
    pub fn alone_key(
        provider: &str,
        voice: &str,
        model: &str,
        speed: f64,
        text: &str,
        seed: u64,
    ) -> String {
        let canon = serde_json::json!({
            "alone": true, "provider": provider, "voice": voice, "model": model,
            "speed": format!("{speed:.4}"), "text": text, "seed": seed,
        });
        super::sha256_hex(canon.to_string().as_bytes())
    }

    fn path(&self, provider: &str, key: &str, ext: &str) -> PathBuf {
        let p: String = provider
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        self.root.join(p).join(format!("{key}.{ext}"))
    }

    pub fn has(&self, provider: &str, key: &str) -> bool {
        self.path(provider, key, "wav").is_file() && self.path(provider, key, "json").is_file()
    }

    /// Why the paid take of this batch did not cut, when one was kept (`put_failed`).
    pub fn failed(&self, provider: &str, key: &str) -> Option<String> {
        let st: Stored =
            serde_json::from_slice(&std::fs::read(self.path(provider, key, "failed.json")).ok()?)
                .ok()?;
        Some(st.failed.unwrap_or_default())
    }

    /// Forgets a batch: its audio and its alignment.
    pub fn remove(&self, provider: &str, key: &str) {
        let _ = std::fs::remove_file(self.path(provider, key, "wav"));
        let _ = std::fs::remove_file(self.path(provider, key, "json"));
    }

    pub fn get(&self, provider: &str, key: &str) -> Option<Aligned> {
        let pcm = wav::read(&std::fs::read(self.path(provider, key, "wav")).ok()?).ok()?;
        let st: Stored =
            serde_json::from_slice(&std::fs::read(self.path(provider, key, "json")).ok()?).ok()?;
        Some(Aligned {
            pcm,
            alignment: st.alignment,
            billed_chars: None,
        })
    }

    /// Removes every batch of one voice and model. Returns how many it removed.
    pub fn remove_voice(&self, provider: &str, voice: &str, model: &str) -> u32 {
        let dir = self.path(provider, "x", "json");
        let Some(dir) = dir.parent() else {
            return 0;
        };
        let mut n = 0;
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "json") {
                continue;
            }
            let Some(st) = std::fs::read(&p)
                .ok()
                .and_then(|b| serde_json::from_slice::<Stored>(&b).ok())
            else {
                continue;
            };
            if st.voice == voice && st.model == model {
                let _ = std::fs::remove_file(p.with_extension("wav"));
                if std::fs::remove_file(&p).is_ok() {
                    n += 1;
                }
            }
        }
        n
    }

    #[allow(clippy::too_many_arguments)]
    pub fn put(
        &self,
        provider: &str,
        key: &str,
        voice: &str,
        model: &str,
        speed: f64,
        seed: u64,
        text: &str,
        a: &Aligned,
    ) -> Result<()> {
        let wav_path = self.path(provider, key, "wav");
        std::fs::create_dir_all(wav_path.parent().unwrap())?;
        let st = Stored {
            voice: voice.into(),
            model: model.into(),
            speed,
            seed,
            text: text.into(),
            alignment: a.alignment.clone(),
            failed: None,
        };
        // The alignment goes first: a WAV without its alignment would not count as cached.
        let jp = self.path(provider, key, "json");
        let jt = jp.with_extension("tmp");
        std::fs::write(&jt, serde_json::to_vec(&st)?)?;
        std::fs::rename(&jt, &jp)?;
        let wt = wav_path.with_extension("tmp");
        std::fs::write(&wt, wav::write(&a.pcm))?;
        std::fs::rename(&wt, &wav_path)?;
        Ok(())
    }

    /// Keeps a paid take that did not cut, with why, so no render sends its batch again.
    /// `has` stays false: no line is ever cut from it.
    pub fn put_failed(
        &self,
        provider: &str,
        key: &str,
        ctx: &Ctx<'_>,
        text: &str,
        a: &Aligned,
        why: &str,
    ) -> Result<()> {
        let jp = self.path(provider, key, "failed.json");
        std::fs::create_dir_all(jp.parent().unwrap())?;
        let st = Stored {
            voice: ctx.voice.into(),
            model: ctx.model.into(),
            speed: ctx.settings.speed,
            seed: ctx.settings.seed,
            text: text.into(),
            alignment: a.alignment.clone(),
            failed: Some(why.into()),
        };
        let wp = self.path(provider, key, "failed.wav");
        let wt = wp.with_extension("tmp");
        std::fs::write(&wt, wav::write(&a.pcm))?;
        std::fs::rename(&wt, &wp)?;
        let jt = jp.with_extension("tmp");
        std::fs::write(&jt, serde_json::to_vec(&st)?)?;
        std::fs::rename(&jt, &jp)?;
        Ok(())
    }
}

// ----- rendering a batch -----

/// What a batch render needs.
pub struct Ctx<'a> {
    pub tts: &'a dyn Tts,
    pub cache: &'a BatchCache,
    pub voice: &'a str,
    pub model: &'a str,
    pub settings: &'a RenderSettings,
}

impl Ctx<'_> {
    pub fn key(&self, b: &Batch) -> String {
        BatchCache::key(
            self.tts.id(),
            self.voice,
            self.model,
            self.settings.speed,
            &b.text,
            self.settings.seed,
        )
    }

    pub fn cached(&self, b: &Batch) -> bool {
        self.cache.has(self.tts.id(), &self.key(b))
    }

    /// Whether an estimate prices this batch: neither cached nor kept as a take that did not
    /// cut.
    pub fn to_send(&self, b: &Batch) -> bool {
        !self.cached(b) && self.cache.failed(self.tts.id(), &self.key(b)).is_none()
    }
}

/// One batch, cut: from the cache, or from the provider. A fresh take is cached only when it
/// cuts; a paid take that does not cut is kept apart (`put_failed`), and its batch is never
/// sent again: the same text would bill again for the same failure. A cached take that does
/// not cut is removed, and the next render fetches it again. The provider is asked only when
/// `may_send` (the estimate priced the batch): a cached take that cannot be read is removed
/// instead, so the next estimate prices it. True when the cache answered.
pub fn fetch(ctx: &Ctx<'_>, b: &Batch, snap_ms: u32, may_send: bool) -> Result<(Vec<Cut>, bool)> {
    let key = ctx.key(b);
    if let Some(a) = ctx.cache.get(ctx.tts.id(), &key) {
        return match cut(b, &a, snap_ms) {
            Ok(c) => Ok((c, true)),
            Err(e) => {
                ctx.cache.remove(ctx.tts.id(), &key);
                Err(e.context(format!(
                    "the cached batch of {} {} lines does not cut, so it was removed; a new estimate prices it again",
                    b.items.len(),
                    b.tone
                )))
            }
        };
    }
    if let Some(why) = ctx.cache.failed(ctx.tts.id(), &key) {
        bail!(
            "an earlier take of a batch of {} {} lines did not cut ({why}); it was paid for once and is not sent again: change that line's spelling, or render with another seed",
            b.items.len(),
            b.tone
        );
    }
    if !may_send {
        ctx.cache.remove(ctx.tts.id(), &key);
        bail!(
            "the cached batch of {} {} lines could not be read, so it was removed; a new estimate prices it again",
            b.items.len(),
            b.tone
        );
    }
    let a = ctx
        .tts
        .render_aligned(&TtsRequest {
            text: &b.text,
            voice: ctx.voice,
            model: ctx.model,
            speed: ctx.settings.speed,
            seed: ctx.settings.seed,
        })
        .with_context(|| format!("rendering a batch of {} {} lines", b.items.len(), b.tone))?;
    if let Some(billed) = a.billed_chars {
        rates::Recorded::new(ctx.cache.cache_dir()).record(
            ctx.model,
            billed,
            b.text.chars().count() as u64,
            &rates::today(),
        );
    }
    let cuts = match cut(b, &a, snap_ms) {
        Ok(c) => c,
        Err(e) => {
            let why = format!("{e:#}");
            let kept = ctx
                .cache
                .put_failed(ctx.tts.id(), &key, ctx, &b.text, &a, &why);
            return Err(e.context(format!(
                "the take of a batch of {} {} lines does not cut; {}",
                b.items.len(),
                b.tone,
                if kept.is_ok() {
                    "it is kept, and this batch is not sent again: change that line's spelling, or render with another seed"
                } else {
                    "it could not be kept, so the next render asks again"
                }
            )));
        }
    };
    ctx.cache.put(
        ctx.tts.id(),
        &key,
        ctx.voice,
        ctx.model,
        ctx.settings.speed,
        ctx.settings.seed,
        &b.text,
        &a,
    )?;
    Ok((cuts, false))
}

// ----- cutting -----

/// One line cut out of a batch.
#[derive(Debug, Clone)]
pub struct Cut {
    pub spoken: String,
    pub pcm: Pcm,
    /// Where it was cut, in the batch's seconds.
    pub from_s: f64,
    pub to_s: f64,
}

/// Where a cut edge goes, as a sample, searching `lo..=hi` (samples). The level at a point is
/// the mean square over the 10 ms around it, in 3 dB steps (`ilog2`; under 64, an RMS of 8,
/// counts as silence). The points at the lowest step make runs; the run nearest `at` is the
/// gap between line and carrier. A start edge sits up to 10 ms before the gap's end (the line
/// follows it), an end edge up to 10 ms after its start, never past the gap's middle. When the
/// gap holds `at`, the edge stays at `at`, at least that far inside the gap. Integer math, so
/// the same take always cuts the same.
pub fn snap_edge(p: &Pcm, lo: usize, hi: usize, at: usize, start: bool) -> usize {
    let len = p.samples.len();
    let hi = hi.min(len);
    if p.samples.is_empty() || lo >= hi {
        return at.min(len);
    }
    let step = (p.rate / 1000).max(1) as usize;
    let half = (p.rate as usize / 200).max(1);
    let base = lo.saturating_sub(half);
    let top = (hi + half).min(len);
    // Sums of squares from `base`, so each window costs two lookups.
    let mut sums = Vec::with_capacity(top - base + 1);
    sums.push(0u64);
    for v in &p.samples[base..top] {
        let sq = (i64::from(*v) * i64::from(*v)) as u64;
        sums.push(sums.last().copied().unwrap_or(0) + sq);
    }
    let step_of = |at: usize| -> u32 {
        let a = at.saturating_sub(half).max(base);
        let b = (at + half).min(top);
        if b <= a {
            return 0;
        }
        let ms = (sums[b - base] - sums[a - base]) / (b - a) as u64;
        if ms < 64 {
            0
        } else {
            ms.ilog2()
        }
    };
    let points: Vec<(usize, u32)> = (lo..=hi).step_by(step).map(|x| (x, step_of(x))).collect();
    let least = points.iter().map(|(_, l)| *l).min().unwrap_or(0);
    // The runs of points at the lowest step, as (first, last).
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut open: Option<(usize, usize)> = None;
    for (x, l) in &points {
        if *l == least {
            open = Some(open.map_or((*x, *x), |(a, _)| (a, *x)));
        } else if let Some(r) = open.take() {
            runs.push(r);
        }
    }
    runs.extend(open);
    let away = |r: &(usize, usize)| {
        if (r.0..=r.1).contains(&at) {
            0
        } else {
            r.0.abs_diff(at).min(r.1.abs_diff(at))
        }
    };
    let Some(gap) = runs.into_iter().min_by_key(away) else {
        return at.min(len);
    };
    let inset = ((gap.1 - gap.0) / 2).min(half * 2);
    if (gap.0..=gap.1).contains(&at) {
        // The timestamp is in the quiet already (or the window holds no quieter place).
        at.clamp(gap.0 + inset, gap.1 - inset)
    } else if start {
        gap.1 - inset
    } else {
        gap.0 + inset
    }
}

/// The hiss a line's first fricative keeps before its vowel, at most.
const KEEP_HISS_MS: usize = 120;

/// Whether a line's first sound is a fricative: s, z, f, h, th, sh, ch, or a soft c.
fn starts_with_hiss(spoken: &str) -> bool {
    let l: Vec<char> = spoken
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    matches!(
        l.as_slice(),
        ['s' | 'z' | 'f' | 'h', ..] | ['t' | 'c', 'h'] | ['c', 'e' | 'i' | 'y']
    )
}

/// Whether the 10 ms around sample `at` is hiss: louder than silence (a mean square of 64),
/// with `HISS_CROSSINGS` zero crossings or more.
fn hiss_at(p: &Pcm, at: usize) -> bool {
    let half = (p.rate as usize / 200).max(1);
    let a = at.saturating_sub(half);
    let b = (at + half).min(p.samples.len());
    if b <= a + 1 {
        return false;
    }
    let f = &p.samples[a..b];
    let ms = f
        .iter()
        .map(|v| (i64::from(*v) * i64::from(*v)) as u64)
        .sum::<u64>()
        / f.len() as u64;
    let zc = f.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
    // Scaled to 10 ms, so a window clipped at the take's edge counts the same.
    ms >= 64 && zc * 2 * half >= HISS_CROSSINGS as usize * f.len()
}

/// The start edge of a line that opens with a fricative. With no quiet between the carrier
/// and the line ("is Signal": the carrier's "s" runs into the line's), `snap_edge` finds the
/// quietest point at the hiss's end, just before the vowel, and the line loses its first
/// sound. When `at` sits in hiss, the edge moves back over it to `KEEP_HISS_MS` before the
/// hiss ends, never before `lo` or the hiss's start, and never later than `at`.
fn keep_hiss(p: &Pcm, lo: usize, at: usize, end: usize) -> usize {
    if !hiss_at(p, at) {
        return at;
    }
    let step = (p.rate / 1000).max(1) as usize;
    let mut from = at;
    while from >= lo + step && hiss_at(p, from - step) {
        from -= step;
    }
    let mut to = at;
    let end = end.min(p.samples.len());
    while to + step < end && hiss_at(p, to + step) {
        to += step;
    }
    let keep = p.rate as usize * KEEP_HISS_MS / 1000;
    at.min(from.max(to.saturating_sub(keep)))
}

/// How far from where the timestamps put it a pause between sentences is looked for.
const PAUSE_SEARCH_MS: usize = 600;
/// A pause between sentences lasts at least this long.
const MIN_PAUSE_MS: usize = 80;
/// A sentence heard this far or more from its timestamps is placed by the take.
const DRIFT_MS: usize = 150;

/// The level of each millisecond of a take: the mean square over the 10 ms around it, in
/// 3 dB steps, as `snap_edge` reads it.
fn levels(p: &Pcm) -> Vec<u32> {
    let step = (p.rate / 1000).max(1) as usize;
    let half = (p.rate as usize / 200).max(1);
    let len = p.samples.len();
    let mut sums = Vec::with_capacity(len + 1);
    sums.push(0u64);
    for v in &p.samples {
        let sq = (i64::from(*v) * i64::from(*v)) as u64;
        sums.push(sums.last().copied().unwrap_or(0) + sq);
    }
    (0..len.div_ceil(step))
        .map(|i| {
            let at = i * step;
            let a = at.saturating_sub(half);
            let b = (at + half).min(len);
            let ms = (sums[b] - sums[a]) / (b - a).max(1) as u64;
            if ms < 64 {
                0
            } else {
                ms.ilog2()
            }
        })
        .collect()
}

/// The longest quiet run that starts or ends in `lo..hi` (milliseconds of `lv`): points at most
/// one step above the window's quietest, `MIN_PAUSE_MS` or longer, in a window whose loudest
/// is four steps or more above its quietest. A run the window cuts off goes on to its own
/// ends. The earliest wins a tie.
fn pause_in(lv: &[u32], lo: usize, hi: usize) -> Option<(usize, usize)> {
    let hi = hi.min(lv.len());
    if lo >= hi {
        return None;
    }
    let least = *lv[lo..hi].iter().min()?;
    // Quiet only beside speech: 12 dB or more below the window's loudest.
    if *lv[lo..hi].iter().max()? < least + 4 {
        return None;
    }
    let quiet = |x: usize| lv[x] <= least + 1;
    let mut best: Option<(usize, usize)> = None;
    let mut open: Option<usize> = None;
    for x in lo..=hi {
        match (x < hi && quiet(x), open) {
            (true, None) => open = Some(x),
            (false, Some(a)) => {
                if best.is_none_or(|(b0, b1)| x - a > b1 - b0) {
                    best = Some((a, x));
                }
                open = None;
            }
            _ => {}
        }
    }
    let (mut a, mut b) = best?;
    while a > 0 && quiet(a - 1) {
        a -= 1;
    }
    while b < lv.len() && quiet(b) {
        b += 1;
    }
    (b - a >= MIN_PAUSE_MS).then_some((a, b))
}

/// Where a sentence was heard in the take, in ms: the end of the pause before it (its onset)
/// and the start of the pause after it.
#[derive(Debug, Clone, Copy, Default)]
struct Heard {
    onset: Option<usize>,
    pause: Option<usize>,
}

/// Finds each sentence in the take by the pauses between them. Each pause is the longest quiet
/// run within `PAUSE_SEARCH_MS` of the gap the timestamps give, moved by how far the take ran
/// from the timestamps at the pause before, and never before that pause. `sentences` holds
/// each sentence's first and last letter. The take's leading quiet gives the first onset, its
/// trailing quiet the last pause.
fn heard(p: &Pcm, al: &Alignment, sentences: &[(usize, usize)]) -> Vec<Heard> {
    let ms = |t: f64| (t.max(0.0) * 1000.0).round() as usize;
    let lv = levels(p);
    let end = lv.len();
    let mut out = vec![Heard::default(); sentences.len()];
    let Some(&(first, _)) = sentences.first() else {
        return out;
    };
    if let Some((0, p1)) = pause_in(&lv, 0, ms(al.starts[first]) + PAUSE_SEARCH_MS) {
        out[0].onset = Some(p1);
    }
    let moved = |t: usize, d: i64| (t as i64 + d).max(0) as usize;
    // How far the take runs behind the timestamps, as of the last pause.
    let mut drift: i64 = 0;
    let mut floor = 0;
    for (k, &(_, last)) in sentences.iter().enumerate() {
        let a = ms(al.ends[last]);
        let next = sentences.get(k + 1).map(|&(first, _)| ms(al.starts[first]));
        let lo = moved(a, drift).saturating_sub(PAUSE_SEARCH_MS).max(floor);
        let hi = next.map_or(end, |b| moved(b, drift) + PAUSE_SEARCH_MS);
        let Some((p0, p1)) = pause_in(&lv, lo, hi) else {
            continue;
        };
        match next {
            Some(b) => {
                out[k].pause = Some(p0);
                out[k + 1].onset = Some(p1);
                drift = p1 as i64 - b as i64;
            }
            None if p1 == end => out[k].pause = Some(p0),
            None => {}
        }
        floor = p1;
    }
    out
}

/// The value at `num / den` of the way up, or none.
fn quantile(mut v: Vec<i64>, num: usize, den: usize) -> Option<i64> {
    v.sort_unstable();
    v.get(v.len() * num / den).copied()
}

/// How far `cut` moves an item's timestamps onto the take, in seconds.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Moved {
    /// The line's start, and the start edge's search with it.
    start: f64,
    /// The line's end.
    end: f64,
    /// The next sentence's first letter, where the end edge stops looking.
    next: f64,
}

/// ElevenLabs' timestamps can drift up to a second off the audio for a stretch of a long
/// batch: they shrink the pause between sentences and stretch the carrier, so a line cut on
/// them starts in the carrier ("is eighty") and stops before its own last syllable ("eight").
/// A sentence whose onset in the take sits `DRIFT_MS` or more from its first letter's
/// timestamp is placed by the take instead. Its line starts as long after the onset as lines
/// do in the batch's sentences that fit (the lower quartile: the carrier is spoken much the
/// same each time, and a start too late loses the line's first sound). Its line ends as long
/// before the pause after it as they do (the median). Neither moves earlier than its timestamp.
/// The line before such a sentence looks for its end up to that sentence's onset. Sentences
/// that fit, and every sentence of a take with none that fit, keep their timestamps.
fn moved(al: &Alignment, b: &Batch, sentences: &[(usize, usize)], heard: &[Heard]) -> Vec<Moved> {
    let ms = |t: f64| (t.max(0.0) * 1000.0).round() as i64;
    let off = |k: usize| {
        let (first, _) = sentences[k];
        heard[k]
            .onset
            .map(|o| o as i64 - ms(al.starts[first]))
            .filter(|d| d.unsigned_abs() >= DRIFT_MS as u64)
    };
    let drifted: Vec<bool> = (0..sentences.len()).map(|k| off(k).is_some()).collect();
    let (mut lead, mut tail) = (Vec::new(), Vec::new());
    for (k, (_, r)) in b.items.iter().enumerate() {
        if drifted[k] {
            continue;
        }
        if let Some(o) = heard[k].onset {
            lead.push(ms(al.starts[r.start]) - o as i64);
        }
        if let Some(p) = heard[k].pause {
            tail.push(p as i64 - ms(al.ends[r.end - 1]));
        }
    }
    let (lead, tail) = (quantile(lead, 1, 4), quantile(tail, 1, 2));
    let s = |v: i64| v as f64 / 1000.0;
    b.items
        .iter()
        .enumerate()
        .map(|(k, (_, r))| {
            let mut m = Moved::default();
            if let (true, Some(o), Some(l)) = (drifted[k], heard[k].onset, lead) {
                m.start = s((o as i64 + l - ms(al.starts[r.start])).max(0));
            }
            if let (true, Some(p), Some(t)) = (drifted[k], heard[k].pause, tail) {
                m.end = s((p as i64 - t - ms(al.ends[r.end - 1])).max(0));
            }
            if k + 1 < sentences.len() {
                m.next = off(k + 1).map_or(0.0, s);
            }
            m
        })
        .collect()
}

/// The line where the provider's characters first differ from the batch text: the line that
/// holds that character, or the next one when it falls in a carrier.
fn changed_line<'a>(b: &'a Batch, got: &[String]) -> Option<&'a str> {
    let want: Vec<char> = b.text.chars().collect();
    let at = (0..want.len()).find(|&i| {
        got.get(i)
            .is_none_or(|c| c.chars().ne(std::iter::once(want[i])))
    })?;
    b.items
        .iter()
        .find(|(_, r)| r.end > at)
        .or(b.items.last())
        .map(|(i, _)| i.spoken.as_str())
}

/// Cuts every item of a batch out of its take. Fails when the alignment does not describe
/// the batch text or runs past the audio.
pub fn cut(b: &Batch, a: &Aligned, snap_ms: u32) -> Result<Vec<Cut>> {
    let n = b.text.chars().count();
    let al = &a.alignment;
    if al.characters.len() != n || al.starts.len() != n || al.ends.len() != n {
        bail!(
            "the timestamps cover {} characters but the batch has {n}: the provider changed the text{}",
            al.characters.len(),
            changed_line(b, &al.characters)
                .map(|l| format!(" of the line {l:?}"))
                .unwrap_or_default()
        );
    }
    let audio_s = a.pcm.samples.len() as f64 / f64::from(a.pcm.rate);
    let last = al.ends.iter().copied().fold(0.0, f64::max);
    if last > audio_s + 0.25 {
        bail!("the audio is {audio_s:.2} s but the timestamps run to {last:.2} s");
    }
    let chars: Vec<char> = b.text.chars().collect();
    let rate = f64::from(a.pcm.rate);
    let sample = |t: f64| ((t.max(0.0) * rate).round() as usize).min(a.pcm.samples.len());
    let snap = f64::from(snap_ms) / 1000.0;
    for (item, r) in &b.items {
        if r.is_empty() || r.end > n {
            bail!("the line {:?} has no place in the batch", item.spoken);
        }
    }
    // Each sentence's first and last letter: from the first letter after the line before it
    // through its own line's last.
    let mut sentences = Vec::new();
    let mut from = 0;
    for (_, r) in &b.items {
        let first = (from..r.end).find(|&i| chars[i].is_alphanumeric());
        let last = (from..r.end).rev().find(|&i| chars[i].is_alphanumeric());
        sentences.push((first.unwrap_or(r.start), last.unwrap_or(r.end - 1)));
        from = r.end;
    }
    let moved = if snap_ms > 0 {
        moved(al, b, &sentences, &heard(&a.pcm, al, &sentences))
    } else {
        Vec::new()
    };
    let mut out = Vec::new();
    for (k, (item, r)) in b.items.iter().enumerate() {
        let (t0, t1) = (al.starts[r.start], al.ends[r.end - 1]);
        let (mut s0, mut s1) = (sample(t0), sample(t1));
        if snap_ms > 0 {
            // The start looks back to the end of the letter before the line (the carrier's
            // last) and on through the line's first letter; the end looks on to the start of
            // the next letter (the next sentence), or to the end of the take. A sentence the
            // timestamps missed is first moved onto the take (`moved`).
            let Moved { start, end, next } = moved[k];
            let before = (0..r.start).rev().find(|&i| chars[i].is_alphanumeric());
            let after = (r.end..n).find(|&i| chars[i].is_alphanumeric());
            let lo0 = before.map_or(t0 - snap, |i| al.ends[i].min(t0 - snap)) + start;
            let hi0 = al.ends[r.start].max(t0 + snap) + start;
            let (t0, t1) = (t0 + start, t1 + end);
            let hi1 = after
                .map_or(audio_s, |i| al.starts[i] + next)
                .max(t1 + snap);
            s0 = snap_edge(&a.pcm, sample(lo0), sample(hi0), sample(t0), true);
            s1 = snap_edge(&a.pcm, sample(t1 - snap), sample(hi1), sample(t1), false);
            if starts_with_hiss(&item.spoken) {
                s0 = keep_hiss(&a.pcm, sample(lo0), s0, s1);
            }
        }
        if s1 <= s0 {
            // Snapping closed the gap: fall back to the alignment itself.
            s0 = (t0 * rate) as usize;
            s1 = ((t1 * rate) as usize).min(a.pcm.samples.len());
        }
        if s1 <= s0 {
            bail!("the line {:?} came out empty", item.spoken);
        }
        let mut c = Cut {
            spoken: item.spoken.clone(),
            pcm: Pcm {
                rate: a.pcm.rate,
                samples: a.pcm.samples[s0..s1].to_vec(),
            },
            from_s: s0 as f64 / rate,
            to_s: s1 as f64 / rate,
        };
        drop_bleed(&mut c);
        out.push(c);
    }
    Ok(out)
}

/// Drops the carrier's last sound from the start of a cut (`carrier_bleed`): the cut starts
/// in the quiet after the hiss.
fn drop_bleed(c: &mut Cut) {
    let Some((_, to, _)) = bleed(c) else {
        return;
    };
    let n = (c.pcm.rate as usize / 100).max(1) * to;
    c.pcm.samples.drain(..n.min(c.pcm.samples.len()));
    c.from_s += n as f64 / f64::from(c.pcm.rate);
}

// ----- checking cuts -----

/// A cut shorter than this is a clip, not a word.
const MIN_MS: f64 = 120.0;
/// A cut longer than this took in the carrier or a neighbour.
const MAX_MS: f64 = 4500.0;
/// A cut whose loudest sample is under this holds no speech.
const MIN_PEAK: i32 = 100;
/// A 10 ms frame with this many zero crossings or more is hiss (3 kHz and up), not voice.
const HISS_CROSSINGS: u32 = 30;

/// The mean square and zero crossings of each whole 10 ms frame.
fn frames(p: &Pcm) -> Vec<(u64, u32)> {
    let n = (p.rate as usize / 100).max(1);
    p.samples
        .chunks_exact(n)
        .map(|f| {
            let ms = f
                .iter()
                .map(|v| (i64::from(*v) * i64::from(*v)) as u64)
                .sum::<u64>()
                / n as u64;
            let zc = f.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count() as u32;
            (ms, zc)
        })
        .collect()
}

/// The carrier's last sound at the start of a cut: 20 to 120 ms of hiss, then 20 ms or more of
/// quiet, then the line (the cut began on the timestamp, inside the carrier's "s"). Returns the
/// hiss and the gap in ms. A line that starts with a hissing letter may start with a hiss and
/// a stop ("Stop", "Sky"), so it is not judged.
fn carrier_bleed(c: &Cut) -> Option<(u32, u32)> {
    bleed(c).map(|(from, to, gap)| ((to - from) as u32 * 10, (gap - to) as u32 * 10))
}

/// `carrier_bleed` in 10 ms frames: where the hiss starts, where it ends, and where the
/// quiet after it ends.
fn bleed(c: &Cut) -> Option<(usize, usize, usize)> {
    let first = c.spoken.chars().find(|x| x.is_alphanumeric())?;
    if "szxcfj".contains(first.to_ascii_lowercase()) {
        return None;
    }
    let f = frames(&c.pcm);
    let peak = f.iter().map(|(ms, _)| *ms).max()?;
    // Sound is within 20 dB of the cut's loudest frame.
    let sound = |i: usize| f[i].0.saturating_mul(100) >= peak;
    let mut i = 0;
    // The cut may start one frame before the hiss.
    if i < f.len() && !sound(i) {
        i += 1;
    }
    let mut j = i;
    while j < f.len() && sound(j) {
        j += 1;
    }
    let mut g = j;
    while g < f.len() && !sound(g) {
        g += 1;
    }
    let hiss = j - i;
    if !(2..=12).contains(&hiss) || g - j < 2 || g >= f.len() {
        return None;
    }
    let crossings = f[i..j].iter().map(|(_, z)| *z).sum::<u32>() / hiss as u32;
    (crossings >= HISS_CROSSINGS).then_some((i, j, g))
}

/// Each letter's name, A to Z, as a voice says it when it spells a word out.
pub const LETTER_NAMES: [&str; 26] = [
    "ay",
    "bee",
    "see",
    "dee",
    "ee",
    "ef",
    "gee",
    "aitch",
    "eye",
    "jay",
    "kay",
    "el",
    "em",
    "en",
    "oh",
    "pee",
    "cue",
    "ar",
    "ess",
    "tee",
    "you",
    "vee",
    "double you",
    "ex",
    "why",
    "zee",
];

/// The letters a line is spoken with, for its pace: a word of capitals and digits (an
/// initialism such as "OSD", "3D", or a letter of "L.Q.") counts the letters of each
/// character's name ("oh ess dee"), as it takes that long to say.
fn spoken_letters(spoken: &str) -> usize {
    const DIGITS: [&str; 10] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    ];
    spoken
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let initialism = w.chars().count() <= 4
                && w.chars().any(|c| c.is_ascii_uppercase())
                && w.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
            if !initialism {
                return w.chars().count();
            }
            w.chars()
                .map(|c| match c.to_digit(10) {
                    Some(d) => DIGITS[d as usize].len(),
                    None => LETTER_NAMES[(c as u8 - b'A') as usize]
                        .replace(' ', "")
                        .len(),
                })
                .sum()
        })
        .sum::<usize>()
        .max(1)
}

/// A cut the checks found wrong: its line needs a re-take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flag {
    pub spoken: String,
    pub reason: String,
}

impl std::fmt::Display for Flag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.spoken, self.reason)
    }
}

/// What is wrong with a cut, if anything. Reads the batch's cuts together: a line is long or
/// short for its letters against the batch's median pace. A take spoken alone (`carrier`
/// false) had no carrier, so its start is not judged for one.
pub fn check(cuts: &[Cut], carrier: bool) -> Vec<Flag> {
    let ms = |c: &Cut| c.pcm.samples.len() as f64 * 1000.0 / f64::from(c.pcm.rate);
    let letters = |c: &Cut| spoken_letters(&c.spoken) as f64;
    let mut pace: Vec<f64> = cuts.iter().map(|c| ms(c) / letters(c)).collect();
    pace.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = if pace.len() >= 4 {
        Some(pace[pace.len() / 2])
    } else {
        None
    };
    let mut out = Vec::new();
    for c in cuts {
        let d = ms(c);
        let peak = c
            .pcm
            .samples
            .iter()
            .map(|v| i32::from(*v).abs())
            .max()
            .unwrap_or(0);
        let reason = if peak < MIN_PEAK {
            Some("the cut is silent".to_string())
        } else if let Some((hiss, gap)) = carrier.then(|| carrier_bleed(c)).flatten() {
            Some(format!(
                "starts with {hiss} ms of hiss and a {gap} ms gap: the carrier's last sound"
            ))
        } else if d < MIN_MS {
            Some(format!("only {d:.0} ms"))
        } else if d > MAX_MS {
            Some(format!("{d:.0} ms is too long for one line"))
        } else if let Some(m) = median {
            let p = d / letters(c);
            // Short words are slower per letter than long ones, so the band is wide.
            if p > m * 3.5 {
                Some(format!(
                    "{d:.0} ms is long for its text (carrier or neighbour in the cut?)"
                ))
            } else if p < m * 0.25 {
                Some(format!("{d:.0} ms is short for its text (clipped?)"))
            } else {
                None
            }
        } else {
            None
        };
        if let Some(reason) = reason {
            out.push(Flag {
                spoken: c.spoken.clone(),
                reason,
            });
        }
    }
    out
}

// ----- estimate and price -----

/// What a render costs and what the account holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Pricing {
    /// Credits one character bills on the model.
    pub cost_per_char: f64,
    /// Where that figure comes from: `estimated` (the rate table), `recorded` (the
    /// `x-character-count` of an earlier call) or `account` (a model the table lacks).
    pub basis: &'static str,
    /// USD per 1,000 characters today.
    pub usd_per_1k: f64,
    /// The last day of the promo rate in `usd_per_1k`, while one applies.
    pub promo_until: Option<String>,
    /// The longest text one request takes here, or 0 for no limit: `plan_within` reads it.
    pub batch_limit: usize,
    /// Credits left on the account; `None` when the provider cannot say.
    pub remaining: Option<u64>,
}

/// What a set of batches would cost.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct Estimate {
    pub batches: u32,
    /// Batches the cache already holds.
    pub cached_batches: u32,
    /// Lines in the batches still to render.
    pub lines: u32,
    /// Characters of those batches, carriers included.
    pub chars: u64,
    pub cost_per_char: f64,
    /// `estimated`, `recorded` or `account`: where `cost_per_char` comes from.
    #[serde(default)]
    pub credits_basis: String,
    /// USD per 1,000 characters today.
    #[serde(default)]
    pub usd_per_1k: f64,
    /// The last day of a promo rate in `usd_per_1k`.
    #[serde(default)]
    pub promo_until: Option<String>,
    /// Credits the render bills (an estimate unless `credits_basis` is `recorded`).
    pub credits: u64,
    /// What the render costs in USD.
    #[serde(default)]
    pub usd: f64,
    pub remaining: Option<u64>,
    /// The credits cover the render (true when the provider cannot say).
    pub affordable: bool,
}

pub fn estimate(ctx: &Ctx<'_>, batches: &[Batch], price: &Pricing) -> Estimate {
    let mut e = Estimate {
        cost_per_char: price.cost_per_char,
        credits_basis: price.basis.into(),
        usd_per_1k: price.usd_per_1k,
        promo_until: price.promo_until.clone(),
        remaining: price.remaining,
        ..Default::default()
    };
    for b in batches {
        e.batches += 1;
        if !ctx.to_send(b) {
            e.cached_batches += 1;
        } else {
            e.lines += b.items.len() as u32;
            e.chars += b.text.chars().count() as u64;
        }
    }
    e.credits = (e.chars as f64 * price.cost_per_char).ceil() as u64;
    e.usd = e.chars as f64 * price.usd_per_1k / 1000.0;
    e.affordable = price.remaining.is_none_or(|r| e.credits <= r);
    e
}

impl Estimate {
    /// Refuses a render the credits do not cover.
    pub fn check(&self) -> Result<()> {
        if !self.affordable {
            bail!(
                "this render needs {} credits ({} characters at {} a character) but the account has {}",
                self.credits,
                self.chars,
                self.cost_per_char,
                self.remaining.unwrap_or(0)
            );
        }
        Ok(())
    }
}

// ----- the whole run -----

/// What `render_all` returns.
#[derive(Debug, Default)]
pub struct Rendered {
    /// Each spoken text with its cut; never a flagged one.
    pub cuts: HashMap<String, Pcm>,
    /// The cuts `check` flagged, by spoken text, with the reason. They need a re-take.
    pub flagged: HashMap<String, String>,
    pub warnings: Vec<String>,
    pub batches_rendered: u32,
    pub batches_cached: u32,
}

/// A run stopped between batches at the person's request. The batches made before it are in
/// the cache; `anyhow::Error::is::<Stopped>` tells it from a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stopped;

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("stopped after the current batch")
    }
}

impl std::error::Error for Stopped {}

/// Renders (or finds) every batch and cuts it. A flagged cut goes to `flagged`, not `cuts`.
/// Only the batches whose keys are in `priced` go to the provider. `each` hears (done,
/// total) after each batch; false from it stops the run there with `Stopped`.
pub fn render_all(
    ctx: &Ctx<'_>,
    batches: &[Batch],
    snap_ms: u32,
    priced: &[String],
    each: &mut dyn FnMut(usize, usize) -> bool,
) -> Result<Rendered> {
    let mut r = Rendered::default();
    for (n, b) in batches.iter().enumerate() {
        let (cuts, hit) = fetch(ctx, b, snap_ms, priced.contains(&ctx.key(b)))?;
        if hit {
            r.batches_cached += 1;
        } else {
            r.batches_rendered += 1;
        }
        for f in check(&cuts, true) {
            r.warnings.push(f.to_string());
            r.flagged.insert(f.spoken, f.reason);
        }
        for c in cuts {
            if !r.flagged.contains_key(&c.spoken) {
                r.cuts.insert(c.spoken, c.pcm);
            }
        }
        if !each(n + 1, batches.len()) && n + 1 < batches.len() {
            return Err(Stopped.into());
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gear::voice::eleven::fake::{timestamps_json, FakeHttp};
    use crate::gear::voice::eleven::Eleven;
    use std::sync::Arc;

    /// The keys an estimate made now would price.
    fn priced(ctx: &Ctx<'_>, batches: &[Batch]) -> Vec<String> {
        batches
            .iter()
            .filter(|b| ctx.to_send(b))
            .map(|b| ctx.key(b))
            .collect()
    }

    fn item(s: &str, tone: &str) -> Item {
        Item {
            spoken: s.into(),
            tone: tone.into(),
        }
    }

    fn price(cost_per_char: f64, remaining: Option<u64>) -> Pricing {
        Pricing {
            cost_per_char,
            basis: "estimated",
            usd_per_1k: 0.04,
            promo_until: None,
            batch_limit: 0,
            remaining,
        }
    }

    #[test]
    fn a_batch_stays_below_the_request_limit() {
        let v: Vec<Item> = (0..10)
            .map(|i| item(&format!("line {i}"), "calm"))
            .collect();
        let s = BatchSettings::default();
        let one = plan(&v, &s);
        assert_eq!(one.len(), 1);
        let limit = one[0].text.chars().count() / 3;
        let b = plan_within(&v, &s, limit);
        assert!(b.len() > 3);
        assert!(b.iter().all(|b| b.text.chars().count() <= limit));
        assert_eq!(b.iter().map(|b| b.items.len()).sum::<usize>(), 10);
        let chars: Vec<char> = b[1].text.chars().collect();
        for (i, r) in &b[1].items {
            assert_eq!(chars[r.clone()].iter().collect::<String>(), i.spoken);
        }
        // A sentence longer than the limit still gets a batch of its own.
        assert_eq!(plan_within(&v, &s, 3).len(), 10);
    }

    #[test]
    fn the_estimate_gives_usd_and_credits() {
        let dir = tempfile::tempdir().unwrap();
        let batches = plan(&[item("Six", "number")], &BatchSettings::default());
        let (_h, tts) = eleven_with(0.06, &batches[0]);
        let cache = BatchCache::new(dir.path());
        let settings = RenderSettings::default();
        let ctx = Ctx {
            tts: &tts,
            cache: &cache,
            voice: "abc",
            model: "m",
            settings: &settings,
        };
        let e = estimate(&ctx, &batches, &price(0.5, None));
        assert_eq!(e.chars, 16);
        assert_eq!(e.credits, 8);
        assert!((e.usd - 16.0 * 0.04 / 1000.0).abs() < 1e-12);
        assert_eq!(
            (e.usd_per_1k, e.credits_basis.as_str()),
            (0.04, "estimated")
        );
    }

    #[test]
    fn the_carrier_wraps_the_line_at_the_end() {
        let b = plan(
            &[item("Six", "number"), item("Armed", "number")],
            &BatchSettings::default(),
        );
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].text, "The word is Six. The word is Armed.");
        let chars: Vec<char> = b[0].text.chars().collect();
        for (i, r) in &b[0].items {
            assert_eq!(chars[r.clone()].iter().collect::<String>(), i.spoken);
        }
    }

    #[test]
    fn a_line_with_its_own_end_mark_takes_no_second_one() {
        let b = plan(&[item("Beast mode!", "fun")], &BatchSettings::default());
        assert_eq!(b[0].text, "The word is Beast mode!");
    }

    #[test]
    fn tones_split_batches_and_long_tones_split_again() {
        let mut v = vec![
            item("one", "number"),
            item("Failsafe", "alert"),
            item("two", "number"),
        ];
        for i in 0..5 {
            v.push(item(&format!("calm {i}"), "calm"));
        }
        let s = BatchSettings {
            max_lines: 3,
            ..Default::default()
        };
        let b = plan(&v, &s);
        let shape: Vec<(String, usize)> =
            b.iter().map(|b| (b.tone.clone(), b.items.len())).collect();
        assert_eq!(
            shape,
            vec![
                ("number".into(), 2),
                ("alert".into(), 1),
                ("calm".into(), 3),
                ("calm".into(), 2)
            ]
        );
    }

    #[test]
    fn a_repeated_spoken_text_renders_once() {
        let b = plan(
            &[item("Six", "number"), item("Six", "number")],
            &BatchSettings::default(),
        );
        assert_eq!(b[0].items.len(), 1);
    }

    #[test]
    fn a_tone_can_have_its_own_carrier() {
        let mut s = BatchSettings::default();
        s.tone_carriers
            .insert("number".into(), "The number is {line}.".into());
        let b = plan(&[item("six", "number"), item("Armed", "calm")], &s);
        assert_eq!(b[0].text, "The number is six.");
        assert_eq!(b[1].text, "The word is Armed.");
    }

    #[test]
    fn carriers_are_checked() {
        assert!(check_carrier("The word is {line}.").is_ok());
        assert!(check_carrier("I said {line}").is_ok());
        assert!(check_carrier("no slot.").is_err());
        assert!(check_carrier("{line} {line}.").is_err());
        assert!(check_carrier("{line} is the word.").is_err());
    }

    /// Two bursts at 0.30-0.50 s and 0.80-1.00 s of a 1.2 s take at 1 kHz units, with a
    /// quiet gap between them.
    fn burst_take() -> Pcm {
        let rate = 32000usize;
        let mut v = vec![0i16; rate * 12 / 10];
        for s in v.iter_mut().take(rate / 2).skip(rate * 3 / 10) {
            *s = 6000;
        }
        for s in v.iter_mut().take(rate).skip(rate * 8 / 10) {
            *s = -6000;
        }
        Pcm {
            rate: 32000,
            samples: v,
        }
    }

    #[test]
    fn snapping_moves_a_cut_into_the_quiet() {
        let p = burst_take();
        let s = |t: f64| (t * 32000.0) as usize;
        // An end edge 20 ms inside the first burst lands in the silence after it.
        let at = snap_edge(&p, s(0.44), s(0.70), s(0.48), false) as f64 / 32000.0;
        assert!((0.50..=0.52).contains(&at), "{at}");
        // A start edge 20 ms late, inside the second burst, lands in the silence before it.
        let at = snap_edge(&p, s(0.50), s(0.86), s(0.82), true) as f64 / 32000.0;
        assert!((0.78..=0.80).contains(&at), "{at}");
        // An empty window keeps the edge where it was.
        assert_eq!(snap_edge(&p, s(0.6), s(0.6), s(0.65), true), s(0.65));
    }

    /// A deterministic hiss: white noise of +-`amp`.
    fn hiss(n: usize, amp: i32, seed: &mut u32) -> Vec<i16> {
        (0..n)
            .map(|_| {
                *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((*seed >> 16) as i32 % (2 * amp + 1) - amp) as i16
            })
            .collect()
    }

    /// A tone of `hz` at `amp` for `n` samples at 32 kHz.
    fn tone(n: usize, hz: f64, amp: f64) -> Vec<i16> {
        (0..n)
            .map(|i| (amp * (i as f64 * hz * std::f64::consts::TAU / 32000.0).sin()) as i16)
            .collect()
    }

    /// The shape of a real take ("The word is Battery critical. The ..."), measured in 10 ms
    /// frames: "is" voiced to 3.11 s, its "s" hiss 3.11-3.165, a quiet gap 3.165-3.21, the
    /// line voiced 3.21-4.37, silence, the next "The" from 4.80. The timestamps put the
    /// line's "B" at 3.12, inside the hiss, and its last letter's end at 4.24, inside the word.
    fn carrier_take() -> (Batch, Aligned) {
        let ms = |t: f64| (t * 32000.0).round() as usize;
        let mut seed = 7;
        let mut v = hiss(ms(2.90), 2, &mut seed);
        v.extend(tone(ms(3.11) - v.len(), 220.0, 3000.0));
        v.extend(hiss(ms(3.165) - v.len(), 2500, &mut seed));
        v.extend(tone(ms(3.21) - v.len(), 60.0, 200.0));
        v.extend(tone(ms(4.30) - v.len(), 180.0, 6000.0));
        v.extend(tone(ms(4.37) - v.len(), 180.0, 300.0));
        v.extend(hiss(ms(4.80) - v.len(), 2, &mut seed));
        v.extend(tone(ms(5.10) - v.len(), 200.0, 5000.0));
        v.extend(hiss(ms(5.30) - v.len(), 2, &mut seed));
        let text = "is Battery critical. The";
        let n = text.chars().count();
        // Letters 2.88 s on at 80 ms each up to the line, the line's letters spread to 4.24,
        // the full stop and space to 4.80, then "The".
        let line = 3..19;
        let mut starts = Vec::with_capacity(n);
        for i in 0..n {
            starts.push(if i < line.start {
                2.88 + 0.08 * i as f64
            } else if i < line.end {
                3.12 + (4.24 - 3.12) * (i - line.start) as f64 / (line.end - line.start) as f64
            } else if i < 21 {
                4.24 + 0.28 * (i - line.end) as f64
            } else {
                4.80 + 0.08 * (i - 21) as f64
            });
        }
        let mut ends: Vec<f64> = starts.iter().skip(1).copied().collect();
        ends.push(starts[n - 1] + 0.08);
        let ch = |c: char| c.to_string();
        (
            Batch {
                tone: "alert".into(),
                text: text.into(),
                items: vec![(item("Battery critical", "alert"), line)],
            },
            Aligned {
                pcm: Pcm {
                    rate: 32000,
                    samples: v,
                },
                alignment: Alignment {
                    characters: text.chars().map(ch).collect(),
                    starts,
                    ends,
                },
                billed_chars: None,
            },
        )
    }

    #[test]
    fn a_cut_starts_after_the_carriers_hiss_and_ends_after_the_word() {
        let (b, a) = carrier_take();
        assert!((a.alignment.starts[3] - 3.12).abs() < 1e-9);
        let c = cut(&b, &a, 40).unwrap();
        // In the gap: after the hiss, before the word.
        assert!(
            (3.165..=3.21).contains(&c[0].from_s),
            "starts at {}",
            c[0].from_s
        );
        // After the word's tail, before the next "The".
        assert!((4.37..=4.80).contains(&c[0].to_s), "ends at {}", c[0].to_s);
        assert!(check(&c, true).is_empty(), "{:?}", check(&c, true));
        // Cut on the timestamps alone, the line would start with the carrier's "s": the cut
        // drops it and starts in the quiet after it.
        let raw = cut(&b, &a, 0).unwrap();
        assert!(
            (3.13..=3.21).contains(&raw[0].from_s),
            "starts at {}",
            raw[0].from_s
        );
        assert!(check(&raw, true).is_empty(), "{:?}", check(&raw, true));
        // A cut that still starts with the hiss is flagged.
        let n = ((raw[0].from_s - 3.12) * 32000.0).round() as usize;
        let start = (3.12 * 32000.0) as usize;
        let bled = Cut {
            pcm: Pcm {
                rate: 32000,
                samples: [
                    &a.pcm.samples[start..start + n],
                    raw[0].pcm.samples.as_slice(),
                ]
                .concat(),
            },
            from_s: 3.12,
            ..raw[0].clone()
        };
        let flags = check(std::slice::from_ref(&bled), true);
        assert_eq!(flags.len(), 1, "{flags:?}");
        assert!(
            flags[0].reason.contains("hiss") && flags[0].reason.contains("gap"),
            "{}",
            flags[0].reason
        );
        // A take spoken alone had no carrier: its start is not judged for one.
        let alone = check(std::slice::from_ref(&bled), false);
        assert!(alone.is_empty(), "{alone:?}");
    }

    #[test]
    fn an_initialism_is_paced_by_its_letters_names() {
        assert_eq!(spoken_letters("L.Q."), 5);
        assert_eq!(spoken_letters("OSD"), 8);
        assert_eq!(spoken_letters("3D"), 8);
        assert_eq!(spoken_letters("Signal low"), 9);
        assert_eq!(spoken_letters("Armed"), 5);
        // An LQ of a second among words of a usual pace is not long; three seconds is.
        let mut cuts = vec![
            cut_of("battery low", 800, 3000),
            cut_of("signal low", 700, 3000),
            cut_of("armed", 450, 3000),
            cut_of("disarmed", 600, 3000),
            cut_of("L.Q.", 1100, 3000),
            cut_of("O.S.D.", 1300, 3000),
        ];
        assert!(check(&cuts, true).is_empty(), "{:?}", check(&cuts, true));
        cuts.push(cut_of("V.T.X.", 3000, 3000));
        let w = check(&cuts, true);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].reason.contains("long for its text"));
    }

    #[test]
    fn a_line_that_starts_with_a_hiss_keeps_it() {
        // "Six" after "is": one hiss from the carrier's "s" into the line, no gap. The cut
        // stays near the timestamp, and a hissing first letter is never flagged.
        let mut seed = 3;
        let mut v = tone(32000 / 2, 220.0, 3000.0);
        v.extend(hiss(32000 * 3 / 10, 2000, &mut seed));
        v.extend(tone(32000 / 5, 180.0, 6000.0));
        v.extend(hiss(32000 * 3 / 10, 2, &mut seed));
        let pcm = Pcm {
            rate: 32000,
            samples: v,
        };
        let s = |t: f64| (t * 32000.0) as usize;
        let at = snap_edge(&pcm, s(0.55), s(0.71), s(0.65), true) as f64 / 32000.0;
        assert!((0.60..=0.70).contains(&at), "{at}");
        let c = Cut {
            spoken: "Six".into(),
            pcm: Pcm {
                rate: 32000,
                samples: pcm.samples[s(0.62)..s(1.1)].to_vec(),
            },
            from_s: 0.62,
            to_s: 1.1,
        };
        assert_eq!(carrier_bleed(&c), None);
    }

    #[test]
    fn a_fricative_onset_is_spotted() {
        for l in [
            "Signal critical",
            "six",
            "Failsafe",
            "Home",
            "Throttle",
            "Shutdown",
            "Cell low",
            "Zero",
            "Check",
        ] {
            assert!(starts_with_hiss(l), "{l}");
        }
        for l in [
            "Battery low",
            "Armed",
            "Turtle mode",
            "Critical",
            "Return to home",
            "1",
            "",
        ] {
            assert!(!starts_with_hiss(l), "{l}");
        }
    }

    /// "is Failsafe" as the provider speaks it: the carrier's "i" voiced to 3.00 s, its "s" a
    /// loud hiss to 3.06, the line's "f" a low, high-pitched hiss to 3.20 that fades to 3.23,
    /// then the vowel. No quiet anywhere between. The timestamps put the "F" at 3.20.
    fn fricative_take(line: &str) -> (Batch, Aligned) {
        let ms = |t: f64| (t * 32000.0).round() as usize;
        let mut seed = 11;
        let mut v = hiss(ms(2.80), 2, &mut seed);
        v.extend(tone(ms(3.00) - v.len(), 220.0, 3000.0));
        v.extend(hiss(ms(3.06) - v.len(), 2000, &mut seed));
        // Band-limited: the first difference of white noise leans to the high end.
        let f = |n: usize, amp: i32, seed: &mut u32| -> Vec<i16> {
            let w = hiss(n + 1, amp, seed);
            w.windows(2)
                .map(|p| ((i32::from(p[1]) - i32::from(p[0])) / 2) as i16)
                .collect()
        };
        v.extend(f(ms(3.20) - v.len(), 400, &mut seed));
        v.extend(f(ms(3.23) - v.len(), 120, &mut seed));
        v.extend(tone(ms(3.60) - v.len(), 180.0, 6000.0));
        v.extend(hiss(ms(4.00) - v.len(), 2, &mut seed));
        let text = format!("is {line}.");
        let n = text.chars().count();
        let span = 3..3 + line.chars().count();
        let starts: Vec<f64> = (0..n)
            .map(|i| match i {
                0 => 2.88,
                1 => 2.98,
                2 => 3.06,
                i if i < span.end => 3.20 + 0.03 * (i - span.start) as f64,
                _ => 3.62,
            })
            .collect();
        let mut ends: Vec<f64> = starts.iter().skip(1).copied().collect();
        ends.push(3.70);
        (
            Batch {
                tone: "alert".into(),
                text: text.clone(),
                items: vec![(item(line, "alert"), span)],
            },
            Aligned {
                pcm: Pcm {
                    rate: 32000,
                    samples: v,
                },
                alignment: Alignment {
                    characters: text.chars().map(|c| c.to_string()).collect(),
                    starts,
                    ends,
                },
                billed_chars: None,
            },
        )
    }

    #[test]
    fn a_fricative_onset_keeps_its_hiss() {
        let (b, a) = fricative_take("Failsafe");
        let s = |t: f64| (t * 32000.0) as usize;
        // Snapping alone finds the quietest place at the hiss's fading end, at the vowel.
        let snapped = snap_edge(&a.pcm, s(3.06), s(3.24), s(3.20), true) as f64 / 32000.0;
        assert!(snapped >= 3.18, "snaps to {snapped}");
        // The cut keeps 120 ms of the hiss before the vowel, not the carrier's loud "s".
        let c = cut(&b, &a, 40).unwrap();
        assert!(
            (3.09..=3.13).contains(&c[0].from_s),
            "starts at {}",
            c[0].from_s
        );
        let lead = &c[0].pcm.samples[..s(0.10)];
        let ms = lead.iter().map(|v| i64::from(*v).pow(2)).sum::<i64>() / lead.len() as i64;
        assert!(ms > 64 * 64, "the hiss is kept: {ms}");
        // Trim and fades keep it too: the shaped take starts with the hiss after its lead.
        let shaped =
            super::super::render::shape(&c[0].pcm.samples, &RenderSettings::default()).unwrap();
        let lead = &shaped[s(0.020) + s(0.010)..s(0.020) + s(0.08)];
        assert!(
            lead.iter().any(|v| v.abs() > 200),
            "the shaped take keeps the hiss"
        );
        // A line that starts with a vowel is cut as before.
        let (b, a) = fricative_take("Armedsafe");
        let c = cut(&b, &a, 40).unwrap();
        assert!(c[0].from_s >= 3.18, "starts at {}", c[0].from_s);
    }

    #[test]
    fn a_fricative_line_after_a_quiet_gap_stays_in_the_gap() {
        // The carrier take's gap is a low hum, not hiss: the cut stays in it.
        let (mut b, a) = carrier_take();
        b.items[0].0.spoken = "Sattery critical".into();
        let c = cut(&b, &a, 40).unwrap();
        assert!(
            (3.165..=3.21).contains(&c[0].from_s),
            "starts at {}",
            c[0].from_s
        );
    }

    /// Six sentences "The word is <line>." as a long batch speaks them, each from `starts[k]`:
    /// "The word" (450 ms), a 30 ms closure, "is" (150 ms, softer), the line ("eigh" 200 ms, a
    /// 40 ms closure, "ty" 150 ms), then 700 ms of quiet. With `drift`, the timestamps of
    /// sentences 3 and 4 run ahead of the take as ElevenLabs' do: the sentence 600 ms early,
    /// the line 250 ms early (inside "The word"), its end 200 ms early (before the closure).
    fn drifting_take(drift: bool) -> (Batch, Aligned, Vec<f64>) {
        let ms = |t: f64| (t * 32000.0).round() as usize;
        let lines = [
            "eighty",
            "eleven",
            "eighty one",
            "one",
            "eighty eight",
            "eight",
        ];
        let starts: Vec<f64> = (0..lines.len()).map(|k| 0.3 + 1.72 * k as f64).collect();
        let mut seed = 5;
        let mut v = Vec::new();
        for s in &starts {
            v.extend(hiss(ms(*s) - v.len(), 2, &mut seed));
            v.extend(tone(ms(s + 0.45) - v.len(), 150.0, 4000.0));
            v.extend(hiss(ms(s + 0.48) - v.len(), 2, &mut seed));
            v.extend(tone(ms(s + 0.63) - v.len(), 250.0, 1500.0));
            v.extend(tone(ms(s + 0.83) - v.len(), 180.0, 6000.0));
            v.extend(hiss(ms(s + 0.87) - v.len(), 2, &mut seed));
            v.extend(tone(ms(s + 1.02) - v.len(), 300.0, 5000.0));
        }
        let end = starts[lines.len() - 1] + 1.72;
        v.extend(hiss(ms(end) - v.len(), 2, &mut seed));
        let (mut text, mut items) = (String::new(), Vec::new());
        let (mut t_starts, mut t_ends) = (Vec::new(), Vec::new());
        // Each letter of `s` spread evenly from `a` to `b`.
        let mut spread = |s: &str, a: f64, b: f64, text: &mut String| {
            let n = s.chars().count();
            for i in 0..n {
                t_starts.push(a + (b - a) * i as f64 / n as f64);
                t_ends.push(a + (b - a) * (i + 1) as f64 / n as f64);
            }
            text.push_str(s);
        };
        for (k, (line, s)) in lines.iter().zip(&starts).enumerate() {
            let ahead = drift && (k == 3 || k == 4);
            let (t, t0, t1) = if ahead {
                (s - 0.6, s + 0.38, s + 0.82)
            } else {
                (s + 0.0, s + 0.63, s + 1.02)
            };
            if k > 0 {
                spread(" ", starts[k - 1] + 1.12, t, &mut text);
            }
            spread("The word is ", t, t0, &mut text);
            let at = text.chars().count();
            spread(line, t0, t1, &mut text);
            items.push((item(line, "number"), at..at + line.chars().count()));
            spread(".", t1, t1 + 0.1, &mut text);
        }
        (
            Batch {
                tone: "number".into(),
                text: text.clone(),
                items,
            },
            Aligned {
                pcm: Pcm {
                    rate: 32000,
                    samples: v,
                },
                alignment: Alignment {
                    characters: text.chars().map(|c| c.to_string()).collect(),
                    starts: t_starts,
                    ends: t_ends,
                },
                billed_chars: None,
            },
            starts,
        )
    }

    #[test]
    fn a_line_the_timestamps_ran_ahead_of_holds_no_carrier_and_its_whole_end() {
        let (b, a, starts) = drifting_take(true);
        let s = starts[3];
        // The timestamps put the line inside "The word", and its end before the closure.
        let r = &b.items[3].1;
        assert!((a.alignment.starts[r.start] - (s + 0.38)).abs() < 1e-9);
        assert!((a.alignment.ends[r.end - 1] - (s + 0.82)).abs() < 1e-9);
        let c = cut(&b, &a, 40).unwrap();
        for (k, s) in [(3, starts[3]), (4, starts[4])] {
            // At most the snap's 10 ms inset of "is" (150 ms), and all of the line.
            assert!(
                (s + 0.61..=s + 0.63).contains(&c[k].from_s),
                "{k} starts at {} for {}",
                c[k].from_s,
                s + 0.63
            );
            // Past "ty", not at the closure after "eigh"; before the next sentence.
            assert!(
                (s + 1.02..=s + 1.72).contains(&c[k].to_s),
                "{k} ends at {} for {}",
                c[k].to_s,
                s + 1.02
            );
        }
        // Sentences the timestamps fit cut exactly as in a take they fit throughout.
        let (b, fit, _) = drifting_take(false);
        let f = cut(&b, &fit, 40).unwrap();
        for k in [0, 1, 2, 5] {
            assert_eq!((c[k].from_s, c[k].to_s), (f[k].from_s, f[k].to_s), "{k}");
        }
        // Cut on the timestamps alone, nothing moves.
        let raw = cut(&b, &a, 0).unwrap();
        assert!((raw[3].from_s - (s + 0.38)).abs() < 1e-4);
    }

    #[test]
    fn the_pauses_between_sentences_are_found_in_the_take() {
        let (b, a, starts) = drifting_take(true);
        let sentences: Vec<(usize, usize)> = b
            .items
            .iter()
            .enumerate()
            .map(|(k, (_, r))| {
                let first = if k == 0 { 0 } else { b.items[k - 1].1.end + 2 };
                (first, r.end - 1)
            })
            .collect();
        let h = heard(&a.pcm, &a.alignment, &sentences);
        let ms = |t: f64| (t * 1000.0).round() as usize;
        for (k, s) in starts.iter().enumerate() {
            let onset = h[k].onset.expect("an onset");
            let pause = h[k].pause.expect("a pause");
            assert!(onset.abs_diff(ms(*s)) <= 10, "{k}: onset {onset} for {s}");
            assert!(pause.abs_diff(ms(s + 1.02)) <= 10, "{k}: pause {pause}");
        }
        // Without the pauses (one long tone) nothing is found, and nothing moves.
        let flat = Pcm {
            rate: 32000,
            samples: tone(a.pcm.samples.len(), 200.0, 4000.0),
        };
        let h = heard(&flat, &a.alignment, &sentences);
        assert!(h.iter().all(|h| h.onset.is_none() && h.pause.is_none()));
        let m = moved(&a.alignment, &b, &sentences, &h);
        assert!(m.iter().all(|m| *m == Moved::default()));
    }

    fn two_item_batch() -> Batch {
        // "ab cd": a and b span 0.30-0.50, c and d 0.80-1.00.
        Batch {
            tone: "calm".into(),
            text: "ab cd".into(),
            items: vec![(item("ab", "calm"), 0..2), (item("cd", "calm"), 3..5)],
        }
    }

    fn two_item_take() -> Aligned {
        let ch = |s: &str| s.to_string();
        Aligned {
            pcm: burst_take(),
            alignment: Alignment {
                characters: ["a", "b", " ", "c", "d"].map(ch).to_vec(),
                // Early and late by 15 ms: the cuts still have to hold each whole burst.
                starts: vec![0.315, 0.40, 0.50, 0.785, 0.90],
                ends: vec![0.40, 0.485, 0.785, 0.90, 1.015],
            },
            billed_chars: None,
        }
    }

    #[test]
    fn a_cut_holds_the_whole_burst_and_nothing_of_the_next() {
        let c = cut(&two_item_batch(), &two_item_take(), 40).unwrap();
        assert_eq!(c.len(), 2);
        // The first starts in the silence before the burst and ends in the silence after.
        assert!(
            c[0].from_s <= 0.30 && c[0].from_s >= 0.26,
            "{}",
            c[0].from_s
        );
        assert!(c[0].to_s >= 0.50 && c[0].to_s <= 0.525, "{}", c[0].to_s);
        let loud = |c: &Cut| c.pcm.samples.iter().filter(|v| v.abs() > 1000).count();
        assert_eq!(loud(&c[0]), 32000 / 5);
        assert_eq!(loud(&c[1]), 32000 / 5);
        assert!(c[1].pcm.samples.iter().all(|v| *v <= 0));
    }

    #[test]
    fn an_alignment_for_other_text_is_refused() {
        let mut a = two_item_take();
        a.alignment.characters.pop();
        a.alignment.starts.pop();
        a.alignment.ends.pop();
        assert!(cut(&two_item_batch(), &a, 40)
            .unwrap_err()
            .to_string()
            .contains("changed the text"));
        let mut a = two_item_take();
        a.pcm.samples.truncate(8000);
        assert!(cut(&two_item_batch(), &a, 40)
            .unwrap_err()
            .to_string()
            .contains("timestamps run to"));
    }

    fn cut_of(spoken: &str, ms: usize, level: i16) -> Cut {
        Cut {
            spoken: spoken.into(),
            pcm: Pcm {
                rate: 32000,
                samples: vec![level; 32 * ms],
            },
            from_s: 0.0,
            to_s: 0.0,
        }
    }

    #[test]
    fn duration_outliers_are_flagged() {
        let cuts = vec![
            cut_of("battery low", 800, 3000),
            cut_of("signal low", 700, 3000),
            cut_of("armed", 450, 3000),
            cut_of("disarmed", 600, 3000),
            cut_of("failsafe", 2900, 3000), // took in a neighbour
            cut_of("six", 60, 3000),        // clipped
            cut_of("turtle mode", 700, 5),  // silent
        ];
        let w: Vec<String> = check(&cuts, true).iter().map(Flag::to_string).collect();
        assert_eq!(w.len(), 3, "{w:?}");
        assert!(w
            .iter()
            .any(|x| x.contains("failsafe") && x.contains("long for its text")));
        assert!(w.iter().any(|x| x.contains("six") && x.contains("60 ms")));
        assert!(w
            .iter()
            .any(|x| x.contains("turtle mode") && x.contains("silent")));
        assert!(check(&cuts[..4], true).is_empty());
    }

    fn eleven_with(text_step: f64, batch: &Batch) -> (Arc<FakeHttp>, Eleven) {
        let h = Arc::new(FakeHttp::default());
        h.on(
            "with-timestamps",
            200,
            timestamps_json(&batch.text, text_step),
        );
        (
            h.clone(),
            Eleven::new(h, "sk_test_key_00000".into()).without_backoff(),
        )
    }

    #[test]
    fn a_stop_ends_the_run_after_the_current_batch() {
        let dir = tempfile::tempdir().unwrap();
        let bs = BatchSettings {
            max_lines: 1,
            ..Default::default()
        };
        let batches = plan(&[item("Six", "number"), item("Armed", "number")], &bs);
        assert_eq!(batches.len(), 2);
        let h = Arc::new(FakeHttp::default());
        h.synth(0.06);
        let tts = Eleven::new(h.clone(), "sk_test_key_00000".into()).without_backoff();
        let cache = BatchCache::new(dir.path());
        let settings = RenderSettings::default();
        let ctx = Ctx {
            tts: &tts,
            cache: &cache,
            voice: "abc",
            model: "m",
            settings: &settings,
        };
        let mut seen = Vec::new();
        let e = render_all(&ctx, &batches, 40, &priced(&ctx, &batches), &mut |d, t| {
            seen.push((d, t));
            false
        })
        .unwrap_err();
        assert!(e.is::<Stopped>(), "{e:#}");
        assert_eq!(seen, vec![(1, 2)]);
        assert_eq!(h.posts(), 1);
        // The batch made before the stop is in the cache: the next run sends only the other.
        assert!(ctx.cached(&batches[0]));
        let r = render_all(&ctx, &batches, 40, &priced(&ctx, &batches), &mut |_, _| {
            true
        })
        .unwrap();
        assert_eq!((r.batches_rendered, r.batches_cached), (1, 1));
        assert_eq!(h.posts(), 2);
    }

    #[test]
    fn a_batch_renders_once_and_every_recut_comes_from_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let batches = plan(
            &[
                item("Six", "number"),
                item("Armed", "number"),
                item("Failsafe", "number"),
            ],
            &BatchSettings::default(),
        );
        let (http, tts) = eleven_with(0.06, &batches[0]);
        let cache = BatchCache::new(dir.path());
        let settings = RenderSettings::default();
        let ctx = Ctx {
            tts: &tts,
            cache: &cache,
            voice: "abc",
            model: "eleven_turbo_v2_5",
            settings: &settings,
        };
        let price = price(0.5, Some(1000));

        let before = estimate(&ctx, &batches, &price);
        assert_eq!(before.chars, batches[0].text.chars().count() as u64);
        assert_eq!(before.credits, (before.chars as f64 * 0.5).ceil() as u64);
        assert_eq!(
            (before.batches, before.cached_batches, before.lines),
            (1, 0, 3)
        );
        assert!(before.affordable);

        let first = render_all(&ctx, &batches, 40, &priced(&ctx, &batches), &mut |_, _| {
            true
        })
        .unwrap();
        assert_eq!((first.batches_rendered, first.batches_cached), (1, 0));
        assert_eq!(first.cuts.len(), 3);
        assert_eq!(http.posts(), 1);

        // The same batch again: no request, and the estimate is zero.
        let second = render_all(&ctx, &batches, 40, &priced(&ctx, &batches), &mut |_, _| {
            true
        })
        .unwrap();
        assert_eq!((second.batches_rendered, second.batches_cached), (0, 1));
        assert_eq!(http.posts(), 1);
        let after = estimate(&ctx, &batches, &price);
        assert_eq!((after.credits, after.cached_batches), (0, 1));
        assert_eq!(first.cuts["Six"].samples, second.cuts["Six"].samples);

        // Another voice, model or seed is another batch.
        let other = Ctx {
            voice: "def",
            ..ctx
        };
        assert!(!other.cached(&batches[0]));
        let seeded = RenderSettings {
            seed: 9,
            ..RenderSettings::default()
        };
        let other = Ctx {
            settings: &seeded,
            ..ctx
        };
        assert!(!other.cached(&batches[0]));
    }

    #[test]
    fn a_paid_take_that_does_not_cut_is_kept_and_never_sent_again() {
        let dir = tempfile::tempdir().unwrap();
        let batches = plan(&[item("Six", "number")], &BatchSettings::default());
        // The provider changed the text: the timestamps cover other characters.
        let h = Arc::new(FakeHttp::default());
        h.on(
            "with-timestamps",
            200,
            timestamps_json("The word is 6.", 0.06),
        );
        let tts = Eleven::new(h.clone(), "sk_test_key_00000".into()).without_backoff();
        let cache = BatchCache::new(dir.path());
        let settings = RenderSettings::default();
        let ctx = Ctx {
            tts: &tts,
            cache: &cache,
            voice: "abc",
            model: "m",
            settings: &settings,
        };
        let e = render_all(&ctx, &batches, 40, &priced(&ctx, &batches), &mut |_, _| {
            true
        })
        .unwrap_err();
        let msg = format!("{e:#}");
        assert!(msg.contains("changed the text"), "{msg}");
        assert!(msg.contains("\"Six\""), "the error names the line: {msg}");
        // The paid take is kept, but never as a take a line is cut from.
        assert!(!ctx.cached(&batches[0]));
        assert!(cache.failed("elevenlabs", &ctx.key(&batches[0])).is_some());
        // The next estimate prices nothing for it, and a re-run fails from the kept take
        // without sending it again, even when the batch is in `priced`.
        let e = estimate(&ctx, &batches, &price(1.0, None));
        assert_eq!((e.cached_batches, e.chars), (1, 0));
        assert!(priced(&ctx, &batches).is_empty());
        let key = vec![ctx.key(&batches[0])];
        let e = render_all(&ctx, &batches, 40, &key, &mut |_, _| true).unwrap_err();
        assert!(format!("{e:#}").contains("\"Six\""), "{e:#}");
        assert_eq!(h.posts(), 1, "the batch was sent and billed only once");
        // Clearing the voice's takes clears the kept one too.
        assert_eq!(cache.remove_voice("elevenlabs", "abc", "m"), 1);
        assert!(cache.failed("elevenlabs", &ctx.key(&batches[0])).is_none());

        // A bad take already in the cache (from an older QuadCam) is removed.
        let bad: Aligned = Eleven::new(
            {
                let h = Arc::new(FakeHttp::default());
                h.on(
                    "with-timestamps",
                    200,
                    timestamps_json("The word is 6.", 0.06),
                );
                h
            },
            "sk_test_key_00000".into(),
        )
        .render_aligned(&TtsRequest {
            text: "x",
            voice: "abc",
            model: "m",
            speed: 1.0,
            seed: 0,
        })
        .unwrap();
        let key = ctx.key(&batches[0]);
        cache
            .put(
                "elevenlabs",
                &key,
                "abc",
                "m",
                1.0,
                0,
                &batches[0].text,
                &bad,
            )
            .unwrap();
        assert!(ctx.cached(&batches[0]));
        let e = render_all(&ctx, &batches, 40, &priced(&ctx, &batches), &mut |_, _| {
            true
        })
        .unwrap_err();
        assert!(format!("{e:#}").contains("removed"), "{e:#}");
        assert!(!ctx.cached(&batches[0]));
        assert_eq!(
            h.posts(),
            1,
            "a cached take that does not cut is removed, not sent"
        );
    }

    #[test]
    fn a_cached_take_that_cannot_be_read_is_never_sent_unpriced() {
        let dir = tempfile::tempdir().unwrap();
        let batches = plan(&[item("Six", "number")], &BatchSettings::default());
        let (http, tts) = eleven_with(0.06, &batches[0]);
        let cache = BatchCache::new(dir.path());
        let settings = RenderSettings::default();
        let ctx = Ctx {
            tts: &tts,
            cache: &cache,
            voice: "abc",
            model: "m",
            settings: &settings,
        };
        render_all(&ctx, &batches, 40, &priced(&ctx, &batches), &mut |_, _| {
            true
        })
        .unwrap();
        assert_eq!(http.posts(), 1);
        // Both files are there, but the audio is not a WAV.
        let key = ctx.key(&batches[0]);
        std::fs::write(cache.path("elevenlabs", &key, "wav"), b"not a wav").unwrap();
        let e = estimate(&ctx, &batches, &price(1.0, None));
        assert_eq!((e.chars, e.cached_batches), (0, 1));
        // The estimate priced nothing, so nothing is sent: the take is removed instead.
        let p = priced(&ctx, &batches);
        assert!(p.is_empty());
        let e = render_all(&ctx, &batches, 40, &p, &mut |_, _| true).unwrap_err();
        assert!(
            format!("{e:#}").contains("could not be read, so it was removed"),
            "{e:#}"
        );
        assert_eq!(http.posts(), 1);
        // The next estimate prices it, and a render of that estimate sends it.
        let e = estimate(&ctx, &batches, &price(1.0, None));
        assert_eq!(e.cached_batches, 0);
        assert!(e.chars > 0);
        render_all(&ctx, &batches, 40, &priced(&ctx, &batches), &mut |_, _| {
            true
        })
        .unwrap();
        assert_eq!(http.posts(), 2);
    }

    #[test]
    fn a_flagged_cut_stays_out_and_its_batch_stays_cached() {
        let dir = tempfile::tempdir().unwrap();
        // "..." is silence in the fake take: its cut has a peak of 0.
        let batches = plan(
            &[item("Six", "number"), item("...", "number")],
            &BatchSettings::default(),
        );
        let (http, tts) = eleven_with(0.06, &batches[0]);
        let cache = BatchCache::new(dir.path());
        let settings = RenderSettings::default();
        let ctx = Ctx {
            tts: &tts,
            cache: &cache,
            voice: "abc",
            model: "m",
            settings: &settings,
        };
        let r = render_all(&ctx, &batches, 40, &priced(&ctx, &batches), &mut |_, _| {
            true
        })
        .unwrap();
        assert!(r.cuts.contains_key("Six"));
        assert!(!r.cuts.contains_key("..."));
        assert_eq!(r.flagged["..."], "the cut is silent");
        assert_eq!(r.warnings.len(), 1);
        assert!(ctx.cached(&batches[0]));
        assert_eq!(http.posts(), 1);
    }

    #[test]
    fn a_render_the_credits_do_not_cover_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let batches = plan(&[item("Six", "number")], &BatchSettings::default());
        let (_h, tts) = eleven_with(0.06, &batches[0]);
        let cache = BatchCache::new(dir.path());
        let settings = RenderSettings::default();
        let ctx = Ctx {
            tts: &tts,
            cache: &cache,
            voice: "abc",
            model: "m",
            settings: &settings,
        };
        let e = estimate(&ctx, &batches, &price(1.0, Some(5)));
        assert!(!e.affordable);
        let msg = e.check().unwrap_err().to_string();
        assert!(
            msg.contains("needs 16 credits") && msg.contains("has 5"),
            "{msg}"
        );
        assert!(estimate(&ctx, &batches, &price(1.0, None)).check().is_ok());
    }
}
