//! `Core`'s library log match: radio logs matched again to clips already in the library.

use super::Core;
use crate::library::{self as lib, LibClip};
use crate::logs::Badge;
use crate::metadata::FlightStats;
use crate::pipeline::{self, DateInput, DateSource};
use crate::sources::{dji, SourceKind};
use anyhow::{bail, Context, Result};
use chrono::{Datelike, Local, NaiveDate, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// `library_match_logs`: which clips, which logs, and whether to write the result.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct LibMatchParams {
    /// Library clip ids; empty means every clip (cuts excluded).
    pub ids: Vec<String>,
    /// The log folder; the `logDir` setting when missing.
    pub logs: Option<PathBuf>,
    /// One log day for every clip. Without it, each clip is matched to the logs of its own
    /// day, and clips with none to the newest reset-clock day (`2000-01-01`).
    pub day: Option<NaiveDate>,
    /// Write the flight numbers and moments into the matched clips' files. Without it,
    /// nothing changes. Writes "matched" clips, and "likely" ones only when named in `ids`.
    pub apply: bool,
}

/// One clip's result.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct LibMatch {
    pub id: String,
    pub path: PathBuf,
    pub duration: f64,
    pub badge: Badge,
    /// The log day matched against.
    pub log_day: Option<NaiveDate>,
    /// The log's start date and time, when the radio clock is believable.
    pub log_date: Option<NaiveDate>,
    pub log_time: Option<NaiveTime>,
    pub log_model: Option<String>,
    pub packs: usize,
    pub reason: Option<String>,
    /// The clip second of the first armed row, when the clip's picture placed the log.
    /// `flight.pack_spans` are in clip seconds with it.
    pub log_offset_s: Option<f64>,
    pub flight: Option<FlightStats>,
    pub moments: usize,
    /// The flight numbers and moments were written into the file.
    pub applied: bool,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct LibMatchReport {
    pub clips: Vec<LibMatch>,
    pub warnings: Vec<String>,
}

/// A library clip as dating input: its source (DJI by its file name), its DJI clip clock,
/// and its profile.
fn input(c: &LibClip) -> DateInput {
    let dvr = c.dvr.clone().unwrap_or_default();
    let analog = !dji::is_clip_name(&dvr);
    let (kind, clock) = if !analog {
        let clock = dji::parse_name(&dvr).and_then(|(t, _)| {
            Local
                .from_local_datetime(&t)
                .earliest()
                .map(|t| t.with_timezone(&chrono::Utc))
        });
        (SourceKind::Dji, clock)
    } else {
        (SourceKind::Analog, None)
    };
    DateInput {
        name: c.path.display().to_string(),
        duration: c.duration,
        clock,
        kind,
        profile: c.aircraft.clone(),
        day: Some(c.date),
        keep: if analog { c.keep.clone() } else { Vec::new() },
        follows: false,
    }
}

