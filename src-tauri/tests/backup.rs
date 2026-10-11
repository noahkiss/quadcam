//! Backups and the backup store (design 7.1, WP4): snapshots of a synthetic radio card and
//! of `FakeFc`, retention and collection, the log store, the import of an old backup folder
//! built here, a crash between blobs and manifest, and the card check with its repair on a
//! fake `diskutil`. Every card and folder is temporary; no real disk, port or card is
//! touched.

use chrono::{Duration as Days, TimeZone, Utc};
use quadcam_lib::core::{
    backup_hooks, BackupDiffParams, BackupFilter, BackupParams, BackupPinParams, BackupReadParams,
    CardCheckParams, CardRepairParams, Core, ExportParams, ImportBackupsParams, NoHooks, OsdParams,
    PruneParams,
};
use quadcam_lib::disk::{DiskInfo, Volume};
use quadcam_lib::gear::backup::{self, BackupImportOutcome, Retention, Snapshots, TakeOptions};
use quadcam_lib::gear::bf::cli::Timing;
use quadcam_lib::gear::bf::fake::FakeFc;
use quadcam_lib::gear::blobs::Blobs;
use quadcam_lib::gear::cues::{CueService, RecordedCues};
use quadcam_lib::gear::edgetx::synth::{self, SynthCard};
use quadcam_lib::gear::health::{self, CheckState, FakeDisk, RunOutput};
use quadcam_lib::gear::model::{Device, DeviceKind, DiffItem, Identity, Trigger};
use quadcam_lib::gear::serial::{FakePorts, Ports};
use quadcam_lib::gear::store::Store;
use quadcam_lib::gear::Env;
use quadcam_lib::mcp::{LocalBackend, Server};
use quadcam_lib::photos::Recorder;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const PORT: &str = "/dev/cu.usbmodemFAKE1";
const G473_DUMP: &str = include_str!("fixtures/bf/g473-2025.12.5.dump_all.txt");
const G473_DIFF: &str = include_str!("fixtures/bf/g473-2025.12.5.diff_all.txt");
const UID: [u8; 12] = [
    0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90, 0xa0, 0xb0, 0xc0,
];

struct Bench {
    core: Arc<Core>,
    cues: Arc<RecordedCues>,
    disk: Arc<FakeDisk>,
    /// Whole disks unmounted (`diskutil unmountDisk`, faked).
    unmounts: Arc<std::sync::Mutex<Vec<String>>>,
    /// Makes the next unmounts fail.
    unmount_fails: Arc<AtomicBool>,
    dir: tempfile::TempDir,
}

impl Bench {
    fn gear(&self) -> PathBuf {
        self.dir.path().join("support/gear")
    }
    fn store(&self) -> Store {
        Store::new(self.gear())
    }
    fn snaps(&self) -> Snapshots {
        Snapshots::new(self.store())
    }
    fn card(&self) -> PathBuf {
        self.dir.path().join("CARD")
    }
}

fn radio_volume(root: &Path) -> Volume {
    Volume {
        mount: root.to_path_buf(),
        info: DiskInfo {
            volume_uuid: Some("11111111-2222-3333-4444-555555555555".into()),
            parent_whole_disk: "disk42".into(),
            bus_protocol: Some("USB".into()),
            removable: true,
            ..Default::default()
        },
        is_card: false,
        source: None,
        is_radio: true,
        warnings: vec![],
    }
}

/// A core on a temp support folder with a synthetic radio card mounted (unless `card` is
/// false), these serial ports and this fake `diskutil`.
fn bench_with(card: bool, ports: Arc<dyn Ports>, disk: FakeDisk) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("CARD");
    let vols = if card {
        synth::write_card(&root, &SynthCard::default()).unwrap();
        vec![radio_volume(&root)]
    } else {
        vec![]
    };
    let cues = Arc::new(RecordedCues::default());
    let disk = Arc::new(disk);
    let mut env = Env::fake(vols, ports);
    env.cues = Arc::new(CueService::inline(cues.clone()));
    env.disk = disk.clone();
    let unmounts: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let unmount_fails = Arc::new(AtomicBool::new(false));
    let (u, f) = (unmounts.clone(), unmount_fails.clone());
    env.unmount = Arc::new(move |d| {
        u.lock().unwrap().push(d.to_string());
        if f.load(Ordering::SeqCst) {
            anyhow::bail!("Unmount of {d} failed: at least one volume could not be unmounted")
        }
        Ok(())
    });
    let core = Arc::new(
        Core::new(
            dir.path().join("cache"),
            None,
            Arc::new(NoHooks),
            Arc::new(Recorder::default()),
        )
        .with_settings(dir.path().join("support/settings.json"))
        .with_gear_env(env)
        .with_fc_timing(Timing::fast()),
    );
    Bench {
        core,
        cues,
        disk,
        unmounts,
        unmount_fails,
        dir,
    }
}

fn bench() -> Bench {
    bench_with(true, Arc::new(FakePorts::new(vec![])), FakeDisk::ok())
}

