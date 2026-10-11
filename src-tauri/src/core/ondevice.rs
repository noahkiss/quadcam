//! On-device suggestions: clip names, keywords and notes from Apple's on-device model
//! (`crate::intel`, the helper's `suggest`), beside the rules QuadCam applies itself.
//!
//! - **The model** names each clip from its facts (date and time of day, place, aircraft,
//!   length, radio-log moments and flight) and, on macOS 27, its thumbnail; then one name
//!   for the outing.
//! - **The rules** keep a session consistent: clips of one outing (same day, place and
//!   aircraft) share the outing's name and get a pack number in time order, continuing
//!   after the day's numbered clips in the library, in the word the person uses ("pack").
//! - **Never over a person's value:** a field is filled only while it is empty or still
//!   holds the last on-device value. A name the person typed, or an agent suggested, stays.
//!
//! The GUI runs it once after a card is analysed, when the `on_device_suggestions` setting
//! is on; the Review step, the CLI (`names`) and MCP (`quadcam_suggest` with `on_device`)
//! run it on request. A Mac without the model skips it silently.

use super::Core;
use crate::api::{Event, OnDeviceProgress};
use crate::gear::report::SessionReport;
use crate::intel::{ClipAsk, ClipIdea, NamesAsk};
use crate::metadata::{self as md};
use crate::session::{ClipPlan, OnDevice, Session};
use anyhow::{bail, Result};
use chrono::{NaiveDate, NaiveTime, Timelike};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

/// `suggest_on_device`: the clips to ask about (every clip to import when None).
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct OnDeviceParams {
    #[serde(default)]
    pub ids: Option<Vec<usize>>,
}

/// One clip's on-device suggestion, and which fields it filled.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Type)]
pub struct OnDeviceClip {
    pub id: usize,
    pub suggestion: Option<OnDevice>,
    /// The fields it filled: `name`, `note`, `keywords`. A field the person or an agent set
    /// keeps its value.
    pub filled: Vec<String>,
    /// Why this clip has no suggestion.
    pub error: Option<String>,
}

/// `suggest_on_device`'s answer.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Type)]
pub struct OnDeviceReport {
    /// False when this Mac has no on-device model, or the setting is off.
    pub available: bool,
    pub reason: Option<String>,
    pub clips: Vec<OnDeviceClip>,
    /// Seconds the model took.
    pub seconds: f64,
}

/// A clip name ending in a pack number: (the outing's name, the word, the number).
pub fn numbered(name: &str) -> Option<(&str, &str, u32)> {
    let name = name.trim();
    let (head, n) = name.rsplit_once([' ', '-', '_'])?;
    let n: u32 = n.parse().ok()?;
    let (outing, word) = head.rsplit_once([' ', '-', '_'])?;
    let lower = word.to_ascii_lowercase();
    if !["pack", "battery", "lipo", "flight", "run"].contains(&lower.as_str()) {
        return None;
    }
    Some((outing.trim(), word, n))
}

/// The time of day in words, for the model and for names like "early morning cruising".
pub fn day_part(t: NaiveTime) -> &'static str {
    match t.hour() {
        0..=4 => "night",
        5..=9 => "early morning",
        10..=11 => "morning",
        12..=13 => "midday",
        14..=16 => "afternoon",
        17..=19 => "evening",
        _ => "night",
    }
}

/// `m:ss`.
fn mmss(s: f64) -> String {
    crate::gear::report::mmss(s)
}

/// One outing: the clips of a session with the same day, place and aircraft.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Outing {
    date: NaiveDate,
    place: String,
    aircraft: String,
}

/// What the library says about naming: earlier names (newest first, a numbered set once),
/// the pack word the person uses, and the highest pack number per day.
#[derive(Debug, Default)]
pub struct Style {
    pub examples: Vec<String>,
    pub word: Option<String>,
    pub day_max: BTreeMap<NaiveDate, u32>,
}

