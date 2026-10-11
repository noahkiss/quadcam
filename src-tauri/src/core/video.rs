//! `Core`'s video analysis: highlights and the key frame of session and library clips through
//! the `quadcam-intel` helper (`crate::highlights`), one clip at a time, in the background.
//! The analyzer runs about 5x real time, so an import never waits for it: the session job
//! stops when another step starts, and the library job picks up what it left. Each answer
//! is cached by the clip's content fingerprint. Where video analysis is unavailable (macOS
//! older than 27, no helper), every job does nothing and says why.

use super::Core;
use crate::api::{Event, LibraryTask, Task};
use crate::highlights::{self, VideoScan};
use crate::intel::{Feature, Watched};
use crate::library::{self as lib, LibClip};
use crate::media;
use crate::moments::{Moment, MomentKind, Source, Span};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Mutex;

/// One analysis at a time across the session and the library jobs.
static ONE: Mutex<()> = Mutex::new(());

/// The width of the key-frame picture in the library.
const COVER_W: u32 = 384;

/// `library_analyze_video`: which clips, and whether to analyse them again.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct VideoParams {
    /// Library clip ids; empty means every clip (cuts excluded).
    pub ids: Vec<String>,
    /// Analyse clips again, past the cache. Without it, clips analysed already are skipped.
    pub again: bool,
}

/// What a library analysis did with one clip.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct VideoClip {
    pub id: String,
    pub path: PathBuf,
    pub highlights: usize,
    /// Seconds into the clip: the frame that now stands for it.
    pub key_frame: Option<f64>,
    /// The answer came from the cache.
    pub cached: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, specta::Type)]
pub struct VideoReport {
    /// Why video analysis cannot run on this Mac; nothing ran.
    pub unavailable: Option<String>,
    pub clips: Vec<VideoClip>,
}

/// Dead air as moments, from the ranges with a picture: what lies between them.
fn dead_from_keep(keep: &[Span], duration: f64) -> Vec<Moment> {
    if keep.is_empty() {
        return Vec::new();
    }
    let dead = |start: f64, end: f64| Moment {
        kind: MomentKind::DeadAir,
        start,
        end,
        score: 1.0,
        source: Source::Video,
        detail: String::new(),
    };
    let mut out = Vec::new();
    let mut at = 0.0;
    for k in keep {
        if k.start > at {
            out.push(dead(at, k.start));
        }
        at = at.max(k.end);
    }
    if duration > at {
        out.push(dead(at, duration));
    }
    out
}

impl Core {
    /// Video analysis on this Mac: available, or why not. Never an error.
    pub fn video_feature(&self) -> Feature {
        self.gear.intel.video_feature()
    }

    /// The analyzer's answer for `file`, whose content is `key`: from the cache, unless
    /// `again`, else from the helper (and then cached). None when video analysis is
    /// unavailable, with the reason.
    fn watch_file(
        &self,
        file: &Path,
        key: &str,
        seconds: f64,
        again: bool,
    ) -> Result<std::result::Result<(Watched, bool), String>> {
        if !again {
            if let Some(w) = highlights::cached(&self.cache, key) {
                return Ok(Ok((w, true)));
            }
        }
        let a = self.gear.intel.video(file, true, true, seconds)?;
        if let Some(why) = a.unavailable {
            return Ok(Err(why));
        }
        let w = a.watched.unwrap_or_default();
        if let Some(e) = &w.error {
            bail!("{e}");
        }
        highlights::store(&self.cache, key, &w);
        Ok(Ok((w, false)))
    }

    fn video_progress(&self, done: usize, total: usize) {
        self.hooks.event(Event::LibraryTask(LibraryTask {
            task: Task::Video,
            done,
            total,
        }));
    }

    /// Analyses the session's clips that have no analysis yet, in the background after
    /// `analyse`: highlights become the moments of a clip with no radio-log moments, and the
    /// key frame its thumbnail. Does nothing when the `videoAnalysis` setting is off or the
    /// feature is unavailable. Stops when the session changes or another step (an export)
    /// starts. Returns the clips it analysed.
    pub fn analyse_video(&self) -> usize {
        if !self.defaults().video_analysis || !self.video_feature().available {
            return 0;
        }
        let Some(s) = self.session() else { return 0 };
        let todo: Vec<crate::pipeline::Clip> = s
            .clips
            .iter()
            .filter(|c| c.probe.is_some() && c.part_of.is_none() && c.video.is_none())
            // A joined recording's source is an ffconcat list the analyzer cannot read; the
            // library job analyses its output.
            .filter(|c| !c.join.as_ref().is_some_and(|j| j.on))
            .filter(|c| !c.key.is_empty())
            .cloned()
            .collect();
        if todo.is_empty() {
            return 0;
        }
        let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
        let tools = media::find_tools().ok();
        let total = todo.len();
        let mut done = 0;
        for (i, c) in todo.iter().enumerate() {
            let same = self
                .session()
                .is_some_and(|now| now.staging == s.staging && now.clips.len() == s.clips.len());
            if !same || self.busy.load(Ordering::SeqCst) || !self.defaults().video_analysis {
                break;
            }
            self.video_progress(i, total);
            let Some(src) = c.source() else { continue };
            let w = match self.watch_file(src, &c.key, c.duration, false) {
                Ok(Ok((w, _))) => w,
                Ok(Err(_)) => break,
                Err(e) => {
                    eprintln!("quadcam: video analysis of {} failed: {e:#}", c.name);
                    continue;
                }
            };
            let dead = c
                .signal
                .as_ref()
                .map(|s| s.dead_air.clone())
                .unwrap_or_default();
            let scan = highlights::scan(&w, &dead, c.duration);
            let thumb = match (&tools, scan.key_frame, &c.thumb) {
                (Some(t), Some(k), Some(old)) => {
                    let dst = old.with_extension("key.jpg");
                    media::frame_at(t, src, k, 240, &dst).ok().map(|_| dst)
                }
                _ => None,
            };
            let mut guard = self.session.lock().unwrap();
            let Some(now) = guard.as_mut().filter(|n| n.staging == s.staging) else {
                break;
            };
            let mut next = now.clone();
            next.set_video(c.id, scan, thumb);
            *now = next.clone();
            drop(guard);
            if self.commit(Some(next)).is_ok() {
                done += 1;
            }
        }
        self.video_progress(total, total);
        done
    }

