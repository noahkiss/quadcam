//! Shape-first radio-log matching, for every source.
//!
//! A clip matches the armed segments ("packs") whose shape fits it: the clip's length
//! against the packs' armed span, and, for a run of clips, the order of the packs and the
//! gaps between them. Clocks only break ties and confirm: the clip clock (DJI) against the
//! log time when the radio clock is believable, and the gaps between clip clocks against
//! the gaps between packs of one power-on run even when it is not. A log whose radio clock
//! reset (`2000-01-01`) still matches; its rows keep their file order, and a time that
//! jumps back starts a new run.
//!
//! An analog clip with dead air is matched by its picture instead of its length. On a whoop
//! where the battery powers the camera and the VTX, the goggles' DVR keeps recording across
//! battery swaps, so each picture stretch (a keep range) is one battery: its packs sit
//! inside it, with unarmed picture around them, and a battery swap is dead air. Short dead
//! air while armed is signal breakup, not a swap. Voltage steps scale with the pack's cell
//! count, estimated from the log (`cells`). The matcher slides the log along the clip
//! (`Found::offset_s`) and scores each place by how the packs fit the stretches
//! (`picture_fit`). Files a DVR split from one recording
//! (`Want::follows`) are matched as one timeline, so a pack may span the file boundary.
//!
//! The EdgeTX model filters first: a log whose model a profile lists is a candidate only
//! for clips that profile fits (the clip's own profile, else its video system). A model no
//! profile lists matches by shape alone, and the reason says so.

use crate::logs::{self, Badge, LogRow, Tunables};
use crate::metadata::Profile;
use crate::sources::SourceKind;
use chrono::{NaiveDate, NaiveDateTime};
use std::path::Path;

/// One log file's rows of one day, in file order, and its EdgeTX model.
#[derive(Debug, Clone)]
pub struct LogFile {
    pub model: Option<String>,
    pub rows: Vec<LogRow>,
}

/// Every log file under `dir`, rows in file order.
pub fn read_files(dir: &Path) -> Vec<LogFile> {
    logs::csv_files(dir)
        .iter()
        .filter_map(|p| logs::read_rows(p).ok())
        .filter(|rows| !rows.is_empty())
        .map(|rows| LogFile {
            model: rows[0].model.as_deref().map(str::to_string),
            rows,
        })
        .collect()
}

/// The files cut down to the rows of `day`, empty files dropped.
pub fn files_of_day(files: &[LogFile], day: NaiveDate) -> Vec<LogFile> {
    files
        .iter()
        .map(|f| LogFile {
            model: f.model.clone(),
            rows: f
                .rows
                .iter()
                .filter(|r| r.time.date() == day)
                .cloned()
                .collect(),
        })
        .filter(|f| !f.rows.is_empty())
        .collect()
}

/// An armed segment: rows `a..b` of file `file`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seg {
    pub file: usize,
    pub a: usize,
    pub b: usize,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    /// Numbered across all files. A new session starts at a long gap, a time that goes
    /// back (a power cycle with a reset clock) or a new file.
    pub session: usize,
    /// The receiver battery at the first and last rows that read one (`RxBt`). A pack that
    /// starts well above the last one's end is on a new battery.
    pub v_start: Option<f64>,
    pub v_end: Option<f64>,
}

impl Seg {
    pub fn secs(&self) -> f64 {
        (self.end - self.start).num_milliseconds() as f64 / 1000.0
    }
}

/// Packs shorter than this are power-on blips, not flights.
const MIN_PACK_S: f64 = 1.0;

/// The segments of every file, in file order. A gap over `segment_gap_s` ends a segment;
/// a gap over `session_gap_min`, a time going back, or a new file starts a new session.
pub fn segments(files: &[LogFile], tun: &Tunables) -> Vec<Seg> {
    let gap = chrono::Duration::milliseconds((tun.segment_gap_s * 1000.0) as i64);
    let sgap = chrono::Duration::milliseconds((tun.session_gap_min * 60_000.0) as i64);
    let mut out: Vec<Seg> = Vec::new();
    let mut session = 0;
    for (fi, f) in files.iter().enumerate() {
        let mut cur: Option<Seg> = None;
        for (i, r) in f.rows.iter().enumerate() {
            let t = r.time;
            match &mut cur {
                Some(s) if t >= s.end && t - s.end <= gap => {
                    s.end = t;
                    s.b = i + 1;
                }
                _ => {
                    if let Some(s) = cur.take() {
                        if t < s.end || t - s.end > sgap {
                            session += 1;
                        }
                        out.push(s);
                    }
                    cur = Some(Seg {
                        file: fi,
                        a: i,
                        b: i + 1,
                        start: t,
                        end: t,
                        session,
                        v_start: None,
                        v_end: None,
                    });
                }
            }
        }
        if let Some(s) = cur {
            out.push(s);
        }
        session += 1;
    }
    out.retain(|s| s.secs() >= MIN_PACK_S);
    for s in &mut out {
        // 0 V means no telemetry yet, not a reading.
        let v = || {
            files[s.file].rows[s.a..s.b]
                .iter()
                .filter_map(|r| r.rx_bat.filter(|v| *v > 0.0))
        };
        s.v_start = v().next();
        s.v_end = v().next_back();
    }
    out
}

/// What matching needs from one clip.
#[derive(Debug, Clone, Default)]
pub struct Want {
    pub duration: f64,
    /// The clip clock in local time (DJI), when believable.
    pub clock: Option<NaiveDateTime>,
    /// The EdgeTX models the clip's profile(s) list; `None` when none do.
    pub models: Option<Vec<String>>,
    /// The flying day, when known apart from the log (a library clip). Clips of different
    /// days never share one power-on run.
    pub day: Option<NaiveDate>,
    /// The picture stretches (keep ranges, in clip seconds) of an analog clip with dead
    /// air. Empty: the clip is matched by its length.
    pub keep: Vec<(f64, f64)>,
    /// The clip is the next file of the recording the one before it started (a DVR split
    /// it): both are matched as one timeline.
    pub follows: bool,
}

