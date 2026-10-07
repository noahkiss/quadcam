//! Card prep (gear design 7.9): format a card with no session, new or with every clip in the
//! library. On FAT32 disk images this test creates; the only disk ever erased is such an
//! image, and the test checks `BusProtocol == "Disk Image"` before it lets an erase run. A
//! DJI goggles card is erased as exFAT, on such an image too.

mod common;

use common::*;
use quadcam_lib::core::{Core, FormatRequest, ImportOptions, NoHooks};
use quadcam_lib::disk::{self, CardIdentity};
use quadcam_lib::media::Format;
use quadcam_lib::photos::Recorder;
use quadcam_lib::sources::{self, Source};
use std::path::Path;
use std::sync::Arc;

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

fn core(work: &Path) -> Core {
    let core = Core::new(
        work.join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    );
    let mut d = core.defaults();
    d.output_dir = Some(work.join("out"));
    std::fs::create_dir_all(work.join("out")).unwrap();
    d.layout = quadcam_lib::library::Layout::Flat;
    core.set_defaults(d);
    core
}

fn err(r: anyhow::Result<impl std::fmt::Debug>) -> String {
    format!("{:#}", r.unwrap_err())
}

/// Prep refuses while a clip on the card is not in the library, passes once every clip is,
/// refuses a wrong device, UUID or a missing confirm, then erases and unmounts the image.
#[test]
fn prep_needs_every_clip_in_the_library() {
    let mut card = Image::create("64m", "QCPREP", false);
    // Every call below that could reach an erase targets this image.
    assert_is_test_image(&card);
    std::fs::create_dir_all(card.mount.join("DCIM")).unwrap();
    make_clip(&card.mount.join("DCIM/PICT0001.AVI"), 2, true);
    make_clip(&card.mount.join("DCIM/PICT0002.AVI"), 1, true);
    let work = tempfile::tempdir().unwrap();
    let core = core(work.path());

    // Nothing imported yet: both clips are missing.
    let e = err(core.card_prep_plan(&card.mount, None));
    assert!(e.starts_with("Refused: 2 of 2 clips"), "{e}");
    assert!(e.contains("PICT0001.AVI"), "{e}");

    // Staged in a session is not enough.
    core.load(Some(&card.mount)).unwrap();
    assert!(err(core.card_prep_plan(&card.mount, None)).contains("not in the library"));

    // Imported: the library has both, and the session goes away.
    core.import(&ImportOptions {
        format: Some(Format::Mp4),
        ..Default::default()
    })
    .unwrap();
    core.clear().unwrap();
    let plan = core.card_prep_plan(&card.mount, Some("spare1")).unwrap();
    assert_eq!(plan.clip_count, 2);
    assert_eq!(plan.label, "SPARE1");
    assert_eq!(plan.device, format!("/dev/{}", card.disk));
    assert!(!plan.volume_uuid.is_empty());

    // A new clip the DVR wrote since: refused again, by the plan and by the erase.
    let extra = card.mount.join("DCIM/PICT0003.AVI");
    make_clip(&extra, 3, true); // a length no other clip has: synthetic clips of one length match
    let e = err(core.card_prep_plan(&card.mount, None));
    assert!(e.starts_with("Refused: 1 of 3 clips"), "{e}");
    let req = FormatRequest {
        device: plan.device.clone(),
        volume_uuid: plan.volume_uuid.clone(),
        label: Some("SPARE1".into()),
        confirm: true,
    };
    assert!(err(core.card_prep(&req, false)).contains("not in the library"));
    assert!(card.is_attached() && extra.is_file());
    std::fs::remove_file(&extra).unwrap();

    // Wrong device, wrong UUID, no confirm: refused, nothing erased.
    for bad in [
        FormatRequest {
            device: "/dev/disk0".into(),
            ..req.clone()
        },
        FormatRequest {
            volume_uuid: "00000000-0000-0000-0000-000000000000".into(),
            ..req.clone()
        },
        FormatRequest {
            confirm: false,
            ..req.clone()
        },
    ] {
        let e = err(core.card_prep(&bad, false));
        assert!(e.starts_with("Refused"), "{e}");
    }
    assert!(card.is_attached() && card.mount.join("DCIM/PICT0001.AVI").is_file());

    // The guard right before the erase: a recheck that refuses stops `format_card`.
    let id = CardIdentity::from_info(&disk::info(&card.mount.to_string_lossy()).unwrap());
    let analog = sources::analog::Analog.card_policy();
    let e = disk::format_card(
        &id,
        &card.mount,
        "SPARE1",
        &analog,
        disk::EraseBy::Prep,
        &|| anyhow::bail!("Refused: recheck"),
    )
    .unwrap_err()
    .to_string();
    assert_eq!(e, "Refused: recheck");
    assert!(card.is_attached() && card.mount.join("DCIM/PICT0001.AVI").is_file());

    assert_is_test_image(&card);
    let done = core.card_prep(&req, false).unwrap();
    assert_eq!(done.label, "SPARE1");
    assert_eq!(done.filesystem, "FAT32");
    assert!(
        done.warnings.is_empty(),
        "a 64 MB card: {:?}",
        done.warnings
    );
    assert!(
        card.is_attached() && !card.is_mounted(),
        "unmounted after the erase"
    );
    card.remount();
    let after = disk::info(&card.mount.to_string_lossy()).unwrap();
    assert_eq!(after.volume_name.as_deref(), Some("SPARE1"));
    assert!(after.is_fat32(), "{:?}", after.filesystem);
    assert!(!quadcam_lib::scan::has_clips(&card.mount));
}