/// Every file under the gear folder's blobs and snapshots, with size and modified time.
fn store_files(gear: &Path) -> BTreeMap<String, (u64, std::time::SystemTime)> {
    let mut out = BTreeMap::new();
    for top in ["blobs", "snapshots", "logs"] {
        let mut stack = vec![gear.join(top)];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.file_name().unwrap() != "store.lock" {
                    let m = e.metadata().unwrap();
                    out.insert(
                        p.strip_prefix(gear).unwrap().display().to_string(),
                        (m.len(), m.modified().unwrap()),
                    );
                }
            }
        }
    }
    out
}

fn blob_count(b: &Bench) -> usize {
    Blobs::new(b.store()).list().unwrap().len()
}

fn by_mount(b: &Bench) -> BackupParams {
    BackupParams {
        mount: Some(b.card()),
        ..Default::default()
    }
}

#[test]
fn a_second_snapshot_of_an_unchanged_card_writes_nothing() {
    let b = bench();
    let first = b.core.gear_backup(&by_mount(&b)).unwrap();
    assert!(first.report.new);
    assert_eq!(first.kind, DeviceKind::Radio);
    // The card's free space is recorded with the backup.
    let seen = b.core.gear_devices().unwrap()[0].last_space.clone();
    assert!(seen.is_some_and(|s| s.free > 0));
    assert!(first.report.read >= 5, "{:?}", first.report);
    assert_eq!(first.report.logs.added, 2);
    assert!(
        first
            .report
            .backup
            .files
            .iter()
            .all(|f| !f.path.starts_with("LOGS/")),
        "logs are not snapshot content"
    );
    let saved = b.core.gear_devices().unwrap();
    assert_eq!(saved.len(), 1, "the radio is saved as a device");
    assert_eq!(
        saved[0].last_backup.as_deref(),
        Some(first.report.backup.id.as_str())
    );
    let before = store_files(&b.gear());

    let second = b.core.gear_backup(&by_mount(&b)).unwrap();
    assert!(!second.report.new);
    assert_eq!(second.report.read, 0, "unchanged files are not read");
    assert_eq!(second.report.skipped, first.report.read);
    assert_eq!(second.report.logs.unchanged, 2);
    assert_eq!(second.report.backup.id, first.report.backup.id);
    assert_eq!(store_files(&b.gear()), before, "nothing written");
    assert_eq!(b.snaps().list(&first.device).len(), 1);
    // Each backup is one job with one cue ("The Radio", then its saved name).
    assert_eq!(b.cues.spoken().len(), 2);
}

/// Edits `MODELS/model01.yml` in place to the same length, and gives it `mtime`.
fn same_length_edit(b: &Bench, mtime: std::time::SystemTime) -> Vec<u8> {
    let model = b.card().join("MODELS/model01.yml");
    let mut bytes = std::fs::read(&model).unwrap();
    let at = bytes.iter().position(u8::is_ascii_digit).unwrap();
    bytes[at] = if bytes[at] == b'5' { b'6' } else { b'5' };
    std::fs::write(&model, &bytes).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&model)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    bytes
}

fn model_blob(b: &Bench, r: &quadcam_lib::core::BackupResult) -> Option<Vec<u8>> {
    let f = r
        .report
        .backup
        .files
        .iter()
        .find(|f| f.path == "MODELS/model01.yml")?;
    b.snaps().blobs().get(&backup::blob_of(f)).ok()
}

#[test]
fn a_reset_radio_clock_makes_the_backup_read_the_files_it_wrote() {
    // A time from the radio's reset clock (before 2020) proves nothing about a file.
    let b = bench();
    let model = b.card().join("MODELS/model01.yml");
    let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(946_684_800 + 42);
    std::fs::File::options()
        .write(true)
        .open(&model)
        .unwrap()
        .set_modified(old)
        .unwrap();
    b.core.gear_backup(&by_mount(&b)).unwrap();
    let edited = same_length_edit(&b, old);
    let second = b.core.gear_backup(&by_mount(&b)).unwrap();
    assert!(second.report.new, "the same-length edit is seen");
    assert_eq!(second.report.read, 1, "only the file with the reset time");
    assert_eq!(model_blob(&b, &second).as_deref(), Some(&edited[..]));

    // A reset clock (a log from 2000, next to the older logs) makes every YAML file suspect,
    // even one whose time looks right.
    let b = bench();
    let first = b.core.gear_backup(&by_mount(&b)).unwrap();
    let kept = first
        .report
        .backup
        .files
        .iter()
        .find(|f| f.path == "MODELS/model01.yml")
        .unwrap()
        .mtime
        .unwrap();
    let edited = same_length_edit(&b, kept.into());
    // Without the reset, size and time match and the file is not read: the old bytes stay.
    let blind = b.core.gear_backup(&by_mount(&b)).unwrap();
    assert_eq!(blind.report.read, 0);
    std::fs::write(b.card().join("LOGS/Model01-2000-01-01.csv"), b"Date,Time\n").unwrap();
    let third = b.core.gear_backup(&by_mount(&b)).unwrap();
    assert!(third.report.new);
    assert_eq!(model_blob(&b, &third).as_deref(), Some(&edited[..]));
    let yaml = third
        .report
        .backup
        .files
        .iter()
        .filter(|f| f.path.ends_with(".yml"))
        .count() as u32;
    assert_eq!(third.report.read, yaml, "every YAML file, and nothing else");
}

