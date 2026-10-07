//! `Core`'s backups (design 7.1): take a snapshot of a radio card or an FC, list, read,
//! diff and pin snapshots, the Storage view, prune, export, import old backup folders,
//! and the card check before a known card is read.
//!
//! - A backup is one job: it holds the device's link, shows its progress in
//!   `GearStatus.jobs`, and can be stopped (`gear_stop`) between files.
//! - "Back up on connect" registers two on-connect steps (`backup_hooks`, added by the
//!   app): "Card check" (a known card: `diskutil verifyVolume`) and "Backup". A failed
//!   check plays its own cue; the backup still reads the card.
//! - Pruning runs after each new plug-in or manual snapshot, not after an import.

use super::{link_handle, Core, HookFn, OnConnectHook};
use crate::gear::backup::{
    self, BackupContent, BackupProgress, ExportReport, ImportBackupsReport, PruneReport, Retention,
    Snapshots, StorageView, TakeOptions, TakeReport,
};
use crate::gear::health::{self, CardCheck, CheckKind, CheckState, HealthLog};
use crate::gear::model::{
    device_id, Backup, Connected, Device, DeviceKind, DiffItem, Identity, Link, Trigger,
};
use crate::gear::Automation;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// `gear_backup`: what to back up. One of them, or none when exactly one radio card or FC
/// is plugged in.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct BackupParams {
    /// A connected device's id.
    #[serde(default)]
    pub device: Option<String>,
    /// An FC's serial port.
    #[serde(default)]
    pub port: Option<String>,
    /// A radio card's mount point.
    #[serde(default)]
    pub mount: Option<PathBuf>,
}

/// `gear_backups`: one device's snapshots, or every device's.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct BackupFilter {
    #[serde(default)]
    pub device: Option<String>,
}

/// `gear_backup_read`: a snapshot, and one of its files (none: the file list).
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct BackupReadParams {
    pub id: String,
    #[serde(default)]
    pub path: Option<String>,
}

/// `gear_backup_diff`: from snapshot `a` to `b` (none: from the one before `a` to `a`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct BackupDiffParams {
    pub a: String,
    #[serde(default)]
    pub b: Option<String>,
    /// One file only.
    #[serde(default)]
    pub path: Option<String>,
}

/// `gear_backup_pin`: keep a snapshot through pruning, or stop keeping it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct BackupPinParams {
    pub id: String,
    pub pinned: bool,
}

/// `gear_prune`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct PruneParams {
    #[serde(default)]
    pub dry_run: bool,
}

/// `gear_export`: a snapshot, or every snapshot of a device, to a folder.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct ExportParams {
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub snapshot: Option<String>,
    pub to: PathBuf,
}

/// `gear_import_backups`: an old backup folder.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct ImportBackupsParams {
    pub folder: PathBuf,
    /// The saved device that items no id names go to, when several could match.
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub dry_run: bool,
}

/// `gear_card_check`: which card. None: the one card plugged in.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct CardCheckParams {
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub mount: Option<PathBuf>,
}

/// `gear_card_checks`: a device's check log.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct CardChecksParams {
    pub device: String,
}

/// `gear_card_repair`: the failed check to answer, and the confirm.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct CardRepairParams {
    /// The id of the card's latest check, which failed.
    pub check: String,
    #[serde(default)]
    pub confirm: bool,
}

/// `gear_stop`: a running job's link (`GearJob.handle`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct StopParams {
    pub handle: String,
}

/// A snapshot without its file list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct BackupSummary {
    pub id: String,
    pub device: String,
    pub trigger: Trigger,
    pub taken_at: DateTime<Utc>,
    pub identity: Identity,
    pub pinned: bool,
    pub files: u32,
    pub bytes: u64,
}

impl From<&Backup> for BackupSummary {
    fn from(b: &Backup) -> Self {
        Self {
            id: b.id.clone(),
            device: b.device.clone(),
            trigger: b.trigger,
            taken_at: b.taken_at,
            identity: b.identity.clone(),
            pinned: b.pinned,
            files: b.files.len() as u32,
            bytes: b.files.iter().map(|f| f.size).sum(),
        }
    }
}

