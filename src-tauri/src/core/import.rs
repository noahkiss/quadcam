//! `Core`'s import half: stage a card or folder, analyse, date, patch the plans, convert and
//! verify, delete the imported clips from the card when the setting says so, then make the card safe to remove and
//! format the card. Also the "N new" count for a card.

use super::Core;
use super::{
    ClipDeletion, DeletionState, FormatPlan, FormatRequest, ImportOptions, ImportOutcome,
    LogChoice, VerifyReport,
};
use crate::api::{Event, ImportProgress, ImportResult, Phase, Progress};
use crate::disk;
use crate::library as lib;
use crate::media;
use crate::pipeline::{Clip, ClipResult, ImportSettings, Outcome};
use crate::session::{self, Editor, PlanPatch, Session};
use anyhow::{anyhow, bail, Context, Result};
use chrono::NaiveDate;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct CardStatus {
    pub mount: PathBuf,
    pub clips: usize,
    /// Clips whose content is not in the library yet.
    pub new: usize,
    pub free: Option<u64>,
    pub size: u64,
}

impl Core {
    /// Copies the clips off `source` (a card mount or folder). With no source, the first
    /// detected card.
    pub fn stage(&self, source: Option<&Path>) -> Result<Session> {
        self.stage_with(source, None)
    }

    /// `stage`, with this run's choice to join split recordings (None: the setting).
    pub fn stage_with(&self, source: Option<&Path>, join: Option<bool>) -> Result<Session> {
        self.stage_device(source, None, join)
    }

    /// `stage_with`, and a card named by device id when `source` is empty. Mount, work,
    /// unmount: an unmounted card that is still plugged in is mounted for the stage; the
    /// end of the import unmounts it again.
    pub fn stage_device(
        &self,
        source: Option<&Path>,
        device: Option<&str>,
        join: Option<bool>,
    ) -> Result<Session> {
        let _b = self.claim()?;
        let device = device.map(str::trim).filter(|d| !d.is_empty());
        let source = match (source, device) {
            (Some(p), _) => p.to_path_buf(),
            (None, Some(d)) => self.mount_source(d)?,
            (None, None) => match disk::list_volumes().into_iter().find(|v| v.is_card) {
                Some(v) => v.mount,
                None => self.mount_only_card()?.ok_or_else(|| {
                    anyhow!("No card detected. Insert the card or pass a folder.")
                })?,
            },
        };
        let hooks = self.hooks.clone();
        let s = Session::stage(
            &source,
            &self.cache.join("staging"),
            &mut |index, total, done, size| {
                hooks.event(Event::Progress(Progress {
                    phase: Phase::Stage,
                    index,
                    total,
                    done,
                    size,
                }));
            },
        )?;
        let s = Session { join, ..s };
        self.commit(Some(s.clone()))?;
        Ok(s)
    }

    /// Copies the clips among `files` (dropped on the window) to staging; a new session.
    pub fn stage_files(&self, files: &[PathBuf]) -> Result<Session> {
        let _b = self.claim()?;
        let hooks = self.hooks.clone();
        let s = Session::stage_files(
            files,
            &self.cache.join("staging"),
            &mut |index, total, done, size| {
                hooks.event(Event::Progress(Progress {
                    phase: Phase::Stage,
                    index,
                    total,
                    done,
                    size,
                }));
            },
        )?;
        self.commit(Some(s.clone()))?;
        Ok(s)
    }

    /// What the GUI does with things dropped on its window: one folder loads like a card;
    /// files load the DVR clips among them.
    pub fn load_dropped(&self, paths: &[PathBuf]) -> Result<Session> {
        media::find_tools()?;
        match paths {
            [one] if one.is_dir() => return self.load(Some(one)),
            _ if paths.iter().any(|p| p.is_dir()) => {
                bail!("Drop one folder, or clip files, not both.")
            }
            _ => self.stage_files(paths)?,
        };
        self.analyse()?;
        self.plan_dates(LogChoice::Keep, None)
    }