#[test]
fn a_changed_model_file_adds_one_blob() {
    let b = bench();
    let first = b.core.gear_backup(&by_mount(&b)).unwrap();
    let blobs = blob_count(&b);
    let model = b.card().join("MODELS/model01.yml");
    let mut text = std::fs::read_to_string(&model).unwrap();
    text.push_str("  # changed\r\n");
    std::fs::write(&model, text).unwrap();
    let second = b.core.gear_backup(&by_mount(&b)).unwrap();
    assert!(second.report.new);
    assert_eq!(second.report.read, 1, "only the changed file is read");
    assert_eq!(blob_count(&b), blobs + 1);
    assert_eq!(b.snaps().list(&first.device).len(), 2);
    // The diff from the one before shows the change.
    let d = b
        .core
        .gear_backup_diff(&BackupDiffParams {
            a: second.report.backup.id.clone(),
            ..Default::default()
        })
        .unwrap();
    match &d[0] {
        DiffItem::Files { put, delete, .. } => {
            assert_eq!(put, &vec!["MODELS/model01.yml".to_string()]);
            assert!(delete.is_empty());
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(&d[1], DiffItem::Lines { label, .. } if label == "MODELS/model01.yml"));
    // Read back a file, and the list.
    let r = b
        .core
        .gear_backup_read(&BackupReadParams {
            id: second.report.backup.id.clone(),
            path: Some("MODELS/model01.yml".into()),
        })
        .unwrap();
    assert!(r.text.unwrap().ends_with("  # changed\r\n"));
    let list = b
        .core
        .gear_backups(&BackupFilter {
            device: Some(first.device.clone()),
        })
        .unwrap();
    assert_eq!(list[0].id, second.report.backup.id, "newest first");
}

#[test]
fn fc_backup_through_fakefc() {
    let fc = FakeFc::new(G473_DUMP).with_uid(UID).with_reboot_opens(0);
    let dir_locks = tempfile::tempdir().unwrap();
    let b = bench_with(
        false,
        Arc::new(fc.ports(PORT, Some(dir_locks.path().to_path_buf()))),
        FakeDisk::ok(),
    );
    let r = b
        .core
        .gear_backup(&BackupParams {
            port: Some(PORT.into()),
            ..Default::default()
        })
        .unwrap();
    assert!(r.report.new);
    assert_eq!(r.kind, DeviceKind::Fc);
    let paths: Vec<&str> = r
        .report
        .backup
        .files
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(paths, ["version", "status", "diff all", "dump all"]);
    assert_eq!(
        r.report.backup.identity.board.as_deref(),
        Some("BETAFPVG473")
    );
    assert_eq!(fc.exits(), 1, "the read ends with exit (the FC reboots)");
    let dev = b.store().device(&r.device).unwrap().unwrap();
    assert_eq!(
        dev.last_backup.as_deref(),
        Some(r.report.backup.id.as_str())
    );
    // Unchanged FC: nothing new, even though `status` is read again.
    let again = b
        .core
        .gear_backup(&BackupParams {
            device: Some(r.device.clone()),
            ..Default::default()
        })
        .unwrap();
    assert!(!again.report.new);
    // The OSD reads the device's latest backup.
    let osd = b
        .core
        .gear_osd(&OsdParams {
            device: Some(r.device.clone()),
            ..Default::default()
        })
        .unwrap();
    assert!(osd.source[0].ends_with("dump all"), "{:?}", osd.source);
}

/// A snapshot with one file of its own and one shared file.
fn snap(s: &Snapshots, at: chrono::DateTime<Utc>, trigger: Trigger, n: usize) -> String {
    s.take_files(
        "fc-test",
        &Identity::default(),
        trigger,
        at,
        &[
            ("shared".into(), b"same in every snapshot".to_vec()),
            ("diff all".into(), format!("set n = {n}\n").into_bytes()),
        ],
        false,
    )
    .unwrap()
    .backup
    .id
}

#[test]
fn retention_keeps_apply_and_pinned_and_collection_removes_only_unreferenced_blobs() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let s = Snapshots::new(store.clone());
    let now = Utc.with_ymd_and_hms(2026, 10, 7, 12, 0, 0).unwrap();
    let apply = snap(&s, now - Days::days(400), Trigger::BeforeApply, 1000);
    let pinned = snap(&s, now - Days::days(300), Trigger::Manual, 1001);
    s.pin(&pinned, true).unwrap();
    let mut ids = Vec::new();
    for d in (0..90).rev() {
        ids.push(snap(&s, now - Days::days(d), Trigger::Connect, d as usize));
    }
    // A staged change names a blob no manifest names.
    let staged = Blobs::new(store.clone()).put(b"staged bytes").unwrap();
    std::fs::create_dir_all(store.changes_dir().join("c1")).unwrap();
    std::fs::write(
        store.changes_dir().join("c1/change.json"),
        serde_json::to_vec(&serde_json::json!({"edits": [{"kind": "card_files",
            "put": [{"path": "SOUNDS/x.wav", "xxh64": staged.xxh64, "size": staged.size}]}]}))
        .unwrap(),
    )
    .unwrap();
    let r = Retention {
        keep_recent: 5,
        keep_weeks: 4,
        keep_monthly: true,
    };
    let dry = s.prune(&r, now, true).unwrap();
    assert_eq!(s.all().len(), 92, "a dry run deletes nothing");
    let report = s.prune(&r, now, false).unwrap();
    assert_eq!(report.dropped, dry.dropped);
    assert_eq!(report.collected.blobs, dry.collected.blobs);
    let left: Vec<String> = s.all().into_iter().map(|b| b.id).collect();
    assert!(left.contains(&apply) && left.contains(&pinned));
    for id in ids.iter().rev().take(5) {
        assert!(left.contains(id), "recent {id}");
    }
    assert!(left.len() < 40 && left.len() > 8, "{}", left.len());
    // Each dropped snapshot's own blob went; the shared blob, the kept snapshots' blobs and
    // the staged change's blob stay and verify.
    let blobs = Blobs::new(store.clone());
    assert_eq!(report.collected.blobs as usize, report.dropped.len());
    assert!(blobs.verify(&staged));
    for b in s.all() {
        for f in &b.files {
            assert!(blobs.verify(&backup::blob_of(f)), "{} {}", b.id, f.path);
        }
    }
    assert_eq!(blobs.list().unwrap().len(), left.len() + 2);
    // A second prune finds nothing.
    let again = s.prune(&r, now, false).unwrap();
    assert!(again.dropped.is_empty());
    assert_eq!(again.collected.blobs, 0);
}

