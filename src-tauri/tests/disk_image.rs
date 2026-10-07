//! Card detection and the format step, on FAT32 disk images. These attach images with hdiutil
//! at private mount points. The only disk ever erased is a disk image created by the test,
//! and the test re-checks that before it lets the format run.

mod common;

use common::*;
use quadcam_lib::disk::{self, CardIdentity};
use quadcam_lib::media::{Encoder, Format};
use quadcam_lib::naming::NamePlanner;
use quadcam_lib::pipeline::{self, ClipJob, DateSource, ImportSettings, Outcome};
use quadcam_lib::sources::{self, CardPolicy, Source};
use std::path::Path;

fn analog() -> CardPolicy {
    sources::analog::Analog.card_policy()
}

fn put_clips(root: &Path) {
    std::fs::create_dir_all(root.join("DCIM")).unwrap();
    make_clip(&root.join("DCIM/PICT0001.AVI"), 2, true);
    make_clip(&root.join("DCIM/PICT0002.AVI"), 1, true);
}

/// Test-only belt and braces: the target must be a disk image this test attached.
fn assert_is_test_image(img: &Image) {
    let whole = disk::info(&img.disk).unwrap();
    assert_eq!(
        whole.bus_protocol.as_deref(),
        Some("Disk Image"),
        "refusing to format a non-image disk"
    );
    let vol = disk::info(&img.mount.to_string_lossy()).unwrap();
    assert_eq!(disk::whole_disk_of(&vol.parent_whole_disk), img.disk);
}

/// Check 1: a FAT32 image holding PICT files is flagged as a card; one without is not.
#[test]
fn detection() {
    let card = Image::create("64m", "QCCARD", false);
    put_clips(&card.mount);
    let v = disk::probe_volume(&card.mount).unwrap();
    assert!(v.is_card, "{v:?}");
    assert!(!v.is_radio);
    assert!(v.info.is_fat32(), "{:?}", v.info.filesystem);
    assert!(v.warnings.is_empty(), "{:?}", v.warnings);
    assert_eq!(disk::whole_disk_of(&v.info.parent_whole_disk), card.disk);

    let stick = Image::create("64m", "QCSTICK", false);
    std::fs::write(stick.mount.join("notes.txt"), b"hello").unwrap();
    std::fs::create_dir(stick.mount.join("Photos")).unwrap();
    std::fs::write(stick.mount.join("Photos/IMG_0001.JPG"), b"x").unwrap();
    let v = disk::probe_volume(&stick.mount).unwrap();
    assert!(!v.is_card);

    let radio = Image::create("64m", "QCRADIO", false);
    for d in ["LOGS", "MODELS", "RADIO"] {
        std::fs::create_dir(radio.mount.join(d)).unwrap();
    }
    let v = disk::probe_volume(&radio.mount).unwrap();
    assert!(v.is_radio && !v.is_card);
}

