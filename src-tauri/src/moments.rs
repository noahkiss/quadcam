//! Moments: time ranges in a clip worth a look (a roll, a punch-out, a dive) or worth cutting
//! (dead air). Each kind of evidence is a `MomentSource`:
//!
//! - `RadioLog`: EdgeTX stick channels from the log rows a clip claimed (see `logs`).
//! - `VideoSignal`: the clip's own frames, sampled at low rate and resolution, for dead air
//!   (no-signal blue screen, static, test pattern, black).
//! - A Betaflight blackbox source (gyro rates, far better for flips) can be added as another
//!   `Source` variant and `MomentSource` implementation. It is not implemented.
//!
//! Detection is pure: it takes samples and returns moments, so the tests feed it synthetic
//! rows and frames. Every threshold is a constant in `tune`.

use crate::logs::LogRow;
use crate::media::{self, Tools};
use anyhow::Result;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Every detection threshold, in one place.
pub mod tune {
    // Radio log rows. Stick fractions: 1.0 is full deflection; throttle 0.0 is the bottom.
    /// A row interval at or below this is fine-grained (EdgeTX 0.1 s logging).
    pub const FINE_INTERVAL_S: f64 = 0.15;
    /// A row interval above this is coarse (EdgeTX 0.5 s or 1 s logging).
    pub const COARSE_INTERVAL_S: f64 = 0.3;
    /// Score multipliers for medium and coarse logs: their timing is less certain.
    pub const MEDIUM_FACTOR: f64 = 0.85;
    pub const COARSE_FACTOR: f64 = 0.6;
    /// Roll / flip: aileron or elevator at or past this fraction of full deflection...
    pub const ROLL_DEFLECTION: f64 = 0.8;
    /// ...held at least this long...
    pub const ROLL_MIN_S: f64 = 0.3;
    /// ...and at most this long. A longer hold up to twice this still counts, at half score
    /// (several rolls in a row, or a slow one).
    pub const ROLL_MAX_S: f64 = 1.5;
    /// Punch-out: throttle from at or below this...
    pub const PUNCH_FROM: f64 = 0.35;
    /// ...to at or above this...
    pub const PUNCH_TO: f64 = 0.85;
    /// ...within this long (or one row, on a coarser log).
    pub const PUNCH_RISE_S: f64 = 0.6;
    /// A punch held this long scores higher.
    pub const PUNCH_HOLD_S: f64 = 1.0;
    /// Dive / hang time: throttle at or below this...
    pub const HANG_THROTTLE: f64 = 0.15;
    /// ...for at least this long, mid-flight, then a punch.
    pub const HANG_MIN_S: f64 = 1.0;
    /// A hang this long scores higher.
    pub const HANG_LONG_S: f64 = 2.0;
    /// Crash: any stick at or past this fraction (or a punch)...
    pub const CRASH_INPUT: f64 = 0.8;
    /// ...in this long before the log segment ends (disarm).
    pub const CRASH_WINDOW_S: f64 = 1.5;

    // Video frames for dead air.
    /// Frames sampled per second of video.
    pub const SAMPLE_FPS: f64 = 2.0;
    /// Sampled frame size (luma and chroma, 4:4:4).
    pub const SAMPLE_W: usize = 64;
    pub const SAMPLE_H: usize = 48;
    /// A pixel counts as blue when U is above this and V below `BLUE_V`.
    pub const BLUE_U: u8 = 150;
    pub const BLUE_V: u8 = 140;
    /// Blue screen: at least this share of blue pixels, darker than `BLUE_MAX_Y`, and flat.
    pub const BLUE_SHARE: f64 = 0.7;
    pub const BLUE_MAX_Y: f64 = 110.0;
    pub const BLUE_MAX_DETAIL: f64 = 6.0;
    /// Black: mean luma below this and luma spread below `BLACK_MAX_SD`.
    pub const BLACK_Y: f64 = 32.0;
    pub const BLACK_MAX_SD: f64 = 12.0;
    /// Blank (one flat colour): luma spread below this and almost no detail.
    pub const BLANK_MAX_SD: f64 = 4.0;
    pub const BLANK_MAX_DETAIL: f64 = 2.0;
    /// Static: neighbouring pixels differ about as much as the whole frame does (no
    /// structure), and the frame changes about that much between samples.
    pub const STATIC_MIN_SD: f64 = 12.0;
    pub const STATIC_DETAIL_RATIO: f64 = 0.85;
    pub const STATIC_CHANGE_RATIO: f64 = 0.6;
    /// Test pattern (colour bars): frozen, colourful, high contrast, and columns that do
    /// not change down the frame.
    pub const BARS_MAX_CHANGE: f64 = 1.5;
    pub const BARS_MIN_SD: f64 = 40.0;
    pub const BARS_MAX_VERTICAL_RATIO: f64 = 0.12;
    pub const BARS_MIN_SAT: f64 = 25.0;
    /// Dead air shorter than this is a breakup inside a flight and stays in.
    pub const DEAD_MIN_S: f64 = 3.0;
    /// Live samples spanning less than this inside dead air do not split it.
    pub const DEAD_BRIDGE_S: f64 = 1.0;
    /// A suggested keep range shorter than this is dropped.
    pub const KEEP_MIN_S: f64 = 2.0;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MomentKind {
    /// Full aileron, held.
    Roll,
    /// Full elevator, held.
    Flip,
    /// Throttle from low to near full, fast.
    Punch,
    /// Throttle near zero for a while mid-flight, then a punch.
    Dive,
    /// The log segment ends right after big inputs. Low confidence.
    Crash,
    /// No usable picture: blue screen, static, test pattern, black.
    DeadAir,
}

impl MomentKind {
    pub fn label(self) -> &'static str {
        match self {
            MomentKind::Roll => "roll",
            MomentKind::Flip => "flip",
            MomentKind::Punch => "punch-out",
            MomentKind::Dive => "dive",
            MomentKind::Crash => "crash?",
            MomentKind::DeadAir => "dead air",
        }
    }
}