#[test]
fn a_grown_log_replaces_the_stored_one() {
    let b = bench();
    let first = b.core.gear_backup(&by_mount(&b)).unwrap();
    let log = b.card().join("LOGS/ALPHA-2026-05-01.csv");
    let mut grown = std::fs::read(&log).unwrap();
    grown.extend_from_slice(b"2026-05-01,12:00:01.000,-61,99,4.0\n");
    std::fs::write(&log, &grown).unwrap();
    std::fs::write(b.card().join("LOGS/BRAVO 2-2026-05-02.csv"), "rewritten\n").unwrap();
    let second = b.core.gear_backup(&by_mount(&b)).unwrap();
    assert!(!second.report.new, "logs are not snapshot content");
    assert_eq!(second.report.logs.grown, 1);
    assert_eq!(second.report.logs.kept_both, 1);
    let dir = b.store().logs_dir(&first.device);
    assert_eq!(
        std::fs::read(dir.join("ALPHA-2026-05-01.csv")).unwrap(),
        grown
    );
    assert!(dir.join("BRAVO 2-2026-05-02 (2).csv").is_file());
    let st = b.core.gear_storage().unwrap();
    assert_eq!(st.devices[0].logs, 3);
}

/// An old backup folder as people keep them: dated folders, each a card copy or FC files.
fn old_backups(root: &Path) {
    let card = SynthCard::default();
    synth::write_card(&root.join("2026-05-01 radio"), &card).unwrap();
    // A week later, unchanged; then a model edit.
    synth::write_card(&root.join("2026-05-08 radio"), &card).unwrap();
    synth::write_card(&root.join("2026-05-15/sd"), &card).unwrap();
    std::fs::write(
        root.join("2026-05-15/sd/MODELS/model00.yml"),
        "header:\r\n  name: \"ALPHA\"\r\n# edited\r\n",
    )
    .unwrap();
    // FC pairs with a made-up MCU id.
    let uid = "mcu_id 00aa00bb00cc00dd00ee00ff";
    let dump = G473_DUMP.replace("mcu_id 000000000000000000000000", uid);
    let diff = G473_DIFF.replace("mcu_id 000000000000000000000000", uid);
    for day in ["2026-05-01", "2026-05-08"] {
        let d = root.join(format!("{day} quad"));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("quad.diff_all.txt"), &diff).unwrap();
        std::fs::write(d.join("quad.dump_all.txt"), &dump).unwrap();
    }
    // A later diff alone, with one change, and a stray note that is not CLI output.
    let d = root.join("2026-05-20 quad");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("after-tune.txt"),
        diff.replace(
            "board_name BETAFPVG473",
            "board_name BETAFPVG473\nset tpa_rate = 70",
        ),
    )
    .unwrap();
    std::fs::write(d.join("notes.txt"), "props changed").unwrap();
    // Logs on their own.
    std::fs::create_dir_all(root.join("2026-06-01/LOGS")).unwrap();
    std::fs::write(
        root.join("2026-06-01/LOGS/ALPHA-2026-06-01.csv"),
        "Date,Time\n",
    )
    .unwrap();
}

fn radio_device(id: &str) -> Device {
    Device {
        id: id.into(),
        kind: DeviceKind::Radio,
        name: String::new(),
        aircraft: None,
        identity: Identity {
            board: Some("pocket".into()),
            firmware: Some("EdgeTX".into()),
            ..Default::default()
        },
        last_seen: None,
        last_backup: None,
        last_space: None,
        aliases: Vec::new(),
        dfu_serial: None,
        radio_aircraft: None,
    }
}

