//! The import pipeline without any UI: stage, analyse, date, convert, verify.
//! `session` and `core` build on these.

pub mod import;

use crate::library::{self, Layout};
use crate::logmatch;
use crate::logs::{self, Badge, Tunables};
use crate::media::{self, Encoder, Format, Meta, Probe, Tools};
use crate::metadata::{self as md, FlightStats, Resolved};
use crate::moments::{Moment, MomentSource, RadioLog, SignalScan, Span};
use crate::naming::{self, NamePlanner};
use crate::scan::FoundClip;
use crate::sources::SourceKind;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ClipStatus {
    Ok,
    Incomplete,
    Empty,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct Clip {
    pub id: usize,
    pub name: String,
    pub rel: String,
    pub card_path: PathBuf,
    pub size: u64,
    pub staged: Option<PathBuf>,
    /// Set when the copy failed, for example because the card was pulled.
    pub stage_error: Option<String>,
    pub status: ClipStatus,
    /// Duration of what will be converted (the recovered length for a half-written clip).
    pub duration: f64,
    /// A rewritten copy of a half-written clip; it becomes the conversion source.
    pub recovered: Option<PathBuf>,
    pub probe: Option<Probe>,
    pub thumb: Option<PathBuf>,
    /// One line on what analysis found.
    pub detail: String,
    /// Dead air found in the clip's frames, and the ranges worth keeping.
    #[serde(default)]
    pub signal: Option<SignalScan>,
    /// Content fingerprint (see `fingerprint`). Names the clip's cached thumbnail and
    /// preview, since DVRs reuse file names: the Echo restarts at PICT0001 after a format.
    #[serde(default)]
    pub key: String,
    /// The video system the clip came from.
    #[serde(default)]
    pub kind: SourceKind,
    /// The time the clip itself recorded (DJI: the unit's clock), read at analysis.
    #[serde(default)]
    pub clock: Option<DateTime<Utc>>,
    /// Staged copies of files that belong to the clip (a DJI `.SRT`). They are kept next
    /// to the original.
    #[serde(default)]
    pub sidecars: Vec<PathBuf>,
    /// The card file's modified time, when it has one.
    #[serde(default)]
    pub mtime: Option<DateTime<Utc>>,
    /// Set on the first file of a recording the DVR split into several files (see `join`).
    #[serde(default)]
    pub join: Option<crate::join::Join>,
    /// Set on a later file of a joined recording: the id of the clip it is part of. It
    /// imports as part of that clip, not on its own.
    #[serde(default)]
    pub part_of: Option<usize>,
}

/// A clip's content fingerprint: size plus the first and last MB (see `identity`). Two
/// different flights with the same file name (DVR numbering restarts after a format, or a
/// Finder copy) get different fingerprints. It is written into the library as the clip's
/// identity (`app.quadcam.source`), so it is a specified hash.
pub fn fingerprint(path: &Path) -> Result<String> {
    crate::identity::fingerprint(path)
}

impl Clip {
    /// The file that gets converted and verified against: the `ffconcat` list of a joined
    /// recording, the repaired copy of a half-written clip, else the staged copy.
    pub fn source(&self) -> Option<&Path> {
        self.join
            .as_ref()
            .filter(|j| j.on)
            .map(|j| j.list.as_path())
            .or(self.recovered.as_deref())
            .or(self.staged.as_deref())
    }

    /// Bytes of what gets converted (every file of a joined recording).
    pub fn source_bytes(&self) -> u64 {
        match self.join.as_ref().filter(|j| j.on) {
            Some(j) => j.bytes,
            None => self
                .source()
                .and_then(|p| p.metadata().ok())
                .map(|m| m.len())
                .unwrap_or(self.size),
        }
    }
}

/// Staged name for a card file. Files with the same name in different folders get the
/// folder names prefixed so they cannot collide.
fn staged_name(c: &FoundClip, seen: &mut HashSet<String>) -> String {
    let lower = c.name.to_lowercase();
    if seen.insert(lower) {
        return c.name.clone();
    }
    let flat = c.rel.replace(['/', '\\'], "_");
    seen.insert(flat.to_lowercase());
    flat
}

/// Copies one file with progress. Deletes the partial copy on error.
fn copy_with_progress(src: &Path, dst: &Path, on_bytes: &mut dyn FnMut(u64)) -> Result<()> {
    let mut run = || -> Result<()> {
        let mut r = std::fs::File::open(src)?;
        let mut w = std::fs::File::create(dst)?;
        let mut buf = vec![0u8; 4 << 20];
        let mut done = 0u64;
        loop {
            let n = r.read(&mut buf)?;
            if n == 0 {
                break;
            }
            w.write_all(&buf[..n])?;
            done += n as u64;
            on_bytes(done);
        }
        w.sync_all()?;
        Ok(())
    };
    let res = run();
    if res.is_err() {
        let _ = std::fs::remove_file(dst);
    }
    res
}

/// Copies every clip on the card into `staging` before anything else. A failed copy
/// (card pulled) stops that file only, keeps what staged, and records the error.
/// A file already staged with the same content (size and fingerprint) is not copied again.
pub fn stage(
    card: &Path,
    staging: &Path,
    on_progress: &mut dyn FnMut(usize, usize, u64, u64),
) -> Result<Vec<Clip>> {
    let source = crate::sources::for_root(card);
    stage_found(source.list(card), source.kind(), staging, on_progress)
}

/// `stage` for clips already found: a card's, or files a person dropped.
pub fn stage_found(
    found: Vec<FoundClip>,
    kind: SourceKind,
    staging: &Path,
    on_progress: &mut dyn FnMut(usize, usize, u64, u64),
) -> Result<Vec<Clip>> {
    std::fs::create_dir_all(staging).with_context(|| format!("creating {}", staging.display()))?;
    let source = crate::sources::get(kind);
    let total = found.len();
    let mut seen = HashSet::new();
    let mut clips = Vec::with_capacity(total);
    for (i, f) in found.into_iter().enumerate() {
        let dst = staging.join(staged_name(&f, &mut seen));
        let mut clip = Clip {
            id: i,
            name: f.name.clone(),
            rel: f.rel.clone(),
            card_path: f.path.clone(),
            size: f.size,
            staged: None,
            stage_error: None,
            status: ClipStatus::Empty,
            duration: 0.0,
            recovered: None,
            probe: None,
            thumb: None,
            detail: String::new(),
            signal: None,
            key: String::new(),
            kind,
            clock: None,
            sidecars: Vec::new(),
            mtime: f
                .path
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .map(DateTime::<Utc>::from),
            join: None,
            part_of: None,
        };
        // Reuse a staged copy only when it is the same content, not just the same name.
        let already = dst.metadata().is_ok_and(|m| m.len() == f.size)
            && fingerprint(&dst)
                .ok()
                .is_some_and(|a| fingerprint(&f.path).ok() == Some(a));
        let res = if already {
            Ok(())
        } else {
            copy_with_progress(&f.path, &dst, &mut |b| on_progress(i, total, b, f.size))
        };
        match res {
            Ok(()) if dst.metadata().is_ok_and(|m| m.len() == f.size) => clip.staged = Some(dst),
            Ok(()) => {
                let _ = std::fs::remove_file(&dst);
                clip.stage_error = Some("copy came out the wrong size".into());
            }
            Err(e) => clip.stage_error = Some(format!("copy failed: {e:#}")),
        }
        // Sidecars follow the clip's staged name; one that fails to copy is left out.
        if let Some(staged) = clip.staged.clone() {
            for side in source.sidecars(&f.path) {
                let ext = side.extension().unwrap_or_default().to_string_lossy();
                let dst = staged.with_extension(ext.as_ref());
                if dst.metadata().ok().map(|m| m.len()) == side.metadata().ok().map(|m| m.len())
                    || std::fs::copy(&side, &dst).is_ok()
                {
                    clip.sidecars.push(dst);
                }
            }
        }
        on_progress(i, total, f.size, f.size);
        clips.push(clip);
    }
    Ok(clips)
}

fn mmss(secs: f64) -> String {
    let s = secs.round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Probes a staged clip, detects half-written files and recovers them, and makes a thumbnail.
pub fn analyse(tools: &Tools, clip: &mut Clip, cache: &Path) -> Result<()> {
    let Some(staged) = clip.staged.clone() else {
        clip.status = ClipStatus::Empty;
        clip.detail = clip
            .stage_error
            .clone()
            .unwrap_or_else(|| "not staged".into());
        return Ok(());
    };
    if clip.size == 0 {
        clip.status = ClipStatus::Empty;
        clip.detail = "zero bytes".into();
        return Ok(());
    }
    let source = crate::sources::get(clip.kind);
    clip.clock = source.intrinsic_time(&staged);
    let whole = source.inspect(&staged)?.complete;
    let probe = media::probe(tools, &staged).ok();
    let readable = probe.as_ref().is_some_and(|p| p.video_packets > 0);
    let probe_errors = probe.as_ref().is_some_and(|p| !p.errors.is_empty());

    if whole && readable && !probe_errors {
        let p = probe.unwrap();
        clip.status = ClipStatus::Ok;
        clip.duration = p.duration;
        clip.detail = format!("{} frames", p.video_packets);
        clip.probe = Some(p);
    } else {
        // Half-written (for analog: RIFF size wrong, no idx1), or ffprobe complained.
        let rec = source.repair_path(&staged);
        let recovered = source
            .repair(tools, &staged, &rec)
            .and_then(|_| media::probe(tools, &rec));
        match recovered {
            Ok(p) if p.video_packets > 0 => {
                clip.status = ClipStatus::Incomplete;
                clip.duration = p.duration;
                clip.detail = format!("incomplete, recovered {}", mmss(p.duration));
                clip.probe = Some(p);
                clip.recovered = Some(rec);
            }
            _ => {
                let _ = std::fs::remove_file(&rec);
                clip.status = ClipStatus::Empty;
                clip.detail = "unreadable".into();
                return Ok(());
            }
        }
    }
    clip.key = fingerprint(&staged).unwrap_or_default();
    let thumb = cache.join(format!(
        "{}-{}.jpg",
        staged.file_stem().unwrap_or_default().to_string_lossy(),
        clip.key
    ));
    if media::thumbnail(tools, clip.source().unwrap(), &thumb).is_ok() {
        clip.thumb = Some(thumb);
    }
    // Dead air is a suggestion only; a clip that cannot be sampled imports as usual.
    let fps = clip.probe.as_ref().and_then(|p| p.fps);
    clip.signal = source.signal(tools, clip.source().unwrap(), fps, clip.duration);
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum DateSource {
    Log,
    Import,
    Edited,
    /// The clip's own clock (DJI).
    Clip,
}

impl DateSource {
    pub fn label(self) -> &'static str {
        match self {
            DateSource::Log => "radio log",
            DateSource::Import => "import",
            DateSource::Edited => "edited",
            DateSource::Clip => "clip clock",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct DateSuggestion {
    pub date: NaiveDate,
    /// From a radio log (the start of the first claimed armed segment) or the clip clock.
    pub time: Option<NaiveTime>,
    pub source: DateSource,
    pub badge: Badge,
    pub segments: usize,
    /// Moments from the stick channels of the claimed log rows, timed from the first armed
    /// row (clip time when the clip starts at arm).
    #[serde(default)]
    pub moments: Vec<Moment>,
    /// Median seconds between the claimed log rows.
    #[serde(default)]
    pub log_interval_s: Option<f64>,
    /// The EdgeTX model name of the claimed log.
    #[serde(default)]
    pub log_model: Option<String>,
    #[serde(default)]
    pub flight: Option<FlightStats>,
    /// Why the log matched (or how sure), in a few words.
    #[serde(default)]
    pub match_reason: Option<String>,
}

/// Flight numbers from the log rows inside `windows`. `zero` is the log time at clip
/// second 0 (the first armed row): the packs' ranges are in seconds from it.
pub fn flight_stats(
    rows: &[logs::LogRow],
    windows: &[(NaiveDateTime, NaiveDateTime)],
    zero: NaiveDateTime,
) -> FlightStats {
    let secs = |t: NaiveDateTime| ((t - zero).num_milliseconds() as f64).round() / 1000.0;
    let inside: Vec<&logs::LogRow> = rows
        .iter()
        .filter(|r| windows.iter().any(|(a, b)| r.time >= *a && r.time <= *b))
        .collect();
    let min = |f: &dyn Fn(&logs::LogRow) -> Option<f64>| {
        inside
            .iter()
            .filter_map(|r| f(r))
            .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.min(v))))
    };
    FlightStats {
        armed_s: windows
            .iter()
            .map(|(a, b)| (*b - *a).num_milliseconds() as f64 / 1000.0)
            .sum(),
        packs: windows.len(),
        // 0 V and 0 % mean no telemetry yet, not a reading.
        min_rx_bat_v: min(&|r| r.rx_bat.filter(|v| *v > 0.0)),
        min_lq: min(&|r| r.lq.filter(|v| *v > 0.0)),
        min_rssi_db: min(&|r| r.rssi.filter(|v| *v < 0.0)),
        max_throttle: inside
            .iter()
            .filter_map(|r| {
                r.sticks
                    .map(|s| ((s.thr + 1024.0) / 2048.0).clamp(0.0, 1.0))
            })
            .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v)))),
        pack_spans: windows
            .iter()
            .map(|(a, b)| Span {
                start: secs(*a),
                end: secs(*b),
            })
            .collect(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct DatePlan {
    pub suggestions: Vec<DateSuggestion>,
    pub warnings: Vec<String>,
    /// Days that have log rows, newest first, for the day picker.
    pub log_days: Vec<NaiveDate>,
    pub day_used: Option<NaiveDate>,
}

/// What dating needs from one clip.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DateInput {
    pub name: String,
    pub duration: f64,
    /// The clip's own clock (`Clip::clock`).
    pub clock: Option<DateTime<Utc>>,
    /// The source, and the clip's own profile: they pick which logs' EdgeTX models fit.
    pub kind: SourceKind,
    pub profile: Option<String>,
    /// The flying day, when known apart from the log (a library clip's date).
    pub day: Option<NaiveDate>,
}