/// Where a moment's evidence came from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    RadioLog,
    Video,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Moment {
    pub kind: MomentKind,
    /// Seconds from the start of the clip.
    pub start: f64,
    pub end: f64,
    /// 0..1. Coarse logs score lower; see `detail`.
    pub score: f64,
    pub source: Source,
    /// One line on what was seen.
    pub detail: String,
}

impl Moment {
    /// Moves the moment by `by` seconds.
    pub fn shifted(&self, by: f64) -> Moment {
        Moment {
            start: self.start + by,
            end: self.end + by,
            ..self.clone()
        }
    }
}

/// A time range in a clip, in seconds: a suggested keep range or a cut.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Span {
    pub start: f64,
    pub end: f64,
}

impl Span {
    pub fn secs(&self) -> f64 {
        self.end - self.start
    }
}

/// Anything that finds moments in a clip.
pub trait MomentSource {
    fn source(&self) -> Source;
    fn moments(&self) -> Vec<Moment>;
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

// ---------- radio log ----------

/// One log row in clip time, with sticks as fractions: ail, ele, rud -1..1, thr 0..1.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StickSample {
    pub t: f64,
    pub ail: f64,
    pub ele: f64,
    pub thr: f64,
    pub rud: f64,
    /// Attitude telemetry in radians, when the receiver sends it.
    pub roll: Option<f64>,
    pub pitch: Option<f64>,
}

impl StickSample {
    /// From a log row, `t` seconds into the clip. None when the row has no stick columns.
    pub fn from_row(row: &LogRow, t: f64) -> Option<StickSample> {
        let s = row.sticks?;
        let f = |v: f64| (v / 1024.0).clamp(-1.0, 1.0);
        Some(StickSample {
            t,
            ail: f(s.ail),
            ele: f(s.ele),
            thr: ((s.thr + 1024.0) / 2048.0).clamp(0.0, 1.0),
            rud: f(s.rud),
            roll: row.roll,
            pitch: row.pitch,
        })
    }
}

/// The log rows a clip claimed, as stick samples per armed segment.
pub struct RadioLog {
    pub segments: Vec<Vec<StickSample>>,
}

impl RadioLog {
    /// Samples for the rows inside `segments` (start, end), timed from `zero` plus `offset_s`.
    /// `rows` must be sorted by time.
    pub fn from_rows(
        rows: &[LogRow],
        segments: &[(NaiveDateTime, NaiveDateTime)],
        zero: NaiveDateTime,
        offset_s: f64,
    ) -> RadioLog {
        let secs = |t: NaiveDateTime| (t - zero).num_milliseconds() as f64 / 1000.0 + offset_s;
        let segments = segments
            .iter()
            .map(|(a, b)| {
                rows.iter()
                    .filter(|r| r.time >= *a && r.time <= *b)
                    .filter_map(|r| StickSample::from_row(r, secs(r.time)))
                    .collect::<Vec<_>>()
            })
            .filter(|s| !s.is_empty())
            .collect();
        RadioLog { segments }
    }