#[test]
fn import_of_card_copies_and_fc_pairs_dedupes() {
    let b = bench_with(false, Arc::new(FakePorts::new(vec![])), FakeDisk::ok());
    let src = b.dir.path().join("old");
    old_backups(&src);
    let src_before = store_files_any(&src);
    b.store()
        .save_device(&radio_device("radio-00000000000000aa"))
        .unwrap();
    let p = ImportBackupsParams {
        folder: src.clone(),
        device: None,
        dry_run: true,
    };
    let dry = b.core.gear_import_backups(&p).unwrap();
    assert!(!b.gear().join("blobs").exists(), "a dry run writes nothing");
    let r = b
        .core
        .gear_import_backups(&ImportBackupsParams {
            dry_run: false,
            ..p.clone()
        })
        .unwrap();
    let outcomes: Vec<(String, BackupImportOutcome)> = r
        .items
        .iter()
        .map(|i| {
            (
                i.path.strip_prefix(&src).unwrap().display().to_string(),
                i.outcome,
            )
        })
        .collect();
    assert_eq!(
        outcomes,
        vec![
            (
                "2026-05-01 quad/quad.diff_all.txt".into(),
                BackupImportOutcome::Imported
            ),
            ("2026-05-01 radio".into(), BackupImportOutcome::Imported),
            (
                "2026-05-08 quad/quad.diff_all.txt".into(),
                BackupImportOutcome::Same
            ),
            ("2026-05-08 radio".into(), BackupImportOutcome::Same),
            ("2026-05-15/sd".into(), BackupImportOutcome::Imported),
            (
                "2026-05-20 quad/after-tune.txt".into(),
                BackupImportOutcome::Imported
            ),
            ("2026-06-01/LOGS".into(), BackupImportOutcome::Logs),
        ],
        "{:#?}",
        r.items
    );
    assert_eq!(
        dry.items.iter().map(|i| i.outcome).collect::<Vec<_>>(),
        r.items.iter().map(|i| i.outcome).collect::<Vec<_>>(),
        "the dry run reports the same"
    );
    assert_eq!((r.imported, r.same, r.skipped), (4, 2, 0));
    // The FC is new: its MCU id names it, and it is saved.
    assert_eq!(r.new_devices.len(), 1);
    let fc = &r.new_devices[0];
    assert!(fc.starts_with("fc-"));
    let fc_snaps = b.snaps().list(fc);
    assert_eq!(fc_snaps.len(), 2);
    assert_eq!(fc_snaps[0].files.len(), 2, "the pair is one snapshot");
    assert_eq!(fc_snaps[1].files.len(), 1, "a diff alone");
    assert_eq!(fc_snaps[0].taken_at.date_naive().to_string(), "2026-05-01");
    // Dedupe: every distinct file content is one blob.
    let mut distinct = std::collections::HashSet::new();
    for s in b.snaps().all() {
        for f in s.files {
            distinct.insert((f.xxh64, f.size));
        }
    }
    assert_eq!(blob_count(&b), distinct.len());
    assert_eq!(r.logs.added, 3, "two card logs and the LOGS folder's");
    // Running it again takes nothing new; the source is untouched.
    let again = b
        .core
        .gear_import_backups(&ImportBackupsParams {
            dry_run: false,
            ..p.clone()
        })
        .unwrap();
    assert_eq!((again.imported, again.same), (0, 6));
    assert_eq!(store_files_any(&src), src_before);
    let dev = b.store().device(fc).unwrap().unwrap();
    assert_eq!(dev.last_backup, Some(fc_snaps[1].id.clone()));
}

#[test]
fn an_import_asks_when_two_devices_could_match() {
    let b = bench_with(false, Arc::new(FakePorts::new(vec![])), FakeDisk::ok());
    let src = b.dir.path().join("old");
    synth::write_card(&src.join("2026-05-01"), &SynthCard::default()).unwrap();
    let p = ImportBackupsParams {
        folder: src.clone(),
        device: None,
        dry_run: false,
    };
    let none = b.core.gear_import_backups(&p).unwrap();
    assert_eq!(none.skipped, 1);
    assert!(none.items[0]
        .reason
        .as_deref()
        .unwrap()
        .contains("No saved Radio"));
    b.store()
        .save_device(&radio_device("radio-00000000000000aa"))
        .unwrap();
    b.store()
        .save_device(&radio_device("radio-00000000000000bb"))
        .unwrap();
    let two = b.core.gear_import_backups(&p).unwrap();
    assert!(two.items[0]
        .reason
        .as_deref()
        .unwrap()
        .contains("pass a device"));
    let picked = b
        .core
        .gear_import_backups(&ImportBackupsParams {
            device: Some("radio-00000000000000bb".into()),
            ..p
        })
        .unwrap();
    assert_eq!(picked.imported, 1);
    assert_eq!(
        picked.items[0].device.as_deref(),
        Some("radio-00000000000000bb")
    );
}

fn store_files_any(root: &Path) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.insert(p.display().to_string(), e.metadata().unwrap().len());
            }
        }
    }
    out
}