/// Check 7: the format runs only after every clip verified, then unmounts the card.
#[test]
fn format_after_verified_import() {
    let mut card = Image::create("64m", "QCFMT", false);
    put_clips(&card.mount);
    let vol = disk::probe_volume(&card.mount).unwrap();
    assert!(vol.is_card);
    let id = CardIdentity::from_info(&vol.info);

    let t = tools();
    let work = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let mut clips = pipeline::stage(
        &card.mount,
        &work.path().join("staging"),
        &mut |_, _, _, _| {},
    )
    .unwrap();
    for c in clips.iter_mut() {
        pipeline::analyse(&t, c, work.path()).unwrap();
    }
    assert_eq!(clips.len(), 2);

    // Before import: locked.
    assert!(pipeline::can_format(&clips, &[]).is_err());

    let settings = ImportSettings {
        output_dir: out.path().to_path_buf(),
        format: Format::Mp4,
        encoder: Encoder::Videotoolbox,
        keep_originals: false,
        add_time: false,
        default_name: "flight".into(),
        places: Vec::new(),
        profiles: Vec::new(),
        default_profile: None,
        layout: quadcam_lib::library::Layout::Flat,
        place_folders: false,
        import_id: String::new(),
        name_date_format: Default::default(),
    };
    let mut planner = NamePlanner::new();
    let mut results = Vec::new();
    for c in &clips {
        let job = ClipJob {
            id: c.id,
            skip: false,
            date: "2026-09-30".into(),
            time: None,
            source: DateSource::Import,
            name: String::new(),
            note: String::new(),
            meta: Default::default(),
            extra: Vec::new(),
            parts: Vec::new(),
        };
        let r = pipeline::import_clip(&t, c, &job, &settings, &mut planner, &mut |_| {});
        assert_eq!(r.outcome, Outcome::Verified, "{:?}", r.error);
        results.push(r);
    }
    // One clip verified, one not yet: still locked.
    assert!(pipeline::can_format(&clips, &results[..1]).is_err());
    pipeline::can_format(&clips, &results).unwrap();
    let names: Vec<_> = results
        .iter()
        .map(|r| r.output.as_ref().unwrap().file_name().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["2026-09-30_flight.mp4", "2026-09-30_flight-2.mp4"]);

    // A bad label is refused before anything runs.
    assert!(
        disk::format_card(&id, &card.mount, "WAY-TOO-LONG-NAME", &analog(), &|| Ok(())).is_err()
    );
    assert!(card.mount.join("DCIM/PICT0001.AVI").is_file());

    assert_is_test_image(&card);
    assert_eq!(
        disk::format_card(&id, &card.mount, "fpvcard", &analog(), &|| Ok(())).unwrap(),
        None,
        "erased and unmounted"
    );

    // Safe to remove at once: unmounted, still attached (the disk stays until it is pulled).
    assert!(
        card.is_attached() && !card.is_mounted(),
        "card should be unmounted after the erase"
    );
    // Mount again: an empty FAT32 volume named FPVCARD with no clips.
    card.remount();
    let after = disk::info(&card.mount.to_string_lossy()).unwrap();
    assert_eq!(after.volume_name.as_deref(), Some("FPVCARD"));
    assert!(after.is_fat32(), "{:?}", after.filesystem);
    assert!(!quadcam_lib::scan::has_clips(&card.mount));
}

/// Check 7: a swapped card, the internal disk and a disk over 64 GB are all refused.
#[test]
fn format_refusals() {
    // Swapped card: identity from A, then B is in the slot.
    let a = Image::create("64m", "QCA", false);
    let b = Image::create("64m", "QCB", false);
    let id_a = CardIdentity::from_info(&disk::info(&a.mount.to_string_lossy()).unwrap());
    let e = disk::format_card(&id_a, &b.mount, "FPVCARD", &analog(), &|| Ok(()))
        .unwrap_err()
        .to_string();
    assert!(e.contains("Refused"), "{e}");
    // Same slot, same device, different volume UUID (reformatted elsewhere, then reinserted).
    let mut forged = CardIdentity::from_info(&disk::info(&b.mount.to_string_lossy()).unwrap());
    forged.volume_uuid = id_a.volume_uuid.clone();
    let e = disk::format_card(&forged, &b.mount, "FPVCARD", &analog(), &|| Ok(()))
        .unwrap_err()
        .to_string();
    assert!(e.contains("UUID"), "{e}");
    assert!(b.is_attached() && a.is_attached());

    // The internal boot disk, even with a matching identity.
    let root = disk::info("/").unwrap();
    let id_root = CardIdentity::from_info(&root);
    let e = disk::verify_card_for_format(&id_root, Path::new("/"))
        .unwrap_err()
        .to_string();
    assert!(e.contains("Refused"), "{e}");

    // Over 64 GB: a sparse 70 GB FAT32 image.
    let big = Image::create("70g", "QCBIG", true);
    let id_big = CardIdentity::from_info(&disk::info(&big.mount.to_string_lossy()).unwrap());
    let e = disk::verify_card_for_format(&id_big, &big.mount)
        .unwrap_err()
        .to_string();
    assert!(e.contains("64 GB"), "{e}");
    assert!(big.is_attached());
}

/// "Safe to remove" unmounts a card's whole disk and leaves the disk attached, so it stays
/// listed until it is pulled. A disk image reports removable media, as a card does.
#[test]
fn safe_remove_unmounts_and_keeps_the_disk() {
    let mut card = Image::create("64m", "QCSAFE", false);
    put_clips(&card.mount);
    let whole = disk::info(&card.disk).unwrap();
    assert!(whole.removable, "{whole:?}");
    assert!(disk::is_removable(&whole));
    disk::safe_remove(&card.mount.to_string_lossy()).unwrap();
    assert!(card.is_attached() && !card.is_mounted());
    card.remount();
    assert!(card.is_mounted() && card.mount.join("DCIM/PICT0001.AVI").is_file());
}
