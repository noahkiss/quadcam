//! Batched carrier-sentence rendering (design 7.4). A lone word sounds wrong from a voice
//! (breathy edges, no sentence around it), so each line is spoken inside a carrier
//! ("The word is six.") and cut out afterwards:
//!
//! 1. `plan` groups lines by tone, wraps each in the carrier and joins up to `max_lines`
//!    sentences into one batch text, remembering where each line sits in it.
//! 2. `fetch` renders a batch with character timestamps, or finds it in the `BatchCache`
//!    (raw audio and alignment, by provider, voice, model, speed, batch text and seed).
//! 3. `cut` takes each line from its first character's start to its last character's end,
//!    each edge moved to the quietest point within +-`snap_ms`. A fresh take is cached only
//!    after it cuts; a cached take that no longer cuts is removed.
//! 4. `check` flags cuts that are silent or whose length is off for their text. A flagged
//!    line stays out of the pack and needs a re-take; its batch stays cached (it was paid).
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
    /// Each cut edge moves to the quietest point within this many milliseconds.
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

/// Raw batch audio and alignment: `<cache>/voice/batch/<provider>/<key>.wav` and `.json`.
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
}

/// One batch, cut: from the cache, or from the provider. A fresh take is cached only when it
/// cuts, so a take the provider changed never stays cached; a cached take that does not cut
/// is removed, and the next render fetches it again. True when the cache answered.
pub fn fetch(ctx: &Ctx<'_>, b: &Batch, snap_ms: u32) -> Result<(Vec<Cut>, bool)> {
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
        );
    }
    let cuts = cut(b, &a, snap_ms).with_context(|| {
        format!(
            "the take of a batch of {} {} lines does not cut, so it was not kept",
            b.items.len(),
            b.tone
        )
    })?;
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

/// The sample nearest `centre` (a time in seconds) with the least sound around it, looking
/// `radius_ms` either way. The sound around a point is its mean absolute level over 2 ms.
pub fn quietest(p: &Pcm, centre_s: f64, radius_ms: u32) -> usize {
    let rate = f64::from(p.rate);
    let c = (centre_s * rate).round().clamp(0.0, p.samples.len() as f64) as usize;
    if radius_ms == 0 || p.samples.is_empty() {
        return c;
    }
    let step = (p.rate / 1000).max(1) as usize;
    let half = (p.rate as usize * 2 / 1000 / 2).max(1);
    let reach = p.rate as usize * radius_ms as usize / 1000;
    let lo = c.saturating_sub(reach);
    let hi = (c + reach).min(p.samples.len());
    let level = |at: usize| -> u64 {
        let a = at.saturating_sub(half);
        let b = (at + half).min(p.samples.len());
        if b <= a {
            return 0;
        }
        p.samples[a..b]
            .iter()
            .map(|v| u64::from(v.unsigned_abs()))
            .sum::<u64>()
            / (b - a) as u64
    };
    let mut best = (level(c), 0usize, c);
    let mut at = lo;
    while at <= hi {
        let l = level(at);
        let d = at.abs_diff(c);
        // Quieter wins; at equal level the point nearer the alignment wins.
        if l < best.0 || (l == best.0 && d < best.1) {
            best = (l, d, at);
        }
        at += step;
    }
    best.2
}

/// Cuts every item of a batch out of its take. Fails when the alignment does not describe
/// the batch text or runs past the audio.
pub fn cut(b: &Batch, a: &Aligned, snap_ms: u32) -> Result<Vec<Cut>> {
    let n = b.text.chars().count();
    let al = &a.alignment;
    if al.characters.len() != n || al.starts.len() != n || al.ends.len() != n {
        bail!(
            "the timestamps cover {} characters but the batch has {n}: the provider changed the text",
            al.characters.len()
        );
    }
    let audio_s = a.pcm.samples.len() as f64 / f64::from(a.pcm.rate);
    let last = al.ends.iter().copied().fold(0.0, f64::max);
    if last > audio_s + 0.25 {
        bail!("the audio is {audio_s:.2} s but the timestamps run to {last:.2} s");
    }
    let mut out = Vec::new();
    for (item, r) in &b.items {
        if r.is_empty() || r.end > n {
            bail!("the line {:?} has no place in the batch", item.spoken);
        }
        let (t0, t1) = (al.starts[r.start], al.ends[r.end - 1]);
        let rate = f64::from(a.pcm.rate);
        let mut s0 = quietest(&a.pcm, t0, snap_ms);
        let mut s1 = quietest(&a.pcm, t1, snap_ms);
        if s1 <= s0 {
            // Snapping closed the gap: fall back to the alignment itself.
            s0 = (t0 * rate) as usize;
            s1 = ((t1 * rate) as usize).min(a.pcm.samples.len());
        }
        if s1 <= s0 {
            bail!("the line {:?} came out empty", item.spoken);
        }
        out.push(Cut {
            spoken: item.spoken.clone(),
            pcm: Pcm {
                rate: a.pcm.rate,
                samples: a.pcm.samples[s0..s1].to_vec(),
            },
            from_s: s0 as f64 / rate,
            to_s: s1 as f64 / rate,
        });
    }
    Ok(out)
}