/// The EdgeTX models for a clip: its own profile's, else those of every profile whose
/// video system is the clip's source. `None` when they list none.
pub fn clip_models(
    profile: Option<&str>,
    kind: SourceKind,
    profiles: &[Profile],
) -> Option<Vec<String>> {
    let own = profile.filter(|n| !n.trim().is_empty()).and_then(|n| {
        profiles
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(n.trim()))
    });
    let models: Vec<String> = match own {
        Some(p) => p.edgetx_models.clone(),
        None => profiles
            .iter()
            .filter(|p| p.video_system.trim().eq_ignore_ascii_case(kind.label()))
            .flat_map(|p| p.edgetx_models.iter().cloned())
            .collect(),
    };
    let models: Vec<String> = models
        .into_iter()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .collect();
    (!models.is_empty()).then_some(models)
}

/// The profile that lists an EdgeTX model.
pub fn profile_of_model<'a>(model: &str, profiles: &'a [Profile]) -> Option<&'a Profile> {
    profiles.iter().find(|p| {
        p.edgetx_models
            .iter()
            .any(|m| m.trim().eq_ignore_ascii_case(model.trim()))
    })
}

/// A clip's match.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub badge: Badge,
    /// Index of the first claimed segment in the list given to `match_all`, and the count.
    pub first: usize,
    pub segments: usize,
    pub start: NaiveDateTime,
    /// Armed span, first start to last end, in seconds.
    pub span_s: f64,
    /// Clip clock minus log start, in seconds, when the clip has a clock and the radio
    /// clock is believable.
    pub skew_s: Option<f64>,
    pub model: Option<String>,
    /// The clip second of the first claimed pack's start, when the picture placed it.
    /// Negative when the recording started after the arm.
    pub offset_s: Option<f64>,
    /// Why, in a few words.
    pub reason: String,
}

/// A candidate claim: segments `first..=last` (one file, one session) for clip `clip`.
#[derive(Debug, Clone, Copy)]
struct Cand {
    clip: usize,
    first: usize,
    last: usize,
    cost: f64,
    /// How the packs fit the picture, for a clip matched by it.
    fit: Option<Fit>,
}

/// How a clip's picture is scored against the packs (`picture_fit`).
pub mod pic {
    /// Slack at a picture stretch's edges: dead-air detection samples frames, and a log row
    /// comes every half second or so.
    pub const EDGE_S: f64 = 4.0;
    /// A pack armed in dead air or across it: the battery powers the VTX, so a swap cannot
    /// happen while armed. Plus `DARK_PER_S` per second of it.
    pub const CROSS: f64 = 40.0;
    pub const DARK_PER_S: f64 = 0.5;
    /// Dead air of at most this inside an armed pack, with picture on both sides, is signal
    /// breakup (range, a crash, interference): only `DARK_PER_S` per second, and the
    /// stretches on both sides count as one battery.
    pub const BREAKUP_S: f64 = 10.0;
    /// A picture stretch with no pack: free up to `EMPTY_FREE_S` (a battery in, never
    /// armed), then `EMPTY_PER_S` per second, at most `EMPTY_MAX`.
    pub const EMPTY_FREE_S: f64 = 30.0;
    pub const EMPTY_PER_S: f64 = 0.25;
    pub const EMPTY_MAX: f64 = 40.0;
    /// A new battery (the pack starts `SWAP_V` a cell above the last one's end) with no
    /// dead air between the packs.
    pub const SWAP_IN_STRETCH: f64 = 15.0;
    pub const SWAP_V: f64 = 0.3;
    /// Packs of one battery (within `SAME_V` a cell) with dead air between: an unplug and
    /// replug.
    pub const REPLUG: f64 = 5.0;
    pub const SAME_V: f64 = 0.15;
    /// A cell holds at most this (LiHV, with telemetry error). `cells` divides by it.
    pub const CELL_MAX_V: f64 = 4.5;
    /// The cost of leaving a clip unmatched. A fit that costs more is no match.
    pub const UNMATCHED: f64 = 60.0;
    /// A fit at or below this, with no pack in dead air and no swap without dead air, is
    /// "matched".
    pub const MATCHED_MAX: f64 = 12.0;
    /// Another solution within this of the best one makes the match only "likely".
    pub const MARGIN: f64 = 8.0;
}

/// How a run of packs fits a clip's picture stretches at one offset.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Fit {
    /// The clip second of the run's first pack start.
    pub offset: f64,
    /// Half the width of the offsets that fit as well.
    pub spread: f64,
    /// Stretches that hold a pack, of all.
    pub filled: usize,
    pub stretches: usize,
    /// Packs armed in or across dead air.
    pub crossing: usize,
    /// Packs armed across short dead air (`pic::BREAKUP_S`): signal breakup.
    pub breakups: usize,
    /// Battery swaps at dead air, and without it.
    pub swaps_at_gap: usize,
    pub swaps_in_stretch: usize,
    /// Packs of one battery with dead air between.
    pub replugs: usize,
}

/// The cell count of the packs in `run`, from the highest pack-start battery reading
/// (`RxBt`, nearly rested at arm): that voltage over `pic::CELL_MAX_V`, rounded up, 1 to 8.
/// 1 when the log reads no battery.
pub fn cells(run: &[Seg]) -> f64 {
    let v = run.iter().filter_map(|s| s.v_start).fold(0.0, f64::max);
    if v <= 0.0 {
        1.0
    } else {
        (v / pic::CELL_MAX_V).ceil().clamp(1.0, 8.0)
    }
}