    /// Median row interval over every segment, in seconds.
    pub fn interval(&self) -> Option<f64> {
        let mut d: Vec<f64> = self
            .segments
            .iter()
            .flat_map(|s| s.windows(2).map(|w| w[1].t - w[0].t))
            .filter(|d| *d > 0.0)
            .collect();
        if d.is_empty() {
            return None;
        }
        d.sort_by(f64::total_cmp);
        Some(round3(d[d.len() / 2]))
    }
}

impl MomentSource for RadioLog {
    fn source(&self) -> Source {
        Source::RadioLog
    }
    fn moments(&self) -> Vec<Moment> {
        let dt = self.interval().unwrap_or(tune::FINE_INTERVAL_S);
        let mut out: Vec<Moment> = self
            .segments
            .iter()
            .flat_map(|s| detect_sticks(s, dt))
            .collect();
        out.sort_by(|a, b| a.start.total_cmp(&b.start));
        out
    }
}

/// Score factor and a note for the log's row interval.
fn resolution(dt: f64) -> (f64, String) {
    if dt <= tune::FINE_INTERVAL_S {
        (1.0, String::new())
    } else if dt <= tune::COARSE_INTERVAL_S {
        (tune::MEDIUM_FACTOR, format!("; log rows {dt:.1} s apart"))
    } else {
        (
            tune::COARSE_FACTOR,
            format!("; coarse log ({dt:.1} s rows): times are +/- {dt:.1} s and short moves may be missed"),
        )
    }
}

/// Finds rolls, flips, punches, dives and a possible crash in one armed segment. `dt` is the
/// log's row interval; each row stands for `dt` seconds.
pub fn detect_sticks(s: &[StickSample], dt: f64) -> Vec<Moment> {
    let mut out = Vec::new();
    if s.is_empty() {
        return out;
    }
    let (factor, note) = resolution(dt);
    let moment = |kind, start: f64, end: f64, score: f64, detail: String| Moment {
        kind,
        start: round3(start),
        end: round3(end),
        score: round3((score * factor).clamp(0.0, 1.0)),
        source: Source::RadioLog,
        detail: format!("{detail}{note}"),
    };

    // Rolls and flips: runs of rows at full deflection, one sign.
    for (kind, axis, name) in [
        (MomentKind::Roll, 0usize, "aileron"),
        (MomentKind::Flip, 1usize, "elevator"),
    ] {
        let v = |x: &StickSample| if axis == 0 { x.ail } else { x.ele };
        let mut i = 0;
        while i < s.len() {
            let sign = v(&s[i]).signum();
            if v(&s[i]).abs() < tune::ROLL_DEFLECTION {
                i += 1;
                continue;
            }
            let mut j = i;
            while j + 1 < s.len()
                && v(&s[j + 1]).abs() >= tune::ROLL_DEFLECTION
                && v(&s[j + 1]).signum() == sign
                && s[j + 1].t - s[j].t <= dt * 1.5
            {
                j += 1;
            }
            let (start, end) = (s[i].t, s[j].t + dt);
            let held = end - start;
            if (tune::ROLL_MIN_S..=2.0 * tune::ROLL_MAX_S).contains(&held) {
                let peak = s[i..=j].iter().map(|x| v(x).abs()).fold(0.0, f64::max);
                let inverted = s
                    .iter()
                    .filter(|x| x.t >= start - dt && x.t <= end + dt)
                    .any(|x| {
                        x.roll
                            .is_some_and(|r| r.abs() > std::f64::consts::FRAC_PI_2)
                            || x.pitch
                                .is_some_and(|p| p.abs() > std::f64::consts::FRAC_PI_2)
                    });
                let mut score = 0.6;
                if peak >= 0.95 {
                    score += 0.2;
                }
                if inverted {
                    score += 0.2;
                }
                if held > tune::ROLL_MAX_S {
                    score *= 0.5;
                }
                out.push(moment(
                    kind,
                    start,
                    end,
                    score,
                    format!(
                        "{name} {:.0}% for {held:.1} s{}",
                        peak * 100.0,
                        if inverted {
                            ", telemetry shows inverted"
                        } else {
                            ""
                        }
                    ),
                ));
            }
            i = j + 1;
        }
    }

    // Punches and dives, walking the throttle.
    let rise = tune::PUNCH_RISE_S.max(dt * 1.01);
    let mut i = 0;
    while i < s.len() {
        if s[i].thr > tune::PUNCH_FROM {
            i += 1;
            continue;
        }
        // The last low row before throttle comes up.
        let mut lo = i;
        while lo + 1 < s.len() && s[lo + 1].thr <= tune::PUNCH_FROM {
            lo += 1;
        }
        let Some(hi) = (lo + 1..s.len())
            .take_while(|&k| s[k].t - s[lo].t <= rise)
            .find(|&k| s[k].thr >= tune::PUNCH_TO)
        else {
            i = lo + 1;
            continue;
        };
        let mut top = hi;
        while top + 1 < s.len() && s[top + 1].thr >= tune::PUNCH_TO {
            top += 1;
        }
        let end = s[top].t + dt;
        let held = end - s[hi].t;
        // The hang before it: rows at or below HANG_THROTTLE, ending at `lo`.
        let mut h = lo;
        while h > 0 && s[h - 1].thr <= tune::HANG_THROTTLE {
            h -= 1;
        }
        let hang = if s[lo].thr <= tune::HANG_THROTTLE {
            s[lo].t + dt - s[h].t
        } else {
            0.0
        };
        let takeoff = s[..=lo].iter().all(|x| x.thr <= tune::PUNCH_FROM);
        if takeoff {
            out.push(moment(
                MomentKind::Punch,
                s[lo].t,
                end,
                0.3,
                "takeoff".into(),
            ));
        } else if h > 0 && hang >= tune::HANG_MIN_S {
            let score = if hang >= tune::HANG_LONG_S { 0.9 } else { 0.75 };
            out.push(moment(
                MomentKind::Dive,
                s[h].t,
                end,
                score,
                format!("throttle off for {hang:.1} s, then a punch"),
            ));
        } else {
            let score = if held >= tune::PUNCH_HOLD_S {
                0.85
            } else {
                0.7
            };
            out.push(moment(
                MomentKind::Punch,
                s[lo].t,
                end,
                score,
                format!(
                    "throttle {:.0}% to {:.0}% in {:.1} s, held {held:.1} s",
                    s[lo].thr * 100.0,
                    s[hi].thr * 100.0,
                    s[hi].t - s[lo].t
                ),
            ));
        }
        i = top + 1;
    }

    // A crash: big inputs right before the segment (the armed run) ends.
    let last = s[s.len() - 1].t;
    let big = s
        .iter()
        .rev()
        .take_while(|x| last - x.t <= tune::CRASH_WINDOW_S)
        .find(|x| {
            x.ail.abs().max(x.ele.abs()).max(x.rud.abs()) >= tune::CRASH_INPUT
                || x.thr >= tune::PUNCH_TO
        });
    if let Some(b) = big {
        if s.len() > 1 {
            out.push(moment(
                MomentKind::Crash,
                (b.t - 1.0).max(s[0].t),
                last + dt,
                0.3,
                "disarmed right after big inputs".into(),
            ));
        }
    }
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    out
}

