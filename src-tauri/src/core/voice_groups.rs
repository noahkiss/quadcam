//! Grouped re-takes (design 7.4): the re-take lines of a studio pack, grouped by the word each
//! got wrong. Lines that share a word often fail the same way, so one line of each group (the
//! sample, its shortest) is made again in other ways of saying it until the speech check hears
//! it right: another seed, a carrier that ends without a hiss, the word written another way.
//! The way that wins becomes a structure of the pack (`packs::Structure`) for the group's
//! lines, and the rest of the group is made that way. A group no way wins keeps its lines out.
//!
//! A paid call. Without `confirm` (or with `dry_run`) it prices the most every try and every
//! group could cost and answers that plan's digest; `confirm` with the digest spends at most
//! that. Each try is a studio render of one line, priced and checked like any render.

use super::voice_check::{read_manifest, write_manifest};
use super::voice_samples::{RetakeParams, RetakeReport};
use super::voice_studio::SampleParams;
use super::voice_studio::PROVIDER;
use super::voice_studio::{plan_digest, same_plan};
use super::{Core, VoiceRenderParams};
use crate::gear::voice::asr;
use crate::gear::voice::batch::{self, BatchCache, BatchSettings, Estimate, Item, LETTER_NAMES};
use crate::gear::voice::lines::Spelling;
use crate::gear::voice::packs::{self, Structure};
use crate::gear::voice::render::RenderSettings;
use crate::gear::voice::reuse::{LineIndex, Scope};
use crate::gear::voice::sets;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Ways one sample is tried at most, by default.
pub const TRIES: u32 = 6;
/// The most a call may ask for.
const MAX_TRIES: u32 = 12;

/// Carriers a try may use besides the render's own ("The word is {line}."): neither ends in
/// a hiss, so the line's first sound stands apart from the carrier's last.
const CARRIERS: &[&str] = &["Say {line}.", "Okay, {line}."];

/// One group of a grouped re-take.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct RetakeGroup {
    /// The word the group's lines got wrong, as the first line's text writes it.
    pub word: String,
    /// Its card paths.
    pub lines: Vec<String>,
    /// The line tried first.
    pub sample: String,
    /// Each way the sample was tried (all of them, priced, in a dry run).
    pub tries: Vec<RetakeTry>,
    /// The way that won, kept by the pack for the group's lines; absent when none won, or
    /// when a new seed alone was enough.
    #[serde(default)]
    pub structure: Option<Structure>,
    /// A way won (the sample is in the pack).
    #[serde(default)]
    pub won: bool,
    /// Lines in the pack now.
    #[serde(default)]
    pub passed: Vec<String>,
    /// Lines still out of the pack.
    #[serde(default)]
    pub still_out: Vec<String>,
}

/// One way of saying a group's sample.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct RetakeTry {
    /// What the voice is sent for the word.
    pub say: String,
    /// The carrier; absent for the render's own.
    #[serde(default)]
    pub carrier: Option<String>,
    pub seed: u64,
    /// The sentence sent.
    pub sentence: String,
    /// Not tried yet (a dry run, or a way after the one that won).
    #[serde(default)]
    pub tried: bool,
    /// The speech check heard the line right and its cut passed.
    #[serde(default)]
    pub ok: bool,
    /// What the speech check heard.
    #[serde(default)]
    pub heard: Option<String>,
    /// Why the line is still out.
    #[serde(default)]
    pub reason: Option<String>,
}

/// The words of a text, as its own spelling writes them.
fn text_words(text: &str) -> Vec<&str> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect()
}

/// The word a line got wrong: the first of its words the recognizer did not hear, or its first
/// word when nothing was heard or the cut failed instead.
fn root_word(text: &str, heard: Option<&str>, sp: &Spelling) -> String {
    let words = text_words(text);
    if let Some(h) = heard.filter(|h| !h.trim().is_empty()) {
        let got = asr::words(h, sp);
        if let Some(w) = words.iter().find(|w| {
            let want = asr::normalise(&sp.apply(w), sp);
            !want.is_empty() && !got.contains(&want)
        }) {
            return w.to_string();
        }
    }
    words.first().map(|w| w.to_string()).unwrap_or_default()
}

