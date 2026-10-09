//! Firmware (WP10): the check, the splash preview, and the EdgeTX flash plan and apply. The
//! network is a fixture server in memory and the radio is a fake DFU device; nothing here
//! downloads, opens a USB device or flashes anything real.

use quadcam_lib::core::{
    BackupFilter, Core, FirmwareParams, FlashParams, FlashRequest, Hooks, NoHooks,
};
use quadcam_lib::gear::detect::DfuInfo;
use quadcam_lib::gear::dfu::{FakeDfu, FLASH_BASE};
use quadcam_lib::gear::firmware::check::{
    BETAFLIGHT_RELEASES, EDGETX_RELEASES, ELRS_INDEX, FirmwareState,
};
use quadcam_lib::gear::firmware::edgetx::{asset_name, release_url, DOWNLOAD_PREFIX};
use quadcam_lib::gear::firmware::{FixtureFetch, FwEnv, Recorder as FlashRecorder};
use quadcam_lib::gear::model::{
    ApplyPlan, ChangeStatus, Device, DeviceKind, Identity, RefusalCode, Refusal,
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
    std::fs::write(src.join(format!("fw-radiomaster-pocket-v{VERSION}.bin")), pocket).unwrap();
    std::fs::write(src.join(format!("fw-tx16s-v{VERSION}.bin")), full_image(9)).unwrap();
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
    f.serve(EDGETX_RELEASES, serde_json::to_vec(&rel("v2.12.4")).unwrap());
    f.serve(BETAFLIGHT_RELEASES, serde_json::to_vec(&rel("2026.6.0")).unwrap());
    f.serve(ELRS_INDEX, json!({"tags": {"3.5.3": "x"}}).to_string().into_bytes());
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
        Core::new(dir.path().join("cache"), None, hooks, Arc::new(Recorder::default()))
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
    let v = b.core.gear_firmware(&FirmwareParams { check: Some(true) }).unwrap();
    assert_eq!(b.fetch.requested.lock().unwrap().len(), 3);
    assert_eq!(v.latest.edgetx.as_deref(), Some("2.12.4"));
    assert_eq!(v.devices[0].state, FirmwareState::UpToDate);
    assert_eq!(v.devices[0].installed.as_deref(), Some("2.12.4"));
    // Looking again reads the saved answer.
    let v = b.core.gear_firmware(&FirmwareParams { check: Some(false) }).unwrap();
    assert_eq!(b.fetch.requested.lock().unwrap().len(), 3);
    assert_eq!(v.latest.betaflight.as_deref(), Some("2026.6.0"));
    // `daily` checks on its own, once.
    b.core
        .settings_set(&serde_json::from_value(json!({"firmware_check": "daily"})).unwrap())
        .unwrap();
    let before = b.fetch.requested.lock().unwrap().len();
    b.core.gear_firmware(&FirmwareParams::default()).unwrap();
    assert_eq!(b.fetch.requested.lock().unwrap().len(), before, "answer is fresh");
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
    assert!(text.contains("fw-radiomaster-pocket-v2.12.4.bin"), "{text}");
    assert!(text.contains("Splash:"), "{text}");
    // The same params give the same digest; no splash gives another.
    assert_eq!(b.core.gear_flash_plan(&p).unwrap().digest, plan.digest);
    assert_ne!(
        b.core.gear_flash_plan(&params(&b, None)).unwrap().digest,
        plan.digest
    );
    assert!(plan.warnings.iter().any(|w| w.contains("not been tried on a real radio")));
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
    assert_eq!(failed(&plan)[0], ("Known board and version", RefusalCode::UnknownVersion));
    assert!(plan.digest.is_empty());
    let other = bench(Arc::new(NoHooks), "tx16s", VERSION, true);
    let plan = other.core.gear_flash_plan(&params(&other, None)).unwrap();
    assert_eq!(failed(&plan)[0], ("Known board and version", RefusalCode::UnknownBoard));
    // Neither asked the network for a release.
    for bench in [&b, &other] {
        assert!(bench.fetch.requested.lock().unwrap().is_empty());
    }
    // A colour radio's splash is not supported, whatever the version.
    let d = tempfile::tempdir().unwrap();
    let mut s = picture(d.path());
    s.board = Some("tx16s".into());
    let plan = other.core.gear_flash_plan(&params(&other, Some(s))).unwrap();
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
    assert!(failed(&plan).iter().any(|(n, c)| *n == "Splash layout" && *c == RefusalCode::UnknownVersion), "{:?}", failed(&plan));
}

