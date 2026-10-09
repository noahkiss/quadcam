//! Firmware (WP10): the check, the splash preview, and the EdgeTX flash plan and apply. The
//! network is a fixture server in memory and the radio is a fake DFU device; nothing here
//! downloads, opens a USB device or flashes anything real.

use quadcam_lib::core::{
    BackupFilter, Core, FirmwareParams, FirmwareReadParams, FlashParams, FlashRequest, Hooks,
    NoHooks,
};
use quadcam_lib::gear::detect::DfuInfo;
use quadcam_lib::gear::dfu::{FakeDfu, FLASH_BASE};
use quadcam_lib::gear::firmware::check::{
    FirmwareState, BETAFLIGHT_RELEASES, EDGETX_RELEASES, ELRS_INDEX,
};
use quadcam_lib::gear::firmware::edgetx::{asset_name, release_url, DOWNLOAD_PREFIX};
use quadcam_lib::gear::firmware::{FixtureFetch, FwEnv, Recorder as FlashRecorder};
use quadcam_lib::gear::model::{
    ApplyPlan, ChangeStatus, Device, DeviceKind, Identity, Refusal, RefusalCode,
};
use quadcam_lib::gear::splash::{self, SplashParams};
use quadcam_lib::photos::Recorder;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const VERSION: &str = "2.12.4";

/// A full image: a bootloader vector table, the firmware's at 0x8000, the splash markers.
fn full_image(salt: u8) -> Vec<u8> {
    let mut b = vec![salt; 600 * 1024];
    b[0..4].copy_from_slice(&0x2002_0000u32.to_le_bytes());
    b[4..8].copy_from_slice(&0x0800_0101u32.to_le_bytes());
    b[0x8000..0x8004].copy_from_slice(&0x2001_FFF0u32.to_le_bytes());
    b[0x8004..0x8008].copy_from_slice(&0x0800_8201u32.to_le_bytes());
    // The version string the real image carries.
    let v = b"edgetx-pocket-2.12.4 (def35ad3)\0";
    b[0x9000..0x9000 + v.len()].copy_from_slice(v);
    let at = 0x20000;
    b[at..at + 4].copy_from_slice(splash::START);
    b[at + 4] = 0x80;
    b[at + 5] = 0x40;
    b[at + 6..at + 6 + splash::IMAGE_BYTES].fill(0);
    let end = at + 6 + splash::IMAGE_BYTES;
    b[end..end + 3].copy_from_slice(splash::END);
    b
}

fn release_zip(dir: &Path, pocket: &[u8]) -> PathBuf {
    let src = dir.join("zip-src");
    let _ = std::fs::remove_dir_all(&src);
    std::fs::create_dir_all(&src).unwrap();
    // The names and extra files of the real 2.12.4 release zip.
    std::fs::write(src.join("pocket-def35ad.bin"), pocket).unwrap();
    std::fs::write(src.join("tx16s-def35ad.bin"), full_image(9)).unwrap();
    std::fs::write(src.join("fw.json"), b"{}").unwrap();
    std::fs::write(src.join("LICENSE"), b"x").unwrap();
    let zip = dir.join(asset_name(VERSION));
    let st = Command::new("/usr/bin/ditto")
        .args(["-c", "-k", "--sequesterRsrc"])
        .arg(&src)
        .arg(&zip)
        .status()
        .unwrap();
    assert!(st.success());
    zip
}

fn serve_release(f: &FixtureFetch, zip: &Path) {
    let bytes = std::fs::read(zip).unwrap();
    let url = format!("{DOWNLOAD_PREFIX}v{VERSION}/{}", asset_name(VERSION));
    f.serve(
        &release_url(VERSION),
        serde_json::to_vec(&json!({
            "tag_name": format!("v{VERSION}"), "prerelease": false, "draft": false,
            "assets": [{"name": asset_name(VERSION), "browser_download_url": url, "size": bytes.len()}]
        }))
        .unwrap(),
    );
    f.serve(&url, bytes);
}

fn serve_indexes(f: &FixtureFetch) {
    let rel = |t: &str| json!([{"tag_name": t, "prerelease": false, "draft": false, "assets": []}]);
    f.serve(
        EDGETX_RELEASES,
        serde_json::to_vec(&rel("v2.12.4")).unwrap(),
    );
    f.serve(
        BETAFLIGHT_RELEASES,
        serde_json::to_vec(&rel("2026.6.0")).unwrap(),
    );
    f.serve(
        ELRS_INDEX,
        json!({"tags": {"3.5.3": "x"}}).to_string().into_bytes(),
    );
}

#[derive(Default)]
struct Gui {
    deny: bool,
    asked: AtomicUsize,
    changed: AtomicUsize,
}

impl Hooks for Gui {
    fn has_gui(&self) -> bool {
        true
    }
    fn confirm_apply(
        &self,
        change: &quadcam_lib::gear::model::StagedChange,
        _plan: &ApplyPlan,
    ) -> anyhow::Result<()> {
        assert_eq!(change.id, "flash");
        self.asked.fetch_add(1, Ordering::SeqCst);
        if self.deny {
            anyhow::bail!("Refused: the user cancelled the apply in quadcam.")
        }
        Ok(())
    }
    fn gear_changed(&self) {
        self.changed.fetch_add(1, Ordering::SeqCst);
    }
}

