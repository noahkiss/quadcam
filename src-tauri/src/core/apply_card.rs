//! `Core`'s card apply and the mount cycle (design 8, 7.1).
//!
//! - **Mount cycle.** (Any card: a radio's, a DVR card, goggles. The import session's card follows
//!   the same cycle in `session_card.rs`.) A card stays unmounted between operations. A job that needs the card
//!   mounts it (`diskutil mountDisk`, `Env.mount`), works, and unmounts it again
//!   (`gear_finish_card`: "safe to unplug" only after the unmount worked). A plan mounts and
//!   unmounts quietly. The person's **Mount** (`gear_card_mount`) mounts a card to browse it
//!   and unmounts it on **Done** (`gear_card_unmount`) or after `MOUNT_MINUTES`
//!   (`gear_mount_tick`, run by the app's poll).
//! - **Where a card is.** A mounted card shows in `gear_connected`. An unmounted one is found
//!   in `gear_released`: the cards this process unmounted, and the app's unmounted list
//!   (`gear_note_unmounted`), kept while the system still shows the disk.
//! - **Apply.** Plan (mount, read, checks), confirm, back up (a full snapshot,
//!   `BeforeApply`, always kept), write file by file, verify on fresh reads, roll back every
//!   file on a failed write or read-back, take an `AfterApply` snapshot, record, unmount.

use super::apply::refusal;
use super::{link_handle, Core};
use crate::gear::apply::card::{self as cardplan, CardCand, CardFacts, CardPlanned, CardWork};
use crate::gear::apply::{first_refusal, ApplyReport, ApplyRequest, StepReport, StepState};
use crate::gear::backup::{blob_of, BackupProgress, TakeOptions};
use crate::gear::edgetx::card::{self as engine, WriteOptions, WriteProgress};
use crate::gear::events::still_present;
use crate::gear::health::{CheckState, HealthLog};
use crate::gear::model::{
    ChangeStatus, Connected, DeviceKind, Edit, Link, Refusal, RefusalCode, StagedChange, Trigger,
};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::time::{Duration, Instant};

/// How long a card the person mounted stays mounted.
pub const MOUNT_MINUTES: u32 = 10;

/// `gear_card_mount` and `gear_card_unmount`: the card's device id.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct CardMountParams {
    pub device: String,
    /// Minutes to keep it mounted (1-60); `MOUNT_MINUTES` when left out.
    #[serde(default)]
    pub minutes: Option<u32>,
}

/// A card mounted for the person.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct CardMounted {
    pub device: String,
    pub mount: std::path::PathBuf,
    /// QuadCam unmounts it at this time unless the person is done sooner.
    pub until: DateTime<Utc>,
}

/// A card found mounted, and whether this call mounted it.
pub(super) struct Located {
    pub connected: Connected,
    pub root: std::path::PathBuf,
    pub mounted_here: bool,
}

impl Core {
    /// Remembers the cards that are unmounted but still plugged in (the app's tracker
    /// list), so a later plan or apply can mount them again.
    pub fn gear_note_unmounted(&self, list: &[Connected]) {
        let mut m = self.gear_released.lock().unwrap();
        for c in list {
            if let Some(id) = &c.id {
                m.insert(id.clone(), c.clone());
            }
        }
    }

    pub(super) fn card_note_released(&self, c: &Connected) {
        if let (Some(id), Link::Volume { .. }) = (&c.id, &c.link) {
            self.gear_released
                .lock()
                .unwrap()
                .insert(id.clone(), c.clone());
        }
    }

    /// The radio cards mounted now.
    fn card_cands(&self) -> Result<Vec<CardCand>> {
        self.volume_cands(true)
    }

    /// The cards mounted now: radio cards only, or a volume of any kind (a DVR card,
    /// goggles, a radio).
    fn volume_cands(&self, radio_only: bool) -> Result<Vec<CardCand>> {
        Ok(self
            .gear_connected()?
            .into_iter()
            .filter(|c| !radio_only || c.kind == DeviceKind::Radio)
            .filter_map(|c| match &c.link {
                Link::Volume { mount, .. } => Some(CardCand {
                    root: mount.clone(),
                    connected: c.clone(),
                }),
                _ => None,
            })
            .collect())
    }

