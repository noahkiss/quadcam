//! `Core`'s file work: Photos for the session and the library, previews (one proxy at a
//! time), hover-scrub strips, and the Trash.

use super::library::rel_to;
use super::Core;
use crate::api::{Event, LibraryTask, Task};
use crate::library::{self as lib, LibClip};
use crate::media;
use crate::photos::{self, ShareReport};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct TrashReport {
    pub trashed: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
    /// Where each file went, so `library_untrash` can put it back.
    pub moved: Vec<Moved>,
}

/// A library file in the Trash: the clip it belongs to, where it was, where it is now.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct Moved {
    pub id: String,
    pub from: PathBuf,
    pub to: PathBuf,
}

impl Core {
    /// Adds verified outputs to Photos (all verified clips when `ids` is None).
    pub fn add_to_photos(
        &self,
        ids: Option<Vec<usize>>,
        album: Option<String>,
    ) -> Result<ShareReport> {
        let s = self.current()?;
        let outs = s.verified_outputs(ids.as_deref());
        let files: Vec<PathBuf> = outs.iter().map(|(_, p)| p.clone()).collect();
        let report = self.share_files(&files, album.as_deref())?;
        let added: Vec<usize> = outs
            .iter()
            .filter(|(_, p)| report.added.contains(p))
            .map(|(id, _)| *id)
            .collect();
        if !added.is_empty() {
            let mut latest = self.current()?;
            for id in added {
                if !latest.in_photos.contains(&id) {
                    latest.in_photos.push(id);
                }
            }
            self.commit(Some(latest))?;
        }
        Ok(report)
    }

    /// Makes the previews of every analysed clip in the session, so Play starts at once. It
    /// stops when the session changes or another step (an export) starts.
    pub fn make_previews(&self) -> usize {
        let Some(s) = self.session() else { return 0 };
        let mut made = 0;
        for c in s.clips.iter().filter(|c| c.probe.is_some()) {
            let same = self
                .session()
                .is_some_and(|now| now.staging == s.staging && now.clips.len() == s.clips.len());
            if !same || self.busy.load(Ordering::SeqCst) {
                break;
            }
            if self.preview(c.id).is_ok() {
                made += 1;
            }
        }
        made
    }

    /// A small H.264 preview the webview can play, made once per clip content. It is named
    /// by the source's fingerprint, so a new PICT0001 never reuses an old flight's preview.
    pub fn preview(&self, id: usize) -> Result<PathBuf> {
        let s = self.current()?;
        let c = s
            .clips
            .iter()
            .find(|c| c.id == id)
            .context("No such clip.")?;
        let src = c.source().context("Clip is not readable.")?;
        let tools = media::find_tools()?;
        let dst = self.cache.join("proxies").join(format!(
            "{}-{}.mp4",
            src.file_stem().unwrap_or_default().to_string_lossy(),
            crate::pipeline::fingerprint(src)?
        ));
        self.proxy_once(&tools, src, &dst)
    }

    /// Makes `dst`, a preview of `src`, unless it exists. One preview is made at a time; a
    /// second call for the same file waits for the first and reuses its file.
    pub(crate) fn proxy_once(
        &self,
        tools: &media::Tools,
        src: &Path,
        dst: &Path,
    ) -> Result<PathBuf> {
        static MAKING: Mutex<()> = Mutex::new(());
        if let Some(dir) = dst.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let _one = MAKING.lock().unwrap_or_else(|e| e.into_inner());
        if !dst.is_file() {
            media::proxy(tools, src, dst)?;
        }
        Ok(dst.to_path_buf())
    }

    /// Adds files to Photos (into `album`, or the library only) and marks the library's
    /// clips among them as in Photos.
    pub(crate) fn share_files(
        &self,
        files: &[PathBuf],
        album: Option<&str>,
    ) -> Result<ShareReport> {
        let report = photos::share(self.photos.as_ref(), files, album)?;
        self.library_mark_photos(&report.added);
        Ok(report)
    }

    pub(super) fn strip_path(&self, id: &str) -> PathBuf {
        self.cache
            .join("library")
            .join(format!("{}.jpg", lib::id_file(id)))
    }

    /// The single-frame fallback when the strip failed.
    pub(super) fn poster_path(&self, id: &str) -> PathBuf {
        self.cache
            .join("library")
            .join(format!("{}.poster.jpg", lib::id_file(id)))
    }

    /// Marks a clip whose strip and poster both failed, so it is not tried again on every
    /// reload. Removed with the cache, or by `library_strips` for these ids.
    pub(super) fn no_picture_path(&self, id: &str) -> PathBuf {
        self.cache
            .join("library")
            .join(format!("{}.none", lib::id_file(id)))
    }