#[test]
fn missing_and_doubled_markers_refuse_in_the_plan() {
    let d = tempfile::tempdir().unwrap();
    for (what, mutate) in [
        ("missing", Box::new(|b: &mut Vec<u8>| b[0x20000] = b'X') as Box<dyn Fn(&mut Vec<u8>)>),
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
        assert!(f.contains(&("Splash markers", RefusalCode::BadImage)), "{what}: {f:?}");
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
    assert_eq!(failed(&plan), [("One radio in DFU mode", RefusalCode::NoDevice)]);
    let one = DfuInfo { vid: 0x0483, pid: 0xdf11, serial: None };
    b.dfu.lock().unwrap().extend([one.clone(), one]);
    let plan = b.core.gear_flash_plan(&params(&b, None)).unwrap();
    assert_eq!(failed(&plan), [("One radio in DFU mode", RefusalCode::SeveralDevices)]);
    // A DFU device that is not the STM32 bootloader does not count.
    b.dfu.lock().unwrap().clear();
    b.dfu.lock().unwrap().push(DfuInfo { vid: 0x1209, pid: 0x0001, serial: None });
    let plan = b.core.gear_flash_plan(&params(&b, None)).unwrap();
    assert_eq!(failed(&plan), [("One radio in DFU mode", RefusalCode::NoDevice)]);
    let e = b
        .core
        .gear_flash(&FlashRequest { params: params(&b, None), digest: "x".into(), confirm: true })
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
    };
    b.core.gear_store().save_device(&fc).unwrap();
    let e = b
        .core
        .gear_flash_plan(&FlashParams { device: "fc-1".into(), ..FlashParams::default() })
        .unwrap_err();
    assert_eq!(refusal(e).code, RefusalCode::Incompatible);
    assert!(b
        .core
        .gear_flash_plan(&FlashParams { device: "nope".into(), ..FlashParams::default() })
        .is_err());
}

#[test]
fn apply_needs_confirm_and_the_plans_digest() {
    let b = plain();
    let p = params(&b, None);
    let plan = b.core.gear_flash_plan(&p).unwrap();
    let mut req = request(&p, &plan);
    req.confirm = false;
    assert!(b.core.gear_flash(&req).unwrap_err().to_string().contains("confirm=true"));
    let mut req = request(&p, &plan);
    req.digest = "0000000000000000".into();
    let e = b.core.gear_flash(&req).unwrap_err();
    assert_eq!(refusal(e).code, RefusalCode::BeforeMismatch);
    assert!(b.flasher.opened.lock().unwrap().is_empty(), "nothing opened a USB device");
    assert_eq!(b.flasher.device.lock().unwrap().flash[..b.old.len()], b.old[..]);
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
    assert_eq!(gui.asked.load(Ordering::SeqCst), 1, "the person was asked once");
    let names: Vec<&str> = report.steps.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        ["Back up the current firmware", "Erase", "Write", "Read back", "Leave DFU"]
    );
    assert!(report.steps.iter().all(|s| s.state == quadcam_lib::gear::apply::StepState::Done));

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

    // The firmware it ran before is kept as a before-flash backup.
    let backups = b
        .core
        .gear_backups(&BackupFilter { device: Some(b.radio.clone()) })
        .unwrap();
    assert_eq!(backups.len(), 1);
    assert_eq!(report.backup.as_deref(), Some(backups[0].id.as_str()));
    let kept = b
        .core
        .gear_backup_read(&quadcam_lib::core::BackupReadParams {
            id: backups[0].id.clone(),
            path: Some("firmware.bin".into()),
        })
        .unwrap();
    let text = serde_json::to_string(&kept).unwrap();
    assert!(text.contains("firmware.bin"), "{text}");
    assert!(gui.changed.load(Ordering::SeqCst) >= 1);
    // Nothing logs the picture's path or the radio's name in the report's notes.
    assert!(!report.message.contains("splash.png"));
    let _ = FLASH_BASE;
}

#[test]
fn the_apply_sheets_click_is_the_confirm_and_a_denied_request_writes_nothing() {
    let gui = Arc::new(Gui { deny: true, ..Gui::default() });
    let b = bench(gui.clone(), "pocket", VERSION, true);
    let p = params(&b, None);
    let plan = b.core.gear_flash_plan(&p).unwrap();
    let e = b.core.gear_flash(&request(&p, &plan)).unwrap_err();
    assert!(e.to_string().contains("cancelled"), "{e}");
    assert!(b.flasher.opened.lock().unwrap().is_empty());
    assert!(!b.flasher.device.lock().unwrap().left);
    assert!(b.core.gear_backups(&BackupFilter::default()).unwrap().is_empty());
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
    let states: Vec<_> = r.steps.iter().map(|s| (s.name.as_str(), s.state)).collect();
    use quadcam_lib::gear::apply::StepState::{Done, Failed, Skipped};
    assert_eq!(
        states,
        [
            ("Back up the current firmware", Done),
            ("Erase", Done),
            ("Write", Done),
            ("Read back", Failed),
            ("Leave DFU", Skipped)
        ]
    );
    assert!(!b.flasher.device.lock().unwrap().left, "never starts a bad image");
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