/// A card with no clips needs no library; a radio's SD card is refused.
#[test]
fn prep_plans_a_blank_card_and_refuses_a_radio() {
    let work = tempfile::tempdir().unwrap();
    let core = Core::new(
        work.path().join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    );

    let blank = Image::create("64m", "QCBLANK", false);
    std::fs::write(blank.mount.join("notes.txt"), b"hello").unwrap();
    let plan = core.card_prep_plan(&blank.mount, None).unwrap();
    assert_eq!(plan.clip_count, 0);
    assert_eq!(plan.label, "DVR");

    assert_eq!(plan.filesystem, "FAT32");

    let radio = Image::create("64m", "QCRADIO", false);
    for d in ["LOGS", "MODELS", "RADIO"] {
        std::fs::create_dir(radio.mount.join(d)).unwrap();
    }
    let e = err(core.card_prep_plan(&radio.mount, None));
    assert!(e.contains("radio"), "{e}");
    assert!(radio.is_attached());
}

/// A removable DJI goggles card: prep refuses while a clip is not in the library, then plans
/// and erases it as exFAT (its `CardPolicy`). An emptied DJI card plans as exFAT too. The
/// format after an import stays refused for DJI. Erased only on a disk image.
#[test]
fn prep_erases_a_dji_goggles_card_as_exfat() {
    let mut card = Image::create("64m", "QCDJI", false);
    assert_is_test_image(&card);
    let dir = card.mount.join("DCIM/DJI_001");
    std::fs::create_dir_all(&dir).unwrap();
    let clip = dir.join("DJI_20260105120000_0001_D.MP4");
    make_dji_clip(&clip, 1, "2026-01-05T17:00:00Z");
    let work = tempfile::tempdir().unwrap();
    let core = core(work.path());

    let e = err(core.card_prep_plan(&card.mount, None));
    assert!(e.starts_with("Refused: 1 of 1 clips"), "{e}");

    // After an import, "Format card" stays refused for DJI; card prep is the way.
    core.load(Some(&card.mount)).unwrap();
    core.import(&ImportOptions {
        format: Some(Format::Mp4),
        ..Default::default()
    })
    .unwrap();
    let e = err(core.format_plan(None));
    assert!(e.contains("after an import"), "{e}");
    core.clear().unwrap();

    let plan = core.card_prep_plan(&card.mount, Some("GOGGLES")).unwrap();
    assert_eq!(plan.clip_count, 1);
    assert_eq!(plan.filesystem, "exFAT");
    assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    let req = FormatRequest {
        device: plan.device.clone(),
        volume_uuid: plan.volume_uuid.clone(),
        label: Some("GOGGLES".into()),
        confirm: true,
    };
    assert_is_test_image(&card);
    let done = core.card_prep(&req, false).unwrap();
    assert_eq!(done.filesystem, "exFAT");
    assert!(card.is_attached() && !card.is_mounted());
    card.remount();
    let after = disk::info(&card.mount.to_string_lossy()).unwrap();
    assert_eq!(after.volume_name.as_deref(), Some("GOGGLES"));
    assert!(
        after
            .filesystem
            .as_deref()
            .is_some_and(|f| f.to_ascii_lowercase().contains("exfat")),
        "{:?}",
        after.filesystem
    );
    assert!(!dir.exists());

    // An emptied DJI card (folders, no clips) plans as a goggles card: exFAT.
    let emptied = Image::create("64m", "QCDJI2", false);
    std::fs::create_dir_all(emptied.mount.join("DCIM/DJI_001")).unwrap();
    let plan = core.card_prep_plan(&emptied.mount, None).unwrap();
    assert_eq!((plan.clip_count, plan.filesystem.as_str()), (0, "exFAT"));
}