impl Core {
    /// Matches radio logs to library clips by shape, as an import does. Reports what
    /// matched; with `apply`, writes the flight numbers and moments into the files. It never
    /// changes a clip's date, time or name.
    pub fn library_match_logs(&self, p: &LibMatchParams) -> Result<LibMatchReport> {
        let d = self.defaults();
        let dir = p
            .logs
            .clone()
            .or(d.log_dir.clone())
            .context("No log folder: give logs, or set logDir.")?;
        if !dir.is_dir() {
            bail!("{} is not a folder.", dir.display());
        }
        let (root, mut clips) =
            self.with_index(|root, ix| Ok(((root.to_path_buf(), ix.clips.clone()), false)))?;
        clips.retain(|c| c.cut_of.is_none());
        if !p.ids.is_empty() {
            for id in &p.ids {
                if !clips.iter().any(|c| &c.id == id || c.aliases.contains(id)) {
                    bail!("No clip {id} in the library.");
                }
            }
            clips.retain(|c| p.ids.iter().any(|id| &c.id == id || c.aliases.contains(id)));
        }
        // Recording order: day, time, DVR name.
        clips.sort_by(|a, b| (a.date, &a.time, &a.dvr).cmp(&(b.date, &b.time, &b.dvr)));
        let today = Local::now().date_naive();
        let log_days: Vec<NaiveDate> = crate::logmatch::read_files(&dir)
            .iter()
            .flat_map(|f| f.rows.iter().map(|r| r.time.date()))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        // Groups of clips and the log day each is matched against.
        let mut groups: BTreeMap<Option<NaiveDate>, Vec<usize>> = BTreeMap::new();
        let reset_day = log_days.iter().rev().find(|d| d.year() < 2020).copied();
        for (i, c) in clips.iter().enumerate() {
            let day = match p.day {
                Some(day) => Some(day),
                None if log_days.contains(&c.date) => Some(c.date),
                None => reset_day,
            };
            groups.entry(day).or_default().push(i);
        }
        let mut out: Vec<Option<LibMatch>> = vec![None; clips.len()];
        let mut warnings = Vec::new();
        for (day, idx) in groups {
            let mut inputs: Vec<DateInput> = idx.iter().map(|&i| input(&clips[i])).collect();
            // The next file of a recording the DVR split (imported as clips of their own).
            for n in 1..idx.len() {
                let (a, b) = (&clips[idx[n - 1]], &clips[idx[n]]);
                inputs[n].follows = a.date == b.date
                    && a.parts.is_empty()
                    && match (&a.dvr, &b.dvr) {
                        (Some(x), Some(y)) => crate::join::may_follow(x, a.duration, y),
                        _ => false,
                    };
            }
            let plan = match day {
                Some(_) => pipeline::plan_dates_with(
                    &inputs,
                    Some(&dir),
                    day,
                    today,
                    &d.tunables,
                    &d.profiles,
                ),
                None => {
                    warnings.push(format!("No log of the clips' days in {}.", dir.display()));
                    pipeline::plan_dates_with(&inputs, None, None, today, &d.tunables, &[])
                }
            };
            for w in plan.warnings {
                if !warnings.contains(&w) {
                    warnings.push(w);
                }
            }
            for (&i, s) in idx.iter().zip(plan.suggestions) {
                let c = &clips[i];
                let offset = s.log_offset_s.unwrap_or(0.0);
                let moments: Vec<_> = s.moments.iter().map(|m| m.shifted(offset)).collect();
                let dated = s.source == DateSource::Log;
                out[i] = Some(LibMatch {
                    id: c.id.clone(),
                    path: c.path.clone(),
                    duration: c.duration,
                    badge: s.badge,
                    log_day: plan.day_used,
                    log_date: dated.then_some(s.date),
                    log_time: if dated { s.time } else { None },
                    log_model: s.log_model.clone(),
                    packs: s.segments,
                    reason: s.match_reason.clone(),
                    log_offset_s: s.log_offset_s,
                    flight: s.flight.as_ref().map(|f| f.shifted(offset)),
                    moments: s.moments.len(),
                    applied: false,
                });
                if p.apply {
                    let named = p.ids.contains(&c.id);
                    let write = match s.badge {
                        Badge::Matched => true,
                        Badge::Likely => named,
                        Badge::Unmatched => false,
                    };
                    if write {
                        let flight = s.flight.as_ref().map(|f| f.shifted(offset));
                        let stats = flight
                            .as_ref()
                            .and_then(|f| serde_json::to_string(f).ok())
                            .unwrap_or_default();
                        let line = s.flight.as_ref().map(FlightStats::line).unwrap_or_default();
                        lib::write_keys(
                            &root.join(&c.path),
                            &[
                                (lib::KEY_MOMENTS, lib::moments_value(&moments)),
                                (lib::KEY_STATS, stats),
                                ("app.quadcam.flight", line),
                            ],
                        )?;
                        if let Some(m) = out[i].as_mut() {
                            m.applied = true;
                        }
                    }
                }
            }
        }
        let written: Vec<String> = out
            .iter()
            .flatten()
            .filter(|m| m.applied)
            .map(|m| m.id.clone())
            .collect();
        if !written.is_empty() {
            self.reread_clips(&written)?;
        }
        Ok(LibMatchReport {
            clips: out.into_iter().flatten().collect(),
            warnings,
        })
    }
}
