//! Snapshots (design 7.1): take, list, read, diff, retain, prune and export them, and
//! import old backup folders.
//!
//! A snapshot is a manifest, `<gear>/snapshots/<device>/<YYYY-MM-DDTHHMMSS>-<trigger>.json`
//! (a `model::Backup`: path, size, XXH64 and modified time per file), over the blob store
//! (`blobs`). Its id is `<device>/<file stem>`.
//!
//! - **Order of writes:** every blob first, the manifest last (a temporary name, fsync,
//!   rename). A crash in between leaves blobs no manifest names; the next collection
//!   removes them. The store's lock is held from the first blob to the manifest.
//! - **Unchanged files are not read:** a card file whose size and modified time match the
//!   device's latest snapshot keeps that snapshot's hash. A radio over USB reads about
//!   0.5 MB/s, so a plug-in backup of an unchanged card reads nothing.
//! - **No empty snapshots:** a snapshot whose files equal the latest one's is not written.
//!   An FC's `status` (uptime, load) changes on every read and does not count. A snapshot
//!   before an apply or a flash is always written (it is always kept; its blobs are shared).
//! - **Retention** keeps apply and flash snapshots and pinned ones, the newest
//!   `keep_recent`, then one a week for `keep_weeks`, then one a month (`keep_monthly`).
//!   Pruning then collects the blobs no manifest, log or staged change names.

use super::blobs::{BlobRef, Blobs, Collected};
use super::edgetx::card::{identity_from_radio_yml, read_marker, RADIO_FILE};
use super::model::{
    device_id, Backup, BackupFile, Device, DeviceKind, DiffItem, Identity, Trigger,
};
use super::radiologs::{is_log_name, LogOutcome, LogStore};
use super::store::{safe, Store};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Files whose content changes on every read and never makes a snapshot new.
pub const VOLATILE: &[&str] = &["status"];

/// Folders and files a card snapshot leaves out: the logs (they go to the log store) and
/// what macOS writes on a FAT card.
const SKIP_DIRS: &[&str] = &[
    ".spotlight-v100",
    ".fseventsd",
    ".trashes",
    ".temporaryitems",
    ".documentrevisions-v100",
    "system volume information",
];
const SKIP_FILES: &[&str] = &[".ds_store", ".metadata_never_index"];

/// Where a snapshot is in its run, for the progress row.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct BackupProgress {
    /// `reading`, `logs`, `saving`.
    pub stage: String,
    pub files_done: u32,
    pub files_total: u32,
    /// Bytes read so far from files that changed (unchanged ones are not read).
    pub bytes_done: u64,
    /// Bytes of the files that changed.
    pub bytes_total: u64,
    /// The file being read.
    pub path: String,
}

/// How a snapshot runs.
#[derive(Default)]
pub struct TakeOptions<'a> {
    /// Checked between files: a stop leaves no manifest (and blobs the next collection
    /// removes).
    pub stop: Option<&'a AtomicBool>,
    pub progress: Option<&'a mut dyn FnMut(&BackupProgress)>,
    /// Tests: fail after the blobs, before the manifest, as a crash would.
    pub crash_before_manifest: bool,
}

impl TakeOptions<'_> {
    fn stopped(&self) -> bool {
        self.stop.is_some_and(|s| s.load(Ordering::SeqCst))
    }

    fn report(&mut self, p: &BackupProgress) {
        if let Some(f) = self.progress.as_mut() {
            f(p);
        }
    }
}

/// What a snapshot run did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct TakeReport {
    /// The new snapshot, or the latest one when nothing changed.
    pub backup: Backup,
    /// False when the files equal the latest snapshot's: nothing was written.
    pub new: bool,
    /// Files read and hashed.
    pub read: u32,
    /// Files whose size and time matched the latest snapshot: not read.
    pub skipped: u32,
    pub bytes_read: u64,
    /// What happened to the card's logs.
    pub logs: LogCounts,
}

/// Logs taken into the log store, by outcome.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct LogCounts {
    pub added: u32,
    pub grown: u32,
    pub same: u32,
    pub kept_both: u32,
    pub unchanged: u32,
}

impl LogCounts {
    fn add(&mut self, o: LogOutcome) {
        match o {
            LogOutcome::Added => self.added += 1,
            LogOutcome::Grown => self.grown += 1,
            LogOutcome::Same => self.same += 1,
            LogOutcome::KeptBoth => self.kept_both += 1,
            LogOutcome::Unchanged => self.unchanged += 1,
        }
    }

    fn merge(&mut self, o: &LogCounts) {
        self.added += o.added;
        self.grown += o.grown;
        self.same += o.same;
        self.kept_both += o.kept_both;
        self.unchanged += o.unchanged;
    }
}

/// The snapshots of a gear folder.
#[derive(Debug, Clone)]
pub struct Snapshots {
    store: Store,
}

impl Snapshots {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn blobs(&self) -> Blobs {
        Blobs::new(self.store.clone())
    }

    /// The devices with a snapshot folder.
    pub fn devices(&self) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(self.store.root().join("snapshots"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .collect();
        out.sort();
        out
    }

    /// A device's snapshots, oldest first. A manifest that does not parse is skipped, so
    /// one bad file never hides the rest; temporary files are not manifests.
    pub fn list(&self, device: &str) -> Vec<Backup> {
        let mut out: Vec<Backup> = std::fs::read_dir(self.store.snapshots_dir(device))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
            .collect();
        out.sort_by(|a: &Backup, b| a.taken_at.cmp(&b.taken_at).then(a.id.cmp(&b.id)));
        out
    }

    /// Every snapshot, oldest first.
    pub fn all(&self) -> Vec<Backup> {
        let mut out: Vec<Backup> = self.devices().iter().flat_map(|d| self.list(d)).collect();
        out.sort_by(|a, b| a.taken_at.cmp(&b.taken_at).then(a.id.cmp(&b.id)));
        out
    }

    /// The device's newest snapshot.
    pub fn latest(&self, device: &str) -> Option<Backup> {
        self.list(device).pop()
    }

    fn path_of(&self, id: &str) -> Result<PathBuf> {
        let (device, stem) = id
            .split_once('/')
            .with_context(|| format!("{id:?} is not a backup id (<device>/<name>)"))?;
        Ok(self
            .store
            .snapshots_dir(device)
            .join(format!("{}.json", safe(stem))))
    }

    /// The snapshot with this id.
    pub fn get(&self, id: &str) -> Result<Backup> {
        let p = self.path_of(id)?;
        let bytes = std::fs::read(&p).with_context(|| format!("No backup {id:?}."))?;
        serde_json::from_slice(&bytes).with_context(|| format!("backup {id:?} does not parse"))
    }

    /// Writes a manifest: a temporary name, fsync, rename. Never replaces one with
    /// another id.
    pub fn write(&self, b: &Backup) -> Result<()> {
        let p = self.path_of(&b.id)?;
        let dir = p.parent().context("manifest has no folder")?;
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let tmp = p.with_extension("json.tmp");
        let mut f = std::fs::File::create(&tmp)?;
        std::io::Write::write_all(&mut f, &serde_json::to_vec_pretty(b)?)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, &p).with_context(|| format!("writing {}", p.display()))
    }

