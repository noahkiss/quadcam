//! `Core`'s cuts: setting a clip's cut list in the session or the library, the decision on
//! exported cuts a new list drops, and writing a library clip's new cuts.

use super::library::rel_to;
use super::Core;
use crate::api::{Event, LibraryTask, Task};
use crate::library::{self as lib, LibCut};
use crate::media;
use crate::moments::Span;
use crate::trim::{self, CutChange, ExportedCut, RemovedCuts};
use anyhow::{anyhow, bail, Context, Result};
use std::path::PathBuf;

impl Core {
    /// Sets a library clip's cut list. Exported cuts that the list drops need `decision`;
    /// without it nothing changes and the answer names their files. New ranges wait for
    /// `library_export_cuts`.
    pub fn library_set_cuts(
        &self,
        id: &str,
        cuts: &[Span],
        decision: Option<RemovedCuts>,
    ) -> Result<CutChange> {
        let (root, c) = self.clip(id)?;
        let exported = crate::cuts::library_exported(&root, &c);
        let (checked, removed, ask) =
            trim::plan_change(&c.display_name(), &exported, cuts, c.duration, decision)?;
        if let Some(ask) = ask {
            return Ok(ask);
        }
        let (kept, trashed) = self.drop_exported(&removed, decision)?;
        self.with_index(|_, ix| {
            let e = ix.get_mut(id).context("clip left the library")?;
            e.cuts
                .retain(|x| !removed.iter().any(|r| r.path.ends_with(&x.path)));
            e.pending_cuts = checked
                .iter()
                .filter(|n| {
                    !e.cuts.iter().any(|x| {
                        trim::same_span(
                            n,
                            &Span {
                                start: x.start,
                                end: x.end,
                            },
                        )
                    })
                })
                .copied()
                .collect();
            Ok(((), true))
        })?;
        Ok(CutChange::Applied {
            cuts: checked,
            kept,
            trashed,
        })
    }

    /// "Split by flight" for a library clip: one cut per radio-log pack, added to its cut
    /// list. The new ranges wait for `library_export_cuts`, as hand-made cuts do.
    pub fn library_split_by_flight(&self, id: &str) -> Result<CutChange> {
        let (root, c) = self.clip(id)?;
        let packs = c
            .stats
            .as_ref()
            .map(|f| f.pack_spans.clone())
            .unwrap_or_default();
        let add = trim::flight_cuts(&c.display_name(), &packs, c.duration)?;
        let mut have: Vec<Span> = crate::cuts::library_exported(&root, &c)
            .iter()
            .map(|e| e.span)
            .collect();
        have = trim::with_cuts(&have, &c.pending_cuts);
        self.library_set_cuts(id, &trim::with_cuts(&have, &add), None)
    }

    /// "Split by flight" for a session clip: one cut per radio-log pack, added to its cut
    /// list.
    pub fn session_split_by_flight(&self, id: usize) -> Result<CutChange> {
        let patch = crate::session::PlanPatch {
            id,
            split_by_flight: Some(true),
            ..Default::default()
        };
        self.patch_inner(&[patch], crate::session::Editor::User)?;
        let s = self.current()?;
        let p = s
            .plans
            .iter()
            .find(|p| p.id == id)
            .context("no such clip")?;
        Ok(CutChange::Applied {
            cuts: p.cuts.clone(),
            kept: Vec::new(),
            trashed: Vec::new(),
        })
    }

