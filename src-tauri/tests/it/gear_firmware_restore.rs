//! The restore of a saved EdgeTX firmware copy: the list, the plan's guards, the confirm, the
//! backup of the firmware the radio runs now, the write and its read back. The radio is a
//! fake DFU device; nothing here opens a USB device or flashes anything real.

use quadcam_lib::core::{
    Core, FirmwareCopiesParams, FirmwareRestoreParams, FirmwareRestoreRequest, Hooks, NoHooks,
};
use quadcam_lib::gear::apply::StepState::{Done, Failed, Skipped};
use quadcam_lib::gear::detect::DfuInfo;
use quadcam_lib::gear::dfu::FakeDfu;
use quadcam_lib::gear::firmware::{FixtureFetch, FwEnv, Recorder as FlashRecorder};
use quadcam_lib::gear::fwcopy::{self, CopyKind, FwCopy};
use quadcam_lib::gear::model::{
    ApplyPlan, ChangeStatus, Device, DeviceKind, Identity, Refusal, RefusalCode, StagedChange,
};
use quadcam_lib::photos::Recorder;
use serde_json::json;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A full image for `board`: a bootloader vector table, the firmware's at 0x8000, and the
/// version string. `salt` makes two images differ.
fn image(salt: u8, board: &str, version: &str) -> Vec<u8> {
    let mut b = vec![salt; 600 * 1024];
    b[0..4].copy_from_slice(&0x2002_0000u32.to_le_bytes());
    b[4..8].copy_from_slice(&0x0800_0101u32.to_le_bytes());
    b[0x8000..0x8004].copy_from_slice(&0x2001_FFF0u32.to_le_bytes());
    b[0x8004..0x8008].copy_from_slice(&0x0800_8201u32.to_le_bytes());
    let v = format!("edgetx-{board}-{version} (def35ad3)\0");
    b[0x9000..0x9000 + v.len()].copy_from_slice(v.as_bytes());
    b
}

#[derive(Default)]
struct Gui {
    deny: bool,
    asked: AtomicUsize,
}

impl Hooks for Gui {
    fn has_gui(&self) -> bool {
        true
    }
    fn confirm_apply(&self, change: &StagedChange, _plan: &ApplyPlan) -> anyhow::Result<()> {
        assert_eq!(change.id, "firmware-restore");
        assert_eq!(change.title, "Restore firmware");
        self.asked.fetch_add(1, Ordering::SeqCst);
        if self.deny {
            anyhow::bail!("Refused: the user cancelled the apply in quadcam.")
        }
        Ok(())
    }
}

struct Bench {
    _dir: tempfile::TempDir,
    core: Arc<Core>,
    flasher: Arc<FlashRecorder>,
    /// The firmware the radio runs now.
    current: Vec<u8>,
    radio: String,
}

fn bench_with(hooks: Arc<dyn Hooks>, flasher: Option<Arc<FlashRecorder>>) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let current = image(3, "pocket", "2.12.4");
    let flasher =
        flasher.unwrap_or_else(|| Arc::new(FlashRecorder::new(FakeDfu::with_firmware(&current))));
    let dfu = Arc::new(Mutex::new(vec![DfuInfo {
        vid: 0x0483,
        pid: 0xdf11,
        serial: Some("0001".into()),
    }]));
    let mut env = quadcam_lib::gear::Env::fake(
        vec![],
        Arc::new(quadcam_lib::gear::serial::FakePorts::new(vec![])),
    );
    env.dfu = Arc::new(move || dfu.lock().unwrap().clone());
    let core = Arc::new(
        Core::new(
            dir.path().join("cache"),
            None,
            hooks,
            Arc::new(Recorder::default()),
        )
        .with_settings(dir.path().join("support/settings.json"))
        .with_gear_env(env)
        .with_firmware_env(FwEnv {
            fetch: Arc::new(FixtureFetch::new()),
            flasher: flasher.clone(),
        }),
    );
    let radio = Device {
        id: "radio-0000000000000001".into(),
        kind: DeviceKind::Radio,
        name: "Pocket".into(),
        aircraft: None,
        identity: Identity {
            board: Some("pocket".into()),
            firmware: Some("EdgeTX".into()),
            version: Some("2.12.4".into()),
            ..Identity::default()
        },
        last_seen: None,
        last_backup: Some("2026-10-01T100000-manual".to_string()),
        last_space: None,
        aliases: Vec::new(),
        dfu_serial: None,
        radio_aircraft: None,
    };
    core.gear_store().save_device(&radio).unwrap();
    Bench {
        _dir: dir,
        core,
        flasher,
        current,
        radio: radio.id,
    }
}