/// Scores `run` (consecutive packs of one session) placed with its first pack's start at
/// clip second `offset`, against the picture stretches `keep` of a clip `duration` long.
/// 0 is a perfect fit; see `pic` for the costs.
pub fn picture_fit(run: &[Seg], offset: f64, keep: &[(f64, f64)], duration: f64) -> (f64, Fit) {
    use pic::*;
    let base = run[0].start;
    let rel = |t: NaiveDateTime| (t - base).num_milliseconds() as f64 / 1000.0;
    let n_cells = cells(run);
    let mut cost = 0.0;
    let mut fit = Fit {
        offset,
        stretches: keep.len(),
        ..Default::default()
    };
    // Stretches joined by signal breakup count as one battery: `group[k]` is the first
    // stretch of k's group.
    let mut group: Vec<usize> = (0..keep.len()).collect();
    // Per pack, the stretch that holds it.
    let mut home: Vec<Option<usize>> = Vec::with_capacity(run.len());
    let mut touched = vec![false; keep.len()];
    for s in run {
        let (a, b) = (offset + rel(s.start), offset + rel(s.end));
        let (va, vb) = (a.max(0.0), b.min(duration));
        let len = (vb - va).max(0.0);
        let mut best: Option<(usize, f64)> = None;
        let ov_of = |k: usize| {
            let (ks, ke) = keep[k];
            (vb.min(ke + EDGE_S) - va.max(ks - EDGE_S)).max(0.0)
        };
        for k in 0..keep.len() {
            let ov = ov_of(k);
            if ov > EDGE_S.min(len / 2.0) {
                touched[k] = true;
            }
            if best.is_none_or(|(_, o)| ov > o) {
                best = Some((k, ov));
            }
        }
        let (k, ov) = best.unwrap_or((0, 0.0));
        let dark = len - ov;
        if dark <= 0.05 {
            home.push(Some(k));
            continue;
        }
        // Breakup: the pack starts in stretch `first` and ends in stretch `last`, and every
        // gap between them is short dead air.
        let inside = |x: f64| {
            keep.iter()
                .position(|(ks, ke)| x >= ks - EDGE_S && x <= ke + EDGE_S)
        };
        if let (Some(first), Some(last)) = (inside(va), inside(vb)) {
            let gaps: Vec<f64> = (first..last).map(|g| keep[g + 1].0 - keep[g].1).collect();
            if last > first && gaps.iter().all(|g| *g <= BREAKUP_S) {
                fit.breakups += 1;
                cost += gaps.iter().sum::<f64>() * DARK_PER_S;
                for g in first..=last {
                    touched[g] = true;
                    group[g] = group[first];
                }
                home.push(Some(first));
                continue;
            }
        }
        fit.crossing += 1;
        cost += CROSS + dark * DARK_PER_S;
        home.push(None);
    }
    let home: Vec<Option<usize>> = home.into_iter().map(|h| h.map(|k| group[k])).collect();
    fit.stretches = (0..keep.len()).filter(|&k| group[k] == k).count();
    for (k, (ks, ke)) in keep.iter().enumerate() {
        if home.contains(&Some(group[k])) {
            if group[k] == k {
                fit.filled += 1;
            }
        } else if !touched[k] {
            cost += ((ke - ks - EMPTY_FREE_S).max(0.0) * EMPTY_PER_S).min(EMPTY_MAX);
        }
    }
    for i in 1..run.len() {
        let (Some(x), Some(y)) = (home[i - 1], home[i]) else {
            continue;
        };
        let (Some(end), Some(start)) = (run[i - 1].v_end, run[i].v_start) else {
            continue;
        };
        let jump = start - end;
        if jump >= SWAP_V * n_cells {
            if x == y {
                fit.swaps_in_stretch += 1;
                cost += SWAP_IN_STRETCH;
            } else {
                fit.swaps_at_gap += 1;
            }
        } else if jump.abs() <= SAME_V * n_cells && x != y {
            fit.replugs += 1;
            cost += REPLUG;
        }
    }
    (cost, fit)
}

/// Picture candidates for clip `ci`: every run of packs of one session that some offset
/// puts in the clip, at its best offset. A run holds every pack of the session the clip
/// would have seen: a pack the DVR recorded cannot be left out.
fn picture_cands(
    ci: usize,
    w: &Want,
    segs: &[Seg],
    eligible: &dyn Fn(&Seg) -> bool,
    clock_ok: bool,
    tun: &Tunables,
) -> Vec<Cand> {
    use pic::EDGE_S;
    let d = w.duration;
    let mut out = Vec::new();
    let mut g0 = 0;
    while g0 < segs.len() {
        let mut g1 = g0;
        while g1 < segs.len()
            && segs[g1].file == segs[g0].file
            && segs[g1].session == segs[g0].session
        {
            g1 += 1;
        }
        let group = &segs[g0..g1];
        let next = g1;
        if !eligible(&group[0]) {
            g0 = next;
            continue;
        }
        let base = group[0].start;
        let rel = |t: NaiveDateTime| (t - base).num_milliseconds() as f64 / 1000.0;
        let packs: Vec<(f64, f64)> = group.iter().map(|s| (rel(s.start), rel(s.end))).collect();
        // Offsets of the group's first pack where a pack edge meets a stretch or clip edge.
        let mut edges: Vec<f64> = vec![0.0, d];
        for (ks, ke) in &w.keep {
            edges.extend([*ks, *ke]);
        }
        let mut at: Vec<f64> = Vec::new();
        for (a, b) in &packs {
            for e in &edges {
                for x in [-EDGE_S, 0.0, EDGE_S] {
                    at.extend([e + x - a, e + x - b]);
                }
            }
        }
        at.sort_by(f64::total_cmp);
        at.dedup_by(|x, y| (*x - *y).abs() < 1e-6);
        let mids: Vec<f64> = at.windows(2).map(|p| (p[0] + p[1]) / 2.0).collect();
        at.extend(mids);
        // Per run, every offset tried with its cost.
        let mut runs: std::collections::BTreeMap<(usize, usize), Vec<(f64, f64)>> =
            Default::default();
        for o in at {
            let seen: Vec<usize> = (0..packs.len())
                .filter(|&i| {
                    let (a, b) = (o + packs[i].0, o + packs[i].1);
                    let vis = b.min(d) - a.max(0.0);
                    vis > EDGE_S.min((b - a) / 2.0)
                })
                .collect();
            let (Some(&i), Some(&k)) = (seen.first(), seen.last()) else {
                continue;
            };
            let off = o + packs[i].0;
            let (cost, _) = picture_fit(&group[i..=k], off, &w.keep, d);
            runs.entry((i, k)).or_default().push((off, cost));
        }
        for ((i, k), tries) in runs {
            let min = tries.iter().map(|t| t.1).fold(f64::INFINITY, f64::min);
            let good: Vec<f64> = tries
                .iter()
                .filter(|t| t.1 <= min + 1e-6)
                .map(|t| t.0)
                .collect();
            let lo = good.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = good.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            // The middle of the offsets that fit best, when it fits as well; else the first.
            let mid = (lo + hi) / 2.0;
            let (mut cost, mut fit) = picture_fit(&group[i..=k], mid, &w.keep, d);
            fit.spread = (hi - lo) / 2.0;
            if cost > min + 1e-6 {
                (cost, fit) = picture_fit(&group[i..=k], good[0], &w.keep, d);
                fit.spread = 0.0;
            }
            if let (true, Some(clock)) = (clock_ok, w.clock) {
                let zero =
                    group[i].start - chrono::Duration::milliseconds((fit.offset * 1000.0) as i64);
                let skew = (clock - zero).num_milliseconds().abs() as f64 / 1000.0;
                cost += (skew / 100.0).min(tun.tolerance_s);
            }
            if cost < pic::UNMATCHED {
                out.push(Cand {
                    clip: ci,
                    first: g0 + i,
                    last: g0 + k,
                    cost,
                    fit: Some(fit),
                });
            }
        }
        g0 = next;
    }
    out
}

