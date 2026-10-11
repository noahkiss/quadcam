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
        }
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
    #[serde(default)]
    batches: Vec<Entry>,
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
                by_sentence
                    .entry(i.sentence.to_lowercase())
                    .or_default()
                    .push(n);
            }
        }
        LineIndex {
            batches,
            by_sentence,
        }
    }

    /// Adds or replaces these takes, and drops takes the cache no longer holds. Holds a lock
    /// on the index while it reads and writes it.
    pub fn record(s: &Scope, entries: Vec<Entry>) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let path = s.path();
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
        let mut all: Vec<Entry> = Self::read(&path)
            .into_iter()
            .filter(|e| !new.contains(&e.key) && s.cache.has(s.provider, &e.key))
            .collect();
        all.extend(entries);
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec(&IndexFile { batches: all })?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
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
            want.iter().map(move |w| {
                (
                    w.line.as_str(),
                    e.cut == batch::CUT_VERSION && e.flagged.contains(&w.take),
                )
            })
        })
    }

    pub fn is_empty(&self) -> bool {
        self.takes.is_empty()
    }
}

/// Splits `items` into the lines a cached take already holds and the rest, which still go to
/// the provider, in order. A take holds a line when it holds the same sentence, ignoring case
/// ("Six" and "six" sound alike). A repeated spoken text counts once (the first wins, as in
/// `batch::plan`). A take is picked in this order: one that passed its checks with this seed,
/// one that passed with any seed, one that failed with this seed.
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
        let mut best: Option<(u8, usize, &str)> = None;
        for &n in index.by_sentence.get(&sentence).into_iter().flatten() {
            let e = &index.batches[n];
            let Some(it) = e
                .items
                .iter()
                .find(|x| x.sentence.to_lowercase() == sentence)
            else {
                continue;
            };
            // Under another cut version the checks are not known yet: the take is cut again.
            let flagged = e.cut == batch::CUT_VERSION && e.flagged.contains(&it.spoken);
            let same_seed = e.seed == s.settings.seed;
            let rank = match (flagged, same_seed) {
                (false, true) => 0,
                (false, false) => 1,
                (true, true) => 2,
                (true, false) => continue,
            };
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
/// with the lines its checks flag now, for `LineIndex::record`.
pub fn cut_reused(
    s: &Scope,
    reuse: &Reuse,
    snap_ms: u32,
    done: &mut Rendered,
) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    for (e, want) in &reuse.takes {
        let b = e.batch();
        let a = s.cache.get(s.provider, &e.key).with_context(|| {
            format!(
                "a cached take of {} lines went missing: estimate again",
                b.items.len()
            )
        })?;
        let cuts = batch::cut(&b, &a, snap_ms).with_context(|| {
            format!(
                "the cached take of a batch of {} {} lines does not cut",
                b.items.len(),
                b.tone
            )
        })?;
        let flags = batch::check(&cuts);
        for c in &cuts {
            for w in want.iter().filter(|w| w.take == c.spoken) {
                match flags.iter().find(|f| f.spoken == c.spoken) {
                    Some(f) => {
                        done.warnings.push(format!("{:?}: {}", w.line, f.reason));
                        done.flagged.insert(w.line.clone(), f.reason.clone());
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
    Ok(out)
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
