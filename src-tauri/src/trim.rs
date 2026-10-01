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

pub fn same_span(a: &Span, b: &Span) -> bool {
    (a.start - b.start).abs() < 0.001 && (a.end - b.end).abs() < 0.001
}

/// A cut that was written as its own file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RemovedCuts {
    /// The file stays; it becomes a clip of its own in the library.
    Keep,
    /// The file goes to the Trash.
    Trash,
}

/// The answer to a cut change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