/// `gear_backup`'s answer.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BackupResult {
    pub device: String,
    pub kind: DeviceKind,
    /// The device's name, or "Unnamed <kind>".
    pub name: String,
    pub report: TakeReport,
    /// The prune that followed a new snapshot.
    #[serde(default)]
    pub pruned: Option<PruneReport>,
    /// FC board notes after the read.
    #[serde(default)]
    pub notes: Vec<String>,
}

/// `gear_card_repair`'s answer.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RepairResult {
    /// The snapshot taken first (always kept), when the card could be read.
    pub backup: Option<String>,
    /// Why the backup first failed; the repair ran anyway.
    pub backup_error: Option<String>,
    pub repair: CardCheck,
    /// The check after the repair.
    pub verify: CardCheck,
}

/// A job running on a device now: a backup or a card check.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct GearJob {
    /// The link (`link_handle`): what `gear_stop` takes.
    pub handle: String,
    pub device: Option<String>,
    /// `Backing up` or `Checking card`.
    pub step: String,
    pub progress: BackupProgress,
    /// A stop was asked for; it takes effect between files.
    pub stopping: bool,
}

/// One running job's stop flag and progress.
pub(super) struct JobSlot {
    stop: Arc<AtomicBool>,
    job: GearJob,
    last_note: Instant,
}

/// Unregisters a job when it ends.
struct JobGuard<'a> {
    core: &'a Core,
    handle: String,
}

impl Drop for JobGuard<'_> {
    fn drop(&mut self) {
        self.core.gear_jobs.lock().unwrap().remove(&self.handle);
        self.core.hooks.gear_changed();
    }
}

/// The on-connect steps for "Back up on connect": the card check, then the backup.
pub fn backup_hooks() -> Vec<OnConnectHook> {
    let check: HookFn = Arc::new(|core: &Core, c: &Connected| core.gear_check_on_connect(c));
    let backup: HookFn = Arc::new(|core: &Core, c: &Connected| {
        core.gear_backup_connected(c, Trigger::Connect, false)
            .map(|_| ())
    });
    vec![
        OnConnectHook {
            name: "Card check",
            automation: Automation::Backup,
            kinds: vec![DeviceKind::Radio, DeviceKind::Goggles, DeviceKind::DvrCard],
            run: check,
        },
        OnConnectHook {
            name: "Backup",
            automation: Automation::Backup,
            kinds: vec![DeviceKind::Radio, DeviceKind::Fc],
            run: backup,
        },
    ]
}

impl Core {
    fn snapshots(&self) -> Snapshots {
        Snapshots::new(self.gear_store())
    }

    /// The jobs running now (backups, card checks), for `GearStatus`.
    pub fn gear_jobs(&self) -> Vec<GearJob> {
        let mut v: Vec<GearJob> = self
            .gear_jobs
            .lock()
            .unwrap()
            .values()
            .map(|s| s.job.clone())
            .collect();
        v.sort_by(|a, b| a.handle.cmp(&b.handle));
        v
    }

    /// Asks a running job to stop. True when one was running on that link.
    pub fn gear_stop(&self, handle: &str) -> bool {
        let mut jobs = self.gear_jobs.lock().unwrap();
        let Some(s) = jobs.get_mut(handle) else {
            return false;
        };
        s.stop.store(true, Ordering::SeqCst);
        s.job.stopping = true;
        drop(jobs);
        self.hooks.gear_changed();
        true
    }

    /// Registers a job on a link; refuses a second job on the same link.
    fn job_start(
        &self,
        handle: &str,
        device: Option<&str>,
        step: &str,
    ) -> Result<(JobGuard<'_>, Arc<AtomicBool>)> {
        let stop = Arc::new(AtomicBool::new(false));
        {
            let mut jobs = self.gear_jobs.lock().unwrap();
            if let Some(j) = jobs.get(handle) {
                bail!(
                    "Refused: {} is already running on this device.",
                    j.job.step.to_lowercase()
                );
            }
            jobs.insert(
                handle.to_string(),
                JobSlot {
                    stop: stop.clone(),
                    job: GearJob {
                        handle: handle.to_string(),
                        device: device.map(str::to_string),
                        step: step.to_string(),
                        progress: BackupProgress::default(),
                        stopping: false,
                    },
                    last_note: Instant::now(),
                },
            );
        }
        self.hooks.gear_changed();
        Ok((
            JobGuard {
                core: self,
                handle: handle.to_string(),
            },
            stop,
        ))
    }