struct Bench {
    _dir: tempfile::TempDir,
    core: Arc<Core>,
    fetch: Arc<FixtureFetch>,
    flasher: Arc<FlashRecorder>,
    dfu: Arc<Mutex<Vec<DfuInfo>>>,
    old: Vec<u8>,
    new: Vec<u8>,
    radio: String,
}

fn bench(hooks: Arc<dyn Hooks>, board: &str, version: &str, backed_up: bool) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let old = full_image(3);
    let new = full_image(5);
    let zip = release_zip(dir.path(), &new);
    let fetch = Arc::new(FixtureFetch::new());
    serve_release(&fetch, &zip);
    serve_indexes(&fetch);
    let flasher = Arc::new(FlashRecorder::new(FakeDfu::with_firmware(&old)));
    let dfu = Arc::new(Mutex::new(vec![DfuInfo {
        vid: 0x0483,
        pid: 0xdf11,
        serial: Some("0001".into()),
    }]));
    let d2 = dfu.clone();
    let mut env = quadcam_lib::gear::Env::fake(
        vec![],
        Arc::new(quadcam_lib::gear::serial::FakePorts::new(vec![])),
    );
    env.dfu = Arc::new(move || d2.lock().unwrap().clone());
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
            fetch: fetch.clone(),
            flasher: flasher.clone(),
        }),
    );
    let radio = Device {
        id: "radio-0000000000000001".into(),
        kind: DeviceKind::Radio,
        name: "Pocket".into(),
        aircraft: None,
        identity: Identity {
            board: Some(board.into()),
            firmware: Some("EdgeTX".into()),
            version: Some(version.into()),
            ..Identity::default()
        },
        last_seen: None,
        last_backup: backed_up.then(|| "2026-10-01T100000-manual".to_string()),
        last_space: None,
        aliases: Vec::new(),
        dfu_serial: None,
    };
    core.gear_store().save_device(&radio).unwrap();
    Bench {
        _dir: dir,
        core,
        fetch,
        flasher,
        dfu,
        old,
        new,
        radio: radio.id,
    }
}

fn plain() -> Bench {
    bench(Arc::new(NoHooks), "pocket", VERSION, true)
}

fn picture(dir: &Path) -> SplashParams {
    let mut px = vec![255u8; 128 * 64];
    for i in 0..64 {
        px[i * 128 + i] = 0;
        px[i * 128 + 127 - i] = 0;
    }
    let f = dir.join("splash.png");
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, 128, 64);
    enc.set_color(png::ColorType::Grayscale);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&px).unwrap();
    std::fs::write(&f, out).unwrap();
    SplashParams {
        image: f,
        threshold: None,
        invert: false,
        board: Some("pocket".into()),
    }
}

fn params(b: &Bench, splash: Option<SplashParams>) -> FlashParams {
    FlashParams {
        device: b.radio.clone(),
        version: None,
        splash,
    }
}

fn request(p: &FlashParams, plan: &ApplyPlan) -> FlashRequest {
    FlashRequest {
        params: p.clone(),
        digest: plan.digest.clone(),
        confirm: true,
    }
}

fn refusal(e: anyhow::Error) -> Refusal {
    e.downcast::<Refusal>().expect("a refusal")
}

fn failed(plan: &ApplyPlan) -> Vec<(&str, RefusalCode)> {
    plan.checks
        .iter()
        .filter_map(|c| c.refusal.as_ref().map(|r| (c.name.as_str(), r.code)))
        .collect()
}

#[test]
fn the_check_reads_the_network_only_when_asked_and_remembers_the_answer() {
    let b = plain();
    // Manual by default: nothing is fetched, nothing is known.
    let v = b.core.gear_firmware(&FirmwareParams::default()).unwrap();
    assert!(b.fetch.requested.lock().unwrap().is_empty());
    assert_eq!(v.mode, "manual");
    assert_eq!(v.devices[0].state, FirmwareState::Unknown);
    // Asked: three sources, EdgeTX 2.12.4 matches the radio.
    let v = b
        .core
        .gear_firmware(&FirmwareParams { check: Some(true) })
        .unwrap();
    assert_eq!(b.fetch.requested.lock().unwrap().len(), 3);
    assert_eq!(v.latest.edgetx.as_deref(), Some("2.12.4"));
    assert_eq!(v.devices[0].state, FirmwareState::UpToDate);
    assert_eq!(v.devices[0].installed.as_deref(), Some("2.12.4"));
    // Looking again reads the saved answer.
    let v = b
        .core
        .gear_firmware(&FirmwareParams { check: Some(false) })
        .unwrap();
    assert_eq!(b.fetch.requested.lock().unwrap().len(), 3);
    assert_eq!(v.latest.betaflight.as_deref(), Some("2026.6.0"));
    // `daily` checks on its own, once.
    b.core
        .settings_set(&serde_json::from_value(json!({"firmware_check": "daily"})).unwrap())
        .unwrap();
    let before = b.fetch.requested.lock().unwrap().len();
    b.core.gear_firmware(&FirmwareParams::default()).unwrap();
    assert_eq!(
        b.fetch.requested.lock().unwrap().len(),
        before,
        "answer is fresh"
    );
}