    /// The card with this device id. Mounted: as is. Unmounted but still plugged in:
    /// mounted now (`mount` true). Else None.
    pub(super) fn locate_card(&self, device: &str, mount: bool) -> Result<Option<Located>> {
        self.locate_volume(device, mount, true)
    }

    /// `locate_card` for a volume of any kind (`radio_only` false): the one mount cycle
    /// every card job shares. Mounted: as is. Unmounted but still plugged in: mounted now
    /// (`mount` true), and `mounted_here` tells the caller to unmount it when done.
    pub(super) fn locate_volume(
        &self,
        device: &str,
        mount: bool,
        radio_only: bool,
    ) -> Result<Option<Located>> {
        let find = |core: &Core| -> Result<Option<CardCand>> {
            Ok(core
                .volume_cands(radio_only)?
                .into_iter()
                .find(|c| c.connected.id.as_deref() == Some(device)))
        };
        if let Some(c) = find(self)? {
            return Ok(Some(Located {
                connected: c.connected,
                root: c.root,
                mounted_here: false,
            }));
        }
        if !mount {
            return Ok(None);
        }
        let Some(rel) = self.gear_released.lock().unwrap().get(device).cloned() else {
            return Ok(None);
        };
        let Link::Volume {
            whole_disk: Some(disk),
            ..
        } = &rel.link
        else {
            return Ok(None);
        };
        if !still_present(&rel, &(self.gear.presence)()) {
            self.gear_released.lock().unwrap().remove(device);
            return Ok(None);
        }
        {
            let _hold = self.gear_hold(&link_handle(&rel.link));
            (self.gear.mount)(disk)
                .with_context(|| format!("Mounting {disk} for {}", connected_label(&rel)))?;
            for i in 0..40 {
                if let Some(c) = find(self)? {
                    self.gear_released.lock().unwrap().remove(device);
                    return Ok(Some(Located {
                        connected: c.connected,
                        root: c.root,
                        mounted_here: true,
                    }));
                }
                if i < 39 {
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        }
        bail!(
            "{} did not mount. Take the card out and put it back, or plug the radio in again.",
            connected_label(&rel)
        )
    }

    /// Unmounts a card this call mounted, with no cue (a plan or a refusal is not a job).
    pub(super) fn card_quiet_unmount(&self, l: &Located) {
        if !l.mounted_here {
            return;
        }
        if let Link::Volume {
            whole_disk: Some(disk),
            ..
        } = &l.connected.link
        {
            let _hold = self.gear_hold(&link_handle(&l.connected.link));
            if (self.gear.unmount)(disk).is_ok() {
                self.card_note_released(&l.connected);
            }
        }
    }

    /// The cards this process unmounted (or the app listed as unmounted) that are still
    /// plugged in, and not mounted now. `radio_only` keeps the radio cards.
    pub(super) fn unmounted_cards(&self, radio_only: bool) -> Vec<Connected> {
        let mounted: Vec<String> = self
            .gear_connected()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|c| c.id)
            .collect();
        let presence = (self.gear.presence)();
        let mut out: Vec<Connected> = self
            .gear_released
            .lock()
            .unwrap()
            .values()
            .filter(|c| !radio_only || c.kind == DeviceKind::Radio)
            .filter(|c| c.id.as_ref().is_some_and(|id| !mounted.contains(id)))
            .filter(|c| still_present(c, &presence))
            .cloned()
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// Mounts `c` for a job when it was picked from `unmounted_cards`; a card that is
    /// mounted already comes back as is. The job's own finish unmounts it again.
    pub(super) fn mount_picked(&self, c: Connected, radio_only: bool) -> Result<Connected> {
        Ok(self.mount_picked_here(c, radio_only)?.0)
    }

    /// `mount_picked`, and whether this call mounted the card.
    pub(super) fn mount_picked_here(
        &self,
        c: Connected,
        radio_only: bool,
    ) -> Result<(Connected, bool)> {
        let mounted = self.gear_connected()?;
        if mounted.iter().any(|m| m.id.is_some() && m.id == c.id) {
            return Ok((c, false));
        }
        let l = self.card_for_job(&c, radio_only)?;
        Ok((l.connected, l.mounted_here))
    }

    /// Makes sure the card a job picked is mounted. A mounted one comes back as is; an
    /// unmounted one is mounted now and comes back with `mounted_here`, so the job (or
    /// `card_quiet_unmount`) releases it when done.
    pub(super) fn card_for_job(&self, c: &Connected, radio_only: bool) -> Result<Located> {
        let id = c.id.clone().with_context(|| {
            format!(
                "{} has no id, so QuadCam cannot mount it.",
                connected_label(c)
            )
        })?;
        self.locate_volume(&id, true, radio_only)?
            .with_context(|| format!("{} is not plugged in.", connected_label(c)))
    }

    /// The card's last check failed, and a job that runs on its link.
    fn card_facts(&self, c: &Connected) -> CardFacts {
        let check_failed =
            c.id.as_deref()
                .and_then(|id| HealthLog::new(self.gear_store()).latest(id))
                .filter(|k| k.state == CheckState::Failed)
                .map(|k| k.summary);
        let busy = self
            .gear_jobs()
            .into_iter()
            .find(|j| j.handle == link_handle(&c.link))
            .map(|j| j.step);
        CardFacts { check_failed, busy }
    }

    /// What a card change asks of the card. A restore stands alone and becomes the files
    /// the backup holds for the paths it names (a path the backup lacks is removed).
    fn card_work(&self, edits: &[Edit]) -> Result<CardWork> {
        let restores: Vec<&Edit> = edits
            .iter()
            .filter(|e| matches!(e, Edit::Restore { .. }))
            .collect();
        let files: Vec<&Edit> = edits
            .iter()
            .filter(|e| matches!(e, Edit::CardFiles { .. }))
            .collect();
        if !files.is_empty() {
            if files.len() != edits.len() {
                bail!(
                    "Refused (shape_unknown): card files (a voice) stand with no model or radio edit; stage them as their own change."
                );
            }
            let blobs = self.snapshots().blobs();
            let mut out: Vec<(String, Option<Vec<u8>>)> = Vec::new();
            for e in files {
                let Edit::CardFiles { put, delete } = e else {
                    unreachable!()
                };
                for f in put {
                    check_sound_path(&f.path)?;
                    let bytes = blobs
                        .get(&crate::gear::blobs::BlobRef {
                            xxh64: f.xxh64.clone(),
                            size: f.size,
                        })
                        .with_context(|| {
                            format!("QuadCam lost the bytes staged for {}.", f.path)
                        })?;
                    out.retain(|(p, _)| p != &f.path);
                    out.push((f.path.clone(), Some(bytes)));
                }
                for d in delete {
                    check_sound_path(d)?;
                    out.retain(|(p, _)| p != d);
                    out.push((d.clone(), None));
                }
            }
            return Ok(CardWork::Files(out));
        }
        if restores.is_empty() {
            return Ok(CardWork::Edits(edits.to_vec()));
        }
        if restores.len() != edits.len() || restores.len() > 1 {
            bail!(
                "Refused (shape_unknown): a card restore stands alone; stage it as its own change."
            );
        }
        let Edit::Restore { backup, paths } = restores[0] else {
            unreachable!()
        };
        if paths.is_empty() {
            bail!("Refused (shape_unknown): a card restore names the files to put back; none were named.");
        }
        let snaps = self.snapshots();
        let b = snaps.get(backup)?;
        let blobs = snaps.blobs();
        let mut files = Vec::new();
        for p in paths {
            let bytes = match b.files.iter().find(|f| &f.path == p) {
                Some(f) => Some(
                    blobs
                        .get(&blob_of(f))
                        .with_context(|| format!("Backup {backup} lost the bytes of {p}."))?,
                ),
                None => None,
            };
            files.push((p.clone(), bytes));
        }
        Ok(CardWork::Files(files))
    }

    /// Stage-time check of a card change's edits: the work builds and the edits are ones
    /// the engine plans. The files are checked at plan time, when the card is mounted.
    pub(super) fn check_card_stageable(&self, edits: &[Edit]) -> Result<()> {
        for e in edits {
            match e {
                Edit::Model { .. }
                | Edit::Radio { .. }
                | Edit::Checklist { .. }
                | Edit::ModelCopy { .. }
                | Edit::ModelDelete { .. }
                | Edit::CardFiles { .. }
                | Edit::Restore { .. } => {}
                other => {
                    return Err(refusal(Refusal::new(
                        RefusalCode::ShapeUnknown,
                        format!(
                            "{} edits are not a radio card change.",
                            serde_json::to_value(other)
                                .ok()
                                .and_then(|v| v["kind"].as_str().map(str::to_string))
                                .unwrap_or_default()
                        ),
                    )))
                }
            }
        }
        self.card_work(edits).map(|_| ())
    }

    /// The plan of a card change; the card is mounted for it and, when this call mounted
    /// it, unmounted again. Returns the located card for an apply to go on with.
    pub(super) fn plan_card(
        &self,
        change: &StagedChange,
        keep_mounted: bool,
    ) -> Result<(CardPlanned, Option<Located>)> {
        let work = self.card_work(&change.edits)?;
        let located = self.locate_card(&change.device, true)?;
        let (cands, facts) = match &located {
            Some(l) => (
                vec![CardCand {
                    connected: l.connected.clone(),
                    root: l.root.clone(),
                }],
                self.card_facts(&l.connected),
            ),
            None => (self.card_cands()?, CardFacts::default()),
        };
        let planned = cardplan::plan(change, &work, &cands, &facts);
        if !keep_mounted {
            if let Some(l) = &located {
                self.card_quiet_unmount(l);
            }
            return Ok((planned, None));
        }
        Ok((planned, located))
    }

    /// The person's Mount: the card is mounted to browse and unmounts after `minutes`.
    pub fn gear_card_mount(&self, p: &CardMountParams) -> Result<CardMounted> {
        let minutes = p.minutes.unwrap_or(MOUNT_MINUTES).clamp(1, 60);
        let device = p.device.trim();
        let l = self
            .locate_volume(device, true, false)?
            .with_context(|| format!("Card {device:?} is not plugged in."))?;
        let until = Utc::now() + chrono::Duration::minutes(minutes as i64);
        self.gear_mounted_for_user.lock().unwrap().insert(
            device.to_string(),
            (
                Instant::now() + Duration::from_secs(minutes as u64 * 60),
                until,
            ),
        );
        self.hooks.gear_changed();
        Ok(CardMounted {
            device: device.to_string(),
            mount: l.root,
            until,
        })
    }

    /// Done: unmounts a card the person mounted (or any mounted radio card). Plays the
    /// "safe to unplug" cue after the unmount worked.
    pub fn gear_card_unmount(&self, p: &CardMountParams) -> Result<()> {
        let device = p.device.trim();
        self.gear_mounted_for_user.lock().unwrap().remove(device);
        let l = self
            .locate_volume(device, false, false)?
            .with_context(|| format!("Card {device:?} is not mounted."))?;
        self.gear_release_card(&l.connected)
    }

    /// Unmounts the cards whose Mount time ran out. The app's poll calls it.
    pub fn gear_mount_tick(&self, now: Instant) {
        let due: Vec<String> = {
            let mut m = self.gear_mounted_for_user.lock().unwrap();
            let due: Vec<String> = m
                .iter()
                .filter(|(_, (t, _))| *t <= now)
                .map(|(k, _)| k.clone())
                .collect();
            for k in &due {
                m.remove(k);
            }
            due
        };
        for device in due {
            let _ = self.gear_card_unmount(&CardMountParams {
                device,
                minutes: None,
            });
        }
    }

    /// The cards the person mounted and when each unmounts.
    pub fn gear_mounted_cards(&self) -> Vec<CardMounted> {
        let m = self.gear_mounted_for_user.lock().unwrap();
        let mut out: Vec<CardMounted> = m
            .iter()
            .filter_map(|(device, (_, until))| {
                let l = self.locate_volume(device, false, false).ok()??;
                Some(CardMounted {
                    device: device.clone(),
                    mount: l.root,
                    until: *until,
                })
            })
            .collect();
        out.sort_by(|a, b| a.device.cmp(&b.device));
        out
    }

    /// Applies a staged card change. The plan was read already (the caller compared its
    /// digest). The card is still mounted in `l`.
    pub(super) fn apply_card(
        &self,
        change: &StagedChange,
        planned: CardPlanned,
        l: Located,
        stash: &std::sync::Mutex<Option<ApplyReport>>,
    ) -> Result<ApplyReport> {
        let c = l.connected.clone();
        let handle = link_handle(&c.link);
        let job = self.job_start(&handle, Some(&change.device), "Applying");
        let (_job, stop) = match job {
            Ok(j) => j,
            Err(e) => {
                self.card_quiet_unmount(&l);
                return Err(e);
            }
        };
        let _hold = self.gear_hold(&handle);
        let out = self.apply_card_inner(change, planned, &l, &stop, stash);
        // Mount, work, unmount: the card is released whatever the job did; "safe to
        // unplug" plays only when it worked.
        let (failed, refused) = match &out {
            Ok(r) if r.status == ChangeStatus::Verified => (None, false),
            Ok(r) => (Some(r.message.clone()), false),
            Err(e) => (
                Some(format!("{e:#}")),
                e.downcast_ref::<Refusal>().is_some() || format!("{e:#}").starts_with("Refused"),
            ),
        };
        if refused {
            // A refusal plays no cue.
            self.card_quiet_unmount(&Located {
                mounted_here: true,
                ..l
            });
        } else if let Err(why) = self.gear_finish_card(&c, failed.as_deref().map(|m| ("Apply", m)))
        {
            if let Ok(r) = &out {
                let mut r = r.clone();
                r.notes.push(format!("The card did not unmount: {why}"));
                return Ok(r);
            }
        }
        out
    }

    fn apply_card_inner(
        &self,
        change: &StagedChange,
        planned: CardPlanned,
        l: &Located,
        stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        stash: &std::sync::Mutex<Option<ApplyReport>>,
    ) -> Result<ApplyReport> {
        let _ = stash;
        let handle = link_handle(&l.connected.link);
        let step = |s: &str| self.job_step(&handle, s);
        let snaps = self.snapshots();
        let plan = planned.files.context("The plan holds no files.")?;
        let radio_usb = l.connected.usb.is_some();
        let mut opts = if radio_usb {
            WriteOptions::radio_usb()
        } else {
            WriteOptions::reader()
        };
        opts.stop = stop.clone();
        opts.fail_readback = self.gear.fail_readback.clone();

        // Backup first; a failed backup stops everything. A full snapshot, always kept.
        step("Backing up");
        let nobackup = |why: String| {
            refusal(Refusal::new(
                RefusalCode::NoBackup,
                format!("The backup failed: {why}. Nothing was written."),
            ))
        };
        let mut identity = plan.identity.clone();
        crate::gear::store::merge_identity(&mut identity, &l.connected.identity);
        let taken = {
            let mut on_progress = |p: &BackupProgress| self.job_progress(&handle, p);
            let mut take = TakeOptions {
                stop: Some(stop),
                progress: Some(&mut on_progress),
                crash_before_manifest: false,
            };
            snaps
                .take_card(
                    &change.device,
                    &identity,
                    &l.root,
                    Trigger::BeforeApply,
                    &mut take,
                )
                .map_err(|e| nobackup(format!("{e:#}")))?
        };
        let backup = taken.backup.clone();
        let _ = self.after_backup(
            &change.device,
            DeviceKind::Radio,
            &identity,
            taken,
            Vec::new(),
            super::flights::space_seen(&l.root),
        );

        // Write. Each touched file must be in the backup with the bytes the plan read.
        step("Writing");
        let blobs = snaps.blobs();
        let mut check_backup = |path: &str, bytes: &[u8]| -> Result<()> {
            let f = backup
                .files
                .iter()
                .find(|f| f.path == path)
                .with_context(|| format!("the backup has no {path}"))?;
            if f.xxh64 != crate::gear::blobs::hash(bytes) || f.size != bytes.len() as u64 {
                bail!("the backup holds different bytes of {path}");
            }
            blobs.get(&blob_of(f)).map(|_| ())
        };
        let mut progress = |p: &WriteProgress| {
            self.job_progress(
                &handle,
                &BackupProgress {
                    stage: "writing".into(),
                    files_done: p.file.saturating_sub(1),
                    files_total: p.files,
                    bytes_done: p.bytes_done,
                    bytes_total: p.bytes_total,
                    path: p.path.clone(),
                },
            )
        };
        let wrote = engine::write(&l.root, &plan, &opts, &mut check_backup, &mut progress);

        let mut steps = vec![StepReport {
            name: "Back up".into(),
            state: StepState::Done,
            detail: Some(backup.id.clone()),
        }];
        let skipped = |n: &str| StepReport {
            name: n.into(),
            state: StepState::Skipped,
            detail: None,
        };
        let mut report = ApplyReport {
            change: change.id.clone(),
            device: change.device.clone(),
            status: ChangeStatus::Failed,
            steps: Vec::new(),
            backup: Some(backup.id.clone()),
            after_backup: None,
            sent: Vec::new(),
            failed_line: None,
            verify: Vec::new(),
            saved: false,
            files: Vec::new(),
            message: String::new(),
            notes: planned.warnings.clone(),
            at: Utc::now(),
        };
        let done = match wrote {
            Ok(w) => w,
            Err(e) => {
                // A refusal before the first byte went out is not a failed apply.
                if let Some(r) = e.downcast_ref::<Refusal>() {
                    return Err(refusal(r.clone()));
                }
                // A write or a read-back went wrong: put every file back.
                let rb = cardplan::roll_back(&l.root, &plan, &opts);
                steps.push(StepReport {
                    name: "Write".into(),
                    state: StepState::Failed,
                    detail: Some(format!("{e:#}")),
                });
                steps.push(StepReport {
                    name: "Roll back".into(),
                    state: if rb.failed.is_empty() {
                        StepState::Done
                    } else {
                        StepState::Failed
                    },
                    detail: Some(if rb.failed.is_empty() {
                        format!("{} files put back", rb.restored.len())
                    } else {
                        rb.failed.join("; ")
                    }),
                });
                steps.push(skipped("Read back"));
                steps.push(skipped("Verify"));
                report.files = rb.restored.clone();
                report.message = if rb.failed.is_empty() {
                    format!("{e:#} The card is as it was.")
                } else {
                    format!(
                        "{e:#} Putting the files back failed for {}. Restore the backup.",
                        rb.failed.join(", ")
                    )
                };
                return self.record_card(change, report, steps);
            }
        };
        report.files = done
            .written
            .iter()
            .chain(done.deleted.iter())
            .cloned()
            .collect();
        steps.push(StepReport {
            name: "Write".into(),
            state: StepState::Done,
            detail: Some(format!(
                "{} written, {} deleted",
                done.written.len(),
                done.deleted.len()
            )),
        });
        if done.stopped {
            let rb = cardplan::roll_back(&l.root, &plan, &opts);
            steps.push(StepReport {
                name: "Roll back".into(),
                state: if rb.failed.is_empty() {
                    StepState::Done
                } else {
                    StepState::Failed
                },
                detail: Some(format!("{} files put back", rb.restored.len())),
            });
            steps.push(skipped("Read back"));
            steps.push(skipped("Verify"));
            report.message = format!(
                "Stopped after {} files; QuadCam put them back. The card is as it was.",
                done.written.len() + done.deleted.len()
            );
            return self.record_card(change, report, steps);
        }
        report.saved = true;

        // Read back on fresh reads.
        step("Verifying");
        steps.push(StepReport {
            name: "Read back".into(),
            state: StepState::Done,
            detail: None,
        });
        let wrong = cardplan::verify(&l.root, &plan);
        if wrong.is_empty() {
            steps.push(StepReport {
                name: "Verify".into(),
                state: StepState::Done,
                detail: None,
            });
            report.status = ChangeStatus::Verified;
            report.message = format!(
                "Verified: {} files read back as written.",
                done.written.len() + done.deleted.len()
            );
            // The state now, as a backup, so the next plan compares with it.
            if let Ok(rep) = snaps.take_card(
                &change.device,
                &identity,
                &l.root,
                Trigger::AfterApply,
                &mut TakeOptions::default(),
            ) {
                report.after_backup = Some(rep.backup.id.clone());
                let _ = self.after_backup(
                    &change.device,
                    DeviceKind::Radio,
                    &identity,
                    rep,
                    Vec::new(),
                    super::flights::space_seen(&l.root),
                );
            }
        } else {
            let rb = cardplan::roll_back(&l.root, &plan, &opts);
            steps.push(StepReport {
                name: "Verify".into(),
                state: StepState::Failed,
                detail: Some(format!("{} files differ", wrong.len())),
            });
            steps.push(StepReport {
                name: "Roll back".into(),
                state: if rb.failed.is_empty() {
                    StepState::Done
                } else {
                    StepState::Failed
                },
                detail: Some(format!("{} files put back", rb.restored.len())),
            });
            report.message = if rb.failed.is_empty() {
                format!(
                    "{} did not read back as written; QuadCam put the files back. The card is as it was.",
                    wrong.join(", ")
                )
            } else {
                format!(
                    "{} did not read back as written, and putting the files back failed. Restore the backup.",
                    wrong.join(", ")
                )
            };
        }
        self.record_card(change, report, steps)
    }

    fn record_card(
        &self,
        change: &StagedChange,
        mut report: ApplyReport,
        steps: Vec<StepReport>,
    ) -> Result<ApplyReport> {
        report.steps = steps;
        report.at = Utc::now();
        self.finish_change(change, &report)?;
        Ok(report)
    }

    /// The card side of `apply_inner`: plan with the card mounted, compare the digest,
    /// confirm, then the job.
    pub(super) fn apply_card_change(
        &self,
        req: &ApplyRequest,
        from_gui: bool,
    ) -> Result<ApplyReport> {
        let change = self.changes().get(&req.id)?;
        let (planned, located) = self.plan_card(&change, true)?;
        let Some(l) = located else {
            let r = first_refusal(&planned.plan.checks).unwrap_or_else(|| {
                Refusal::new(RefusalCode::NoDevice, "The card is not plugged in.")
            });
            return Err(refusal(r));
        };
        let early = if let Some(r) = first_refusal(&planned.plan.checks) {
            Some(r)
        } else if planned.plan.digest != req.digest {
            Some(Refusal::new(
                RefusalCode::BeforeMismatch,
                "The plan changed since you read it; plan again.",
            ))
        } else {
            None
        };
        if let Some(r) = early {
            self.card_quiet_unmount(&l);
            return Err(refusal(r));
        }
        if !from_gui {
            if let Err(e) = self.hooks.confirm_apply(&change, &planned.plan) {
                self.card_quiet_unmount(&l);
                return Err(e);
            }
        }
        let stash = std::sync::Mutex::new(None);
        self.apply_card(&change, planned, l, &stash)
    }
}

fn connected_label(c: &Connected) -> String {
    super::connected_name(c)
}

/// A card-files edit puts sounds under `SOUNDS/` only.
fn check_sound_path(path: &str) -> Result<()> {
    crate::gear::voice::lines::check_path(path)
        .map_err(|e| anyhow::anyhow!("Refused (shape_unknown): card files go under SOUNDS/: {e:#}"))
}
