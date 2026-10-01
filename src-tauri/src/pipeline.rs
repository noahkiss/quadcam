//! The import pipeline without any UI: stage, analyse, date, convert, verify.
//! `session` and `core` build on these.

use crate::logs::{self, Badge, Tunables};
use crate::media::{self, Encoder, Format, Meta, Probe, Tools};
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
/// A file already staged at the same size is not copied again.
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
        };
        let already = dst.metadata().is_ok_and(|m| m.len() == f.size);
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
    let thumb = cache.join(format!(
        "{}.jpg",
        staged.file_stem().unwrap_or_default().to_string_lossy()
    ));
    if media::thumbnail(tools, clip.source().unwrap(), &thumb).is_ok() {
        clip.thumb = Some(thumb);
    }
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
    };
    let mut plan = DatePlan {
        suggestions: durations.iter().map(fallback).collect(),
        warnings: Vec::new(),
        log_days: Vec::new(),
        day_used: None,
    };
    let Some(dir) = log_dir else { return plan };
    let days = logs::read_log_dir(dir);
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
    let segs = logs::segments(rows, tun);
    for (s, m) in plan
        .suggestions
        .iter_mut()
        .zip(logs::match_clips(durations, &segs, tun))
    {
        if let (Some(start), Badge::Matched | Badge::Likely) = (m.start, m.badge) {
            *s = DateSuggestion {
                date: start.date(),
                time: Some(start.time()),
                source: DateSource::Log,
                badge: m.badge,
                segments: m.segments,
            };
        }
    }
    plan
}

/// creation_time for a clip: the log start time when the date came from a log,
/// otherwise local noon of the date so a UTC conversion cannot roll the day.
pub fn creation_time(
    date: NaiveDate,
    time: Option<NaiveTime>,
    source: DateSource,
) -> DateTime<Utc> {
    let t = match (source, time) {
        (DateSource::Log, Some(t)) => t,
        _ => NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
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
    /// `HH:MM:SS`, only when the date came from a radio log.
    pub time: Option<String>,
    pub source: DateSource,
    pub name: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSettings {
    pub output_dir: PathBuf,
    pub format: Format,
    pub encoder: Encoder,
    pub keep_originals: bool,
    pub add_time: bool,
    pub default_name: String,
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
        (DateSource::Log, Some(t)) => NaiveTime::parse_from_str(t, "%H:%M:%S%.f").ok(),
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
    let out = planner.claim(&settings.output_dir, &stem, settings.format.ext());
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
    if let Err(e) = media::verify(tools, src_probe, &tmp, &meta) {
        let _ = std::fs::remove_file(&tmp);
        return fail(r, format!("verify failed: {e:#}"));
    }
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
        let dir = settings.output_dir.join("originals");
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