// ----- checking cuts -----

/// A cut shorter than this is a clip, not a word.
const MIN_MS: f64 = 120.0;
/// A cut longer than this took in the carrier or a neighbour.
const MAX_MS: f64 = 4500.0;
/// A cut whose loudest sample is under this holds no speech.
const MIN_PEAK: i32 = 100;

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
/// short for its letters against the batch's median pace.
pub fn check(cuts: &[Cut]) -> Vec<Flag> {
    let ms = |c: &Cut| c.pcm.samples.len() as f64 * 1000.0 / f64::from(c.pcm.rate);
    let letters = |c: &Cut| {
        c.spoken
            .chars()
            .filter(|x| x.is_alphanumeric())
            .count()
            .max(1) as f64
    };
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
        if ctx.cached(b) {
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

/// Renders (or finds) every batch and cuts it. A flagged cut goes to `flagged`, not `cuts`.
/// `each` hears (done, total) after each batch.
pub fn render_all(
    ctx: &Ctx<'_>,
    batches: &[Batch],
    snap_ms: u32,
    each: &mut dyn FnMut(usize, usize),
) -> Result<Rendered> {
    let mut r = Rendered::default();
    for (n, b) in batches.iter().enumerate() {
        let (cuts, hit) = fetch(ctx, b, snap_ms)?;
        if hit {
            r.batches_cached += 1;
        } else {
            r.batches_rendered += 1;
        }
        for f in check(&cuts) {
            r.warnings.push(f.to_string());
            r.flagged.insert(f.spoken, f.reason);
        }
        for c in cuts {
            if !r.flagged.contains_key(&c.spoken) {
                r.cuts.insert(c.spoken, c.pcm);
            }
        }
        each(n + 1, batches.len());
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gear::voice::eleven::fake::{timestamps_json, FakeHttp};
    use crate::gear::voice::eleven::Eleven;
    use std::sync::Arc;

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
        // An alignment edge 20 ms inside the first burst's end lands in the silence after it.
        let at = quietest(&p, 0.48, 40);
        let t = at as f64 / 32000.0;
        assert!((0.50..=0.52).contains(&t), "{t}");
        // An edge already in silence stays closest to where it was.
        let at = quietest(&p, 0.65, 40);
        assert_eq!(at, (0.65f64 * 32000.0) as usize);
        // No snap: the alignment itself.
        assert_eq!(quietest(&p, 0.48, 0), (0.48f64 * 32000.0) as usize);
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
        let w: Vec<String> = check(&cuts).iter().map(Flag::to_string).collect();
        assert_eq!(w.len(), 3, "{w:?}");
        assert!(w
            .iter()
            .any(|x| x.contains("failsafe") && x.contains("long for its text")));
        assert!(w.iter().any(|x| x.contains("six") && x.contains("60 ms")));
        assert!(w
            .iter()
            .any(|x| x.contains("turtle mode") && x.contains("silent")));
        assert!(check(&cuts[..4]).is_empty());
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

        let first = render_all(&ctx, &batches, 40, &mut |_, _| {}).unwrap();
        assert_eq!((first.batches_rendered, first.batches_cached), (1, 0));
        assert_eq!(first.cuts.len(), 3);
        assert_eq!(http.posts(), 1);

        // The same batch again: no request, and the estimate is zero.
        let second = render_all(&ctx, &batches, 40, &mut |_, _| {}).unwrap();
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
    fn a_take_that_does_not_cut_is_never_cached() {
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
        let e = render_all(&ctx, &batches, 40, &mut |_, _| {}).unwrap_err();
        assert!(format!("{e:#}").contains("changed the text"), "{e:#}");
        assert!(!ctx.cached(&batches[0]));
        assert_eq!(
            estimate(&ctx, &batches, &price(1.0, None)).cached_batches,
            0
        );
        // A re-run asks the provider again instead of failing from the cache.
        assert!(render_all(&ctx, &batches, 40, &mut |_, _| {}).is_err());
        assert_eq!(h.posts(), 2);

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
        let e = render_all(&ctx, &batches, 40, &mut |_, _| {}).unwrap_err();
        assert!(format!("{e:#}").contains("removed"), "{e:#}");
        assert!(!ctx.cached(&batches[0]));
        assert_eq!(h.posts(), 2);
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
        let r = render_all(&ctx, &batches, 40, &mut |_, _| {}).unwrap();
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