/// Matches clips (in recording order) to segments (file order; time order within a
/// session). `clock_ok`: the log's clock is believable, so a clip clock may be compared to
/// it directly. A clip that `follows` the one before it is matched with it as one
/// timeline; each file then gets the packs it shows.
pub fn match_all(
    clips: &[Want],
    segs: &[Seg],
    files: &[LogFile],
    profiles: &[Profile],
    clock_ok: bool,
    tun: &Tunables,
) -> Vec<Option<Found>> {
    // Timelines: runs of clips where each later one follows the one before, all with a
    // picture and of one day.
    let mut lines: Vec<Vec<usize>> = Vec::new();
    for (i, w) in clips.iter().enumerate() {
        match lines.last_mut() {
            Some(l)
                if w.follows
                    && !w.keep.is_empty()
                    && l.iter()
                        .all(|&j| !clips[j].keep.is_empty() && clips[j].day == w.day) =>
            {
                l.push(i)
            }
            _ => lines.push(vec![i]),
        }
    }
    if lines.iter().all(|l| l.len() == 1) {
        return match_timelines(clips, segs, files, profiles, clock_ok, tun);
    }
    let merged: Vec<Want> = lines
        .iter()
        .map(|l| {
            let mut w = clips[l[0]].clone();
            let mut at = 0.0;
            w.keep.clear();
            for &j in l {
                for (a, b) in &clips[j].keep {
                    let (a, b) = (a + at, b + at);
                    match w.keep.last_mut() {
                        // A stretch running to the end of one file goes on in the next.
                        Some(last) if a - last.1 <= 1.0 => last.1 = b,
                        _ => w.keep.push((a, b)),
                    }
                }
                at += clips[j].duration;
            }
            w.duration = at;
            w.follows = false;
            w
        })
        .collect();
    let found = match_timelines(&merged, segs, files, profiles, clock_ok, tun);
    let mut out: Vec<Option<Found>> = vec![None; clips.len()];
    for (l, f) in lines.iter().zip(found) {
        let Some(f) = f else { continue };
        if l.len() == 1 {
            out[l[0]] = Some(f);
            continue;
        }
        let Some(o) = f.offset_s else { continue };
        let base = segs[f.first].start;
        let rel = |t: NaiveDateTime| (t - base).num_milliseconds() as f64 / 1000.0;
        let mut at = 0.0;
        for (n, &j) in l.iter().enumerate() {
            let (t0, t1) = (at, at + clips[j].duration);
            at = t1;
            let mine: Vec<usize> = (f.first..f.first + f.segments)
                .filter(|&k| {
                    let (a, b) = (o + rel(segs[k].start), o + rel(segs[k].end));
                    b.min(t1) - a.max(t0) > 1.0
                })
                .collect();
            let (Some(&first), Some(&last)) = (mine.first(), mine.last()) else {
                continue;
            };
            out[j] = Some(Found {
                first,
                segments: last - first + 1,
                start: segs[first].start,
                span_s: (segs[last].end - segs[first].start).num_milliseconds() as f64 / 1000.0,
                offset_s: Some(o + rel(segs[first].start) - t0),
                reason: format!(
                    "{}; file {} of {} of one DVR recording",
                    f.reason,
                    n + 1,
                    l.len()
                ),
                ..f.clone()
            });
        }
    }
    out
}

