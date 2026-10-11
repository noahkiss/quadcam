//! Mount, work, unmount for the import session's card and card prep (the cycle of
//! `core/session_card.rs`), on FAT32 disk images this test creates. The core's mount and
//! unmount are closures that run `diskutil` on the image's own disk, so no real card is
//! touched. A test image reports removable media, like a card.

mod common;

use common::*;
use quadcam_lib::core::{Core, FormatRequest, ImportOptions, NoHooks};
use quadcam_lib::disk;
use quadcam_lib::gear::events::Presence;
use quadcam_lib::gear::serial::FakePorts;
use quadcam_lib::gear::Env;
use quadcam_lib::media::Format;
use quadcam_lib::photos::Recorder;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

/// Test-only belt and braces: the target must be a disk image this test attached.
fn assert_is_test_image(img: &Image) {
    let whole = disk::info(&img.disk).unwrap();
    assert_eq!(
        whole.bus_protocol.as_deref(),
        Some("Disk Image"),
        "refusing to touch a non-image disk"
    );
}

fn diskutil(verb: &str, disk: &str) -> anyhow::Result<()> {
    let out = Command::new("/usr/sbin/diskutil")
        .args([verb, &format!("/dev/{disk}")])
        .output()?;
    anyhow::ensure!(
        out.status.success(),
        "diskutil {verb}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(())
}

struct Bench {
    core: Core,
    log: Arc<Mutex<Vec<String>>>,
    _work: tempfile::TempDir,
}

/// A core that sees the image as a card and mounts and unmounts it with `diskutil`, only
/// for that image's disk. The log holds each request.
fn bench(img: &Image) -> Bench {
    bench_with(img, false)
}

/// `bench`, with a mount that fails when `mount_fails`.
fn bench_with(img: &Image, mount_fails: bool) -> Bench {
    assert_is_test_image(img);
    let work = tempfile::tempdir().unwrap();
    let disk_id = img.disk.clone();
    let uuid = disk::info(&img.mount.to_string_lossy())
        .unwrap()
        .volume_uuid
        .unwrap();
    let log: Arc<Mutex<Vec<String>>> = Arc::default();
    let mut env = Env::fake(vec![], Arc::new(FakePorts::new(vec![])));
    env.volumes = Arc::new(move || {
        disk::info(&uuid)
            .ok()
            .and_then(|i| i.mount_point)
            .and_then(|m| disk::probe_volume(Path::new(&m)))
            .into_iter()
            .collect()
    });
    let d = disk_id.clone();
    env.presence = Arc::new(move || vec![Presence::Disk { disk: d.clone() }]);
    let (d, l) = (disk_id.clone(), log.clone());
    env.unmount = Arc::new(move |w| {
        assert_eq!(w, d, "only the test image is unmounted");
        l.lock().unwrap().push(format!("unmount {w}"));
        diskutil("unmountDisk", w)
    });
    let (d, l) = (disk_id, log.clone());
    env.mount = Arc::new(move |w| {
        assert_eq!(w, d, "only the test image is mounted");
        l.lock().unwrap().push(format!("mount {w}"));
        if mount_fails {
            anyhow::bail!("the test's mount fails");
        }
        diskutil("mountDisk", w)
    });
    std::fs::create_dir_all(work.path().join("support")).unwrap();
    std::fs::create_dir_all(work.path().join("out")).unwrap();
    let core = Core::new(
        work.path().join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(work.path().join("support/settings.json"))
    .with_gear_env(env);
    let mut defaults = core.defaults();
    defaults.output_dir = Some(work.path().join("out"));
    defaults.layout = quadcam_lib::library::Layout::Flat;
    core.set_defaults(defaults);
    Bench {
        core,
        log,
        _work: work,
    }
}

fn mp4() -> ImportOptions {
    ImportOptions {
        format: Some(Format::Mp4),
        ..Default::default()
    }
}

fn card_id(b: &Bench) -> String {
    b.core
        .gear_connected()
        .unwrap()
        .into_iter()
        .find_map(|c| c.id)
        .expect("the image shows as a card")
}

/// The image mounted under another path than the one it was attached at is the same card.
fn mounted_somewhere(img: &Image) -> bool {
    img.is_mounted()
}

#[test]
fn an_import_unmounts_its_card_and_the_card_steps_mount_it_again() {
    let img = Image::create("64m", "QCMC1", false);
    std::fs::create_dir_all(img.mount.join("DCIM")).unwrap();
    make_clip(&img.mount.join("DCIM/PICT0001.AVI"), 2, true);
    let b = bench(&img);

    b.core.load(Some(&img.mount)).unwrap();
    assert!(mounted_somewhere(&img), "staging leaves the card mounted");
    let out = b.core.import(&mp4()).unwrap();

    // Work done: the card is unmounted, and the outcome says it is safe to remove.
    let release = out.card.expect("a card release");
    assert!(release.released, "{}", release.message);
    assert!(img.is_attached() && !img.is_mounted());
    assert_eq!(*b.log.lock().unwrap(), ["unmount ".to_string() + &img.disk]);

    // "Safe to remove" works on a card that is already unmounted.
    b.core.eject(None).unwrap();

    // A format plan mounts the card, reads, and unmounts it again.
    b.log.lock().unwrap().clear();
    let plan = b.core.format_plan(Some("QCMC1")).unwrap();
    assert_eq!(plan.device, format!("/dev/{}", img.disk));
    assert_eq!(
        *b.log.lock().unwrap(),
        [
            format!("mount {}", img.disk),
            format!("unmount {}", img.disk)
        ]
    );
    assert!(img.is_attached() && !img.is_mounted());

    // The erase mounts it, erases, and leaves it unmounted.
    b.log.lock().unwrap().clear();
    assert_is_test_image(&img);
    b.core
        .format(
            &FormatRequest {
                device: plan.device.clone(),
                volume_uuid: plan.volume_uuid.clone(),
                label: Some("QCMC1".into()),
                confirm: true,
            },
            true,
        )
        .unwrap();
    assert_eq!(
        b.log.lock().unwrap().first().unwrap(),
        &format!("mount {}", img.disk)
    );
    assert!(img.is_attached() && !img.is_mounted());
}

#[test]
fn a_pulled_card_is_never_confused_with_the_disk_that_took_its_number() {
    let img = Image::create("64m", "QCMC5", false);
    std::fs::create_dir_all(img.mount.join("DCIM")).unwrap();
    make_clip(&img.mount.join("DCIM/PICT0001.AVI"), 2, true);
    let b = bench(&img);
    b.core.load(Some(&img.mount)).unwrap();
    assert!(b.core.import(&mp4()).unwrap().card.unwrap().released);

    // The card is pulled; another disk attaches, often on the same disk number.
    img.detach();
    let other = Image::create("64m", "QCMC6", false);
    assert_is_test_image(&other);
    assert!(other.is_mounted());
    b.log.lock().unwrap().clear();

    // "Safe to remove" ejects nothing.
    let e = b.core.eject(None).unwrap_err();
    assert!(format!("{e:#}").contains("not plugged in"), "{e:#}");
    // A card step does not mount the other disk, and its release unmounts nothing.
    let e = b.core.format_plan(Some("QCMC5")).unwrap_err();
    assert!(format!("{e:#}").contains("not plugged in"), "{e:#}");
    assert!(
        b.log.lock().unwrap().is_empty(),
        "{:?}",
        b.log.lock().unwrap()
    );
    assert!(other.is_mounted(), "the other disk stays mounted");
}

#[test]
fn a_card_that_does_not_mount_for_deleting_clips_says_so_per_clip() {
    let img = Image::create("64m", "QCMC7", false);
    std::fs::create_dir_all(img.mount.join("DCIM")).unwrap();
    make_clip(&img.mount.join("DCIM/PICT0001.AVI"), 2, true);
    let b = bench_with(&img, true);
    let mut d = b.core.defaults();
    d.delete_clips_after_import = true;
    b.core.set_defaults(d);
    b.core.load(Some(&img.mount)).unwrap();
    // The card was unmounted after the stage; the mount for the deletion fails.
    diskutil("unmountDisk", &img.disk).unwrap();
    let out = b.core.import(&mp4()).unwrap();
    let d = out.clip_deletion.expect("the setting is on");
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].state, quadcam_lib::core::DeletionState::Kept);
    let why = d[0].reason.as_deref().unwrap();
    assert!(why.contains("did not mount"), "{why}");
    assert!(why.contains("the test's mount fails"), "{why}");
    assert!(b
        .core
        .session()
        .unwrap()
        .warnings
        .iter()
        .any(|w| w.starts_with("No clips were deleted: the card did not mount")));
    assert_eq!(
        *b.log.lock().unwrap(),
        [
            format!("mount {}", img.disk),
            format!("unmount {}", img.disk)
        ]
    );
}

