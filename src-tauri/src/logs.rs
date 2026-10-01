//! EdgeTX radio logs: read armed rows, split into segments and sessions, and match them to
//! clips.

use anyhow::{Context, Result};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Tunables {
    /// A gap over this many seconds between rows starts a new armed segment.
    pub segment_gap_s: f64,
    /// A gap over this many minutes between segments starts a new session.
    pub session_gap_min: f64,
    /// Slack, in seconds, allowed when a clip claims segments.
    pub tolerance_s: f64,
    /// A log day further than this from the import date means the radio clock reset.
    pub max_log_age_days: i64,
}

impl Default for Tunables {
    fn default() -> Self {
        Self {
            segment_gap_s: 5.0,
            session_gap_min: 20.0,
            tolerance_s: 30.0,
            max_log_age_days: 60,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct Segment {
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub session: usize,
}

impl Segment {
    pub fn secs(&self) -> f64 {
        (self.end - self.start).num_milliseconds() as f64 / 1000.0
    }
}

/// Every CSV under `dir` (and `dir/LOGS` when `dir` is a radio's root).
pub fn csv_files(dir: &Path) -> Vec<PathBuf> {
    let base = if dir.join("LOGS").is_dir() {
        dir.join("LOGS")
    } else {
        dir.to_path_buf()
    };
    let Ok(rd) = std::fs::read_dir(&base) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            !n.starts_with('.') && n.to_lowercase().ends_with(".csv")
        })
        .collect();
    v.sort();
    v
}

/// Row timestamps from one EdgeTX CSV. Date and Time are the first two columns.
pub fn read_csv(path: &Path) -> Result<Vec<NaiveDateTime>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let mut cols = line.split(',');
        let (Some(d), Some(t)) = (cols.next(), cols.next()) else {
            continue;
        };
        let s = format!("{} {}", d.trim(), t.trim());
        if let Ok(dt) = NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S%.f") {
            out.push(dt);
        }
    }
    Ok(out)
}

/// All row times under `dir`, grouped by day, sorted.
pub fn read_log_dir(dir: &Path) -> BTreeMap<NaiveDate, Vec<NaiveDateTime>> {
    let mut days: BTreeMap<NaiveDate, Vec<NaiveDateTime>> = BTreeMap::new();
    for f in csv_files(dir) {
        for t in read_csv(&f).unwrap_or_default() {
            days.entry(t.date()).or_default().push(t);
        }
    }
    for v in days.values_mut() {
        v.sort();
    }
    days
}