    /// Sets or clears a snapshot's pin.
    pub fn pin(&self, id: &str, pinned: bool) -> Result<Backup> {
        let mut b = self.get(id)?;
        if b.pinned != pinned {
            b.pinned = pinned;
            self.write(&b)?;
        }
        Ok(b)
    }

    /// A free id for a device, a time and a trigger: `<device>/<stamp>-<trigger>`, with
    /// `-2`, `-3` ... when one exists.
    pub fn new_id(&self, device: &str, at: DateTime<Utc>, trigger: Trigger) -> String {
        let base = format!("{}-{}", at.format("%Y-%m-%dT%H%M%S"), trigger.slug());
        let dir = self.store.snapshots_dir(device);
        let mut stem = base.clone();
        let mut n = 2;
        while dir.join(format!("{stem}.json")).exists() {
            stem = format!("{base}-{n}");
            n += 1;
        }
        format!("{}/{}", safe(device), stem)
    }

    /// Writes a snapshot of these files unless they equal `compare_to`'s. The blobs must be
    /// stored already; the caller holds the store's lock.
    #[allow(clippy::too_many_arguments)]
    fn commit(
        &self,
        device: &str,
        identity: &Identity,
        trigger: Trigger,
        taken_at: DateTime<Utc>,
        files: Vec<BackupFile>,
        compare_to: Option<&Backup>,
        crash: bool,
    ) -> Result<(Backup, bool)> {
        if let Some(prev) = compare_to {
            if same_files(&prev.files, &files) {
                return Ok((prev.clone(), false));
            }
        }
        if crash {
            bail!("simulated crash before the manifest");
        }
        let b = Backup {
            id: self.new_id(device, taken_at, trigger),
            device: device.to_string(),
            trigger,
            taken_at,
            identity: identity.clone(),
            files,
            pinned: false,
        };
        self.write(&b)?;
        Ok((b, true))
    }

    /// Takes a snapshot of an EdgeTX card (or any card tree) at `root`, then its `LOGS/`
    /// into the log store. Files whose size and modified time match the latest snapshot are
    /// not read.
    pub fn take_card(
        &self,
        device: &str,
        identity: &Identity,
        root: &Path,
        trigger: Trigger,
        opts: &mut TakeOptions,
    ) -> Result<TakeReport> {
        let blobs = self.blobs();
        let lock = blobs.lock()?;
        let prev = self.latest(device);
        let listed = card_files(root)?;
        let known: HashMap<&str, &BackupFile> = prev
            .iter()
            .flat_map(|b| b.files.iter())
            .map(|f| (f.path.as_str(), f))
            .collect();
        let changed: Vec<&CardEntry> = listed
            .iter()
            .filter(|(p, size, mtime)| {
                !known
                    .get(p.as_str())
                    .is_some_and(|k| k.size == *size && mtime.is_some() && k.mtime == *mtime)
            })
            .collect();
        let mut prog = BackupProgress {
            stage: "reading".into(),
            files_total: changed.len() as u32,
            bytes_total: changed.iter().map(|c| c.1).sum(),
            ..Default::default()
        };
        opts.report(&prog);
        let mut files = Vec::with_capacity(listed.len());
        let (mut read, mut skipped) = (0u32, 0u32);
        for (path, size, mtime) in &listed {
            if let Some(k) = known
                .get(path.as_str())
                .filter(|k| k.size == *size && mtime.is_some() && k.mtime == *mtime)
            {
                files.push((*k).clone());
                skipped += 1;
                continue;
            }
            if opts.stopped() {
                bail!("Stopped. No backup was saved.");
            }
            prog.path = path.clone();
            opts.report(&prog);
            let base = prog.bytes_done;
            let r = {
                let report = &mut opts.progress;
                let p = &mut prog;
                blobs.put_file(&root.join(path), &mut |n| {
                    p.bytes_done = base + n;
                    if let Some(f) = report.as_mut() {
                        f(p);
                    }
                })?
            };
            prog.bytes_done = base + r.size;
            prog.files_done += 1;
            read += 1;
            files.push(BackupFile {
                path: path.clone(),
                size: r.size,
                xxh64: r.xxh64,
                mtime: *mtime,
            });
        }
        prog.stage = "saving".into();
        prog.path.clear();
        opts.report(&prog);
        let (backup, new) = self.commit(
            device,
            identity,
            trigger,
            Utc::now(),
            files,
            prev.as_ref().filter(|_| !trigger.always_kept()),
            opts.crash_before_manifest,
        )?;
        drop(lock);
        prog.stage = "logs".into();
        opts.report(&prog);
        let logs = self.take_logs(device, &root.join(logs_dir_name(root)))?;
        Ok(TakeReport {
            backup,
            new,
            read,
            skipped,
            bytes_read: prog.bytes_done,
            logs,
        })
    }

