//! `Core`'s batch time shift: the real time of one library clip moves a batch of clips by
//! the same offset, unless a radio log gives a clip its own time.

use super::Core;
use crate::library::LibClip;
use crate::logs::Badge;
use anyhow::{bail, Context, Result};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

/// `library_time_shift`: the clips, the one whose real time is known, and that time.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct LibShiftParams {
    /// Library clip ids to shift; `from` is added when missing.
    pub ids: Vec<String>,
    /// The clip whose real date and time are `date` and `time`.
    pub from: String,
    /// Its real flying day; its own day when missing.
    pub date: Option<NaiveDate>,
    /// Its real time of day, `HH:MM`.
    pub time: String,
    /// The log folder to check for better times; the `logDir` setting when missing. No
    /// folder at all shifts every clip by the offset.
    pub logs: Option<PathBuf>,
    /// Write the new dates and times. Without it, nothing changes.
    pub apply: bool,
}

/// Where a clip's new time came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ShiftSource {
    /// The clip whose time was given.
    Set,
    /// Its own time plus the offset.
    Offset,
    /// A radio log that matched the clip.
    Log,
    /// No new time: the clip has no time of day and no log matched it.
    Skipped,
}

/// One clip's result.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct LibShift {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub old_date: NaiveDate,
    /// `HH:MM`, when the clip had a time of day.
    pub old_time: Option<String>,
    pub date: Option<NaiveDate>,
    /// `HH:MM`.
    pub time: Option<String>,
    pub source: ShiftSource,
    /// Why a log answer or a skip.
    pub reason: Option<String>,
    /// The new date or time differs from the old one.
    pub changed: bool,
    /// The new date and time were written.
    pub applied: bool,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct LibShiftReport {
    /// The offset in seconds: the given time minus the `from` clip's own.
    pub offset_s: i64,
    pub clips: Vec<LibShift>,
    pub warnings: Vec<String>,
}

/// A radio log's date and time for a clip, and the match's reason.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LogTime {
    pub at: NaiveDateTime,
    pub reason: Option<String>,
}

fn clip_time(c: &LibClip) -> Option<NaiveDateTime> {
    let t = NaiveTime::parse_from_str(c.time.as_deref()?, "%H:%M").ok()?;
    Some(c.date.and_time(t))
}

fn hm(t: NaiveDateTime) -> String {
    t.format("%H:%M").to_string()
}

/// The shift of each clip: the `from` clip takes `to`, a clip a log matched takes the log's
/// time, every other clip with a time moves by the offset, and a clip without one is skipped.
pub(crate) fn plan(
    clips: &[LibClip],
    from: &str,
    to: NaiveDateTime,
    logs: &HashMap<String, LogTime>,
) -> Result<(i64, Vec<LibShift>)> {
    let anchor = clips
        .iter()
        .find(|c| c.id == from)
        .with_context(|| format!("from: no clip {from} among the clips to shift."))?;
    let old = clip_time(anchor).with_context(|| {
        format!(
            "{} has no time of day to shift from. Set its time with an edit, or pick a clip that has one.",
            anchor.display_name()
        )
    })?;
    let offset = to - old;
    let rows = clips
        .iter()
        .map(|c| {
            let own = clip_time(c);
            let (at, source, reason) = if c.id == from {
                (Some(to), ShiftSource::Set, None)
            } else if let Some(l) = logs.get(&c.id) {
                (Some(l.at), ShiftSource::Log, l.reason.clone())
            } else if let Some(t) = own {
                (Some(t + offset), ShiftSource::Offset, None)
            } else {
                (
                    None,
                    ShiftSource::Skipped,
                    Some("No time of day, and no radio log matched.".to_string()),
                )
            };
            LibShift {
                id: c.id.clone(),
                name: c.display_name(),
                path: c.path.clone(),
                old_date: c.date,
                old_time: c.time.clone(),
                date: at.map(|t| t.date()),
                time: at.map(hm),
                source,
                reason,
                changed: at.is_some_and(|t| t.date() != c.date || Some(hm(t)) != c.time),
                applied: false,
            }
        })
        .collect();
    Ok((offset.num_seconds(), rows))
}