impl Style {
    /// From library clips as (date, time, name).
    pub fn of(mut clips: Vec<(NaiveDate, Option<String>, String)>) -> Style {
        clips.sort_by(|a, b| (b.0, &b.1).cmp(&(a.0, &a.1)));
        let mut st = Style::default();
        for (date, _, name) in &clips {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let base = match numbered(name) {
                Some((outing, word, n)) => {
                    if st.word.is_none() {
                        st.word = Some(word.to_lowercase());
                    }
                    let m = st.day_max.entry(*date).or_default();
                    *m = (*m).max(n);
                    outing
                }
                None => name,
            };
            let seen = st
                .examples
                .iter()
                .any(|e| numbered(e).map_or(e.as_str(), |x| x.0) == base);
            if !seen && st.examples.len() < 8 {
                st.examples.push(name.to_string());
            }
        }
        st
    }
}

/// The name for clip `n` (1-based) of an outing: its name, the word and the number,
/// joined the way the outing's name joins its words.
pub fn pack_name(outing: &str, word: &str, n: u32) -> String {
    let sep = if !outing.contains(' ') && outing.contains('-') {
        "-"
    } else if !outing.contains(' ') && outing.contains('_') {
        "_"
    } else {
        " "
    };
    format!("{outing}{sep}{word}{sep}{n}")
}

