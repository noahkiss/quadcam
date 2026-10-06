//! The trim model shared by the import session and the library: cut ranges on one clip,
//! checked the same way wherever they are edited, and the rule for cuts that were already
//! written as files. The GUI's one trim component drives both through `Core`.

use crate::moments::{Moment, MomentKind, Span};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A cut shorter than this is refused.
pub const MIN_CUT_S: f64 = 0.5;
/// Most cuts one clip may have.
pub const MAX_CUTS: usize = 20;
/// Seconds added before and after a moment when it becomes in and out points.
pub const MOMENT_PAD_S: f64 = 1.0;

/// Cut ranges rounded to milliseconds, clamped to the clip, sorted. Refuses empty, inverted,
/// too short or too many ranges. `what` names the clip in errors.
pub fn check_cuts(what: &str, cuts: &[Span], duration: f64) -> Result<Vec<Span>> {
    if cuts.len() > MAX_CUTS {
        bail!("{what}: at most {MAX_CUTS} cuts");
    }
    let r = |x: f64| (x * 1000.0).round() / 1000.0;
    let mut out = Vec::with_capacity(cuts.len());
    for c in cuts {
        if !c.start.is_finite() || !c.end.is_finite() {
            bail!("{what}: cut times must be numbers");
        }
        let span = Span {
            start: r(c.start.max(0.0)),
            end: r(if duration > 0.0 {
                c.end.min(duration)
            } else {
                c.end
            }),
        };
        if span.secs() < MIN_CUT_S {
            bail!(
                "{what}: cut {:.2}-{:.2} s is shorter than {MIN_CUT_S} s or outside the {duration:.1} s clip",
                c.start,
                c.end
            );
        }
        out.push(span);
    }
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(out)
}

/// In and out points around a moment: dead air exactly, anything else padded.
pub fn frame_moment(m: &Moment, duration: f64) -> Span {
    let pad = if m.kind == MomentKind::DeadAir {
        0.0
    } else {
        MOMENT_PAD_S
    };
    let r = |x: f64| (x * 10.0).round() / 10.0;
    Span {
        start: r((m.start - pad).max(0.0)),
        end: r(if duration > 0.0 {
            (m.end + pad).min(duration)
        } else {
            m.end + pad
        }),
    }
}

/// Seconds added before arm and after disarm when a pack becomes a cut, at most half the
/// gap to the next pack.
pub const FLIGHT_PAD_S: f64 = 2.0;
/// One pack is worth a cut only when it leaves out at least this much of the clip...
pub const SPLIT_MIN_LEFT_S: f64 = 10.0;
/// ...and at least this share of it.
pub const SPLIT_MIN_LEFT_SHARE: f64 = 0.1;

/// "Split by flight": one cut per radio-log pack (`packs`, armed ranges in clip seconds),
/// padded by `FLIGHT_PAD_S` and clamped to the clip. A pack outside the clip, or shorter
/// than `MIN_CUT_S` inside it, gives no cut. Refuses when there is nothing to split: no
/// packs in the clip, or one pack that covers nearly all of it.
pub fn flight_cuts(what: &str, packs: &[Span], duration: f64) -> Result<Vec<Span>> {
    if packs.is_empty() {
        bail!("{what}: nothing to split; the clip has no radio-log packs (match the logs first)");
    }
    let r = |x: f64| (x * 10.0).round() / 10.0;
    let mut sorted: Vec<Span> = packs.to_vec();
    sorted.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut out = Vec::new();
    for (i, p) in sorted.iter().enumerate() {
        let before = i
            .checked_sub(1)
            .map(|j| ((p.start - sorted[j].end) / 2.0).max(0.0))
            .unwrap_or(FLIGHT_PAD_S);
        let after = sorted
            .get(i + 1)
            .map(|n| ((n.start - p.end) / 2.0).max(0.0))
            .unwrap_or(FLIGHT_PAD_S);
        let span = Span {
            start: r((p.start - FLIGHT_PAD_S.min(before)).max(0.0)),
            end: r((p.end + FLIGHT_PAD_S.min(after)).min(duration)),
        };
        if span.secs() >= MIN_CUT_S {
            out.push(span);
        }
    }
    if out.is_empty() {
        bail!("{what}: nothing to split; no radio-log pack falls inside the clip");
    }
    if let [one] = &out[..] {
        let left = duration - one.secs();
        if left < SPLIT_MIN_LEFT_S.max(duration * SPLIT_MIN_LEFT_SHARE) {
            bail!("{what}: nothing to split; its one pack covers nearly the whole clip");
        }
    }
    if out.len() > MAX_CUTS {
        bail!("{what}: {} packs; at most {MAX_CUTS} cuts", out.len());
    }
    Ok(out)
}