impl DateInput {
    /// A clip without a clock (analog).
    pub fn duration(duration: f64) -> DateInput {
        DateInput {
            duration,
            ..Default::default()
        }
    }

    pub fn of(c: &Clip, kind: SourceKind, profile: Option<&str>) -> DateInput {
        DateInput {
            name: c.name.clone(),
            duration: c.duration,
            clock: c.clock,
            kind,
            profile: profile.map(str::to_string),
            day: None,
        }
    }
}

/// The earliest year a clip clock is believed. A clock before it, or after tomorrow, was
/// reset (no time set since the battery ran down).
pub const CLIP_CLOCK_MIN_YEAR: i32 = 2015;

/// Suggests a date per clip, without profiles (any log model fits any clip).
pub fn plan_dates(
    clips: &[DateInput],
    log_dir: Option<&Path>,
    day: Option<NaiveDate>,
    import_day: NaiveDate,
    tun: &Tunables,
) -> DatePlan {
    plan_dates_with(clips, log_dir, day, import_day, tun, &[])
}

/// Suggests a date per clip. A clip with a believable clock of its own takes it ("clip
/// clock"); every other clip gets the import date. Radio logs then match by shape
/// (`logmatch`): pack lengths, order and gaps, with the profiles' EdgeTX models as a
/// filter. A match gives the clip its flight numbers and moments, and its date and time
/// when the radio clock is believable (for a clocked clip, only within `clock_skew_s` of
/// its clock). A log of a reset radio clock still matches; the clip keeps its own date.
pub fn plan_dates_with(
    clips: &[DateInput],
    log_dir: Option<&Path>,
    day: Option<NaiveDate>,
    import_day: NaiveDate,
    tun: &Tunables,
    profiles: &[md::Profile],
) -> DatePlan {
    let mut warnings = Vec::new();
    let tomorrow = import_day.succ_opt().unwrap_or(import_day);
    // Clip clocks in local time, the time EdgeTX logs are in.
    let clocks: Vec<Option<NaiveDateTime>> = clips
        .iter()
        .map(|c| {
            let t = c.clock?.with_timezone(&Local).naive_local();
            if t.year() < CLIP_CLOCK_MIN_YEAR || t.date() > tomorrow {
                warnings.push(format!(
                    "{}: the clip clock reads {} (the clock may have reset). Ignoring it.",
                    c.name,
                    t.format("%Y-%m-%d %H:%M")
                ));
                return None;
            }
            Some(t)
        })
        .collect();
    let unmatched = |date, time, source| DateSuggestion {
        date,
        time,
        source,
        badge: Badge::Unmatched,
        segments: 0,
        moments: Vec::new(),
        log_interval_s: None,
        log_model: None,
        flight: None,
        match_reason: None,
    };
    let mut plan = DatePlan {
        suggestions: clocks
            .iter()
            .map(|c| match c {
                Some(t) => unmatched(t.date(), Some(t.time()), DateSource::Clip),
                None => unmatched(import_day, None, DateSource::Import),
            })
            .collect(),
        warnings,
        log_days: Vec::new(),
        day_used: None,
    };
    let Some(dir) = log_dir else { return plan };
    let files = logmatch::read_files(dir);
    let days: std::collections::BTreeSet<NaiveDate> = files
        .iter()
        .flat_map(|f| f.rows.iter().map(|r| r.time.date()))
        .collect();
    plan.log_days = days.iter().rev().copied().collect();
    if days.is_empty() {
        plan.warnings
            .push(format!("No EdgeTX logs found in {}.", dir.display()));
        return plan;
    }
    // Default: the newest clip-clock day with logs, else the newest plausible day on or
    // before the import date, else the newest day of a reset radio clock.
    let day = day
        .or_else(|| {
            clocks
                .iter()
                .flatten()
                .map(NaiveDateTime::date)
                .filter(|d| days.contains(d))
                .max()
        })
        .or_else(|| {
            days.iter()
                .rev()
                .find(|d| **d <= import_day && logs::day_is_plausible(**d, import_day, tun))
                .copied()
        })
        .or_else(|| days.iter().rev().find(|d| d.year() < 2020).copied());
    let Some(day) = day else {
        plan.warnings.push(
            "Radio log dates look wrong (the radio clock may have reset). Using the import date."
                .into(),
        );
        return plan;
    };
    let files = logmatch::files_of_day(&files, day);
    if files.is_empty() {
        plan.warnings.push(format!("No log rows for {day}."));
        return plan;
    }
    plan.day_used = Some(day);
    let clock_ok = logs::day_is_plausible(day, import_day, tun);
    if !clock_ok {
        plan.warnings.push(format!(
            "Log day {day} is wrong or far from today (the radio clock may have reset). Matching by pack lengths; clips keep their own date."
        ));
    }
    let segs = logmatch::segments(&files, tun);
    // Clips in recording order: by clock when every clip has one, else as given.
    let mut order: Vec<usize> = (0..clips.len()).collect();
    if clocks.iter().all(Option::is_some) {
        order.sort_by_key(|i| clocks[*i]);
    }
    // With a believable radio clock, a clip clock of another day says nothing about it.
    let takes_part = |i: usize| !(clock_ok && clocks[i].is_some_and(|c| c.date() != day));
    let order: Vec<usize> = order.into_iter().filter(|i| takes_part(*i)).collect();
    let wants: Vec<logmatch::Want> = order
        .iter()
        .map(|&i| logmatch::Want {
            duration: clips[i].duration,
            clock: clocks[i],
            models: logmatch::clip_models(clips[i].profile.as_deref(), clips[i].kind, profiles),
            day: clips[i].day,
        })
        .collect();
    let found = logmatch::match_all(&wants, &segs, &files, profiles, clock_ok, tun);
    for (&i, f) in order.iter().zip(found) {
        let Some(f) = f else { continue };
        let claimed = &segs[f.first..f.first + f.segments];
        let file = &files[claimed[0].file];
        let rows = &file.rows[claimed[0].a..claimed[claimed.len() - 1].b];
        let windows: Vec<(NaiveDateTime, NaiveDateTime)> =
            claimed.iter().map(|g| (g.start, g.end)).collect();
        let log = RadioLog::from_rows(rows, &windows, f.start, 0.0);
        let s = &mut plan.suggestions[i];
        // The log dates the clip only when its clock is believable, and, for a clip with
        // its own clock, close to it.
        let dates = clock_ok
            && match f.skew_s {
                Some(k) if k.abs() > tun.clock_skew_s => {
                    plan.warnings.push(format!(
                        "{}: the matching radio log starts {:.0} s from the clip clock (more than {} s). Keeping the clip clock.",
                        clips[i].name,
                        k.abs(),
                        tun.clock_skew_s
                    ));
                    false
                }
                _ => true,
            };
        if dates {
            s.date = f.start.date();
            s.time = Some(f.start.time());
            s.source = DateSource::Log;
        }
        s.badge = f.badge;
        s.segments = f.segments;
        s.moments = log.moments();
        s.log_interval_s = log.interval();
        s.log_model = f.model.clone();
        s.flight = Some(flight_stats(rows, &windows, f.start));
        s.match_reason = Some(f.reason);
    }
    plan
}

