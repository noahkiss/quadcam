//! Recordings an analog DVR split into several files. A DVR writes one long recording as
//! files of a fixed length (a Fat Shark Echo: about 600 s each, `PICT0001.AVI`,
//! `PICT0002.AVI`, ...). The next file starts where the last one ended, with no gap.
//!
//! Detection is conservative: a file continues the one before it only when every piece of
//! evidence agrees (`continues`). Anything less keeps the files apart.
//!
//! A joined recording stays one `Clip` per file in the session, so each file keeps its own
//! identity, card path, size and fingerprint. The first file (the head) carries a `Join`;
//! the later files (the parts) carry `part_of`. While the join is on, the head's duration,
//! probe and dead air are the whole recording's, and its source is an `ffconcat` list of
//! every file in staging, which ffmpeg and ffprobe read as one input. So matching, moments,
//! cuts, the encode and verify all see one clip, and the join happens in the encode.

use crate::media::Probe;
use crate::moments::{self, Moment, SignalScan};
use crate::pipeline::{Clip, ClipStatus};
use crate::sources::SourceKind;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The length a DVR splits a recording at. Two real Echo files were 600.004 s and 600.031 s.
pub const SPLIT_S: f64 = 600.0;
/// A file within this of `SPLIT_S` was split by the DVR.
pub const SURE_S: f64 = 1.0;
/// A file within this of `SPLIT_S` (but not `SURE_S`) may have been: unsure, so it stays
/// apart.
pub const UNSURE_S: f64 = 10.0;
/// File times further apart than the next file's length plus this say the files were not
/// one recording (when the DVR's clock runs at all).
pub const TIME_SLACK_S: f64 = 300.0;

/// The head's values for the other state of the join: the file's own while the join is on,
/// the whole recording's while it is off. Turning the join on or off swaps them in.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct Swap {
    pub duration: f64,
    pub probe: Option<Probe>,
    pub signal: Option<SignalScan>,
    pub detail: String,
}

/// A recording the DVR split into files, on its first file's clip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct Join {
    /// The later files' clip ids, in recording order.
    pub parts: Vec<usize>,
    /// The `ffconcat` list of every file, in staging.
    pub list: PathBuf,
    /// True while the files import as one clip.
    pub on: bool,
    /// Bytes of every file's source, for the free-space check.
    pub bytes: u64,
    pub swap: Swap,
}

/// Whether one file continues the one before it.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// One recording.
    Join,
    /// Maybe one recording; the files stay apart. The reason says what was missing.
    Unsure(String),
    /// Two recordings.
    Separate(String),
}

/// The number at the end of a file stem (`PICT0012` → `("PICT", 12, 4)`).
fn numbered(name: &str) -> Option<(String, u64, usize, String)> {
    let (stem, ext) = name.rsplit_once('.')?;
    let digits = stem.len() - stem.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    let (prefix, num) = stem.split_at(stem.len() - digits);
    Some((
        prefix.to_string(),
        num.parse().ok()?,
        digits,
        ext.to_lowercase(),
    ))
}

/// Whether a file named `b` may be the next file of the recording that `a` (`a_secs`
/// long) holds: the next number with the same prefix and extension, and `a` as long as a
/// DVR split. File times are not checked; a caller with more evidence uses `continues`.
pub fn may_follow(a: &str, a_secs: f64, b: &str) -> bool {
    let (Some(x), Some(y)) = (numbered(a), numbered(b)) else {
        return false;
    };
    x.0 == y.0 && x.3 == y.3 && x.1 + 1 == y.1 && (a_secs - SPLIT_S).abs() <= SURE_S
}