    /// Takes each log in `dir` into the radio's log store.
    pub fn take_logs(&self, radio: &str, dir: &Path) -> Result<LogCounts> {
        let logs = LogStore::new(self.store.clone());
        let mut c = LogCounts::default();
        let Ok(rd) = std::fs::read_dir(dir) else {
            return Ok(c);
        };
        let mut paths: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(is_log_name)
            })
            .collect();
        paths.sort();
        for p in paths {
            let (_, o) = logs.put_file(radio, &p)?;
            c.add(o);
        }
        Ok(c)
    }

    /// Takes a snapshot from files already read (an FC's `version`, `status`, `diff all`,
    /// `dump all`). `compare_to` is the latest snapshot unless the caller names another.
    pub fn take_files(
        &self,
        device: &str,
        identity: &Identity,
        trigger: Trigger,
        taken_at: DateTime<Utc>,
        contents: &[(String, Vec<u8>)],
        crash: bool,
    ) -> Result<TakeReport> {
        let blobs = self.blobs();
        let lock = blobs.lock()?;
        let prev = self.latest(device);
        let mut files = Vec::new();
        let mut bytes = 0;
        for (path, data) in contents {
            let r = blobs.put(data)?;
            bytes += r.size;
            files.push(BackupFile {
                path: path.clone(),
                size: r.size,
                xxh64: r.xxh64,
                mtime: None,
            });
        }
        let (backup, new) = self.commit(
            device,
            identity,
            trigger,
            taken_at,
            files,
            prev.as_ref().filter(|_| !trigger.always_kept()),
            crash,
        )?;
        drop(lock);
        Ok(TakeReport {
            backup,
            new,
            read: contents.len() as u32,
            skipped: 0,
            bytes_read: bytes,
            logs: LogCounts::default(),
        })
    }

    /// One file of a snapshot, or its file list when `path` is None.
    pub fn read(&self, id: &str, path: Option<&str>) -> Result<BackupContent> {
        let backup = self.get(id)?;
        let Some(path) = path.map(str::trim).filter(|p| !p.is_empty()) else {
            return Ok(BackupContent {
                backup,
                path: None,
                size: 0,
                text: None,
                binary: false,
            });
        };
        let f = backup
            .files
            .iter()
            .find(|f| f.path == path)
            .with_context(|| format!("Backup {id} has no file {path:?}."))?
            .clone();
        let bytes = self.blobs().get(&blob_of(&f))?;
        let text = as_text(&bytes);
        Ok(BackupContent {
            path: Some(f.path),
            size: f.size,
            binary: text.is_none(),
            text,
            backup,
        })
    }

    /// What changed from snapshot `a` to `b`: the files added, changed and removed, and a
    /// line diff of each changed text file (only `path` when given). `status` is left out
    /// unless asked for.
    pub fn diff(&self, a: &str, b: &str, path: Option<&str>) -> Result<Vec<DiffItem>> {
        let (x, y) = (self.get(a)?, self.get(b)?);
        let path = path.map(str::trim).filter(|p| !p.is_empty());
        let xs: BTreeMap<&str, &BackupFile> =
            x.files.iter().map(|f| (f.path.as_str(), f)).collect();
        let ys: BTreeMap<&str, &BackupFile> =
            y.files.iter().map(|f| (f.path.as_str(), f)).collect();
        let wanted = |p: &str| match path {
            Some(w) => p == w,
            None => !VOLATILE.contains(&p),
        };
        let mut put = Vec::new();
        let mut delete = Vec::new();
        let mut lines = Vec::new();
        let blobs = self.blobs();
        for (p, f) in &ys {
            if !wanted(p) {
                continue;
            }
            match xs.get(p) {
                None => put.push(p.to_string()),
                Some(old) if old.xxh64 != f.xxh64 || old.size != f.size => {
                    put.push(p.to_string());
                    let (ob, nb) = (blobs.get(&blob_of(old))?, blobs.get(&blob_of(f))?);
                    if let (Some(ot), Some(nt)) = (as_text(&ob), as_text(&nb)) {
                        lines.push(DiffItem::Lines {
                            label: p.to_string(),
                            lines: super::edgetx::card::line_diff(&ot, &nt),
                        });
                    }
                }
                Some(_) => {}
            }
        }
        for p in xs.keys() {
            if wanted(p) && !ys.contains_key(p) {
                delete.push(p.to_string());
            }
        }
        let mut out = Vec::new();
        if !put.is_empty() || !delete.is_empty() {
            out.push(DiffItem::Files {
                label: "Files".into(),
                put,
                delete,
            });
        }
        out.extend(lines);
        Ok(out)
    }

    /// Every blob key a manifest names, but those of the snapshots in `dropping`.
    fn manifest_keys(&self, dropping: &HashSet<String>) -> HashSet<String> {
        self.all()
            .iter()
            .filter(|b| !dropping.contains(&b.id))
            .flat_map(|b| b.files.iter().map(|f| blob_of(f).key()))
            .collect()
    }

    /// Thins every device's snapshots by the retention settings, then collects the blobs
    /// nothing names. A dry run writes and deletes nothing and reports what it would.
    pub fn prune(&self, r: &Retention, now: DateTime<Utc>, dry_run: bool) -> Result<PruneReport> {
        let blobs = self.blobs();
        let lock = blobs.lock()?;
        let mut dropped = Vec::new();
        let mut kept = 0u32;
        for d in self.devices() {
            let list = self.list(&d);
            let drop_ids = thin(&list, r, now);
            kept += (list.len() - drop_ids.len()) as u32;
            dropped.extend(drop_ids);
        }
        let dropping: HashSet<String> = dropped.iter().cloned().collect();
        let mut keep = self.manifest_keys(&dropping);
        keep.extend(change_keys(&self.store));
        let collected = if dry_run {
            let mut c = Collected::default();
            for (k, size) in blobs.list()? {
                if !keep.contains(&k) {
                    c.blobs += 1;
                    c.bytes += size;
                }
            }
            c
        } else {
            for id in &dropped {
                let p = self.path_of(id)?;
                std::fs::remove_file(&p).with_context(|| format!("removing {}", p.display()))?;
            }
            blobs.collect(&keep)?
        };
        drop(lock);
        Ok(PruneReport {
            dry_run,
            dropped,
            kept,
            collected,
        })
    }

    /// The Storage view: sizes in total and per device.
    pub fn storage(&self, devices: &[Device]) -> Result<StorageView> {
        let blobs = self.blobs().list()?;
        let blob_size: HashMap<&str, u64> = blobs.iter().map(|(k, s)| (k.as_str(), *s)).collect();
        let logs = LogStore::new(self.store.clone());
        let all = self.all();
        // Which devices name each blob.
        let mut users: HashMap<String, HashSet<&str>> = HashMap::new();
        for b in &all {
            for f in &b.files {
                users
                    .entry(blob_of(f).key())
                    .or_default()
                    .insert(b.device.as_str());
            }
        }
        let mut ids: Vec<String> = self.devices();
        for r in logs.radios() {
            if !ids.contains(&r) {
                ids.push(r);
            }
        }
        ids.sort();
        let mut out = Vec::new();
        let mut manifest_total = 0;
        let mut log_total = 0;
        for id in ids {
            let snaps: Vec<&Backup> = all.iter().filter(|b| b.device == id).collect();
            let manifest_bytes: u64 = std::fs::read_dir(self.store.snapshots_dir(&id))
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum();
            let stored_logs = logs.list(&id);
            let log_bytes: u64 = stored_logs.iter().map(|l| l.size).sum();
            let own: HashSet<String> = snaps
                .iter()
                .flat_map(|b| b.files.iter().map(|f| blob_of(f).key()))
                .filter(|k| users.get(k).is_some_and(|u| u.len() == 1))
                .collect();
            let own_blob_bytes = own.iter().filter_map(|k| blob_size.get(k.as_str())).sum();
            let mut by_trigger: BTreeMap<String, u32> = BTreeMap::new();
            for b in &snaps {
                *by_trigger.entry(b.trigger.slug().to_string()).or_default() += 1;
            }
            let saved = devices.iter().find(|d| d.id == id);
            manifest_total += manifest_bytes;
            log_total += log_bytes;
            out.push(DeviceStorage {
                name: saved.map(|d| d.display_name()),
                kind: saved.map(|d| d.kind).or_else(|| kind_of_id(&id)),
                snapshots: snaps.len() as u32,
                pinned: snaps.iter().filter(|b| b.pinned).count() as u32,
                by_trigger,
                latest: snaps.last().map(|b| b.taken_at),
                manifest_bytes,
                logs: stored_logs.len() as u32,
                log_bytes,
                own_blob_bytes,
                total_bytes: manifest_bytes + log_bytes + own_blob_bytes,
                device: id,
            });
        }
        let blob_bytes: u64 = blobs.iter().map(|(_, s)| s).sum();
        Ok(StorageView {
            gear_dir: self.store.root().to_path_buf(),
            total_bytes: blob_bytes + manifest_total + log_total,
            blob_bytes,
            blobs: blobs.len() as u32,
            shared_blob_bytes: blob_bytes
                - out
                    .iter()
                    .map(|d| d.own_blob_bytes)
                    .sum::<u64>()
                    .min(blob_bytes),
            manifest_bytes: manifest_total,
            log_bytes: log_total,
            snapshots: all.len() as u32,
            devices: out,
        })
    }

    /// Writes snapshots as plain folders: `<to>/<device>/<snapshot>/<files>`. An FC's
    /// command files become `diff_all.txt`, `dump_all.txt`, ... Never writes into a folder
    /// that exists.
    pub fn export(&self, ids: &[String], to: &Path) -> Result<ExportReport> {
        if !to.is_dir() {
            bail!("{} is not a folder.", to.display());
        }
        let blobs = self.blobs();
        let mut report = ExportReport::default();
        for id in ids {
            let b = self.get(id)?;
            let (dev, stem) = b.id.split_once('/').unwrap_or((&b.device, &b.id));
            let dir = to.join(safe(dev)).join(safe(stem));
            if dir.exists() {
                bail!(
                    "Refused: {} exists; export never writes into an existing folder.",
                    dir.display()
                );
            }
            for f in &b.files {
                let rel = export_name(&f.path)?;
                let dest = dir.join(&rel);
                std::fs::create_dir_all(dest.parent().unwrap_or(&dir))?;
                let bytes = blobs.get(&blob_of(f))?;
                std::fs::write(&dest, &bytes)
                    .with_context(|| format!("writing {}", dest.display()))?;
                report.files += 1;
                report.bytes += bytes.len() as u64;
            }
            report.folders.push(dir);
        }
        Ok(report)
    }
}