/// creation_time for a clip: its time of day (the log start time, the clip clock, or one
/// set by hand), otherwise local noon of the date so a UTC conversion cannot roll the day.
pub fn creation_time(
    date: NaiveDate,
    time: Option<NaiveTime>,
    source: DateSource,
) -> DateTime<Utc> {
    let t = match (source, time) {
        (DateSource::Import, _) | (_, None) => NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        (_, Some(t)) => t,
    };
    let local = NaiveDateTime::new(date, t);
    Local
        .from_local_datetime(&local)
        .earliest()
        .unwrap_or_else(|| Local.from_utc_datetime(&local))
        .with_timezone(&Utc)
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ClipJob {
    pub id: usize,
    pub skip: bool,
    /// `YYYY-MM-DD`.
    pub date: String,
    /// `HH:MM:SS`, from the radio log or set by hand.
    pub time: Option<String>,
    pub source: DateSource,
    pub name: String,
    pub note: String,
    /// Location, gear and keywords to write, after the profile is applied.
    #[serde(default)]
    pub meta: Resolved,
    /// The library's own QuickTime items (see `library::import_items`).
    #[serde(default)]
    pub extra: Vec<(String, String)>,
    /// The later files of a joined recording: (DVR file name, staged copy). Kept originals
    /// go next to the first file's as `<stem>.part2.avi`, and so on.
    #[serde(default)]
    pub parts: Vec<(String, PathBuf)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ImportSettings {
    pub output_dir: PathBuf,
    pub format: Format,
    pub encoder: Encoder,
    pub keep_originals: bool,
    pub add_time: bool,
    pub default_name: String,
    /// Saved places, aircraft profiles and the session's default profile, for metadata.
    #[serde(default)]
    pub places: Vec<md::Place>,
    #[serde(default)]
    pub profiles: Vec<md::Profile>,
    #[serde(default)]
    pub default_profile: Option<String>,
    /// How clips are filed under `output_dir` (the library folder).
    #[serde(default)]
    pub layout: Layout,
    /// Add the clip's place name to its day folder.
    #[serde(default)]
    pub place_folders: bool,
    /// This import's id (`YYYYMMDD-HHMMSS`), written into every file.
    #[serde(default)]
    pub import_id: String,
    /// How the date starts file names.
    #[serde(default)]
    pub name_date_format: naming::DateFormat,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Verified,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ClipResult {
    pub id: usize,
    pub outcome: Outcome,
    pub output: Option<PathBuf>,
    pub original: Option<PathBuf>,
    pub size: u64,
    pub error: Option<String>,
    /// How the file was made: the encoder that ran, or `copy` or `remux`.
    pub encoder: Option<crate::media::Encoded>,
    /// The metadata written, so a later verify checks exactly what went into the file.
    #[serde(default)]
    pub meta: Option<Meta>,
    /// One extra output per cut range of the clip.
    #[serde(default)]
    pub cuts: Vec<CutResult>,
    /// The QuickTime items written (see `qtmeta`), so a later verify checks them too.
    #[serde(default)]
    pub qt: Vec<(String, String)>,
}

/// One cut range written as its own file next to the clip's output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct CutResult {
    pub start: f64,
    pub end: f64,
    pub outcome: Outcome,
    pub output: Option<PathBuf>,
    pub size: u64,
    pub error: Option<String>,
}

pub use crate::paths::{default_output_dir, DEFAULT_OUTPUT_REL};

/// Free bytes on the volume holding `dir` (`df -Pk`).
pub fn free_bytes(dir: &Path) -> Result<u64> {
    let out = std::process::Command::new("/bin/df")
        .arg("-Pk")
        .arg(dir)
        .output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().nth(1).context("df printed nothing")?;
    let avail: u64 = line
        .split_whitespace()
        .nth(3)
        .context("df format")?
        .parse()?;
    Ok(avail * 1024)
}

/// Expected bytes: the source sizes (MP4 comes out far smaller, MOV the same), plus the
/// originals when kept, plus 5%. Deliberately an over-estimate.
pub fn expected_bytes(clips: &[&Clip], keep_originals: bool) -> u64 {
    let src: u64 = clips.iter().map(|c| c.source_bytes()).sum();
    let total = if keep_originals { src * 2 } else { src };
    total + total / 20
}

/// Checks the output folder exists and has room.
pub fn preflight(settings: &ImportSettings, clips: &[&Clip]) -> Result<()> {
    let dir = &settings.output_dir;
    // The default folder is created on first use; a folder the user picked must exist.
    if Some(dir) == default_output_dir().as_ref() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    if !dir.is_dir() {
        bail!("Output folder {} does not exist.", dir.display());
    }
    let need = expected_bytes(clips, settings.keep_originals);
    let free = free_bytes(dir)?;
    if free < need {
        bail!(
            "Output folder has {:.1} GB free; this import needs about {:.1} GB.",
            free as f64 / 1e9,
            need as f64 / 1e9
        );
    }
    Ok(())
}

pub fn meta_for(clip: &Clip, job: &ClipJob, date: NaiveDate, time: Option<NaiveTime>) -> Meta {
    Meta {
        title: job.name.trim().to_string(),
        comment: job.note.trim().to_string(),
        creation_time: creation_time(date, time, job.source),
        date: date.format("%Y-%m-%d").to_string(),
        description: format!(
            "{} {}; date source: {}",
            clip.kind.file_label(),
            std::iter::once(clip.name.as_str())
                .chain(job.parts.iter().map(|(n, _)| n.as_str()))
                .collect::<Vec<_>>()
                .join(" + "),
            job.source.label()
        ),
    }
}

/// Converts one clip, verifies it, sets metadata and mtime, and keeps the original if asked.
/// A failed verify deletes the output and keeps the source.
pub fn import_clip(
    tools: &Tools,
    clip: &Clip,
    job: &ClipJob,
    settings: &ImportSettings,
    planner: &mut NamePlanner,
    on_progress: &mut dyn FnMut(f64),
) -> ClipResult {
    let mut r = ClipResult {
        id: clip.id,
        outcome: Outcome::Failed,
        output: None,
        original: None,
        size: 0,
        error: None,
        encoder: None,
        meta: None,
        cuts: Vec::new(),
        qt: Vec::new(),
    };
    let fail = |mut r: ClipResult, e: String| {
        r.error = Some(e);
        r
    };
    let (Some(src), Some(src_probe)) = (clip.source(), clip.probe.as_ref()) else {
        return fail(r, "clip was not staged or could not be read".into());
    };
    let Ok(date) = NaiveDate::parse_from_str(job.date.trim(), "%Y-%m-%d") else {
        return fail(r, format!("date {:?} is not YYYY-MM-DD", job.date));
    };
    let time = match (job.source, job.time.as_deref()) {
        (DateSource::Import, _) => None,
        (_, Some(t)) => NaiveTime::parse_from_str(t, "%H:%M:%S%.f").ok(),
        _ => None,
    };
    let meta = meta_for(clip, job, date, time);
    let hhmm = if settings.add_time {
        time.map(|t| t.format("%H%M").to_string())
    } else {
        None
    };
    let stem = naming::stem(
        &settings.name_date_format.format(date),
        hhmm.as_deref(),
        &job.name,
        &settings.default_name,
    );
    let place = settings
        .place_folders
        .then(|| job.meta.location.as_ref().and_then(|l| l.name.as_deref()))
        .flatten();
    let dir = library::day_dir(&settings.output_dir, settings.layout, date, place);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return fail(r, format!("creating {}: {e}", dir.display()));
    }
    let out = planner.claim(&dir, &stem, settings.format.ext());
    // Write under a hidden temporary name; rename only after verify passes.
    let tmp = crate::cuts::part_path(&out);

    let source = crate::sources::get(clip.kind);
    let plan = source.encode_plan(src_probe, settings.format);
    let enc = match media::convert(
        tools,
        src,
        &tmp,
        settings.format,
        plan,
        settings.encoder,
        &meta,
        on_progress,
    ) {
        Ok(e) => e,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return fail(r, format!("{e:#}"));
        }
    };
    r.encoder = Some(enc);
    r.meta = Some(meta.clone());
    let mut qt = md::qt_items(&job.meta, &meta);
    qt.extend(job.extra.iter().cloned());
    let written = crate::cuts::write_qt(&tmp, &qt)
        .and_then(|_| media::verify(tools, src_probe, &tmp, &meta))
        .and_then(|_| media::verify_qt(tools, &tmp, &qt));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return fail(r, format!("verify failed: {e:#}"));
    }
    r.qt = qt;
    if out.exists() {
        let _ = std::fs::remove_file(&tmp);
        return fail(
            r,
            format!("{} appeared during import; not overwriting", out.display()),
        );
    }
    if let Err(e) = std::fs::rename(&tmp, &out).context("renaming output") {
        let _ = std::fs::remove_file(&tmp);
        return fail(r, format!("{e:#}"));
    }
    let _ = media::set_mtime(&out, meta.creation_time);
    r.size = out.metadata().map(|m| m.len()).unwrap_or(0);
    r.output = Some(out.clone());

    if settings.keep_originals {
        let dir = dir.join(library::ORIGINALS);
        let orig_stem = out.file_stem().unwrap().to_string_lossy().to_string();
        let copy = std::fs::create_dir_all(&dir)
            .map_err(anyhow::Error::from)
            .and_then(|_| {
                let staged = clip.staged.as_deref().unwrap_or(src);
                let dst = planner.claim(&dir, &orig_stem, &source.original_ext(staged));
                std::fs::copy(staged, &dst)?;
                if dst.metadata()?.len() != staged.metadata()?.len() {
                    bail!("original copy came out the wrong size");
                }
                let _ = media::set_mtime(&dst, meta.creation_time);
                // The later files of a joined recording sit next to it as `.part2`, `.part3`.
                let ext = dst
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                for (n, (_, part)) in job.parts.iter().enumerate() {
                    let to = dst.with_extension(format!("part{}.{ext}", n + 2));
                    if to.exists() {
                        bail!("{} exists already; not overwriting", to.display());
                    }
                    std::fs::copy(part, &to)
                        .with_context(|| format!("keeping {}", part.display()))?;
                    if to.metadata()?.len() != part.metadata()?.len() {
                        bail!(
                            "original copy of {} came out the wrong size",
                            part.display()
                        );
                    }
                    let _ = media::set_mtime(&to, meta.creation_time);
                }
                // Sidecars sit next to the original, under its stem.
                for side in &clip.sidecars {
                    let ext = side.extension().unwrap_or_default().to_string_lossy();
                    let to = dst.with_extension(ext.to_lowercase());
                    if !to.exists() {
                        std::fs::copy(side, &to)
                            .with_context(|| format!("keeping {}", side.display()))?;
                    }
                }
                Ok(dst)
            });
        match copy {
            Ok(p) => r.original = Some(p),
            // The converted file verified; losing the archive copy still blocks the format.
            Err(e) => {
                return fail(
                    r,
                    format!("output verified, but keeping the original failed: {e:#}"),
                )
            }
        }
    }
    r.outcome = Outcome::Verified;
    r
}

