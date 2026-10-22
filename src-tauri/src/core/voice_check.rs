//! The speech check on the core (design 7.4): each line of a voice pack heard by the on-device
//! recognizer (`crate::intel`, the `quadcam-intel` helper) and compared with the line's text
//! (`gear::voice::asr`). It runs after every render and sample, and on request.
//!
//! The pack's `pack.json` keeps what was heard per line. In a studio pack (ElevenLabs, batched)
//! the line index keeps it per take too, so a mismatch is a re-take like a cut that failed its
//! checks: the line leaves the pack, a render with the same seed keeps it out at no cost, and
//! a render with another seed prices it, waits for `confirm` and renders it again. Nothing here
//! calls a provider. The person may accept what was heard: the line passes, and its file goes
//! back into the pack. A missing helper or an older macOS makes the check unavailable, never
//! an error.

use super::voice_studio::SampleReport;
use super::voice_studio::{kept_key, PROVIDER};
use super::{Core, RenderReport};
use crate::gear::voice::asr;
use crate::gear::voice::batch::{self, BatchCache};
use crate::gear::voice::lines::Spelling;
use crate::gear::voice::packs::{self, HeardLine, PackManifest, Retake};
use crate::gear::voice::render::RenderSettings;
use crate::gear::voice::reuse::{Entry, EntryHeard, LineIndex, Scope};
use crate::gear::voice::sets;
use crate::gear::voice::wav::{self, Pcm};
use crate::intel::{self, Feature};
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// `gear_voice_check`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct VoiceCheckParams {
    /// An installed or rendered pack id.
    pub pack: String,
    /// Card paths to check; empty for every line of the pack. A line the check took out of
    /// the pack is heard again from its held take, and goes back in when heard right.
    #[serde(default)]
    pub lines: Vec<String>,
    /// Card paths whose heard text the person accepts: each passes, and a line the check took
    /// out of the pack goes back in. Nothing is heard again.
    #[serde(default)]
    pub accept: Vec<String>,
    /// First install the recognizer's assets for the pack's language through macOS.
    #[serde(default)]
    pub install: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct VoiceCheckReport {
    pub pack: String,
    /// The speech check ran; else `reason` says why it could not.
    pub available: bool,
    #[serde(default)]
    pub reason: Option<String>,
    /// The lines heard (or accepted) now.
    pub lines: Vec<HeardLine>,
    /// Lines whose heard text does not match.
    pub mismatches: u32,
    /// Of those, lines taken out of the pack as re-takes.
    #[serde(default)]
    pub held: u32,
    /// Lines out of the pack as re-takes, named in `lines`, whose held take the check heard
    /// right now: back in the pack.
    #[serde(default)]
    pub back: u32,
    pub notes: Vec<String>,
}

/// The pack's files, read again.
pub(super) fn read_manifest(dir: &std::path::Path) -> Result<PackManifest> {
    let b = std::fs::read(dir.join("pack.json"))?;
    Ok(serde_json::from_slice(&b)?)
}