/// The blob a manifest entry names.
pub fn blob_of(f: &BackupFile) -> BlobRef {
    BlobRef {
        xxh64: f.xxh64.clone(),
        size: f.size,
    }
}

/// True when two file lists hold the same paths with the same content (modified times and
/// `VOLATILE` files aside).
pub fn same_files(a: &[BackupFile], b: &[BackupFile]) -> bool {
    let set = |l: &[BackupFile]| -> BTreeMap<String, (u64, String)> {
        l.iter()
            .filter(|f| !VOLATILE.contains(&f.path.as_str()))
            .map(|f| (f.path.clone(), (f.size, f.xxh64.clone())))
            .collect()
    };
    set(a) == set(b)
}

/// Text when the bytes are UTF-8 without NULs and at most 4 MB.
fn as_text(bytes: &[u8]) -> Option<String> {
    if bytes.len() > 4 << 20 || bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes.to_vec()).ok()
}

/// A snapshot path as a relative path for an export: FC commands get a `.txt` name; a
/// card path must stay inside its folder.
fn export_name(path: &str) -> Result<PathBuf> {
    if (!path.contains('/') && path.contains(' ')) || matches!(path, "version" | "status") {
        return Ok(PathBuf::from(format!("{}.txt", path.replace(' ', "_"))));
    }
    let p = PathBuf::from(path);
    if p.is_absolute()
        || p.components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        bail!("backup path {path:?} leaves its folder");
    }
    Ok(p)
}

/// The kind a device id names by its prefix.
pub fn kind_of_id(id: &str) -> Option<DeviceKind> {
    DeviceKind::ALL
        .into_iter()
        .filter(|k| id.starts_with(&format!("{}-", k.id_prefix())))
        .max_by_key(|k| k.id_prefix().len())
}

/// The card's log folder name (`LOGS`, in whatever case the card has it).
fn logs_dir_name(root: &Path) -> String {
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .find(|n| n.eq_ignore_ascii_case("LOGS"))
        .unwrap_or_else(|| "LOGS".into())
}

/// A card file: its path from the card's root, size and modified time.
pub type CardEntry = (String, u64, Option<DateTime<Utc>>);

/// Every file of a card tree, sorted, with size and modified time: all but `LOGS/`,
/// AppleDouble files and what macOS keeps on a card. Links are not followed.
pub fn card_files(root: &Path) -> Result<Vec<CardEntry>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<CardEntry>) -> Result<()> {
        let rd = std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let lower = name.to_ascii_lowercase();
            let ft = e.file_type()?;
            if ft.is_symlink() || name.starts_with("._") {
                continue;
            }
            let path = e.path();
            if ft.is_dir() {
                if SKIP_DIRS.contains(&lower.as_str()) || (dir == root && lower == "logs") {
                    continue;
                }
                walk(root, &path, out)?;
            } else if ft.is_file() && !SKIP_FILES.contains(&lower.as_str()) {
                let m = e.metadata()?;
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().to_string())
                    .collect::<Vec<_>>()
                    .join("/");
                out.push((rel, m.len(), m.modified().ok().map(DateTime::<Utc>::from)));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// The blob keys staged changes name: any object with `xxh64` and `size` in a JSON file
/// under `<gear>/changes/`.
pub fn change_keys(store: &Store) -> HashSet<String> {
    fn scan(v: &serde_json::Value, out: &mut HashSet<String>) {
        match v {
            serde_json::Value::Object(m) => {
                if let (Some(h), Some(s)) = (
                    m.get("xxh64").and_then(|x| x.as_str()),
                    m.get("size").and_then(|x| x.as_u64()),
                ) {
                    out.insert(super::blobs::key(h, s));
                }
                m.values().for_each(|x| scan(x, out));
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| scan(x, out)),
            _ => {}
        }
    }
    fn walk(dir: &Path, out: &mut HashSet<String>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "json") {
                if let Ok(v) = serde_json::from_slice(&std::fs::read(&p).unwrap_or_default()) {
                    scan(&v, out);
                }
            }
        }
    }
    let mut out = HashSet::new();
    walk(&store.changes_dir(), &mut out);
    // The sounds a person rendered for one line are kept in the blob store, named in gear.json.
    if let Some(v) = store.read().ok().and_then(|v| v.get("voice").cloned()) {
        scan(&v, &mut out);
    }
    out
}