/// The format button unlocks only when every clip staged and every non-skipped clip verified.
pub fn can_format(clips: &[Clip], results: &[ClipResult]) -> Result<()> {
    if clips.is_empty() {
        bail!("No clips were read from this card.");
    }
    if let Some(c) = clips.iter().find(|c| c.stage_error.is_some()) {
        bail!(
            "{} did not copy off the card ({}). Never format.",
            c.name,
            c.stage_error.as_deref().unwrap_or("")
        );
    }
    for c in clips {
        // A part of a joined recording imports with its first file.
        let id = c.part_of.unwrap_or(c.id);
        match results.iter().rev().find(|r| r.id == id).map(|r| r.outcome) {
            Some(Outcome::Verified | Outcome::Skipped) => {}
            Some(Outcome::Failed) => bail!("{} failed to import.", c.name),
            None => bail!("{} has not been imported.", c.name),
        }
    }
    Ok(())
}

/// Writes each cut range of a verified clip as `<output stem>_cutN.<ext>` next to its output,
/// and verifies it. A cut that already verified with the same range keeps its file and is not
/// written again. Never overwrites: a taken name gets `-2`, `-3`.
pub fn export_cuts(
    tools: &Tools,
    clip: &Clip,
    main: &ClipResult,
    cuts: &[Span],
    settings: &ImportSettings,
    planner: &mut NamePlanner,
) -> Vec<CutResult> {
    let (Some(out), Some(meta)) = (main.output.as_deref(), main.meta.as_ref()) else {
        return Vec::new();
    };
    let dir = out.parent().unwrap_or(&settings.output_dir);
    let stem = out
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let ext = out
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let format = crate::cuts::format_for_ext(&ext);
    let same = |a: f64, b: f64| (a - b).abs() < 0.001;
    cuts.iter()
        .enumerate()
        .map(|(i, span)| {
            if let Some(prev) = main.cuts.iter().find(|c| {
                c.outcome == Outcome::Verified
                    && same(c.start, span.start)
                    && same(c.end, span.end)
                    && c.output.as_deref().is_some_and(Path::is_file)
            }) {
                return prev.clone();
            }
            let mut r = CutResult {
                start: span.start,
                end: span.end,
                outcome: Outcome::Failed,
                output: None,
                size: 0,
                error: None,
            };
            let (Some(src), Some(probe)) = (clip.source(), clip.probe.as_ref()) else {
                r.error = Some("clip was not staged or could not be read".into());
                return r;
            };
            let dst = planner.claim(dir, &format!("{stem}_cut{}", i + 1), &ext);
            let cut_meta = Meta {
                creation_time: meta.creation_time
                    + chrono::Duration::milliseconds((span.start * 1000.0) as i64),
                description: format!(
                    "{}; cut {:.1}-{:.1} s",
                    meta.description, span.start, span.end
                ),
                ..meta.clone()
            };
            // The clip's QuickTime items, with this cut's start time and description.
            let mut qt: Vec<(String, String)> = main
                .qt
                .iter()
                .map(|(k, v)| {
                    let v = match k.as_str() {
                        "com.apple.quicktime.creationdate" => {
                            md::creation_date(cut_meta.creation_time)
                        }
                        "com.apple.quicktime.description" => cut_meta.description.clone(),
                        _ => v.clone(),
                    };
                    (k.clone(), v)
                })
                .collect();
            crate::qtmeta::set(
                &mut qt,
                library::KEY_CUT,
                &format!("{:.3}-{:.3}", span.start, span.end),
            );
            let cut = crate::cuts::Cut {
                src,
                probe,
                span: *span,
                format,
                encoder: settings.encoder,
                meta: &cut_meta,
                qt: &qt,
            };
            match crate::cuts::write_cut(tools, &cut, &dst) {
                Ok(size) => {
                    r.size = size;
                    r.output = Some(dst);
                    r.outcome = Outcome::Verified;
                }
                Err(e) => r.error = Some(format!("{e:#}")),
            }
            r
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Fixed bytes for the identity test vectors.
    pub(crate) fn pattern(len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| (i.wrapping_mul(31) ^ (i >> 7)) as u8)
            .collect()
    }

    /// A local wall-clock time on 2026-09-28 as the UTC instant a clip clock holds.
    fn local(hms: &str) -> DateTime<Utc> {
        let t = NaiveDateTime::parse_from_str(&format!("2026-09-28 {hms}"), "%Y-%m-%d %H:%M:%S")
            .unwrap();
        Local
            .from_local_datetime(&t)
            .earliest()
            .unwrap()
            .with_timezone(&Utc)
    }

    fn clocked(name: &str, duration: f64, clock: Option<DateTime<Utc>>) -> DateInput {
        DateInput {
            name: name.into(),
            duration,
            clock,
            ..Default::default()
        }
    }

    /// One pack, 10:00:00 to 10:03:00 on 2026-09-28, a row a second.
    fn one_pack_log() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("LOGS")).unwrap();
        let mut csv = String::from("Date,Time,1RSS(dB),RQly(%)\n");
        for s in 0..=180 {
            csv.push_str(&format!(
                "2026-09-28,10:{:02}:{:02}.000,-50,100\n",
                s / 60,
                s % 60
            ));
        }
        std::fs::write(d.path().join("LOGS/Quad-2026-09-28-100000.csv"), csv).unwrap();
        d
    }

    #[test]
    fn clip_clock_dates_and_reset_clocks_are_ignored() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        let tun = Tunables::default();
        let reset = Local
            .with_ymd_and_hms(2000, 1, 1, 0, 1, 0)
            .unwrap()
            .with_timezone(&Utc);
        let ahead = Local
            .with_ymd_and_hms(2026, 10, 2, 9, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        let tomorrow = Local
            .with_ymd_and_hms(2026, 10, 1, 9, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        let plan = plan_dates(
            &[
                clocked("A.MP4", 60.0, Some(local("18:36:36"))),
                clocked("B.MP4", 60.0, Some(reset)),
                clocked("C.MP4", 60.0, Some(ahead)),
                clocked("D.MP4", 60.0, Some(tomorrow)),
                DateInput::duration(60.0),
            ],
            None,
            None,
            today,
            &tun,
        );
        let s = &plan.suggestions;
        assert_eq!(s[0].source, DateSource::Clip);
        assert_eq!(s[0].date.to_string(), "2026-09-28");
        assert_eq!(s[0].time.unwrap().to_string(), "18:36:36");
        assert_eq!(DateSource::Clip.label(), "clip clock");
        for i in [1, 2] {
            assert_eq!(s[i].source, DateSource::Import, "reset clock {i}");
            assert_eq!(s[i].date, today);
        }
        assert_eq!(s[3].source, DateSource::Clip, "tomorrow is still believed");
        assert_eq!(s[4].source, DateSource::Import);
        assert_eq!(plan.warnings.len(), 2);
        assert!(plan.warnings[0].starts_with("B.MP4: the clip clock reads 2000-01-01"));
        assert!(plan.warnings[1].contains("C.MP4"));
        // The clip clock is the creation time, not local noon.
        assert_eq!(
            creation_time(s[0].date, s[0].time, s[0].source),
            local("18:36:36")
        );
    }

    #[test]
    fn a_log_dates_a_clocked_clip_only_within_the_skew() {
        let logs = one_pack_log();
        let today = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        let tun = Tunables::default();
        assert_eq!(tun.clock_skew_s, 300.0);
        let run = |clock: Option<DateTime<Utc>>| {
            plan_dates(
                &[clocked("DJI_1.MP4", 200.0, clock)],
                Some(logs.path()),
                None,
                today,
                &tun,
            )
        };
        // 61 s apart: the log wins, with its time and model.
        let p = run(Some(local("09:58:59")));
        assert_eq!(p.day_used, NaiveDate::from_ymd_opt(2026, 9, 28));
        assert_eq!(p.suggestions[0].source, DateSource::Log);
        assert_eq!(p.suggestions[0].time.unwrap().to_string(), "10:00:00");
        assert_eq!(p.suggestions[0].log_model.as_deref(), Some("Quad"));
        assert!(p.warnings.is_empty());
        // 30 minutes apart: the clip clock stays, and a warning names the skew.
        let p = run(Some(local("10:30:00")));
        assert_eq!(p.suggestions[0].source, DateSource::Clip);
        assert_eq!(p.suggestions[0].time.unwrap().to_string(), "10:30:00");
        assert_eq!(
            p.warnings,
            ["DJI_1.MP4: the matching radio log starts 1800 s from the clip clock (more than 300 s). Keeping the clip clock."]
        );
        // A wider skew lets it through.
        let wide = Tunables {
            clock_skew_s: 3600.0,
            ..Tunables::default()
        };
        let p = plan_dates(
            &[clocked("DJI_1.MP4", 200.0, Some(local("10:30:00")))],
            Some(logs.path()),
            None,
            today,
            &wide,
        );
        assert_eq!(p.suggestions[0].source, DateSource::Log);
        // A clip of another day keeps its clock without a warning.
        let other = Local
            .with_ymd_and_hms(2026, 9, 27, 10, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        let p = plan_dates(
            &[clocked("DJI_1.MP4", 200.0, Some(other))],
            Some(logs.path()),
            NaiveDate::from_ymd_opt(2026, 9, 28),
            today,
            &tun,
        );
        assert_eq!(p.suggestions[0].source, DateSource::Clip);
        assert!(p.warnings.is_empty());
        // A reset clock: the log dates it, as for analog.
        let reset = Local
            .with_ymd_and_hms(2000, 1, 1, 0, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        let p = run(Some(reset));
        assert_eq!(p.suggestions[0].source, DateSource::Log);
        assert_eq!(p.warnings.len(), 1);
    }

    /// A clip's fingerprint is persisted (`app.quadcam.source`) and compared on every card
    /// insert, so the same bytes must always give the same id.
    #[test]
    fn fingerprint_test_vector() {
        let d = tempfile::tempdir().unwrap();
        let small = d.path().join("small.avi");
        std::fs::write(&small, pattern(1000)).unwrap();
        let big = d.path().join("big.avi");
        std::fs::write(&big, pattern(3 * (1 << 20) + 123)).unwrap();
        assert_eq!(fingerprint(&small).unwrap(), "x1c63a88ed3c3c2d9");
        assert_eq!(fingerprint(&big).unwrap(), "x472bd6ccd6749ca4");
    }

    /// An EdgeTX CSV: rows every 0.5 s over each `(day, from, to)` span, in the order given
    /// (a reset radio clock writes times that go back).
    fn edgetx_csv(spans: &[(&str, &str, &str)]) -> String {
        let mut csv = String::from("Date,Time,1RSS(dB),RQly(%),RxBt(V)\n");
        for (day, a, b) in spans {
            let p = |s: &str| {
                NaiveDateTime::parse_from_str(&format!("{day} {s}"), "%Y-%m-%d %H:%M:%S%.f")
                    .unwrap()
            };
            let (mut t, b) = (p(a), p(b));
            while t <= b {
                csv.push_str(&format!(
                    "{},{},-60,99,4.1\n",
                    t.format("%Y-%m-%d"),
                    t.format("%H:%M:%S%.3f")
                ));
                t += chrono::Duration::milliseconds(500);
            }
        }
        csv
    }

    fn profile(name: &str, system: &str, models: &[&str]) -> md::Profile {
        md::Profile {
            name: name.into(),
            video_system: system.into(),
            edgetx_models: models.iter().map(|m| m.to_string()).collect(),
            ..Default::default()
        }
    }

    /// Three sessions on one day, the last never disarmed (the log ends armed). A DJI clip
    /// recorded during the last one matches it, not the day's first pack.
    #[test]
    fn a_dji_clip_matches_the_pack_at_its_clock_not_the_first_of_the_day() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("LOGS")).unwrap();
        std::fs::write(
            d.path().join("LOGS/METEOR75-2026-09-28.csv"),
            edgetx_csv(&[
                ("2026-09-28", "16:28:12.0", "16:28:13.0"),
                ("2026-09-28", "16:33:12.0", "16:33:19.5"),
                ("2026-09-28", "18:32:18.5", "18:34:47.5"),
                ("2026-09-28", "18:36:34.5", "18:38:28.0"),
            ]),
        )
        .unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
        let profiles = [
            profile("Whoop", "Analog", &["AIR65 II"]),
            profile("Meteor", "DJI", &["METEOR75"]),
        ];
        let clip = DateInput {
            name: "DJI_20260928183636_0001_D.MP4".into(),
            duration: 112.8,
            clock: Some(local("18:36:36")),
            kind: SourceKind::Dji,
            profile: None,
            day: None,
        };
        let p = plan_dates_with(
            &[clip],
            Some(d.path()),
            None,
            today,
            &Tunables::default(),
            &profiles,
        );
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        let s = &p.suggestions[0];
        assert_eq!(s.badge, Badge::Matched);
        assert_eq!(s.source, DateSource::Log);
        assert_eq!(s.time.unwrap().to_string(), "18:36:34.500");
        assert_eq!(s.segments, 1);
        let f = s.flight.as_ref().unwrap();
        assert_eq!((f.packs, f.armed_s), (1, 113.5));
        // The pack's armed range, in clip seconds from the first armed row.
        assert_eq!(
            f.pack_spans,
            vec![Span {
                start: 0.0,
                end: 113.5
            }]
        );
        assert!(s
            .match_reason
            .as_deref()
            .unwrap()
            .contains("log METEOR75 → profile Meteor"));
    }

    /// The radio clock battery was dead: every power-on starts again at 2000-01-01 00:00.
    /// Clips still match by pack length and order, and keep their own date.
    #[test]
    fn a_log_with_a_reset_clock_matches_by_shape() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("LOGS")).unwrap();
        std::fs::write(
            d.path().join("LOGS/AIR65 II-2000-01-01.csv"),
            edgetx_csv(&[
                // First power-on: one 3-minute pack.
                ("2000-01-01", "00:00:40", "00:03:40"),
                // Second power-on: the clock starts over. Packs of 100 s and 160 s.
                ("2000-01-01", "00:00:30", "00:02:10"),
                ("2000-01-01", "00:03:00", "00:05:40"),
            ]),
        )
        .unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        let profiles = [profile("Whoop", "Analog", &["AIR65 II"])];
        let analog = |duration| DateInput {
            duration,
            ..Default::default()
        };
        let p = plan_dates_with(
            &[analog(200.0), analog(115.0), analog(175.0)],
            Some(d.path()),
            None,
            today,
            &Tunables::default(),
            &profiles,
        );
        assert_eq!(p.day_used, NaiveDate::from_ymd_opt(2000, 1, 1));
        assert!(p.warnings[0].contains("clock may have reset"));
        let b: Vec<Badge> = p.suggestions.iter().map(|s| s.badge).collect();
        assert_eq!(b, [Badge::Matched; 3]);
        for s in &p.suggestions {
            assert_eq!(s.source, DateSource::Import, "the clip keeps its own date");
            assert_eq!(s.date, today);
            assert!(s.flight.is_some());
            assert!(s
                .match_reason
                .as_deref()
                .unwrap()
                .contains("radio clock wrong"));
        }
        // The second clip took the 100 s pack of the second power-on, not rows of the first.
        assert_eq!(p.suggestions[1].flight.as_ref().unwrap().armed_s, 100.0);
        // The third took the 160 s pack; its range starts at the clip's first armed row.
        let f = p.suggestions[2].flight.as_ref().unwrap();
        assert_eq!(
            f.pack_spans,
            vec![Span {
                start: 0.0,
                end: 160.0
            }]
        );
    }

    /// Two packs of the same length and one clip without a clock: either fits, so the match
    /// stays "likely" and says so.
    #[test]
    fn an_ambiguous_match_stays_likely() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("LOGS")).unwrap();
        std::fs::write(
            d.path().join("LOGS/Quad-2026-09-28.csv"),
            edgetx_csv(&[
                ("2026-09-28", "10:00:00", "10:02:00"),
                ("2026-09-28", "11:00:00", "11:02:00"),
            ]),
        )
        .unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        let p = plan_dates(
            &[DateInput::duration(130.0)],
            Some(d.path()),
            None,
            today,
            &Tunables::default(),
        );
        let s = &p.suggestions[0];
        assert_eq!(s.badge, Badge::Likely);
        let why = s.match_reason.as_deref().unwrap();
        assert!(why.contains("fits as well"), "{why}");
        assert!(why.contains("log Quad is in no profile"), "{why}");
    }

    /// A log whose model a profile lists matches only clips that profile fits; a model in
    /// no profile matches by shape alone.
    #[test]
    fn edgetx_models_pick_the_log_for_the_source() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("LOGS")).unwrap();
        // The analog quad flew a 120 s pack at 10:00:00, the DJI quad one at 10:00:05.
        std::fs::write(
            d.path().join("LOGS/AIR65 II-2026-09-28.csv"),
            edgetx_csv(&[("2026-09-28", "10:00:00", "10:02:00")]),
        )
        .unwrap();
        std::fs::write(
            d.path().join("LOGS/METEOR75-2026-09-28.csv"),
            edgetx_csv(&[("2026-09-28", "10:00:05", "10:02:05")]),
        )
        .unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        let profiles = [
            profile("Whoop", "Analog", &["AIR65 II"]),
            profile("Meteor", "DJI", &["METEOR75"]),
        ];
        let run = |kind, clock| {
            plan_dates_with(
                &[DateInput {
                    duration: 125.0,
                    clock,
                    kind,
                    ..Default::default()
                }],
                Some(d.path()),
                NaiveDate::from_ymd_opt(2026, 9, 28),
                today,
                &Tunables::default(),
                &profiles,
            )
            .suggestions
            .remove(0)
        };
        let analog = run(SourceKind::Analog, None);
        assert_eq!(analog.log_model.as_deref(), Some("AIR65 II"));
        assert_eq!(analog.time.unwrap().to_string(), "10:00:00");
        assert!(analog.match_reason.unwrap().contains("profile Whoop"));
        let dji = run(SourceKind::Dji, Some(local("10:00:00")));
        assert_eq!(dji.log_model.as_deref(), Some("METEOR75"));
        assert_eq!(dji.time.unwrap().to_string(), "10:00:05");
        // With no profiles, both logs are candidates; the analog clip's two choices tie.
        let p = plan_dates(
            &[DateInput::duration(125.0)],
            Some(d.path()),
            NaiveDate::from_ymd_opt(2026, 9, 28),
            today,
            &Tunables::default(),
        );
        assert_eq!(p.suggestions[0].badge, Badge::Likely);
    }
}