/// A name ffmpeg's concat list takes without `-safe 0`: letters, digits, `.`, `_`, `-`, and
/// no leading dot.
fn list_safe(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

/// Whether clip `b` continues clip `a`: both analog, in the same folder, numbered one after
/// the other, read whole (`b` may be cut off at its end), the same picture size, rate and
/// streams, `a` as long as the DVR's split length, and file times (when the DVR keeps any)
/// that do not contradict it.
pub fn continues(a: &Clip, b: &Clip) -> Verdict {
    use Verdict::*;
    if a.kind != SourceKind::Analog || b.kind != SourceKind::Analog {
        return Separate("not analog".into());
    }
    if a.stage_error.is_some() || b.stage_error.is_some() {
        return Separate("not copied".into());
    }
    let parent = |c: &Clip| Path::new(&c.rel).parent().map(Path::to_path_buf);
    if parent(a) != parent(b) {
        return Separate("different folders".into());
    }
    let (Some(na), Some(nb)) = (numbered(&a.name), numbered(&b.name)) else {
        return Separate("not numbered".into());
    };
    if na.0 != nb.0 || na.2 != nb.2 || na.3 != nb.3 || na.1 + 1 != nb.1 {
        return Separate("not numbered one after the other".into());
    }
    if a.status != ClipStatus::Ok {
        return Separate(format!("{} is not whole", a.name));
    }
    if b.status == ClipStatus::Empty {
        return Separate(format!("{} is empty", b.name));
    }
    let (Some(pa), Some(pb)) = (a.probe.as_ref(), b.probe.as_ref()) else {
        return Separate("not probed".into());
    };
    let rate = |p: &Probe| p.fps.map(|f| (f * 1000.0).round() as i64);
    if (
        pa.width,
        pa.height,
        rate(pa),
        pa.video_streams,
        pa.audio_streams,
    ) != (
        pb.width,
        pb.height,
        rate(pb),
        pb.video_streams,
        pb.audio_streams,
    ) {
        return Separate("different video formats".into());
    }
    let off = (a.duration - SPLIT_S).abs();
    if off > UNSURE_S {
        return Separate(format!("{} is {:.1} s long", a.name, a.duration));
    }
    if off > SURE_S {
        return Unsure(format!(
            "{} is {:.1} s long, near but not at the DVR's {SPLIT_S:.0} s split",
            a.name, a.duration
        ));
    }
    // File times only count when the DVR keeps a clock that runs.
    let clocked = |t: &Option<DateTime<Utc>>| t.filter(|t| t.year() >= 2015);
    if let (Some(ta), Some(tb)) = (clocked(&a.mtime), clocked(&b.mtime)) {
        let gap = (tb - ta).num_milliseconds() as f64 / 1000.0;
        if gap < 0.0 {
            return Unsure(format!("{} has an earlier file time", b.name));
        }
        if gap > b.duration + TIME_SLACK_S {
            return Unsure(format!("the file times are {:.0} min apart", gap / 60.0));
        }
    }
    let staged_safe = |c: &Clip| c.source().map(file_name).is_some_and(|n| list_safe(&n));
    if !staged_safe(a) || !staged_safe(b) {
        return Unsure("file names ffmpeg cannot list".into());
    }
    Join
}

/// Groups of clip indexes (two or more) that are one recording, in order. `clips` is the
/// session's list, in recording order.
pub fn groups(clips: &[Clip]) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut run = vec![0usize];
    for i in 1..clips.len() {
        if continues(&clips[i - 1], &clips[i]) == Verdict::Join {
            run.push(i);
        } else {
            if run.len() > 1 {
                out.push(std::mem::take(&mut run));
            }
            run = vec![i];
        }
    }
    if run.len() > 1 && !clips.is_empty() {
        out.push(run);
    }
    out
}

/// Dead air of files played one after the other: each file's moved by its start, and runs
/// that meet across a file boundary merged.
pub fn merge_signal(parts: &[(f64, Option<&SignalScan>)], duration: f64) -> Option<SignalScan> {
    let scans: Vec<(f64, &SignalScan)> = parts
        .iter()
        .map(|(at, s)| s.map(|s| (*at, s)))
        .collect::<Option<_>>()?;
    let mut dead: Vec<Moment> = Vec::new();
    for (at, s) in &scans {
        for m in &s.dead_air {
            let m = m.shifted(*at);
            match dead.last_mut() {
                Some(prev) if m.start - prev.end < moments::tune::DEAD_BRIDGE_S => {
                    prev.end = prev.end.max(m.end);
                    prev.score = prev.score.min(m.score);
                }
                _ => dead.push(m),
            }
        }
    }
    Some(SignalScan {
        step: scans.first().map(|(_, s)| s.step).unwrap_or(0.0),
        samples: scans.iter().map(|(_, s)| s.samples).sum(),
        keep: moments::keep_ranges(&dead, duration),
        dead_air: dead,
    })
}