fn bench(hooks: Arc<dyn Hooks>) -> Bench {
    bench_with(hooks, None)
}

/// Saves `bytes` as a Read copy of the radio, taken at `at`.
fn save(b: &Bench, at: &str, bytes: &[u8]) -> FwCopy {
    fwcopy::save(
        &b.core.gear_store(),
        &b.radio,
        CopyKind::Read,
        at.parse().unwrap(),
        bytes,
        quadcam_lib::gear::firmware::edgetx::image_identity(bytes),
    )
    .unwrap()
}

fn params(b: &Bench, c: &FwCopy) -> FirmwareRestoreParams {
    FirmwareRestoreParams {
        device: b.radio.clone(),
        copy: c.id.clone(),
    }
}

fn request(p: &FirmwareRestoreParams, plan: &ApplyPlan) -> FirmwareRestoreRequest {
    FirmwareRestoreRequest {
        params: p.clone(),
        digest: plan.digest.clone(),
        confirm: true,
    }
}

fn failed(plan: &ApplyPlan) -> Vec<(&str, RefusalCode)> {
    plan.checks
        .iter()
        .filter_map(|c| c.refusal.as_ref().map(|r| (c.name.as_str(), r.code)))
        .collect()
}

fn refusal(e: anyhow::Error) -> Refusal {
    e.downcast::<Refusal>().expect("a refusal")
}

#[test]
fn copies_list_per_radio_newest_first_with_their_version() {
    let b = bench(Arc::new(NoHooks));
    let old = save(&b, "2026-10-01T12:00:00Z", &image(7, "pocket", "2.11.2"));
    let new = save(&b, "2026-10-05T12:00:00Z", &b.current.clone());
    // A read that named no radio, from a chip no radio holds the link to.
    let loose = fwcopy::save(
        &b.core.gear_store(),
        "dfu-0002",
        CopyKind::Read,
        "2026-10-03T12:00:00Z".parse().unwrap(),
        &image(8, "pocket", "2.12.4"),
        None,
    )
    .unwrap();
    let all = b
        .core
        .gear_firmware_copies(&FirmwareCopiesParams::default())
        .unwrap();
    let ids: Vec<(&str, Option<&str>)> = all
        .iter()
        .map(|c| (c.copy.id.as_str(), c.radio.as_deref()))
        .collect();
    assert_eq!(
        ids,
        [
            (new.id.as_str(), Some(b.radio.as_str())),
            (loose.id.as_str(), None),
            (old.id.as_str(), Some(b.radio.as_str())),
        ]
    );
    assert_eq!(all[2].copy.image_version.as_deref(), Some("2.11.2"));
    assert_eq!(all[2].copy.size, 600 * 1024);
    assert_eq!(all[2].copy.sha256.len(), 64);

    // One radio's: its own folder only.
    let one = b
        .core
        .gear_firmware_copies(&FirmwareCopiesParams {
            device: Some(b.radio.clone()),
        })
        .unwrap();
    assert_eq!(one.len(), 2);
    // Once the chip is linked to the radio, its copies are the radio's.
    let mut d = b.core.gear_store().devices().unwrap().remove(0);
    d.dfu_serial = Some("0002".into());
    b.core.gear_store().save_device(&d).unwrap();
    let one = b
        .core
        .gear_firmware_copies(&FirmwareCopiesParams {
            device: Some(b.radio.clone()),
        })
        .unwrap();
    assert_eq!(one.len(), 3);
    assert!(b
        .core
        .gear_firmware_copies(&FirmwareCopiesParams {
            device: Some("nope".into()),
        })
        .is_err());
}

