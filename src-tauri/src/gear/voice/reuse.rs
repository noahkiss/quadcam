//! Line reuse (design 7.4): a line a voice has spoken once is never sent again. The batch
//! cache keys a take by its whole batch text, so the same line in another set, or in a set
//! added later, would sit in a batch of other text and be billed again. `LineIndex`
//! remembers which sentences each cached take holds, per provider, voice, model and speed.
//! `split` takes a line out of the plan when a cached take of the same sentence exists, and
//! `cut_reused` cuts it from that take without calling the provider.
//!
//! A take whose cut failed its checks is reused only with the seed that made it: that is
//! the same take, so the line stays a re-take and nothing is billed. Another seed sends it
//! again. A take is always cut again when it is used, so a new `batch::CUT_VERSION` reaches
//! every reused line; a flag from an older cut version does not count until then. The index is a cache of the cache: a take it names that the cache lost is skipped,
//! and a lost index only means lines are priced and sent again.

use super::batch::{self, Batch, BatchCache, BatchSettings, Item, Rendered};
use super::render::RenderSettings;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Whose takes: the provider, voice, model and render settings (speed and seed).
pub struct Scope<'a> {
    pub cache: &'a BatchCache,
    pub provider: &'a str,
    pub voice: &'a str,
    pub model: &'a str,
    pub settings: &'a RenderSettings,
}

impl<'a> Scope<'a> {
    pub fn of(ctx: &batch::Ctx<'a>) -> Self {
        Scope {
            cache: ctx.cache,
            provider: ctx.tts.id(),
            voice: ctx.voice,
            model: ctx.model,
            settings: ctx.settings,
        }
    }

    /// The cache key of a batch with these settings (`batch::Ctx::key`).
    pub fn key(&self, b: &Batch) -> String {
        BatchCache::key(
            self.provider,
            self.voice,
            self.model,
            self.settings.speed,
            &b.text,
            self.settings.seed,
        )
    }

    pub fn cached(&self, b: &Batch) -> bool {
        self.cache.has(self.provider, &self.key(b))
    }

    /// Whether a render would send this batch: neither cached nor kept as a paid take that
    /// did not cut (`batch::Ctx::to_send`).
    pub fn to_send(&self, b: &Batch) -> bool {
        !self.cached(b) && self.cache.failed(self.provider, &self.key(b)).is_none()
    }

    /// The cache key of one line spoken alone (`BatchCache::alone_key`).
    pub fn alone_key(&self, text: &str) -> String {
        BatchCache::alone_key(
            self.provider,
            self.voice,
            self.model,
            self.settings.speed,
            text,
            self.settings.seed,
        )
    }

    #[cfg(test)]
    fn of_parts(s: &Scope<'a>) -> Self {
        Scope {
            cache: s.cache,
            provider: s.provider,
            voice: s.voice,
            model: s.model,
            settings: s.settings,
        }
    }

    fn path(&self) -> PathBuf {
        let who = serde_json::json!({
            "voice": self.voice, "model": self.model, "speed": format!("{:.4}", self.settings.speed),
        });
        let provider: String = self
            .provider
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        self.cache
            .cache_dir()
            .join("voice")
            .join("lines")
            .join(provider)
            .join(format!(
                "{}.json",
                &super::sha256_hex(who.to_string().as_bytes())[..16]
            ))
    }
}

/// One cached take as the index knows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// The batch cache key.
    pub key: String,
    pub seed: u64,
    pub tone: String,
    pub text: String,
    pub items: Vec<EntryItem>,
    /// The spoken texts whose cut failed its checks.
    #[serde(default)]
    pub flagged: Vec<String>,
    /// The `batch::CUT_VERSION` that cut and checked it. `flagged` holds for that version
    /// only: under another, the take is cut and checked again when it is used.
    #[serde(default)]
    pub cut: u32,
    /// A take of one line spoken alone, with no carrier (`import_takes`): the whole take is
    /// the cut, and it matches a line by its spoken text.
    #[serde(default)]
    pub alone: bool,
    /// What the speech check heard in its lines (`asr`).
    #[serde(default)]
    pub heard: Vec<EntryHeard>,
    /// The spoken texts the person flagged in this take (`gear_voice_flag`): each needs a
    /// re-take, whatever the cut version, until the flag is cleared.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected: Vec<String>,
}

