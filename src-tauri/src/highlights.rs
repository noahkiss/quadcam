//! Highlights and the key frame from macOS video analysis (`crate::intel`, the helper's
//! `video` command, macOS 27 or later). They are a moments source for clips with no
//! radio-log match (DJI clips, analog clips without a log), and the key frame is the clip's
//! poster. Radio-log moments keep priority: a clip that has any shows only those.
//!
//! The analyzer does the detection; this module turns its answer into moments, drops what
//! falls in dead air (analog static can look interesting to it), and stores the result: in
//! the clip file (`app.quadcam.video`, its own item, so `app.quadcam.moments` stays readable
//! by older QuadCam) and in a cache per clip content fingerprint, so a clip is analysed once.

use crate::intel::Watched;
use crate::moments::{Moment, MomentKind, Source};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Every threshold, in one place.
pub mod tune {
    /// A highlight with more than this share of its length in dead air is dropped.
    pub const DEAD_SHARE_MAX: f64 = 0.5;
    /// The score of a highlight when the analyzer gave no levels.
    pub const NO_LEVEL_SCORE: f64 = 0.5;
}

/// The version of the cache and file formats.
const VERSION: u8 = 1;

/// What video analysis found in one clip.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct VideoScan {
    /// Highlights as moments (kind `highlight`, source `video_analysis`), by start time.
    #[serde(default)]
    pub highlights: Vec<Moment>,
    /// Seconds into the clip: the frame the analyzer picked to stand for it. None when it
    /// picked none, or the frame is in dead air.
    #[serde(default)]
    pub key_frame: Option<f64>,
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// Seconds of `[a, b)` inside `dead`.
fn dead_secs(a: f64, b: f64, dead: &[Moment]) -> f64 {
    dead.iter()
        .filter(|d| d.kind == MomentKind::DeadAir)
        .map(|d| (b.min(d.end) - a.max(d.start)).max(0.0))
        .sum()
}

/// The analyzer's answer as a scan of a clip `duration` seconds long, with `dead` its dead
/// air. A highlight scores its mean level over the clip's highest level.
pub fn scan(w: &Watched, dead: &[Moment], duration: f64) -> VideoScan {
    let peak = w.levels.iter().map(|l| l.level).fold(0.0, f64::max);
    let mean_level = |a: f64, b: f64| {
        let (mut sum, mut secs) = (0.0, 0.0);
        for l in &w.levels {
            let o = (b.min(l.end) - a.max(l.start)).max(0.0);
            sum += l.level * o;
            secs += o;
        }
        (secs > 0.0).then(|| sum / secs)
    };
    let end_at = |x: f64| if duration > 0.0 { x.min(duration) } else { x };
    let mut highlights: Vec<Moment> = w
        .highlights
        .iter()
        .flatten()
        .filter_map(|r| {
            let (start, end) = (r.start.max(0.0), end_at(r.end));
            if end <= start || dead_secs(start, end, dead) > tune::DEAD_SHARE_MAX * (end - start) {
                return None;
            }
            let level = mean_level(start, end);
            let score = match level {
                Some(l) if peak > 0.0 => (l / peak).clamp(0.0, 1.0),
                _ => tune::NO_LEVEL_SCORE,
            };
            Some(Moment {
                kind: MomentKind::Highlight,
                start: round3(start),
                end: round3(end),
                score: round3(score),
                source: Source::VideoAnalysis,
                detail: match level {
                    Some(l) => format!("video analysis, level {l:.1} of {peak:.0}"),
                    None => "video analysis".into(),
                },
            })
        })
        .collect();
    highlights.sort_by(|a, b| a.start.total_cmp(&b.start));
    let key_frame = w
        .keyframe
        .filter(|k| *k >= 0.0 && (duration <= 0.0 || *k < duration))
        .filter(|k| dead_secs(*k, *k + 0.001, dead) == 0.0)
        .map(round3);
    VideoScan {
        highlights,
        key_frame,
    }
}

/// The moments a clip shows: its radio-log moments when it has any, else its highlights.
/// Other moments (dead air) stay either way.
pub fn merged(moments: &[Moment], video: Option<&VideoScan>) -> Vec<Moment> {
    let mut out: Vec<Moment> = moments
        .iter()
        .filter(|m| m.source != Source::VideoAnalysis)
        .cloned()
        .collect();
    if !out.iter().any(|m| m.source == Source::RadioLog) {
        if let Some(v) = video {
            out.extend(v.highlights.iter().cloned());
            out.sort_by(|a, b| a.start.total_cmp(&b.start));
        }
    }
    out
}

// ---------- the file item ----------

/// The scan as written into a clip file: version, key frame, and highlights as
/// `[start, end, score]`.
#[derive(Serialize, Deserialize)]
struct Lite {
    v: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    k: Option<f64>,
    #[serde(default)]
    h: Vec<[f64; 3]>,
}

/// The `app.quadcam.video` value. Never empty, so "analysed, found nothing" is recorded.
pub fn value(s: &VideoScan) -> String {
    let r1 = |x: f64| (x * 10.0).round() / 10.0;
    serde_json::to_string(&Lite {
        v: VERSION,
        k: s.key_frame.map(r1),
        h: s.highlights
            .iter()
            .map(|m| [r1(m.start), r1(m.end), (m.score * 100.0).round() / 100.0])
            .collect(),
    })
    .unwrap_or_default()
}

/// Reads an `app.quadcam.video` value; None when it is not one this build understands.
pub fn parse(s: &str) -> Option<VideoScan> {
    let l: Lite = serde_json::from_str(s).ok()?;
    if l.v != VERSION {
        return None;
    }
    Some(VideoScan {
        key_frame: l.k,
        highlights: l
            .h
            .iter()
            .map(|[a, b, c]| Moment {
                kind: MomentKind::Highlight,
                start: *a,
                end: *b,
                score: *c,
                source: Source::VideoAnalysis,
                detail: "video analysis".into(),
            })
            .collect(),
    })
}