/// `cuts` with `add` appended, leaving out ranges it already has.
pub fn with_cuts(cuts: &[Span], add: &[Span]) -> Vec<Span> {
    let mut out = cuts.to_vec();
    for a in add {
        if !out.iter().any(|c| same_span(c, a)) {
            out.push(*a);
        }
    }
    out
}

pub fn same_span(a: &Span, b: &Span) -> bool {
    (a.start - b.start).abs() < 0.001 && (a.end - b.end).abs() < 0.001
}

/// A cut that was written as its own file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct ExportedCut {
    pub span: Span,
    pub path: PathBuf,
}

/// The exported cuts a new cut list drops.
pub fn removed_exported(exported: &[ExportedCut], new: &[Span]) -> Vec<ExportedCut> {
    exported
        .iter()
        .filter(|e| !new.iter().any(|n| same_span(n, &e.span)))
        .cloned()
        .collect()
}

/// What happens to the file of an exported cut that is removed from the list. With no
/// decision, the change is not applied and the caller is asked.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum RemovedCuts {
    /// The file stays; it becomes a clip of its own in the library.
    Keep,
    /// The file goes to the Trash.
    Trash,
}

/// The answer to a cut change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CutChange {
    /// The new list is saved. `kept` and `trashed` list the files of removed cuts.
    Applied {
        cuts: Vec<Span>,
        #[serde(default)]
        kept: Vec<PathBuf>,
        #[serde(default)]
        trashed: Vec<PathBuf>,
    },
    /// Nothing changed: these exported files would lose their cut. Ask, then call again
    /// with a `RemovedCuts` decision.
    Confirm { files: Vec<PathBuf> },
}