#[test]
fn a_crash_between_blob_and_manifest_leaves_a_consistent_store() {
    let b = bench();
    let first = b.core.gear_backup(&by_mount(&b)).unwrap();
    let dev = first.device.clone();
    std::fs::write(b.card().join("SOUNDS/en/new.wav"), b"RIFF new sound").unwrap();
    let s = b.snaps();
    let mut opts = TakeOptions {
        crash_before_manifest: true,
        ..Default::default()
    };
    let e = s
        .take_card(
            &dev,
            &Identity::default(),
            &b.card(),
            Trigger::Manual,
            &mut opts,
        )
        .unwrap_err();
    assert!(e.to_string().contains("crash"));
    // A half-written manifest and blob as a crash mid-write would leave them.
    std::fs::write(
        b.store()
            .snapshots_dir(&dev)
            .join("2026-01-01T000000-manual.json.tmp"),
        b"{\"id\":",
    )
    .unwrap();
    std::fs::create_dir_all(b.gear().join("blobs/tmp")).unwrap();
    std::fs::write(b.gear().join("blobs/tmp/put-1-0"), b"half").unwrap();
    // The store still reads: the latest snapshot is the one before the crash, and every
    // blob it names is there.
    assert_eq!(s.list(&dev).len(), 1);
    assert_eq!(s.latest(&dev).unwrap().id, first.report.backup.id);
    for f in &s.latest(&dev).unwrap().files {
        assert!(s.blobs().verify(&backup::blob_of(f)));
    }
    // The blob nothing names, and the temp file, go at the next collection.
    let dry = b.core.gear_prune(&PruneParams { dry_run: true }).unwrap();
    assert_eq!(dry.collected.blobs, 1);
    let pr = b.core.gear_prune(&PruneParams { dry_run: false }).unwrap();
    assert_eq!((pr.collected.blobs, pr.collected.temp_files), (1, 1));
    // The next backup works and is new.
    let next = b.core.gear_backup(&by_mount(&b)).unwrap();
    assert!(next.report.new);
    assert_eq!(next.report.read, 1);
}

#[test]
fn a_stop_between_files_saves_no_snapshot() {
    let b = bench();
    let s = b.snaps();
    let stop = AtomicBool::new(false);
    let mut seen = 0;
    let mut progress = |p: &backup::BackupProgress| {
        if p.files_done >= 1 {
            stop.store(true, Ordering::SeqCst);
        }
        seen += 1;
    };
    let mut opts = TakeOptions {
        stop: Some(&stop),
        progress: Some(&mut progress),
        crash_before_manifest: false,
    };
    let e = s
        .take_card(
            "radio-x",
            &Identity::default(),
            &b.card(),
            Trigger::Manual,
            &mut opts,
        )
        .unwrap_err();
    assert!(e.to_string().starts_with("Stopped"));
    assert!(s.list("radio-x").is_empty());
    assert!(seen > 1);
    assert!(!b.core.gear_stop("disk42"), "nothing runs now");
}

#[test]
fn pin_export_and_storage() {
    let b = bench();
    let first = b.core.gear_backup(&by_mount(&b)).unwrap();
    let pinned = b
        .core
        .gear_backup_pin(&BackupPinParams {
            id: first.report.backup.id.clone(),
            pinned: true,
        })
        .unwrap();
    assert!(pinned.pinned);
    let out = b.dir.path().join("export");
    std::fs::create_dir_all(&out).unwrap();
    let ex = b
        .core
        .gear_export(&ExportParams {
            device: Some(first.device.clone()),
            snapshot: None,
            to: out.clone(),
        })
        .unwrap();
    assert_eq!(ex.folders.len(), 1);
    assert!(ex.folders[0].join("MODELS/model01.yml").is_file());
    assert!(
        b.core
            .gear_export(&ExportParams {
                device: Some(first.device.clone()),
                snapshot: None,
                to: out,
            })
            .unwrap_err()
            .to_string()
            .starts_with("Refused"),
        "never into an existing folder"
    );
    let st = b.core.gear_storage().unwrap();
    assert_eq!(st.snapshots, 1);
    assert_eq!(st.devices[0].pinned, 1);
    assert_eq!(st.devices[0].by_trigger.get("manual"), Some(&1));
    assert_eq!(st.blob_bytes, st.devices[0].own_blob_bytes);
    assert!(st.total_bytes > st.blob_bytes);
}

fn failed_then_fixed() -> FakeDisk {
    let out = |code, text: &str| RunOutput {
        code: Some(code),
        output: text.into(),
        ..Default::default()
    };
    FakeDisk::with(vec![
        out(1, health::VERIFY_FAILED),
        out(0, health::REPAIR_FIXED),
        out(0, health::VERIFY_OK),
    ])
}

