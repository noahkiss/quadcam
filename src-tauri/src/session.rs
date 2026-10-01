//! One import session: the clips staged from a card or folder, the per-clip plan (date,
//! name, note, skip) and the import results. The GUI, the CLI and the MCP server all work on
//! this one type, and it saves to JSON so separate CLI runs can continue a session.

use crate::disk::{self, CardIdentity, Volume};
use crate::logs::{Badge, Tunables};
use crate::media::Tools;
use crate::metadata::{self as md, ClipMeta, FlightStats, Location};
use crate::moments::{Moment, Span};
use crate::pipeline::{self, Clip, ClipJob, ClipResult, ClipStatus, DateSource, Outcome};
use anyhow::{bail, Context, Result};
use chrono::{NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SESSION_VERSION: u32 = 1;

pub use crate::pipeline::import::run_import;
pub use crate::settings::Defaults;
pub use crate::trim::{MAX_CUTS, MIN_CUT_S};

/// Which fields of a plan an agent wrote and the user has not edited since.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Suggested {
    pub date: bool,
    pub name: bool,
    pub note: bool,
    pub skip: bool,
    #[serde(default)]
    pub cuts: bool,
    #[serde(default)]
    pub meta: bool,
}

impl Suggested {
    pub fn any(&self) -> bool {
        self.date || self.name || self.note || self.skip || self.cuts || self.meta
    }
}

/// What will happen to one clip on import. The GUI shows it and edits it; an agent may
/// suggest values for it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClipPlan {
    pub id: usize,
    pub skip: bool,
    pub date: NaiveDate,
    /// The time of day: from the radio log, or set by hand. None means local noon.
    pub time: Option<NaiveTime>,
    pub source: DateSource,
    pub badge: Badge,
    pub segments: usize,
    pub name: String,
    pub note: String,
    #[serde(default)]
    pub suggested: Suggested,
    /// Why the agent suggested what it did, shown next to the suggestion.
    #[serde(default)]
    pub reason: Option<String>,
    /// Moments from the radio log, in clip seconds (log time plus `log_offset_s`).
    #[serde(default)]
    pub moments: Vec<Moment>,
    /// Median seconds between the log rows the clip claimed. 0.5 s logs give rough moments.
    #[serde(default)]
    pub log_interval_s: Option<f64>,
    /// Seconds into the clip where the first armed log row falls. 0 assumes the clip starts
    /// at arm; the DVR usually starts earlier, so the person can set it.
    #[serde(default)]
    pub log_offset_s: f64,
    /// Ranges to export as extra files (`<name>_cutN`), in clip seconds.
    #[serde(default)]
    pub cuts: Vec<Span>,
    /// Location, profile, keywords and author for this clip.
    #[serde(default)]
    pub meta: ClipMeta,
    /// The EdgeTX model of the matched log; it picks the profile when the clip has none.
    #[serde(default)]
    pub log_model: Option<String>,
    #[serde(default)]
    pub flight: Option<FlightStats>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Editor {
    User,
    Agent,
}