/// Decides a cut change: the checked list, and the exported files that need a decision.
pub fn plan_change(
    what: &str,
    exported: &[ExportedCut],
    new: &[Span],
    duration: f64,
    decision: Option<RemovedCuts>,
) -> Result<(Vec<Span>, Vec<ExportedCut>, Option<CutChange>)> {
    let cuts = check_cuts(what, new, duration)?;
    let removed = removed_exported(exported, &cuts);
    if !removed.is_empty() && decision.is_none() {
        let files = removed.iter().map(|r| r.path.clone()).collect();
        return Ok((cuts, removed, Some(CutChange::Confirm { files })));
    }
    Ok((cuts, removed, None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::moments::Source;

    fn span(a: f64, b: f64) -> Span {
        Span { start: a, end: b }
    }

    #[test]
    fn cuts_are_checked_clamped_and_sorted() {
        let c = check_cuts("clip 0", &[span(20.0, 30.0), span(-1.0, 4.0)], 25.0).unwrap();
        assert_eq!(c, vec![span(0.0, 4.0), span(20.0, 25.0)]);
        assert!(check_cuts("clip 0", &[span(5.0, 5.2)], 25.0).is_err());
        assert!(check_cuts("clip 0", &[span(30.0, 40.0)], 25.0).is_err());
        assert!(check_cuts("clip 0", &[span(f64::NAN, 4.0)], 25.0).is_err());
        let many: Vec<Span> = (0..21).map(|i| span(i as f64, i as f64 + 0.6)).collect();
        assert!(check_cuts("clip 0", &many, 100.0).is_err());
    }

    #[test]
    fn moments_frame_with_padding_but_dead_air_exactly() {
        let m = Moment {
            kind: MomentKind::Flip,
            start: 0.4,
            end: 1.0,
            score: 0.9,
            source: Source::RadioLog,
            detail: String::new(),
        };
        assert_eq!(frame_moment(&m, 10.0), span(0.0, 2.0));
        let d = Moment {
            kind: MomentKind::DeadAir,
            start: 3.0,
            end: 9.5,
            ..m.clone()
        };
        assert_eq!(frame_moment(&d, 9.0), span(3.0, 9.0));
    }

    #[test]
    fn flights_become_padded_cuts_clamped_to_the_clip() {
        // Three packs; the first starts 1 s in, the last runs past the 300 s clip.
        let packs = [span(1.0, 80.0), span(83.0, 150.0), span(200.0, 320.0)];
        let c = flight_cuts("clip 0", &packs, 300.0).unwrap();
        assert_eq!(
            c,
            vec![
                // 1 s of lead-in is all there is; the 3 s gap gives 1.5 s each side.
                span(0.0, 81.5),
                span(81.5, 152.0),
                span(198.0, 300.0),
            ]
        );
        // Unsorted packs and a pack wholly outside the clip.
        let c = flight_cuts(
            "clip 0",
            &[span(50.0, 90.0), span(10.0, 30.0), span(400.0, 500.0)],
            120.0,
        )
        .unwrap();
        assert_eq!(c, vec![span(8.0, 32.0), span(48.0, 92.0)]);
        // A log offset that puts a pack before the clip start: only its tail stays.
        let c = flight_cuts("clip 0", &[span(-20.0, 40.0), span(60.0, 100.0)], 120.0).unwrap();
        assert_eq!(c, vec![span(0.0, 42.0), span(58.0, 102.0)]);
    }

    #[test]
    fn nothing_to_split() {
        let err = |packs: &[Span], d: f64| flight_cuts("clip 0", packs, d).unwrap_err().to_string();
        assert!(err(&[], 100.0).contains("no radio-log packs"));
        assert!(err(&[span(200.0, 300.0)], 100.0).contains("no radio-log pack falls inside"));
        // One pack that fills the clip: nothing to split.
        assert!(err(&[span(1.0, 95.0)], 100.0).contains("covers nearly the whole clip"));
        // 600 s clip, one 520 s pack: 76 s left out (more than 10 s and 10 %), so it cuts.
        assert_eq!(
            flight_cuts("clip 0", &[span(30.0, 550.0)], 600.0).unwrap(),
            vec![span(28.0, 552.0)]
        );
        // 600 s clip, one 560 s pack: 36 s left out is under 10 %: nothing to split.
        assert!(err(&[span(20.0, 580.0)], 600.0).contains("covers nearly"));
        // Two packs always split, even when together they cover the clip.
        assert_eq!(
            flight_cuts("clip 0", &[span(0.0, 50.0), span(52.0, 100.0)], 100.0)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn with_cuts_skips_ranges_it_has() {
        let c = with_cuts(&[span(1.0, 3.0)], &[span(1.0, 3.0), span(5.0, 9.0)]);
        assert_eq!(c, vec![span(1.0, 3.0), span(5.0, 9.0)]);
    }

    #[test]
    fn removing_an_exported_cut_asks_first() {
        let exported = vec![
            ExportedCut {
                span: span(1.0, 3.0),
                path: "a_cut1.mp4".into(),
            },
            ExportedCut {
                span: span(5.0, 8.0),
                path: "a_cut2.mp4".into(),
            },
        ];
        // Adding a cut keeps both exported ones: no question.
        let (cuts, removed, ask) = plan_change(
            "a",
            &exported,
            &[span(1.0, 3.0), span(5.0, 8.0), span(9.0, 10.0)],
            20.0,
            None,
        )
        .unwrap();
        assert_eq!(cuts.len(), 3);
        assert!(removed.is_empty() && ask.is_none());
        // Dropping cut 2 without a decision asks and names its file.
        let (_, removed, ask) = plan_change("a", &exported, &[span(1.0, 3.0)], 20.0, None).unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(
            ask,
            Some(CutChange::Confirm {
                files: vec!["a_cut2.mp4".into()]
            })
        );
        // With a decision it goes ahead.
        let (_, removed, ask) = plan_change(
            "a",
            &exported,
            &[span(1.0, 3.0)],
            20.0,
            Some(RemovedCuts::Keep),
        )
        .unwrap();
        assert_eq!(removed.len(), 1);
        assert!(ask.is_none());
    }
}