    /// Keeps (as clips of their own) or trashes the files of removed exported cuts.
    pub(crate) fn drop_exported(
        &self,
        removed: &[ExportedCut],
        decision: Option<RemovedCuts>,
    ) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
        let mut kept = Vec::new();
        let mut trashed = Vec::new();
        for r in removed {
            if !r.path.is_file() {
                continue;
            }
            match decision {
                Some(RemovedCuts::Keep) => {
                    lib::write_keys(&r.path, &[(lib::KEY_DETACHED, "1".into())])?;
                    kept.push(r.path.clone());
                }
                Some(RemovedCuts::Trash) => {
                    self.trash.trash(&r.path)?;
                    trashed.push(r.path.clone());
                }
                None => bail!("{} needs a decision: keep or trash", r.path.display()),
            }
        }
        if !kept.is_empty() {
            let _ = self.with_index(|root, ix| {
                for k in &kept {
                    if let Ok(lib::Found::Clip(c)) = lib::read_file(root, &rel_to(root, k)) {
                        ix.upsert(*c);
                    }
                }
                Ok(((), true))
            });
        }
        Ok((kept, trashed))
    }

    /// Writes a library clip's unsaved cuts as files next to it, from its kept original when
    /// there is one, else from the clip itself.
    pub fn library_export_cuts(&self, id: &str) -> Result<Vec<LibCut>> {
        let tools = media::find_tools()?;
        let encoder = self.defaults().encoder;
        let _b = self.claim()?;
        let (root, c) = self.clip(id)?;
        if c.pending_cuts.is_empty() {
            return Ok(Vec::new());
        }
        let file = root.join(&c.path);
        let src = c
            .original
            .as_ref()
            .map(|o| root.join(o))
            .filter(|o| o.is_file())
            .unwrap_or_else(|| file.clone());
        let probe = media::probe(&tools, &src).context("reading the clip")?;
        let items = crate::qtmeta::read(&file)?;
        let stem = file
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let ext = file
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let format = crate::cuts::format_for_ext(&ext);
        let created = crate::qtmeta::get(&items, "com.apple.quicktime.creationdate")
            .and_then(|v| chrono::DateTime::parse_from_str(v, "%Y-%m-%dT%H:%M:%S%z").ok())
            .map(|t| t.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);
        let dir = file.parent().context("clip has no folder")?.to_path_buf();
        let next_n = c
            .cuts
            .iter()
            .filter_map(|x| {
                let s = x.path.file_stem()?.to_string_lossy().to_string();
                s.rsplit_once("_cut")?
                    .1
                    .split('-')
                    .next()?
                    .parse::<u32>()
                    .ok()
            })
            .max()
            .unwrap_or(0)
            + 1;
        let mut planner = crate::naming::NamePlanner::new();
        let mut made = Vec::new();
        let mut errors = Vec::new();
        let total = c.pending_cuts.len();
        for (i, span) in c.pending_cuts.iter().enumerate() {
            let n = next_n + i as u32;
            self.hooks.event(Event::LibraryTask(LibraryTask {
                task: Task::Cuts,
                done: i,
                total,
            }));
            let dst = planner.claim(&dir, &format!("{stem}_cut{n}"), &ext);
            let start_time = created + chrono::Duration::milliseconds((span.start * 1000.0) as i64);
            let meta = media::Meta {
                title: c.title.clone(),
                comment: c.note.clone(),
                creation_time: start_time,
                date: c.date.format("%Y-%m-%d").to_string(),
                description: format!(
                    "{}; cut {:.1}-{:.1} s",
                    c.display_name(),
                    span.start,
                    span.end
                ),
            };
            let mut qt = items.clone();
            crate::qtmeta::set(
                &mut qt,
                "com.apple.quicktime.creationdate",
                &crate::metadata::creation_date(start_time),
            );
            crate::qtmeta::set(
                &mut qt,
                "com.apple.quicktime.description",
                &meta.description,
            );
            crate::qtmeta::set(
                &mut qt,
                lib::KEY_CUT,
                &format!("{:.3}-{:.3}", span.start, span.end),
            );
            for k in [
                lib::KEY_RATING,
                lib::KEY_FLAG,
                lib::KEY_PHOTOS,
                lib::KEY_DETACHED,
            ] {
                crate::qtmeta::set(&mut qt, k, "");
            }
            let cut = crate::cuts::Cut {
                src: &src,
                probe: &probe,
                span: *span,
                format,
                encoder,
                meta: &meta,
                qt: &qt,
            };
            match crate::cuts::write_cut(&tools, &cut, &dst) {
                Ok(size) => made.push(LibCut {
                    path: rel_to(&root, &dst),
                    start: span.start,
                    end: span.end,
                    size,
                }),
                Err(e) => errors.push(format!("cut {:.1}-{:.1} s: {e:#}", span.start, span.end)),
            }
        }
        self.hooks.event(Event::LibraryTask(LibraryTask {
            task: Task::Cuts,
            done: total,
            total,
        }));
        self.with_index(|_, ix| {
            let e = ix.get_mut(id).context("clip left the library")?;
            for m in &made {
                let s = Span {
                    start: m.start,
                    end: m.end,
                };
                e.pending_cuts.retain(|p| !trim::same_span(p, &s));
                e.cuts.push(m.clone());
            }
            e.cuts.sort_by(|a, b| a.start.total_cmp(&b.start));
            Ok(((), !made.is_empty()))
        })?;
        if !errors.is_empty() {
            bail!("{}", errors.join("; "));
        }
        Ok(made)
    }

    /// Sets a session clip's cuts from the GUI's trim component. Exported cuts the list
    /// drops need `decision`, as in the library.
    pub fn set_session_cuts(
        &self,
        id: usize,
        cuts: &[Span],
        decision: Option<RemovedCuts>,
    ) -> Result<CutChange> {
        let patch = crate::session::PlanPatch {
            id,
            cuts: Some(cuts.to_vec()),
            removed_cuts: decision,
            ..Default::default()
        };
        match self.patch_cuts_check(&patch)? {
            Some(ask) => Ok(ask),
            None => {
                let (kept, trashed) = self.patch_inner(&[patch], crate::session::Editor::User)?;
                let s = self.current()?;
                let p = s
                    .plans
                    .iter()
                    .find(|p| p.id == id)
                    .context("no such clip")?;
                Ok(CutChange::Applied {
                    cuts: p.cuts.clone(),
                    kept,
                    trashed,
                })
            }
        }
    }

    /// The exported cut files a session patch would drop, if it has no decision for them.
    pub(crate) fn patch_cuts_check(
        &self,
        patch: &crate::session::PlanPatch,
    ) -> Result<Option<CutChange>> {
        let Some(cuts) = &patch.cuts else {
            return Ok(None);
        };
        let s = self.current()?;
        let duration = s
            .clips
            .iter()
            .find(|c| c.id == patch.id)
            .map(|c| c.duration)
            .ok_or_else(|| anyhow!("no clip with id {}", patch.id))?;
        let exported = crate::cuts::session_exported(&s, patch.id);
        let (_, _, ask) = trim::plan_change(
            &format!("clip {}", patch.id),
            &exported,
            cuts,
            duration,
            patch.removed_cuts,
        )?;
        Ok(ask)
    }
}