// ----- retention -----

/// The retention settings (`gearKeepRecent`, `gearKeepWeeks`, `gearKeepMonthly`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    pub keep_recent: u32,
    pub keep_weeks: u32,
    pub keep_monthly: bool,
}

impl From<&super::GearSettings> for Retention {
    fn from(s: &super::GearSettings) -> Self {
        Self {
            keep_recent: s.keep_recent,
            keep_weeks: s.keep_weeks,
            keep_monthly: s.keep_monthly,
        }
    }
}

/// The snapshots of one device that retention drops. Kept: apply and flash snapshots,
/// pinned ones, the newest `keep_recent` of the rest; then the newest in each week for
/// `keep_weeks` weeks back from `now`; then, with `keep_monthly`, the newest in each
/// month. The newest snapshot is always kept.
pub fn thin(list: &[Backup], r: &Retention, now: DateTime<Utc>) -> Vec<String> {
    let mut rest: Vec<&Backup> = list
        .iter()
        .filter(|b| !b.pinned && !b.trigger.always_kept())
        .collect();
    rest.sort_by_key(|b| std::cmp::Reverse(b.taken_at));
    let newest = list.iter().max_by_key(|b| b.taken_at).map(|b| b.id.clone());
    let mut weeks = HashSet::new();
    let mut months = HashSet::new();
    let mut drop = Vec::new();
    for (i, b) in rest.into_iter().enumerate() {
        if i < r.keep_recent as usize || Some(&b.id) == newest.as_ref() {
            continue;
        }
        let age_days = (now - b.taken_at).num_days().max(0);
        let local = b.taken_at.with_timezone(&Local);
        if age_days < 7 * r.keep_weeks as i64 {
            let w = local.iso_week();
            if weeks.insert((w.year(), w.week())) {
                continue;
            }
        } else if r.keep_monthly && months.insert((local.year(), local.month())) {
            continue;
        }
        drop.push(b.id.clone());
    }
    drop
}

// ----- answers -----

/// One file of a backup, or the backup's file list.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BackupContent {
    pub backup: Backup,
    /// The file read; None for the list.
    pub path: Option<String>,
    pub size: u64,
    /// The file as text; None for a binary file.
    pub text: Option<String>,
    pub binary: bool,
}

/// What a prune did, or would do.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct PruneReport {
    pub dry_run: bool,
    /// The snapshots dropped (or that would be).
    pub dropped: Vec<String>,
    pub kept: u32,
    pub collected: Collected,
}

/// Sizes in the gear folder.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct StorageView {
    pub gear_dir: PathBuf,
    /// Blobs, manifests and logs.
    pub total_bytes: u64,
    pub blobs: u32,
    pub blob_bytes: u64,
    /// Blobs that more than one device names.
    pub shared_blob_bytes: u64,
    pub manifest_bytes: u64,
    pub log_bytes: u64,
    pub snapshots: u32,
    pub devices: Vec<DeviceStorage>,
}

/// One device's share of the gear folder.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DeviceStorage {
    pub device: String,
    /// The saved device's name; None when QuadCam no longer lists it.
    pub name: Option<String>,
    pub kind: Option<DeviceKind>,
    pub snapshots: u32,
    pub pinned: u32,
    /// Snapshots per trigger (`connect`, `manual`, `before_apply`, `import`, ...).
    pub by_trigger: BTreeMap<String, u32>,
    pub latest: Option<DateTime<Utc>>,
    pub manifest_bytes: u64,
    pub logs: u32,
    pub log_bytes: u64,
    /// Blobs only this device's snapshots name.
    pub own_blob_bytes: u64,
    /// Manifests, logs and its own blobs.
    pub total_bytes: u64,
}

/// What an export wrote.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct ExportReport {
    /// One folder per snapshot.
    pub folders: Vec<PathBuf>,
    pub files: u32,
    pub bytes: u64,
}

// ----- import of old backup folders -----

/// What an import found in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BackupImportKind {
    /// An EdgeTX card copy.
    Card,
    /// A Betaflight `diff all` and/or `dump all`.
    Fc,
    /// A plain `LOGS/` folder.
    Logs,
}

/// What happened to one found item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BackupImportOutcome {
    /// A new snapshot (or, in a dry run, one would be written).
    Imported,
    /// Its files equal a snapshot the device has from that day or before: nothing written.
    Same,
    /// Logs only.
    Logs,
    /// Not taken; `reason` says why.
    Skipped,
}

/// One item an import found.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BackupImportItem {
    /// The folder (a card copy, a `LOGS/` folder) or the first FC file.
    pub path: PathBuf,
    pub kind: BackupImportKind,
    pub outcome: BackupImportOutcome,
    /// The device it went to.
    pub device: Option<String>,
    pub identity: Identity,
    pub taken_at: Option<DateTime<Utc>>,
    /// The snapshot written, or the one it equals.
    pub backup: Option<String>,
    pub files: u32,
    pub logs: LogCounts,
    /// Why it was skipped, or which snapshot it equals.
    pub reason: Option<String>,
}

/// `gear_import_backups`' answer.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ImportBackupsReport {
    pub folder: PathBuf,
    pub dry_run: bool,
    pub items: Vec<BackupImportItem>,
    /// Snapshots written (or that would be).
    pub imported: u32,
    pub same: u32,
    pub skipped: u32,
    pub logs: LogCounts,
    /// Devices the import saved because it learned their id (an FC's MCU id).
    pub new_devices: Vec<String>,
}