/// The probe of files played one after the other: frames and durations add up; streams,
/// size and rate are the first file's.
pub fn merge_probe(probes: &[&Probe]) -> Probe {
    let first = probes.first().copied().cloned().unwrap_or_default();
    Probe {
        duration: probes.iter().map(|p| p.duration).sum(),
        video_packets: probes.iter().map(|p| p.video_packets).sum(),
        errors: probes
            .iter()
            .map(|p| p.errors.as_str())
            .filter(|e| !e.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        ..first
    }
}

fn mmss(secs: f64) -> String {
    let s = secs.round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Writes the `ffconcat` list for `clips[group]` next to the staged files and returns the
/// head's `Join`, off. A `#` line names every file's fingerprint, so two cards' lists of the
/// same file names differ (previews are cached by the list's content).
pub fn make(clips: &[Clip], group: &[usize]) -> Result<Join> {
    let head = &clips[group[0]];
    let staged = head.staged.as_deref().context("not staged")?;
    let dir = staged.parent().context("staged file has no folder")?;
    let mut text = String::from("ffconcat version 1.0\n");
    text.push_str(&format!(
        "# {}\n",
        group
            .iter()
            .map(|&i| clips[i].key.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    ));
    let mut bytes = 0;
    for &i in group {
        let src = clips[i].source().context("not staged")?;
        if src.parent() != Some(dir) {
            bail!("{} is not in the staging folder", src.display());
        }
        let name = file_name(src);
        if !list_safe(&name) {
            bail!("{name} cannot go in an ffmpeg list");
        }
        bytes += src.metadata().map(|m| m.len()).unwrap_or(clips[i].size);
        text.push_str(&format!("file {name}\n"));
    }
    let list = dir.join(format!(
        "{}.joined.ffconcat",
        staged.file_stem().unwrap_or_default().to_string_lossy()
    ));
    std::fs::write(&list, text).with_context(|| format!("writing {}", list.display()))?;
    let probes: Vec<&Probe> = group
        .iter()
        .map(|&i| clips[i].probe.as_ref().context("not probed"))
        .collect::<Result<_>>()?;
    let probe = merge_probe(&probes);
    let duration: f64 = group.iter().map(|&i| clips[i].duration).sum();
    let mut at = 0.0;
    let starts: Vec<(f64, Option<&SignalScan>)> = group
        .iter()
        .map(|&i| {
            let s = (at, clips[i].signal.as_ref());
            at += clips[i].duration;
            s
        })
        .collect();
    Ok(Join {
        parts: group[1..].iter().map(|&i| clips[i].id).collect(),
        list,
        on: false,
        bytes,
        swap: Swap {
            duration,
            probe: Some(probe),
            signal: merge_signal(&starts, duration),
            detail: format!("{} files, one recording, {}", group.len(), mmss(duration)),
        },
    })
}

/// Turns a head's join on or off: swaps its values with the stashed ones. Returns false
/// when it already was.
pub fn set(head: &mut Clip, on: bool) -> bool {
    let Some(j) = head.join.as_mut() else {
        return false;
    };
    if j.on == on {
        return false;
    }
    std::mem::swap(&mut head.duration, &mut j.swap.duration);
    std::mem::swap(&mut head.probe, &mut j.swap.probe);
    std::mem::swap(&mut head.signal, &mut j.swap.signal);
    std::mem::swap(&mut head.detail, &mut j.swap.detail);
    j.on = on;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::moments::{MomentKind, Source, Span};

    fn probe(secs: f64, w: u64) -> Probe {
        Probe {
            duration: secs,
            video_packets: (secs * 30.0).round() as u64,
            video_streams: 1,
            audio_streams: 1,
            fps: Some(30.0),
            width: Some(w),
            height: Some(480),
            ..Default::default()
        }
    }

    pub(crate) fn clip(id: usize, name: &str, secs: f64) -> Clip {
        Clip {
            id,
            name: name.into(),
            rel: format!("DCIM/{name}"),
            card_path: PathBuf::from(format!("/card/DCIM/{name}")),
            size: 1000,
            staged: Some(PathBuf::from(format!("/staging/{name}"))),
            stage_error: None,
            status: ClipStatus::Ok,
            duration: secs,
            recovered: None,
            probe: Some(probe(secs, 720)),
            thumb: None,
            detail: String::new(),
            signal: None,
            key: format!("k{id}"),
            kind: SourceKind::Analog,
            clock: None,
            sidecars: Vec::new(),
            mtime: None,
            join: None,
            part_of: None,
        }
    }

    #[test]
    fn a_split_recording_joins() {
        let a = clip(0, "PICT0001.AVI", 600.004);
        let b = clip(1, "PICT0002.AVI", 450.5);
        assert_eq!(continues(&a, &b), Verdict::Join);
        // A last file cut off by power loss still continues the recording.
        let mut cut_off = b.clone();
        cut_off.status = ClipStatus::Incomplete;
        cut_off.recovered = Some("/staging/PICT0002.recovered.avi".into());
        assert_eq!(continues(&a, &cut_off), Verdict::Join);
        // File times that fit: b closed about its own length after a.
        let mut a2 = a.clone();
        let mut b2 = b.clone();
        a2.mtime = Some("2026-09-25T14:10:00Z".parse().unwrap());
        b2.mtime = Some("2026-09-25T14:17:31Z".parse().unwrap());
        assert_eq!(continues(&a2, &b2), Verdict::Join);
        // A DVR without a clock writes one fixed time on every file: no evidence either way.
        a2.mtime = Some("2020-01-01T00:00:00Z".parse().unwrap());
        b2.mtime = a2.mtime;
        assert_eq!(continues(&a2, &b2), Verdict::Join);
    }

    #[test]
    fn separate_recordings_stay_apart() {
        let a = clip(0, "PICT0001.AVI", 600.0);
        let sep = |b: &Clip| matches!(continues(&a, b), Verdict::Separate(_));
        // A short first file: the person stopped recording.
        assert!(matches!(
            continues(
                &clip(0, "PICT0001.AVI", 312.0),
                &clip(1, "PICT0002.AVI", 90.0)
            ),
            Verdict::Separate(_)
        ));
        // Not the next number, another prefix, another folder.
        assert!(sep(&clip(1, "PICT0003.AVI", 90.0)));
        assert!(sep(&clip(1, "MOVI0002.AVI", 90.0)));
        let mut other = clip(1, "PICT0002.AVI", 90.0);
        other.rel = "DCIM/100/PICT0002.AVI".into();
        assert!(sep(&other));
        // Another picture size.
        let mut wide = clip(1, "PICT0002.AVI", 90.0);
        wide.probe = Some(probe(90.0, 640));
        assert!(sep(&wide));
        // A first file that was cut off cannot continue into the next.
        let mut a_cut = a.clone();
        a_cut.status = ClipStatus::Incomplete;
        assert!(matches!(
            continues(&a_cut, &clip(1, "PICT0002.AVI", 90.0)),
            Verdict::Separate(_)
        ));
        // A DJI file never joins.
        let mut dji = clip(1, "PICT0002.AVI", 90.0);
        dji.kind = SourceKind::Dji;
        assert!(sep(&dji));
    }

    #[test]
    fn unsure_stays_apart() {
        let b = clip(1, "PICT0002.AVI", 90.0);
        // Near the split length, not at it.
        let v = continues(&clip(0, "PICT0001.AVI", 596.0), &b);
        assert!(
            matches!(&v, Verdict::Unsure(w) if w.contains("near")),
            "{v:?}"
        );
        // File times that contradict one recording.
        let mut a = clip(0, "PICT0001.AVI", 600.0);
        let mut b2 = b.clone();
        a.mtime = Some("2026-09-25T14:10:00Z".parse().unwrap());
        b2.mtime = Some("2026-09-25T15:30:00Z".parse().unwrap());
        assert!(matches!(continues(&a, &b2), Verdict::Unsure(_)));
        b2.mtime = Some("2026-09-25T14:00:00Z".parse().unwrap());
        assert!(matches!(continues(&a, &b2), Verdict::Unsure(_)));
        // A staged name ffmpeg's list refuses.
        let mut odd = clip(1, "PICT0002.AVI", 90.0);
        odd.staged = Some("/staging/DCIM 2_PICT0002.AVI".into());
        assert!(matches!(
            continues(&clip(0, "PICT0001.AVI", 600.0), &odd),
            Verdict::Unsure(_)
        ));
        // Unsure files are not grouped.
        assert!(groups(&[clip(0, "PICT0001.AVI", 596.0), b]).is_empty());
    }

    #[test]
    fn groups_chain_whole_recordings() {
        let clips = vec![
            clip(0, "PICT0001.AVI", 120.0),
            clip(1, "PICT0002.AVI", 600.0),
            clip(2, "PICT0003.AVI", 600.02),
            clip(3, "PICT0004.AVI", 33.0),
            clip(4, "PICT0005.AVI", 600.0),
        ];
        assert_eq!(groups(&clips), vec![vec![1, 2, 3]]);
        assert!(groups(&[]).is_empty());
    }

    #[test]
    fn dead_air_across_a_file_boundary_is_one_run() {
        let dead = |a: f64, b: f64| Moment {
            kind: MomentKind::DeadAir,
            start: a,
            end: b,
            score: 1.0,
            source: Source::Video,
            detail: "blue screen".into(),
        };
        let one = SignalScan {
            step: 0.5,
            samples: 1200,
            dead_air: vec![dead(10.0, 20.0), dead(590.0, 600.0)],
            keep: Vec::new(),
        };
        let two = SignalScan {
            step: 0.5,
            samples: 200,
            dead_air: vec![dead(0.0, 30.0)],
            keep: Vec::new(),
        };
        let s = merge_signal(&[(0.0, Some(&one)), (600.0, Some(&two))], 700.0).unwrap();
        let spans: Vec<(f64, f64)> = s.dead_air.iter().map(|m| (m.start, m.end)).collect();
        assert_eq!(spans, [(10.0, 20.0), (590.0, 630.0)]);
        assert_eq!(
            s.keep,
            vec![
                Span {
                    start: 0.0,
                    end: 10.0
                },
                Span {
                    start: 20.0,
                    end: 590.0
                },
                Span {
                    start: 630.0,
                    end: 700.0
                }
            ]
        );
        assert_eq!(s.samples, 1400);
        // A file that could not be scanned: no dead air for the whole recording.
        assert!(merge_signal(&[(0.0, Some(&one)), (600.0, None)], 700.0).is_none());
    }

    #[test]
    fn probes_add_up_and_the_join_swaps() {
        let p = merge_probe(&[&probe(600.0, 720), &probe(30.0, 720)]);
        assert_eq!((p.duration, p.video_packets), (630.0, 18900));
        assert_eq!(p.width, Some(720));
        let d = tempfile::tempdir().unwrap();
        let mut clips = vec![
            clip(0, "PICT0001.AVI", 600.0),
            clip(1, "PICT0002.AVI", 30.0),
        ];
        for c in &mut clips {
            let f = d.path().join(&c.name);
            std::fs::write(&f, b"x").unwrap();
            c.staged = Some(f);
        }
        let j = make(&clips, &[0, 1]).unwrap();
        let text = std::fs::read_to_string(&j.list).unwrap();
        assert_eq!(
            text,
            "ffconcat version 1.0\n# k0 k1\nfile PICT0001.AVI\nfile PICT0002.AVI\n"
        );
        assert_eq!(j.parts, vec![1]);
        let mut head = clips[0].clone();
        head.join = Some(j);
        assert!(set(&mut head, true));
        assert_eq!(head.duration, 630.0);
        assert_eq!(
            head.source(),
            Some(head.join.as_ref().unwrap().list.as_path())
        );
        assert!(!set(&mut head, true));
        assert!(set(&mut head, false));
        assert_eq!(head.duration, 600.0);
        assert_eq!(head.source(), head.staged.as_deref());
    }
}