#[test]
fn a_folder_import_has_no_card_to_release() {
    let img = Image::create("64m", "QCMC2", false);
    let b = bench(&img);
    let dir = tempfile::tempdir().unwrap();
    make_clip(&dir.path().join("PICT0001.AVI"), 2, true);
    b.core.load(Some(dir.path())).unwrap();
    let out = b.core.import(&mp4()).unwrap();
    assert!(out.card.is_none());
    assert!(b.log.lock().unwrap().is_empty());
}

#[test]
fn an_unmounted_card_is_mounted_for_a_stage_by_device() {
    let img = Image::create("64m", "QCMC3", false);
    std::fs::create_dir_all(img.mount.join("DCIM")).unwrap();
    make_clip(&img.mount.join("DCIM/PICT0001.AVI"), 2, true);
    let b = bench(&img);
    let id = card_id(&b);

    // The app's poll noted the card as unmounted but still plugged in.
    let seen = b.core.gear_connected().unwrap();
    b.core.gear_note_unmounted(&seen);
    diskutil("unmountDisk", &img.disk).unwrap();
    assert!(!img.is_mounted());

    let s = b.core.stage_device(None, Some(&id), None).unwrap();
    assert_eq!(s.clips.len(), 1);
    assert_eq!(*b.log.lock().unwrap(), [format!("mount {}", img.disk)]);
    assert!(img.is_mounted());

    // A card that is not plugged in is not mounted.
    let e = b
        .core
        .stage_device(None, Some("dvr-nothing"), None)
        .unwrap_err();
    assert!(format!("{e:#}").contains("not plugged in"), "{e:#}");
}