impl Core {
    /// Shifts library clips by the offset between one clip's real time and its own. A clip a
    /// radio log matches takes the log's time instead. Writes only with `apply`.
    pub fn library_time_shift(&self, p: &LibShiftParams) -> Result<LibShiftReport> {
        if p.from.trim().is_empty() {
            bail!("from is required: the library clip id whose real time is given.");
        }
        let time = crate::session::parse_time(&p.time)?
            .context("time is required: the real time of day, HH:MM.")?;
        let mut ids: Vec<String> = Vec::new();
        let mut clips: Vec<LibClip> = Vec::new();
        for id in std::iter::once(&p.from).chain(&p.ids) {
            let (_, c) = self.clip(id)?;
            if !ids.contains(&c.id) {
                ids.push(c.id.clone());
                clips.push(c);
            }
        }
        let from = ids[0].clone();
        let to = p.date.unwrap_or(clips[0].date).and_time(time);

        // First the offset alone, for the day each clip lands on; then the logs of those days.
        let (_, first) = plan(&clips, &from, to, &HashMap::new())?;
        let mut warnings = Vec::new();
        let d = self.defaults();
        let dir = match (&p.logs, &d.log_dir) {
            (Some(dir), _) => {
                if !dir.is_dir() {
                    bail!("{} is not a folder.", dir.display());
                }
                Some(dir.clone())
            }
            (None, Some(dir)) if dir.is_dir() => Some(dir.clone()),
            _ => None,
        };
        let mut logs: HashMap<String, LogTime> = HashMap::new();
        if let Some(dir) = &dir {
            let offset = to - clip_time(&clips[0]).unwrap_or(to);
            let mut by_day: BTreeMap<NaiveDate, Vec<String>> = BTreeMap::new();
            for (c, s) in clips.iter().zip(&first) {
                if c.id == from || c.cut_of.is_some() {
                    continue;
                }
                // A clip without a time lands on the day its noon shifts to.
                let day = s.date.unwrap_or_else(|| {
                    (c.date.and_time(NaiveTime::from_hms_opt(12, 0, 0).unwrap()) + offset).date()
                });
                by_day.entry(day).or_default().push(c.id.clone());
            }
            for (day, ids) in by_day {
                let r = self.library_match_logs(&super::LibMatchParams {
                    ids,
                    logs: Some(dir.clone()),
                    day: Some(day),
                    apply: false,
                })?;
                for w in r.warnings {
                    if !warnings.contains(&w) {
                        warnings.push(w);
                    }
                }
                for m in r.clips {
                    if let (Badge::Matched, Some(date), Some(t)) = (m.badge, m.log_date, m.log_time)
                    {
                        let t = t.with_nanosecond(0).unwrap_or(t);
                        logs.insert(
                            m.id,
                            LogTime {
                                at: date.and_time(t),
                                reason: m.reason,
                            },
                        );
                    }
                }
            }
        }
        let (offset_s, mut rows) = plan(&clips, &from, to, &logs)?;
        if p.apply {
            for (r, c) in rows.iter_mut().zip(&clips) {
                let (Some(date), Some(t)) = (r.date, r.time.as_deref()) else {
                    continue;
                };
                if !r.changed {
                    continue;
                }
                let t = crate::session::parse_time(t)?;
                let (root, c) = self.clip(&c.id)?;
                let items = crate::qtmeta::read(&root.join(&c.path))?;
                self.redate(
                    &root,
                    &c,
                    &items,
                    Some(date),
                    Some(t),
                    r.source == ShiftSource::Log,
                )?;
                r.applied = true;
                r.path = self.clip(&c.id)?.1.path;
            }
        }
        Ok(LibShiftReport {
            offset_s,
            clips: rows,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(id: &str, date: &str, time: Option<&str>) -> LibClip {
        let mut c: LibClip = serde_json::from_value(serde_json::json!({
            "id": id,
            "path": format!("{date}_{id}.mp4"),
            "title": id,
            "note": "",
            "date": date,
            "duration": 10.0,
            "size": 1,
        }))
        .unwrap();
        c.time = time.map(str::to_string);
        c
    }

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn the_offset_moves_every_clip_and_a_log_wins() {
        let clips = vec![
            clip("a", "2000-01-01", Some("00:05")),
            clip("b", "2000-01-01", Some("00:20")),
            clip("c", "2000-01-01", Some("23:50")),
            clip("d", "2000-01-01", None),
            clip("e", "2000-01-01", Some("00:30")),
        ];
        let mut logs = HashMap::new();
        logs.insert(
            "e".to_string(),
            LogTime {
                at: at("2026-09-28 18:47"),
                reason: Some("shape".into()),
            },
        );
        let (offset, rows) = plan(&clips, "a", at("2026-09-28 18:30"), &logs).unwrap();
        let got: Vec<_> = rows
            .iter()
            .map(|r| {
                (
                    r.id.as_str(),
                    r.date.map(|d| d.to_string()),
                    r.time.as_deref(),
                    r.source,
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (
                    "a",
                    Some("2026-09-28".into()),
                    Some("18:30"),
                    ShiftSource::Set
                ),
                (
                    "b",
                    Some("2026-09-28".into()),
                    Some("18:45"),
                    ShiftSource::Offset
                ),
                // Past midnight: the next day.
                (
                    "c",
                    Some("2026-09-29".into()),
                    Some("18:15"),
                    ShiftSource::Offset
                ),
                ("d", None, None, ShiftSource::Skipped),
                (
                    "e",
                    Some("2026-09-28".into()),
                    Some("18:47"),
                    ShiftSource::Log
                ),
            ]
        );
        assert_eq!(
            offset,
            (at("2026-09-28 18:30") - at("2000-01-01 00:05")).num_seconds()
        );
        assert!(rows.iter().filter(|r| r.date.is_some()).all(|r| r.changed));
        assert!(!rows[3].changed);
        assert_eq!(rows[4].reason.as_deref(), Some("shape"));
    }

    #[test]
    fn a_from_clip_without_a_time_refuses() {
        let clips = vec![
            clip("a", "2026-09-28", None),
            clip("b", "2026-09-28", Some("10:00")),
        ];
        let e = plan(&clips, "a", at("2026-09-28 10:00"), &HashMap::new()).unwrap_err();
        assert!(e.to_string().contains("no time of day"), "{e}");
        assert!(plan(&clips, "x", at("2026-09-28 10:00"), &HashMap::new()).is_err());
    }

    #[test]
    fn the_same_time_changes_nothing() {
        let clips = vec![
            clip("a", "2026-09-28", Some("10:00")),
            clip("b", "2026-09-28", Some("10:10")),
        ];
        let (offset, rows) = plan(&clips, "a", at("2026-09-28 10:00"), &HashMap::new()).unwrap();
        assert_eq!(offset, 0);
        assert!(rows.iter().all(|r| !r.changed));
    }
}
