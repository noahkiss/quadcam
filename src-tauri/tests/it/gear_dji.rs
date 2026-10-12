//! The DJI O4 air unit over USB follows the mount cycle every card follows: QuadCam
//! unmounts it when a job or the person's Mount is done, finds it again on the USB bus
//! while it stays plugged in, mounts it for the next job, and the person's Mount lasts
//! `MOUNT_MINUTES`. Unmount is not format: the never-format guard still refuses it.
//! The volume is a temporary folder and "diskutil" is a closure; no real disk is touched.

use quadcam_lib::core::{CardMountParams, Core, NoHooks, MOUNT_MINUTES};
use quadcam_lib::disk::{self, DiskInfo, Volume};
use quadcam_lib::gear::cues::{CueService, RecordedCues};
use quadcam_lib::gear::events::{Presence, DJI_VID};
use quadcam_lib::gear::model::DeviceKind;
use quadcam_lib::gear::serial::FakePorts;
use quadcam_lib::gear::Env;
use quadcam_lib::photos::Recorder;
use quadcam_lib::sources::SourceKind;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DISK: &str = "disk7";

fn o4_info() -> DiskInfo {
    DiskInfo {
        device_identifier: format!("{DISK}s1"),
        parent_whole_disk: DISK.into(),
        volume_uuid: Some("00000000-0000-4000-8000-0000000000a4".into()),
        volume_name: Some("O4".into()),
        bus_protocol: Some("USB".into()),
        // An air unit is ejectable USB storage, not removable media.
        ejectable: true,
        media_name: Some("DJI Air Unit".into()),
        filesystem: Some("ExFAT".into()),
        ..Default::default()
    }
}

struct Bench {
    core: Core,
    cues: Arc<RecordedCues>,
    mounted: Arc<AtomicBool>,
    log: Arc<Mutex<Vec<String>>>,
    root: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("O4");
    std::fs::create_dir_all(root.join("DCIM/DJI_001")).unwrap();
    std::fs::write(
        root.join("DCIM/DJI_001/DJI_20260105120000_0001_D.MP4"),
        b"x",
    )
    .unwrap();
    let vol = Volume {
        mount: root.clone(),
        info: o4_info(),
        is_card: true,
        source: Some(SourceKind::Dji),
        is_radio: false,
        warnings: vec![],
    };
    let mounted = Arc::new(AtomicBool::new(true));
    let log: Arc<Mutex<Vec<String>>> = Arc::default();
    let cues = Arc::new(RecordedCues::default());
    let mut env = Env::fake(vec![], Arc::new(FakePorts::new(vec![])));
    env.cues = Arc::new(CueService::inline(cues.clone()));
    let m = mounted.clone();
    env.volumes = Arc::new(move || {
        if m.load(Ordering::SeqCst) {
            vec![vol.clone()]
        } else {
            vec![]
        }
    });
    // Unmounted, the air unit's disk node goes, but the unit stays on the USB bus.
    env.presence = Arc::new(|| {
        vec![Presence::Usb {
            vid: DJI_VID,
            pid: 0x0020,
            name: None,
        }]
    });
    env.dji_usb = Arc::new(|| true);
    let (m, l) = (mounted.clone(), log.clone());
    env.unmount = Arc::new(move |d| {
        l.lock().unwrap().push(format!("unmount {d}"));
        m.store(false, Ordering::SeqCst);
        Ok(())
    });
    let (m, l) = (mounted.clone(), log.clone());
    env.mount = Arc::new(move |d| {
        l.lock().unwrap().push(format!("mount {d}"));
        m.store(true, Ordering::SeqCst);
        Ok(())
    });
    std::fs::create_dir_all(dir.path().join("support")).unwrap();
    std::fs::write(
        dir.path().join("support/settings.json"),
        r#"{"gearCues": {"debounce_s": 0}}"#,
    )
    .unwrap();
    let core = Core::new(
        dir.path().join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(dir.path().join("support/settings.json"))
    .with_gear_env(env);
    Bench {
        core,
        cues,
        mounted,
        log,
        root,
        _dir: dir,
    }
}

fn o4_id(b: &Bench) -> String {
    let c = b
        .core
        .gear_connected()
        .unwrap()
        .into_iter()
        .find(|c| c.kind == DeviceKind::Goggles)
        .expect("the O4 shows as a DJI device");
    c.id.expect("the O4 has an id")
}

#[test]
fn the_o4_over_usb_is_unmounted_mounted_for_a_job_and_unmounted_again() {
    let b = bench();
    let id = o4_id(&b);
    let p = |minutes| CardMountParams {
        device: id.clone(),
        minutes,
    };

    // Done: the O4 unmounts like any card, and "safe to unplug" plays after the unmount.
    b.core.gear_card_unmount(&p(None)).unwrap();
    assert_eq!(*b.log.lock().unwrap(), [format!("unmount {DISK}")]);
    assert!(!b.mounted.load(Ordering::SeqCst));
    assert!(
        b.cues.spoken().iter().any(|s| s.contains("safe to unplug")),
        "{:?}",
        b.cues.spoken()
    );
    assert!(b.core.gear_connected().unwrap().is_empty());

    // The person's Mount finds it on the USB bus, mounts it, and keeps it MOUNT_MINUTES.
    b.log.lock().unwrap().clear();
    b.cues.played.lock().unwrap().clear();
    let before = chrono::Utc::now();
    let m = b.core.gear_card_mount(&p(None)).unwrap();
    assert_eq!(*b.log.lock().unwrap(), [format!("mount {DISK}")]);
    assert_eq!(m.mount, b.root);
    let minutes = (m.until - before).num_seconds() as f64 / 60.0;
    assert!(
        (minutes - MOUNT_MINUTES as f64).abs() < 0.1,
        "mounted for {minutes} min"
    );
    assert_eq!(b.core.gear_mounted_cards().len(), 1);

    // Not yet due: it stays mounted. Past the timer it unmounts and says so.
    b.core.gear_mount_tick(Instant::now());
    assert!(b.mounted.load(Ordering::SeqCst));
    b.core
        .gear_mount_tick(Instant::now() + Duration::from_secs(MOUNT_MINUTES as u64 * 60 + 1));
    assert!(!b.mounted.load(Ordering::SeqCst));
    assert_eq!(
        *b.log.lock().unwrap(),
        [format!("mount {DISK}"), format!("unmount {DISK}")]
    );
    assert!(b.core.gear_mounted_cards().is_empty());
    assert!(b.cues.spoken().iter().any(|s| s.contains("safe to unplug")));

    // Unmount is not format: the O4 is still refused for an erase.
    let e = disk::check_volume_contents(&b.root, &o4_info(), true)
        .unwrap_err()
        .to_string();
    assert!(e.contains("never formats a DJI device over USB"), "{e}");
}

#[test]
fn safe_to_remove_unmounts_the_o4_and_never_ejects_it() {
    // An eject would take the air unit off the bus, so no later job could mount it.
    assert!(disk::removes_by_unmount(&o4_info(), &|| false));
    let unnamed = DiskInfo {
        media_name: None,
        ..o4_info()
    };
    assert!(disk::removes_by_unmount(&unnamed, &|| true));
}