/// A word a voice spells out letter by letter: dotted ("L.Q.") or short capitals and digits
/// ("OSD", "3D").
fn initialism(said: &str) -> bool {
    let letters: Vec<char> = said.chars().filter(|c| c.is_alphanumeric()).collect();
    !letters.is_empty()
        && (said.contains('.') && letters.iter().all(|c| !c.is_lowercase())
            || letters.len() <= 4
                && letters.iter().any(|c| c.is_ascii_uppercase())
                && letters
                    .iter()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
}

/// The word written closer to how it sounds, when a rule changes it: "oi" as "oy", "ck" as
/// "k", "ph" as "f", a final "ed" after a voiceless sound as "t".
fn respell(word: &str) -> Option<String> {
    let mut w = word
        .replace("oi", "oy")
        .replace("ck", "k")
        .replace("ph", "f");
    for end in ["ked", "ped", "sed", "fed", "xed", "shed", "ched"] {
        if let Some(stem) = w.strip_suffix(end) {
            w = format!("{stem}{}t", &end[..end.len() - 2]);
            break;
        }
    }
    (w != word).then_some(w)
}

/// What a voice may be sent for a word, the spelling's own first: an initialism also as
/// spaced letters and as its letters' names, another word as `respell` writes it and with its
/// first letter's case flipped.
fn says(word: &str, sp: &Spelling) -> Vec<String> {
    let said = sp.apply(word);
    let mut out = vec![said.clone()];
    if initialism(&said) {
        let letters: Vec<char> = said.chars().filter(|c| c.is_alphanumeric()).collect();
        out.push(
            letters
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(" "),
        );
        out.push(
            letters
                .iter()
                .map(|c| match c.to_digit(10) {
                    Some(d) => crate::gear::voice::asr::digit_word(d).to_string(),
                    None => LETTER_NAMES[(c.to_ascii_uppercase() as u8 - b'A') as usize].into(),
                })
                .collect::<Vec<_>>()
                .join(" "),
        );
    } else {
        if let Some(r) = respell(&said) {
            out.push(r);
        }
        // A voice may read a capital as the start of a sentence, and a small letter as a word
        // inside one.
        let mut c = said.chars();
        if let Some(f) = c.next() {
            let flipped: String = if f.is_uppercase() {
                f.to_lowercase().chain(c).collect()
            } else {
                f.to_uppercase().chain(c).collect()
            };
            if flipped != said {
                out.push(flipped);
            }
        }
    }
    out.dedup();
    out
}

/// The ways a group's sample is tried, in order: the way it was made with a new seed, then
/// ways further from it, a change of carrier before a change of word. At most `n`.
fn ways(word: &str, sp: &Spelling, n: u32) -> Vec<(String, Option<String>)> {
    let says = says(word, sp);
    let carriers: Vec<Option<String>> = std::iter::once(None)
        .chain(CARRIERS.iter().map(|c| Some(c.to_string())))
        .collect();
    let mut all: Vec<(usize, usize)> = (0..says.len())
        .flat_map(|s| (0..carriers.len()).map(move |c| (s, c)))
        .collect();
    all.sort_by_key(|&(s, c)| (s + c, s));
    all.into_iter()
        .take(n as usize)
        .map(|(s, c)| (says[s].clone(), carriers[c].clone()))
        .collect()
}

/// The seed of one try: from the pack, the word, the sample's last reason, how many takes of
/// the word the voice has, and the try's place, so a confirm repeats the plan its estimate
/// priced and a later run tries new takes.
fn try_seed(pack: &str, word: &str, reason: &str, k: usize) -> u64 {
    let h = crate::gear::voice::sha256_hex(format!("{pack}\n{word}\n{reason}\n{k}").as_bytes());
    1 + u64::from_str_radix(&h[..12], 16).unwrap_or(0) % 2_000_000_000
}

/// The sentence a line is sent in: its text spoken with `say` for `word`, in `carrier` or the
/// render's own.
fn sentence(text: &str, word: &str, say: &str, carrier: Option<&str>, sp: &Spelling) -> String {
    let spoken = sp
        .with_words(&[(word.to_string(), say.to_string())])
        .apply(text);
    let bs = BatchSettings {
        carrier: carrier
            .map(String::from)
            .unwrap_or_else(|| BatchSettings::default().carrier),
        ..Default::default()
    };
    batch::sentence_of(
        &bs,
        &Item {
            spoken,
            tone: String::new(),
        },
    )
}

/// What a grouped re-take works on: a studio pack, or one voice's samples.
enum On {
    Pack { id: String, dir: PathBuf },
    Sample { id: String, dir: PathBuf },
}

/// The voice, model and settings of what a grouped re-take works on, and its line sets.
struct Who {
    voice: String,
    model: String,
    settings: RenderSettings,
    sets: Vec<String>,
}

/// A line out: a pack's re-take line, or a sample the speech check heard wrong.
struct Out {
    path: String,
    text: String,
    reason: String,
    heard: Option<String>,
}

/// Where a line stands after a try: in (heard right, its cut passed), what was heard, why
/// it is out.
type Standing = (bool, Option<String>, Option<String>);

impl Core {
    /// The pack or sample voice a grouped re-take names, and its lines out.
    fn group_on(&self, p: &RetakeParams) -> Result<(On, Who, Vec<Out>)> {
        let (pack, sample) = (p.pack.trim(), p.sample.trim());
        if pack.is_empty() == sample.is_empty() {
            bail!("Name one pack or one sample for a grouped re-take.");
        }
        if !pack.is_empty() {
            let dir = self.voices_dir().join(pack);
            let m = read_manifest(&dir)
                .map_err(|_| anyhow::anyhow!("Pack {pack:?} is not on this Mac."))?;
            if !super::voice_check::holds_lines(&m) {
                bail!("Only a pack the Voice studio rendered can make a re-take.");
            }
            let out = m
                .retakes
                .iter()
                .map(|r| Out {
                    path: r.path.clone(),
                    text: r.text.clone(),
                    reason: r.reason.clone(),
                    heard: asr::is_speech_reason(&r.reason)
                        .then(|| m.heard.iter().find(|h| h.path == r.path))
                        .flatten()
                        .map(|h| h.heard.clone()),
                })
                .collect();
            let who = Who {
                voice: m.voice_id.clone(),
                model: m.model.clone(),
                settings: m.settings.clone(),
                sets: sets::ids().into_iter().map(String::from).collect(),
            };
            return Ok((
                On::Pack {
                    id: pack.into(),
                    dir,
                },
                who,
                out,
            ));
        }
        let (dir, set) = self.sample_set(sample)?;
        if set.voice.is_empty() {
            bail!("The sample {sample} was made before QuadCam kept its voice's id: sample the voice again in the Voice studio.");
        }
        let out = set
            .takes
            .iter()
            .filter(|t| t.heard_ok == Some(false))
            .map(|t| {
                let heard = t.heard.clone().unwrap_or_default();
                Out {
                    path: t.line.clone(),
                    text: t.text.clone(),
                    reason: asr::reason(&heard),
                    heard: Some(heard),
                }
            })
            .collect();
        let who = Who {
            voice: set.voice.clone(),
            model: set.model.clone(),
            settings: RenderSettings {
                speed: set.speed,
                ..Default::default()
            },
            sets: if set.sets.is_empty() {
                vec!["sample".into()]
            } else {
                set.sets.clone()
            },
        };
        Ok((
            On::Sample {
                id: sample.into(),
                dir,
            },
            who,
            out,
        ))
    }

    /// Renders `lines` of what a grouped re-take works on with `seed`: priced (`dry_run`),
    /// or paid with the digest of that price. Answers the estimate and the digest.
    fn group_render(
        &self,
        who: &Who,
        on: &On,
        lines: Vec<String>,
        seed: u64,
        dry_run: bool,
        digest: Option<String>,
    ) -> Result<(Estimate, String)> {
        let settings = RenderSettings {
            seed,
            ..who.settings.clone()
        };
        match on {
            On::Pack { .. } => {
                let r = self.voice_studio_render(&VoiceRenderParams {
                    voice: who.voice.clone(),
                    lines,
                    dry_run,
                    confirm: !dry_run,
                    settings: Some(settings),
                    sets: who.sets.clone(),
                    model: who.model.clone(),
                    batch: None,
                    digest,
                })?;
                Ok((r.estimate.unwrap_or_default(), r.digest))
            }
            On::Sample { .. } => {
                let r = self.gear_voice_sample(&SampleParams {
                    voices: vec![who.voice.clone()],
                    models: vec![who.model.clone()],
                    sets: who.sets.clone(),
                    lines,
                    dry_run,
                    confirm: !dry_run,
                    settings: Some(settings),
                    batch: None,
                    digest,
                })?;
                Ok((r.estimate, r.digest))
            }
        }
    }

    /// Puts a group's structure in place of the word's structure for those lines (`None`
    /// takes the lines out of it).
    fn group_structure_set(
        &self,
        on: &On,
        word: &str,
        lines: &[String],
        s: Option<Structure>,
    ) -> Result<()> {
        match on {
            On::Pack { dir, .. } => {
                let mut m = read_manifest(dir)?;
                packs::replace_structure(&mut m.structures, word, lines, s);
                write_manifest(dir, &m)
            }
            On::Sample { dir, .. } => self.set_sample_structure(dir, word, lines, s),
        }
    }

    /// The structures of what a grouped re-take works on, read again.
    fn group_structures(&self, on: &On) -> Result<Vec<Structure>> {
        Ok(match on {
            On::Pack { dir, .. } => read_manifest(dir)?.structures,
            On::Sample { id, .. } => self.sample_set(id)?.1.structures,
        })
    }

    /// Where each line stands now. A sample whose take still has another seed than `seed`
    /// got no new take: its cut failed a check.
    fn group_standing(&self, on: &On, paths: &[String], seed: u64) -> Result<Vec<Standing>> {
        Ok(match on {
            On::Pack { dir, .. } => {
                let m = read_manifest(dir)?;
                paths
                    .iter()
                    .map(|path| {
                        let out = m.retakes.iter().find(|r| r.path == *path);
                        (
                            m.files.iter().any(|f| f == path) && out.is_none(),
                            m.heard
                                .iter()
                                .find(|h| h.path == *path)
                                .map(|h| h.heard.clone()),
                            out.map(|r| r.reason.clone()),
                        )
                    })
                    .collect()
            }
            On::Sample { id, .. } => {
                let set = self.sample_set(id)?.1;
                paths
                    .iter()
                    .map(|path| {
                        let t = set.takes.iter().find(|t| t.line == *path);
                        let fresh = t.is_some_and(|t| t.seed == Some(seed));
                        let ok = fresh && t.is_some_and(|t| t.heard_ok == Some(true));
                        let heard = t.and_then(|t| t.heard.clone());
                        let reason = (!ok).then(|| {
                            if fresh {
                                asr::reason(heard.as_deref().unwrap_or(""))
                            } else {
                                "its new take failed a check of its cut".into()
                            }
                        });
                        (ok, heard, reason)
                    })
                    .collect()
            }
        })
    }

    /// `gear_voice_retake` with `grouped`: see the module.
    pub(super) fn voice_retake_grouped(&self, p: &RetakeParams) -> Result<RetakeReport> {
        let tries = p.tries.unwrap_or(TRIES);
        if !(1..=MAX_TRIES).contains(&tries) {
            bail!("tries takes 1 to {MAX_TRIES}, not {tries}.");
        }
        let (on, who, all) = self.group_on(p)?;
        let (kind, id) = match &on {
            On::Pack { id, .. } => ("pack", id.clone()),
            On::Sample { id, .. } => ("sample", id.clone()),
        };
        for l in &p.lines {
            if !all.iter().any(|r| r.path == *l) {
                bail!("{l:?} is not a line of {id} to re-take.");
            }
        }
        let out: Vec<&Out> = all
            .iter()
            .filter(|r| p.lines.is_empty() || p.lines.contains(&r.path))
            .collect();
        if out.is_empty() {
            bail!("{id} has no line to re-take.");
        }
        let sp = Spelling::builtin()?;
        // The groups, in the order their first line comes.
        let mut groups: Vec<RetakeGroup> = Vec::new();
        for r in &out {
            let word = root_word(&r.text, r.heard.as_deref(), &sp);
            match groups
                .iter_mut()
                .find(|g| g.word.eq_ignore_ascii_case(&word))
            {
                Some(g) => g.lines.push(r.path.clone()),
                None => groups.push(RetakeGroup {
                    word,
                    lines: vec![r.path.clone()],
                    sample: String::new(),
                    tries: Vec::new(),
                    structure: None,
                    won: false,
                    passed: Vec::new(),
                    still_out: Vec::new(),
                }),
            }
        }
        let text_of = |path: &str| -> String {
            out.iter()
                .find(|r| r.path == path)
                .map(|r| r.text.clone())
                .unwrap_or_default()
        };
        let reason_of = |path: &str| -> String {
            out.iter()
                .find(|r| r.path == path)
                .map(|r| r.reason.clone())
                .unwrap_or_default()
        };
        let cache = BatchCache::new(&self.cache);
        let index = LineIndex::load(&Scope {
            cache: &cache,
            provider: PROVIDER,
            voice: &who.voice,
            model: &who.model,
            settings: &who.settings,
        });
        // How many takes hold the word: each paid try adds one.
        let takes_of = |word: &str| {
            index
                .entries()
                .iter()
                .filter(|e| {
                    e.items.iter().any(|i| {
                        text_words(&i.spoken)
                            .iter()
                            .any(|w| w.eq_ignore_ascii_case(word))
                    })
                })
                .count()
        };
        let mut chars: u64 = 0;
        for g in &mut groups {
            g.sample = g
                .lines
                .iter()
                .min_by_key(|l| (text_of(l).chars().count(), (*l).clone()))
                .cloned()
                .unwrap_or_default();
            let text = text_of(&g.sample);
            let key = format!("{}|{}", reason_of(&g.sample), takes_of(&g.word));
            g.tries = ways(&g.word, &sp, tries)
                .into_iter()
                .enumerate()
                .map(|(k, (say, carrier))| RetakeTry {
                    sentence: sentence(&text, &g.word, &say, carrier.as_deref(), &sp),
                    say,
                    carrier,
                    seed: try_seed(&id, &g.word, &key, k),
                    tried: false,
                    ok: false,
                    heard: None,
                    reason: None,
                })
                .collect();
            chars += g
                .tries
                .iter()
                .map(|t| t.sentence.chars().count() as u64)
                .sum::<u64>();
            // The rest of the group, said the longest way a try could win with.
            for l in g.lines.iter().filter(|l| **l != g.sample) {
                let t = text_of(l);
                chars += g
                    .tries
                    .iter()
                    .map(|x| {
                        sentence(&t, &g.word, &x.say, x.carrier.as_deref(), &sp)
                            .chars()
                            .count() as u64
                    })
                    .max()
                    .unwrap_or(0);
            }
        }
        // The price of a character, from the render's own estimate of the samples.
        let (rate, _) = self.group_render(
            &who,
            &on,
            groups.iter().map(|g| g.sample.clone()).collect(),
            1,
            true,
            None,
        )?;
        let most = Estimate {
            batches: groups.iter().map(|g| g.tries.len() as u32 + 1).sum(),
            cached_batches: 0,
            lines: out.len() as u32,
            chars,
            credits: (chars as f64 * rate.cost_per_char).ceil() as u64,
            usd: chars as f64 / 1000.0 * rate.usd_per_1k,
            affordable: rate
                .remaining
                .is_none_or(|r| r as f64 >= chars as f64 * rate.cost_per_char),
            ..rate.clone()
        };
        let digest = plan_digest(&serde_json::json!({
            "kind": "retake_grouped", "target": kind, "id": id, "chars": chars,
            "credits": most.credits,
            "groups": groups.iter().map(|g| serde_json::json!({
                "word": g.word, "lines": g.lines, "sample": g.sample,
                "tries": g.tries.iter().map(|t| (&t.sentence, t.seed)).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        }));
        let mut report = RetakeReport {
            kind: kind.into(),
            id: id.clone(),
            lines: out.iter().map(|r| r.path.clone()).collect(),
            seed: 0,
            estimate: Some(most.clone()),
            needs_confirm: false,
            dry_run: p.dry_run,
            digest: digest.clone(),
            cleared: Vec::new(),
            notes: vec![format!(
                "{} lines in {} groups by the word each got wrong; at most {tries} ways per group's sample. The price is the most it can cost: every try made, and the rest of each group said its longest way.",
                out.len(),
                groups.len()
            )],
            groups: groups.clone(),
            still_out: Vec::new(),
        };
        if p.dry_run {
            return Ok(report);
        }
        if !p.confirm {
            report.needs_confirm = true;
            return Ok(report);
        }
        same_plan(p.digest.as_deref(), &digest)?;
        if !most.affordable {
            bail!("The credits left do not cover the most this re-take can cost.");
        }
        // Characters and credits sent so far. The plan bounds the characters: credits round up
        // per render.
        let mut spent: (u64, u64) = (0, 0);
        let run = |lines: Vec<String>, seed: u64, spent: &mut (u64, u64)| -> Result<()> {
            let (e, d) = self.group_render(&who, &on, lines.clone(), seed, true, None)?;
            if spent.0 + e.chars > chars {
                bail!("This try would pass the most the plan priced; stopped.");
            }
            self.group_render(&who, &on, lines, seed, false, Some(d))?;
            spent.0 += e.chars;
            spent.1 += e.credits;
            Ok(())
        };
        let said = |w: &str| sp.apply(w);
        for g in &mut groups {
            let text = text_of(&g.sample);
            let mut winner: Option<usize> = None;
            for k in 0..g.tries.len() {
                let t = g.tries[k].clone();
                let structure =
                    (t.say != said(&g.word) || t.carrier.is_some()).then(|| Structure {
                        word: g.word.clone(),
                        say: t.say.clone(),
                        carrier: t.carrier.clone(),
                        lines: g.lines.clone(),
                    });
                self.group_structure_set(&on, &g.word, &g.lines, structure)?;
                let r = run(vec![g.sample.clone()], t.seed, &mut spent);
                let (ok, heard, reason) = self
                    .group_standing(&on, std::slice::from_ref(&g.sample), t.seed)?
                    .remove(0);
                let x = &mut g.tries[k];
                x.tried = true;
                x.ok = ok && r.is_ok();
                x.heard = heard;
                x.reason = match r {
                    Err(e) => Some(format!("{e:#}")),
                    Ok(()) => reason,
                };
                if x.ok {
                    winner = Some(k);
                    break;
                }
                if x.reason.as_deref().is_some_and(|r| r.contains("stopped")) {
                    break;
                }
            }
            let seed = match winner {
                Some(k) => g.tries[k].seed,
                None => {
                    self.group_structure_set(&on, &g.word, &g.lines, None)?;
                    report.notes.push(format!(
                        "{:?}: no way of saying {:?} was heard right; its lines stay out.",
                        text, g.word
                    ));
                    0
                }
            };
            if winner.is_some() {
                g.won = true;
                g.structure = self
                    .group_structures(&on)?
                    .into_iter()
                    .find(|s| s.word.eq_ignore_ascii_case(&g.word) && s.lines.contains(&g.sample));
                let rest: Vec<String> = g
                    .lines
                    .iter()
                    .filter(|l| **l != g.sample)
                    .cloned()
                    .collect();
                if !rest.is_empty() {
                    if let Err(e) = run(rest, seed, &mut spent) {
                        report.notes.push(format!(
                            "{:?}: the rest of its group was not made: {e:#}",
                            g.word
                        ));
                    }
                }
            }
            let now = self.group_standing(&on, &g.lines, seed)?;
            for (l, (ok, _, reason)) in g.lines.iter().zip(now) {
                if ok || (winner.is_some() && *l == g.sample) {
                    g.passed.push(l.clone());
                } else {
                    g.still_out.push(l.clone());
                    report.still_out.push(packs::Retake {
                        path: l.clone(),
                        text: text_of(l),
                        reason: reason.unwrap_or_default(),
                    });
                }
            }
        }
        report.cleared = groups.iter().flat_map(|g| g.passed.clone()).collect();
        report.notes.push(format!(
            "Sent {} of the {chars} characters priced, about {} credits.",
            spent.0, spent.1
        ));
        report.groups = groups;
        Ok(report)
    }
}

/// A grouped re-take's groups as text.
pub fn groups_text(groups: &[RetakeGroup]) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    for g in groups {
        let _ = write!(
            s,
            "\n{:?}: {} lines, sample {}",
            g.word,
            g.lines.len(),
            g.sample
        );
        for t in &g.tries {
            let what = if !t.tried {
                "-".to_string()
            } else if t.ok {
                "WON".to_string()
            } else {
                t.reason.clone().unwrap_or_else(|| "out".into())
            };
            let _ = write!(s, "\n  {:?} seed {}: {what}", t.sentence, t.seed);
        }
        if let Some(st) = &g.structure {
            let _ = write!(
                s,
                "\n  kept: say {:?}{}",
                st.say,
                st.carrier
                    .as_deref()
                    .map(|c| format!(" in {c:?}"))
                    .unwrap_or_default()
            );
        }
        if g.won || !g.passed.is_empty() || !g.still_out.is_empty() {
            let _ = write!(
                s,
                "\n  in the pack: {}; still out: {}",
                g.passed.len(),
                g.still_out.len()
            );
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_groups_by_the_word_it_got_wrong() {
        let sp = Spelling::builtin().unwrap();
        assert_eq!(
            root_word("Spoilers down", Some("Boilers down."), &sp),
            "Spoilers"
        );
        assert_eq!(root_word("Angle off", Some("Angle loft."), &sp), "off");
        assert_eq!(root_word("Angle on", Some("Hang along."), &sp), "Angle");
        assert_eq!(root_word("RF mode", Some("Ref mode."), &sp), "RF");
        assert_eq!(
            root_word("Diffs locked", Some("Diff's lock."), &sp),
            "locked"
        );
        assert_eq!(root_word("LQ", None, &sp), "LQ");
        assert_eq!(root_word("High rate", None, &sp), "High");
        assert_eq!(root_word("Lap", Some(""), &sp), "Lap");
    }

    #[test]
    fn the_ways_start_near_the_take_and_move_away() {
        let sp = Spelling::builtin().unwrap();
        let w = ways("Spoilers", &sp, 6);
        assert_eq!(w[0], ("Spoilers".into(), None));
        assert_eq!(w[1], ("Spoilers".into(), Some("Say {line}.".into())));
        assert_eq!(w[2], ("Spoylers".into(), None));
        assert_eq!(w.len(), 6);
        let lq = says("LQ", &sp);
        assert_eq!(lq, vec!["L.Q.", "L Q", "el cue"]);
        assert_eq!(says("3D", &sp), vec!["3D", "3 D", "three dee"]);
        assert_eq!(says("Locked", &sp), vec!["Locked", "Lokt", "locked"]);
        assert_eq!(says("thirty", &sp), vec!["thirty", "Thirty"]);
        assert_eq!(ways("thirty", &sp, 6).len(), 6);
        assert_eq!(ways("thirty", &sp, 4).len(), 4);
    }

    #[test]
    fn a_try_is_its_sentence() {
        let sp = Spelling::builtin().unwrap();
        assert_eq!(
            sentence("Spoilers down", "Spoilers", "Spoylers", None, &sp),
            "The word is Spoylers down."
        );
        assert_eq!(
            sentence("RF mode", "RF", "R F", Some("Say {line}."), &sp),
            "Say R F mode."
        );
        assert_ne!(try_seed("p", "w", "r", 0), try_seed("p", "w", "r", 1));
        assert_ne!(try_seed("p", "w", "r", 0), try_seed("p", "w", "s", 0));
    }
}
