//! `Core`'s library half: the index, listing, rating, editing, renaming, redating and
//! moving the clips in the library folder.

use super::Core;
use crate::api::{Event, LibraryTask, Task};
use crate::library::{self as lib, Filter, Flag, Index, LibClip};
use crate::media;
use crate::pipeline::Outcome;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A clip as the front ends see it: the index entry plus absolute paths.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct LibItem {
    #[serde(flatten)]
    pub clip: LibClip,
    pub name: String,
    pub file: PathBuf,
    /// The hover-scrub strip, once made.
    pub strip: Option<PathBuf>,
    /// One frame, when no strip could be made.
    pub poster: Option<PathBuf>,
    /// Neither a strip nor a poster could be made.
    pub no_picture: bool,
    pub last_import: bool,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct LibEdit {
    pub note: Option<String>,
    pub keywords: Option<Vec<String>>,
    pub author: Option<String>,
    /// A saved place's name; empty removes the location.
    pub place: Option<String>,
    pub location: Option<crate::metadata::Location>,
    /// A new flying day. The clip, its cuts and its original move to that day's folder.
    pub date: Option<chrono::NaiveDate>,
    /// Time of day, `HH:MM`; empty means local noon. Without it a new date keeps the
    /// clip's time.
    pub time: Option<String>,
    /// An aircraft profile name; empty removes the profile's details.
    pub profile: Option<String>,
}

impl LibEdit {
    /// True when the edit changes nothing.
    pub fn is_empty(&self) -> bool {
        let LibEdit {
            note,
            keywords,
            author,
            place,
            location,
            date,
            time,
            profile,
        } = self;
        note.is_none()
            && keywords.is_none()
            && author.is_none()
            && place.is_none()
            && location.is_none()
            && date.is_none()
            && time.is_none()
            && profile.is_none()
    }
}

/// Every change one call may make to library clips: stars and flag, a new name (one clip
/// only), and the details in `LibEdit`. Missing fields stay as they are.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct LibUpdate {
    /// Stars, 0 to 5; 0 clears.
    pub rating: Option<u8>,
    pub flag: Option<Flag>,
    /// A new short name; needs exactly one clip.
    pub name: Option<String>,
    #[serde(flatten)]
    pub edit: LibEdit,
}

/// An edit whose names, location and time were checked.
pub(super) struct PreparedEdit {
    location: Option<Option<crate::metadata::Location>>,
    profile: Option<Option<crate::metadata::Profile>>,
    time: Option<Option<chrono::NaiveTime>>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct RebuildReport {
    pub clips: usize,
    pub cuts: usize,
    pub problems: Vec<String>,
}

/// What `library_apply_name_format` did, by path relative to the library folder.
#[derive(Debug, Clone, Default, Serialize, specta::Type)]
pub struct RenameReport {
    pub renamed: Vec<(PathBuf, PathBuf)>,
    pub unchanged: usize,
    /// Names that do not start with a date.
    pub skipped: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
}

/// The index in memory, and the index file's mtime when it was read.
pub(crate) struct Loaded {
    root: PathBuf,
    index: Index,
    mtime: Option<std::time::SystemTime>,
}

pub(super) fn rel_to(root: &Path, p: &Path) -> PathBuf {
    p.strip_prefix(root).unwrap_or(p).to_path_buf()
}

impl Core {
    /// The library folder: the output folder setting.
    pub fn library_root(&self) -> Result<PathBuf> {
        self.defaults()
            .output_dir
            .context("No library folder set. Choose one in Settings.")
    }