// ---------- video signal ----------

/// Measurements of one small 4:4:4 frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FrameStats {
    pub mean_y: f64,
    pub sd_y: f64,
    /// Mean chroma distance from grey.
    pub sat: f64,
    /// Share of pixels that are blue.
    pub blue: f64,
    /// Mean luma difference between horizontal and between vertical neighbours.
    pub h_detail: f64,
    pub v_detail: f64,
    /// Mean luma difference from the previous sample; None for the first.
    pub change: Option<f64>,
}

/// What a sampled frame shows.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
    Live,
    Blue,
    Black,
    Blank,
    Static,
    Bars,
}

impl Signal {
    pub fn dead(self) -> bool {
        self != Signal::Live
    }
    pub fn label(self) -> &'static str {
        match self {
            Signal::Live => "picture",
            Signal::Blue => "blue no-signal screen",
            Signal::Black => "black",
            Signal::Blank => "blank screen",
            Signal::Static => "static",
            Signal::Bars => "test pattern",
        }
    }
}

/// Measures a planar YUV 4:4:4 frame of `w` x `h` (Y plane, then U, then V).
pub fn frame_stats(frame: &[u8], w: usize, h: usize, prev_y: Option<&[u8]>) -> FrameStats {
    let n = w * h;
    let (y, rest) = frame.split_at(n);
    let (u, v) = rest.split_at(n);
    let nf = n as f64;
    let mean_y = y.iter().map(|&p| p as f64).sum::<f64>() / nf;
    let sd_y = (y.iter().map(|&p| (p as f64 - mean_y).powi(2)).sum::<f64>() / nf).sqrt();
    let mut sat = 0.0;
    let mut blue = 0usize;
    for (&a, &b) in u.iter().zip(&v[..n]) {
        let (du, dv) = (a as f64 - 128.0, b as f64 - 128.0);
        sat += (du * du + dv * dv).sqrt();
        if a > tune::BLUE_U && b < tune::BLUE_V {
            blue += 1;
        }
    }
    let mut hd = 0.0;
    let mut vd = 0.0;
    for r in 0..h {
        for c in 0..w {
            let p = y[r * w + c] as f64;
            if c + 1 < w {
                hd += (p - y[r * w + c + 1] as f64).abs();
            }
            if r + 1 < h {
                vd += (p - y[(r + 1) * w + c] as f64).abs();
            }
        }
    }
    let change = prev_y.map(|py| {
        y.iter()
            .zip(py)
            .map(|(&a, &b)| (a as f64 - b as f64).abs())
            .sum::<f64>()
            / nf
    });
    FrameStats {
        mean_y,
        sd_y,
        sat: sat / nf,
        blue: blue as f64 / nf,
        h_detail: hd / (h * (w - 1)).max(1) as f64,
        v_detail: vd / ((h - 1) * w).max(1) as f64,
        change,
    }
}