    pub fn analyse(&self) -> Result<Session> {
        let _b = self.claim()?;
        let tools = media::find_tools()?;
        let mut s = self.current()?;
        let hooks = self.hooks.clone();
        let join = s.join.unwrap_or(self.defaults().join_split_recordings);
        s.analyse(
            &tools,
            &self.cache.join("thumbs"),
            join,
            &mut |index, total| {
                hooks.event(Event::Progress(Progress {
                    phase: Phase::Analyse,
                    index,
                    total,
                    done: 0,
                    size: 0,
                }));
            },
        )?;
        self.commit(Some(s.clone()))?;
        drop(_b);
        self.hooks.analysed();
        Ok(s)
    }

    /// Stage, analyse and plan dates in one go, the way the GUI does on card insert.
    pub fn load(&self, source: Option<&Path>) -> Result<Session> {
        self.load_with(source, None)
    }

    /// `load`, with this run's choice to join split recordings (None: the setting).
    pub fn load_with(&self, source: Option<&Path>, join: Option<bool>) -> Result<Session> {
        self.load_device(source, None, join)
    }

    /// `load_with`, and a card named by device id when `source` is empty.
    pub fn load_device(
        &self,
        source: Option<&Path>,
        device: Option<&str>,
        join: Option<bool>,
    ) -> Result<Session> {
        media::find_tools()?;
        self.stage_device(source, device, join)?;
        self.analyse()?;
        self.plan_dates(LogChoice::Keep, None)
    }

    pub fn plan_dates(&self, logs: LogChoice, day: Option<NaiveDate>) -> Result<Session> {
        let mut s = self.current()?;
        let defaults = self.defaults();
        let dir = match logs {
            LogChoice::Keep => s.log_dir.clone().or(defaults.log_dir.clone()),
            LogChoice::None => None,
            LogChoice::Dir(d) => Some(d),
        };
        s.plan_dates(
            dir.as_deref(),
            day,
            &defaults.tunables,
            chrono::Local::now().date_naive(),
            &defaults.profiles,
        );
        self.commit(Some(s.clone()))?;
        Ok(s)
    }