impl Core {
    /// Asks the on-device model for names, keywords and notes for the session's clips, and
    /// fills each empty field (see the module). Skips (`available` false) when the Mac has
    /// no model or the `on_device_suggestions` setting is off. One run at a time.
    pub fn suggest_on_device(&self, p: &OnDeviceParams) -> Result<OnDeviceReport> {
        let off = |why: String| OnDeviceReport {
            available: false,
            reason: Some(why),
            ..Default::default()
        };
        if !self.defaults().on_device_suggestions {
            return Ok(off(
                "On-device suggestions are off (the on_device_suggestions setting).".into(),
            ));
        }
        let feature = self.gear.intel.suggestions();
        if !feature.available {
            return Ok(off(feature.reason.unwrap_or_else(|| {
                "On-device suggestions are unavailable on this Mac.".into()
            })));
        }
        if self.ondevice_running.swap(true, Ordering::SeqCst) {
            bail!("On-device suggestions are already running.");
        }
        struct Running<'a>(&'a Core);
        impl Drop for Running<'_> {
            fn drop(&mut self) {
                self.0.ondevice_running.store(false, Ordering::SeqCst);
            }
        }
        let _r = Running(self);
        let s = self.current()?;
        self.ondevice_run(&s, p)
    }

    /// The helper's on-device suggestions feature: available, frames, or why not.
    pub fn on_device_feature(&self) -> crate::intel::Feature {
        self.gear.intel.suggestions()
    }

    /// The GUI's run after a card is analysed: only with the setting on and the model here;
    /// a failure is dropped.
    pub fn suggest_on_device_auto(&self) {
        // A load runs the radio-log match after the analysis: wait for it, so the model
        // gets the times of day and the moments.
        let until = std::time::Instant::now() + std::time::Duration::from_secs(600);
        while self.busy.load(Ordering::SeqCst) && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        let _ = self.suggest_on_device(&OnDeviceParams::default());
    }

    fn ondevice_run(&self, s: &Session, p: &OnDeviceParams) -> Result<OnDeviceReport> {
        let started = std::time::Instant::now();
        let d = self.defaults();
        let wanted = |id: usize| p.ids.as_ref().is_none_or(|ids| ids.contains(&id));
        // Clips to import, in time order.
        let mut plans: Vec<&ClipPlan> = s
            .plans
            .iter()
            .filter(|pl| !pl.skip)
            .filter(|pl| {
                s.clips.iter().any(|c| {
                    c.id == pl.id
                        && c.part_of.is_none()
                        && c.probe.is_some()
                        && c.stage_error.is_none()
                })
            })
            .collect();
        plans.sort_by_key(|pl| (pl.date, pl.time, pl.id));

        let lib: Vec<(NaiveDate, Option<String>, String)> = self
            .library(&crate::library::Filter::default())
            .map(|v| {
                v.clips
                    .into_iter()
                    .map(|i| (i.clip.date, i.clip.time, i.clip.title))
                    .collect()
            })
            .unwrap_or_default();
        let style = Style::of(lib);
        let word = style.word.clone().unwrap_or_else(|| "pack".into());

        // Outings, each with its clips in time order.
        let mut outings: BTreeMap<Outing, Vec<(&ClipPlan, md::Resolved)>> = BTreeMap::new();
        for pl in &plans {
            let c = s.clips.iter().find(|c| c.id == pl.id).unwrap();
            let r = md::resolve(
                &pl.meta,
                pl.log_model.as_deref(),
                c.kind,
                &d.profiles,
                d.default_profile.as_deref(),
                &d.places,
                &pl.moments,
                c.duration,
                pl.flight.as_ref(),
            );
            let key = Outing {
                date: pl.date,
                place: r
                    .location
                    .as_ref()
                    .and_then(|l| l.name.clone())
                    .unwrap_or_default(),
                aircraft: r.profile.clone().unwrap_or_default(),
            };
            outings.entry(key).or_default().push((pl, r));
        }

        let total: usize = outings
            .values()
            .map(|v| v.iter().filter(|(pl, _)| wanted(pl.id)).count())
            .sum();
        let mut done = 0;
        self.ondevice_progress(true, done, total);
        let mut report = OnDeviceReport {
            available: true,
            ..Default::default()
        };
        let mut suggestions: Vec<(usize, OnDevice)> = Vec::new();
        for (key, members) in &outings {
            let ask: Vec<ClipAsk> = members
                .iter()
                .filter(|(pl, _)| wanted(pl.id))
                .map(|(pl, r)| {
                    let c = s.clips.iter().find(|c| c.id == pl.id).unwrap();
                    ClipAsk {
                        id: pl.id,
                        facts: facts(pl, r, c.duration),
                        frames: c.thumb.iter().filter(|t| t.is_file()).cloned().collect(),
                    }
                })
                .collect();
            if ask.is_empty() {
                continue;
            }
            let ideas = match self.gear.intel.suggest_names(&NamesAsk {
                examples: style.examples.clone(),
                clips: ask.clone(),
            }) {
                Ok(i) => i,
                Err(e) => {
                    for a in &ask {
                        report.clips.push(OnDeviceClip {
                            id: a.id,
                            error: Some(format!("{e:#}")),
                            ..Default::default()
                        });
                    }
                    continue;
                }
            };
            if let Some(why) = ideas.unavailable {
                self.ondevice_progress(false, total, total);
                return Ok(OnDeviceReport {
                    available: false,
                    reason: Some(why),
                    ..Default::default()
                });
            }
            // The rule: an outing of two or more clips shares one name and numbers its packs
            // in time order, after the day's numbered clips in the library.
            let first = style.day_max.get(&key.date).copied().unwrap_or(0);
            let shared = ideas.group.filter(|_| members.len() > 1);
            for (i, (pl, r)) in members.iter().enumerate() {
                if !wanted(pl.id) {
                    continue;
                }
                let idea = ideas.clips.iter().find(|x| x.id == pl.id);
                let own = idea.and_then(|x| x.name.clone()).filter(|n| !n.is_empty());
                let name = match (&shared, &own) {
                    (Some(g), _) => pack_name(g, &word, first + i as u32 + 1),
                    (None, Some(o)) => o.clone(),
                    (None, None) => {
                        report.clips.push(OnDeviceClip {
                            id: pl.id,
                            error: Some(
                                idea.and_then(|x| x.error.clone())
                                    .unwrap_or_else(|| "The model gave no name.".into()),
                            ),
                            ..Default::default()
                        });
                        continue;
                    }
                };
                let od = OnDevice {
                    name,
                    words: own.unwrap_or_default(),
                    keywords: new_keywords(idea, r),
                    note: idea.and_then(|x| x.note.clone()).unwrap_or_default(),
                    images: idea.map_or(0, |x| x.images),
                };
                suggestions.push((pl.id, od));
            }
            done += ask.len();
            self.ondevice_progress(true, done, total);
        }

        let filled = self.ondevice_apply(s, &suggestions)?;
        for (id, od) in suggestions {
            report.clips.push(OnDeviceClip {
                id,
                suggestion: Some(od),
                filled: filled.get(&id).cloned().unwrap_or_default(),
                error: None,
            });
        }
        report.clips.sort_by_key(|c| c.id);
        report.seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
        self.ondevice_progress(false, total, total);
        Ok(report)
    }

    /// Writes the suggestions into the session the run started from, if it is still loaded:
    /// each field only while it is empty or holds the last on-device value.
    fn ondevice_apply(
        &self,
        from: &Session,
        suggestions: &[(usize, OnDevice)],
    ) -> Result<BTreeMap<usize, Vec<String>>> {
        let mut filled = BTreeMap::new();
        let mut guard = self.session.lock().unwrap();
        let Some(s) = guard.as_mut() else {
            return Ok(filled);
        };
        if s.staging != from.staging || s.clips.len() != from.clips.len() {
            return Ok(filled);
        }
        let mut next = s.clone();
        for (id, od) in suggestions {
            if next.results.iter().any(|r| r.id == *id) {
                continue;
            }
            let Ok(p) = next.plan_mut(*id) else { continue };
            let last = p.ondevice.take().unwrap_or_default();
            let mut f = Vec::new();
            let ours = |value: &str, by_us: bool, last: &str| {
                value.trim().is_empty() || (by_us && !last.is_empty() && value == last)
            };
            if !od.name.is_empty() && ours(&p.name, p.suggested.name, &last.name) {
                p.name = od.name.clone();
                p.suggested.name = true;
                f.push("name".to_string());
            }
            if !od.note.is_empty() && ours(&p.note, p.suggested.note, &last.note) {
                p.note = od.note.clone();
                p.suggested.note = true;
                f.push("note".to_string());
            }
            let kw_ours = p.meta.keywords.is_empty()
                || (p.suggested.meta
                    && !last.keywords.is_empty()
                    && p.meta.keywords == last.keywords);
            if !od.keywords.is_empty() && kw_ours {
                p.meta.keywords = od.keywords.clone();
                p.suggested.meta = true;
                f.push("keywords".to_string());
            }
            p.ondevice = Some(od.clone());
            filled.insert(*id, f);
        }
        *s = next.clone();
        drop(guard);
        self.commit(Some(next))?;
        Ok(filled)
    }

    /// Adds the on-device model's paragraph about the report's days (`summary`), or why
    /// there is none (`summary_unavailable`), and puts it into the Markdown.
    pub(super) fn summarize_day(&self, r: &mut SessionReport) {
        let why = if !self.defaults().on_device_suggestions {
            Some("On-device suggestions are off (the on_device_suggestions setting).".to_string())
        } else {
            let f = self.gear.intel.suggestions();
            (!f.available).then(|| {
                f.reason
                    .unwrap_or_else(|| "On-device suggestions are unavailable on this Mac.".into())
            })
        };
        if let Some(why) = why {
            r.summary_unavailable = Some(why);
            return;
        }
        let clips: Vec<crate::library::LibClip> = r
            .days
            .iter()
            .flat_map(|d| {
                self.library(&crate::library::Filter {
                    day: Some(*d),
                    ..Default::default()
                })
                .map(|v| v.clips.into_iter().map(|i| i.clip).collect::<Vec<_>>())
                .unwrap_or_default()
            })
            .collect();
        match self.gear.intel.summarize(&day_facts(r, &clips)) {
            Ok(Ok(t)) => r.summary = Some(t),
            Ok(Err(why)) => r.summary_unavailable = Some(why),
            Err(e) => r.summary_unavailable = Some(format!("{e:#}")),
        }
        r.markdown = crate::gear::report::markdown(r);
    }

    fn ondevice_progress(&self, running: bool, done: usize, total: usize) {
        self.hooks.event(Event::OnDeviceProgress(OnDeviceProgress {
            running,
            done,
            total,
        }));
    }
}