    /// The library picture of a clip's key frame.
    pub(super) fn cover_path(&self, id: &str) -> PathBuf {
        self.cache
            .join("library")
            .join(format!("{}.key.jpg", lib::id_file(id)))
    }

    /// Makes the key-frame picture of a library clip when it has a key frame and no picture
    /// yet (or `again`), and removes a stale one when it has none.
    pub(super) fn make_cover(&self, tools: &media::Tools, root: &Path, c: &LibClip, again: bool) {
        let dst = self.cover_path(&c.id);
        match c.video.as_ref().and_then(|v| v.key_frame) {
            Some(k) if again || !dst.is_file() => {
                if let Err(e) = media::frame_at(tools, &root.join(&c.path), k, COVER_W, &dst) {
                    eprintln!(
                        "quadcam: no key frame picture for {}: {e:#}",
                        c.path.display()
                    );
                }
            }
            None => {
                let _ = std::fs::remove_file(&dst);
            }
            _ => {}
        }
    }

    /// Finds highlights and the key frame of library clips with macOS video analysis, and
    /// writes them into the clip files (`app.quadcam.video`). `ids` empty means every clip;
    /// without `again`, clips analysed already are skipped and cached answers are used. A clip
    /// that fails is reported and the rest go on.
    pub fn library_analyze_video(&self, p: &VideoParams) -> Result<VideoReport> {
        let (root, mut clips) =
            self.with_index(|root, ix| Ok(((root.to_path_buf(), ix.clips.clone()), false)))?;
        clips.retain(|c| c.cut_of.is_none());
        if !p.ids.is_empty() {
            for id in &p.ids {
                if !clips.iter().any(|c| &c.id == id || c.aliases.contains(id)) {
                    bail!("No clip {id} in the library.");
                }
            }
            clips.retain(|c| p.ids.iter().any(|id| &c.id == id || c.aliases.contains(id)));
        }
        if !p.again {
            clips.retain(|c| c.video.is_none());
        }
        let feature = self.video_feature();
        if !feature.available {
            return Ok(VideoReport {
                unavailable: Some(
                    feature
                        .reason
                        .unwrap_or_else(|| "Video analysis is unavailable.".into()),
                ),
                clips: Vec::new(),
            });
        }
        let mut report = VideoReport::default();
        if clips.is_empty() {
            return Ok(report);
        }
        let tools = media::find_tools()?;
        let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
        let total = clips.len();
        for (i, c) in clips.iter().enumerate() {
            self.video_progress(i, total);
            let file = root.join(&c.path);
            let mut row = VideoClip {
                id: c.id.clone(),
                path: c.path.clone(),
                highlights: 0,
                key_frame: None,
                cached: false,
                error: None,
            };
            match self.watch_file(&file, &c.id, c.duration, p.again) {
                Ok(Ok((w, cached))) => {
                    let scan =
                        highlights::scan(&w, &dead_from_keep(&c.keep, c.duration), c.duration);
                    row.cached = cached;
                    row.highlights = scan.highlights.len();
                    row.key_frame = scan.key_frame;
                    match self.write_video(&tools, &root, c, &scan) {
                        Ok(()) => {}
                        Err(e) => row.error = Some(format!("{e:#}")),
                    }
                }
                Ok(Err(why)) => {
                    report.unavailable = Some(why);
                    break;
                }
                Err(e) => row.error = Some(format!("{e:#}")),
            }
            report.clips.push(row);
        }
        self.video_progress(total, total);
        Ok(report)
    }

    /// Writes a scan into a library clip's file and index entry, and makes its picture.
    fn write_video(
        &self,
        tools: &media::Tools,
        root: &Path,
        c: &LibClip,
        scan: &VideoScan,
    ) -> Result<()> {
        lib::write_keys(
            &root.join(&c.path),
            &[(lib::KEY_VIDEO, highlights::value(scan))],
        )?;
        self.reread_clips(std::slice::from_ref(&c.id))?;
        let (_, fresh) = self.clip(&c.id)?;
        self.make_cover(tools, root, &fresh, true);
        self.hooks.library_changed();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dead_air_is_what_the_keep_ranges_leave() {
        let keep = [
            Span {
                start: 5.0,
                end: 20.0,
            },
            Span {
                start: 24.0,
                end: 30.0,
            },
        ];
        let d = dead_from_keep(&keep, 40.0);
        let r: Vec<(f64, f64)> = d.iter().map(|m| (m.start, m.end)).collect();
        assert_eq!(r, vec![(0.0, 5.0), (20.0, 24.0), (30.0, 40.0)]);
        assert!(dead_from_keep(&[], 40.0).is_empty());
    }
}