#[test]
fn a_restore_backs_up_the_current_firmware_writes_reads_back_and_leaves() {
    let gui = Arc::new(Gui::default());
    let b = bench(gui.clone());
    let want = image(7, "pocket", "2.11.2");
    let c = save(&b, "2026-10-01T12:00:00Z", &want);
    let p = params(&b, &c);
    let plan = b.core.gear_firmware_restore_plan(&p).unwrap();
    assert!(plan.ready(), "{:?}", failed(&plan));
    assert_eq!(plan.change, "firmware-restore");
    let names: Vec<&str> = plan.checks.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Copy matches its SHA-256",
            "Known board",
            "Copy is for this radio's board",
            "Firmware image",
            "Card backup",
            "One radio in DFU mode"
        ]
    );
    // 2.11.2 is not a proven release flash: a warning, not a refusal.
    assert!(
        plan.warnings.iter().any(|w| w.contains("EdgeTX 2.11.2")),
        "{:?}",
        plan.warnings
    );
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.contains("A verified restore links it")),
        "{:?}",
        plan.warnings
    );
    // The plan opened nothing.
    assert!(b.flasher.opened.lock().unwrap().is_empty());

    let r = b.core.gear_firmware_restore(&request(&p, &plan)).unwrap();
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    assert!(r.saved);
    assert_eq!(r.change, "firmware-restore");
    assert!(r.message.contains(&c.id), "{}", r.message);
    assert_eq!(
        gui.asked.load(Ordering::SeqCst),
        1,
        "the person was asked once"
    );
    let steps: Vec<_> = r.steps.iter().map(|s| (s.name.as_str(), s.state)).collect();
    assert_eq!(
        steps,
        [
            ("Copy the current firmware", Done),
            ("Erase", Done),
            ("Write", Done),
            ("Read back", Done),
            ("Leave DFU", Done)
        ]
    );
    let dev = b.flasher.device.lock().unwrap();
    assert!(dev.left, "the radio left DFU");
    assert_eq!(
        &dev.flash[..want.len()],
        &want[..],
        "the copy, byte for byte"
    );
    drop(dev);

    // The firmware it ran before is kept as a new copy, read back from disk.
    let copies = fwcopy::list(&b.core.gear_store(), &b.radio);
    assert_eq!(copies.len(), 2);
    let kept = copies
        .iter()
        .find(|x| x.kind == CopyKind::BeforeRestore)
        .unwrap();
    assert_eq!(r.backup.as_deref(), Some(kept.id.as_str()));
    assert!(kept.id.ends_with("-before-restore"), "{}", kept.id);
    let bytes = fwcopy::read(&b.core.gear_store(), kept).unwrap();
    assert_eq!(&bytes[..], &b.current[..bytes.len()]);
    // A verified restore links the DFU chip to the radio.
    let d = b.core.gear_store().devices().unwrap().remove(0);
    assert_eq!(d.dfu_serial.as_deref(), Some("0001"));
}

#[test]
fn no_second_copy_when_a_saved_copy_already_holds_the_current_firmware() {
    let b = bench(Arc::new(NoHooks));
    let held = save(&b, "2026-10-05T12:00:00Z", &b.current.clone());
    let c = save(&b, "2026-10-01T12:00:00Z", &image(7, "pocket", "2.12.4"));
    let p = params(&b, &c);
    let plan = b.core.gear_firmware_restore_plan(&p).unwrap();
    let r = b.core.gear_firmware_restore(&request(&p, &plan)).unwrap();
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    assert_eq!(r.backup.as_deref(), Some(held.id.as_str()));
    assert!(
        r.steps[0]
            .detail
            .as_deref()
            .unwrap()
            .contains(&format!("already saved as {}", held.id)),
        "{:?}",
        r.steps[0]
    );
    assert_eq!(fwcopy::list(&b.core.gear_store(), &b.radio).len(), 2);
    // A saved copy whose file no longer matches its hash is no backup: a new one is taken.
    let b = bench(Arc::new(NoHooks));
    let held = save(&b, "2026-10-05T12:00:00Z", &b.current.clone());
    let c = save(&b, "2026-10-01T12:00:00Z", &image(7, "pocket", "2.12.4"));
    let file = b
        .core
        .gear_store()
        .root()
        .join("firmware")
        .join(&b.radio)
        .join(format!("{}.bin", held.id.rsplit('/').next().unwrap()));
    std::fs::write(&file, b"damaged").unwrap();
    let p = params(&b, &c);
    let plan = b.core.gear_firmware_restore_plan(&p).unwrap();
    let r = b.core.gear_firmware_restore(&request(&p, &plan)).unwrap();
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    assert_ne!(r.backup.as_deref(), Some(held.id.as_str()));
    assert_eq!(fwcopy::list(&b.core.gear_store(), &b.radio).len(), 3);
}