/// A change to one clip's plan. Missing fields stay as they are.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PlanPatch {
    pub id: usize,
    #[serde(default)]
    pub date: Option<NaiveDate>,
    /// Time of day, `HH:MM` (or `HH:MM:SS`); empty removes it (local noon).
    #[serde(default)]
    pub time: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub skip: Option<bool>,
    #[serde(default)]
    pub reason: Option<String>,
    /// Replaces the clip's cut list. An empty list removes every cut.
    #[serde(default)]
    pub cuts: Option<Vec<Span>>,
    /// Where the first armed log row falls in the clip, in seconds. Moves the log moments.
    #[serde(default)]
    pub log_offset_s: Option<f64>,
    /// Aircraft profile name; empty to fall back to the log's model or the default.
    #[serde(default)]
    pub profile: Option<String>,
    /// Location in decimal degrees.
    #[serde(default)]
    pub location: Option<Location>,
    /// A saved place's name (`Core::patch` turns it into `location`); empty removes the
    /// clip's location.
    #[serde(default)]
    pub place: Option<String>,
    /// Replaces the clip's own keywords.
    #[serde(default)]
    pub keywords: Option<Vec<String>>,
    /// Empty falls back to the profile's author.
    #[serde(default)]
    pub author: Option<String>,
    /// What happens to the files of exported cuts that `cuts` drops. Required when it
    /// drops one.
    #[serde(default)]
    pub removed_cuts: Option<crate::trim::RemovedCuts>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    /// Card mount point or folder the clips came from.
    pub source: PathBuf,
    /// The card the clips were read from; None for a plain folder (no format step).
    pub card: Option<CardIdentity>,
    pub card_volume: Option<Volume>,
    pub staging: PathBuf,
    pub clips: Vec<Clip>,
    pub plans: Vec<ClipPlan>,
    pub results: Vec<ClipResult>,
    pub analysed: bool,
    pub log_dir: Option<PathBuf>,
    pub log_day: Option<NaiveDate>,
    pub log_days: Vec<NaiveDate>,
    pub warnings: Vec<String>,
    pub date_warnings: Vec<String>,
    /// Clip ids whose outputs were added to Photos.
    pub in_photos: Vec<usize>,
    /// Output folder of the last import.
    pub output_dir: Option<PathBuf>,
}