/// The model's keywords without the ones QuadCam adds anyway (FPV, the aircraft's, the
/// place, the moments), at most five.
fn new_keywords(idea: Option<&ClipIdea>, r: &md::Resolved) -> Vec<String> {
    let place = r
        .location
        .as_ref()
        .and_then(|l| l.name.clone())
        .unwrap_or_default();
    let have = |k: &str| {
        r.keywords.iter().any(|x| x.eq_ignore_ascii_case(k))
            || k.eq_ignore_ascii_case(&place)
            || k.eq_ignore_ascii_case(&r.aircraft)
            || r.profile
                .as_deref()
                .is_some_and(|p| k.eq_ignore_ascii_case(p))
    };
    let words: Vec<&str> = idea
        .map(|i| {
            i.keywords
                .iter()
                .map(String::as_str)
                .filter(|k| !have(k))
                .collect()
        })
        .unwrap_or_default();
    md::clean_keywords(words).into_iter().take(5).collect()
}

/// The facts the model gets about a flying day, one per line.
fn day_facts(r: &SessionReport, clips: &[crate::library::LibClip]) -> String {
    let mut f = Vec::new();
    let days: Vec<String> = r
        .days
        .iter()
        .map(|d| d.format("%A %Y-%m-%d").to_string())
        .collect();
    f.push(format!("Day: {}", days.join(", ")));
    let mut places: Vec<&str> = clips.iter().filter_map(|c| c.place.as_deref()).collect();
    places.sort();
    places.dedup();
    if !places.is_empty() {
        f.push(format!("Places: {}", places.join(", ")));
    }
    let mut aircraft: Vec<&str> = clips.iter().filter_map(|c| c.aircraft.as_deref()).collect();
    aircraft.sort();
    aircraft.dedup();
    if !aircraft.is_empty() {
        f.push(format!("Aircraft: {}", aircraft.join(", ")));
    }
    if !clips.is_empty() {
        let secs: f64 = clips.iter().map(|c| c.duration).sum();
        f.push(format!("Clips: {}, {} of video", clips.len(), mmss(secs)));
        let mut times: Vec<&str> = clips.iter().filter_map(|c| c.time.as_deref()).collect();
        times.sort();
        if let (Some(a), Some(b)) = (times.first(), times.last()) {
            f.push(format!("First clip {a}, last clip {b}"));
        }
        let names: Vec<&str> = clips
            .iter()
            .map(|c| c.title.as_str())
            .filter(|t| !t.is_empty())
            .take(12)
            .collect();
        if !names.is_empty() {
            f.push(format!("Clip names: {}", names.join("; ")));
        }
        let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
        for m in clips.iter().flat_map(|c| &c.moments) {
            if m.kind != crate::moments::MomentKind::DeadAir && m.score >= 0.5 {
                *kinds.entry(m.kind.label()).or_default() += 1;
            }
        }
        if !kinds.is_empty() {
            let k: Vec<String> = kinds.iter().map(|(k, n)| format!("{n} {k}")).collect();
            f.push(format!("Moments: {}", k.join(", ")));
        }
    }
    if r.flights > 0 {
        f.push(format!(
            "Radio-log flights: {}, air time {}",
            r.flights,
            mmss(r.air_s)
        ));
    }
    if let Some(l) = &r.longest {
        f.push(format!("Longest flight: {}", mmss(l.secs)));
    }
    if r.dropouts > 0 {
        f.push(format!("Link dropouts: {}", r.dropouts));
    }
    if !r.packs.is_empty() {
        f.push(format!("Battery packs used: {}", r.packs.len()));
    }
    f.push(format!("Crashes logged: {}", r.crashes.len()));
    f.join("\n")
}