#[test]
fn a_copy_that_fails_its_hash_is_refused_and_nothing_is_opened() {
    let b = bench(Arc::new(NoHooks));
    let good = save(&b, "2026-10-01T12:00:00Z", &image(7, "pocket", "2.12.4"));
    let p = params(&b, &good);
    let plan = b.core.gear_firmware_restore_plan(&p).unwrap();
    assert!(plan.ready());
    // The file changes on disk after the plan.
    let file = b
        .core
        .gear_store()
        .root()
        .join("firmware")
        .join(&b.radio)
        .join(format!("{}.bin", good.id.rsplit('/').next().unwrap()));
    let mut bytes = std::fs::read(&file).unwrap();
    bytes[0x10000] ^= 1;
    std::fs::write(&file, &bytes).unwrap();
    let again = b.core.gear_firmware_restore_plan(&p).unwrap();
    assert_eq!(
        failed(&again),
        [("Copy matches its SHA-256", RefusalCode::BadImage)]
    );
    assert!(again.digest.is_empty());
    let e = refusal(
        b.core
            .gear_firmware_restore(&request(&p, &plan))
            .unwrap_err(),
    );
    assert_eq!(e.code, RefusalCode::BadImage);
    assert!(
        e.reason.contains("does not match its recorded hash"),
        "{}",
        e.reason
    );
    assert!(b.flasher.opened.lock().unwrap().is_empty());
    assert_eq!(b.flasher.device.lock().unwrap().mutations, 0);
}

#[test]
fn a_copy_for_another_board_or_not_a_full_image_is_refused() {
    let b = bench(Arc::new(NoHooks));
    let other = save(&b, "2026-10-01T12:00:00Z", &image(7, "tx16s", "2.12.4"));
    let p = params(&b, &other);
    let plan = b.core.gear_firmware_restore_plan(&p).unwrap();
    assert_eq!(
        failed(&plan),
        [("Copy is for this radio's board", RefusalCode::Incompatible)]
    );
    let reason = &plan.checks[2].refusal.as_ref().unwrap().reason;
    assert!(reason.contains("board tx16s, not pocket"), "{reason}");
    let e = refusal(
        b.core
            .gear_firmware_restore(&FirmwareRestoreRequest {
                params: p,
                digest: "x".into(),
                confirm: true,
            })
            .unwrap_err(),
    );
    assert_eq!(e.code, RefusalCode::Incompatible);

    // A firmware-only image (no bootloader) would overwrite the bootloader.
    let mut bare = image(7, "pocket", "2.12.4");
    bare[0..8].fill(0x11);
    let bare = save(&b, "2026-10-02T12:00:00Z", &bare);
    let plan = b
        .core
        .gear_firmware_restore_plan(&params(&b, &bare))
        .unwrap();
    assert_eq!(failed(&plan), [("Firmware image", RefusalCode::BadImage)]);
    // Bytes with no EdgeTX string are not EdgeTX firmware.
    let none = save(&b, "2026-10-03T12:00:00Z", &vec![0x42; 500 * 1024]);
    let plan = b
        .core
        .gear_firmware_restore_plan(&params(&b, &none))
        .unwrap();
    assert_eq!(
        failed(&plan),
        [("Copy is for this radio's board", RefusalCode::BadImage)]
    );
    assert!(b.flasher.opened.lock().unwrap().is_empty());
    // Another radio's copy, or an FC, is not this radio's to restore.
    assert!(b
        .core
        .gear_firmware_restore_plan(&FirmwareRestoreParams {
            device: b.radio.clone(),
            copy: "radio-9/2026-10-01T120000-read".into(),
        })
        .is_err());
}