#[test]
fn card_prep_mounts_an_unmounted_card_for_the_plan_and_the_erase() {
    let img = Image::create("64m", "QCMC4", false);
    std::fs::create_dir_all(img.mount.join("DCIM")).unwrap();
    make_clip(&img.mount.join("DCIM/PICT0001.AVI"), 2, true);
    let b = bench(&img);
    let id = card_id(&b);
    let seen = b.core.gear_connected().unwrap();
    // Every clip on the card is in the library; the import unmounts the card.
    b.core.load(Some(&img.mount)).unwrap();
    b.core.import(&mp4()).unwrap();
    b.core.clear().unwrap();
    b.core.gear_note_unmounted(&seen);
    assert!(img.is_attached() && !img.is_mounted());
    b.log.lock().unwrap().clear();

    // The plan mounts, reads and unmounts.
    let plan = b.core.card_prep_plan_device(&id, Some("spare")).unwrap();
    assert_eq!(plan.label, "SPARE");
    assert_eq!(
        *b.log.lock().unwrap(),
        [
            format!("mount {}", img.disk),
            format!("unmount {}", img.disk)
        ]
    );
    assert!(img.is_attached() && !img.is_mounted());

    // A refused prep (wrong device) releases the card it mounted.
    b.log.lock().unwrap().clear();
    let bad = FormatRequest {
        device: "/dev/disk0".into(),
        volume_uuid: plan.volume_uuid.clone(),
        label: None,
        confirm: true,
    };
    let e = b.core.card_prep(&bad, false).unwrap_err();
    assert!(format!("{e:#}").starts_with("Refused"), "{e:#}");
    assert_eq!(
        *b.log.lock().unwrap(),
        [
            format!("mount {}", img.disk),
            format!("unmount {}", img.disk)
        ]
    );
    assert!(!img.is_mounted());

    // The erase mounts the card, erases it, and leaves it unmounted.
    b.log.lock().unwrap().clear();
    assert_is_test_image(&img);
    let done = b
        .core
        .card_prep(
            &FormatRequest {
                device: plan.device.clone(),
                volume_uuid: plan.volume_uuid.clone(),
                label: Some("spare".into()),
                confirm: true,
            },
            false,
        )
        .unwrap();
    assert_eq!(done.label, "SPARE");
    assert_eq!(
        b.log.lock().unwrap().first().unwrap(),
        &format!("mount {}", img.disk)
    );
    assert!(img.is_attached() && !img.is_mounted());
}