/// Classifies one sampled frame. `change` must be filled in (see `classify_frames`).
pub fn classify(s: &FrameStats) -> Signal {
    use tune::*;
    let detail = (s.h_detail + s.v_detail) / 2.0;
    let change = s.change.unwrap_or(0.0);
    if s.blue >= BLUE_SHARE && s.mean_y < BLUE_MAX_Y && detail < BLUE_MAX_DETAIL {
        Signal::Blue
    } else if s.mean_y < BLACK_Y && s.sd_y < BLACK_MAX_SD {
        Signal::Black
    } else if s.sd_y < BLANK_MAX_SD && detail < BLANK_MAX_DETAIL {
        Signal::Blank
    } else if s.sd_y >= STATIC_MIN_SD
        && detail / s.sd_y >= STATIC_DETAIL_RATIO
        && change >= STATIC_CHANGE_RATIO * s.sd_y
    {
        Signal::Static
    } else if change < BARS_MAX_CHANGE
        && s.sd_y >= BARS_MIN_SD
        && s.v_detail / s.sd_y < BARS_MAX_VERTICAL_RATIO
        && s.sat >= BARS_MIN_SAT
    {
        Signal::Bars
    } else {
        Signal::Live
    }
}

/// Classifies a run of sampled frames (each `w*h*3` bytes). A frame's change is the smaller
/// of its change from the previous and to the next sample, so the first frame of a frozen
/// test pattern counts as frozen, and one frame between two pictures is not static.
pub fn classify_frames(frames: &[Vec<u8>], w: usize, h: usize) -> Vec<Signal> {
    let n = w * h;
    let mut stats: Vec<FrameStats> = frames
        .iter()
        .enumerate()
        .map(|(i, f)| frame_stats(f, w, h, (i > 0).then(|| &frames[i - 1][..n])))
        .collect();
    let to_prev: Vec<Option<f64>> = stats.iter().map(|s| s.change).collect();
    for (i, s) in stats.iter_mut().enumerate() {
        let next = to_prev.get(i + 1).copied().flatten();
        s.change = match (to_prev[i], next) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }
    stats.iter().map(classify).collect()
}

/// Dead air from per-sample classes, `step` seconds apart, in a clip of `duration` seconds.
pub struct VideoSignal {
    pub classes: Vec<Signal>,
    pub step: f64,
    pub duration: f64,
}

impl MomentSource for VideoSignal {
    fn source(&self) -> Source {
        Source::Video
    }
    fn moments(&self) -> Vec<Moment> {
        dead_air(&self.classes, self.step, self.duration)
    }
}

/// Runs of dead samples, bridged over live gaps under `DEAD_BRIDGE_S`, kept when at least
/// `DEAD_MIN_S` long. Sample `i` stands for `[i*step, (i+1)*step)`.
pub fn dead_air(classes: &[Signal], step: f64, duration: f64) -> Vec<Moment> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (i, c) in classes.iter().enumerate() {
        if !c.dead() {
            continue;
        }
        match runs.last_mut() {
            Some((_, end)) if (i - *end - 1) as f64 * step < tune::DEAD_BRIDGE_S => *end = i,
            _ => runs.push((i, i)),
        }
    }
    runs.into_iter()
        .filter_map(|(a, b)| {
            let start = a as f64 * step;
            let end = ((b + 1) as f64 * step).min(duration.max(start));
            if end - start < tune::DEAD_MIN_S {
                return None;
            }
            let dead: Vec<Signal> = classes[a..=b]
                .iter()
                .copied()
                .filter(|c| c.dead())
                .collect();
            let mut kinds: Vec<(Signal, usize)> = Vec::new();
            for c in &dead {
                match kinds.iter_mut().find(|(k, _)| k == c) {
                    Some((_, n)) => *n += 1,
                    None => kinds.push((*c, 1)),
                }
            }
            kinds.sort_by_key(|k| std::cmp::Reverse(k.1));
            let names: Vec<&str> = kinds.iter().map(|(k, _)| k.label()).collect();
            Some(Moment {
                kind: MomentKind::DeadAir,
                start: round3(start),
                end: round3(end),
                score: round3(dead.len() as f64 / (b - a + 1) as f64),
                source: Source::Video,
                detail: names.join(", "),
            })
        })
        .collect()
}