#[test]
fn a_bad_read_back_is_reported_and_keeps_the_radio_in_dfu_and_the_backup() {
    let b = bench(Arc::new(NoHooks));
    let c = save(&b, "2026-10-01T12:00:00Z", &image(7, "pocket", "2.12.4"));
    b.flasher.device.lock().unwrap().faults.drop_write_block = Some(40);
    let p = params(&b, &c);
    let plan = b.core.gear_firmware_restore_plan(&p).unwrap();
    let r = b.core.gear_firmware_restore(&request(&p, &plan)).unwrap();
    assert_eq!(r.status, ChangeStatus::Failed);
    assert!(!r.saved);
    assert!(
        r.message.starts_with("The restore failed:"),
        "{}",
        r.message
    );
    assert!(r.message.contains("different bytes"), "{}", r.message);
    assert!(r.message.contains("stays in DFU mode"), "{}", r.message);
    let steps: Vec<_> = r.steps.iter().map(|s| (s.name.as_str(), s.state)).collect();
    assert_eq!(
        steps,
        [
            ("Copy the current firmware", Done),
            ("Erase", Done),
            ("Write", Failed),
            ("Read back", Skipped),
            ("Leave DFU", Skipped)
        ]
    );
    assert!(
        !b.flasher.device.lock().unwrap().left,
        "never starts a bad image"
    );
    let kept = r.backup.clone().unwrap();
    assert!(r.message.contains(&kept), "{}", r.message);
    // The DFU chip is not linked by a failed restore.
    assert!(b.core.gear_store().devices().unwrap()[0]
        .dfu_serial
        .is_none());
}

#[test]
fn a_restore_needs_confirm_the_digest_and_the_persons_click() {
    let gui = Arc::new(Gui {
        deny: true,
        ..Gui::default()
    });
    let b = bench(gui.clone());
    let c = save(&b, "2026-10-01T12:00:00Z", &image(7, "pocket", "2.12.4"));
    let p = params(&b, &c);
    let plan = b.core.gear_firmware_restore_plan(&p).unwrap();
    let mut req = request(&p, &plan);
    req.confirm = false;
    let e = b.core.gear_firmware_restore(&req).unwrap_err();
    assert!(e.to_string().contains("confirm=true"), "{e}");
    req.confirm = true;
    req.digest = "0000".into();
    let e = refusal(b.core.gear_firmware_restore(&req).unwrap_err());
    assert_eq!(e.code, RefusalCode::BeforeMismatch);
    assert_eq!(gui.asked.load(Ordering::SeqCst), 0);
    // The person cancels in the app: nothing is opened.
    let e = b
        .core
        .gear_firmware_restore(&request(&p, &plan))
        .unwrap_err();
    assert!(e.to_string().contains("cancelled"), "{e}");
    assert_eq!(gui.asked.load(Ordering::SeqCst), 1);
    assert!(b.flasher.opened.lock().unwrap().is_empty());
    // The sheet's own Restore click is the confirm, and does not ask again.
    let r = b
        .core
        .gear_firmware_restore_click(&request(&p, &plan))
        .unwrap();
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    assert_eq!(gui.asked.load(Ordering::SeqCst), 1);
    // No DFU radio, no card backup: refused in the plan.
    let mut d = b.core.gear_store().devices().unwrap().remove(0);
    d.last_backup = None;
    b.core.gear_store().save_device(&d).unwrap();
    let plan = b.core.gear_firmware_restore_plan(&p).unwrap();
    assert!(failed(&plan).contains(&("Card backup", RefusalCode::NoBackup)));
}