/// The facts the model gets about one clip, one per line.
fn facts(pl: &ClipPlan, r: &md::Resolved, duration: f64) -> String {
    let mut f = Vec::new();
    match pl.time {
        Some(t) if pl.source != crate::pipeline::DateSource::Import => f.push(format!(
            "Date: {}, {} ({})",
            pl.date,
            t.format("%H:%M"),
            day_part(t)
        )),
        _ => f.push(format!("Date: {}", pl.date)),
    }
    if let Some(n) = r.location.as_ref().and_then(|l| l.name.as_deref()) {
        f.push(format!("Place: {n}"));
    }
    match (r.profile.as_deref(), r.aircraft.trim()) {
        (Some(p), a) if !a.is_empty() && !a.eq_ignore_ascii_case(p) => {
            f.push(format!("Aircraft: {p} ({a})"))
        }
        (Some(p), _) => f.push(format!("Aircraft: {p}")),
        (None, a) if !a.is_empty() => f.push(format!("Aircraft: {a}")),
        _ => {}
    }
    if !r.video_system.trim().is_empty() {
        f.push(format!("Video: {}", r.video_system.trim()));
    }
    f.push(format!("Length: {}", mmss(duration)));
    let moments: Vec<String> = pl
        .moments
        .iter()
        .filter(|m| m.kind != crate::moments::MomentKind::DeadAir && m.score >= 0.5)
        .filter(|m| m.start >= 0.0 && m.start < duration)
        .take(8)
        .map(|m| format!("{} at {}", m.kind.label(), mmss(m.start)))
        .collect();
    if !moments.is_empty() {
        f.push(format!("Moments: {}", moments.join(", ")));
    }
    if let Some(st) = &pl.flight {
        f.push(format!("Flight: {}", st.line()));
    }
    f.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn numbered_names_are_read() {
        assert_eq!(
            numbered("early morning cruising pack 3"),
            Some(("early morning cruising", "pack", 3))
        );
        assert_eq!(numbered("farm-fun-pack-2"), Some(("farm-fun", "pack", 2)));
        assert_eq!(numbered("farm-bike-chase"), None);
        assert_eq!(numbered("meteor75 first o4 flight"), None);
        assert_eq!(numbered("pack 2"), None);
    }

    #[test]
    fn the_library_gives_examples_the_word_and_the_days_last_pack() {
        let st = Style::of(vec![
            (
                d("2026-10-05"),
                Some("18:36".into()),
                "meteor75 first o4 flight".into(),
            ),
            (
                d("2026-10-07"),
                Some("09:21".into()),
                "early morning cruising pack 1".into(),
            ),
            (
                d("2026-10-07"),
                Some("09:25".into()),
                "early morning cruising pack 2".into(),
            ),
            (d("2026-09-26"), None, "farm-bike-chase".into()),
            (d("2026-09-26"), None, "".into()),
        ]);
        // Newest first; a numbered set counts once.
        assert_eq!(
            st.examples,
            [
                "early morning cruising pack 2",
                "meteor75 first o4 flight",
                "farm-bike-chase"
            ]
        );
        assert_eq!(st.word.as_deref(), Some("pack"));
        assert_eq!(st.day_max.get(&d("2026-10-07")), Some(&2));
        assert_eq!(st.day_max.get(&d("2026-10-05")), None);
    }

    #[test]
    fn pack_names_follow_the_outings_separator() {
        assert_eq!(
            pack_name("morning loops", "pack", 2),
            "morning loops pack 2"
        );
        assert_eq!(pack_name("farm-fun", "pack", 1), "farm-fun-pack-1");
        assert_eq!(pack_name("park_rips", "battery", 4), "park_rips_battery_4");
    }

    #[test]
    fn times_of_day_read_as_words() {
        let t = |h| NaiveTime::from_hms_opt(h, 0, 0).unwrap();
        assert_eq!(day_part(t(9)), "early morning");
        assert_eq!(day_part(t(10)), "morning");
        assert_eq!(day_part(t(15)), "afternoon");
        assert_eq!(day_part(t(18)), "evening");
        assert_eq!(day_part(t(23)), "night");
    }
}
