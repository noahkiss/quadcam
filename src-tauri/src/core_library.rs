//! `Core`'s library half: listing, rating, editing, cuts, Trash and Photos for the clips
//! in the library folder. Every surface reaches these through `Core` (and `dispatch`).

use super::Core;
use crate::library::{self as lib, Filter, Flag, Index, LibClip, LibCut};
use crate::media::{self, Format};
use crate::moments::Span;
use crate::photos::{self, ShareReport};
use crate::pipeline::Outcome;
use crate::trim::{self, CutChange, ExportedCut, RemovedCuts};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A clip as the front ends see it: the index entry plus absolute paths.
#[derive(Debug, Clone, Serialize)]
pub struct LibItem {
    #[serde(flatten)]
    pub clip: LibClip,
    pub name: String,
    pub file: PathBuf,
    /// The hover-scrub strip, once made.
    pub strip: Option<PathBuf>,
    pub last_import: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LibraryView {
    pub root: PathBuf,
    pub exists: bool,
    /// Media files in the folder that the index does not list (an older export to adopt).
    pub unindexed: usize,
    pub last_import: Option<String>,
    pub totals: lib::Totals,
    pub groups: HashMap<&'static str, Vec<(String, usize)>>,
    pub layout: lib::Layout,
    pub place_folders: bool,
    pub clips: Vec<LibItem>,
}

/// Details to change on a library clip. Missing fields stay as they are.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LibEdit {
    pub note: Option<String>,
    pub keywords: Option<Vec<String>>,
    pub author: Option<String>,
    /// A saved place's name; empty removes the location.
    pub place: Option<String>,
    pub location: Option<crate::metadata::Location>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RebuildReport {
    pub clips: usize,
    pub cuts: usize,
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardStatus {
    pub mount: PathBuf,
    pub clips: usize,
    /// Clips whose content is not in the library yet.
    pub new: usize,
    pub free: Option<u64>,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrashReport {
    pub trashed: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
}

/// The index in memory, and the index file's mtime when it was read.
pub(crate) struct Loaded {
    root: PathBuf,
    index: Index,
    mtime: Option<std::time::SystemTime>,
}

fn rel_to(root: &Path, p: &Path) -> PathBuf {
    p.strip_prefix(root).unwrap_or(p).to_path_buf()
}

impl Core {
    /// The library folder: the output folder setting.
    pub fn library_root(&self) -> Result<PathBuf> {
        self.defaults()
            .output_dir
            .context("No library folder set. Choose one in Settings.")
    }

    fn strip_path(&self, id: &str) -> PathBuf {
        self.cache
            .join("library")
            .join(format!("{}.jpg", lib::id_file(id)))
    }

    /// Runs `f` on the loaded index (loading it first, and again when another process
    /// changed the file), then saves it when `f` says it changed. A missing index starts
    /// empty.
    fn with_index<T>(&self, f: impl FnOnce(&Path, &mut Index) -> Result<(T, bool)>) -> Result<T> {
        let root = self.library_root()?;
        let mtime = |r: &Path| {
            lib::index_path(r)
                .metadata()
                .and_then(|m| m.modified())
                .ok()
        };
        let mut guard = self.library.lock().unwrap();
        let on_disk = mtime(&root);
        if guard
            .as_ref()
            .is_none_or(|l| l.root != root || l.mtime != on_disk)
        {
            let index = lib::Index::load(&root)?.unwrap_or(Index {
                version: lib::INDEX_VERSION,
                ..Default::default()
            });
            *guard = Some(Loaded {
                root: root.clone(),
                index,
                mtime: on_disk,
            });
        }
        let loaded = guard.as_mut().unwrap();
        let mut next = loaded.index.clone();
        let (out, changed) = f(&root, &mut next)?;
        if changed {
            next.sort();
            if root.is_dir() {
                next.save(&root)?;
            }
            loaded.index = next;
            loaded.mtime = mtime(&root);
            drop(guard);
            self.hooks.library_changed();
        }
        Ok(out)
    }

    /// The library, narrowed by `filter`.
    pub fn library(&self, filter: &Filter) -> Result<LibraryView> {
        let d = self.defaults();
        self.with_index(|root, ix| {
            let indexed: std::collections::HashSet<&Path> = ix
                .clips
                .iter()
                .flat_map(|c| {
                    std::iter::once(c.path.as_path()).chain(c.cuts.iter().map(|x| x.path.as_path()))
                })
                .collect();
            let unindexed = if root.is_dir() {
                lib::media_files(root)
                    .iter()
                    .filter(|p| !indexed.contains(p.as_path()))
                    .count()
            } else {
                0
            };
            let clips = ix
                .clips
                .iter()
                .filter(|c| lib::matches(ix, c, filter))
                .map(|c| {
                    let strip = self.strip_path(&c.id);
                    LibItem {
                        name: c.display_name(),
                        file: root.join(&c.path),
                        strip: strip.is_file().then_some(strip),
                        last_import: c.import.is_some() && c.import == ix.last_import,
                        clip: c.clone(),
                    }
                })
                .collect();
            Ok((
                LibraryView {
                    root: root.to_path_buf(),
                    exists: root.is_dir(),
                    unindexed,
                    last_import: ix.last_import.clone(),
                    totals: lib::totals(ix),
                    groups: lib::groups(ix),
                    layout: d.layout,
                    place_folders: d.place_folders,
                    clips,
                },
                false,
            ))
        })
    }

    /// Makes the index again from the files. Also how an existing export folder is adopted:
    /// nothing is moved or written.
    pub fn library_rebuild(&self) -> Result<RebuildReport> {
        let root = self.library_root()?;
        if !root.is_dir() {
            bail!("Library folder {} does not exist.", root.display());
        }
        self.hooks.event(
            "library-task",
            json!({"task": "rebuild", "done": 0, "total": 1}),
        );
        let report = self.with_index(|root, ix| {
            let (fresh, problems) = lib::rebuild(root, Some(ix));
            let report = RebuildReport {
                clips: fresh.clips.len(),
                cuts: fresh.clips.iter().map(|c| c.cuts.len()).sum(),
                problems,
            };
            *ix = fresh;
            Ok((report, true))
        });
        self.hooks.event(
            "library-task",
            json!({"task": "rebuild", "done": 1, "total": 1}),
        );
        report
    }

    fn clip(&self, id: &str) -> Result<(PathBuf, LibClip)> {
        self.with_index(|root, ix| {
            let c = ix
                .get(id)
                .cloned()
                .with_context(|| format!("No clip {id} in the library."))?;
            Ok(((root.to_path_buf(), c), false))
        })
    }

    /// Re-reads clips from their files after a change on disk.
    fn reread_clips(&self, ids: &[String]) -> Result<()> {
        self.with_index(|root, ix| {
            for id in ids {
                let Some(old) = ix.get(id).cloned() else {
                    continue;
                };
                let cut_rels: Vec<PathBuf> = old
                    .cuts
                    .iter()
                    .map(|c| c.path.clone())
                    .filter(|p| root.join(p).is_file())
                    .collect();
                let mut c = lib::reread(root, &old.path, &cut_rels)?;
                c.pending_cuts = old.pending_cuts.clone();
                ix.upsert(c);
            }
            Ok(((), true))
        })
    }

    /// Adds an import's verified files to the library and marks them as the last import.
    pub(crate) fn library_add_results(
        &self,
        root: &Path,
        s: &crate::session::Session,
        import_id: &str,
    ) -> Result<()> {
        if self.library_root().ok().as_deref() != Some(root) {
            return Ok(());
        }
        self.with_index(|root, ix| {
            let mut any = false;
            for r in s.results.iter().filter(|r| r.outcome == Outcome::Verified) {
                let Some(out) = &r.output else { continue };
                let cuts: Vec<PathBuf> = r
                    .cuts
                    .iter()
                    .filter(|c| c.outcome == Outcome::Verified)
                    .filter_map(|c| c.output.as_deref().map(|o| rel_to(root, o)))
                    .collect();
                let mut c = lib::reread(root, &rel_to(root, out), &cuts)?;
                c.in_photos |= s.in_photos.contains(&r.id);
                ix.upsert(c);
                any = true;
            }
            if any
                && s.results
                    .iter()
                    .any(|r| r.outcome == Outcome::Verified && r.size > 0)
            {
                let newest = ix
                    .clips
                    .iter()
                    .filter_map(|c| c.import.clone())
                    .max()
                    .unwrap_or_else(|| import_id.to_string());
                ix.last_import = Some(newest);
            }
            Ok(((), any))
        })
    }

    /// Sets star ratings (0 clears) and pick or reject flags, in the files and the index.
    pub fn library_rate(
        &self,
        ids: &[String],
        rating: Option<u8>,
        flag: Option<Flag>,
    ) -> Result<Vec<LibClip>> {
        if rating.is_some_and(|r| r > 5) {
            bail!("A rating is 0 to 5 stars.");
        }
        self.with_index(|root, ix| {
            let mut out = Vec::new();
            for id in ids {
                let c = ix
                    .get_mut(id)
                    .with_context(|| format!("No clip {id} in the library."))?;
                let mut set = Vec::new();
                if let Some(r) = rating {
                    c.rating = r;
                    set.push((
                        lib::KEY_RATING,
                        if r == 0 { String::new() } else { r.to_string() },
                    ));
                }
                if let Some(f) = flag {
                    c.flag = f;
                    set.push((lib::KEY_FLAG, f.as_str().to_string()));
                }
                lib::write_keys(&root.join(&c.path), &set)?;
                out.push(c.clone());
            }
            Ok((out, true))
        })
    }

    /// Changes a clip's note, keywords, author or location, in its file and its cuts.
    pub fn library_edit(&self, id: &str, e: &LibEdit) -> Result<LibClip> {
        let places = self.defaults().places;
        let location = match (&e.location, e.place.as_deref().map(str::trim)) {
            (Some(l), _) => {
                l.check()?;
                Some(Some(l.clone()))
            }
            (None, Some("")) => Some(None),
            (None, Some(name)) => {
                let p = places
                    .iter()
                    .find(|p| p.name.eq_ignore_ascii_case(name))
                    .with_context(|| format!("no saved place {name:?}"))?;
                Some(Some(crate::metadata::Location {
                    lat: p.lat,
                    lon: p.lon,
                    name: Some(p.name.clone()),
                }))
            }
            (None, None) => None,
        };
        let (root, c) = self.clip(id)?;
        let mut set: Vec<(&str, String)> = Vec::new();
        if let Some(n) = &e.note {
            set.push(("com.apple.quicktime.comment", n.trim().to_string()));
        }
        if let Some(k) = &e.keywords {
            let words = crate::metadata::clean_keywords(k.iter().map(String::as_str));
            set.push(("com.apple.quicktime.keywords", words.join(",")));
        }
        if let Some(a) = &e.author {
            set.push(("com.apple.quicktime.author", a.trim().to_string()));
        }
        if let Some(l) = &location {
            set.push((
                "com.apple.quicktime.location.ISO6709",
                l.as_ref().map(|l| l.iso6709()).unwrap_or_default(),
            ));
            set.push((
                lib::KEY_PLACE,
                l.as_ref().and_then(|l| l.name.clone()).unwrap_or_default(),
            ));
        }
        for f in std::iter::once(c.path.clone()).chain(c.cuts.iter().map(|x| x.path.clone())) {
            lib::write_keys(&root.join(f), &set)?;
        }
        self.reread_clips(&[id.to_string()])?;
        Ok(self.clip(id)?.1)
    }

    /// Renames a clip's file, its cuts and its original to a new short name.
    pub fn library_rename(&self, id: &str, name: &str) -> Result<LibClip> {
        let default_name = self.defaults().default_name;
        let (root, c) = self.clip(id)?;
        let old = root.join(&c.path);
        let dir = old.parent().context("clip has no folder")?.to_path_buf();
        let ext = old
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let old_stem = old
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let date = c.date.format("%Y-%m-%d").to_string();
        let stem = crate::naming::stem(&date, None, name, &default_name);
        if stem == old_stem {
            lib::write_keys(
                &old,
                &[("com.apple.quicktime.title", name.trim().to_string())],
            )?;
            self.reread_clips(&[id.to_string()])?;
            return Ok(self.clip(id)?.1);
        }
        let mut planner = crate::naming::NamePlanner::new();
        let new = planner.claim(&dir, &stem, &ext);
        let new_stem = new
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        // Work out every move first; refuse if any target is taken.
        let mut moves = vec![(old.clone(), new.clone())];
        for cut in &c.cuts {
            let p = root.join(&cut.path);
            let cs = p
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let suffix = cs.strip_prefix(&old_stem).unwrap_or("_cut");
            let ce = p
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            moves.push((
                p.clone(),
                p.with_file_name(format!("{new_stem}{suffix}.{ce}")),
            ));
        }
        if let Some(o) = &c.original {
            let p = root.join(o);
            let oe = p
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            moves.push((p.clone(), p.with_file_name(format!("{new_stem}.{oe}"))));
        }
        if let Some((_, to)) = moves.iter().skip(1).find(|(_, to)| to.exists()) {
            bail!("{} exists already; not renaming.", to.display());
        }
        for (from, to) in &moves {
            std::fs::rename(from, to)
                .with_context(|| format!("renaming {} to {}", from.display(), to.display()))?;
        }
        lib::write_keys(
            &new,
            &[("com.apple.quicktime.title", name.trim().to_string())],
        )?;
        self.with_index(|root, ix| {
            let Some(e) = ix.get_mut(id) else {
                return Ok(((), false));
            };
            let cut_rels: Vec<PathBuf> = moves[1..1 + e.cuts.len()]
                .iter()
                .map(|(_, to)| rel_to(root, to))
                .collect();
            let mut fresh = lib::reread(root, &rel_to(root, &new), &cut_rels)?;
            fresh.pending_cuts = e.pending_cuts.clone();
            fresh.id = e.id.clone();
            *e = fresh;
            Ok(((), true))
        })?;
        Ok(self.clip(id)?.1)
    }

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
        let exported: Vec<ExportedCut> = c
            .cuts
            .iter()
            .map(|x| ExportedCut {
                span: Span {
                    start: x.start,
                    end: x.end,
                },
                path: root.join(&x.path),
            })
            .collect();
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
        let format = if ext.eq_ignore_ascii_case("mov") {
            Format::Mov
        } else {
            Format::Mp4
        };
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
            self.hooks.event(
                "library-task",
                json!({"task": "cuts", "done": i, "total": total}),
            );
            let dst = planner.claim(&dir, &format!("{stem}_cut{n}"), &ext);
            let tmp = dst.with_file_name(format!(
                ".{}.part",
                dst.file_name().unwrap().to_string_lossy()
            ));
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
            let loc =
                crate::qtmeta::get(&qt, "com.apple.quicktime.location.ISO6709").map(str::to_string);
            let res = media::cut(&tools, &src, &tmp, *span, format, encoder, &meta)
                .and_then(|_| crate::qtmeta::write(&tmp, &qt, loc.as_deref()))
                .and_then(|_| media::verify_cut(&tools, &probe, &tmp, *span).map(|_| ()))
                .and_then(|_| {
                    if dst.exists() {
                        bail!("{} appeared during export; not overwriting", dst.display());
                    }
                    std::fs::rename(&tmp, &dst).context("renaming cut")
                });
            match res {
                Ok(()) => {
                    let _ = media::set_mtime(&dst, start_time);
                    made.push(LibCut {
                        path: rel_to(&root, &dst),
                        start: span.start,
                        end: span.end,
                        size: dst.metadata().map(|m| m.len()).unwrap_or(0),
                    });
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    errors.push(format!("cut {:.1}-{:.1} s: {e:#}", span.start, span.end));
                }
            }
        }
        self.hooks.event(
            "library-task",
            json!({"task": "cuts", "done": total, "total": total}),
        );
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

    /// Moves clips (with their cuts and originals) to the Trash and out of the library.
    pub fn library_trash(&self, ids: &[String]) -> Result<TrashReport> {
        let mut report = TrashReport {
            trashed: Vec::new(),
            failed: Vec::new(),
        };
        let mut gone = Vec::new();
        for id in ids {
            let (root, c) = self.clip(id)?;
            let mut ok = true;
            for f in c.files(&root) {
                if !f.exists() {
                    continue;
                }
                match self.trash.trash(&f) {
                    Ok(()) => report.trashed.push(f),
                    Err(e) => {
                        ok = false;
                        report.failed.push((f, format!("{e:#}")));
                    }
                }
            }
            if ok {
                gone.push(id.clone());
            }
        }
        self.with_index(|_, ix| {
            ix.clips.retain(|c| !gone.contains(&c.id));
            Ok(((), true))
        })?;
        let _ = self.reread_clips(
            &ids.iter()
                .filter(|i| !gone.contains(i))
                .cloned()
                .collect::<Vec<_>>(),
        );
        Ok(report)
    }

    /// Adds library clips and their cuts to Photos.
    pub fn library_photos(&self, ids: &[String], album: Option<String>) -> Result<ShareReport> {
        let mut files = Vec::new();
        for id in ids {
            let (root, c) = self.clip(id)?;
            files.push(root.join(&c.path));
            files.extend(c.cuts.iter().map(|x| root.join(&x.path)));
        }
        let album = album.or(Some(self.defaults().photos_album));
        let report = photos::share(self.photos.as_ref(), &files, album.as_deref())?;
        self.library_mark_photos(&report.added);
        Ok(report)
    }

    /// Marks added files as in Photos, in the files and the index.
    pub(crate) fn library_mark_photos(&self, added: &[PathBuf]) {
        if added.is_empty() {
            return;
        }
        for f in added {
            let _ = lib::write_keys(f, &[(lib::KEY_PHOTOS, "1".into())]);
        }
        let _ = self.with_index(|root, ix| {
            let mut any = false;
            for c in &mut ix.clips {
                if added.iter().any(|a| *a == root.join(&c.path)) {
                    c.in_photos = true;
                    any = true;
                }
            }
            Ok(((), any))
        });
    }

    /// A file the webview can play: the clip itself when it is an MP4, else a small preview.
    pub fn library_preview(&self, id: &str) -> Result<PathBuf> {
        let (root, c) = self.clip(id)?;
        let file = root.join(&c.path);
        if file
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("mp4"))
        {
            return Ok(file);
        }
        let tools = media::find_tools()?;
        let dst = self
            .cache
            .join("proxies")
            .join(format!("lib-{}.mp4", lib::id_file(id)));
        if !dst.is_file() {
            std::fs::create_dir_all(dst.parent().unwrap())?;
            media::proxy(&tools, &file, &dst)?;
        }
        Ok(dst)
    }

    /// Makes the hover-scrub strips that are missing (for `ids`, or every clip).
    pub fn library_strips(&self, ids: Option<Vec<String>>) -> Result<usize> {
        let tools = media::find_tools()?;
        let todo: Vec<(PathBuf, LibClip)> = self.with_index(|root, ix| {
            Ok((
                ix.clips
                    .iter()
                    .filter(|c| ids.as_ref().is_none_or(|ids| ids.contains(&c.id)))
                    .filter(|c| !self.strip_path(&c.id).is_file())
                    .map(|c| (root.to_path_buf(), c.clone()))
                    .collect(),
                false,
            ))
        })?;
        let total = todo.len();
        let mut made = 0;
        for (i, (root, c)) in todo.iter().enumerate() {
            self.hooks.event(
                "library-task",
                json!({"task": "thumbnails", "done": i, "total": total}),
            );
            if lib::make_strip(
                &tools,
                &root.join(&c.path),
                c.duration,
                &self.strip_path(&c.id),
            )
            .is_ok()
            {
                made += 1;
            }
        }
        if total > 0 {
            self.hooks.event(
                "library-task",
                json!({"task": "thumbnails", "done": total, "total": total}),
            );
            self.hooks.library_changed();
        }
        Ok(made)
    }

    /// Finds dead air again in a clip's frames and stores its keep ranges in the file.
    pub fn library_rescan(&self, id: &str) -> Result<LibClip> {
        let tools = media::find_tools()?;
        let (root, c) = self.clip(id)?;
        let src = c
            .original
            .as_ref()
            .map(|o| root.join(o))
            .filter(|o| o.is_file())
            .unwrap_or_else(|| root.join(&c.path));
        self.hooks.event(
            "library-task",
            json!({"task": "moments", "done": 0, "total": 1}),
        );
        let probe = media::probe(&tools, &src)?;
        let scan = crate::moments::scan_signal(&tools, &src, probe.fps, probe.duration)?;
        lib::write_keys(
            &root.join(&c.path),
            &[(lib::KEY_KEEP, lib::spans_value(&scan.keep))],
        )?;
        self.reread_clips(&[id.to_string()])?;
        self.hooks.event(
            "library-task",
            json!({"task": "moments", "done": 1, "total": 1}),
        );
        Ok(self.clip(id)?.1)
    }

    /// Clips on a card, and how many of them are not in the library yet (by content).
    pub fn card_status(&self, mount: &Path) -> Result<CardStatus> {
        let found = crate::scan::find_clips(mount);
        let known = self
            .with_index(|_, ix| Ok((lib::known_sources(ix), false)))
            .unwrap_or_default();
        let staged: Vec<String> = self
            .session()
            .map(|s| s.clips.iter().map(|c| c.key.clone()).collect())
            .unwrap_or_default();
        let new = found
            .iter()
            .filter(|f| {
                crate::pipeline::fingerprint(&f.path)
                    .map(|k| !known.contains(&k) && !staged.contains(&k))
                    .unwrap_or(true)
            })
            .count();
        let size = crate::disk::probe_volume(mount)
            .map(|v| v.info.total_size)
            .unwrap_or(0);
        Ok(CardStatus {
            mount: mount.to_path_buf(),
            clips: found.len(),
            new,
            free: crate::pipeline::free_bytes(mount).ok(),
            size,
        })
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
        let exported = session_exported(&s, patch.id);
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

/// A session clip's cuts that verified and still exist as files.
pub(crate) fn session_exported(s: &crate::session::Session, id: usize) -> Vec<ExportedCut> {
    s.results
        .iter()
        .rev()
        .find(|r| r.id == id && r.outcome == Outcome::Verified)
        .map(|r| {
            r.cuts
                .iter()
                .filter(|c| c.outcome == Outcome::Verified)
                .filter_map(|c| {
                    let p = c.output.clone()?;
                    p.is_file().then_some(ExportedCut {
                        span: Span {
                            start: c.start,
                            end: c.end,
                        },
                        path: p,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}