/// A time of day from `HH:MM` or `HH:MM:SS`; empty is None (local noon).
pub fn parse_time(s: &str) -> Result<Option<NaiveTime>> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    NaiveTime::parse_from_str(s, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(s, "%H:%M:%S"))
        .map(Some)
        .with_context(|| format!("time {s:?} is not HH:MM (24-hour)"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub results: Vec<ClipResult>,
    pub imported: usize,
    pub skipped: usize,
    pub failed: usize,
    pub total_bytes: u64,
    pub output_dir: Option<PathBuf>,
    /// Ok when the format step may unlock, else the reason it stays locked.
    pub format_ready: Result<(), String>,
}

impl Session {
    /// Copies every clip off `source` into `staging_root/<key>`.
    pub fn stage(
        source: &Path,
        staging_root: &Path,
        on_progress: &mut dyn FnMut(usize, usize, u64, u64),
    ) -> Result<Session> {
        if !source.is_dir() {
            bail!("{} is not a folder or mounted volume.", source.display());
        }
        let vol = disk::probe_volume(source).filter(|v| v.is_card);
        let key = vol
            .as_ref()
            .and_then(|v| v.info.volume_uuid.clone())
            .unwrap_or_else(|| chrono::Local::now().format("%Y%m%d-%H%M%S").to_string());
        let staging = staging_root.join(key);
        let clips = pipeline::stage(source, &staging, on_progress)?;
        Ok(Session::staged(source, vol, staging, clips))
    }

    /// Copies the clips among `files` (files dropped on the window) into staging. The
    /// session's source is the folder of the first clip; it is never a card.
    pub fn stage_files(
        files: &[PathBuf],
        staging_root: &Path,
        on_progress: &mut dyn FnMut(usize, usize, u64, u64),
    ) -> Result<Session> {
        let found = crate::scan::clips_in(files);
        let Some(source) = found
            .first()
            .and_then(|f| f.path.parent())
            .map(Path::to_path_buf)
        else {
            bail!("None of these files is a DVR clip (AVI).");
        };
        let staging = staging_root.join(chrono::Local::now().format("%Y%m%d-%H%M%S").to_string());
        let clips = pipeline::stage_found(found, &staging, on_progress)?;
        Ok(Session::staged(&source, None, staging, clips))
    }

    /// A new session for clips just staged: every clip planned for today, unnamed.
    fn staged(source: &Path, vol: Option<Volume>, staging: PathBuf, clips: Vec<Clip>) -> Session {
        let today = chrono::Local::now().date_naive();
        let plans = clips
            .iter()
            .map(|c| ClipPlan {
                id: c.id,
                skip: c.stage_error.is_some(),
                date: today,
                time: None,
                source: DateSource::Import,
                badge: Badge::Unmatched,
                segments: 0,
                name: String::new(),
                note: String::new(),
                suggested: Suggested::default(),
                reason: None,
                moments: Vec::new(),
                log_interval_s: None,
                log_offset_s: 0.0,
                cuts: Vec::new(),
                meta: ClipMeta::default(),
                log_model: None,
                flight: None,
            })
            .collect();
        let mut warnings = vol.as_ref().map(|v| v.warnings.clone()).unwrap_or_default();
        let missing: Vec<&str> = clips
            .iter()
            .filter(|c| c.stage_error.is_some())
            .map(|c| c.rel.as_str())
            .collect();
        if !missing.is_empty() && vol.is_some() {
            warnings.push(format!(
                "Card pulled or unreadable. Not copied: {}. What staged is kept; the format step stays locked.",
                missing.join(", ")
            ));
        } else if !missing.is_empty() {
            warnings.push(format!("Unreadable. Not copied: {}.", missing.join(", ")));
        }
        Session {
            version: SESSION_VERSION,
            source: source.to_path_buf(),
            card: vol.as_ref().map(|v| CardIdentity::from_info(&v.info)),
            card_volume: vol,
            staging,
            clips,
            plans,
            results: Vec::new(),
            analysed: false,
            log_dir: None,
            log_day: None,
            log_days: Vec::new(),
            warnings,
            date_warnings: Vec::new(),
            in_photos: Vec::new(),
            output_dir: None,
        }
    }

    /// Probes, recovers half-written clips and makes thumbnails. Empty clips get skipped.
    pub fn analyse(
        &mut self,
        tools: &Tools,
        thumbs: &Path,
        on_progress: &mut dyn FnMut(usize, usize),
    ) -> Result<()> {
        std::fs::create_dir_all(thumbs)?;
        let total = self.clips.len();
        for (i, c) in self.clips.iter_mut().enumerate() {
            on_progress(i, total);
            if let Err(e) = pipeline::analyse(tools, c, thumbs) {
                c.detail = format!("analysis failed: {e:#}");
            }
            if c.status == ClipStatus::Empty {
                self.plans[i].skip = true;
            }
        }
        self.analysed = true;
        Ok(())
    }

    /// Suggests dates from radio logs; plans the user or an agent edited keep their date.
    pub fn plan_dates(
        &mut self,
        log_dir: Option<&Path>,
        day: Option<NaiveDate>,
        tun: &Tunables,
        today: NaiveDate,
    ) {
        let durations: Vec<f64> = self.clips.iter().map(|c| c.duration).collect();
        let plan = pipeline::plan_dates(&durations, log_dir, day, today, tun);
        for (p, s) in self.plans.iter_mut().zip(plan.suggestions) {
            p.moments = s
                .moments
                .iter()
                .map(|m| m.shifted(p.log_offset_s))
                .collect();
            p.log_interval_s = s.log_interval_s;
            p.log_model = s.log_model.clone();
            p.flight = s.flight.clone();
            if p.source == DateSource::Edited {
                continue;
            }
            p.date = s.date;
            p.time = s.time;
            p.source = s.source;
            p.badge = s.badge;
            p.segments = s.segments;
        }
        self.log_dir = log_dir.map(Path::to_path_buf);
        self.log_day = plan.day_used;
        self.log_days = plan.log_days;
        self.date_warnings = plan.warnings;
    }

    pub fn plan_mut(&mut self, id: usize) -> Result<&mut ClipPlan> {
        self.plans
            .iter_mut()
            .find(|p| p.id == id)
            .with_context(|| format!("no clip with id {id}"))
    }

    /// Applies edits. An agent's edits are marked suggested; a user's edits clear the mark
    /// on the fields they touch. A new date always becomes an edited date, at local noon
    /// unless the patch also sets a time. A time alone keeps the date and makes it edited.
    pub fn patch(&mut self, patches: &[PlanPatch], editor: Editor) -> Result<()> {
        for patch in patches {
            let (unusable, duration) = {
                let c = self
                    .clips
                    .iter()
                    .find(|c| c.id == patch.id)
                    .with_context(|| format!("no clip with id {}", patch.id))?;
                (
                    c.status == ClipStatus::Empty || c.stage_error.is_some(),
                    c.duration,
                )
            };
            let cuts = patch
                .cuts
                .as_deref()
                .map(|c| crate::trim::check_cuts(&format!("clip {}", patch.id), c, duration))
                .transpose()?;
            if cuts.as_ref().is_some_and(|c| !c.is_empty()) && unusable {
                bail!(
                    "clip {} is empty or was not copied; it cannot be cut",
                    patch.id
                );
            }
            if let Some(o) = patch.log_offset_s {
                if !o.is_finite() || o.abs() > duration.max(1.0) {
                    bail!("log offset {o} s is outside clip {}", patch.id);
                }
            }
            let time = patch
                .time
                .as_deref()
                .map(|t| parse_time(t).with_context(|| format!("clip {}", patch.id)))
                .transpose()?;
            let agent = editor == Editor::Agent;
            let p = self.plan_mut(patch.id)?;
            if let Some(d) = patch.date {
                p.date = d;
                p.time = None;
                p.source = DateSource::Edited;
                p.badge = Badge::Unmatched;
                p.suggested.date = agent;
            }
            if let Some(t) = time {
                p.time = t;
                p.source = DateSource::Edited;
                p.suggested.date = agent;
            }
            if let Some(n) = &patch.name {
                p.name = n.trim().to_string();
                p.suggested.name = agent;
            }
            if let Some(n) = &patch.note {
                p.note = n.trim().to_string();
                p.suggested.note = agent;
            }
            if let Some(s) = patch.skip {
                if !s && unusable {
                    bail!(
                        "clip {} is empty or was not copied; it cannot be imported",
                        patch.id
                    );
                }
                p.skip = s;
                p.suggested.skip = agent;
            }
            if let Some(c) = cuts {
                p.cuts = c;
                p.suggested.cuts = agent;
            }
            let mut meta_changed = false;
            if let Some(n) = &patch.profile {
                p.meta.profile = Some(n.trim().to_string()).filter(|n| !n.is_empty());
                meta_changed = true;
            }
            if let Some(l) = &patch.location {
                l.check()?;
                p.meta.location = Some(l.clone());
                meta_changed = true;
            } else if patch.place.as_deref().is_some_and(|x| x.trim().is_empty()) {
                p.meta.location = None;
                meta_changed = true;
            }
            if let Some(k) = &patch.keywords {
                p.meta.keywords = md::clean_keywords(k.iter().map(String::as_str));
                meta_changed = true;
            }
            if let Some(a) = &patch.author {
                p.meta.author = Some(a.trim().to_string()).filter(|a| !a.is_empty());
                meta_changed = true;
            }
            if meta_changed {
                p.suggested.meta = agent;
            }
            if let Some(o) = patch.log_offset_s {
                let by = o - p.log_offset_s;
                p.moments = p.moments.iter().map(|m| m.shifted(by)).collect();
                p.log_offset_s = o;
            }
            if agent {
                if let Some(r) = &patch.reason {
                    p.reason = Some(r.trim().to_string()).filter(|r| !r.is_empty());
                }
            } else if !p.suggested.any() {
                p.reason = None;
            }
        }
        Ok(())
    }

    pub fn jobs(&self) -> Vec<ClipJob> {
        self.plans
            .iter()
            .map(|p| ClipJob {
                id: p.id,
                skip: p.skip,
                date: p.date.format("%Y-%m-%d").to_string(),
                time: match p.source {
                    DateSource::Import => None,
                    _ => p.time.map(|t| t.format("%H:%M:%S").to_string()),
                },
                source: p.source,
                name: p.name.clone(),
                note: p.note.clone(),
                meta: md::Resolved::default(),
                extra: Vec::new(),
            })
            .collect()
    }

    pub fn format_ready(&self) -> Result<()> {
        if self.card.is_none() {
            bail!("Clips came from a folder, not a card.");
        }
        pipeline::can_format(&self.clips, &self.results)
    }

    pub fn summary(&self) -> Summary {
        let count = |o| self.results.iter().filter(|r| r.outcome == o).count();
        Summary {
            imported: count(Outcome::Verified),
            skipped: count(Outcome::Skipped),
            failed: count(Outcome::Failed),
            total_bytes: self
                .results
                .iter()
                .map(|r| r.size + r.cuts.iter().map(|c| c.size).sum::<u64>())
                .sum(),
            output_dir: self.output_dir.clone(),
            results: self.results.clone(),
            format_ready: self.format_ready().map_err(|e| format!("{e:#}")),
        }
    }

    /// Outputs that verified in this session, cuts included, for the given clips (all when
    /// `ids` is None).
    pub fn verified_outputs(&self, ids: Option<&[usize]>) -> Vec<(usize, PathBuf)> {
        self.results
            .iter()
            .filter(|r| r.outcome == Outcome::Verified)
            .filter(|r| ids.is_none_or(|ids| ids.contains(&r.id)))
            .flat_map(|r| {
                r.output
                    .iter()
                    .cloned()
                    .chain(
                        r.cuts
                            .iter()
                            .filter(|c| c.outcome == Outcome::Verified)
                            .filter_map(|c| c.output.clone()),
                    )
                    .map(move |o| (r.id, o))
            })
            .collect()
    }

    /// True when every staged clip is still in the staging folder and at least one exists.
    pub fn staged_files_exist(&self) -> bool {
        let staged: Vec<&Path> = self
            .clips
            .iter()
            .filter_map(|c| c.staged.as_deref())
            .collect();
        !staged.is_empty() && staged.iter().all(|p| p.is_file())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Session> {
        let s: Session = serde_json::from_slice(
            &std::fs::read(path).with_context(|| format!("reading {}", path.display()))?,
        )
        .with_context(|| format!("parsing {}", path.display()))?;
        if s.version != SESSION_VERSION {
            bail!(
                "{} is a session file from another quadcam version",
                path.display()
            );
        }
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let d = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        let clip = |id: usize, status| Clip {
            id,
            name: format!("PICT000{id}.AVI"),
            rel: format!("DCIM/PICT000{id}.AVI"),
            card_path: PathBuf::new(),
            size: 1,
            staged: None,
            stage_error: None,
            status,
            duration: 5.0,
            recovered: None,
            probe: None,
            thumb: None,
            detail: String::new(),
            signal: None,
            key: String::new(),
        };
        let plan = |id| ClipPlan {
            id,
            skip: false,
            date: d,
            time: None,
            source: DateSource::Import,
            badge: Badge::Unmatched,
            segments: 0,
            name: String::new(),
            note: String::new(),
            suggested: Suggested::default(),
            reason: None,
            moments: Vec::new(),
            log_interval_s: None,
            log_offset_s: 0.0,
            cuts: Vec::new(),
            meta: ClipMeta::default(),
            log_model: None,
            flight: None,
        };
        Session {
            version: SESSION_VERSION,
            source: PathBuf::from("/tmp/x"),
            card: None,
            card_volume: None,
            staging: PathBuf::new(),
            clips: vec![clip(0, ClipStatus::Ok), clip(1, ClipStatus::Empty)],
            plans: vec![plan(0), plan(1)],
            results: Vec::new(),
            analysed: true,
            log_dir: None,
            log_day: None,
            log_days: Vec::new(),
            warnings: Vec::new(),
            date_warnings: Vec::new(),
            in_photos: Vec::new(),
            output_dir: None,
        }
    }

    #[test]
    fn agent_suggestions_are_marked_and_user_edits_clear_them() {
        let mut s = session();
        let d = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        s.patch(
            &[PlanPatch {
                id: 0,
                name: Some(" Backyard Loops ".into()),
                date: Some(d),
                reason: Some("OSD clock".into()),
                ..Default::default()
            }],
            Editor::Agent,
        )
        .unwrap();
        let p = &s.plans[0];
        assert_eq!(p.name, "Backyard Loops");
        assert_eq!(p.source, DateSource::Edited);
        assert!(p.suggested.name && p.suggested.date && !p.suggested.note);
        assert_eq!(p.reason.as_deref(), Some("OSD clock"));

        s.patch(
            &[PlanPatch {
                id: 0,
                name: Some("loops".into()),
                ..Default::default()
            }],
            Editor::User,
        )
        .unwrap();
        assert!(!s.plans[0].suggested.name && s.plans[0].suggested.date);
        s.patch(
            &[PlanPatch {
                id: 0,
                date: Some(d),
                ..Default::default()
            }],
            Editor::User,
        )
        .unwrap();
        assert!(!s.plans[0].suggested.any());
        assert_eq!(s.plans[0].reason, None);

        // An empty clip cannot be un-skipped; an unknown id is an error.
        assert!(s
            .patch(
                &[PlanPatch {
                    id: 1,
                    skip: Some(false),
                    ..Default::default()
                }],
                Editor::Agent
            )
            .is_err());
        assert!(s
            .patch(
                &[PlanPatch {
                    id: 9,
                    skip: Some(true),
                    ..Default::default()
                }],
                Editor::Agent
            )
            .is_err());
    }

    #[test]
    fn cuts_and_log_offset() {
        let mut s = session();
        s.plans[0].moments = vec![Moment {
            kind: crate::moments::MomentKind::Roll,
            start: 1.0,
            end: 1.5,
            score: 0.8,
            source: crate::moments::Source::RadioLog,
            detail: String::new(),
        }];
        let cut = |a, b| Span { start: a, end: b };
        s.patch(
            &[PlanPatch {
                id: 0,
                cuts: Some(vec![cut(3.0, 9.0), cut(0.5, 1.5004)]),
                log_offset_s: Some(2.0),
                ..Default::default()
            }],
            Editor::Agent,
        )
        .unwrap();
        let p = &s.plans[0];
        // Sorted, clamped to the 5 s clip, rounded to ms.
        assert_eq!(p.cuts, vec![cut(0.5, 1.5), cut(3.0, 5.0)]);
        assert!(p.suggested.cuts);
        assert_eq!(p.moments[0].start, 3.0, "the offset moves the moments");
        s.patch(
            &[PlanPatch {
                id: 0,
                log_offset_s: Some(0.5),
                ..Default::default()
            }],
            Editor::User,
        )
        .unwrap();
        assert_eq!(s.plans[0].moments[0].start, 1.5);
        for bad in [
            vec![cut(2.0, 2.2)],
            vec![cut(6.0, 9.0)],
            vec![cut(f64::NAN, 1.0)],
        ] {
            assert!(s
                .patch(
                    &[PlanPatch {
                        id: 0,
                        cuts: Some(bad),
                        ..Default::default()
                    }],
                    Editor::User
                )
                .is_err());
        }
        // The empty clip cannot be cut, but its (empty) cut list can be cleared.
        let one = |cuts| PlanPatch {
            id: 1,
            cuts: Some(cuts),
            ..Default::default()
        };
        assert!(s.patch(&[one(vec![cut(0.0, 2.0)])], Editor::User).is_err());
        s.patch(&[one(vec![])], Editor::User).unwrap();
    }

    #[test]
    fn edited_dates_survive_log_planning_and_save_load_round_trips() {
        let mut s = session();
        let d = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        s.patch(
            &[PlanPatch {
                id: 0,
                date: Some(d),
                ..Default::default()
            }],
            Editor::User,
        )
        .unwrap();
        s.plan_dates(
            None,
            None,
            &Tunables::default(),
            NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
        );
        assert_eq!(s.plans[0].date, d);
        assert_eq!(s.plans[1].date.to_string(), "2026-09-30");
        let jobs = s.jobs();
        assert_eq!(jobs[0].date, "2026-09-01");

        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("session.json");
        s.save(&f).unwrap();
        let back = Session::load(&f).unwrap();
        assert_eq!(back.plans, s.plans);
        assert!(s.format_ready().is_err(), "a folder session never formats");
    }
}