    /// Records a job's progress; tells the GUI at most four times a second.
    fn job_progress(&self, handle: &str, p: &BackupProgress) {
        let note = {
            let mut jobs = self.gear_jobs.lock().unwrap();
            let Some(s) = jobs.get_mut(handle) else {
                return;
            };
            s.job.progress = p.clone();
            let due = s.last_note.elapsed() >= Duration::from_millis(250) || p.stage != "reading";
            if due {
                s.last_note = Instant::now();
            }
            due
        };
        if note {
            self.hooks.gear_changed();
        }
    }

    /// Backs a device up: a radio card (its files, then its logs) or an FC (`version`,
    /// `status`, `diff all`, `dump all` through the CLI; the FC reboots). Nothing is
    /// written when nothing changed since the latest snapshot. One cue at the end.
    pub fn gear_backup(&self, p: &BackupParams) -> Result<BackupResult> {
        let target = self.backup_target(p)?;
        self.gear_backup_connected(&target, Trigger::Manual, true)
    }

    /// What `gear_backup` names: a mount, a port, a device id, or the one radio card or FC
    /// plugged in.
    fn backup_target(&self, p: &BackupParams) -> Result<Connected> {
        let connected = self.gear_connected()?;
        if let Some(port) = p.port.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            return self.gear_fc_pick(Some(port));
        }
        if let Some(m) = &p.mount {
            if let Some(c) = connected
                .iter()
                .find(|c| matches!(&c.link, Link::Volume { mount, .. } if mount == m))
            {
                return Ok(c.clone());
            }
            // A card copy or a card not detected as gear: by its QuadCam marker.
            let raw = crate::gear::edgetx::card::read_marker(m).with_context(|| {
                format!(
                    "{} is not a radio card QuadCam knows: plug the radio in, or import the folder with gear import-backups.",
                    m.display()
                )
            })?;
            return Ok(Connected {
                id: Some(device_id(DeviceKind::Radio, &raw)),
                kind: DeviceKind::Radio,
                link: Link::Volume {
                    mount: m.clone(),
                    volume_uuid: None,
                    bus_protocol: None,
                    whole_disk: None,
                },
                identity: Identity::default(),
                device: None,
                usb: None,
            });
        }
        let able = |c: &&Connected| {
            matches!(
                (c.kind, &c.link),
                (DeviceKind::Radio, Link::Volume { .. }) | (DeviceKind::Fc, Link::Serial { .. })
            )
        };
        if let Some(id) = p.device.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            return connected
                .iter()
                .filter(able)
                .find(|c| c.id.as_deref() == Some(id))
                .cloned()
                .or_else(|| {
                    // An FC not identified yet: the only FC plugged in.
                    (crate::gear::backup::kind_of_id(id) == Some(DeviceKind::Fc))
                        .then(|| self.gear_fc_pick(None).ok())
                        .flatten()
                })
                .with_context(|| {
                    format!("No device: {id:?} is not plugged in (or not a radio card or FC).")
                });
        }
        let list: Vec<&Connected> = connected.iter().filter(able).collect();
        match list.as_slice() {
            [one] => Ok((*one).clone()),
            [] => bail!(
                "No device: no radio card or FC is plugged in. If macOS asked to allow an accessory, click Allow."
            ),
            many => bail!(
                "{} devices are plugged in; name one with device, port or mount.",
                many.len()
            ),
        }
    }

    /// Backs up a connected device. `cue`: play the job's cue (an on-connect run plays its
    /// own; an FC read always plays one).
    pub fn gear_backup_connected(
        &self,
        c: &Connected,
        trigger: Trigger,
        cue: bool,
    ) -> Result<BackupResult> {
        match (&c.kind, &c.link) {
            (DeviceKind::Fc, Link::Serial { .. }) => self.backup_fc(c, trigger),
            (DeviceKind::Radio, Link::Volume { mount, .. }) => {
                let r = self.backup_card(c, mount, trigger);
                if cue {
                    self.gear_job_done(c, r.as_ref().err().map(|_| "Backup"));
                }
                r
            }
            _ => bail!(
                "QuadCam backs up radio cards and FCs; this is a {}.",
                c.kind.label()
            ),
        }
    }

    fn backup_fc(&self, c: &Connected, trigger: Trigger) -> Result<BackupResult> {
        let port = link_handle(&c.link);
        let job = self.gear_fc_read(&super::FcReadParams {
            port: Some(port),
            commands: Vec::new(),
        })?;
        let info = &job.result.info;
        let id = info.id.clone().context(
            "This FC gave no stable id (no MCU id and no USB serial), so QuadCam cannot keep its backups apart.",
        )?;
        let files: Vec<(String, Vec<u8>)> = crate::gear::bf::BACKUP
            .iter()
            .filter_map(|cmd| {
                job.result
                    .file(cmd)
                    .map(|t| (cmd.to_string(), t.into_bytes()))
            })
            .collect();
        let report =
            self.snapshots()
                .take_files(&id, &info.identity, trigger, Utc::now(), &files, false)?;
        self.after_backup(&id, DeviceKind::Fc, &info.identity, report, job.notes)
    }

    fn backup_card(
        &self,
        c: &Connected,
        mount: &std::path::Path,
        trigger: Trigger,
    ) -> Result<BackupResult> {
        let id =
            c.id.clone()
                .context("This card has no id yet (no volume UUID and no QuadCam marker).")?;
        let card = crate::gear::edgetx::card::Card::open(mount)?;
        let mut identity = card.identity();
        crate::gear::store::merge_identity(&mut identity, &c.identity);
        let handle = link_handle(&c.link);
        let (_job, stop) = self.job_start(&handle, Some(&id), "Backing up")?;
        let _hold = self.gear_hold(&handle);
        let mut on_progress = |p: &BackupProgress| self.job_progress(&handle, p);
        let mut opts = TakeOptions {
            stop: Some(&stop),
            progress: Some(&mut on_progress),
            crash_before_manifest: false,
        };
        let report = self
            .snapshots()
            .take_card(&id, &identity, mount, trigger, &mut opts)?;
        self.after_backup(&id, DeviceKind::Radio, &identity, report, Vec::new())
    }

    /// Saves the device as seen with its newest backup, and prunes after a new snapshot.
    fn after_backup(
        &self,
        id: &str,
        kind: DeviceKind,
        identity: &Identity,
        report: TakeReport,
        notes: Vec<String>,
    ) -> Result<BackupResult> {
        let store = self.gear_store();
        let mut d = store.seen(id, kind, identity)?;
        d.last_backup = Some(report.backup.id.clone());
        let d = store.save_device(&d)?;
        let pruned = if report.new && report.backup.trigger != Trigger::Import {
            let s = self.gear_settings();
            Some(
                self.snapshots()
                    .prune(&Retention::from(&s), Utc::now(), false)?,
            )
        } else {
            None
        };
        self.hooks.gear_changed();
        Ok(BackupResult {
            device: id.to_string(),
            kind,
            name: d.display_name(),
            report,
            pruned,
            notes,
        })
    }

    /// Snapshots, newest first, without their file lists.
    pub fn gear_backups(&self, f: &BackupFilter) -> Result<Vec<BackupSummary>> {
        let s = self.snapshots();
        let mut list: Vec<Backup> =
            match f.device.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
                Some(d) => s.list(d),
                None => s.all(),
            };
        list.reverse();
        Ok(list.iter().map(BackupSummary::from).collect())
    }

    /// A snapshot with its files, or one file's content.
    pub fn gear_backup_read(&self, p: &BackupReadParams) -> Result<BackupContent> {
        self.snapshots().read(p.id.trim(), p.path.as_deref())
    }

    /// What changed between two snapshots of a device (from the one before `a` when `b` is
    /// not given).
    pub fn gear_backup_diff(&self, p: &BackupDiffParams) -> Result<Vec<DiffItem>> {
        let s = self.snapshots();
        let a = s.get(p.a.trim())?;
        match p.b.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
            Some(b) => s.diff(&a.id, b, p.path.as_deref()),
            None => {
                let list = s.list(&a.device);
                let i = list.iter().position(|x| x.id == a.id).unwrap_or(0);
                let Some(prev) = i.checked_sub(1).and_then(|i| list.get(i)) else {
                    bail!(
                        "{} is the device's first backup; there is nothing before it.",
                        a.id
                    );
                };
                s.diff(&prev.id, &a.id, p.path.as_deref())
            }
        }
    }

    /// Pins or unpins a snapshot. A pinned one is never pruned.
    pub fn gear_backup_pin(&self, p: &BackupPinParams) -> Result<BackupSummary> {
        let b = self.snapshots().pin(p.id.trim(), p.pinned)?;
        self.hooks.gear_changed();
        Ok(BackupSummary::from(&b))
    }

    /// Sizes of the gear folder, in total and per device.
    pub fn gear_storage(&self) -> Result<StorageView> {
        let devices = self.gear_store().devices()?;
        self.snapshots().storage(&devices)
    }

    /// Thins snapshots by the retention settings and removes blobs nothing names.
    pub fn gear_prune(&self, p: &PruneParams) -> Result<PruneReport> {
        let s = self.gear_settings();
        let r = self
            .snapshots()
            .prune(&Retention::from(&s), Utc::now(), p.dry_run)?;
        if !p.dry_run {
            self.hooks.gear_changed();
        }
        Ok(r)
    }

    /// Writes a snapshot, or every snapshot of a device, as plain folders.
    pub fn gear_export(&self, p: &ExportParams) -> Result<ExportReport> {
        let s = self.snapshots();
        let ids: Vec<String> = match (
            p.snapshot
                .as_deref()
                .map(str::trim)
                .filter(|x| !x.is_empty()),
            p.device.as_deref().map(str::trim).filter(|x| !x.is_empty()),
        ) {
            (Some(id), _) => vec![id.to_string()],
            (None, Some(d)) => s.list(d).into_iter().map(|b| b.id).collect(),
            (None, None) => bail!("Name a snapshot or a device to export."),
        };
        if ids.is_empty() {
            bail!("That device has no backups.");
        }
        s.export(&ids, &p.to)
    }

    /// Imports an old backup folder: card copies, FC `diff all`/`dump all` files, `LOGS/`
    /// folders. Never changes the folder. FCs whose MCU id the files name are saved as
    /// devices.
    pub fn gear_import_backups(&self, p: &ImportBackupsParams) -> Result<ImportBackupsReport> {
        let store = self.gear_store();
        let known = store.devices()?;
        let device = match p.device.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
            Some(id) => Some(
                known
                    .iter()
                    .find(|d| d.id == id)
                    .cloned()
                    .with_context(|| format!("No saved device {id:?} (gear devices)."))?,
            ),
            None => None,
        };
        let s = self.snapshots();
        let report = backup::import(&s, &p.folder, &known, device.as_ref(), p.dry_run)?;
        if !p.dry_run {
            for id in report
                .items
                .iter()
                .filter_map(|i| i.device.as_ref())
                .collect::<std::collections::BTreeSet<_>>()
            {
                let Some(latest) = s.latest(id) else { continue };
                let mut d = match store.device(id)? {
                    Some(d) => d,
                    None => Device {
                        id: id.clone(),
                        kind: backup::kind_of_id(id).unwrap_or(DeviceKind::Fc),
                        name: String::new(),
                        aircraft: None,
                        identity: latest.identity.clone(),
                        last_seen: None,
                        last_backup: None,
                    },
                };
                d.last_backup = Some(latest.id.clone());
                store.save_device(&d)?;
            }
            self.hooks.gear_changed();
        }
        Ok(report)
    }

    // ----- card check -----

    /// The card a check names: a device id, a mount, or the one card plugged in.
    fn check_target(&self, p: &CardCheckParams) -> Result<Connected> {
        let cards: Vec<Connected> = self
            .gear_connected()?
            .into_iter()
            .filter(|c| matches!(c.link, Link::Volume { .. }))
            .collect();
        if let Some(m) = &p.mount {
            return cards
                .into_iter()
                .find(|c| matches!(&c.link, Link::Volume { mount, .. } if mount == m))
                .with_context(|| format!("No device: no card is mounted at {}.", m.display()));
        }
        if let Some(id) = p.device.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            return cards
                .into_iter()
                .find(|c| c.id.as_deref() == Some(id))
                .with_context(|| format!("No device: card {id:?} is not plugged in."));
        }
        match cards.len() {
            1 => Ok(cards.into_iter().next().unwrap()),
            0 => bail!("No device: no card is plugged in."),
            n => bail!("{n} cards are plugged in; name one by device or mount."),
        }
    }

    /// Checks a card's file system (`diskutil verifyVolume`, read-only, about 30 s over a
    /// radio's USB) and logs the result for the card. `gear_stop` ends it early.
    pub fn gear_card_check(&self, p: &CardCheckParams) -> Result<CardCheck> {
        let c = self.check_target(p)?;
        self.card_check(&c, CheckKind::Verify)
    }

    fn card_check(&self, c: &Connected, kind: CheckKind) -> Result<CardCheck> {
        let Link::Volume {
            mount, whole_disk, ..
        } = &c.link
        else {
            bail!("Only a card can be checked.");
        };
        let id =
            c.id.clone()
                .context("This card has no id, so its checks cannot be kept.")?;
        let handle = link_handle(&c.link);
        let step = match kind {
            CheckKind::Verify => "Checking card",
            CheckKind::Repair => "Repairing card",
        };
        let (_job, stop) = self.job_start(&handle, Some(&id), step)?;
        let _hold = self.gear_hold(&handle);
        let check = health::check(
            self.gear.disk.as_ref(),
            &id,
            kind,
            &mount.display().to_string(),
            whole_disk.as_deref(),
            Some(&stop),
        );
        HealthLog::new(self.gear_store()).append(&check)?;
        Ok(check)
    }

    /// The on-connect card check: a card QuadCam knows is checked before it is read. A
    /// check that finds damage fails the step (its cue plays) and leaves the result for the
    /// device page, which offers the repair.
    fn gear_check_on_connect(&self, c: &Connected) -> Result<()> {
        let known = match &c.id {
            Some(id) => self.gear_store().device(id)?.is_some(),
            None => false,
        };
        if !known || !matches!(c.link, Link::Volume { .. }) {
            return Ok(());
        }
        let check = self.card_check(c, CheckKind::Verify)?;
        match check.state {
            CheckState::Ok | CheckState::Stopped => Ok(()),
            _ => bail!("{}", check.summary),
        }
    }

    /// A card's checks, newest first.
    pub fn gear_card_checks(&self, p: &CardChecksParams) -> Vec<CardCheck> {
        HealthLog::new(self.gear_store()).list(p.device.trim())
    }

    /// The latest check of each card plugged in, for `GearStatus`.
    pub fn gear_latest_checks(&self, connected: &[Connected]) -> Vec<CardCheck> {
        let log = HealthLog::new(self.gear_store());
        connected
            .iter()
            .filter(|c| matches!(c.link, Link::Volume { .. }))
            .filter_map(|c| log.latest(c.id.as_deref()?))
            .collect()
    }

    /// Repairs a card whose latest check failed (`diskutil repairVolume`): a backup first
    /// when the card reads (always kept), the repair, then a check. Needs that check's id
    /// and `confirm`. A repair is not stopped once it starts.
    pub fn gear_card_repair(&self, p: &CardRepairParams) -> Result<RepairResult> {
        if !p.confirm {
            bail!("Refused: a repair writes the card's file system; it needs confirm=true.");
        }
        let check_id = p.check.trim();
        let connected = self.gear_connected()?;
        let log = HealthLog::new(self.gear_store());
        let c = connected
            .iter()
            .filter(|c| matches!(c.link, Link::Volume { .. }))
            .find(|c| {
                c.id.as_deref()
                    .and_then(|id| log.latest(id))
                    .is_some_and(|l| l.id == check_id)
            })
            .cloned()
            .with_context(|| {
                format!("Refused: {check_id:?} is not the latest check of a card plugged in. Check the card again.")
            })?;
        let latest = log.latest(c.id.as_deref().unwrap_or("")).unwrap();
        if latest.state == CheckState::Ok {
            bail!("Refused: the card's latest check passed; there is nothing to repair.");
        }
        let (backup, backup_error) = match (&c.kind, &c.link) {
            (DeviceKind::Radio, Link::Volume { mount, .. }) => {
                match self.backup_card(&c, mount, Trigger::BeforeApply) {
                    Ok(r) => (Some(r.report.backup.id), None),
                    Err(e) => (None, Some(format!("{e:#}"))),
                }
            }
            _ => (None, None),
        };
        let repair = self.card_check(&c, CheckKind::Repair)?;
        let verify = self.card_check(&c, CheckKind::Verify)?;
        self.gear_job_done(&c, (verify.state != CheckState::Ok).then_some("Repair"));
        Ok(RepairResult {
            backup,
            backup_error,
            repair,
            verify,
        })
    }
}