/// The parts of a clip outside dead air, each at least `KEEP_MIN_S` long. Empty when there is
/// no dead air (nothing to trim).
pub fn keep_ranges(dead: &[Moment], duration: f64) -> Vec<Span> {
    let mut dead: Vec<&Moment> = dead
        .iter()
        .filter(|m| m.kind == MomentKind::DeadAir)
        .collect();
    if dead.is_empty() {
        return Vec::new();
    }
    dead.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut out = Vec::new();
    let mut at = 0.0;
    for d in dead {
        if d.start - at >= tune::KEEP_MIN_S {
            out.push(Span {
                start: round3(at),
                end: round3(d.start),
            });
        }
        at = at.max(d.end);
    }
    if duration - at >= tune::KEEP_MIN_S {
        out.push(Span {
            start: round3(at),
            end: round3(duration),
        });
    }
    out
}

/// The result of scanning a clip's frames.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SignalScan {
    /// Seconds between sampled frames.
    pub step: f64,
    pub samples: usize,
    pub dead_air: Vec<Moment>,
    /// Suggested ranges to keep; empty when the clip has no dead air.
    pub keep: Vec<Span>,
}

/// Samples `src` at about `SAMPLE_FPS` and finds its dead air.
pub fn scan_signal(
    tools: &Tools,
    src: &Path,
    fps: Option<f64>,
    duration: f64,
) -> Result<SignalScan> {
    let (frames, step) = media::sample_frames(
        tools,
        src,
        fps,
        tune::SAMPLE_FPS,
        tune::SAMPLE_W,
        tune::SAMPLE_H,
    )?;
    let src = VideoSignal {
        classes: classify_frames(&frames, tune::SAMPLE_W, tune::SAMPLE_H),
        step,
        duration,
    };
    let dead_air = src.moments();
    Ok(SignalScan {
        step,
        samples: frames.len(),
        keep: keep_ranges(&dead_air, duration),
        dead_air,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Samples every `dt` seconds for `secs`, built by `f(t)`.
    fn track(dt: f64, secs: f64, f: impl Fn(f64) -> (f64, f64, f64)) -> Vec<StickSample> {
        let n = (secs / dt).round() as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 * dt;
                let (ail, ele, thr) = f(t);
                StickSample {
                    t,
                    ail,
                    ele,
                    thr,
                    ..Default::default()
                }
            })
            .collect()
    }

    /// Cruise at half throttle with one feature at a given time.
    fn flight(t: f64) -> (f64, f64, f64) {
        let ail = if (10.0..10.6).contains(&t) { 1.0 } else { 0.0 };
        let ele = if (20.0..20.5).contains(&t) { -0.9 } else { 0.0 };
        let thr = if t < 2.0 {
            0.0 // on the ground
        } else if t < 2.5 {
            0.9 // takeoff
        } else if (30.0..30.2).contains(&t) {
            0.3
        } else if (30.2..32.0).contains(&t) {
            1.0 // punch
        } else if (40.0..42.5).contains(&t) {
            0.05 // hang
        } else if (42.5..44.0).contains(&t) {
            0.95 // and punch
        } else {
            0.5
        };
        (ail, ele, thr)
    }

    fn kinds(m: &[Moment]) -> Vec<(MomentKind, f64)> {
        m.iter().map(|m| (m.kind, m.start)).collect()
    }

    #[test]
    fn fine_log_finds_each_move() {
        let s = track(0.1, 60.0, flight);
        let m = detect_sticks(&s, 0.1);
        let k = kinds(&m);
        assert!(
            k.iter()
                .any(|(k, t)| *k == MomentKind::Roll && (*t - 10.0).abs() < 0.11),
            "{k:?}"
        );
        assert!(
            k.iter()
                .any(|(k, t)| *k == MomentKind::Flip && (*t - 20.0).abs() < 0.11),
            "{k:?}"
        );
        assert!(
            k.iter()
                .any(|(k, t)| *k == MomentKind::Punch && (*t - 30.1).abs() < 0.11),
            "{k:?}"
        );
        assert!(
            k.iter()
                .any(|(k, t)| *k == MomentKind::Dive && (*t - 40.0).abs() < 0.11),
            "{k:?}"
        );
        // Takeoff is a punch, scored low; the dive's punch is not reported twice.
        let punches: Vec<&Moment> = m.iter().filter(|m| m.kind == MomentKind::Punch).collect();
        assert_eq!(punches.len(), 2, "{punches:?}");
        assert_eq!(punches[0].detail, "takeoff");
        assert!(punches[1].score > 0.8, "held punch scores high");
        // Cruising ends calmly: no crash.
        assert!(!k.iter().any(|(k, _)| *k == MomentKind::Crash));
        let roll = m.iter().find(|m| m.kind == MomentKind::Roll).unwrap();
        assert!((roll.end - 10.6).abs() < 0.11);
        assert!(!roll.detail.contains("coarse"));
    }

    #[test]
    fn coarse_log_degrades_and_says_so() {
        let s = track(0.5, 60.0, flight);
        let m = detect_sticks(&s, 0.5);
        let fine = detect_sticks(&track(0.1, 60.0, flight), 0.1);
        for kind in [MomentKind::Roll, MomentKind::Flip, MomentKind::Dive] {
            let c = m
                .iter()
                .find(|m| m.kind == kind)
                .expect("still found at 0.5 s");
            let f = fine.iter().find(|m| m.kind == kind).unwrap();
            assert!(c.score < f.score, "{kind:?} scores lower on a coarse log");
            assert!(c.detail.contains("coarse log"));
            assert!((c.start - f.start).abs() <= 0.5);
        }
    }

    #[test]
    fn short_and_partial_inputs_are_not_moments() {
        // A 0.2 s full-stick blip, and a long 70% roll: neither counts.
        let s = track(0.1, 20.0, |t| {
            let ail = if (5.0..5.2).contains(&t) {
                1.0
            } else if (10.0..12.0).contains(&t) {
                0.7
            } else {
                0.0
            };
            (ail, 0.0, 0.5)
        });
        let m = detect_sticks(&s, 0.1);
        assert!(m.iter().all(|m| m.kind != MomentKind::Roll), "{m:?}");
        // A slow throttle ramp is not a punch.
        let s = track(0.1, 20.0, |t| {
            let thr = if t < 10.0 {
                0.5
            } else if t < 11.0 {
                0.2
            } else {
                (0.2 + (t - 11.0) * 0.1).min(1.0)
            };
            (0.0, 0.0, thr)
        });
        assert!(detect_sticks(&s, 0.1)
            .iter()
            .all(|m| m.kind != MomentKind::Punch));
    }

    #[test]
    fn crash_and_attitude() {
        // Full aileron right up to the end of the armed run.
        let mut s = track(0.1, 10.0, |t| (if t > 9.0 { 1.0 } else { 0.0 }, 0.0, 0.5));
        s[92].roll = Some(3.0);
        let m = detect_sticks(&s, 0.1);
        let crash = m.iter().find(|m| m.kind == MomentKind::Crash).unwrap();
        assert!(crash.score <= 0.3);
        assert!((crash.end - 10.0).abs() < 0.11);
        let roll = m.iter().find(|m| m.kind == MomentKind::Roll).unwrap();
        assert!(roll.detail.contains("inverted"));
        assert!(roll.score > 0.9);
    }

    #[test]
    fn radio_log_source_times_rows_from_the_clip_start() {
        use crate::logs::{LogRow, Sticks};
        let t0 = chrono::NaiveDate::from_ymd_opt(2026, 9, 30)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap();
        let rows: Vec<LogRow> = (0..100)
            .map(|i| LogRow {
                time: t0 + chrono::Duration::milliseconds(i * 100),
                sticks: Some(Sticks {
                    ail: if (30..40).contains(&i) { 1024.0 } else { 0.0 },
                    ele: 0.0,
                    thr: 0.0,
                    rud: 0.0,
                }),
                roll: None,
                pitch: None,
                rx_bat: None,
                lq: None,
                rssi: None,
                model: None,
            })
            .collect();
        let end = rows[99].time;
        let src = RadioLog::from_rows(&rows, &[(t0, end)], t0, 5.0);
        assert_eq!(src.interval(), Some(0.1));
        assert_eq!(src.source(), Source::RadioLog);
        let roll = src
            .moments()
            .into_iter()
            .find(|m| m.kind == MomentKind::Roll)
            .unwrap();
        assert!(
            (roll.start - 8.0).abs() < 1e-6,
            "3.0 s into the log + 5 s offset"
        );
    }

    /// A 64x48 4:4:4 frame from per-pixel (y, u, v).
    fn frame(mut f: impl FnMut(usize, usize) -> (u8, u8, u8)) -> Vec<u8> {
        let (w, h) = (tune::SAMPLE_W, tune::SAMPLE_H);
        let mut out = vec![0u8; w * h * 3];
        for r in 0..h {
            for c in 0..w {
                let (y, u, v) = f(r, c);
                out[r * w + c] = y;
                out[w * h + r * w + c] = u;
                out[2 * w * h + r * w + c] = v;
            }
        }
        out
    }

    fn noise(seed: u64) -> Vec<u8> {
        let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        frame(|_, _| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((x >> 33) as u8, 128, 128)
        })
    }

    /// A smooth moving scene: gradients plus a bright square that moves each frame.
    fn scene(i: usize) -> Vec<u8> {
        frame(|r, c| {
            let sq = (c + 64 - (i * 3) % 64) % 64 < 12 && (10..30).contains(&r);
            let y = if sq { 220 } else { (40 + r * 2 + c) as u8 };
            (y, (110 + c / 4) as u8, (130 + r / 4) as u8)
        })
    }

    fn bars() -> Vec<u8> {
        const C: [(u8, u8, u8); 7] = [
            (180, 128, 128),
            (168, 44, 136),
            (145, 147, 44),
            (133, 63, 52),
            (63, 193, 204),
            (51, 109, 212),
            (28, 212, 120),
        ];
        frame(|_, c| C[c * 7 / tune::SAMPLE_W])
    }

    #[test]
    fn classifies_synthetic_frames() {
        let one =
            |f: Vec<u8>, g: Vec<u8>| classify_frames(&[f, g], tune::SAMPLE_W, tune::SAMPLE_H)[1];
        assert_eq!(
            one(frame(|_, _| (29, 255, 107)), frame(|_, _| (29, 255, 107))),
            Signal::Blue
        );
        // Blue with OSD text on it is still a blue screen.
        let osd = || {
            frame(|r, c| {
                if r == 20 && c % 3 == 0 {
                    (200, 128, 128)
                } else {
                    (41, 204, 110)
                }
            })
        };
        assert_eq!(one(osd(), osd()), Signal::Blue);
        assert_eq!(
            one(frame(|_, _| (16, 128, 128)), frame(|_, _| (16, 128, 128))),
            Signal::Black
        );
        assert_eq!(
            one(frame(|_, _| (128, 128, 128)), frame(|_, _| (128, 128, 128))),
            Signal::Blank
        );
        assert_eq!(one(noise(1), noise(2)), Signal::Static);
        assert_eq!(one(bars(), bars()), Signal::Bars);
        assert_eq!(one(scene(0), scene(1)), Signal::Live);
        // A frozen smooth scene is not a test pattern: its rows differ.
        assert_eq!(one(scene(0), scene(0)), Signal::Live);
    }

    #[test]
    fn dead_air_runs_and_keep_ranges() {
        use Signal::*;
        let step = 0.5;
        // 0-5 s blue, 5-20 s flight with a 1.5 s static breakup at 10 s, 20-24 s static
        // with one live sample inside, then flight to 30 s.
        let mut c = vec![Live; 60];
        for x in c.iter_mut().take(10) {
            *x = Blue;
        }
        for x in c.iter_mut().take(23).skip(20) {
            *x = Static;
        }
        for x in c.iter_mut().take(48).skip(40) {
            *x = Static;
        }
        c[44] = Live;
        let dead = dead_air(&c, step, 30.0);
        assert_eq!(dead.len(), 2, "{dead:?}");
        assert_eq!((dead[0].start, dead[0].end), (0.0, 5.0));
        assert_eq!(dead[0].detail, "blue no-signal screen");
        assert_eq!((dead[1].start, dead[1].end), (20.0, 24.0));
        assert!(
            dead[1].score < 1.0,
            "the bridged live sample lowers the score"
        );
        let keep = keep_ranges(&dead, 30.0);
        assert_eq!(
            keep,
            vec![
                Span {
                    start: 5.0,
                    end: 20.0
                },
                Span {
                    start: 24.0,
                    end: 30.0
                }
            ]
        );
        // No dead air: nothing to suggest.
        assert!(keep_ranges(&dead_air(&[Live; 20], step, 10.0), 10.0).is_empty());
        // All dead air: nothing to keep.
        assert!(keep_ranges(&dead_air(&[Bars; 20], step, 10.0), 10.0).is_empty());
    }
}