/// How deep an import looks under the folder.
const IMPORT_DEPTH: usize = 6;
/// FC text files larger than this are not read.
const FC_TEXT_MAX: u64 = 4 << 20;

/// A found item before it is matched to a device.
#[derive(Debug, Clone)]
struct Found {
    kind: BackupImportKind,
    path: PathBuf,
    /// For a card copy: the card's root. For logs: the folder.
    root: PathBuf,
    /// For FC: `(command, file)` pairs.
    fc_files: Vec<(String, PathBuf)>,
    identity: Identity,
    /// The id the item names on its own (an FC's MCU id, a card's QuadCam marker).
    own_id: Option<String>,
    taken_at: DateTime<Utc>,
}

/// Imports an old backup folder: card copies (a folder with `RADIO/radio.yml` or
/// `MODELS/`), FC files (`<stem>.diff_all.txt`, `<stem>.dump_all.txt`, or any text file
/// that reads as a Betaflight `diff all` or `dump all`; a pair in one folder is one
/// snapshot) and plain `LOGS/` folders. Each is dated from a `YYYY-MM-DD` in its folder
/// names, else its newest file.
///
/// The device: an FC's MCU id, or a card's QuadCam marker, names it; else the one saved
/// device of that kind and board; else `device` when it is of that kind. Two candidates,
/// or none, skip the item with the reason. The source folder is never changed.
pub fn import(
    snaps: &Snapshots,
    folder: &Path,
    known: &[Device],
    device: Option<&Device>,
    dry_run: bool,
) -> Result<ImportBackupsReport> {
    if !folder.is_dir() {
        bail!("{} is not a folder.", folder.display());
    }
    let mut found = Vec::new();
    find(folder, folder, 0, &mut found)?;
    found.sort_by(|a, b| a.taken_at.cmp(&b.taken_at).then(a.path.cmp(&b.path)));
    let mut report = ImportBackupsReport {
        folder: folder.to_path_buf(),
        dry_run,
        items: Vec::new(),
        imported: 0,
        same: 0,
        skipped: 0,
        logs: LogCounts::default(),
        new_devices: Vec::new(),
    };
    // Snapshots this run would write, per device (a dry run compares with them too).
    let mut planned: HashMap<String, Vec<Backup>> = HashMap::new();
    for f in found {
        let mut item = BackupImportItem {
            path: f.path.clone(),
            kind: f.kind,
            outcome: BackupImportOutcome::Skipped,
            device: None,
            identity: f.identity.clone(),
            taken_at: Some(f.taken_at),
            backup: None,
            files: 0,
            logs: LogCounts::default(),
            reason: None,
        };
        let dev_kind = if f.kind == BackupImportKind::Fc {
            DeviceKind::Fc
        } else {
            DeviceKind::Radio
        };
        let dev = match pick_device(&f, dev_kind, known, device) {
            Ok(d) => d,
            Err(reason) => {
                item.reason = Some(reason);
                report.skipped += 1;
                report.items.push(item);
                continue;
            }
        };
        item.device = Some(dev.clone());
        if f.kind == BackupImportKind::Fc
            && !known.iter().any(|d| d.id == dev)
            && !report.new_devices.contains(&dev)
        {
            report.new_devices.push(dev.clone());
        }
        // Logs go to the log store; a dry run only counts them.
        let logs_dir = match f.kind {
            BackupImportKind::Card => Some(f.root.join(logs_dir_name(&f.root))),
            BackupImportKind::Logs => Some(f.root.clone()),
            BackupImportKind::Fc => None,
        };
        if let Some(ld) = &logs_dir {
            if dry_run {
                item.logs.added = std::fs::read_dir(ld)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| e.file_name().to_str().is_some_and(is_log_name))
                    .count() as u32;
            } else {
                item.logs = snaps.take_logs(&dev, ld)?;
            }
            report.logs.merge(&item.logs);
        }
        if f.kind == BackupImportKind::Logs {
            item.outcome = BackupImportOutcome::Logs;
            report.items.push(item);
            continue;
        }
        // The files, hashed (and stored unless dry run), under the store's lock until the
        // manifest is written.
        let lock = if dry_run {
            None
        } else {
            Some(snaps.blobs().lock()?)
        };
        let files = match import_files(snaps, &f, dry_run) {
            Ok(x) => x,
            Err(e) => {
                item.reason = Some(format!("{e:#}"));
                report.skipped += 1;
                report.items.push(item);
                continue;
            }
        };
        item.files = files.len() as u32;
        // Same as a snapshot of that day, or the newest one before it: nothing to write.
        let existing: Vec<Backup> = snaps
            .list(&dev)
            .into_iter()
            .chain(planned.get(&dev).cloned().unwrap_or_default())
            .collect();
        let day = f.taken_at.with_timezone(&Local).date_naive();
        let before = existing
            .iter()
            .filter(|b| b.taken_at <= f.taken_at)
            .max_by_key(|b| b.taken_at);
        let twin = existing
            .iter()
            .filter(|b| b.taken_at.with_timezone(&Local).date_naive() == day)
            .chain(before)
            .find(|b| same_files(&b.files, &files));
        if let Some(t) = twin {
            item.outcome = BackupImportOutcome::Same;
            item.backup = Some(t.id.clone());
            item.reason = Some(format!("Same as backup {}.", t.id));
            report.same += 1;
            report.items.push(item);
            continue;
        }
        let b = if dry_run {
            Backup {
                id: snaps.new_id(&dev, f.taken_at, Trigger::Import),
                device: dev.clone(),
                trigger: Trigger::Import,
                taken_at: f.taken_at,
                identity: f.identity.clone(),
                files,
                pinned: false,
            }
        } else {
            let (b, _) = snaps.commit(
                &dev,
                &f.identity,
                Trigger::Import,
                f.taken_at,
                files,
                None,
                false,
            )?;
            b
        };
        drop(lock);
        item.backup = Some(b.id.clone());
        item.outcome = BackupImportOutcome::Imported;
        report.imported += 1;
        planned.entry(dev).or_default().push(b);
        report.items.push(item);
    }
    Ok(report)
}