/// `match_all` for clips that are each one timeline.
fn match_timelines(
    clips: &[Want],
    segs: &[Seg],
    files: &[LogFile],
    profiles: &[Profile],
    clock_ok: bool,
    tun: &Tunables,
) -> Vec<Option<Found>> {
    let eligible = |w: &Want, s: &Seg| -> bool {
        let Some(model) = files[s.file].model.as_deref() else {
            return true;
        };
        match &w.models {
            None => true,
            Some(ms) => {
                ms.iter().any(|m| m.eq_ignore_ascii_case(model.trim()))
                    || profile_of_model(model, profiles).is_none()
            }
        }
    };
    let span =
        |a: usize, b: usize| (segs[b].end - segs[a].start).num_milliseconds() as f64 / 1000.0;
    let mut cands: Vec<Cand> = Vec::new();
    for (ci, w) in clips.iter().enumerate() {
        if w.duration <= 0.0 {
            continue;
        }
        if !w.keep.is_empty() {
            cands.extend(picture_cands(
                ci,
                w,
                segs,
                &|s: &Seg| eligible(w, s),
                clock_ok,
                tun,
            ));
            continue;
        }
        let limit = w.duration + tun.tolerance_s;
        for j in 0..segs.len() {
            if !eligible(w, &segs[j]) {
                continue;
            }
            let mut k = j;
            while k < segs.len()
                && segs[k].session == segs[j].session
                && segs[k].file == segs[j].file
                && span(j, k) <= limit
            {
                let mut cost = (w.duration - span(j, k)).abs();
                if let (true, Some(clock)) = (clock_ok, w.clock) {
                    let skew = (clock - segs[j].start).num_milliseconds().abs() as f64 / 1000.0;
                    cost += (skew / 100.0).min(tun.tolerance_s);
                }
                cands.push(Cand {
                    clip: ci,
                    first: j,
                    last: k,
                    cost,
                    fit: None,
                });
                k += 1;
            }
        }
    }
    let best = solve(clips, segs, &cands, clock_ok, tun, None);
    let mut out: Vec<Option<Found>> = vec![None; clips.len()];
    for (ci, pick) in best.picks.iter().enumerate() {
        let Some(c) = pick.map(|p| cands[p]) else {
            continue;
        };
        let w = &clips[ci];
        let s = segs[c.first];
        let span_s = span(c.first, c.last);
        let n = c.last - c.first + 1;
        let skew_s = match (clock_ok, w.clock, c.fit) {
            (true, Some(clock), Some(f)) => {
                Some((clock - s.start).num_milliseconds() as f64 / 1000.0 + f.offset)
            }
            (true, Some(clock), None) => Some((clock - s.start).num_milliseconds() as f64 / 1000.0),
            _ => None,
        };
        let packs = if n == 1 {
            "1 pack".to_string()
        } else {
            format!("{n} packs")
        };
        let armed: f64 = segs[c.first..=c.last].iter().map(Seg::secs).sum();
        let mut why = vec![format!(
            "{packs} from {} ({:.0} s armed over {:.0} s) in a {:.0} s clip",
            s.start.format("%H:%M:%S"),
            armed,
            span_s,
            w.duration
        )];
        let mut badge;
        let margin;
        match c.fit {
            Some(f) => {
                margin = pic::MARGIN;
                badge = if f.crossing == 0 && f.swaps_in_stretch == 0 && c.cost <= pic::MATCHED_MAX
                {
                    Badge::Matched
                } else {
                    Badge::Likely
                };
                why.push(format!(
                    "packs fit {} of {} picture stretches",
                    f.filled, f.stretches
                ));
                let count = |n: usize, one: &str, many: &str| {
                    if n == 1 {
                        format!("1 {one}")
                    } else {
                        format!("{n} {many}")
                    }
                };
                if f.crossing > 0 {
                    why.push(count(f.crossing, "pack in dead air", "packs in dead air"));
                }
                if f.breakups > 0 {
                    why.push(count(
                        f.breakups,
                        "signal breakup while armed",
                        "signal breakups while armed",
                    ));
                }
                if f.swaps_at_gap > 0 {
                    why.push(count(
                        f.swaps_at_gap,
                        "battery swap at dead air",
                        "battery swaps at dead air",
                    ));
                }
                if f.swaps_in_stretch > 0 {
                    why.push(count(
                        f.swaps_in_stretch,
                        "battery swap without dead air",
                        "battery swaps without dead air",
                    ));
                }
                if f.replugs > 0 {
                    why.push(count(f.replugs, "battery replugged", "batteries replugged"));
                }
                if f.spread >= 1.0 {
                    why.push(format!(
                        "first pack at {:.0} s (±{:.0} s)",
                        f.offset, f.spread
                    ));
                } else {
                    why.push(format!("first pack at {:.0} s", f.offset));
                }
            }
            None => {
                margin = tun.tolerance_s / 3.0;
                badge = if w.duration - span_s <= 2.0 * tun.tolerance_s {
                    Badge::Matched
                } else {
                    Badge::Likely
                };
                if badge == Badge::Likely {
                    why.push("much unarmed time".into());
                }
            }
        }
        if let Some(k) = skew_s {
            if k.abs() > tun.clock_skew_s {
                badge = Badge::Likely;
            }
            why.push(format!("clip clock {:.0} s off", k.abs()));
        }
        if !clock_ok {
            why.push(match c.fit {
                Some(_) => "radio clock wrong; matched by picture".into(),
                None => "radio clock wrong; matched by pack lengths".into(),
            });
        }
        // Another solution nearly as good, with this clip elsewhere: not sure.
        let alt = solve(clips, segs, &cands, clock_ok, tun, Some(pick.unwrap()));
        if alt.cost - best.cost <= margin {
            badge = Badge::Likely;
            match alt.picks[ci].map(|p| cands[p]) {
                Some(o) => why.push(format!(
                    "the pack at {} fits as well",
                    segs[o.first].start.format("%H:%M:%S")
                )),
                None => why.push("it may be a clip without a log".into()),
            }
        }
        let model = files[s.file].model.clone();
        match model.as_deref() {
            Some(m) => match profile_of_model(m, profiles) {
                Some(p) => why.push(format!("log {m} → profile {}", p.name)),
                None => why.push(format!("log {m} is in no profile")),
            },
            None => why.push("log without a model name".into()),
        }
        out[ci] = Some(Found {
            badge,
            first: c.first,
            segments: n,
            start: s.start,
            span_s,
            skew_s,
            model,
            offset_s: c.fit.map(|f| f.offset),
            reason: why.join("; "),
        });
    }
    out
}

struct Solution {
    cost: f64,
    /// Per clip, the index into the candidate list.
    picks: Vec<Option<usize>>,
}