    pub fn patch(&self, patches: &[PlanPatch], editor: Editor) -> Result<Session> {
        for p in patches {
            if let Some(crate::trim::CutChange::Confirm { files }) = self.patch_cuts_check(p)? {
                bail!(
                    "clip {}: these cuts were exported already: {}. Say what happens to the files with removed_cuts \"keep\" (they stay as clips of their own) or \"trash\".",
                    p.id,
                    files
                        .iter()
                        .map(|f| f.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
        self.patch_inner(patches, editor)?;
        self.current()
    }

    /// Applies patches whose exported-cut decisions are settled. Returns the kept and the
    /// trashed cut files.
    pub(super) fn patch_inner(
        &self,
        patches: &[PlanPatch],
        editor: Editor,
    ) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
        let before = self.current()?;
        // A saved place name becomes its location here, where the places are known.
        let places = self.defaults().places;
        let mut patches = patches.to_vec();
        for p in &mut patches {
            if let Some(name) = p.place.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                let pl = places
                    .iter()
                    .find(|x| x.name.eq_ignore_ascii_case(name))
                    .with_context(|| {
                        format!(
                            "no saved place {name:?}; saved places: {}",
                            places
                                .iter()
                                .map(|x| x.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })?;
                p.location = Some(crate::metadata::Location {
                    lat: pl.lat,
                    lon: pl.lon,
                    name: Some(pl.name.clone()),
                });
            }
        }
        let patches = &patches[..];
        let mut guard = self.session.lock().unwrap();
        let s = guard
            .as_mut()
            .ok_or_else(|| anyhow!("No clips loaded. Load a card or folder first."))?;
        let mut next = s.clone();
        next.patch(patches, editor)?;
        // A recording joined or parted is matched to the radio logs again, as before.
        if patches.iter().any(|p| p.joined.is_some()) {
            let d = self.defaults();
            let (dir, day) = (next.log_dir.clone(), next.log_day);
            next.plan_dates(
                dir.as_deref(),
                day,
                &d.tunables,
                chrono::Local::now().date_naive(),
                &d.profiles,
            );
        }
        // Exported cuts the new lists drop: keep or trash their files, forget their results.
        let mut removed = Vec::new();
        for p in patches {
            let Some(cuts) = &p.cuts else { continue };
            let gone =
                crate::trim::removed_exported(&crate::cuts::session_exported(&before, p.id), cuts);
            if gone.is_empty() {
                continue;
            }
            if let Some(r) = next
                .results
                .iter_mut()
                .rev()
                .find(|r| r.id == p.id && r.outcome == crate::pipeline::Outcome::Verified)
            {
                r.cuts.retain(|c| {
                    !gone
                        .iter()
                        .any(|g| c.output.as_deref() == Some(g.path.as_path()))
                });
            }
            removed.push((gone, p.removed_cuts));
        }
        *s = next.clone();
        drop(guard);
        self.commit(Some(next.clone()))?;
        let mut kept = Vec::new();
        let mut trashed = Vec::new();
        for (gone, decision) in removed {
            let (k, t) = self.drop_exported(&gone, decision)?;
            kept.extend(k);
            trashed.extend(t);
        }
        if !kept.is_empty() || !trashed.is_empty() {
            if let Some(root) = next.output_dir.clone() {
                let _ = self.library_add_results(&root, &next, "");
            }
        }
        Ok((kept, trashed))
    }

    pub fn import_settings(&self, o: &ImportOptions) -> Result<ImportSettings> {
        let d = self.defaults();
        Ok(ImportSettings {
            output_dir: o
                .output_dir
                .clone()
                .or(d.output_dir)
                .context("No output folder set.")?,
            format: o.format.unwrap_or(d.format),
            encoder: o.encoder.unwrap_or(d.encoder),
            keep_originals: o.keep_originals.unwrap_or(d.keep_originals),
            add_time: o.add_time.unwrap_or(d.add_time),
            default_name: d.default_name,
            places: d.places,
            profiles: d.profiles,
            default_profile: d.default_profile,
            layout: d.layout,
            place_folders: d.place_folders,
            import_id: chrono::Local::now().format("%Y%m%d-%H%M%S").to_string(),
            name_date_format: d.name_date_format,
        })
    }

    /// Converts, verifies and (optionally) adds to Photos, using the session's plans.
    pub fn import(&self, o: &ImportOptions) -> Result<ImportOutcome> {
        let settings = self.import_settings(o)?;
        let tools = media::find_tools()?;
        let s = {
            let _b = self.claim()?;
            let s = self.current()?;
            let hooks = self.hooks.clone();
            let hooks2 = self.hooks.clone();
            let results = session::run_import(
                &tools,
                &s,
                &settings,
                &mut |id, seconds, duration| {
                    hooks.event(Event::ImportProgress(ImportProgress {
                        id,
                        seconds,
                        duration,
                    }))
                },
                &mut |r: &ClipResult| {
                    hooks2.event(Event::ImportResult(Box::new(ImportResult(r.clone()))))
                },
            )?;
            // Plans may have changed while converting; keep them, replace results only.
            let mut latest = self.current()?;
            latest.results = results;
            latest.output_dir = Some(settings.output_dir.clone());
            self.commit(Some(latest.clone()))?;
            // The library learns the new files; a failure here never fails the import.
            if let Err(e) =
                self.library_add_results(&settings.output_dir, &latest, &settings.import_id)
            {
                eprintln!("quadcam: library index not updated: {e:#}");
            }
            latest
        };
        // The setting is the consent; a run can only turn deletion off. Mount, work,
        // unmount: a card released by an earlier export is mounted again to delete from it.
        let clip_deletion =
            (self.defaults().delete_clips_after_import && !o.keep_clips).then(|| {
                match self.ensure_session_card() {
                    Ok(_) => {
                        let s = self.session().unwrap_or_else(|| s.clone());
                        delete_imported_clips(&tools, &s)
                    }
                    // The outcome says why every clip stays, not "no longer on the card".
                    Err(e) => {
                        let why = format!("the card did not mount ({e:#})");
                        if let Ok(mut latest) = self.current() {
                            latest
                                .warnings
                                .push(format!("No clips were deleted: {why}."));
                            let _ = self.commit(Some(latest));
                        }
                        kept_clips(&s, &why)
                    }
                }
            });
        if let Some(d) = &clip_deletion {
            let deleted = d
                .iter()
                .filter(|x| x.state == DeletionState::Deleted)
                .count();
            if deleted > 0 {
                let mut latest = self.current()?;
                latest.warnings.push(format!(
                    "Deleted {deleted} imported clip{} from {}.",
                    if deleted == 1 { "" } else { "s" },
                    latest.source.display()
                ));
                self.commit(Some(latest))?;
            }
        }
        let photos = o.add_to_photos.then(|| {
            let album = o.album.clone().or(Some(self.defaults().photos_album));
            self.add_to_photos(None, album)
                .map_err(|e| format!("{e:#}"))
        });
        // The work on the card is done: unmount it. Format and "Safe to remove" mount it
        // again if they need it.
        let card = self.release_session_card();
        if let Some(r) = card.as_ref().filter(|r| !r.released) {
            if let Ok(mut latest) = self.current() {
                latest.warnings.push(r.message.clone());
                let _ = self.commit(Some(latest));
            }
        }
        let summary = self.session().unwrap_or(s).summary();
        Ok(ImportOutcome {
            summary,
            photos,
            clip_deletion,
            card,
        })
    }

    /// Re-runs the verify checks on verified outputs (all when `ids` is None): frame count,
    /// duration, streams and the metadata that was written.
    pub fn verify(&self, ids: Option<Vec<usize>>) -> Result<Vec<VerifyReport>> {
        let tools = media::find_tools()?;
        let s = self.current()?;
        let mut out = Vec::new();
        for r in s
            .results
            .iter()
            .filter(|r| r.outcome == crate::pipeline::Outcome::Verified)
        {
            if ids.as_ref().is_some_and(|ids| !ids.contains(&r.id)) {
                continue;
            }
            let (Some(output), Some(meta)) = (&r.output, &r.meta) else {
                continue;
            };
            let clip = s
                .clips
                .iter()
                .find(|c| c.id == r.id)
                .context("result for an unknown clip")?;
            let check = clip
                .probe
                .as_ref()
                .context("source was never probed")
                .and_then(|p| media::verify(&tools, p, output, meta))
                .and_then(|_| media::verify_qt(&tools, output, &r.qt));
            out.push(VerifyReport {
                id: r.id,
                output: output.clone(),
                ok: check.is_ok(),
                error: check.err().map(|e| format!("{e:#}")),
            });
        }
        Ok(out)
    }

    /// Makes `target` (a mount point or `/dev/diskN`), or the session's card, safe to remove
    /// (`disk::safe_remove`: unmount a card, eject anything else).
    pub fn eject(&self, target: Option<&str>) -> Result<()> {
        let target = match target {
            Some(t) => t.to_string(),
            None => {
                let s = self.current()?;
                let Some(card) = &s.card else {
                    bail!("This session came from a folder; there is no card to eject.");
                };
                // By disk, not mount point: the end of an import may have unmounted the
                // card already, and unmounting a disk again is harmless. Only the disk the
                // card's volume UUID finds, and only when it is the saved one.
                let Some(whole) = super::session_card::card_disk(card) else {
                    bail!(
                        "The session's card is not plugged in, or another disk took its place. Nothing was ejected."
                    );
                };
                format!("/dev/{whole}")
            }
        };
        disk::safe_remove(&target)
    }

    /// Everything the confirm dialog names. Runs every guard; an Err means format stays locked.
    pub fn format_plan(&self, label: Option<&str>) -> Result<FormatPlan> {
        // Mount, work, unmount: a plan only reads, so a card it mounted is released again.
        let was_mounted = self.session_card_mounted();
        let plan = self.format_plan_with(label);
        if !was_mounted {
            let _ = self.release_session_card();
        }
        plan
    }

    /// `format_plan`, leaving a card it had to mount mounted (the erase follows).
    fn format_plan_with(&self, label: Option<&str>) -> Result<FormatPlan> {
        let s = self.current()?;
        // A source that does not offer formatting refuses here, before every other guard.
        if let Some(kind) = std::iter::once(s.kind)
            .chain(s.clips.iter().map(|c| c.kind))
            .find(|k| !crate::sources::get(*k).card_policy().format_offered)
        {
            bail!(
                "Refused: QuadCam does not format {} cards after an import. Card prep can erase a removable card once every clip is in the library.",
                kind.label()
            );
        }
        s.format_ready()?;
        let policy = crate::sources::get(s.kind).card_policy();
        let card = s
            .card
            .clone()
            .context("Clips came from a folder, not a card.")?;
        let source = self
            .ensure_session_card()?
            .context("Clips came from a folder, not a card.")?;
        let whole = disk::verify_card_for_format(&card, &source)?;
        Ok(FormatPlan {
            disk: card.whole_disk.clone(),
            device: format!("/dev/{}", card.whole_disk),
            volume_uuid: card.volume_uuid.clone().unwrap_or_default(),
            volume_name: card.volume_name.clone().unwrap_or_default(),
            size: whole.total_size,
            media_name: whole.media_name.clone().unwrap_or_default(),
            clip_count: s.clips.len(),
            label: disk::fat_label(
                label
                    .filter(|l| !l.trim().is_empty())
                    .unwrap_or(&self.defaults().format_label),
            )?,
            filesystem: policy.filesystem.to_string(),
            warnings: disk::format_advice(whole.total_size, &policy),
        })
    }

    /// Erases the card. `req` must name the card's whole-disk device and volume UUID and set
    /// `confirm`. Unless the GUI's own button started it, the GUI (when running) must also
    /// get a click on Erase. `disk::format_card` then checks every guard once more.
    pub fn format(&self, req: &FormatRequest, from_gui_button: bool) -> Result<FormatPlan> {
        if !req.confirm {
            bail!("Refused: format needs an explicit confirm.");
        }
        // Mount, work, unmount: a card mounted for the erase is released again when the
        // erase does not happen (a successful erase unmounts it itself).
        let was_mounted = self.session_card_mounted();
        let r = self.format_inner(req, from_gui_button);
        if r.is_err() && !was_mounted {
            let _ = self.release_session_card();
        }
        r
    }

    fn format_inner(&self, req: &FormatRequest, from_gui_button: bool) -> Result<FormatPlan> {
        let plan = self.format_plan_with(req.label.as_deref())?;
        let device = req.device.trim();
        if device != plan.device {
            bail!(
                "Refused: --device {device} is not the card's whole disk {}.",
                plan.device
            );
        }
        if plan.volume_uuid.is_empty()
            || !req
                .volume_uuid
                .trim()
                .eq_ignore_ascii_case(&plan.volume_uuid)
        {
            bail!(
                "Refused: volume UUID {} does not match the card ({}).",
                req.volume_uuid.trim(),
                plan.volume_uuid
            );
        }
        if !from_gui_button {
            self.hooks.confirm_format(&plan)?;
        }
        let _b = self.claim()?;
        let s = self.current()?;
        let card = s
            .card
            .clone()
            .context("Clips came from a folder, not a card.")?;
        let policy = crate::sources::get(s.kind).card_policy();
        let eject_error = disk::format_card(
            &card,
            &s.source,
            &plan.label,
            &policy,
            disk::EraseBy::Import,
            &|| self.current()?.format_ready(),
        )?;
        let mut latest = self.current()?;
        latest.card = None;
        match &eject_error {
            None => latest.warnings.push(format!(
                "Card {} was erased and is safe to remove.",
                plan.device
            )),
            Some(e) => latest.warnings.push(format!(
                "Card {} was erased, but did not unmount: {e}",
                plan.device
            )),
        }
        self.commit(Some(latest))?;
        match eject_error {
            None => Ok(plan),
            Some(e) => bail!(
                "The card was erased, but it did not unmount ({e}). Eject it in Finder before you pull it."
            ),
        }
    }

    /// Clips on a card, and how many of them are not in the library yet (by content).
    pub fn card_status(&self, mount: &Path) -> Result<CardStatus> {
        let found = crate::sources::for_root(mount).list(mount);
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
                // A clip is known by its current fingerprint or by its 0.4 one.
                crate::identity::fingerprints(&f.path)
                    .map(|(now, old)| {
                        ![now, old]
                            .iter()
                            .any(|k| known.contains(k) || staged.contains(k))
                    })
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
}

/// "Delete clips after import": deletes the card or folder file of every clip whose output
/// verified, and reports every clip. Only the clip files go; sidecars, other files and
/// folders stay. Each clip's guards run in `delete_clip_file`, right before its unlink.
pub(crate) fn delete_imported_clips(tools: &media::Tools, s: &Session) -> Vec<ClipDeletion> {
    s.clips
        .iter()
        .map(|c| {
            let reason = delete_clip_file(tools, s, c).err();
            ClipDeletion {
                id: c.id,
                path: c.card_path.clone(),
                state: if reason.is_none() {
                    DeletionState::Deleted
                } else {
                    DeletionState::Kept
                },
                reason,
            }
        })
        .collect()
}

/// Every clip stays, for one reason.
fn kept_clips(s: &Session, why: &str) -> Vec<ClipDeletion> {
    s.clips
        .iter()
        .map(|c| ClipDeletion {
            id: c.id,
            path: c.card_path.clone(),
            state: DeletionState::Kept,
            reason: Some(why.to_string()),
        })
        .collect()
}

/// Deletes one clip's file from the card or folder it came from, or says why it stays.
/// Every guard runs here, immediately before the unlink. Do not move one out of this path.
fn delete_clip_file(tools: &media::Tools, s: &Session, clip: &Clip) -> Result<(), String> {
    let source = crate::sources::get(clip.kind);
    if !source.card_policy().delete_clips_offered {
        return Err(format!(
            "QuadCam does not delete {} clips.",
            clip.kind.label()
        ));
    }
    if let Some(e) = &clip.stage_error {
        return Err(format!("it did not copy off the card ({e})"));
    }
    // The clip's latest result must be a verified export. A part of a joined recording
    // counts only when the joined output verified.
    let id = clip.part_of.unwrap_or(clip.id);
    let r = s
        .results
        .iter()
        .rev()
        .find(|r| r.id == id)
        .ok_or("it was not imported")?;
    match r.outcome {
        Outcome::Verified => {}
        Outcome::Skipped => return Err("it was skipped".into()),
        Outcome::Failed => return Err("it failed to import".into()),
    }
    // Check the output again, the same way `verify` does.
    let (Some(output), Some(meta)) = (&r.output, &r.meta) else {
        return Err("its output was not recorded".into());
    };
    let head = s
        .clips
        .iter()
        .find(|c| c.id == id)
        .ok_or("its recording is not in the session")?;
    let probe = head.probe.as_ref().ok_or("the clip was never probed")?;
    media::verify(tools, probe, output, meta)
        .and_then(|_| media::verify_qt(tools, output, &r.qt))
        .map_err(|e| format!("its output did not verify again: {e:#}"))?;
    // The path must be a plain file inside the card or folder the clips came from.
    let path = &clip.card_path;
    let lmeta =
        std::fs::symlink_metadata(path).map_err(|_| "it is no longer on the card".to_string())?;
    if !lmeta.file_type().is_file() {
        return Err("it is not a plain file".into());
    }
    let root = s
        .source
        .canonicalize()
        .map_err(|e| format!("the card or folder is not readable: {e}"))?;
    let real = path
        .canonicalize()
        .map_err(|e| format!("the file is not readable: {e}"))?;
    if real == root || !real.starts_with(&root) {
        return Err(format!("it is outside {}", s.source.display()));
    }
    // It must still be one of the source's clip files, and the very file that was copied.
    if !source
        .pick(std::slice::from_ref(&real))
        .iter()
        .any(|f| f.path == real)
    {
        return Err("it is not a clip file".into());
    }
    if lmeta.len() != clip.size {
        return Err("it changed since it was copied".into());
    }
    if clip.key.is_empty()
        || crate::identity::fingerprint(&real).ok().as_deref() != Some(clip.key.as_str())
    {
        return Err("it changed since it was copied".into());
    }
    std::fs::remove_file(&real).map_err(|e| format!("delete failed: {e}"))
}