    /// Runs `f` on the loaded index (loading it first, and again when another process
    /// changed the file), then saves it when `f` says it changed. A missing index starts
    /// empty.
    pub(super) fn with_index<T>(
        &self,
        f: impl FnOnce(&Path, &mut Index) -> Result<(T, bool)>,
    ) -> Result<T> {
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
            let mut index = lib::Index::load(&root)?.unwrap_or(Index {
                version: lib::INDEX_VERSION,
                id_scheme: lib::ID_SCHEME,
                ..Default::default()
            });
            if index.id_scheme < lib::ID_SCHEME {
                self.migrate_ids(&root, &mut index);
                if root.is_dir() {
                    index.save(&root)?;
                }
            }
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

    /// Moves an index from QuadCam 0.4's ids to the current scheme (see `identity`). Each
    /// clip gets the id its file now reads as: an adopted file its new head id, a clip with
    /// a kept original that proves its 0.4 id the original's new fingerprint. The 0.4 id
    /// stays as an alias, and the clip's cached pictures follow it. Everything else in the
    /// entry stays, unsaved cuts included. No media file is written.
    fn migrate_ids(&self, root: &Path, ix: &mut Index) {
        for c in &mut ix.clips {
            if !crate::identity::is_legacy(&c.id) {
                continue;
            }
            let Ok(lib::Found::Clip(now)) = lib::read_file(root, &c.path) else {
                continue;
            };
            if now.id == c.id {
                continue;
            }
            for (old, new) in [
                (self.strip_path(&c.id), self.strip_path(&now.id)),
                (self.poster_path(&c.id), self.poster_path(&now.id)),
                (self.no_picture_path(&c.id), self.no_picture_path(&now.id)),
            ] {
                if old.is_file() && !new.exists() {
                    let _ = std::fs::rename(&old, &new);
                }
            }
            let mut aliases = now.aliases.clone();
            if !aliases.contains(&c.id) {
                aliases.push(c.id.clone());
            }
            c.id = now.id.clone();
            c.aliases = aliases;
        }
        ix.id_scheme = lib::ID_SCHEME;
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
                    let poster = self.poster_path(&c.id);
                    LibItem {
                        name: c.display_name(),
                        file: root.join(&c.path),
                        strip: strip.is_file().then_some(strip),
                        poster: poster.is_file().then_some(poster),
                        no_picture: self.no_picture_path(&c.id).is_file(),
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
        self.hooks.event(Event::LibraryTask(LibraryTask {
            task: Task::Rebuild,
            done: 0,
            total: 1,
        }));
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
        self.hooks.event(Event::LibraryTask(LibraryTask {
            task: Task::Rebuild,
            done: 1,
            total: 1,
        }));
        report
    }

    pub(super) fn clip(&self, id: &str) -> Result<(PathBuf, LibClip)> {
        self.with_index(|root, ix| {
            let c = ix
                .get(id)
                .cloned()
                .with_context(|| format!("No clip {id} in the library."))?;
            Ok(((root.to_path_buf(), c), false))
        })
    }

    /// Re-reads clips from their files after a change on disk.
    pub(super) fn reread_clips(&self, ids: &[String]) -> Result<()> {
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

    /// Changes a clip's note, keywords, author, location, aircraft profile, date or time,
    /// in its file and its cuts. A new day moves the clip, its cuts and its original to
    /// that day's folder (and renames them when the file name starts with the date).
    pub fn library_edit(&self, id: &str, e: &LibEdit) -> Result<LibClip> {
        let prepared = self.prepare_edit(e)?;
        self.apply_edit(id, e, &prepared)
    }

    /// Checks an edit before any file changes: the place and profile names, the location and
    /// the time.
    fn prepare_edit(&self, e: &LibEdit) -> Result<PreparedEdit> {
        let d = self.defaults();
        let places = d.places.clone();
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
                    .with_context(|| {
                        format!(
                            "no saved place {name:?}; saved places: {}",
                            places
                                .iter()
                                .map(|p| p.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })?;
                Some(Some(crate::metadata::Location {
                    lat: p.lat,
                    lon: p.lon,
                    name: Some(p.name.clone()),
                }))
            }
            (None, None) => None,
        };
        let profile = match e.profile.as_deref().map(str::trim) {
            None => None,
            Some("") => Some(None),
            Some(name) => Some(Some(
                d.profiles
                    .iter()
                    .find(|p| p.name.eq_ignore_ascii_case(name))
                    .cloned()
                    .with_context(|| {
                        format!(
                            "no aircraft profile {name:?}; profiles: {}",
                            d.profiles
                                .iter()
                                .map(|p| p.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })?,
            )),
        };
        let time = e
            .time
            .as_deref()
            .map(crate::session::parse_time)
            .transpose()?;
        Ok(PreparedEdit {
            location,
            profile,
            time,
        })
    }

    /// Writes a checked edit into one clip, its cuts and the index.
    fn apply_edit(&self, id: &str, e: &LibEdit, prepared: &PreparedEdit) -> Result<LibClip> {
        let d = self.defaults();
        let PreparedEdit {
            location,
            profile,
            time,
        } = prepared;
        let (root, c) = self.clip(id)?;
        let file = root.join(&c.path);
        let items = crate::qtmeta::read(&file)?;
        let mut set: Vec<(&str, String)> = Vec::new();
        if let Some(n) = &e.note {
            set.push(("com.apple.quicktime.comment", n.trim().to_string()));
        }
        let mut keywords = e.keywords.clone().unwrap_or_else(|| c.keywords.clone());
        let mut keywords_changed = e.keywords.is_some();
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
        if let Some(p) = &profile {
            // The profile's own keywords and author come out; the new one's go in.
            let old = crate::qtmeta::get(&items, lib::KEY_PROFILE)
                .and_then(|n| d.profiles.iter().find(|x| x.name.eq_ignore_ascii_case(n)));
            if let Some(old) = old {
                keywords.retain(|k| {
                    k.eq_ignore_ascii_case("FPV")
                        || !old.keywords.iter().any(|o| o.eq_ignore_ascii_case(k))
                });
                let author = crate::qtmeta::get(&items, "com.apple.quicktime.author");
                if e.author.is_none()
                    && !old.author.is_empty()
                    && author == Some(old.author.as_str())
                {
                    set.push((
                        "com.apple.quicktime.author",
                        p.as_ref().map(|p| p.author.clone()).unwrap_or_default(),
                    ));
                }
            }
            let mut words = vec!["FPV".to_string()];
            words.extend(p.iter().flat_map(|p| p.keywords.clone()));
            words.append(&mut keywords);
            keywords = words;
            keywords_changed = true;
            let get = |f: fn(&crate::metadata::Profile) -> &String| {
                p.as_ref().map(|p| f(p).clone()).unwrap_or_default()
            };
            set.push(("com.apple.quicktime.make", get(|p| &p.camera_make)));
            set.push(("com.apple.quicktime.model", get(|p| &p.camera_model)));
            set.push((lib::KEY_AIRCRAFT, get(|p| &p.aircraft)));
            set.push((lib::KEY_VIDEO_SYSTEM, get(|p| &p.video_system)));
            set.push((lib::KEY_PROFILE, get(|p| &p.name)));
        }
        if keywords_changed {
            let words = crate::metadata::clean_keywords(keywords.iter().map(String::as_str));
            set.push(("com.apple.quicktime.keywords", words.join(",")));
        }
        for f in std::iter::once(c.path.clone()).chain(c.cuts.iter().map(|x| x.path.clone())) {
            lib::write_keys(&root.join(f), &set)?;
        }
        if e.date.is_some() || time.is_some() {
            self.redate(&root, &c, &items, e.date, *time)?;
        } else {
            self.reread_clips(&[id.to_string()])?;
        }
        Ok(self.clip(id)?.1)
    }

    /// Changes stars, flag, name and details of library clips in one call. Every value and
    /// clip id is checked before any file changes; then the stars and flag, the name, and the
    /// details are written, in that order.
    pub fn library_update(&self, ids: &[String], u: &LibUpdate) -> Result<Vec<LibClip>> {
        if ids.is_empty() {
            bail!("ids is required: library clip ids from quadcam_library.");
        }
        if u.rating.is_none() && u.flag.is_none() && u.name.is_none() && u.edit.is_empty() {
            bail!("Nothing to change: give rating, flag, name, note, keywords, author, place, location, profile, date or time.");
        }
        if u.name.is_some() && ids.len() != 1 {
            bail!("name renames one clip; give exactly one id.");
        }
        if u.rating.is_some_and(|r| r > 5) {
            bail!("A rating is 0 to 5 stars.");
        }
        for id in ids {
            self.clip(id)?;
        }
        let prepared = self.prepare_edit(&u.edit)?;
        if u.rating.is_some() || u.flag.is_some() {
            self.library_rate(ids, u.rating, u.flag)?;
        }
        if let Some(name) = &u.name {
            self.library_rename(&ids[0], name)?;
        }
        if !u.edit.is_empty() {
            for id in ids {
                self.apply_edit(id, &u.edit, &prepared)?;
            }
        }
        ids.iter().map(|id| Ok(self.clip(id)?.1)).collect()
    }

    /// Gives a clip a new date and time: the QuickTime creation date, the movie header time
    /// and the mtime of the clip, its cuts and its original. A new day moves them all.
    pub(super) fn redate(
        &self,
        root: &Path,
        c: &LibClip,
        items: &[crate::qtmeta::Item],
        date: Option<chrono::NaiveDate>,
        time: Option<Option<chrono::NaiveTime>>,
    ) -> Result<()> {
        let old_created = crate::qtmeta::get(items, "com.apple.quicktime.creationdate")
            .and_then(|v| chrono::DateTime::parse_from_str(v, "%Y-%m-%dT%H:%M:%S%z").ok());
        let old_time = old_created
            .map(|t| t.with_timezone(&chrono::Local).time())
            .or_else(|| {
                c.time
                    .as_deref()
                    .and_then(|t| chrono::NaiveTime::parse_from_str(t, "%H:%M").ok())
            });
        let new_date = date.unwrap_or(c.date);
        let new_time = match time {
            Some(t) => t,
            None => old_time,
        };
        let created =
            crate::pipeline::creation_time(new_date, new_time, crate::pipeline::DateSource::Edited);
        // A time set here is manual; an empty one removes the time; a date alone keeps it.
        let source = match time {
            Some(Some(_)) => Some("manual".to_string()),
            Some(None) => Some("none".to_string()),
            None => None,
        };
        let stamp = |path: &Path, t: chrono::DateTime<chrono::Utc>| -> Result<()> {
            let mut set = vec![(
                "com.apple.quicktime.creationdate",
                crate::metadata::creation_date(t),
            )];
            if let Some(s) = &source {
                set.push((lib::KEY_TIME, s.clone()));
            }
            lib::write_keys(path, &set)?;
            crate::qtmeta::set_movie_time(path, t)?;
            media::set_mtime(path, t)
        };
        stamp(&root.join(&c.path), created)?;
        for cut in &c.cuts {
            let t = created + chrono::Duration::milliseconds((cut.start * 1000.0) as i64);
            stamp(&root.join(&cut.path), t)?;
        }
        if let Some(o) = &c.original {
            let _ = media::set_mtime(&root.join(o), created);
        }
        for side in c.sidecars(root) {
            let _ = media::set_mtime(&side, created);
        }
        if new_date == c.date {
            return self.reread_clips(std::slice::from_ref(&c.id));
        }
        let d = self.defaults();
        let place = d.place_folders.then_some(c.place.as_deref()).flatten();
        let dir = lib::day_dir(root, d.layout, new_date, place);
        let old_stem = c
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let stem = match crate::naming::split_date(&old_stem) {
            Some((_, rest)) => format!("{}{rest}", d.name_date_format.format(new_date)),
            None => old_stem,
        };
        self.relocate(root, c, &dir, &stem).map(|_| ())
    }

    /// Moves a clip, its cuts, its original and the original's sidecars to `dir` under the
    /// stem `stem` (or the next free `stem-2`, ...), updates the index and removes folders the move left empty.
    pub(super) fn relocate(
        &self,
        root: &Path,
        c: &LibClip,
        dir: &Path,
        stem: &str,
    ) -> Result<PathBuf> {
        let old = root.join(&c.path);
        let old_dir = old.parent().context("clip has no folder")?.to_path_buf();
        let ext = |p: &Path| {
            p.extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        };
        let stem_of = |p: &Path| {
            p.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        };
        let old_stem = stem_of(&old);
        let new = if dir == old_dir && stem == old_stem {
            old.clone()
        } else {
            crate::naming::NamePlanner::new().claim(dir, stem, &ext(&old))
        };
        let new_stem = stem_of(&new);
        // Work out every move first; refuse if any target is taken.
        let mut moves = vec![(old.clone(), new.clone())];
        for cut in &c.cuts {
            let p = root.join(&cut.path);
            let cs = stem_of(&p);
            let suffix = cs.strip_prefix(&old_stem).unwrap_or("_cut");
            moves.push((
                p.clone(),
                dir.join(format!("{new_stem}{suffix}.{}", ext(&p))),
            ));
        }
        if let Some(o) = &c.original {
            let p = root.join(o);
            moves.push((
                p.clone(),
                dir.join(lib::ORIGINALS)
                    .join(format!("{new_stem}.{}", ext(&p))),
            ));
            // Sidecars (a DJI `.srt`) follow the original's new stem.
            for side in c.sidecars(root) {
                let to = dir
                    .join(lib::ORIGINALS)
                    .join(format!("{new_stem}.{}", ext(&side)));
                moves.push((side, to));
            }
        }
        if let Some((_, to)) = moves
            .iter()
            .skip(1)
            .find(|(from, to)| from != to && to.exists())
        {
            bail!("{} exists already; not moving.", to.display());
        }
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        for (from, to) in &moves {
            if from == to {
                continue;
            }
            if let Some(p) = to.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::rename(from, to)
                .with_context(|| format!("moving {} to {}", from.display(), to.display()))?;
        }
        if old_dir != dir {
            // Remove what the move emptied: originals/, the day folder, the year folder.
            for d in [old_dir.join(lib::ORIGINALS), old_dir.clone()]
                .into_iter()
                .chain(old_dir.parent().map(Path::to_path_buf))
            {
                if d.starts_with(root) && d != root {
                    let _ = std::fs::remove_dir(&d);
                }
            }
        }
        let id = c.id.clone();
        self.with_index(|root, ix| {
            let Some(e) = ix.get_mut(&id) else {
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
        Ok(new)
    }

    /// Renames a clip's file, its cuts and its original to a new short name.
    pub fn library_rename(&self, id: &str, name: &str) -> Result<LibClip> {
        let d = self.defaults();
        let default_name = d.default_name;
        let (root, c) = self.clip(id)?;
        let old = root.join(&c.path);
        let dir = old.parent().context("clip has no folder")?.to_path_buf();
        let date = d.name_date_format.format(c.date);
        let stem = crate::naming::stem(&date, None, name, &default_name);
        let new = self.relocate(&root, &c, &dir, &stem)?;
        lib::write_keys(
            &new,
            &[("com.apple.quicktime.title", name.trim().to_string())],
        )?;
        self.reread_clips(&[id.to_string()])?;
        Ok(self.clip(id)?.1)
    }

    /// Renames clips (all when `ids` is None) so their file names start with the date in
    /// the name date format setting, with their cuts and originals. Clips whose names do
    /// not start with a date are left alone. Folders stay as they are.
    pub fn library_apply_name_format(&self, ids: Option<Vec<String>>) -> Result<RenameReport> {
        let format = self.defaults().name_date_format;
        let clips: Vec<(PathBuf, LibClip)> = self.with_index(|root, ix| {
            Ok((
                ix.clips
                    .iter()
                    .filter(|c| ids.as_ref().is_none_or(|ids| ids.contains(&c.id)))
                    .map(|c| (root.to_path_buf(), c.clone()))
                    .collect(),
                false,
            ))
        })?;
        let mut report = RenameReport::default();
        for (root, c) in clips {
            let file = root.join(&c.path);
            let stem = file
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let Some((_, rest)) = crate::naming::split_date(&stem) else {
                report.skipped.push(c.path.clone());
                continue;
            };
            let want = format!("{}{rest}", format.format(c.date));
            if want == stem {
                report.unchanged += 1;
                continue;
            }
            let dir = file.parent().context("clip has no folder")?.to_path_buf();
            match self.relocate(&root, &c, &dir, &want) {
                Ok(new) => report.renamed.push((c.path.clone(), rel_to(&root, &new))),
                Err(e) => report.failed.push((c.path.clone(), format!("{e:#}"))),
            }
        }
        Ok(report)
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
        self.hooks.event(Event::LibraryTask(LibraryTask {
            task: Task::Moments,
            done: 0,
            total: 1,
        }));
        let probe = media::probe(&tools, &src)?;
        let scan = crate::moments::scan_signal(&tools, &src, probe.fps, probe.duration)?;
        lib::write_keys(
            &root.join(&c.path),
            &[(lib::KEY_KEEP, lib::spans_value(&scan.keep))],
        )?;
        self.reread_clips(&[id.to_string()])?;
        self.hooks.event(Event::LibraryTask(LibraryTask {
            task: Task::Moments,
            done: 1,
            total: 1,
        }));
        Ok(self.clip(id)?.1)
    }
}