pub(super) fn write_manifest(dir: &std::path::Path, m: &PackManifest) -> Result<()> {
    let tmp = dir.join("pack.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(m)?)?;
    std::fs::rename(&tmp, dir.join("pack.json"))?;
    Ok(())
}

/// Whether a mismatch takes the line out of the pack: a studio pack, which has a re-take path.
pub(super) fn holds_lines(m: &PackManifest) -> bool {
    m.id.starts_with("local-") && m.provider == PROVIDER && !m.voice_id.is_empty()
}

impl Core {
    /// Hears the lines of a pack and compares each with its text, or accepts what was heard
    /// for some lines. Changes only QuadCam's own files.
    pub fn gear_voice_check(&self, p: &VoiceCheckParams) -> Result<VoiceCheckReport> {
        let id = p.pack.trim();
        if id.is_empty() {
            bail!("Name a pack (quadcam-cli gear voice lists them).");
        }
        packs::check_id(id)?;
        let dir = self.voices_dir().join(id);
        let m = read_manifest(&dir).map_err(|_| anyhow!("Pack {id:?} is not on this Mac."))?;
        let mut notes = Vec::new();
        if p.install {
            let locale = intel::locale_of(&m.lang);
            let status = self.gear.intel.install_speech(&locale)?;
            notes.push(if status == "installed" {
                format!("The speech assets for {locale} are installed.")
            } else {
                format!("The speech assets for {locale} were installed already.")
            });
        }
        let mut r = if p.accept.is_empty() {
            self.voice_check_pack(id, &p.lines)?
        } else {
            self.voice_accept(id, &p.accept)?
        };
        notes.append(&mut r.notes);
        r.notes = notes;
        Ok(r)
    }

    /// The speech check's state for a pack language.
    pub(super) fn speech(&self, lang: &str) -> Feature {
        self.gear.intel.speech(&intel::locale_of(lang))
    }

    /// Every line's text by card path: QuadCam's lines, every set's, and the person's own.
    pub(super) fn line_texts(&self) -> Result<HashMap<String, String>> {
        let mut out: HashMap<String, String> = HashMap::new();
        for id in sets::ids() {
            if id == sets::CUSTOM {
                continue;
            }
            for l in sets::lines_of(id)? {
                out.entry(l.line.path).or_insert(l.line.text);
            }
        }
        for l in self.all_lines()? {
            out.insert(l.path, l.text);
        }
        Ok(out)
    }

    /// Where the takes of a pack's lines heard wrong wait, out of the pack.
    pub(super) fn held_dir(&self, id: &str) -> PathBuf {
        self.cache.join("voice").join("held").join(id)
    }

    /// Hears `only` (or every line) of a pack, records what was heard, and in a studio pack
    /// takes each line heard wrong out of the pack as a re-take.
    pub(super) fn voice_check_pack(&self, id: &str, only: &[String]) -> Result<VoiceCheckReport> {
        let dir = self.voices_dir().join(id);
        let mut m = read_manifest(&dir)?;
        let mut report = VoiceCheckReport {
            pack: id.to_string(),
            available: false,
            reason: None,
            lines: Vec::new(),
            mismatches: 0,
            held: 0,
            back: 0,
            notes: Vec::new(),
        };
        // A line the check took out of the pack is heard again from its held take, when named.
        let held_out = |o: &String| {
            !m.files.contains(o)
                && m.retakes
                    .iter()
                    .any(|r| r.path == *o && asr::is_speech_reason(&r.reason))
                && self.held_dir(id).join(o).is_file()
        };
        for o in only {
            if !m.files.contains(o) && !held_out(o) {
                bail!("The pack {id} has no line {o:?}.");
            }
        }
        let texts = self.line_texts()?;
        let mut paths: Vec<String> = Vec::new();
        let named_held: Vec<&String> = only.iter().filter(|o| held_out(o)).collect();
        for f in m.files.iter().chain(named_held.iter().copied()) {
            if !only.is_empty() && !only.contains(f) {
                continue;
            }
            if texts.contains_key(f) {
                paths.push(f.clone());
            } else {
                report.notes.push(format!(
                    "{f}: no line names this file, so it was not heard."
                ));
            }
        }
        let was_held: HashSet<String> = named_held.into_iter().cloned().collect();
        let files: Vec<PathBuf> = paths
            .iter()
            .map(|f| {
                if was_held.contains(f) {
                    self.held_dir(id).join(f)
                } else {
                    dir.join(f)
                }
            })
            .collect();
        let t = self.hear(&files, &intel::locale_of(&m.lang))?;
        if let Some(why) = t.unavailable {
            report.reason = Some(why);
            return Ok(report);
        }
        report.available = true;
        let spelling = Spelling::builtin()?;
        let studio = holds_lines(&m);
        let cache = BatchCache::new(&self.cache);
        let settings = m.settings.clone();
        let scope = Scope {
            cache: &cache,
            provider: PROVIDER,
            voice: &m.voice_id,
            model: &m.model,
            settings: &settings,
        };
        let index = if studio {
            LineIndex::load(&scope)
        } else {
            LineIndex::default()
        };
        let mut takes: HashMap<usize, Entry> = HashMap::new();
        let mut checked: Vec<HeardLine> = Vec::new();
        let mut held: Vec<(String, String, Option<usize>)> = Vec::new();
        for (path, h) in paths.iter().zip(&t.heard) {
            if let Some(e) = &h.error {
                report
                    .notes
                    .push(format!("{path}: the recognizer could not read it ({e})."));
                continue;
            }
            let text = &texts[path];
            let heard = h.text.clone().unwrap_or_default();
            let spoken = spelling.apply(text);
            let ok = asr::same(&spoken, &heard, &spelling);
            // The line index knows a take by what the voice was sent; the check compares with
            // the line's own text.
            let said = m.spelling_for(&spelling, path).apply(text);
            let take = if studio {
                index.take_of(&scope, &said)
            } else {
                None
            };
            // The person accepted this same heard text before, in this pack or for this take:
            // it stays accepted.
            let accepted = !ok
                && (m
                    .heard
                    .iter()
                    .any(|x| x.path == *path && x.accepted && x.heard == heard)
                    || take.is_some_and(|n| {
                        index.entries()[n].heard.iter().any(|x| {
                            x.accepted && x.heard == heard && x.spoken.eq_ignore_ascii_case(&said)
                        })
                    }));
            let line = HeardLine {
                path: path.clone(),
                text: text.clone(),
                heard: heard.clone(),
                ok,
                accepted,
                confidence: h.confidence,
            };
            if let Some(n) = take {
                let e = takes.entry(n).or_insert_with(|| index.entries()[n].clone());
                let said = e
                    .items
                    .iter()
                    .find(|i| i.spoken.eq_ignore_ascii_case(&said))
                    .map(|i| i.spoken.clone())
                    .unwrap_or(said.clone());
                e.set_heard(EntryHeard {
                    spoken: said,
                    heard: heard.clone(),
                    ok,
                    accepted,
                    cut: batch::CUT_VERSION,
                });
            }
            if !ok && !accepted {
                report.mismatches += 1;
                if studio {
                    held.push((path.clone(), heard.clone(), take));
                }
            }
            checked.push(line);
        }
        for l in &checked {
            m.heard.retain(|x| x.path != l.path);
            m.heard.push(l.clone());
        }
        m.heard.sort_by(|a, b| a.path.cmp(&b.path));
        // A held take heard right now goes back into the pack.
        for l in checked
            .iter()
            .filter(|l| (l.ok || l.accepted) && was_held.contains(&l.path))
        {
            let to = dir.join(&l.path);
            std::fs::create_dir_all(to.parent().unwrap())?;
            std::fs::rename(self.held_dir(id).join(&l.path), &to)?;
            m.files.push(l.path.clone());
            m.retakes.retain(|r| r.path != l.path);
            report.back += 1;
        }
        m.files.sort();
        for (path, heard, take) in &held {
            if !was_held.contains(path) {
                let to = self.held_dir(id).join(path);
                std::fs::create_dir_all(to.parent().unwrap())?;
                let _ = std::fs::remove_file(&to);
                std::fs::rename(dir.join(path), &to)?;
            }
            m.files.retain(|f| f != path);
            m.retakes.retain(|r| r.path != *path);
            m.retakes.push(Retake {
                path: path.clone(),
                text: texts[path].clone(),
                reason: asr::reason(heard),
            });
            // The take's batch is no longer whole in the pack: a re-render cuts it again.
            if let Some(n) = take {
                let k = kept_key(
                    &m.voice_id,
                    &m.model,
                    &m.settings,
                    &index.entries()[*n].batch(),
                );
                m.kept_batches.retain(|x| *x != k);
            }
            if !was_held.contains(path) {
                report.held += 1;
            }
        }
        m.lines = m.files.len() as u32;
        write_manifest(&dir, &m)?;
        if !takes.is_empty() {
            if let Err(e) = LineIndex::record(&scope, takes.into_values().collect()) {
                report.notes.push(format!(
                    "The line index was not saved ({e:#}): a later render may not know what the check heard."
                ));
            }
        }
        if report.held > 0 {
            report.notes.push(format!(
                "{} lines the check heard wrong are out of the pack as re-takes: render again with another seed to redo them (priced first), or accept what was heard.",
                report.held
            ));
        }
        report.lines = checked;
        Ok(report)
    }

    /// The person keeps what the check heard for these lines: each passes, and a line the check
    /// took out of the pack goes back in.
    fn voice_accept(&self, id: &str, paths: &[String]) -> Result<VoiceCheckReport> {
        let dir = self.voices_dir().join(id);
        let mut m = read_manifest(&dir)?;
        let mut report = VoiceCheckReport {
            pack: id.to_string(),
            available: true,
            reason: None,
            lines: Vec::new(),
            mismatches: 0,
            held: 0,
            back: 0,
            notes: Vec::new(),
        };
        for p in paths {
            if !m.heard.iter().any(|h| h.path == *p) {
                bail!("The speech check has not heard {p:?} in the pack {id}.");
            }
        }
        let base = Spelling::builtin()?;
        let cache = BatchCache::new(&self.cache);
        let settings = m.settings.clone();
        let scope = Scope {
            cache: &cache,
            provider: PROVIDER,
            voice: &m.voice_id,
            model: &m.model,
            settings: &settings,
        };
        let studio = holds_lines(&m);
        let index = if studio {
            LineIndex::load(&scope)
        } else {
            LineIndex::default()
        };
        let mut takes = Vec::new();
        let wanted: HashSet<&String> = paths.iter().collect();
        let mut back = Vec::new();
        let pack = m.clone();
        for h in m.heard.iter_mut().filter(|h| wanted.contains(&h.path)) {
            h.accepted = !h.ok;
            report.lines.push(h.clone());
            let spoken = pack.spelling_for(&base, &h.path).apply(&h.text);
            // Every take of the line that the check heard this way.
            if studio {
                for e in index.entries() {
                    let hit = |x: &EntryHeard| {
                        !x.ok && x.heard == h.heard && x.spoken.eq_ignore_ascii_case(&spoken)
                    };
                    if e.heard.iter().any(hit) {
                        let mut e = e.clone();
                        for x in e.heard.iter_mut().filter(|x| hit(x)) {
                            x.accepted = true;
                        }
                        takes.push(e);
                    }
                }
            }
            let held = self.held_dir(id).join(&h.path);
            if held.is_file() && !m.files.contains(&h.path) {
                back.push((h.path.clone(), held));
            } else if !m.files.contains(&h.path) {
                report.notes.push(format!(
                    "{}: its file is gone; a render with the same seed puts it back at no cost.",
                    h.path
                ));
            }
        }
        for (path, held) in back {
            let to = dir.join(&path);
            std::fs::create_dir_all(to.parent().unwrap())?;
            std::fs::rename(&held, &to)?;
            m.files.push(path.clone());
            m.retakes
                .retain(|r| !(r.path == path && asr::is_speech_reason(&r.reason)));
        }
        m.files.sort();
        m.lines = m.files.len() as u32;
        write_manifest(&dir, &m)?;
        if !takes.is_empty() {
            if let Err(e) = LineIndex::record(&scope, takes) {
                report.notes.push(format!(
                    "The line index was not saved ({e:#}): a later render may still take these lines out."
                ));
            }
        }
        Ok(report)
    }
}

/// The silence put before and after a file the recognizer heard nothing in.
const PAD_MS: u32 = 500;

impl Core {
    /// What the recognizer hears in each file. A short word with no quiet around it ("Lap",
    /// "no") is often heard as nothing, so a file heard as nothing is heard again with
    /// `PAD_MS` of silence before and after it. Only the copy is padded.
    pub(super) fn hear(&self, files: &[PathBuf], locale: &str) -> Result<intel::Transcripts> {
        let mut t = self.gear.intel.transcribe(files, locale)?;
        if t.unavailable.is_some() || t.heard.len() != files.len() {
            return Ok(t);
        }
        let empty: Vec<usize> = (0..files.len())
            .filter(|&i| {
                let h = &t.heard[i];
                h.error.is_none() && h.text.as_deref().unwrap_or("").trim().is_empty()
            })
            .collect();
        if empty.is_empty() {
            return Ok(t);
        }
        let dir = self.cache.join("voice").join("padded");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let mut padded = Vec::new();
        let mut which = Vec::new();
        for &i in &empty {
            let Ok(p) = std::fs::read(&files[i])
                .map_err(anyhow::Error::from)
                .and_then(|b| wav::read(&b))
            else {
                continue;
            };
            let pad = vec![0i16; (p.rate * PAD_MS / 1000) as usize];
            let out = Pcm {
                rate: p.rate,
                samples: [pad.as_slice(), &p.samples, &pad].concat(),
            };
            let to = dir.join(format!("{i}.wav"));
            std::fs::write(&to, wav::write(&out))?;
            padded.push(to);
            which.push(i);
        }
        let again = self.gear.intel.transcribe(&padded, locale);
        let _ = std::fs::remove_dir_all(&dir);
        let again = again?;
        if again.unavailable.is_some() || again.heard.len() != padded.len() {
            return Ok(t);
        }
        for (i, h) in which.into_iter().zip(again.heard) {
            if h.error.is_none() && !h.text.as_deref().unwrap_or("").trim().is_empty() {
                t.heard[i] = intel::Heard {
                    file: t.heard[i].file.clone(),
                    ..h
                };
            }
        }
        Ok(t)
    }

    /// After a render: the speech check hears the lines rendered into the pack now. A check
    /// that cannot run leaves the report as it is; one that fails adds a note, never an error.
    pub(super) fn check_after_render(&self, id: &str, paths: &[String], report: &mut RenderReport) {
        let Ok(m) = read_manifest(&self.voices_dir().join(id)) else {
            return;
        };
        let paths: Vec<String> = paths
            .iter()
            .filter(|p| m.files.contains(p))
            .cloned()
            .collect();
        // A file held out by an earlier check is stale once the line is in the pack again.
        for p in &paths {
            let _ = std::fs::remove_file(self.held_dir(id).join(p));
        }
        if paths.is_empty() {
            return;
        }
        match self.voice_check_pack(id, &paths) {
            Ok(r) if r.available => {
                if r.mismatches > r.held {
                    report.notes.push(format!(
                        "The speech check heard {} lines wrong: the Voices page shows what it heard.",
                        r.mismatches - r.held
                    ));
                }
                report.heard = r.lines;
                report.notes.extend(r.notes);
            }
            Ok(_) => {}
            Err(e) => report
                .notes
                .push(format!("The speech check did not run: {e:#}")),
        }
    }

    /// After a sample: the speech check hears each sample, and the line index keeps what it
    /// heard, so a later render treats a line heard wrong as a re-take.
    pub(super) fn check_samples(&self, r: &mut SampleReport, settings: &RenderSettings) {
        let files: Vec<PathBuf> = r.items.iter().map(|i| PathBuf::from(&i.file)).collect();
        if files.is_empty() {
            return;
        }
        let t = match self.hear(&files, &intel::locale_of("en")) {
            Ok(t) => t,
            Err(e) => {
                r.warnings
                    .push(format!("The speech check did not run: {e:#}"));
                return;
            }
        };
        if t.unavailable.is_some() {
            return;
        }
        let Ok(spelling) = Spelling::builtin() else {
            return;
        };
        let cache = BatchCache::new(&self.cache);
        let mut takes: HashMap<(String, String), HashMap<usize, Entry>> = HashMap::new();
        let mut indexes: HashMap<(String, String), LineIndex> = HashMap::new();
        for (item, h) in r.items.iter_mut().zip(&t.heard) {
            if h.error.is_some() {
                continue;
            }
            let heard = h.text.clone().unwrap_or_default();
            let spoken = spelling.apply(&item.text);
            let ok = asr::same(&spoken, &heard, &spelling);
            item.heard = Some(heard.clone());
            item.heard_ok = Some(ok);
            let who = (item.voice.clone(), item.model.clone());
            let scope = Scope {
                cache: &cache,
                provider: PROVIDER,
                voice: &who.0,
                model: &who.1,
                settings,
            };
            let index = indexes
                .entry(who.clone())
                .or_insert_with(|| LineIndex::load(&scope));
            let mut accepted = false;
            if let Some(n) = index.take_of(&scope, &spoken) {
                let e = takes
                    .entry(who.clone())
                    .or_default()
                    .entry(n)
                    .or_insert_with(|| index.entries()[n].clone());
                let said = e
                    .items
                    .iter()
                    .find(|i| i.spoken.eq_ignore_ascii_case(&spoken))
                    .map(|i| i.spoken.clone())
                    .unwrap_or(spoken.clone());
                accepted = !ok
                    && e.heard
                        .iter()
                        .any(|x| x.spoken == said && x.accepted && x.heard == heard);
                e.set_heard(EntryHeard {
                    spoken: said,
                    heard: heard.clone(),
                    ok,
                    accepted,
                    cut: batch::CUT_VERSION,
                });
            }
            if !ok && !accepted {
                r.warnings.push(format!(
                    "{}, {}: {}: {}",
                    item.voice_name,
                    item.model,
                    item.line,
                    asr::reason(&heard)
                ));
            }
        }
        for ((voice, model), entries) in takes {
            let scope = Scope {
                cache: &cache,
                provider: PROVIDER,
                voice: &voice,
                model: &model,
                settings,
            };
            let _ = LineIndex::record(&scope, entries.into_values().collect());
        }
    }
}

/// A check report as text.
pub fn check_text(r: &VoiceCheckReport) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    if !r.available {
        let _ = write!(
            s,
            "The speech check is unavailable: {}",
            r.reason.as_deref().unwrap_or("no reason given.")
        );
    } else {
        let _ = write!(
            s,
            "{}: {} lines heard, {} heard wrong",
            r.pack,
            r.lines.len(),
            r.mismatches
        );
        if r.held > 0 {
            let _ = write!(s, ", {} out of the pack as re-takes", r.held);
        }
        if r.back > 0 {
            let _ = write!(s, ", {} heard right now and back in the pack", r.back);
        }
        s.push('.');
        for l in &r.lines {
            let mark = if l.ok {
                "ok"
            } else if l.accepted {
                "accepted"
            } else {
                "WRONG"
            };
            let _ = write!(
                s,
                "\n  {mark:8} {}: {:?} heard {:?}",
                l.path, l.text, l.heard
            );
        }
    }
    for n in &r.notes {
        let _ = write!(s, "\n{n}");
    }
    s
}