// ---------- the cache ----------

#[derive(Serialize, Deserialize)]
struct Cached {
    v: u8,
    watched: Watched,
}

/// Where the analyzer's answer for content `key` (a clip fingerprint) is cached.
pub fn cache_file(cache: &Path, key: &str) -> PathBuf {
    cache
        .join("video")
        .join(format!("{}.json", crate::library::id_file(key)))
}

/// The cached answer for content `key`, when there is one.
pub fn cached(cache: &Path, key: &str) -> Option<Watched> {
    if key.is_empty() {
        return None;
    }
    let c: Cached = serde_json::from_slice(&std::fs::read(cache_file(cache, key)).ok()?).ok()?;
    (c.v == VERSION).then_some(c.watched)
}

/// Caches a complete answer for content `key`. An answer with an error is not cached.
pub fn store(cache: &Path, key: &str, w: &Watched) {
    if key.is_empty()
        || w.error.is_some()
        || w.highlights_error.is_some()
        || w.keyframe_error.is_some()
    {
        return;
    }
    let f = cache_file(cache, key);
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let Ok(json) = serde_json::to_vec(&Cached {
        v: VERSION,
        watched: w.clone(),
    }) else {
        return;
    };
    let tmp = f.with_extension("json.part");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, &f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intel::{Level, Range};

    fn watched() -> Watched {
        Watched {
            file: "a.mp4".into(),
            highlights: Some(vec![
                Range {
                    start: 2.0,
                    end: 6.0,
                },
                Range {
                    start: 20.0,
                    end: 30.0,
                },
                Range {
                    start: 40.0,
                    end: 70.0,
                },
            ]),
            levels: (0..120)
                .map(|i| Level {
                    start: i as f64 * 0.5,
                    end: i as f64 * 0.5 + 0.5,
                    level: if (4..12).contains(&i) { 8.0 } else { 2.0 },
                })
                .collect(),
            keyframe: Some(25.0),
            ..Default::default()
        }
    }

    fn dead(a: f64, b: f64) -> Moment {
        Moment {
            kind: MomentKind::DeadAir,
            start: a,
            end: b,
            score: 1.0,
            source: Source::Video,
            detail: String::new(),
        }
    }

    #[test]
    fn highlights_become_moments_scored_by_level() {
        let s = scan(&watched(), &[], 60.0);
        assert_eq!(s.highlights.len(), 3);
        let h = &s.highlights[0];
        assert_eq!(
            (h.kind, h.source),
            (MomentKind::Highlight, Source::VideoAnalysis)
        );
        assert_eq!((h.start, h.end), (2.0, 6.0));
        assert_eq!(h.score, 1.0, "the clip's peak level scores 1");
        assert!(s.highlights[1].score < 0.3, "{:?}", s.highlights[1]);
        // Clamped to the clip.
        assert_eq!(s.highlights[2].end, 60.0);
        assert_eq!(s.key_frame, Some(25.0));
    }

    #[test]
    fn dead_air_drops_highlights_and_the_key_frame() {
        let s = scan(&watched(), &[dead(18.0, 28.0)], 60.0);
        assert_eq!(s.highlights.len(), 2, "20-30 is mostly dead air");
        assert_eq!(s.key_frame, None, "25 s is in dead air");
        // A highlight barely touching dead air stays.
        let s = scan(&watched(), &[dead(28.0, 31.0)], 60.0);
        assert_eq!(s.highlights.len(), 3);
    }

    #[test]
    fn log_moments_keep_priority() {
        let s = scan(&watched(), &[], 60.0);
        let roll = Moment {
            kind: MomentKind::Roll,
            start: 10.0,
            end: 11.0,
            score: 0.8,
            source: Source::RadioLog,
            detail: String::new(),
        };
        assert_eq!(
            merged(std::slice::from_ref(&roll), Some(&s)),
            vec![roll.clone()]
        );
        let m = merged(&[dead(0.0, 1.0)], Some(&s));
        assert_eq!(m.len(), 4, "dead air stays, highlights join");
        assert_eq!(merged(&[], None), vec![]);
        // Merging twice does not double them.
        assert_eq!(merged(&m, Some(&s)).len(), 4);
    }

    #[test]
    fn the_file_value_round_trips() {
        let s = scan(&watched(), &[], 60.0);
        let back = parse(&value(&s)).unwrap();
        assert_eq!(back.key_frame, Some(25.0));
        assert_eq!(back.highlights.len(), 3);
        assert_eq!(back.highlights[0].start, 2.0);
        assert_eq!(back.highlights[0].source, Source::VideoAnalysis);
        // Nothing found is still a value.
        let none = value(&VideoScan::default());
        assert_eq!(none, r#"{"v":1,"h":[]}"#);
        assert_eq!(parse(&none), Some(VideoScan::default()));
        assert_eq!(parse(r#"{"v":9,"h":[]}"#), None);
        assert_eq!(parse("junk"), None);
    }

    #[test]
    fn the_cache_keeps_complete_answers_only() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(cached(d.path(), "abc"), None);
        store(d.path(), "abc", &watched());
        assert_eq!(cached(d.path(), "abc"), Some(watched()));
        let failed = Watched {
            error: Some("unreadable".into()),
            ..Default::default()
        };
        store(d.path(), "bad", &failed);
        assert_eq!(cached(d.path(), "bad"), None);
        assert_eq!(cached(d.path(), ""), None);
    }
}