#[test]
fn under_cargo_a_restore_reaches_only_the_fake_usb_path() {
    // A core given no firmware env takes `FwEnv::system()`: under cargo, a recorder whose
    // fake device has an empty flash. The restore writes that fake, never a real radio.
    assert!(!quadcam_lib::gear::firmware::flash_enabled(None, true));
    if std::env::var("QUADCAM_FLASH").as_deref() == Ok("real") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut env = quadcam_lib::gear::Env::fake(
        vec![],
        Arc::new(quadcam_lib::gear::serial::FakePorts::new(vec![])),
    );
    env.dfu = Arc::new(|| {
        vec![DfuInfo {
            vid: 0x0483,
            pid: 0xdf11,
            serial: Some("0001".into()),
        }]
    });
    let core = Core::new(
        dir.path().join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(dir.path().join("support/settings.json"))
    .with_gear_env(env);
    let b = Bench {
        _dir: dir,
        core: Arc::new(core),
        flasher: Arc::new(FlashRecorder::empty()),
        current: Vec::new(),
        radio: "radio-0000000000000001".into(),
    };
    let radio = Device {
        id: b.radio.clone(),
        kind: DeviceKind::Radio,
        name: "Pocket".into(),
        aircraft: None,
        identity: Identity {
            board: Some("pocket".into()),
            version: Some("2.12.4".into()),
            ..Identity::default()
        },
        last_seen: None,
        last_backup: Some("x".into()),
        last_space: None,
        aliases: Vec::new(),
        dfu_serial: None,
        radio_aircraft: None,
    };
    b.core.gear_store().save_device(&radio).unwrap();
    let c = save(&b, "2026-10-01T12:00:00Z", &image(7, "pocket", "2.12.4"));
    let p = params(&b, &c);
    let plan = b.core.gear_firmware_restore_plan(&p).unwrap();
    assert!(plan.ready(), "{:?}", failed(&plan));
    let r = b.core.gear_firmware_restore(&request(&p, &plan)).unwrap();
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    // The fake's flash was blank: that is the recorder, not a radio.
    assert_eq!(
        r.steps[0].detail.as_deref(),
        Some("the flash is blank; there is no firmware to keep")
    );
    assert!(r.backup.is_none());
}

// ----- MCP and CLI -----

#[test]
fn the_mcp_actions_list_plan_and_restore_with_a_digest() {
    let b = bench(Arc::new(NoHooks));
    let c = save(&b, "2026-10-01T12:00:00Z", &image(7, "pocket", "2.12.4"));
    let mut s = quadcam_lib::mcp::Server::new(quadcam_lib::mcp::LocalBackend(b.core.clone()));
    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "firmware_copies", "device": b.radio}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let t = r["content"][0]["text"].as_str().unwrap();
    assert!(t.contains(&c.id), "{t}");
    assert!(t.contains("EdgeTX 2.12.4 for pocket"), "{t}");
    assert!(t.contains("600 KB"), "{t}");

    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "firmware_restore_plan", "device": b.radio}),
    );
    assert_eq!(r["isError"], true, "id is required");
    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "firmware_restore_plan", "device": b.radio, "id": c.id}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let t = r["content"][0]["text"].as_str().unwrap();
    assert!(t.contains("ok Copy matches its SHA-256"), "{t}");
    let digest = r["structuredContent"]["digest"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(t.contains(&format!("digest={digest}")), "{t}");

    // Refused without confirm; nothing opened.
    let r = s.call_tool(
        "quadcam_gear_apply",
        json!({"action": "firmware_restore", "device": b.radio, "id": c.id, "digest": digest}),
    );
    assert_eq!(r["isError"], true);
    assert!(b.flasher.opened.lock().unwrap().is_empty());
    let r = s.call_tool(
        "quadcam_gear_apply",
        json!({"action": "firmware_restore", "device": b.radio, "id": c.id, "digest": digest, "confirm": true}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let t = r["content"][0]["text"].as_str().unwrap();
    assert!(t.starts_with("Verified:"), "{t}");
    assert!(t.contains("Backup before (kept)"), "{t}");
    assert!(b.flasher.device.lock().unwrap().left);
}

#[test]
fn the_cli_lists_copies_and_a_restore_needs_a_device_a_digest_and_yes() {
    let home = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
            .arg("--json")
            .args(args)
            .env("HOME", home.path())
            .env("QUADCAM_PHOTOS", "dry-run")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        (
            out.status.code().unwrap(),
            serde_json::from_str::<serde_json::Value>(text.trim())
                .unwrap_or(serde_json::Value::Null),
            text,
        )
    };
    let (code, v, text) = run(&["gear", "firmware", "--copies"]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(v["result"], json!([]));
    let (code, _, text) = run(&["gear", "firmware", "--copies", "--plan"]);
    assert_ne!(code, 0, "{text}");
    let (code, v, text) = run(&["gear", "firmware", "--restore", "r/x"]);
    assert_ne!(code, 0, "{text}");
    assert!(text.contains("--device is required"), "{v}");
    let (code, v, text) = run(&[
        "gear",
        "firmware",
        "--restore",
        "r/x",
        "--device",
        "r",
        "--yes",
    ]);
    assert_ne!(code, 0, "{text}");
    assert!(text.contains("--digest"), "{v}");
    let (code, v, text) = run(&[
        "gear",
        "firmware",
        "--restore",
        "r/x",
        "--device",
        "r",
        "--plan",
    ]);
    assert_ne!(code, 0, "{text}");
    assert!(text.contains("No saved device r"), "{v}");
}