#[test]
fn a_plan_with_a_splash_passes_every_check_and_names_the_image() {
    let b = plain();
    let d = tempfile::tempdir().unwrap();
    let p = params(&b, Some(picture(d.path())));
    let plan = b.core.gear_flash_plan(&p).unwrap();
    assert!(plan.ready(), "{:?}", failed(&plan));
    assert_eq!(plan.digest.len(), 16);
    let names: Vec<&str> = plan.checks.iter().map(|c| c.name.as_str()).collect();
    for want in [
        "Known board and version",
        "Splash layout",
        "Firmware image",
        "Splash markers",
        "Card backup",
        "One radio in DFU mode",
    ] {
        assert!(names.contains(&want), "{want} in {names:?}");
    }
    let text = serde_json::to_string(&plan.diff).unwrap();
    assert!(text.contains("pocket-def35ad.bin"), "{text}");
    assert!(text.contains("Splash:"), "{text}");
    // The same params give the same digest; no splash gives another.
    assert_eq!(b.core.gear_flash_plan(&p).unwrap().digest, plan.digest);
    assert_ne!(
        b.core.gear_flash_plan(&params(&b, None)).unwrap().digest,
        plan.digest
    );
    assert!(plan
        .warnings
        .iter()
        .any(|w| w.contains("not been tried on a real radio")));
    // Planning flashes nothing.
    assert!(b.flasher.opened.lock().unwrap().is_empty());
    assert!(!b.flasher.device.lock().unwrap().left);
}

#[test]
fn a_version_or_board_not_in_compat_refuses_without_a_download() {
    let b = plain();
    let plan = b
        .core
        .gear_flash_plan(&FlashParams {
            device: b.radio.clone(),
            version: Some("2.11.3".into()),
            splash: None,
        })
        .unwrap();
    assert!(!plan.ready());
    assert_eq!(
        failed(&plan)[0],
        ("Known board and version", RefusalCode::UnknownVersion)
    );
    assert!(plan.digest.is_empty());
    let other = bench(Arc::new(NoHooks), "tx16s", VERSION, true);
    let plan = other.core.gear_flash_plan(&params(&other, None)).unwrap();
    assert_eq!(
        failed(&plan)[0],
        ("Known board and version", RefusalCode::UnknownBoard)
    );
    // Neither asked the network for a release.
    for bench in [&b, &other] {
        assert!(bench.fetch.requested.lock().unwrap().is_empty());
    }
    // A colour radio's splash is not supported, whatever the version.
    let d = tempfile::tempdir().unwrap();
    let mut s = picture(d.path());
    s.board = Some("tx16s".into());
    let plan = other
        .core
        .gear_flash_plan(&params(&other, Some(s)))
        .unwrap();
    assert!(failed(&plan).iter().any(|(n, _)| *n == "Splash layout"));
    // The splash pair is narrower than the firmware pair: 2.12.3 is not a proven splash.
    let d = tempfile::tempdir().unwrap();
    let plan = b
        .core
        .gear_flash_plan(&FlashParams {
            device: b.radio.clone(),
            version: Some("2.12.3".into()),
            splash: Some(picture(d.path())),
        })
        .unwrap();
    assert!(
        failed(&plan)
            .iter()
            .any(|(n, c)| *n == "Splash layout" && *c == RefusalCode::UnknownVersion),
        "{:?}",
        failed(&plan)
    );
}

#[test]
fn missing_and_doubled_markers_refuse_in_the_plan() {
    let d = tempfile::tempdir().unwrap();
    for (what, mutate) in [
        (
            "missing",
            Box::new(|b: &mut Vec<u8>| b[0x20000] = b'X') as Box<dyn Fn(&mut Vec<u8>)>,
        ),
        (
            "doubled",
            Box::new(|b: &mut Vec<u8>| {
                let at = 0x80000;
                b[at..at + 4].copy_from_slice(splash::START);
            }),
        ),
    ] {
        let mut img = full_image(5);
        mutate(&mut img);
        let b = bench(Arc::new(NoHooks), "pocket", VERSION, true);
        let zip = release_zip(d.path(), &img);
        serve_release(&b.fetch, &zip);
        let plan = b
            .core
            .gear_flash_plan(&params(&b, Some(picture(d.path()))))
            .unwrap();
        let f = failed(&plan);
        assert!(
            f.contains(&("Splash markers", RefusalCode::BadImage)),
            "{what}: {f:?}"
        );
        assert!(plan.digest.is_empty());
    }
}

#[test]
fn no_backup_no_dfu_or_two_dfu_devices_refuse() {
    let b = bench(Arc::new(NoHooks), "pocket", VERSION, false);
    let plan = b.core.gear_flash_plan(&params(&b, None)).unwrap();
    assert!(failed(&plan).contains(&("Card backup", RefusalCode::NoBackup)));
    let b = plain();
    b.dfu.lock().unwrap().clear();
    let plan = b.core.gear_flash_plan(&params(&b, None)).unwrap();
    assert_eq!(
        failed(&plan),
        [("One radio in DFU mode", RefusalCode::NoDevice)]
    );
    let one = DfuInfo {
        vid: 0x0483,
        pid: 0xdf11,
        serial: None,
    };
    b.dfu.lock().unwrap().extend([one.clone(), one]);
    let plan = b.core.gear_flash_plan(&params(&b, None)).unwrap();
    assert_eq!(
        failed(&plan),
        [("One radio in DFU mode", RefusalCode::SeveralDevices)]
    );
    // A DFU device that is not the STM32 bootloader does not count.
    b.dfu.lock().unwrap().clear();
    b.dfu.lock().unwrap().push(DfuInfo {
        vid: 0x1209,
        pid: 0x0001,
        serial: None,
    });
    let plan = b.core.gear_flash_plan(&params(&b, None)).unwrap();
    assert_eq!(
        failed(&plan),
        [("One radio in DFU mode", RefusalCode::NoDevice)]
    );
    let e = b
        .core
        .gear_flash(&FlashRequest {
            params: params(&b, None),
            digest: "x".into(),
            confirm: true,
        })
        .unwrap_err();
    assert_eq!(refusal(e).code, RefusalCode::NoDevice);
}

