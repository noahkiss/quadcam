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
}

/// Matches clips (in recording order) to segments (file order; time order within a
/// session). `clock_ok`: the log's clock is believable, so a clip clock may be compared to
/// it directly.
pub fn match_all(
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
        let skew_s = match (clock_ok, w.clock) {
            (true, Some(clock)) => Some((clock - s.start).num_milliseconds() as f64 / 1000.0),
            _ => None,
        };
        let mut badge = if w.duration - span_s <= 2.0 * tun.tolerance_s {
            Badge::Matched
        } else {
            Badge::Likely
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
        if badge == Badge::Likely {
            why.push("much unarmed time".into());
        }
        if let Some(k) = skew_s {
            if k.abs() > tun.clock_skew_s {
                badge = Badge::Likely;
                why.push(format!("clip clock {:.0} s off", k.abs()));
            } else {
                why.push(format!("clip clock {:.0} s off", k.abs()));
            }
        }
        if !clock_ok {
            why.push("radio clock wrong; matched by pack lengths".into());
        }
        // Another solution nearly as good, with this clip elsewhere: not sure.
        let alt = solve(clips, segs, &cands, clock_ok, tun, Some(pick.unwrap()));
        if alt.cost - best.cost <= tun.tolerance_s / 3.0 {
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
            s.cost += w.duration.max(0.0) + tun.tolerance_s;
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
}