/// What the speech check heard in one line of a take.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryHeard {
    pub spoken: String,
    pub heard: String,
    /// The heard text matched the line.
    pub ok: bool,
    /// The person kept the line although the check heard something else.
    #[serde(default)]
    pub accepted: bool,
    /// The `batch::CUT_VERSION` of the cut it heard; under another, the result no longer holds.
    pub cut: u32,
}

/// The re-take reason of a line the person flagged.
pub const REJECTED: &str = "flagged for a re-take";

/// The index key of a line spoken alone.
fn alone_key(spoken: &str) -> String {
    format!("\u{1}{}", spoken.to_lowercase())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryItem {
    pub spoken: String,
    /// The carrier sentence it was spoken in.
    pub sentence: String,
    /// Its characters in the batch text.
    pub start: usize,
    pub end: usize,
}

impl Entry {
    /// A batch just made (or found in the cache), with the lines its checks flagged.
    pub fn new(s: &Scope, b: &Batch, bs: &BatchSettings, flagged: &HashSet<&str>) -> Self {
        Entry {
            key: s.key(b),
            seed: s.settings.seed,
            tone: b.tone.clone(),
            text: b.text.clone(),
            items: b
                .items
                .iter()
                .map(|(i, r)| EntryItem {
                    spoken: i.spoken.clone(),
                    sentence: batch::sentence_of(bs, i),
                    start: r.start,
                    end: r.end,
                })
                .collect(),
            flagged: b
                .items
                .iter()
                .filter(|(i, _)| flagged.contains(i.spoken.as_str()))
                .map(|(i, _)| i.spoken.clone())
                .collect(),
            cut: batch::CUT_VERSION,
            alone: false,
            heard: Vec::new(),
            rejected: Vec::new(),
        }
    }

    /// The speech check's mismatch for a line of this take, unless the person accepted it or
    /// another cut version made it.
    pub fn misheard(&self, spoken: &str) -> Option<&EntryHeard> {
        self.heard
            .iter()
            .find(|h| h.cut == batch::CUT_VERSION && h.spoken == spoken && !h.ok && !h.accepted)
    }

    /// A line of this take needs a re-take: its cut failed a check, the speech check heard
    /// something else, or the person flagged it.
    pub fn needs_retake(&self, spoken: &str) -> bool {
        (self.cut == batch::CUT_VERSION && self.flagged.iter().any(|f| f == spoken))
            || self.misheard(spoken).is_some()
            || self.is_rejected(spoken)
    }

    /// The person flagged this line of the take.
    pub fn is_rejected(&self, spoken: &str) -> bool {
        self.rejected.iter().any(|r| r.eq_ignore_ascii_case(spoken))
    }

    /// Records what the speech check heard in one of its lines, replacing an earlier result.
    pub fn set_heard(&mut self, h: EntryHeard) {
        self.heard.retain(|x| x.spoken != h.spoken);
        self.heard.push(h);
    }

    /// The batch to cut this take with.
    pub fn batch(&self) -> Batch {
        Batch {
            tone: self.tone.clone(),
            text: self.text.clone(),
            items: self
                .items
                .iter()
                .map(|i| {
                    (
                        Item {
                            spoken: i.spoken.clone(),
                            tone: self.tone.clone(),
                        },
                        i.start..i.end,
                    )
                })
                .collect(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct IndexFile {
    /// Whose takes these are, for `rendered_counts`.
    #[serde(default)]
    voice: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    speed: f64,
    #[serde(default)]
    batches: Vec<Entry>,
}

/// How many lines one voice, model and speed have a take of.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct TakeCount {
    pub voice: String,
    pub model: String,
    pub speed: f64,
    /// Distinct spoken texts (ignoring case) with a take whose cut passed its checks.
    pub lines: u32,
}

/// The lines each voice, model and speed have a take of, from every line index of the
/// provider, most first. Takes the cache lost do not count.
pub fn rendered_counts(cache: &BatchCache, provider: &str) -> Vec<TakeCount> {
    let dir = cache.cache_dir().join("voice").join("lines").join(provider);
    let mut out = Vec::new();
    for f in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = f.path();
        if p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let Some(file) = std::fs::read(&p)
            .ok()
            .and_then(|b| serde_json::from_slice::<IndexFile>(&b).ok())
        else {
            continue;
        };
        if file.voice.is_empty() {
            continue;
        }
        let mut said: HashSet<String> = HashSet::new();
        for e in &file.batches {
            if !cache.has(provider, &e.key) {
                continue;
            }
            for i in &e.items {
                if !e.needs_retake(&i.spoken) {
                    said.insert(i.spoken.to_lowercase());
                }
            }
        }
        if !said.is_empty() {
            out.push(TakeCount {
                voice: file.voice,
                model: file.model,
                speed: file.speed,
                lines: said.len() as u32,
            });
        }
    }
    out.sort_by(|a, b| b.lines.cmp(&a.lines).then(a.voice.cmp(&b.voice)));
    out
}

/// The takes of one provider, voice, model and speed, by sentence.
#[derive(Debug, Default)]
pub struct LineIndex {
    batches: Vec<Entry>,
    by_sentence: HashMap<String, Vec<usize>>,
}

impl LineIndex {
    fn read(path: &std::path::Path) -> Vec<Entry> {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<IndexFile>(&b).ok())
            .map(|f| f.batches)
            .unwrap_or_default()
    }

    pub fn load(s: &Scope) -> Self {
        let batches = Self::read(&s.path());
        let mut by_sentence: HashMap<String, Vec<usize>> = HashMap::new();
        for (n, e) in batches.iter().enumerate() {
            for i in &e.items {
                let k = if e.alone {
                    alone_key(&i.spoken)
                } else {
                    i.sentence.to_lowercase()
                };
                by_sentence.entry(k).or_default().push(n);
            }
        }
        LineIndex {
            batches,
            by_sentence,
        }
    }

    /// Adds or replaces these takes, and drops takes the cache no longer holds (with no
    /// entries, only that). Holds a lock on the index while it reads and writes it.
    pub fn record(s: &Scope, entries: Vec<Entry>) -> Result<()> {
        let path = s.path();
        if entries.is_empty() && !path.is_file() {
            return Ok(());
        }
        let dir = path.parent().context("no index folder")?;
        std::fs::create_dir_all(dir)?;
        let lock = std::fs::File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path.with_extension("lock"))
            .context("opening the line index lock")?;
        lock.lock().context("locking the line index")?;
        let new: HashSet<String> = entries.iter().map(|e| e.key.clone()).collect();
        let old = Self::read(&path);
        // The same key is the same take: what the speech check heard in it still holds.
        let entries = entries.into_iter().map(|mut e| {
            if e.heard.is_empty() {
                if let Some(o) = old.iter().find(|o| o.key == e.key) {
                    e.heard = o.heard.clone();
                }
            }
            e
        });
        let mut all: Vec<Entry> = old
            .iter()
            .filter(|e| !new.contains(&e.key) && s.cache.has(s.provider, &e.key))
            .cloned()
            .collect();
        all.extend(entries);
        let tmp = path.with_extension("tmp");
        let file = IndexFile {
            voice: s.voice.into(),
            model: s.model.into(),
            speed: s.settings.speed,
            batches: all,
        };
        std::fs::write(&tmp, serde_json::to_vec(&file)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// The take a render picks for a line spoken as `spoken` with this seed, as `split` ranks
    /// them: one that passes with this seed, one that passes with any seed, one that needs a
    /// re-take with this seed; the newest wins a tie. Its index in `entries`.
    pub fn take_of(&self, s: &Scope, spoken: &str) -> Option<usize> {
        let want = spoken.to_lowercase();
        let mut best: Option<(u8, usize)> = None;
        for (n, e) in self.batches.iter().enumerate() {
            let Some(it) = e.items.iter().find(|i| i.spoken.to_lowercase() == want) else {
                continue;
            };
            let rank = match (e.needs_retake(&it.spoken), e.seed == s.settings.seed) {
                (false, true) => 0,
                (false, false) => 1,
                (true, true) => 2,
                (true, false) => continue,
            };
            if best.is_some_and(|(r, _)| r < rank) || !s.cache.has(s.provider, &e.key) {
                continue;
            }
            best = Some((rank, n));
        }
        best.map(|(_, n)| n)
    }

    /// The takes, as `take_of` numbers them.
    pub fn entries(&self) -> &[Entry] {
        &self.batches
    }
}

/// A line taken from an earlier take: the text the take spoke, and the line's own text
/// (the same, or the same but for case).
#[derive(Debug, Clone)]
pub struct Wanted {
    pub take: String,
    pub line: String,
}

/// The lines a plan takes from earlier takes: each take with the lines wanted from it.
#[derive(Debug, Default)]
pub struct Reuse {
    pub takes: Vec<(Entry, Vec<Wanted>)>,
}

impl Reuse {
    /// Every line's spoken text reused, with whether its take flagged it.
    pub fn lines(&self) -> impl Iterator<Item = (&str, bool)> {
        self.takes.iter().flat_map(|(e, want)| {
            want.iter()
                .map(move |w| (w.line.as_str(), e.needs_retake(&w.take)))
        })
    }

    pub fn is_empty(&self) -> bool {
        self.takes.is_empty()
    }
}

/// Splits `items` into the lines a cached take already holds and the rest, which still go to
/// the provider, in order. A take holds a line when it holds the same sentence, ignoring case
/// ("Six" and "six" sound alike). A repeated spoken text counts once (the first wins, as in
/// `batch::plan`). A take spoken alone (imported) holds a line with the same spoken text. A
/// take is picked in this order: one that passed its checks with this seed, one that passed
/// with any seed, one that failed with this seed; a carrier take before a lone one.
pub fn split(
    s: &Scope,
    index: &LineIndex,
    items: &[Item],
    bs: &BatchSettings,
) -> (Vec<Item>, Reuse) {
    let mut fresh = Vec::new();
    let mut reuse = Reuse::default();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut held: HashMap<usize, bool> = HashMap::new();
    let mut slot: HashMap<usize, usize> = HashMap::new();
    for i in items {
        if !seen.insert(i.spoken.as_str()) {
            continue;
        }
        let sentence = batch::sentence_of(bs, i).to_lowercase();
        let alone = alone_key(&i.spoken);
        let mut best: Option<(u8, usize, &str)> = None;
        let found = [&sentence, &alone]
            .into_iter()
            .flat_map(|k| index.by_sentence.get(k).into_iter().flatten());
        for &n in found {
            let e = &index.batches[n];
            let Some(it) = e.items.iter().find(|x| {
                if e.alone {
                    x.spoken.to_lowercase() == i.spoken.to_lowercase()
                } else {
                    x.sentence.to_lowercase() == sentence
                }
            }) else {
                continue;
            };
            // Under another cut version the checks are not known yet: the take is cut again.
            let flagged = e.needs_retake(&it.spoken);
            let same_seed = e.seed == s.settings.seed;
            let rank = match (flagged, same_seed) {
                (false, true) => 0,
                (false, false) => 1,
                (true, true) => 2,
                (true, false) => continue,
            };
            // A take in a carrier wins over one spoken alone.
            let rank = rank * 2 + u8::from(e.alone);
            // The newest take wins a tie.
            if best.is_some_and(|(r, _, _)| r < rank) {
                continue;
            }
            if !*held
                .entry(n)
                .or_insert_with(|| s.cache.has(s.provider, &e.key))
            {
                continue;
            }
            best = Some((rank, n, &it.spoken));
        }
        match best {
            Some((_, n, take)) => {
                let at = *slot.entry(n).or_insert_with(|| {
                    reuse.takes.push((index.batches[n].clone(), Vec::new()));
                    reuse.takes.len() - 1
                });
                reuse.takes[at].1.push(Wanted {
                    take: take.to_string(),
                    line: i.spoken.clone(),
                });
            }
            None => fresh.push(i.clone()),
        }
    }
    (fresh, reuse)
}

/// Cuts the reused lines out of their cached takes into `done`, as `batch::render_all` does
/// for a batch: a flagged cut goes to `flagged`. Never calls the provider. Returns each take
/// with the lines its checks flag now, for `LineIndex::record`. A take that cannot be read
/// or does not cut is removed from the cache, as `batch::fetch` removes one, so the next
/// estimate prices its lines; every take is tried before the first such error returns.
pub fn cut_reused(
    s: &Scope,
    reuse: &Reuse,
    snap_ms: u32,
    done: &mut Rendered,
) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    let mut failed: Vec<anyhow::Error> = Vec::new();
    for (e, want) in &reuse.takes {
        let b = e.batch();
        let cuts = match cut_take(s, e, &b, snap_ms) {
            Ok(c) => c,
            Err(err) => {
                s.cache.remove(s.provider, &e.key);
                failed.push(err.context(format!(
                    "the cached take of {} {} lines could not be reused, so it was removed; a new estimate prices it again",
                    b.items.len(),
                    b.tone
                )));
                continue;
            }
        };
        let flags = batch::check(&cuts, !e.alone);
        for c in &cuts {
            for w in want.iter().filter(|w| w.take == c.spoken) {
                let reason = flags
                    .iter()
                    .find(|f| f.spoken == c.spoken)
                    .map(|f| f.reason.clone())
                    .or_else(|| e.misheard(&c.spoken).map(|h| super::asr::reason(&h.heard)))
                    .or_else(|| e.is_rejected(&c.spoken).then(|| REJECTED.to_string()));
                match reason {
                    Some(reason) => {
                        done.warnings.push(format!("{:?}: {reason}", w.line));
                        done.flagged.insert(w.line.clone(), reason);
                    }
                    None => {
                        done.cuts.insert(w.line.clone(), c.pcm.clone());
                    }
                }
            }
        }
        done.batches_cached += 1;
        out.push(Entry {
            flagged: flags.into_iter().map(|f| f.spoken).collect(),
            cut: batch::CUT_VERSION,
            ..e.clone()
        });
    }
    let n = failed.len();
    if let Some(first) = failed.into_iter().next() {
        return Err(if n > 1 {
            first.context(format!(
                "{n} cached takes could not be reused and were removed"
            ))
        } else {
            first
        });
    }
    Ok(out)
}

/// One cached take, read and cut.
fn cut_take(s: &Scope, e: &Entry, b: &Batch, snap_ms: u32) -> Result<Vec<batch::Cut>> {
    let a = s
        .cache
        .get(s.provider, &e.key)
        .context("its files are missing or unreadable")?;
    if !e.alone {
        return batch::cut(b, &a, snap_ms);
    }
    // The whole take is the line.
    let secs = a.pcm.samples.len() as f64 / f64::from(a.pcm.rate.max(1));
    Ok(b.items
        .iter()
        .map(|(i, _)| batch::Cut {
            spoken: i.spoken.clone(),
            pcm: a.pcm.clone(),
            from_s: 0.0,
            to_s: secs,
        })
        .collect())
}

/// The index entries of batches `render_all` made or found, with the lines it flagged.
pub fn entries(s: &Scope, batches: &[Batch], bs: &BatchSettings, done: &Rendered) -> Vec<Entry> {
    let flagged: HashSet<&str> = done.flagged.keys().map(String::as_str).collect();
    batches
        .iter()
        .filter(|b| s.cached(b))
        .map(|b| Entry::new(s, b, bs, &flagged))
        .collect()
}

/// What `import_takes` found and did.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct ImportReport {
    /// Takes found: a `.json` beside a `.pcm` of the same name.
    pub found: u32,
    /// Takes copied into the cache (with `dry_run`, that would be).
    pub imported: u32,
    /// Takes the cache held already.
    pub already: u32,
    /// Files left out, each with why.
    pub skipped: Vec<String>,
    /// The lines each voice, model and speed have a take of afterwards.
    pub counts: Vec<TakeCount>,
    pub dry_run: bool,
}

/// One take's sidecar: what an earlier line-by-line render wrote beside its PCM.
#[derive(Debug, Deserialize)]
struct TakeFile {
    text: String,
    voice: String,
    model: String,
    #[serde(default = "one")]
    speed: f64,
    #[serde(default)]
    seed: u64,
    /// The PCM's sample rate; 32000 when the file does not say.
    #[serde(default, alias = "sample_rate")]
    rate: Option<u32>,
}

fn one() -> f64 {
    1.0
}

/// The PCM a take may be: 16-bit mono, at most 10 minutes at 48 kHz.
const MAX_PCM: u64 = 48_000 * 2 * 600;

fn walk(dir: &std::path::Path, depth: u32, out: &mut Vec<PathBuf>) {
    if depth > 6 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(t) = e.file_type() else { continue };
        if t.is_dir() {
            walk(&p, depth + 1, out);
        } else if t.is_file() && p.extension().is_some_and(|x| x == "json") {
            out.push(p);
        }
    }
}