/// The device an item goes to, or why none.
fn pick_device(
    f: &Found,
    kind: DeviceKind,
    known: &[Device],
    device: Option<&Device>,
) -> std::result::Result<String, String> {
    if let Some(id) = &f.own_id {
        return Ok(id.clone());
    }
    if let Some(d) = device.filter(|d| d.kind == kind) {
        return Ok(d.id.clone());
    }
    let board = f.identity.board.as_deref();
    let same: Vec<&Device> = known
        .iter()
        .filter(|d| d.kind == kind)
        .filter(|d| match (board, d.identity.board.as_deref()) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            (None, _) => true,
            (Some(_), None) => false,
        })
        .collect();
    let what = format!(
        "{}{}",
        kind.label(),
        board
            .map(|b| format!(" with board {b}"))
            .unwrap_or_default()
    );
    match same.as_slice() {
        [one] => Ok(one.id.clone()),
        [] => Err(format!(
            "No saved {what}. Plug the device in once so QuadCam knows it, or pass a device."
        )),
        many => Err(format!(
            "{} saved devices could match ({}): pass a device.",
            many.len(),
            many.iter()
                .map(|d| format!("{} {}", d.id, d.display_name()))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// An item's files as manifest entries, stored in the blob store (hashed only in a dry
/// run).
fn import_files(snaps: &Snapshots, f: &Found, dry_run: bool) -> Result<Vec<BackupFile>> {
    let blobs = snaps.blobs();
    let entry = |path: String, abs: &Path, mtime| -> Result<BackupFile> {
        let r = if dry_run {
            let b = std::fs::read(abs).with_context(|| format!("reading {}", abs.display()))?;
            BlobRef {
                xxh64: super::blobs::hash(&b),
                size: b.len() as u64,
            }
        } else {
            blobs.put_file(abs, &mut |_| {})?
        };
        Ok(BackupFile {
            path,
            size: r.size,
            xxh64: r.xxh64,
            mtime,
        })
    };
    let mut out = Vec::new();
    match f.kind {
        BackupImportKind::Card => {
            for (rel, _, mtime) in card_files(&f.root)? {
                out.push(entry(rel.clone(), &f.root.join(&rel), mtime)?);
            }
        }
        BackupImportKind::Fc => {
            for (cmd, p) in &f.fc_files {
                out.push(entry(cmd.clone(), p, None)?);
            }
        }
        BackupImportKind::Logs => {}
    }
    Ok(out)
}

/// True for a folder that is an EdgeTX card copy.
fn is_card(dir: &Path) -> bool {
    dir.join(RADIO_FILE).is_file() || child_dir(dir, "MODELS").is_some()
}

fn child_dir(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|e| {
            e.path().is_dir()
                && e.file_name()
                    .to_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .map(|e| e.path())
}

fn find(root: &Path, dir: &Path, depth: usize, out: &mut Vec<Found>) -> Result<()> {
    if is_card(dir) {
        let radio = std::fs::read(dir.join(RADIO_FILE)).unwrap_or_default();
        let identity = if radio.is_empty() {
            Identity {
                firmware: Some("EdgeTX".into()),
                ..Default::default()
            }
        } else {
            identity_from_radio_yml(&radio)
        };
        out.push(Found {
            kind: BackupImportKind::Card,
            path: dir.to_path_buf(),
            root: dir.to_path_buf(),
            fc_files: Vec::new(),
            own_id: read_marker(dir).map(|raw| device_id(DeviceKind::Radio, &raw)),
            taken_at: date_of(root, dir).unwrap_or_else(|| newest(dir)),
            identity,
        });
        return Ok(());
    }
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if dir != root && name.eq_ignore_ascii_case("LOGS") {
        out.push(Found {
            kind: BackupImportKind::Logs,
            path: dir.to_path_buf(),
            root: dir.to_path_buf(),
            fc_files: Vec::new(),
            identity: Identity {
                firmware: Some("EdgeTX".into()),
                ..Default::default()
            },
            own_id: None,
            taken_at: date_of(root, dir).unwrap_or_else(|| newest(dir)),
        });
        return Ok(());
    }
    let mut subdirs = Vec::new();
    let mut texts = Vec::new();
    for e in std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .flatten()
    {
        let ft = e.file_type()?;
        let n = e.file_name().to_string_lossy().to_string();
        if n.starts_with('.') || ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            subdirs.push(e.path());
        } else if ft.is_file() && n.to_ascii_lowercase().ends_with(".txt") {
            texts.push(e.path());
        }
    }
    texts.sort();
    // FC files, grouped by the name without its diff/dump part.
    let mut groups: BTreeMap<String, Vec<(String, PathBuf, crate::gear::bf::dump::Config)>> =
        BTreeMap::new();
    for p in texts {
        if std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > FC_TEXT_MAX {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Some(cmd) = fc_command(&p, &text) else {
            continue;
        };
        groups.entry(fc_stem(&p)).or_default().push((
            cmd,
            p,
            crate::gear::bf::dump::Config::parse(&text),
        ));
    }
    for (_, files) in groups {
        let mut fc_files: Vec<(String, PathBuf)> = Vec::new();
        let mut identity = Identity::default();
        let mut uid = None;
        for (cmd, p, c) in &files {
            if fc_files.iter().any(|(x, _)| x == cmd) {
                continue;
            }
            super::store::merge_identity(&mut identity, &c.identity());
            uid = uid.or_else(|| c.mcu_id());
            fc_files.push((cmd.clone(), p.clone()));
        }
        let own_id = crate::gear::bf::id_source(uid.as_deref(), None, None)
            .map(|(raw, _)| device_id(DeviceKind::Fc, &raw));
        let first = fc_files[0].1.clone();
        out.push(Found {
            kind: BackupImportKind::Fc,
            taken_at: date_of(root, &first).unwrap_or_else(|| newest_of(&fc_files)),
            path: first,
            root: dir.to_path_buf(),
            fc_files,
            identity,
            own_id,
        });
    }
    if depth < IMPORT_DEPTH {
        subdirs.sort();
        for d in subdirs {
            find(root, &d, depth + 1, out)?;
        }
    }
    Ok(())
}

/// `diff all` or `dump all` when a text file is one: by its first line, else its name,
/// else how many `set` lines it has. None for a file that is not Betaflight CLI output.
fn fc_command(path: &Path, text: &str) -> Option<String> {
    let is_bf = text.lines().take(40).any(|l| {
        l.starts_with("# Betaflight /") || l.starts_with("board_name ") || l == "# version"
    });
    if !is_bf {
        return None;
    }
    let first = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let cmd = if first.starts_with("dump") || first.starts_with("# dump") {
        "dump all"
    } else if first.starts_with("diff") || first.starts_with("# diff") {
        "diff all"
    } else if name.contains("dump") {
        "dump all"
    } else if name.contains("diff") {
        "diff all"
    } else if text.lines().filter(|l| l.starts_with("set ")).count() > 300 {
        "dump all"
    } else {
        "diff all"
    };
    Some(cmd.to_string())
}

/// A file name without its `diff`/`dump` part and extension: `quad.diff_all.txt` and
/// `quad.dump_all.txt` share `quad`.
fn fc_stem(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut s = name.trim_end_matches(".txt").to_string();
    for t in [
        "diff_all", "dump_all", "diff-all", "dump-all", "diff all", "dump all", "diffall",
        "dumpall", "diff", "dump",
    ] {
        if let Some(i) = s.rfind(t) {
            s.replace_range(i..i + t.len(), "");
            break;
        }
    }
    s.trim_matches(|c: char| c == '.' || c == '_' || c == '-' || c == ' ')
        .to_string()
}

/// The first `YYYY-MM-DD` in the path's names from `path` up to `root`, at local noon.
fn date_of(root: &Path, path: &Path) -> Option<DateTime<Utc>> {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let mut names: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    names.reverse();
    if let Some(r) = root.file_name() {
        names.push(r.to_string_lossy().to_string());
    }
    names.iter().find_map(|n| find_date(n)).map(|d| {
        let noon = d.and_hms_opt(12, 0, 0).unwrap();
        Local
            .from_local_datetime(&noon)
            .earliest()
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(|| Utc.from_utc_datetime(&noon))
    })
}

/// The first `YYYY-MM-DD` in a name.
pub fn find_date(s: &str) -> Option<NaiveDate> {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(9)).find_map(|i| {
        let w = &s.get(i..i + 10)?;
        let ok = w.bytes().enumerate().all(|(j, c)| match j {
            4 | 7 => c == b'-',
            _ => c.is_ascii_digit(),
        });
        let before_ok = i == 0 || !b[i - 1].is_ascii_digit();
        let after_ok = b.get(i + 10).is_none_or(|c| !c.is_ascii_digit());
        (ok && before_ok && after_ok)
            .then(|| NaiveDate::parse_from_str(w, "%Y-%m-%d").ok())
            .flatten()
    })
}

/// The newest modified time under a folder (now when it holds nothing).
fn newest(dir: &Path) -> DateTime<Utc> {
    card_files(dir)
        .ok()
        .and_then(|l| l.into_iter().filter_map(|(_, _, t)| t).max())
        .unwrap_or_else(Utc::now)
}

fn newest_of(files: &[(String, PathBuf)]) -> DateTime<Utc> {
    files
        .iter()
        .filter_map(|(_, p)| std::fs::metadata(p).ok()?.modified().ok())
        .map(DateTime::<Utc>::from)
        .max()
        .unwrap_or_else(Utc::now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn backup(id: &str, at: DateTime<Utc>, trigger: Trigger, pinned: bool) -> Backup {
        Backup {
            id: id.into(),
            device: "d".into(),
            trigger,
            taken_at: at,
            identity: Identity::default(),
            files: Vec::new(),
            pinned,
        }
    }

    #[test]
    fn thin_keeps_recent_weekly_monthly_and_the_protected() {
        let now = Utc.with_ymd_and_hms(2026, 10, 7, 12, 0, 0).unwrap();
        let r = Retention {
            keep_recent: 3,
            keep_weeks: 2,
            keep_monthly: true,
        };
        // One a day for 120 days, plus an old apply snapshot and an old pinned one.
        let mut list: Vec<Backup> = (0..120)
            .map(|d| {
                backup(
                    &format!("d/{d}"),
                    now - Duration::days(d),
                    Trigger::Connect,
                    false,
                )
            })
            .collect();
        list.push(backup(
            "d/apply",
            now - Duration::days(300),
            Trigger::BeforeApply,
            false,
        ));
        list.push(backup(
            "d/pin",
            now - Duration::days(200),
            Trigger::Manual,
            true,
        ));
        let drop: HashSet<String> = thin(&list, &r, now).into_iter().collect();
        let kept: Vec<&Backup> = list.iter().filter(|b| !drop.contains(&b.id)).collect();
        assert!(!drop.contains("d/apply") && !drop.contains("d/pin"));
        for d in 0..3 {
            assert!(!drop.contains(&format!("d/{d}")), "recent {d} kept");
        }
        // Inside two weeks: at most one per ISO week beyond the recent three.
        let in_weeks = kept
            .iter()
            .filter(|b| b.trigger == Trigger::Connect && (now - b.taken_at).num_days() >= 3)
            .filter(|b| (now - b.taken_at).num_days() < 14)
            .count();
        assert!((2..=3).contains(&in_weeks), "{in_weeks}");
        // Older: one per month.
        let old: Vec<(i32, u32)> = kept
            .iter()
            .filter(|b| b.trigger == Trigger::Connect && (now - b.taken_at).num_days() >= 14)
            .map(|b| {
                let l = b.taken_at.with_timezone(&Local);
                (l.year(), l.month())
            })
            .collect();
        let uniq: HashSet<_> = old.iter().collect();
        assert_eq!(old.len(), uniq.len(), "one per month");
        assert!(old.len() >= 3);
        // Without monthly, everything past the weeks goes.
        let r2 = Retention {
            keep_monthly: false,
            ..r
        };
        let drop2 = thin(&list, &r2, now);
        assert!(drop2.len() > drop.len());
        assert!(!drop2.contains(&"d/pin".to_string()));
    }

    #[test]
    fn dates_and_stems() {
        assert_eq!(
            find_date("backup 2026-05-01 radio"),
            NaiveDate::from_ymd_opt(2026, 5, 1)
        );
        assert_eq!(find_date("12026-05-01"), None);
        assert_eq!(find_date("2026-13-01"), None);
        assert_eq!(find_date("x"), None);
        assert_eq!(fc_stem(Path::new("/a/quad.diff_all.txt")), "quad");
        assert_eq!(fc_stem(Path::new("/a/quad.dump_all.txt")), "quad");
        assert_eq!(
            fc_stem(Path::new("/a/BTFL_cli_quad_diff.txt")),
            "btfl_cli_quad"
        );
        assert_eq!(
            export_name("diff all").unwrap(),
            PathBuf::from("diff_all.txt")
        );
        assert_eq!(
            export_name("MODELS/model01.yml").unwrap(),
            PathBuf::from("MODELS/model01.yml")
        );
        assert!(export_name("../x").is_err());
        assert_eq!(kind_of_id("elrs-tx-00ff"), Some(DeviceKind::ElrsTx));
        assert_eq!(kind_of_id("radio-00ff"), Some(DeviceKind::Radio));
    }
}