/// The CLI (`format --prep`) and the MCP tool (`quadcam_format_card` with `prep`) reach the
/// same plan and the same refusals. Nothing is erased here.
#[test]
fn prep_through_cli_and_mcp() {
    use quadcam_lib::mcp::{LocalBackend, Server};
    use serde_json::{json, Value};

    let card = Image::create("64m", "QCSURF", false);
    assert_is_test_image(&card);
    let home = tempfile::tempdir().unwrap();
    let cli = |args: &[&str]| -> (i32, Value) {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
            .env("HOME", home.path())
            .env("QUADCAM_PHOTOS", "dry-run")
            .arg("--json")
            .args(args)
            .output()
            .unwrap();
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        (out.status.code().unwrap(), v)
    };
    let mount = card.mount.to_string_lossy().to_string();
    let (code, v) = cli(&["format", "--prep", "--plan", "--mount", &mount]);
    assert_eq!(code, 0, "{v}");
    let plan = &v["result"];
    assert_eq!(plan["clip_count"], 0);
    let device = plan["device"].as_str().unwrap().to_string();
    let uuid = plan["volume_uuid"].as_str().unwrap().to_string();
    assert_eq!(device, format!("/dev/{}", card.disk));

    let (code, v) = cli(&[
        "format",
        "--prep",
        "--device",
        &device,
        "--volume-uuid",
        &uuid,
    ]);
    assert_eq!(code, 3, "no --yes: {v}");
    let (code, v) = cli(&[
        "format",
        "--prep",
        "--device",
        &device,
        "--volume-uuid",
        "00000000-0000-0000-0000-000000000000",
        "--yes",
    ]);
    assert_eq!(code, 3, "wrong UUID: {v}");
    assert!(card.is_attached());

    let work = tempfile::tempdir().unwrap();
    let core = Core::new(
        work.path().join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    );
    let mut s = Server::new(LocalBackend(Arc::new(core)));
    let r = s.call_tool(
        "quadcam_format_card",
        json!({"prep": true, "dry_run": true}),
    );
    assert_eq!(r["isError"], true, "prep's dry run needs mount");
    let r = s.call_tool(
        "quadcam_format_card",
        json!({"prep": true, "dry_run": true, "mount": mount}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["device"], device.as_str());
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("prep=true"));
    let r = s.call_tool(
        "quadcam_format_card",
        json!({"prep": true, "device": device, "volume_uuid": "WRONG", "confirm": true}),
    );
    assert_eq!(r["isError"], true, "{r}");
    let r = s.call_tool(
        "quadcam_format_card",
        json!({"prep": true, "device": device, "volume_uuid": uuid}),
    );
    assert_eq!(r["isError"], true, "no confirm: {r}");
    assert!(card.is_attached());
}