/// Splits rows at gaps over `segment_gap_s`, then numbers sessions at gaps over `session_gap_min`.
pub fn segments(rows: &[NaiveDateTime], tun: &Tunables) -> Vec<Segment> {
    let gap = Duration::milliseconds((tun.segment_gap_s * 1000.0) as i64);
    let sgap = Duration::milliseconds((tun.session_gap_min * 60_000.0) as i64);
    let mut out: Vec<Segment> = Vec::new();
    for &t in rows {
        match out.last_mut() {
            Some(s) if t - s.end <= gap => s.end = t,
            last => {
                let session = match last {
                    Some(prev) if t - prev.end > sgap => prev.session + 1,
                    Some(prev) => prev.session,
                    None => 0,
                };
                out.push(Segment {
                    start: t,
                    end: t,
                    session,
                });
            }
        }
    }
    out
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Badge {
    Matched,
    Likely,
    Unmatched,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ClipMatch {
    pub badge: Badge,
    /// Start of the first claimed segment.
    pub start: Option<NaiveDateTime>,
    pub segments: usize,
    /// First start to last end of the claimed segments, in seconds.
    pub span_s: f64,
}

impl ClipMatch {
    fn unmatched() -> Self {
        Self {
            badge: Badge::Unmatched,
            start: None,
            segments: 0,
            span_s: 0.0,
        }
    }
}

/// Walks clips (PICT order) and segments (time order) together. A clip claims consecutive
/// segments of one session while their span fits inside its duration plus the tolerance.
/// **matched**: the unarmed time left over is within twice the tolerance.
/// **likely**: the order fits but the clip holds much more unarmed time than the span.
/// **unmatched**: the next segment does not fit; the segment stays for the next clip.
pub fn match_clips(durations: &[f64], segs: &[Segment], tun: &Tunables) -> Vec<ClipMatch> {
    let mut out = Vec::with_capacity(durations.len());
    let mut j = 0;
    for &dur in durations {
        let limit = dur + tun.tolerance_s;
        if j >= segs.len() || dur <= 0.0 || segs[j].secs() > limit {
            out.push(ClipMatch::unmatched());
            continue;
        }
        let first = segs[j];
        let mut k = j;
        while k + 1 < segs.len()
            && segs[k + 1].session == first.session
            && (segs[k + 1].end - first.start).num_milliseconds() as f64 / 1000.0 <= limit
        {
            k += 1;
        }
        let span = (segs[k].end - first.start).num_milliseconds() as f64 / 1000.0;
        let badge = if dur - span <= 2.0 * tun.tolerance_s {
            Badge::Matched
        } else {
            Badge::Likely
        };
        out.push(ClipMatch {
            badge,
            start: Some(first.start),
            segments: k - j + 1,
            span_s: span,
        });
        j = k + 1;
    }
    out
}

/// A log dated before 2020 (2000-01-01 after an RTC reset) or far from the import date.
pub fn day_is_plausible(day: NaiveDate, import_day: NaiveDate, tun: &Tunables) -> bool {
    day.year() >= 2020 && (import_day - day).num_days().abs() <= tun.max_log_age_days
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(&format!("2026-09-30 {s}"), "%Y-%m-%d %H:%M:%S").unwrap()
    }

    /// Rows every 0.5 s from `a` to `b`.
    fn rows(a: &str, b: &str) -> Vec<NaiveDateTime> {
        let (a, b) = (t(a), t(b));
        let mut v = Vec::new();
        let mut x = a;
        while x <= b {
            v.push(x);
            x += Duration::milliseconds(500);
        }
        v
    }

    #[test]
    fn segments_and_sessions() {
        let mut r = rows("10:00:00", "10:03:00");
        r.extend(rows("10:05:00", "10:08:00")); // 2 min gap: new segment, same session
        r.extend(rows("11:00:00", "11:02:00")); // 52 min gap: new session
        let s = segments(&r, &Tunables::default());
        assert_eq!(s.len(), 3);
        assert_eq!((s[0].session, s[1].session, s[2].session), (0, 0, 1));
        assert_eq!(s[0].secs(), 180.0);
    }

    #[test]
    fn matching_badges() {
        let tun = Tunables::default();
        let mut r = rows("10:00:00", "10:03:00");
        r.extend(rows("10:05:00", "10:08:00"));
        r.extend(rows("11:00:00", "11:02:00"));
        let s = segments(&r, &tun);
        // Clip 1: two packs in one file, 8 min 20 s long (span 8:00) -> matched, 2 segments.
        // Clip 2: a 10 s bench clip -> unmatched, next segment stays.
        // Clip 3: 6 min for a 2 min flight -> likely.
        // Clip 4: nothing left -> unmatched.
        let m = match_clips(&[500.0, 10.0, 360.0, 60.0], &s, &tun);
        assert_eq!(m[0].badge, Badge::Matched);
        assert_eq!(m[0].segments, 2);
        assert_eq!(m[0].start, Some(t("10:00:00")));
        assert_eq!(m[1].badge, Badge::Unmatched);
        assert_eq!(m[2].badge, Badge::Likely);
        assert_eq!(m[2].start, Some(t("11:00:00")));
        assert_eq!(m[3].badge, Badge::Unmatched);
    }

    #[test]
    fn rtc_reset_is_implausible() {
        let tun = Tunables::default();
        let today = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        assert!(!day_is_plausible(
            NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(),
            today,
            &tun
        ));
        assert!(!day_is_plausible(
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            today,
            &tun
        ));
        assert!(day_is_plausible(
            NaiveDate::from_ymd_opt(2026, 9, 28).unwrap(),
            today,
            &tun
        ));
    }

    #[test]
    fn reads_edgetx_csv() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("LOGS")).unwrap();
        std::fs::write(
            d.path().join("LOGS/Model01-2026-09-30-100000.csv"),
            "Date,Time,1RSS(dB),RQly(%)\n2026-09-30,10:00:00.000,-50,100\n2026-09-30,10:00:00.500,-50,100\n",
        )
        .unwrap();
        let days = read_log_dir(d.path());
        assert_eq!(days.len(), 1);
        assert_eq!(days.values().next().unwrap().len(), 2);
    }
}
