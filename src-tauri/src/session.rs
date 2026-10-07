//! One import session: the clips staged from a card or folder, the per-clip plan (date,
//! name, note, skip) and the import results. The GUI, the CLI and the MCP server all work on
//! this one type, and it saves to JSON so separate CLI runs can continue a session.

use crate::disk::{self, CardIdentity, Volume};
use crate::logs::{Badge, Tunables};
use crate::media::Tools;
use crate::metadata::{self as md, ClipMeta, FlightStats, Location};
use crate::moments::{Moment, Span};
use crate::pipeline::{self, Clip, ClipJob, ClipResult, ClipStatus, DateSource, Outcome};
use crate::sources::SourceKind;
use anyhow::{bail, Context, Result};
use chrono::{NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SESSION_VERSION: u32 = 1;

pub use crate::pipeline::import::run_import;
pub use crate::settings::Defaults;
pub use crate::trim::{MAX_CUTS, MIN_CUT_S};

/// Which fields of a plan an agent wrote and the user has not edited since.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, specta::Type)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
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
    /// Why the radio log matched, or how sure the match is.
    #[serde(default)]
    pub match_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum Editor {
    User,
    Agent,
}

/// A change to one clip's plan. Missing fields stay as they are.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, specta::Type)]
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
    /// True adds one cut per radio-log flight (see `trim::flight_cuts`) to the cut list,
    /// after `cuts` when both are given.
    #[serde(default)]
    pub split_by_flight: Option<bool>,
    /// For the first file of a recording the DVR split into files: true imports the files
    /// as one clip, false keeps them as clips of their own.
    #[serde(default)]
    pub joined: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct Session {
    pub version: u32,
    /// Card mount point or folder the clips came from.
    pub source: PathBuf,
    /// The video system of the clips.
    #[serde(default)]
    pub kind: SourceKind,
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
    /// Whether this run joins recordings the DVR split into files. None follows the
    /// `join_split_recordings` setting.
    #[serde(default)]
    pub join: Option<bool>,
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

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct Summary {
    pub results: Vec<ClipResult>,
    pub imported: usize,
    pub skipped: usize,
    pub failed: usize,
    pub total_bytes: u64,
    pub output_dir: Option<PathBuf>,
    /// Ok when the format step may unlock, else the reason it stays locked.
    #[specta(type = crate::api::SerdeResult<(), String>)]
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
        let system = crate::sources::for_root(source);
        let clips =
            pipeline::stage_found(system.list(source), system.kind(), &staging, on_progress)?;
        Ok(Session::staged(source, system.kind(), vol, staging, clips))
    }

    /// Copies the clips among `files` (files dropped on the window) into staging. The
    /// session's source is the folder of the first clip; it is never a card.
    pub fn stage_files(
        files: &[PathBuf],
        staging_root: &Path,
        on_progress: &mut dyn FnMut(usize, usize, u64, u64),
    ) -> Result<Session> {
        // The first source that finds clips among the files.
        let (system, found) = crate::sources::all()
            .into_iter()
            .map(|s| (s, s.pick(files)))
            .find(|(_, f)| !f.is_empty())
            .unwrap_or((crate::sources::get(Default::default()), Vec::new()));
        let Some(source) = found
            .first()
            .and_then(|f| f.path.parent())
            .map(Path::to_path_buf)
        else {
            bail!("None of these files is a clip QuadCam reads (a DVR AVI or a DJI MP4).");
        };
        let staging = staging_root.join(chrono::Local::now().format("%Y%m%d-%H%M%S").to_string());
        let clips = pipeline::stage_found(found, system.kind(), &staging, on_progress)?;
        Ok(Session::staged(
            &source,
            system.kind(),
            None,
            staging,
            clips,
        ))
    }

    /// A new session for clips just staged: every clip planned for today, unnamed.
    fn staged(
        source: &Path,
        kind: SourceKind,
        vol: Option<Volume>,
        staging: PathBuf,
        clips: Vec<Clip>,
    ) -> Session {
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
                match_reason: None,
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
            kind,
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
            join: None,
        }
    }

    /// Probes, recovers half-written clips and makes thumbnails. Empty clips get skipped.
    /// Then finds recordings the DVR split into files (`join::groups`) and, with `join`,
    /// imports each as one clip.
    pub fn analyse(
        &mut self,
        tools: &Tools,
        thumbs: &Path,
        join: bool,
        on_progress: &mut dyn FnMut(usize, usize),
    ) -> Result<()> {
        std::fs::create_dir_all(thumbs)?;
        for c in &mut self.clips {
            c.join = None;
            c.part_of = None;
        }
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
        self.find_joins(join);
        Ok(())
    }

    /// Marks the recordings the DVR split into files, joined when `on`. A recording whose
    /// list cannot be written stays as separate files, with a warning.
    pub fn find_joins(&mut self, on: bool) {
        for group in crate::join::groups(&self.clips) {
            match crate::join::make(&self.clips, &group) {
                Ok(j) => {
                    let id = self.clips[group[0]].id;
                    self.clips[group[0]].join = Some(j);
                    if on {
                        let _ = self.set_joined(id, true);
                    }
                }
                Err(e) => self.warnings.push(format!(
                    "{} and the files after it look like one recording, but stay apart: {e:#}",
                    self.clips[group[0]].name
                )),
            }
        }
    }

    /// Imports a split recording as one clip (`on`) or as clips of their own. `id` is its
    /// first file. Refused once any of its files imported. When the files part, the first
    /// one's cuts are clamped to its own length.
    pub fn set_joined(&mut self, id: usize, on: bool) -> Result<()> {
        let at = self
            .clips
            .iter()
            .position(|c| c.id == id)
            .with_context(|| format!("no clip with id {id}"))?;
        let parts = match &self.clips[at].join {
            Some(j) => j.parts.clone(),
            None => bail!("clip {id} is not the first file of a recording the DVR split"),
        };
        if self.clips[at].join.as_ref().is_some_and(|j| j.on == on) {
            return Ok(());
        }
        let ids: Vec<usize> = std::iter::once(id).chain(parts.iter().copied()).collect();
        if self
            .results
            .iter()
            .any(|r| ids.contains(&r.id) && r.outcome == Outcome::Verified)
        {
            bail!("clip {id} was imported already; its files cannot be joined or parted now");
        }
        crate::join::set(&mut self.clips[at], on);
        for c in self.clips.iter_mut().filter(|c| parts.contains(&c.id)) {
            c.part_of = on.then_some(id);
        }
        let duration = self.clips[at].duration;
        let p = self.plan_mut(id)?;
        if !on {
            p.cuts
                .retain(|c| c.start + crate::trim::MIN_CUT_S <= duration);
            for c in &mut p.cuts {
                c.end = c.end.min(duration);
            }
            p.moments.retain(|m| m.start < duration);
        }
        Ok(())
    }

    /// Suggests dates from radio logs; plans the user or an agent edited keep their date.
    pub fn plan_dates(
        &mut self,
        log_dir: Option<&Path>,
        day: Option<NaiveDate>,
        tun: &Tunables,
        today: NaiveDate,
        profiles: &[crate::metadata::Profile],
    ) {
        // A part of a joined recording is dated with its first file.
        let parts: Vec<bool> = self.clips.iter().map(|c| c.part_of.is_some()).collect();
        let inputs: Vec<pipeline::DateInput> = self
            .clips
            .iter()
            .zip(&self.plans)
            .filter(|(c, _)| c.part_of.is_none())
            .map(|(c, p)| pipeline::DateInput::of(c, self.kind, p.meta.profile.as_deref()))
            .collect();
        let plan = pipeline::plan_dates_with(&inputs, log_dir, day, today, tun, profiles);
        let plans = self
            .plans
            .iter_mut()
            .zip(parts)
            .filter(|(_, part)| !part)
            .map(|(p, _)| p);
        for (p, s) in plans.zip(plan.suggestions) {
            // A log the clip's picture placed sets the offset; otherwise a set one stays.
            if let Some(o) = s.log_offset_s {
                p.log_offset_s = o;
            }
            p.moments = s
                .moments
                .iter()
                .map(|m| m.shifted(p.log_offset_s))
                .collect();
            p.log_interval_s = s.log_interval_s;
            p.log_model = s.log_model.clone();
            p.flight = s.flight.as_ref().map(|f| f.shifted(p.log_offset_s));
            p.match_reason = s.match_reason.clone();
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
            if let Some(head) = self
                .clips
                .iter()
                .find(|c| c.id == patch.id)
                .and_then(|c| c.part_of)
            {
                bail!(
                    "clip {} is part of clip {head}, one recording; set joined false on clip {head} to change it on its own",
                    patch.id
                );
            }
            if let Some(on) = patch.joined {
                self.set_joined(patch.id, on)?;
            }
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
            let what = format!("clip {}", patch.id);
            let mut cuts = patch
                .cuts
                .as_deref()
                .map(|c| crate::trim::check_cuts(&what, c, duration))
                .transpose()?;
            if patch.split_by_flight == Some(true) {
                let p = self.plan_mut(patch.id)?;
                let flights = p
                    .flight
                    .as_ref()
                    .map(|f| f.flight_spans.clone())
                    .unwrap_or_default();
                let base = cuts.clone().unwrap_or_else(|| p.cuts.clone());
                let add = crate::trim::flight_cuts(&what, &flights, duration)?;
                cuts = Some(crate::trim::check_cuts(
                    &what,
                    &crate::trim::with_cuts(&base, &add),
                    duration,
                )?);
            }
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
                p.flight = p.flight.as_ref().map(|f| f.shifted(by));
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

    /// One job per clip to import; a part of a joined recording imports with its first file.
    pub fn jobs(&self) -> Vec<ClipJob> {
        self.plans
            .iter()
            .filter(|p| {
                !self
                    .clips
                    .iter()
                    .any(|c| c.id == p.id && c.part_of.is_some())
            })
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
                parts: Vec::new(),
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
            kind: Default::default(),
            clock: None,
            sidecars: Vec::new(),
            mtime: None,
            join: None,
            part_of: None,
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
            match_reason: None,
            flight: None,
        };
        Session {
            version: SESSION_VERSION,
            source: PathBuf::from("/tmp/x"),
            kind: SourceKind::Analog,
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
            join: None,
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
    fn a_match_by_picture_sets_the_log_offset() {
        let d = tempfile::tempdir().unwrap();
        let mut csv = String::from("Date,Time,RxBt(V)\n");
        for (a, b) in [(30.0, 150.0), (240.0, 360.0)] {
            let mut t: f64 = a;
            while t <= b {
                csv.push_str(&format!(
                    "2026-09-28,18:{:02}:{:06.3},4.1\n",
                    (t / 60.0) as u32,
                    t % 60.0
                ));
                t += 0.5;
            }
        }
        std::fs::write(d.path().join("AIR65 II-2026-09-28.csv"), csv).unwrap();
        let mut s = session();
        s.clips[0].duration = 380.0;
        s.clips[0].signal = Some(crate::moments::SignalScan {
            keep: vec![
                Span {
                    start: 20.0,
                    end: 140.0,
                },
                Span {
                    start: 180.0,
                    end: 360.0,
                },
            ],
            ..Default::default()
        });
        s.plan_dates(
            Some(d.path()),
            None,
            &Tunables::default(),
            NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
            &[],
        );
        let p = &s.plans[0];
        assert_eq!(p.badge, Badge::Matched, "{:?}", p.match_reason);
        assert!(
            (16.0..=24.0).contains(&p.log_offset_s),
            "{}",
            p.log_offset_s
        );
        let f = p.flight.as_ref().unwrap();
        assert_eq!(f.flight_spans[0].start, p.log_offset_s);
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
    fn split_by_flight_adds_a_cut_per_flight_on_the_clip_timeline() {
        let mut s = session();
        s.clips[0].duration = 300.0;
        let span = |a, b| Span { start: a, end: b };
        s.plans[0].flight = Some(FlightStats {
            flights: 2,
            flight_spans: vec![span(0.0, 100.0), span(150.0, 290.0)],
            ..Default::default()
        });
        s.plans[0].cuts = vec![span(10.0, 20.0)];
        let split = |s: &mut Session| {
            s.patch(
                &[PlanPatch {
                    id: 0,
                    split_by_flight: Some(true),
                    ..Default::default()
                }],
                Editor::Agent,
            )
        };
        // The log starts 5 s into the clip: the flights move with it, and the cuts follow.
        s.patch(
            &[PlanPatch {
                id: 0,
                log_offset_s: Some(5.0),
                ..Default::default()
            }],
            Editor::User,
        )
        .unwrap();
        assert_eq!(
            s.plans[0].flight.as_ref().unwrap().flight_spans,
            vec![span(5.0, 105.0), span(155.0, 295.0)]
        );
        split(&mut s).unwrap();
        // The hand-made cut stays; the second flight's lead-out stops at the clip's end.
        assert_eq!(
            s.plans[0].cuts,
            vec![span(3.0, 107.0), span(10.0, 20.0), span(153.0, 297.0)]
        );
        assert!(s.plans[0].suggested.cuts);
        // Again: the same ranges are not added twice.
        split(&mut s).unwrap();
        assert_eq!(s.plans[0].cuts.len(), 3);
        // A clip without flights has nothing to split, and nothing changes.
        s.plans[0].flight = None;
        assert!(split(&mut s)
            .unwrap_err()
            .to_string()
            .contains("nothing to split"));
        assert_eq!(s.plans[0].cuts.len(), 3);
    }

    /// A session of two DVR files of one recording (600 s, then 450 s) and a third, short
    /// recording, staged in `dir`.
    fn split_session(dir: &Path) -> Session {
        let mut s = session();
        let mut c = s.clips[0].clone();
        let mut clips = Vec::new();
        for (id, secs) in [(0, 600.0), (1, 450.0), (2, 90.0)] {
            c.id = id;
            c.name = format!("PICT000{}.AVI", id + 1);
            c.rel = format!("DCIM/{}", c.name);
            c.status = ClipStatus::Ok;
            c.duration = secs;
            c.key = format!("k{id}");
            c.probe = Some(crate::media::Probe {
                duration: secs,
                video_packets: (secs * 30.0) as u64,
                video_streams: 1,
                audio_streams: 1,
                fps: Some(30.0),
                width: Some(720),
                height: Some(480),
                ..Default::default()
            });
            let f = dir.join(&c.name);
            std::fs::write(&f, b"avi").unwrap();
            c.staged = Some(f);
            clips.push(c.clone());
        }
        s.plans = clips
            .iter()
            .map(|c| ClipPlan {
                id: c.id,
                ..s.plans[0].clone()
            })
            .collect();
        s.clips = clips;
        s
    }

    /// One 2026-09-28 log: two flights that straddle the DVR's file boundary at 600 s, then a
    /// 90 s flight for the third file.
    fn split_log(dir: &Path) {
        std::fs::create_dir(dir.join("LOGS")).unwrap();
        let mut csv = String::from("Date,Time,1RSS(dB),RQly(%)\n");
        for (a, b) in [(0, 500), (530, 1040), (1400, 1488)] {
            for t in a..=b {
                let t = 10 * 3600 + t;
                csv.push_str(&format!(
                    "2026-09-28,{:02}:{:02}:{:02}.000,-50,100\n",
                    t / 3600,
                    t / 60 % 60,
                    t % 60
                ));
            }
        }
        std::fs::write(dir.join("LOGS/Quad-2026-09-28.csv"), csv).unwrap();
    }

    #[test]
    fn a_joined_recording_is_one_clip_for_matching_and_import() {
        let d = tempfile::tempdir().unwrap();
        let mut s = split_session(d.path());
        split_log(d.path());
        s.find_joins(true);
        assert_eq!(s.clips[1].part_of, Some(0));
        assert_eq!(s.clips[0].duration, 1050.0);
        assert_eq!(
            s.clips[0].probe.as_ref().unwrap().video_packets,
            31500,
            "frames add up"
        );
        assert!(s.clips[0]
            .source()
            .unwrap()
            .to_string_lossy()
            .ends_with(".ffconcat"));
        assert!(s.clips[2].join.is_none() && s.clips[2].part_of.is_none());
        let today = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        let plan = |s: &mut Session| {
            s.plan_dates(
                Some(&d.path().join("LOGS")),
                NaiveDate::from_ymd_opt(2026, 9, 28),
                &Tunables::default(),
                today,
                &[],
            )
        };
        plan(&mut s);
        // The joined clip claims both flights, across the file boundary; the part is not
        // matched on its own.
        let p0 = &s.plans[0];
        assert_eq!(p0.badge, Badge::Matched, "{:?}", p0.match_reason);
        assert_eq!(p0.segments, 2);
        let f = p0.flight.as_ref().unwrap();
        assert_eq!(
            f.flight_spans,
            vec![
                Span {
                    start: 0.0,
                    end: 500.0
                },
                Span {
                    start: 530.0,
                    end: 1040.0
                }
            ]
        );
        assert_eq!(s.plans[1].badge, Badge::Unmatched);
        assert_eq!(s.plans[2].segments, 1);
        // Split by flight on the joined timeline: the second cut runs past 600 s.
        s.patch(
            &[PlanPatch {
                id: 0,
                split_by_flight: Some(true),
                ..Default::default()
            }],
            Editor::User,
        )
        .unwrap();
        assert_eq!(s.plans[0].cuts.last().unwrap().end, 1042.0);
        // One job for the recording: the part imports with it.
        let ids: Vec<usize> = s.jobs().iter().map(|j| j.id).collect();
        assert_eq!(ids, [0, 2]);
        // A part cannot be edited on its own.
        let err = s
            .patch(
                &[PlanPatch {
                    id: 1,
                    name: Some("x".into()),
                    ..Default::default()
                }],
                Editor::User,
            )
            .unwrap_err();
        assert!(err.to_string().contains("part of clip 0"), "{err}");

        // Kept apart: three clips again, the cuts past 600 s go, and the first file matches
        // the first flight alone.
        s.patch(
            &[PlanPatch {
                id: 0,
                joined: Some(false),
                ..Default::default()
            }],
            Editor::User,
        )
        .unwrap();
        assert_eq!(s.clips[0].duration, 600.0);
        assert_eq!(s.clips[1].part_of, None);
        assert!(s.plans[0].cuts.iter().all(|c| c.end <= 600.0));
        assert_eq!(s.clips[0].source(), s.clips[0].staged.as_deref());
        plan(&mut s);
        assert_eq!(s.jobs().len(), 3);
        assert_eq!(s.plans[0].segments, 1);
        // Joined again by hand.
        s.patch(
            &[PlanPatch {
                id: 0,
                joined: Some(true),
                ..Default::default()
            }],
            Editor::User,
        )
        .unwrap();
        assert_eq!(s.clips[0].duration, 1050.0);
        // Not a recording's first file.
        assert!(s
            .patch(
                &[PlanPatch {
                    id: 2,
                    joined: Some(true),
                    ..Default::default()
                }],
                Editor::User
            )
            .is_err());
    }

    #[test]
    fn found_but_off_keeps_the_files_apart() {
        let d = tempfile::tempdir().unwrap();
        let mut s = split_session(d.path());
        s.find_joins(false);
        let j = s.clips[0].join.as_ref().unwrap();
        assert!(!j.on);
        assert_eq!(j.parts, vec![1]);
        assert_eq!(s.clips[0].duration, 600.0);
        assert_eq!(s.clips[1].part_of, None);
        assert_eq!(s.jobs().len(), 3);
        // Once a file imported, the join cannot change.
        s.results.push(crate::pipeline::ClipResult {
            id: 1,
            outcome: Outcome::Verified,
            output: None,
            original: None,
            size: 0,
            error: None,
            encoder: None,
            meta: None,
            cuts: Vec::new(),
            qt: Vec::new(),
        });
        assert!(s.set_joined(0, true).is_err());
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
            &[],
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