#[test]
fn a_known_card_is_checked_on_connect_and_repaired_on_request() {
    let b = bench_with(true, Arc::new(FakePorts::new(vec![])), failed_then_fixed());
    for h in backup_hooks() {
        b.core.gear_add_hook(h);
    }
    let c = b.core.gear_connected().unwrap().remove(0);
    // An unknown card is not checked; its backup saves it.
    let runs = b.core.gear_on_connect(&c);
    assert!(b.disk.calls().is_empty(), "{runs:?}");
    assert_eq!(b.core.gear_devices().unwrap().len(), 1);
    let c = b.core.gear_connected().unwrap().remove(0);
    // Known now: the check runs first and fails; the backup still reads the card.
    let runs = b.core.gear_on_connect(&c);
    assert_eq!(runs[0].name, "Card check");
    assert!(matches!(
        runs[0].outcome,
        quadcam_lib::core::HookOutcome::Failed { .. }
    ));
    assert_eq!(runs[1].outcome, quadcam_lib::core::HookOutcome::Ran);
    assert_eq!(
        b.disk.calls()[0],
        vec!["verifyVolume".to_string(), b.card().display().to_string()]
    );
    let last_cue = b.cues.spoken().last().unwrap().clone();
    assert!(last_cue.starts_with("Card check failed"), "{last_cue}");
    let status = b.core.gear_status().unwrap();
    let check = status.card_checks[0].clone();
    assert_eq!(check.state, CheckState::Failed);
    assert_eq!(check.fsck_code, Some(206));
    // The repair needs that check's id and the confirm.
    assert!(b
        .core
        .gear_card_repair(&CardRepairParams {
            check: check.id.clone(),
            confirm: false
        })
        .is_err());
    assert!(b
        .core
        .gear_card_repair(&CardRepairParams {
            check: "other".into(),
            confirm: true
        })
        .unwrap_err()
        .to_string()
        .starts_with("Refused"));
    let r = b
        .core
        .gear_card_repair(&CardRepairParams {
            check: check.id.clone(),
            confirm: true,
        })
        .unwrap();
    let kept = b.snaps().get(r.backup.as_deref().unwrap()).unwrap();
    assert_eq!(
        kept.trigger,
        Trigger::BeforeApply,
        "the backup first is always kept"
    );
    assert!(r.repair.modified);
    assert_eq!(r.verify.state, CheckState::Ok);
    assert_eq!(b.disk.calls()[1][0], "repairVolume");
    let log = b
        .core
        .gear_card_checks(&quadcam_lib::core::CardChecksParams {
            device: c.id.clone().unwrap(),
        });
    assert_eq!(log.len(), 3);
    assert_eq!(log[0].state, CheckState::Ok);
    // A card whose latest check passed has nothing to repair.
    assert!(b
        .core
        .gear_card_repair(&CardRepairParams {
            check: log[0].id.clone(),
            confirm: true
        })
        .is_err());
    // A check on request.
    let again = b.core.gear_card_check(&CardCheckParams::default()).unwrap();
    assert_eq!(again.state, CheckState::Ok);
}

#[test]
fn mcp_and_cli_surfaces() {
    let b = bench_with(true, Arc::new(FakePorts::new(vec![])), failed_then_fixed());
    let mut s = Server::new(LocalBackend(b.core.clone()));
    let call = |s: &mut Server<LocalBackend>, tool: &str, args: serde_json::Value| {
        let r = s.call_tool(tool, args);
        assert_eq!(r["isError"], false, "{tool}: {r}");
        (
            r["content"][0]["text"].as_str().unwrap().to_string(),
            r["structuredContent"].clone(),
        )
    };
    let (t, v) = call(&mut s, "quadcam_gear_edit", json!({"action": "backup"}));
    assert!(
        t.starts_with("Backed up The Radio") || t.starts_with("Backed up Unnamed Radio"),
        "{t}"
    );
    let id = v["report"]["backup"]["id"].as_str().unwrap().to_string();
    let dev = v["device"].as_str().unwrap().to_string();
    let (t, _) = call(&mut s, "quadcam_gear_edit", json!({"action": "backup"}));
    assert!(t.contains("no changes since"), "{t}");
    let (t, _) = call(&mut s, "quadcam_gear", json!({"action": "backups"}));
    assert!(t.contains(&id) && t.contains("manual"), "{t}");
    let (t, _) = call(
        &mut s,
        "quadcam_gear",
        json!({"action": "backup_read", "id": id, "path": "RADIO/radio.yml"}),
    );
    assert!(t.contains("semver"), "{t}");
    let (t, _) = call(&mut s, "quadcam_gear", json!({"action": "storage"}));
    assert!(t.contains("1 snapshots"), "{t}");
    let (t, _) = call(
        &mut s,
        "quadcam_gear_edit",
        json!({"action": "backup_pin", "backup": id, "pinned": true}),
    );
    assert!(t.ends_with("pinned"), "{t}");
    let (t, _) = call(
        &mut s,
        "quadcam_gear_edit",
        json!({"action": "prune", "dry_run": true}),
    );
    assert!(t.starts_with("Dry run"), "{t}");
    let (t, v) = call(&mut s, "quadcam_gear_edit", json!({"action": "card_check"}));
    assert!(t.contains("Repair it") && t.contains("card_repair"), "{t}");
    let check = v["id"].as_str().unwrap().to_string();
    let (t, _) = call(&mut s, "quadcam_gear", json!({"action": "status"}));
    assert!(t.contains(&check), "{t}");
    let r = s.call_tool(
        "quadcam_gear_apply",
        json!({"action": "card_repair", "digest": check}),
    );
    assert_eq!(r["isError"], true, "no confirm: {r}");
    let (t, _) = call(
        &mut s,
        "quadcam_gear_apply",
        json!({"action": "card_repair", "digest": check, "confirm": true}),
    );
    assert!(
        t.contains("Backed up first") && t.contains("Repaired."),
        "{t}"
    );
    let (t, _) = call(
        &mut s,
        "quadcam_gear",
        json!({"action": "card_checks", "device": dev}),
    );
    assert_eq!(t.lines().filter(|l| !l.starts_with("  ")).count(), 3, "{t}");
    let src = b.dir.path().join("old");
    synth::write_card(&src.join("2026-05-01"), &SynthCard::default()).unwrap();
    let (t, _) = call(
        &mut s,
        "quadcam_gear_edit",
        json!({"action": "import_backups", "folder": src, "dry_run": true}),
    );
    assert!(
        t.starts_with("Dry run") && t.contains("| imported | radio-"),
        "{t}"
    );
    let (t, _) = call(
        &mut s,
        "quadcam_gear_edit",
        json!({"action": "stop", "handle": "disk42"}),
    );
    assert!(t.starts_with("Nothing is running"), "{t}");

    // The CLI on its own HOME: an empty store, an import dry run, and no disk under cargo.
    let home = tempfile::tempdir().unwrap();
    let cli = |args: &[&str]| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
            .env("HOME", home.path())
            .env_remove("QUADCAM_SERIAL")
            .arg("--json")
            .args(args)
            .output()
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        (out.status.code(), v)
    };
    let (code, v) = cli(&["gear", "storage"]);
    assert_eq!(code, Some(0), "{v}");
    assert_eq!(v["result"]["snapshots"], 0);
    let (code, v) = cli(&["gear", "import-backups", src.to_str().unwrap(), "--dry-run"]);
    assert_eq!(code, Some(0), "{v}");
    assert_eq!(v["result"]["skipped"], 1, "no saved radio: {v}");
    let (code, v) = cli(&["gear", "backups"]);
    assert_eq!(
        (code, v["result"].as_array().map(Vec::len)),
        (Some(0), Some(0))
    );
    let (code, v) = cli(&["gear", "card-repair", "--check", "x"]);
    assert_eq!(code, Some(3), "refused without --yes: {v}");
}

