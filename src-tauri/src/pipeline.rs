//! The import pipeline without any UI: stage, analyse, date, convert, verify.
//! `session` and `core` build on these.

use crate::library::{self, Layout};
use crate::logs::{self, Badge, Tunables};
use crate::media::{self, Encoder, Format, Meta, Probe, Tools};
use crate::metadata::{self as md, FlightStats, Resolved};
use crate::moments::{self, Moment, MomentSource, RadioLog, SignalScan, Span};
use crate::naming::{self, NamePlanner};
use crate::scan::{self, FoundClip};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClipStatus {
    Ok,
    Incomplete,
    Empty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// Bytes read from each end of a file for its fingerprint.
const FINGERPRINT_BYTES: u64 = 1 << 20;

/// A short content fingerprint: size plus the first and last MB. Two different flights
/// with the same file name (DVR numbering restarts after a format, or a Finder copy) get
/// different fingerprints. The hash is only a cache key; it need not be stable across
/// quadcam versions.
pub fn fingerprint(path: &Path) -> Result<String> {
    use std::hash::{Hash, Hasher};
    use std::io::{Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let mut h = std::hash::DefaultHasher::new();
    len.hash(&mut h);
    let mut buf = vec![0u8; FINGERPRINT_BYTES.min(len) as usize];
    f.read_exact(&mut buf)?;
    buf.hash(&mut h);
    if len > FINGERPRINT_BYTES {
        let tail = FINGERPRINT_BYTES.min(len - FINGERPRINT_BYTES);
        f.seek(SeekFrom::Start(len - tail))?;
        let mut buf = vec![0u8; tail as usize];
        f.read_exact(&mut buf)?;
        buf.hash(&mut h);
    }
    Ok(format!("{:016x}", h.finish()))
}

impl Clip {
    /// The file that gets converted and verified against.
    pub fn source(&self) -> Option<&Path> {
        self.recovered.as_deref().or(self.staged.as_deref())
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
    std::fs::create_dir_all(staging).with_context(|| format!("creating {}", staging.display()))?;
    let found = scan::find_clips(card);
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
    let avi = scan::check_avi(&staged)?;
    let probe = media::probe(tools, &staged).ok();
    let readable = probe.as_ref().is_some_and(|p| p.video_packets > 0);
    let probe_errors = probe.as_ref().is_some_and(|p| !p.errors.is_empty());

    if avi.complete() && readable && !probe_errors {
        let p = probe.unwrap();
        clip.status = ClipStatus::Ok;
        clip.duration = p.duration;
        clip.detail = format!("{} frames", p.video_packets);
        clip.probe = Some(p);
    } else {
        // Half-written: RIFF size wrong, no idx1, or ffprobe complained. Try recovery.
        let rec = staged.with_file_name(format!(
            "{}.recovered.avi",
            staged.file_stem().unwrap_or_default().to_string_lossy()
        ));
        let recovered =
            media::recover(tools, &staged, &rec).and_then(|_| media::probe(tools, &rec));
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
    clip.signal = moments::scan_signal(tools, clip.source().unwrap(), fps, clip.duration).ok();
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DateSource {
    Log,
    Import,
    Edited,
}

impl DateSource {
    pub fn label(self) -> &'static str {
        match self {
            DateSource::Log => "radio log",
            DateSource::Import => "import",
            DateSource::Edited => "edited",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DateSuggestion {
    pub date: NaiveDate,
    /// Only from a radio log: the start of the first claimed armed segment.
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
}

/// Flight numbers from the log rows inside `windows`.
pub fn flight_stats(
    rows: &[logs::LogRow],
    windows: &[(NaiveDateTime, NaiveDateTime)],
) -> FlightStats {
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
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatePlan {
    pub suggestions: Vec<DateSuggestion>,
    pub warnings: Vec<String>,
    /// Days that have log rows, newest first, for the day picker.
    pub log_days: Vec<NaiveDate>,
    pub day_used: Option<NaiveDate>,
}

/// Suggests a date per clip. Without logs, or with an implausible log day (radio clock
/// reset), every clip gets the import date.
pub fn plan_dates(
    durations: &[f64],
    log_dir: Option<&Path>,
    day: Option<NaiveDate>,
    import_day: NaiveDate,
    tun: &Tunables,
) -> DatePlan {
    let fallback = |_| DateSuggestion {
        date: import_day,
        time: None,
        source: DateSource::Import,
        badge: Badge::Unmatched,
        segments: 0,
        moments: Vec::new(),
        log_interval_s: None,
        log_model: None,
        flight: None,
    };
    let mut plan = DatePlan {
        suggestions: durations.iter().map(fallback).collect(),
        warnings: Vec::new(),
        log_days: Vec::new(),
        day_used: None,
    };
    let Some(dir) = log_dir else { return plan };
    let days = logs::read_log_rows(dir);
    plan.log_days = days.keys().rev().copied().collect();
    if days.is_empty() {
        plan.warnings
            .push(format!("No EdgeTX logs found in {}.", dir.display()));
        return plan;
    }
    // Default: the newest plausible day on or before the import date.
    let day = day.or_else(|| {
        days.keys()
            .rev()
            .find(|d| **d <= import_day && logs::day_is_plausible(**d, import_day, tun))
            .copied()
    });
    let Some(day) = day else {
        plan.warnings.push(
            "Radio log dates look wrong (the radio clock may have reset). Using the import date."
                .into(),
        );
        return plan;
    };
    if !logs::day_is_plausible(day, import_day, tun) {
        plan.warnings.push(format!(
            "Log day {day} is far from today (the radio clock may have reset). Using the import date."
        ));
        return plan;
    }
    let Some(rows) = days.get(&day) else {
        plan.warnings.push(format!("No log rows for {day}."));
        return plan;
    };
    plan.day_used = Some(day);
    let times: Vec<NaiveDateTime> = rows.iter().map(|r| r.time).collect();
    let segs = logs::segments(&times, tun);
    for (s, m) in plan
        .suggestions
        .iter_mut()
        .zip(logs::match_clips(durations, &segs, tun))
    {
        if let (Some(start), Badge::Matched | Badge::Likely) = (m.start, m.badge) {
            let windows: Vec<(NaiveDateTime, NaiveDateTime)> = segs[m.first..m.first + m.segments]
                .iter()
                .map(|g| (g.start, g.end))
                .collect();
            let log = RadioLog::from_rows(rows, &windows, start, 0.0);
            *s = DateSuggestion {
                date: start.date(),
                time: Some(start.time()),
                source: DateSource::Log,
                badge: m.badge,
                segments: m.segments,
                moments: log.moments(),
                log_interval_s: log.interval(),
                log_model: rows
                    .iter()
                    .find(|r| r.time >= start)
                    .and_then(|r| r.model.as_deref().map(str::to_string)),
                flight: Some(flight_stats(rows, &windows)),
            };
        }
    }
    plan
}

/// creation_time for a clip: its time of day (the log start time, or one set by hand),
/// otherwise local noon of the date so a UTC conversion cannot roll the day.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Verified,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipResult {
    pub id: usize,
    pub outcome: Outcome,
    pub output: Option<PathBuf>,
    pub original: Option<PathBuf>,
    pub size: u64,
    pub error: Option<String>,
    pub encoder: Option<Encoder>,
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CutResult {
    pub start: f64,
    pub end: f64,
    pub outcome: Outcome,
    pub output: Option<PathBuf>,
    pub size: u64,
    pub error: Option<String>,
}

/// Where the output folder setting points until the user picks one, relative to `$HOME`.
pub const DEFAULT_OUTPUT_REL: &str = "Movies/quadcam";

/// `~/Movies/quadcam`, resolved from `$HOME` at runtime.
pub fn default_output_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join(DEFAULT_OUTPUT_REL))
}

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
    let src: u64 = clips
        .iter()
        .map(|c| {
            c.source()
                .and_then(|p| p.metadata().ok())
                .map(|m| m.len())
                .unwrap_or(c.size)
        })
        .sum();
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
        description: format!("DVR {}; date source: {}", clip.name, job.source.label()),
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
        &meta.date,
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
    let tmp = out.with_file_name(format!(
        ".{}.part",
        out.file_name().unwrap().to_string_lossy()
    ));

    let enc = match media::convert(
        tools,
        src,
        &tmp,
        settings.format,
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
    let written = write_qt(&tmp, &qt)
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
                let dst = planner.claim(&dir, &orig_stem, "avi");
                let staged = clip.staged.as_deref().unwrap_or(src);
                std::fs::copy(staged, &dst)?;
                if dst.metadata()?.len() != staged.metadata()?.len() {
                    bail!("original copy came out the wrong size");
                }
                let _ = media::set_mtime(&dst, meta.creation_time);
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
        match results
            .iter()
            .rev()
            .find(|r| r.id == c.id)
            .map(|r| r.outcome)
        {
            Some(Outcome::Verified | Outcome::Skipped) => {}
            Some(Outcome::Failed) => bail!("{} failed to import.", c.name),
            None => bail!("{} has not been imported.", c.name),
        }
    }
    Ok(())
}

/// Writes the QuickTime items into `file`, the location also as `©xyz`.
fn write_qt(file: &Path, qt: &[(String, String)]) -> Result<()> {
    let loc = qt
        .iter()
        .find(|(k, _)| k == "com.apple.quicktime.location.ISO6709")
        .map(|(_, v)| v.as_str());
    crate::qtmeta::write(file, qt, loc).context("writing QuickTime metadata")
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
    let format = if ext.eq_ignore_ascii_case("mov") {
        Format::Mov
    } else {
        Format::Mp4
    };
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
            let tmp = dst.with_file_name(format!(
                ".{}.part",
                dst.file_name().unwrap().to_string_lossy()
            ));
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
            let res = media::cut(tools, src, &tmp, *span, format, settings.encoder, &cut_meta)
                .and_then(|_| write_qt(&tmp, &qt))
                .and_then(|_| media::verify_cut(tools, probe, &tmp, *span))
                .and_then(|_| media::verify_qt(tools, &tmp, &qt).map(|_| ()))
                .and_then(|_| {
                    if dst.exists() {
                        bail!("{} appeared during export; not overwriting", dst.display());
                    }
                    std::fs::rename(&tmp, &dst).context("renaming cut")?;
                    Ok(())
                });
            match res {
                Ok(()) => {
                    let _ = media::set_mtime(&dst, cut_meta.creation_time);
                    r.size = dst.metadata().map(|m| m.len()).unwrap_or(0);
                    r.output = Some(dst);
                    r.outcome = Outcome::Verified;
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    r.error = Some(format!("{e:#}"));
                }
            }
            r
        })
        .collect()
}