/// Copies takes an earlier line-by-line render left in `folder` into the batch cache, each as
/// one line spoken alone, and records them in the line index. A take is a `.json` (text,
/// voice, model, speed, seed, and the sample rate when not 32000) beside a `.pcm` of the same
/// name (16-bit little-endian mono). The folder is only read. `dry_run` writes nothing.
pub fn import_takes(
    cache: &BatchCache,
    provider: &str,
    folder: &std::path::Path,
    dry_run: bool,
) -> Result<ImportReport> {
    if !folder.is_dir() {
        anyhow::bail!("{} is not a folder", folder.display());
    }
    let mut files = Vec::new();
    walk(folder, 0, &mut files);
    files.sort();
    let mut r = ImportReport {
        dry_run,
        ..Default::default()
    };
    // Entries per voice, model and speed.
    let mut by: HashMap<(String, String, String), (f64, Vec<Entry>)> = HashMap::new();
    for json in files {
        let pcm_path = json.with_extension("pcm");
        let name = json
            .strip_prefix(folder)
            .unwrap_or(&json)
            .display()
            .to_string();
        if !pcm_path.is_file() {
            continue;
        }
        r.found += 1;
        let t: TakeFile = match std::fs::read(&json)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
        {
            Some(t) => t,
            None => {
                r.skipped
                    .push(format!("{name}: not a take's JSON (text, voice, model)"));
                continue;
            }
        };
        let text = t.text.trim().to_string();
        let ok_id = |v: &str| {
            !v.is_empty()
                && v.len() <= 100
                && v.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
        };
        if text.is_empty() || !ok_id(&t.voice) || !ok_id(&t.model) {
            r.skipped.push(format!(
                "{name}: no text, or a voice or model id that is not one"
            ));
            continue;
        }
        if !(0.5..=2.0).contains(&t.speed) {
            r.skipped
                .push(format!("{name}: speed {} is outside 0.5 to 2", t.speed));
            continue;
        }
        let rate = t.rate.unwrap_or(32_000);
        if !(8_000..=48_000).contains(&rate) {
            r.skipped.push(format!(
                "{name}: sample rate {rate} is outside 8000 to 48000"
            ));
            continue;
        }
        let len = std::fs::metadata(&pcm_path).map(|m| m.len()).unwrap_or(0);
        if len == 0 || len % 2 != 0 || len > MAX_PCM {
            r.skipped
                .push(format!("{name}: the PCM is empty, odd-sized or too long"));
            continue;
        }
        let settings = RenderSettings {
            speed: t.speed,
            seed: t.seed,
            ..Default::default()
        };
        let s = Scope {
            cache,
            provider,
            voice: &t.voice,
            model: &t.model,
            settings: &settings,
        };
        let n = text.chars().count();
        let b = Batch {
            tone: "calm".into(),
            text: text.clone(),
            items: vec![(
                Item {
                    spoken: text.clone(),
                    tone: "calm".into(),
                },
                0..n,
            )],
        };
        let key = s.alone_key(&text);
        // 0.14.0 keyed an import as a batch of its text, which a batch of one line under the
        // carrier `{line}` shares. Such a take (no alignment) moves to the new key here.
        let legacy = s.key(&b);
        let legacy = cache
            .get(provider, &legacy)
            .filter(|a| a.alignment.characters.is_empty())
            .map(|a| (legacy, a));
        if cache.has(provider, &key) {
            r.already += 1;
        } else if let Some((_, a)) = &legacy {
            r.already += 1;
            if !dry_run {
                cache.put(
                    provider, &key, &t.voice, &t.model, t.speed, t.seed, &text, a,
                )?;
            }
        } else {
            r.imported += 1;
            if !dry_run {
                let bytes = std::fs::read(&pcm_path)?;
                let pcm = super::wav::Pcm {
                    rate,
                    samples: bytes
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|c| i16::from_le_bytes(*c))
                        .collect(),
                };
                let a = super::tts::Aligned {
                    pcm,
                    alignment: Default::default(),
                    billed_chars: None,
                };
                cache.put(
                    provider, &key, &t.voice, &t.model, t.speed, t.seed, &text, &a,
                )?;
            }
        }
        if let (Some((old, _)), false) = (&legacy, dry_run) {
            cache.remove(provider, old);
        }
        let entry = Entry {
            key,
            seed: t.seed,
            tone: "calm".into(),
            text: text.clone(),
            items: vec![EntryItem {
                spoken: text,
                sentence: String::new(),
                start: 0,
                end: n,
            }],
            flagged: Vec::new(),
            // Not checked yet: the first render that uses it cuts and checks it.
            cut: 0,
            alone: true,
            rejected: Vec::new(),
            heard: Vec::new(),
        };
        by.entry((t.voice.clone(), t.model.clone(), format!("{:.4}", t.speed)))
            .or_insert_with(|| (t.speed, Vec::new()))
            .1
            .push(entry);
    }
    if !dry_run {
        for ((voice, model, _), (speed, entries)) in by {
            let settings = RenderSettings {
                speed,
                ..Default::default()
            };
            let s = Scope {
                cache,
                provider,
                voice: &voice,
                model: &model,
                settings: &settings,
            };
            LineIndex::record(&s, entries)?;
        }
    }
    r.counts = rendered_counts(cache, provider);
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gear::voice::tts::Aligned;
    use crate::gear::voice::wav::Pcm;

    fn take(s: &Scope, text: &str) -> Entry {
        let b = batch::plan(
            &[Item {
                spoken: text.into(),
                tone: "calm".into(),
            }],
            &BatchSettings::default(),
        )
        .remove(0);
        let a = Aligned {
            pcm: Pcm {
                rate: 32_000,
                samples: vec![1000; 3200],
            },
            alignment: Default::default(),
            billed_chars: None,
        };
        let e = Entry::new(s, &b, &BatchSettings::default(), &HashSet::new());
        s.cache
            .put(s.provider, &e.key, s.voice, s.model, 1.0, 0, &b.text, &a)
            .unwrap();
        e
    }

    fn keys(s: &Scope) -> Vec<String> {
        LineIndex::load(s)
            .batches
            .into_iter()
            .map(|e| e.key)
            .collect()
    }

    #[test]
    fn the_index_drops_takes_the_cache_lost() {
        let dir = tempfile::tempdir().unwrap();
        let cache = BatchCache::new(dir.path());
        let settings = RenderSettings::default();
        let s = Scope {
            cache: &cache,
            provider: "elevenlabs",
            voice: "v",
            model: "m",
            settings: &settings,
        };
        // Nothing recorded and nothing to drop: no index file.
        LineIndex::record(&s, Vec::new()).unwrap();
        assert!(!s.path().exists());
        let (a, b, c) = (take(&s, "Armed"), take(&s, "Disarmed"), take(&s, "Six"));
        LineIndex::record(&s, vec![a.clone(), b.clone()]).unwrap();
        assert_eq!(keys(&s), [a.key.clone(), b.key.clone()]);
        // A take the cache lost is dropped at the next write.
        cache.remove("elevenlabs", &a.key);
        LineIndex::record(&s, vec![c.clone()]).unwrap();
        assert_eq!(keys(&s), [b.key.clone(), c.key.clone()]);
        // With nothing to add, a write only drops.
        cache.remove("elevenlabs", &c.key);
        LineIndex::record(&s, Vec::new()).unwrap();
        assert_eq!(keys(&s), [b.key]);
    }

    #[test]
    fn a_line_the_speech_check_misheard_is_a_retake_until_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let cache = BatchCache::new(dir.path());
        let settings = RenderSettings::default();
        let s = Scope {
            cache: &cache,
            provider: "elevenlabs",
            voice: "v",
            model: "m",
            settings: &settings,
        };
        let bs = BatchSettings::default();
        let item = |t: &str| Item {
            spoken: t.into(),
            tone: "calm".into(),
        };
        // A take of the line alone, a second of tone: the whole take is its cut.
        let mut e = Entry {
            alone: true,
            ..take(&s, "Armed")
        };
        let tone = Aligned {
            pcm: Pcm {
                rate: 32_000,
                samples: (0..32_000)
                    .map(|i| ((i as f32 * 0.07).sin() * 8000.0) as i16)
                    .collect(),
            },
            alignment: Default::default(),
            billed_chars: None,
        };
        cache
            .put("elevenlabs", &e.key, "v", "m", 1.0, 0, &e.text, &tone)
            .unwrap();
        let misheard = EntryHeard {
            spoken: "Armed".into(),
            heard: "Unarmed.".into(),
            ok: false,
            accepted: false,
            cut: batch::CUT_VERSION,
        };
        e.set_heard(misheard.clone());
        LineIndex::record(&s, vec![e.clone()]).unwrap();
        // The same take written again without a result keeps what the check heard.
        LineIndex::record(
            &s,
            vec![Entry {
                heard: Vec::new(),
                ..e.clone()
            }],
        )
        .unwrap();
        let index = LineIndex::load(&s);
        assert_eq!(index.entries()[0].heard, std::slice::from_ref(&misheard));
        assert!(index.entries()[0].needs_retake("Armed"));
        assert_eq!(index.take_of(&s, "armed"), Some(0));
        // With this seed: a re-take from the same take, nothing sent.
        let (fresh, reuse) = split(&s, &index, &[item("Armed")], &bs);
        assert!(fresh.is_empty());
        assert_eq!(reuse.lines().collect::<Vec<_>>(), [("Armed", true)]);
        // Cut again, the line stays out with the check's reason.
        let mut done = Rendered::default();
        cut_reused(&s, &reuse, bs.snap_ms, &mut done).unwrap();
        assert_eq!(
            done.flagged.get("Armed").map(String::as_str),
            Some("The speech check heard \"Unarmed.\".")
        );
        // Another seed sends the line again.
        let other = RenderSettings {
            seed: 7,
            ..RenderSettings::default()
        };
        let s7 = Scope {
            settings: &other,
            ..Scope::of_parts(&s)
        };
        let (fresh, reuse) = split(&s7, &LineIndex::load(&s7), &[item("Armed")], &bs);
        assert_eq!(fresh.len(), 1);
        assert!(reuse.is_empty());
        // Accepted, the take passes again.
        e.set_heard(EntryHeard {
            accepted: true,
            ..misheard.clone()
        });
        LineIndex::record(&s, vec![e]).unwrap();
        let index = LineIndex::load(&s);
        assert!(!index.entries()[0].needs_retake("Armed"));
        let (_, reuse) = split(&s, &index, &[item("Armed")], &bs);
        assert_eq!(reuse.lines().collect::<Vec<_>>(), [("Armed", false)]);
        // A result from another cut version does not count.
        let old = Entry {
            heard: vec![EntryHeard {
                cut: batch::CUT_VERSION - 1,
                ..misheard
            }],
            ..index.entries()[0].clone()
        };
        assert!(!old.needs_retake("Armed"));
    }
}