/// The cheapest assignment: clips in order, each claiming one candidate or none, claims of
/// one file in segment order and never sharing a segment. `ban` excludes one candidate.
fn solve(
    clips: &[Want],
    segs: &[Seg],
    cands: &[Cand],
    clock_ok: bool,
    tun: &Tunables,
    ban: Option<usize>,
) -> Solution {
    // State: the last claimed candidate (None at the start). Value: cost and back-pointer.
    #[derive(Clone)]
    struct St {
        cost: f64,
        last: Option<usize>,
        picks: Vec<Option<usize>>,
    }
    let mut states: Vec<St> = vec![St {
        cost: 0.0,
        last: None,
        picks: Vec::new(),
    }];
    let gap_cost = |p: &Cand, c: &Cand| -> f64 {
        let (Some(a), Some(b)) = (clips[p.clip].clock, clips[c.clip].clock) else {
            return 0.0;
        };
        let (sp, sc) = (segs[p.first], segs[c.first]);
        let comparable = clock_ok || (sp.file == sc.file && sp.session == sc.session);
        if !comparable {
            return 0.0;
        }
        let d_clip = (b - a).num_milliseconds() as f64 / 1000.0;
        let d_log = (sc.start - sp.start).num_milliseconds() as f64 / 1000.0;
        ((d_clip - d_log).abs() / 10.0).min(tun.tolerance_s)
    };
    let after = |p: &Cand, c: &Cand| -> bool {
        let (sp, sc) = (segs[p.last], segs[c.first]);
        let other_day = matches!(
            (clips[p.clip].day, clips[c.clip].day),
            (Some(a), Some(b)) if a != b
        );
        if other_day && sp.file == sc.file && sp.session == sc.session {
            return false;
        }
        sp.file != sc.file || c.first > p.last
    };
    for (ci, w) in clips.iter().enumerate() {
        let mine: Vec<usize> = (0..cands.len())
            .filter(|i| cands[*i].clip == ci && Some(*i) != ban)
            .collect();
        // Best new state per `last`.
        let mut next: std::collections::HashMap<Option<usize>, St> = Default::default();
        let mut offer = |st: St| {
            let e = next.entry(st.last);
            match e {
                std::collections::hash_map::Entry::Occupied(mut o) => {
                    if st.cost < o.get().cost {
                        o.insert(st);
                    }
                }
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert(st);
                }
            }
        };
        for st in &states {
            // Unmatched.
            let mut s = st.clone();
            s.cost += if w.keep.is_empty() {
                w.duration.max(0.0) + tun.tolerance_s
            } else {
                pic::UNMATCHED
            };
            s.picks.push(None);
            offer(s);
            for &i in &mine {
                let c = &cands[i];
                let extra = match st.last {
                    Some(p) if !after(&cands[p], c) => continue,
                    Some(p) => gap_cost(&cands[p], c),
                    None => 0.0,
                };
                // Claims of other files must not reuse a segment either.
                if st.picks.iter().flatten().any(|q| {
                    let q = &cands[*q];
                    q.first <= c.last && c.first <= q.last
                }) {
                    continue;
                }
                let mut s = st.clone();
                s.cost += c.cost + extra;
                s.last = Some(i);
                s.picks.push(Some(i));
                offer(s);
            }
        }
        states = next.into_values().collect();
    }
    let best = states
        .into_iter()
        .min_by(|a, b| a.cost.total_cmp(&b.cost))
        .expect("at least the start state");
    Solution {
        cost: best.cost,
        picks: best.picks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn rows(day: &str, spans: &[(&str, &str)]) -> Vec<LogRow> {
        let mut v = Vec::new();
        for (a, b) in spans {
            let p = |s: &str| {
                NaiveDateTime::parse_from_str(&format!("{day} {s}"), "%Y-%m-%d %H:%M:%S%.f")
                    .unwrap()
            };
            let (mut t, b) = (p(a), p(b));
            while t <= b {
                v.push(LogRow {
                    time: t,
                    sticks: None,
                    roll: None,
                    pitch: None,
                    rx_bat: None,
                    lq: None,
                    rssi: None,
                    model: None,
                });
                t += Duration::milliseconds(500);
            }
        }
        v
    }

    fn file(model: &str, rows: Vec<LogRow>) -> LogFile {
        LogFile {
            model: Some(model.into()),
            rows,
        }
    }

    fn profile(name: &str, system: &str, models: &[&str]) -> Profile {
        Profile {
            name: name.into(),
            video_system: system.into(),
            edgetx_models: models.iter().map(|m| m.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_reset_clock_starts_a_new_session_when_time_goes_back() {
        let mut r = rows("2000-01-01", &[("00:04:00", "00:05:00")]);
        r.extend(rows(
            "2000-01-01",
            &[("00:01:00", "00:03:00"), ("00:03:30", "00:04:30")],
        ));
        let s = segments(&[file("Q", r)], &Tunables::default());
        assert_eq!(s.len(), 3);
        assert_eq!((s[0].session, s[1].session, s[2].session), (0, 1, 1));
        assert_eq!(s[1].secs(), 120.0);
    }

    #[test]
    fn models_filter_by_profile() {
        let ps = [
            profile("Whoop", "Analog", &["AIR65"]),
            profile("Digital", "DJI", &["METEOR75"]),
        ];
        assert_eq!(
            clip_models(None, SourceKind::Dji, &ps),
            Some(vec!["METEOR75".to_string()])
        );
        assert_eq!(
            clip_models(Some("whoop"), SourceKind::Dji, &ps),
            Some(vec!["AIR65".to_string()])
        );
        assert_eq!(clip_models(None, SourceKind::Dji, &[]), None);
    }

    // Picture matching. Packs are given in seconds from midnight of a reset-clock day.

    fn at(secs: f64) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2000, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            + Duration::milliseconds((secs * 1000.0).round() as i64)
    }

    /// Packs `(start, end, volts at start, volts at end)` of one session.
    fn packs(session: usize, p: &[(f64, f64, f64, f64)]) -> Vec<Seg> {
        p.iter()
            .map(|&(a, b, v0, v1)| Seg {
                file: 0,
                a: 0,
                b: 0,
                start: at(a),
                end: at(b),
                session,
                v_start: Some(v0),
                v_end: Some(v1),
            })
            .collect()
    }

    fn picture(duration: f64, keep: &[(f64, f64)]) -> Want {
        Want {
            duration,
            keep: keep.to_vec(),
            ..Default::default()
        }
    }

    fn run(clips: &[Want], segs: &[Seg]) -> Vec<Option<Found>> {
        let files = [LogFile {
            model: None,
            rows: Vec::new(),
        }];
        match_all(clips, segs, &files, &[], false, &Tunables::default())
    }

    /// Three batteries, each a picture stretch with unarmed picture around its pack, the
    /// log starting 20 s into the clip.
    fn three_batteries() -> (Want, Vec<Seg>) {
        let clip = picture(600.0, &[(5.0, 160.0), (200.0, 380.0), (420.0, 600.0)]);
        let segs = packs(
            0,
            &[
                (1000.0, 1120.0, 4.3, 3.5),
                (1215.0, 1345.0, 4.3, 3.5),
                (1430.0, 1550.0, 4.35, 3.5),
            ],
        );
        (clip, segs)
    }

    #[test]
    fn packs_inside_picture_stretches_match_with_their_offset() {
        let (clip, segs) = three_batteries();
        let f = run(&[clip], &segs)[0].clone().expect("a match");
        assert_eq!(f.badge, Badge::Matched, "{}", f.reason);
        assert_eq!((f.first, f.segments), (0, 3));
        let o = f.offset_s.unwrap();
        // Pack 1 may start 5..40 s in, pack 2 at 200..250, pack 3 at 420..450: 5..30.
        assert!((5.0..=30.0).contains(&o), "offset {o}");
        assert!(
            f.reason.contains("packs fit 3 of 3 picture stretches"),
            "{}",
            f.reason
        );
        assert!(
            f.reason.contains("2 battery swaps at dead air"),
            "{}",
            f.reason
        );
    }

    #[test]
    fn a_pack_that_must_cross_dead_air_is_never_matched() {
        // A 150 s pack cannot sit in a 100 s or a 120 s stretch.
        let clip = picture(300.0, &[(0.0, 100.0), (130.0, 250.0)]);
        let segs = packs(0, &[(1000.0, 1150.0, 4.3, 3.5)]);
        let f = run(&[clip], &segs)[0].clone();
        assert!(
            f.as_ref().is_none_or(|f| f.badge != Badge::Matched),
            "{f:?}"
        );
    }

    #[test]
    fn packs_without_a_clip_are_left_alone() {
        // Five batteries; the DVR recorded only the 2nd and 3rd.
        let segs = packs(
            0,
            &[
                (1000.0, 1100.0, 4.3, 3.5),
                (1300.0, 1420.0, 4.3, 3.5),
                (1500.0, 1650.0, 4.3, 3.5),
                (1800.0, 1960.0, 4.3, 3.5),
                (2100.0, 2200.0, 4.3, 3.5),
            ],
        );
        // Picture from just before pack 2 to 20 s after pack 3, dead air between and after.
        let clip = picture(420.0, &[(30.0, 150.0), (170.0, 400.0)]);
        let f = run(&[clip], &segs)[0].clone().expect("a match");
        assert_eq!(f.badge, Badge::Matched, "{}", f.reason);
        assert_eq!((f.first, f.segments), (1, 2));
    }

    #[test]
    fn a_recording_may_start_late_and_stop_early() {
        // The DVR started about 40 s into pack 1 and stopped about 50 s into pack 3.
        let segs = packs(
            0,
            &[
                (1000.0, 1100.0, 4.3, 3.5),
                (1200.0, 1320.0, 4.3, 3.5),
                (1400.0, 1500.0, 4.3, 3.5),
            ],
        );
        let clip = picture(410.0, &[(0.0, 75.0), (150.0, 285.0), (330.0, 410.0)]);
        let f = run(&[clip], &segs)[0].clone().expect("a match");
        assert_eq!((f.first, f.segments), (0, 3), "{}", f.reason);
        assert_eq!(f.badge, Badge::Matched, "{}", f.reason);
        let o = f.offset_s.unwrap();
        assert!((-54.0..=-31.0).contains(&o), "offset {o}");
    }

    #[test]
    fn several_packs_of_one_battery_share_a_stretch() {
        // A crash and a re-arm on battery 1 (it starts where it stopped), then battery 2.
        let segs = packs(
            0,
            &[
                (1000.0, 1050.0, 4.3, 3.9),
                (1080.0, 1140.0, 3.9, 3.5),
                (1200.0, 1330.0, 4.3, 3.5),
            ],
        );
        let clip = picture(400.0, &[(10.0, 170.0), (195.0, 340.0)]);
        let f = run(&[clip], &segs)[0].clone().expect("a match");
        assert_eq!(f.badge, Badge::Matched, "{}", f.reason);
        assert_eq!(f.segments, 3);
        assert!(f.reason.contains("packs fit 2 of 2"), "{}", f.reason);
        assert!(
            f.reason.contains("1 battery swap at dead air"),
            "{}",
            f.reason
        );
    }

    #[test]
    fn a_battery_swap_needs_dead_air() {
        let segs = packs(0, &[(1000.0, 1050.0, 4.3, 3.5), (1080.0, 1140.0, 4.3, 3.5)]);
        let clip = picture(300.0, &[(0.0, 300.0)]);
        // The only stretch must hold both batteries: no dead air at the swap.
        let (cost, fit) = picture_fit(&segs, 20.0, &clip.keep, 300.0);
        assert_eq!(fit.swaps_in_stretch, 1);
        assert!(cost >= pic::SWAP_IN_STRETCH);
        let f = run(&[clip], &segs)[0].clone();
        assert!(
            f.as_ref().is_none_or(|f| f.badge != Badge::Matched),
            "{f:?}"
        );
    }

    #[test]
    fn voltage_steps_scale_with_the_cell_count() {
        // 1S: 4.3 V rested. 4S: 16.8 V.
        assert_eq!(cells(&packs(0, &[(0.0, 60.0, 4.3, 3.5)])), 1.0);
        assert_eq!(cells(&packs(0, &[(0.0, 60.0, 16.6, 14.0)])), 4.0);
        assert_eq!(cells(&packs(0, &[(0.0, 60.0, 8.3, 7.0)])), 2.0);
        assert_eq!(cells(&packs(0, &[(0.0, 60.0, 24.9, 21.0)])), 6.0);
        // A 4S pack that recovers 0.3 V at rest after an unplug and replug: one battery,
        // not a swap (on 1S thresholds it would read as a new battery).
        let segs = packs(
            0,
            &[(1000.0, 1100.0, 16.6, 14.6), (1200.0, 1300.0, 14.9, 13.9)],
        );
        let keep = [(0.0, 150.0), (180.0, 350.0)];
        let (_, fit) = picture_fit(&segs, 20.0, &keep, 350.0);
        assert_eq!((fit.swaps_at_gap, fit.replugs), (0, 1), "{fit:?}");
        // A fresh 4S pack (2.6 V up) is a swap.
        let segs = packs(
            0,
            &[(1000.0, 1100.0, 16.6, 14.0), (1200.0, 1300.0, 16.6, 14.0)],
        );
        let (_, fit) = picture_fit(&segs, 20.0, &keep, 350.0);
        assert_eq!((fit.swaps_at_gap, fit.replugs), (1, 0), "{fit:?}");
    }

    #[test]
    fn short_dead_air_while_armed_is_signal_breakup() {
        // One 200 s pack; the picture breaks up for 6 s in the middle.
        let segs = packs(0, &[(1000.0, 1200.0, 4.3, 3.5)]);
        let clip = picture(260.0, &[(0.0, 120.0), (126.0, 260.0)]);
        let (cost, fit) = picture_fit(&segs, 20.0, &clip.keep, 260.0);
        assert_eq!((fit.breakups, fit.crossing), (1, 0), "{fit:?}");
        assert_eq!((fit.filled, fit.stretches), (1, 1), "{fit:?}");
        assert!(cost < pic::CROSS, "{cost}");
        let f = run(&[clip], &segs)[0].clone().expect("a match");
        assert_eq!(f.badge, Badge::Matched, "{}", f.reason);
        assert!(
            f.reason.contains("1 signal breakup while armed"),
            "{}",
            f.reason
        );
        // 30 s of dead air inside a pack is not breakup: still a crossing.
        let (_, fit) = picture_fit(&segs, 20.0, &[(0.0, 100.0), (130.0, 260.0)], 260.0);
        assert_eq!((fit.breakups, fit.crossing), (0, 1), "{fit:?}");
    }

    #[test]
    fn a_long_stretch_without_a_pack_is_suspicious() {
        let segs = packs(0, &[(1000.0, 1100.0, 4.3, 3.5)]);
        let clip = picture(450.0, &[(0.0, 120.0), (150.0, 450.0)]);
        let (cost, fit) = picture_fit(&segs, 10.0, &clip.keep, 450.0);
        assert_eq!((fit.filled, fit.crossing), (1, 0));
        assert_eq!(cost, pic::EMPTY_MAX);
        let f = run(&[clip], &segs)[0]
            .clone()
            .expect("a fit, if a weak one");
        assert_eq!(f.badge, Badge::Likely, "{}", f.reason);
        // A short empty stretch (a battery in, never armed) is free.
        let (cost, _) = picture_fit(&segs, 10.0, &[(0.0, 120.0), (150.0, 175.0)], 175.0);
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn clips_of_other_days_never_share_a_session() {
        let (clip, segs) = three_batteries();
        let mut a = clip.clone();
        a.duration = 170.0;
        a.keep = vec![(5.0, 160.0)];
        a.day = NaiveDate::from_ymd_opt(2026, 9, 25);
        let mut b = picture(180.0, &[(0.0, 180.0)]);
        b.day = NaiveDate::from_ymd_opt(2026, 9, 26);
        let f = run(&[a, b], &segs);
        // Each fits a pack of the one session alone; together they may not.
        assert_eq!(f.iter().flatten().count(), 1, "{f:?}");
    }

    #[test]
    fn a_split_recording_matches_as_one_timeline() {
        // One recording, split at 600 s; pack 3 runs across the split.
        let segs = packs(
            0,
            &[
                (1000.0, 1150.0, 4.3, 3.5),
                (1250.0, 1400.0, 4.3, 3.5),
                (1500.0, 1650.0, 4.3, 3.5),
                (1750.0, 1850.0, 4.3, 3.5),
            ],
        );
        // Log offset 30: packs at 30-180, 280-430, 530-680, 780-880 of the recording.
        let a = picture(600.0, &[(10.0, 200.0), (260.0, 450.0), (510.0, 600.0)]);
        let mut b = picture(320.0, &[(0.0, 100.0), (160.0, 300.0)]);
        b.follows = true;
        let f = run(&[a, b], &segs);
        let (fa, fb) = (f[0].clone().unwrap(), f[1].clone().unwrap());
        assert_eq!((fa.first, fa.segments), (0, 3), "{}", fa.reason);
        assert_eq!((fb.first, fb.segments), (2, 2), "{}", fb.reason);
        assert_eq!(fa.badge, Badge::Matched, "{}", fa.reason);
        let (oa, ob) = (fa.offset_s.unwrap(), fb.offset_s.unwrap());
        assert!((oa - ob - 600.0 + 500.0).abs() < 0.01, "{oa} {ob}");
        assert!(fb.reason.contains("file 2 of 2"), "{}", fb.reason);
    }

    /// Pack spans and voltages from a real reset-clock log (four power-on sessions, one
    /// 1S whoop), and the keep ranges of four real analog recordings: two files of one
    /// recording the DVR split, then two recordings of later days. Nothing in it fits
    /// cleanly, and the matcher must say so.
    #[test]
    fn real_reset_clock_log_against_four_recordings() {
        let mut segs = packs(0, &[(262.45, 276.95, 4.1, 3.9), (313.49, 345.19, 4.0, 3.8)]);
        segs.extend(packs(
            1,
            &[(81.53, 95.53, 3.9, 3.7), (173.17, 181.17, 3.8, 3.8)],
        ));
        segs.extend(packs(
            2,
            &[
                (41.55, 226.34, 4.3, 3.4),
                (287.11, 456.32, 4.3, 3.5),
                (514.74, 600.24, 4.3, 3.6),
                (639.55, 671.55, 4.3, 4.0),
                (751.70, 857.55, 4.0, 3.5),
                (998.17, 1135.67, 4.2, 3.5),
                (1188.45, 1361.45, 4.3, 3.5),
            ],
        ));
        segs.extend(packs(
            3,
            &[
                (66.33, 193.82, 4.3, 3.4),
                (237.11, 338.11, 4.3, 3.4),
                (495.20, 561.57, 4.3, 3.9),
                (569.11, 616.11, 3.9, 3.5),
                (651.77, 751.13, 4.3, 3.5),
                (846.04, 934.04, 4.3, 3.6),
            ],
        ));
        let day = |d| NaiveDate::from_ymd_opt(2026, 9, d);
        let mut first = picture(600.0, &[(44.5, 128.5), (237.5, 396.5), (472.5, 600.0)]);
        first.day = day(25);
        let mut second = picture(
            450.5,
            &[(0.0, 44.5), (65.0, 108.0), (113.0, 226.0), (254.0, 450.4)],
        );
        second.day = day(25);
        second.follows = true;
        let mut chase = picture(
            600.0,
            &[
                (14.5, 177.5),
                (207.0, 373.0),
                (400.0, 529.0),
                (555.5, 600.0),
            ],
        );
        chase.day = day(26);
        let mut farm = picture(
            972.1,
            &[
                (0.0, 117.0),
                (131.5, 293.0),
                (305.5, 505.0),
                (522.0, 742.0),
                (763.0, 972.1),
            ],
        );
        farm.day = day(28);
        let f = run(&[first, second.clone(), chase, farm], &segs);
        // The split recording fits no session: its first stretch is 84 s, and every pack
        // of the one long run is longer than that.
        assert!(f[0].is_none() && f[1].is_none(), "{f:?}");
        // The chase fits session 3's middle packs (one battery re-armed); only "likely",
        // since the farm clip could take them instead.
        let c = f[2].clone().expect("the chase");
        assert_eq!(
            (c.first, c.segments, c.badge),
            (13, 4, Badge::Likely),
            "{}",
            c.reason
        );
        assert!(c.reason.contains("packs fit 3 of 4"), "{}", c.reason);
        // The farm clip leaves a 209 s stretch empty with any session: no match.
        assert!(f[3].is_none(), "{:?}", f[3]);
        // Alone, the second file would fit session 3 by chance; joined, it does not.
        second.follows = false;
        let alone = run(&[second], &segs);
        assert!(alone[0].is_some());
        assert!(f
            .iter()
            .all(|f| f.as_ref().is_none_or(|f| f.badge != Badge::Matched)));
    }
}