    /// Whether a clip still needs a strip attempt.
    pub(super) fn needs_picture(&self, id: &str) -> bool {
        !self.strip_path(id).is_file()
            && !self.poster_path(id).is_file()
            && !self.no_picture_path(id).is_file()
    }

    /// Moves clips (with their cuts and originals) to the Trash and out of the library.
    pub fn library_trash(&self, ids: &[String]) -> Result<TrashReport> {
        let mut report = TrashReport {
            trashed: Vec::new(),
            failed: Vec::new(),
            moved: Vec::new(),
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
                    Ok(to) => {
                        if let Some(to) = to {
                            report.moved.push(Moved {
                                id: id.clone(),
                                from: f.clone(),
                                to,
                            });
                        }
                        report.trashed.push(f)
                    }
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

    /// Puts trashed library files back where they were and the clips back in the index:
    /// the undo of `library_trash`. Refuses before moving anything when a file is no longer
    /// in the Trash, its old place is taken, or it was not in the library.
    pub fn library_untrash(&self, moved: &[Moved]) -> Result<Vec<String>> {
        let root = self.library_root()?;
        for m in moved {
            if !m.from.starts_with(&root) {
                bail!("{} is not in the library.", m.from.display());
            }
            if m.from.exists() {
                bail!("{} exists already; not putting it back.", m.from.display());
            }
            if !m.to.is_file() {
                bail!("{} is no longer in the Trash.", m.to.display());
            }
        }
        for m in moved {
            if let Some(dir) = m.from.parent() {
                std::fs::create_dir_all(dir)?;
            }
            crate::trash::move_file(&m.to, &m.from)?;
        }
        let mut ids: Vec<String> = Vec::new();
        for m in moved {
            if !ids.contains(&m.id) {
                ids.push(m.id.clone());
            }
        }
        self.with_index(|root, ix| {
            for id in &ids {
                // The clip comes first in `LibClip::files`, then its cuts, then its original.
                let rels: Vec<PathBuf> = moved
                    .iter()
                    .filter(|m| &m.id == id)
                    .map(|m| rel_to(root, &m.from))
                    .collect();
                let Some((clip, rest)) = rels.split_first() else {
                    continue;
                };
                let mut c = lib::reread(root, clip, rest)?;
                c.id = id.clone();
                ix.upsert(c);
            }
            Ok(((), true))
        })?;
        Ok(ids)
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
        self.share_files(&files, album.as_deref())
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
        self.proxy_once(&tools, &file, &dst)
    }

    /// Makes the hover-scrub strips that are missing (for `ids`, or every clip). When a
    /// strip fails, one frame becomes the clip's poster; when that fails too, the clip is
    /// marked so later calls skip it (naming `ids` tries those again).
    pub fn library_strips(&self, ids: Option<Vec<String>>) -> Result<usize> {
        let tools = media::find_tools()?;
        if let Some(ids) = &ids {
            for id in ids {
                let _ = std::fs::remove_file(self.no_picture_path(id));
            }
        }
        let todo: Vec<(PathBuf, LibClip)> = self.with_index(|root, ix| {
            Ok((
                ix.clips
                    .iter()
                    .filter(|c| ids.as_ref().is_none_or(|ids| ids.contains(&c.id)))
                    .filter(|c| self.needs_picture(&c.id))
                    .map(|c| (root.to_path_buf(), c.clone()))
                    .collect(),
                false,
            ))
        })?;
        let total = todo.len();
        let mut made = 0;
        for (i, (root, c)) in todo.iter().enumerate() {
            self.hooks.event(Event::LibraryTask(LibraryTask {
                task: Task::Thumbnails,
                done: i,
                total,
            }));
            let file = root.join(&c.path);
            let strip = lib::make_strip(&tools, &file, c.duration, &self.strip_path(&c.id));
            if strip.is_ok() {
                made += 1;
                continue;
            }
            let poster = self.poster_path(&c.id);
            if media::thumbnail(&tools, &file, &poster).is_ok() {
                made += 1;
            } else {
                let _ = std::fs::write(self.no_picture_path(&c.id), b"");
                eprintln!(
                    "quadcam: no picture for {}: {:#}",
                    file.display(),
                    strip.unwrap_err()
                );
            }
        }
        if total > 0 {
            self.hooks.event(Event::LibraryTask(LibraryTask {
                task: Task::Thumbnails,
                done: total,
                total,
            }));
            self.hooks.library_changed();
        }
        Ok(made)
    }
}