#[test]
fn only_a_radio_can_be_flashed() {
    let b = plain();
    let fc = Device {
        id: "fc-1".into(),
        kind: DeviceKind::Fc,
        name: String::new(),
        aircraft: None,
        identity: Identity::default(),
        last_seen: None,
        last_backup: None,
        last_space: None,
        aliases: Vec::new(),
        dfu_serial: None,
    };
    b.core.gear_store().save_device(&fc).unwrap();
    let e = b
        .core
        .gear_flash_plan(&FlashParams {
            device: "fc-1".into(),
            ..FlashParams::default()
        })
        .unwrap_err();
    assert_eq!(refusal(e).code, RefusalCode::Incompatible);
    assert!(b
        .core
        .gear_flash_plan(&FlashParams {
            device: "nope".into(),
            ..FlashParams::default()
        })
        .is_err());
}

#[test]
fn apply_needs_confirm_and_the_plans_digest() {
    let b = plain();
    let p = params(&b, None);
    let plan = b.core.gear_flash_plan(&p).unwrap();
    let mut req = request(&p, &plan);
    req.confirm = false;
    assert!(b
        .core
        .gear_flash(&req)
        .unwrap_err()
        .to_string()
        .contains("confirm=true"));
    let mut req = request(&p, &plan);
    req.digest = "0000000000000000".into();
    let e = b.core.gear_flash(&req).unwrap_err();
    assert_eq!(refusal(e).code, RefusalCode::BeforeMismatch);
    assert!(
        b.flasher.opened.lock().unwrap().is_empty(),
        "nothing opened a USB device"
    );
    assert_eq!(
        b.flasher.device.lock().unwrap().flash[..b.old.len()],
        b.old[..]
    );
}

#[test]
fn a_flash_backs_up_writes_reads_back_and_leaves() {
    let gui = Arc::new(Gui::default());
    let b = bench(gui.clone(), "pocket", VERSION, true);
    let d = tempfile::tempdir().unwrap();
    let sp = picture(d.path());
    let p = params(&b, Some(sp.clone()));
    let plan = b.core.gear_flash_plan(&p).unwrap();
    assert!(plan.ready(), "{:?}", failed(&plan));
    let report = b.core.gear_flash(&request(&p, &plan)).unwrap();
    assert_eq!(report.status, ChangeStatus::Verified, "{}", report.message);
    assert!(report.saved);
    assert_eq!(
        gui.asked.load(Ordering::SeqCst),
        1,
        "the person was asked once"
    );
    let names: Vec<&str> = report.steps.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Copy the current firmware",
            "Erase",
            "Write",
            "Read back",
            "Leave DFU"
        ]
    );
    assert!(report
        .steps
        .iter()
        .all(|s| s.state == quadcam_lib::gear::apply::StepState::Done));

    // The radio holds the patched image, and the splash decodes to the picture.
    let dev = b.flasher.device.lock().unwrap();
    assert!(dev.left, "the radio left DFU");
    let flashed = &dev.flash[..b.new.len()];
    let (mono, _) = splash::preview(&sp).unwrap();
    assert_eq!(splash::decode(flashed).unwrap(), mono);
    // Only the 1,024 splash bytes differ from the release's binary.
    let at = splash::locate(&b.new).unwrap();
    assert_eq!(&flashed[..at], &b.new[..at]);
    assert_eq!(&flashed[at + 1024..], &b.new[at + 1024..]);
    assert_eq!(b.flasher.opened.lock().unwrap().as_slice(), ["0483:df11"]);

    // The firmware it ran before is kept as a firmware copy, not as a card backup: a
    // snapshot of only firmware.bin would become the radio's latest card backup.
    assert!(b
        .core
        .gear_backups(&BackupFilter {
            device: Some(b.radio.clone()),
        })
        .unwrap()
        .is_empty());
    let copies = quadcam_lib::gear::fwcopy::list(&b.core.gear_store(), &b.radio);
    assert_eq!(copies.len(), 1);
    assert_eq!(report.backup.as_deref(), Some(copies[0].id.as_str()));
    assert_eq!(
        copies[0].kind,
        quadcam_lib::gear::fwcopy::CopyKind::BeforeFlash
    );
    assert_eq!(copies[0].image_version.as_deref(), Some("2.12.4"));
    let kept = quadcam_lib::gear::fwcopy::read(&b.core.gear_store(), &copies[0]).unwrap();
    assert_eq!(
        &kept[..],
        &b.old[..kept.len()],
        "the old firmware, as it was"
    );
    assert!(gui.changed.load(Ordering::SeqCst) >= 1);
    // Nothing logs the picture's path or the radio's name in the report's notes.
    assert!(!report.message.contains("splash.png"));
    let _ = FLASH_BASE;
}