#[test]
fn an_auto_backup_unmounts_the_card_and_says_safe_only_after_the_unmount() {
    use quadcam_lib::core::HookOutcome;
    let b = bench();
    for h in backup_hooks() {
        b.core.gear_add_hook(h);
    }
    // A card QuadCam does not know: no check (the step counts as off), a backup, then
    // unmountDisk, then "done, safe to unplug".
    let c = b.core.gear_connected().unwrap().remove(0);
    let runs = b.core.gear_on_connect(&c);
    assert_eq!(runs[0].outcome, HookOutcome::Off, "{runs:?}");
    assert_eq!(runs[1].outcome, HookOutcome::Ran);
    assert_eq!(runs.len(), 2, "no unmount failure: {runs:?}");
    assert_eq!(*b.unmounts.lock().unwrap(), vec!["disk42".to_string()]);
    let spoken = b.cues.spoken();
    assert_eq!(spoken.len(), 1);
    assert!(spoken[0].contains("safe to unplug"), "{spoken:?}");
    assert!(b.core.gear_status().unwrap().failures.is_empty());

    // Known now: the check passes, the backup finds nothing new, and the unmount fails.
    // The failed step's cue plays, never "safe to unplug", and the reason shows.
    b.unmount_fails.store(true, Ordering::SeqCst);
    let c = b.core.gear_connected().unwrap().remove(0);
    let runs = b.core.gear_on_connect(&c);
    assert_eq!(b.disk.calls().len(), 1, "the check ran");
    let last = runs.last().unwrap();
    assert_eq!(last.name, "Unmount");
    assert!(
        matches!(&last.outcome, HookOutcome::Failed { message } if message.contains("could not be unmounted")),
        "{last:?}"
    );
    let spoken = b.cues.spoken();
    assert_eq!(spoken.len(), 2, "{spoken:?}");
    assert!(spoken[1].starts_with("Unmount failed"), "{spoken:?}");
    let f = b.core.gear_status().unwrap().failures;
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].step, "Unmount");
    assert!(f[0].message.contains("could not be unmounted"));

    // A card check on request unmounts too; once that works, the failure clears.
    b.unmount_fails.store(false, Ordering::SeqCst);
    let check = b.core.gear_card_check(&CardCheckParams::default()).unwrap();
    assert_eq!(check.state, CheckState::Ok);
    assert_eq!(b.unmounts.lock().unwrap().len(), 3);
    assert!(b.core.gear_status().unwrap().failures.is_empty());
    // A manual backup also ends unmounted; a failed unmount is a note on its answer.
    b.unmount_fails.store(true, Ordering::SeqCst);
    let r = b.core.gear_backup(&by_mount(&b)).unwrap();
    assert!(
        r.notes
            .iter()
            .any(|n| n.starts_with("The card did not unmount")),
        "{:?}",
        r.notes
    );
}