#[test]
fn the_apply_sheets_click_is_the_confirm_and_a_denied_request_writes_nothing() {
    let gui = Arc::new(Gui {
        deny: true,
        ..Gui::default()
    });
    let b = bench(gui.clone(), "pocket", VERSION, true);
    let p = params(&b, None);
    let plan = b.core.gear_flash_plan(&p).unwrap();
    let e = b.core.gear_flash(&request(&p, &plan)).unwrap_err();
    assert!(e.to_string().contains("cancelled"), "{e}");
    assert!(b.flasher.opened.lock().unwrap().is_empty());
    assert!(!b.flasher.device.lock().unwrap().left);
    assert!(b
        .core
        .gear_backups(&BackupFilter::default())
        .unwrap()
        .is_empty());
    // The sheet's own Apply does not ask again.
    let r = b.core.gear_flash_click(&request(&p, &plan)).unwrap();
    assert_eq!(r.status, ChangeStatus::Verified);
    assert_eq!(gui.asked.load(Ordering::SeqCst), 1);
}

#[test]
fn a_bad_read_back_keeps_the_radio_in_dfu_and_the_backup() {
    let b = plain();
    b.flasher.device.lock().unwrap().faults.drop_write_block = Some(40);
    let p = params(&b, None);
    let plan = b.core.gear_flash_plan(&p).unwrap();
    let r = b.core.gear_flash(&request(&p, &plan)).unwrap();
    assert_eq!(r.status, ChangeStatus::Failed);
    assert!(!r.saved);
    assert!(r.message.contains("stays in DFU mode"), "{}", r.message);
    assert!(r.message.contains("different bytes"), "{}", r.message);
    assert!(r.message.contains("ROM bootloader"), "{}", r.message);
    let states: Vec<_> = r.steps.iter().map(|s| (s.name.as_str(), s.state)).collect();
    use quadcam_lib::gear::apply::StepState::{Done, Failed, Skipped};
    assert_eq!(
        states,
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
    assert!(r.backup.is_some());
}

#[test]
fn a_failed_erase_stops_and_marks_the_step() {
    let b = plain();
    b.flasher.device.lock().unwrap().faults.fail_erase = true;
    let p = params(&b, None);
    let plan = b.core.gear_flash_plan(&p).unwrap();
    let r = b.core.gear_flash(&request(&p, &plan)).unwrap();
    assert_eq!(r.status, ChangeStatus::Failed);
    use quadcam_lib::gear::apply::StepState::{Failed, Skipped};
    assert_eq!(r.steps[1].state, Failed, "{:?}", r.steps);
    assert_eq!(r.steps[1].name, "Erase");
    assert_eq!(r.steps[4].state, Skipped);
    assert!(!b.flasher.device.lock().unwrap().left);
}

#[test]
fn a_cargo_process_never_gets_the_real_usb_path() {
    use quadcam_lib::gear::firmware::flash_enabled;
    assert!(!flash_enabled(None, true));
    assert!(flash_enabled(None, false));
    assert!(flash_enabled(Some("real"), true));
    assert!(!flash_enabled(Some("yes"), false));
    if std::env::var("QUADCAM_FLASH").as_deref() != Ok("real") {
        // The default flasher in this process opens a fake whose flash is empty.
        let f = quadcam_lib::gear::firmware::system_flasher();
        let mut usb = f.open_dfu(0x0483, 0xdf11, None).unwrap();
        assert!(quadcam_lib::gear::dfu::flash_size(usb.as_mut(), true).unwrap() > 0);
    }
}

// ----- the read-only DFU trial -----

#[test]
fn the_read_only_trial_saves_a_copy_and_compares_the_version_and_changes_nothing() {
    let b = plain();
    let r = b
        .core
        .gear_firmware_read(&FirmwareReadParams {
            device: Some(b.radio.clone()),
        })
        .unwrap();
    assert_eq!(r.matches, Some(true), "{}", r.message);
    assert_eq!(r.known_version.as_deref(), Some("2.12.4"));
    assert_eq!(r.copy.image_version.as_deref(), Some("2.12.4"));
    assert_eq!(r.copy.image_board.as_deref(), Some("pocket"));
    assert_eq!(r.flash_bytes, 1024 * 1024);
    let kept = quadcam_lib::gear::fwcopy::read(&b.core.gear_store(), &r.copy).unwrap();
    assert_eq!(&kept[..], &b.old[..kept.len()]);
    assert_eq!(kept.len(), b.old.len().min(kept.len()));
    // The radio saw status polls, set-address commands and uploads: no erase, no data block,
    // no leave.
    let dev = b.flasher.device.lock().unwrap();
    assert_eq!(dev.mutations, 0, "{:?}", dev.log);
    assert!(!dev.left);
    assert!(!dev
        .log
        .iter()
        .any(|l| l.starts_with("erase") || l.starts_with("write") || l.starts_with("leave")));
    assert_eq!(&dev.flash[..b.old.len()], &b.old[..]);
    // It is not a card backup.
    drop(dev);
    assert!(b
        .core
        .gear_backups(&BackupFilter::default())
        .unwrap()
        .is_empty());
    assert_eq!(r.steps.len(), 3);
}

#[test]
fn the_trial_says_when_the_image_names_another_version() {
    let b = bench(Arc::new(NoHooks), "pocket", "2.11.3", true);
    let r = b
        .core
        .gear_firmware_read(&FirmwareReadParams {
            device: Some(b.radio.clone()),
        })
        .unwrap();
    assert_eq!(r.matches, Some(false));
    assert!(
        r.message.contains("2.12.4") && r.message.contains("2.11.3"),
        "{}",
        r.message
    );
    // With no radio named, nothing is compared, and the copy sits under the DFU serial.
    let r = b
        .core
        .gear_firmware_read(&FirmwareReadParams::default())
        .unwrap();
    assert_eq!(r.matches, None);
    assert!(r.copy.id.starts_with("dfu-0001/"), "{}", r.copy.id);
}

#[test]
fn the_trial_refuses_without_one_dfu_device_and_on_a_blank_or_flaky_read() {
    let b = plain();
    b.dfu.lock().unwrap().clear();
    let e = b
        .core
        .gear_firmware_read(&FirmwareReadParams::default())
        .unwrap_err();
    let r = refusal(e);
    assert_eq!(r.code, RefusalCode::NoDevice);
    assert!(r.reason.contains("do not hold the trim"), "{}", r.reason);
    assert!(b.flasher.opened.lock().unwrap().is_empty());
    let one = DfuInfo {
        vid: 0x0483,
        pid: 0xdf11,
        serial: None,
    };
    b.dfu.lock().unwrap().extend([one.clone(), one]);
    let e = b
        .core
        .gear_firmware_read(&FirmwareReadParams::default())
        .unwrap_err();
    assert_eq!(refusal(e).code, RefusalCode::SeveralDevices);
    // A blank flash has nothing to copy.
    let blank = plain();
    blank.flasher.device.lock().unwrap().flash.fill(0xFF);
    let e = blank
        .core
        .gear_firmware_read(&FirmwareReadParams::default())
        .unwrap_err();
    assert!(refusal(e).reason.contains("blank"));
    // A link that goes bad part way: the two reads differ and nothing is saved.
    let flaky = plain();
    flaky.flasher.device.lock().unwrap().faults.flip_after_reads = Some(600);
    let e = flaky
        .core
        .gear_firmware_read(&FirmwareReadParams::default())
        .unwrap_err();
    assert!(refusal(e).reason.contains("differ"));
    assert!(quadcam_lib::gear::fwcopy::list(&flaky.core.gear_store(), "dfu-0001").is_empty());
    assert_eq!(flaky.flasher.device.lock().unwrap().mutations, 0);
}

#[test]
fn a_flash_never_erases_when_its_copy_cannot_be_made() {
    // The first read is clean, the second differs: no verified copy, so no erase.
    let b = plain();
    b.flasher.device.lock().unwrap().faults.flip_after_reads = Some(600);
    let p = params(&b, None);
    let plan = b.core.gear_flash_plan(&p).unwrap();
    let e = b.core.gear_flash(&request(&p, &plan)).unwrap_err();
    let r = refusal(e);
    assert_eq!(r.code, RefusalCode::NoBackup);
    assert!(r.reason.contains("Nothing was written"), "{}", r.reason);
    assert_eq!(b.flasher.device.lock().unwrap().mutations, 0);
    assert!(quadcam_lib::gear::fwcopy::list(&b.core.gear_store(), &b.radio).is_empty());
}

#[test]
fn a_blank_radio_can_be_flashed_and_a_wrong_sized_chip_cannot() {
    let b = plain();
    b.flasher.device.lock().unwrap().flash.fill(0xFF);
    let p = params(&b, None);
    let plan = b.core.gear_flash_plan(&p).unwrap();
    let r = b.core.gear_flash(&request(&p, &plan)).unwrap();
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    assert!(r.backup.is_none(), "there was nothing to keep");
    assert!(r.steps[0].detail.as_deref().unwrap().contains("blank"));
    // A 512 KB part is not the Pocket's chip.
    let small = plain();
    {
        let mut d = small.flasher.device.lock().unwrap();
        *d = FakeDfu::with_firmware(&small.old);
        d.layout = "@Internal Flash  /0x08000000/04*016Kg,01*064Kg,03*128Kg".into();
        d.flash.truncate(512 * 1024);
    }
    let p = params(&small, None);
    let plan = small.core.gear_flash_plan(&p).unwrap();
    let e = small.core.gear_flash(&request(&p, &plan)).unwrap_err();
    assert_eq!(refusal(e).code, RefusalCode::Incompatible);
    assert_eq!(small.flasher.device.lock().unwrap().mutations, 0);
}

// ----- MCP -----

fn call(
    s: &mut quadcam_lib::mcp::Server<quadcam_lib::mcp::LocalBackend>,
    tool: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    s.call_tool(tool, args)
}

#[test]
fn the_mcp_actions_check_preview_plan_and_flash_with_a_digest() {
    let b = plain();
    let d = tempfile::tempdir().unwrap();
    let sp = picture(d.path());
    let image = sp.image.to_str().unwrap().to_string();
    let mut s = quadcam_lib::mcp::Server::new(quadcam_lib::mcp::LocalBackend(b.core.clone()));

    let r = call(&mut s, "quadcam_gear", json!({"action": "firmware_check"}));
    assert_eq!(r["isError"], false, "{r}");
    let t = r["content"][0]["text"].as_str().unwrap();
    assert!(
        t.contains("Pocket (EdgeTX, pocket): 2.12.4 installed, 2.12.4 newest, up to date"),
        "{t}"
    );
    let r = call(
        &mut s,
        "quadcam_gear",
        json!({"action": "firmware_check", "check": false}),
    );
    assert_eq!(r["isError"], false);

    let r = call(
        &mut s,
        "quadcam_gear",
        json!({"action": "splash", "image": image, "board": "pocket"}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("dark pixels of 8192"));
    assert!(
        r["structuredContent"]["png_base64"].is_null(),
        "the picture is not sent as text"
    );
    let r = call(
        &mut s,
        "quadcam_gear",
        json!({"action": "splash", "image": image, "board": "tx16s"}),
    );
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("not supported yet"));
    let r = call(&mut s, "quadcam_gear", json!({"action": "splash"}));
    assert_eq!(r["isError"], true);

    let r = call(
        &mut s,
        "quadcam_gear",
        json!({"action": "flash_plan", "device": b.radio, "image": image}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let t = r["content"][0]["text"].as_str().unwrap();
    assert!(t.contains("ok Known board and version"), "{t}");
    assert!(t.contains("EdgeTX firmware"), "{t}");
    assert!(t.contains("2.12.4 -> 2.12.4"), "{t}");
    let digest = r["structuredContent"]["digest"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(t.contains(&format!("digest={digest}")), "{t}");

    // Flash: refused without confirm, then with the digest it flashes the fake.
    let r = call(
        &mut s,
        "quadcam_gear_apply",
        json!({"action": "flash", "device": b.radio, "image": image, "digest": digest}),
    );
    assert_eq!(r["isError"], true);
    assert!(b.flasher.opened.lock().unwrap().is_empty());
    let r = call(
        &mut s,
        "quadcam_gear_apply",
        json!({"action": "flash", "device": b.radio, "image": image, "digest": digest, "confirm": true}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let t = r["content"][0]["text"].as_str().unwrap();
    assert!(t.starts_with("Verified:"), "{t}");
    assert!(t.contains("Copy the current firmware: done"), "{t}");
    assert!(b.flasher.device.lock().unwrap().left);
}

// ----- CLI -----

#[test]
fn the_cli_previews_a_splash_and_plans_in_a_temporary_home() {
    let home = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    let sp = picture(d.path());
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
    let preview = d.path().join("preview.png");
    let (code, v, text) = run(&[
        "gear",
        "splash",
        sp.image.to_str().unwrap(),
        "--board",
        "pocket",
        "--out",
        preview.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(v["result"]["supported"], true);
    assert_eq!(v["result"]["png_base64"], "");
    let g = splash::decode_png(&std::fs::read(&preview).unwrap()).unwrap();
    assert_eq!((g.width, g.height), (512, 256));
    let (_, v, _) = run(&[
        "gear",
        "splash",
        sp.image.to_str().unwrap(),
        "--board",
        "tx16s",
    ]);
    assert_eq!(v["result"]["supported"], false);
    let (code, _, _) = run(&["gear", "splash", "/nonexistent.png"]);
    assert_ne!(code, 0);

    // No devices: an empty table, offline checks report instead of failing.
    let (code, v, text) = run(&["gear", "firmware"]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(v["result"]["devices"].as_array().unwrap().len(), 0);
    let (code, v, text) = run(&["gear", "firmware", "--check"]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(
        v["result"]["latest"]["errors"].as_array().unwrap().len(),
        3,
        "{text}"
    );

    // A flash needs a device, a digest and --yes.
    let (code, _, _) = run(&["gear", "firmware", "--plan"]);
    assert_ne!(code, 0);
    let (code, _, text) = run(&["gear", "firmware", "--device", "radio-1", "--digest", "x"]);
    assert_ne!(code, 0, "{text}");
    assert!(text.contains("--yes"), "{text}");

    // A saved radio on an unproven version: the plan refuses, exit 0, no download.
    let gear = home
        .path()
        .join("Library/Application Support/app.quadcam/gear");
    std::fs::create_dir_all(&gear).unwrap();
    let radio = Device {
        id: "radio-0000000000000002".into(),
        kind: DeviceKind::Radio,
        name: String::new(),
        aircraft: None,
        identity: Identity {
            board: Some("pocket".into()),
            version: Some("2.11.3".into()),
            ..Identity::default()
        },
        last_seen: None,
        last_backup: None,
        last_space: None,
        aliases: Vec::new(),
        dfu_serial: None,
    };
    std::fs::write(
        gear.join("gear.json"),
        json!({"devices": [radio]}).to_string(),
    )
    .unwrap();
    let (code, v, text) = run(&[
        "gear",
        "firmware",
        "--plan",
        "--device",
        "radio-0000000000000002",
    ]);
    assert_eq!(code, 0, "{text}");
    let checks = v["result"]["checks"].as_array().unwrap();
    assert_eq!(checks[0]["refusal"]["code"], "unknown_version", "{text}");
    assert_eq!(v["result"]["digest"], "");
    // Even with a made-up digest the flash is refused (exit 3) and opens no USB device.
    let (code, _, text) = run(&[
        "gear",
        "firmware",
        "--device",
        "radio-0000000000000002",
        "--digest",
        "x",
        "--yes",
    ]);
    assert_eq!(code, 3, "{text}");
}

// --- Linking a DFU device to its saved radio -----------------------------------------

fn second_radio(b: &Bench, seen: bool) -> Device {
    let d = Device {
        id: "radio-0000000000000002".into(),
        kind: DeviceKind::Radio,
        name: "Spare".into(),
        aircraft: None,
        identity: Identity {
            board: Some("pocket".into()),
            ..Identity::default()
        },
        last_seen: seen.then(chrono::Utc::now),
        last_backup: Some("2026-10-01T100000-manual".into()),
        last_space: None,
        aliases: Vec::new(),
        dfu_serial: None,
    };
    b.core.gear_store().save_device(&d).unwrap();
    d
}

#[test]
fn a_dfu_device_links_to_the_radio_seen_last_or_the_one_picked() {
    use quadcam_lib::core::DfuLinkParams;
    let b = plain();
    // One saved radio: it is the last seen.
    let r = b.core.gear_dfu_link(&DfuLinkParams::default()).unwrap();
    assert_eq!(r.how, "last_seen");
    assert_eq!(r.device.id, b.radio);
    assert_eq!(r.serial.as_deref(), Some("0001"));

    // The status shows the DFU device as that radio.
    let c = b.core.gear_connected().unwrap();
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].id.as_deref(), Some(b.radio.as_str()));
    assert_eq!(c[0].device.as_ref().unwrap().name, "Pocket");

    // Two radios: the most recently seen is taken unless one is picked. A serial belongs
    // to one radio, so the pick moves it.
    let spare = second_radio(&b, true);
    let r = b.core.gear_dfu_link(&DfuLinkParams::default()).unwrap();
    assert_eq!(
        (r.how.as_str(), r.device.id.as_str()),
        ("last_seen", spare.id.as_str())
    );
    assert!(
        r.notes
            .iter()
            .any(|n| n.contains("Moved the link from Pocket")),
        "{:?}",
        r.notes
    );
    let again = b
        .core
        .gear_dfu_link(&DfuLinkParams {
            device: Some(b.radio.clone()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(again.how, "picked");
    let devices = b.core.gear_devices().unwrap();
    assert_eq!(
        devices
            .iter()
            .filter(|d| d.dfu_serial.is_some())
            .map(|d| d.id.as_str())
            .collect::<Vec<_>>(),
        [b.radio.as_str()]
    );

    // Unlink.
    let u = b
        .core
        .gear_dfu_link(&DfuLinkParams {
            device: Some(b.radio.clone()),
            unlink: true,
            ..Default::default()
        })
        .unwrap();
    assert!(u.serial.is_none() && u.device.dfu_serial.is_none());
    // No radio in DFU mode: nothing to link.
    b.dfu.lock().unwrap().clear();
    assert!(b.core.gear_dfu_link(&DfuLinkParams::default()).is_err());
}

#[test]
fn the_flash_plan_refuses_a_dfu_device_linked_to_another_radio() {
    use quadcam_lib::core::DfuLinkParams;
    let b = plain();
    let spare = second_radio(&b, true);
    b.core
        .gear_dfu_link(&DfuLinkParams {
            device: Some(spare.id.clone()),
            ..Default::default()
        })
        .unwrap();
    let plan = b.core.gear_flash_plan(&params(&b, None)).unwrap();
    assert!(
        failed(&plan).contains(&(
            "The radio in DFU mode is this radio",
            RefusalCode::DeviceChanged
        )),
        "{:?}",
        failed(&plan)
    );
    // Linked to the picked radio: passes.
    b.core
        .gear_dfu_link(&DfuLinkParams {
            device: Some(b.radio.clone()),
            ..Default::default()
        })
        .unwrap();
    let plan = b.core.gear_flash_plan(&params(&b, None)).unwrap();
    assert!(plan.ready(), "{:?}", failed(&plan));
    // A radio linked to another DFU device than the one plugged in refuses too.
    b.dfu.lock().unwrap()[0].serial = Some("0002".into());
    let plan = b.core.gear_flash_plan(&params(&b, None)).unwrap();
    assert!(failed(&plan).contains(&(
        "The radio in DFU mode is this radio",
        RefusalCode::DeviceChanged
    )));
}

#[test]
fn an_unlinked_dfu_device_warns_with_the_last_seen_radio_and_a_verified_flash_links_it() {
    let b = plain();
    let _spare = second_radio(&b, true);
    let p = params(&b, None);
    let plan = b.core.gear_flash_plan(&p).unwrap();
    assert!(plan.ready(), "{:?}", failed(&plan));
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.contains("Spare was seen most recently, not Pocket")),
        "{:?}",
        plan.warnings
    );
    let report = b.core.gear_flash(&request(&p, &plan)).unwrap();
    assert_eq!(report.status, ChangeStatus::Verified, "{}", report.message);
    let pocket = b.core.gear_store().device(&b.radio).unwrap().unwrap();
    assert_eq!(pocket.dfu_serial.as_deref(), Some("0001"));
}
