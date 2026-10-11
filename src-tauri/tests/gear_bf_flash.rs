//! The Betaflight flash (preview): plan refusals, the flash of a fake FC over a fake DFU
//! device, the carry-over of the old settings through the FC apply, and the half states.
//! The build service is a fixture, the FC is `FakeFc`, the DFU device is `FakeDfu`; nothing
//! here downloads, opens a port or flashes anything real.

use quadcam_lib::core::{
    BackupFilter, BackupParams, Core, FirmwareParams, FlashParams, FlashRequest, Hooks, NoHooks,
};
use quadcam_lib::gear::apply::ApplyReport;
use quadcam_lib::gear::bf::cli::Timing;
use quadcam_lib::gear::bf::fake::FakeFc;
use quadcam_lib::gear::cues::{CueService, RecordedCues};
use quadcam_lib::gear::detect::DfuInfo;
use quadcam_lib::gear::dfu::{FakeDfu, Usb, FLASH_BASE};
use quadcam_lib::gear::firmware::betaflight::{to_hex, FixtureCloud};
use quadcam_lib::gear::firmware::check::{BETAFLIGHT_RELEASES, EDGETX_RELEASES, ELRS_INDEX};
use quadcam_lib::gear::firmware::FixtureFetch;
use quadcam_lib::gear::firmware::Flasher;
use quadcam_lib::gear::model::{
    device_id, ApplyPlan, ChangeStatus, DeviceKind, Refusal, RefusalCode, Section, StagedChange,
    Trigger,
};
use quadcam_lib::gear::serial::Ports;
use quadcam_lib::gear::Env;
use quadcam_lib::photos::Recorder;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PORT: &str = "/dev/cu.usbmodemFAKE1";
const NEW: &str = include_str!("fixtures/bf/g473v2-2026.6.0.dump_all.txt");
const UID: [u8; 12] = [
    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x10, 0x32, 0x54, 0x76,
];
const G4_512K: &str = "@Internal Flash  /0x08000000/256*02Kg";
const TARGET: &str = "BETAFPVG473_V2";
const RELEASE: &str = "2026.6.0";

/// The firmware the FC runs before the flash: the fixture's older release, with one setting
/// the new release does not have.
fn old_dump() -> String {
    NEW.replace("2026.6.0-alpha", "2025.12.5-alpha").replace(
        "manufacturer_id BEFH",
        "manufacturer_id BEFH\nset legacy_thing = 1",
    )
}

/// A firmware-shaped image of `kb` KB: a vector table, then filler.
fn image(kb: usize, salt: u8) -> Vec<u8> {
    let mut b = vec![salt; kb * 1024];
    b[0..4].copy_from_slice(&0x2002_0000u32.to_le_bytes());
    b[4..8].copy_from_slice(&0x0800_0101u32.to_le_bytes());
    b
}

/// A DFU device in a G4 FC. When the host leaves DFU, the FC starts the new firmware if the
/// flash holds the image, else the old one.
struct FcFlasher {
    dfu: Arc<Mutex<FakeDfu>>,
    fc: FakeFc,
    image: Vec<u8>,
    new_dump: String,
    old_dump: String,
    opened: AtomicUsize,
    /// The `wTransferSize` the DFU device reports after the firmware copy; 0 for none.
    transfer: Arc<AtomicUsize>,
}

struct Wrapped {
    dfu: Arc<Mutex<FakeDfu>>,
    fc: FakeFc,
    image: Vec<u8>,
    new_dump: String,
    old_dump: String,
    transfer: Arc<AtomicUsize>,
    asked: usize,
}

impl Usb for Wrapped {
    fn control_out(&mut self, request: u8, value: u16, data: &[u8]) -> anyhow::Result<()> {
        let mut d = self.dfu.lock().unwrap();
        d.control_out(request, value, data)?;
        if request == 1 && value == 0 && data.is_empty() {
            let flashed = d.flash[..self.image.len()] == self.image[..];
            self.fc.install(if flashed {
                &self.new_dump
            } else {
                &self.old_dump
            });
        }
        Ok(())
    }
    fn control_in(&mut self, request: u8, value: u16, len: usize) -> anyhow::Result<Vec<u8>> {
        self.dfu.lock().unwrap().control_in(request, value, len)
    }
    fn layout(&mut self) -> anyhow::Result<String> {
        self.dfu.lock().unwrap().layout()
    }
    fn transfer_size(&mut self) -> Option<usize> {
        // The copy asks first and gets none; the flash asks next.
        self.asked += 1;
        Some(self.transfer.load(Ordering::SeqCst)).filter(|&n| n > 0 && self.asked > 1)
    }
}

impl Flasher for FcFlasher {
    fn open_dfu(&self, _: u16, _: u16, _: Option<&str>) -> anyhow::Result<Box<dyn Usb>> {
        self.opened.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(Wrapped {
            dfu: self.dfu.clone(),
            fc: self.fc.clone(),
            image: self.image.clone(),
            new_dump: self.new_dump.clone(),
            old_dump: self.old_dump.clone(),
            transfer: self.transfer.clone(),
            asked: 0,
        }))
    }
    fn quick(&self) -> bool {
        true
    }
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
        assert_eq!(change.id, "flash");
        self.asked.fetch_add(1, Ordering::SeqCst);
        if self.deny {
            anyhow::bail!("Refused: the user cancelled the apply in quadcam.")
        }
        Ok(())
    }
}

struct Bench {
    core: Arc<Core>,
    fc: FakeFc,
    cloud: Arc<FixtureCloud>,
    flasher: Arc<FcFlasher>,
    image: Vec<u8>,
    id: String,
    /// Scripted: how many DFU devices the host sees besides the FC's own.
    extra_dfu: Arc<AtomicUsize>,
    /// The FC never shows up as a DFU device.
    hide_dfu: Arc<AtomicUsize>,
    _dir: tempfile::TempDir,
}

struct Opts {
    preview: bool,
    old: String,
    hooks: Arc<dyn Hooks>,
    /// A fake DFU device that is not a G4 with 512 KB.
    wrong_chip: bool,
    /// What `get NAME` says the FC allows.
    allowed: Option<(&'static str, &'static str)>,
    /// The build service has the build.
    serve: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            preview: true,
            old: old_dump(),
            hooks: Arc::new(NoHooks),
            wrong_chip: false,
            allowed: None,
            serve: true,
        }
    }
}

fn bench(o: Opts) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("support")).unwrap();
    std::fs::write(
        dir.path().join("support/settings.json"),
        if o.preview {
            r#"{"bfFlashPreview": true}"#
        } else {
            "{}"
        },
    )
    .unwrap();
    let mut fc = FakeFc::new(&o.old).with_uid(UID);
    if let Some((name, text)) = o.allowed {
        fc = fc.with_allowed(name, text);
    }
    // The person tuned the old firmware.
    fc.poke(Section::Master, "small_angle", "160");
    fc.poke(Section::Master, "osd_cap_alarm", "400");
    fc.poke(Section::Master, "legacy_thing", "3");
    let locks = dir.path().join("locks");
    let ports: Arc<dyn Ports> = Arc::new(fc.ports(PORT, Some(locks)));
    let extra_dfu = Arc::new(AtomicUsize::new(0));
    let hide_dfu = Arc::new(AtomicUsize::new(0));
    let mut env = Env::fake(vec![], ports);
    env.cues = Arc::new(CueService::inline(Arc::new(RecordedCues::default())));
    {
        let (fc, extra, hide) = (fc.clone(), extra_dfu.clone(), hide_dfu.clone());
        env.dfu = Arc::new(move || {
            let mut v: Vec<DfuInfo> = (0..extra.load(Ordering::SeqCst))
                .map(|i| DfuInfo {
                    vid: 0x0483,
                    pid: 0xdf11,
                    serial: Some(format!("OTHER{i}")),
                })
                .collect();
            if fc.in_bootloader() && hide.load(Ordering::SeqCst) == 0 {
                v.push(DfuInfo {
                    vid: 0x0483,
                    pid: 0xdf11,
                    serial: Some("FCCHIP".into()),
                });
            }
            v
        });
    }
    let img = image(300, 5);
    let cloud = Arc::new(FixtureCloud::new());
    if o.serve {
        cloud.serve(
            TARGET,
            RELEASE,
            "betaflight_2026.6.0_BETAFPVG473_V2.hex",
            to_hex(FLASH_BASE, &img).into_bytes(),
        );
    }
    let mut dfu = FakeDfu::with_firmware(&image(300, 9));
    if !o.wrong_chip {
        dfu.layout = G4_512K.into();
        dfu.flash = vec![0xFF; 512 * 1024];
        dfu.flash[..300 * 1024].copy_from_slice(&image(300, 9));
    }
    let flasher = Arc::new(FcFlasher {
        dfu: Arc::new(Mutex::new(dfu)),
        fc: fc.clone(),
        image: img.clone(),
        new_dump: NEW.to_string(),
        old_dump: o.old.clone(),
        opened: AtomicUsize::new(0),
        transfer: Arc::new(AtomicUsize::new(0)),
    });
    let fetch = Arc::new(FixtureFetch::new());
    let rel = |t: &str| serde_json::json!([{"tag_name": t, "prerelease": false, "draft": false, "assets": []}]);
    fetch.serve(
        EDGETX_RELEASES,
        serde_json::to_vec(&rel("v2.12.4")).unwrap(),
    );
    fetch.serve(
        BETAFLIGHT_RELEASES,
        serde_json::to_vec(&rel(RELEASE)).unwrap(),
    );
    fetch.serve(
        ELRS_INDEX,
        serde_json::json!({"tags": {"3.5.3": "x"}})
            .to_string()
            .into_bytes(),
    );
    let core = Arc::new(
        Core::new(
            dir.path().join("cache"),
            None,
            o.hooks,
            Arc::new(Recorder::default()),
        )
        .with_settings(dir.path().join("support/settings.json"))
        .with_gear_env(env)
        .with_fc_timing(Timing::fast())
        .with_firmware_env(quadcam_lib::gear::firmware::FwEnv {
            fetch: fetch.clone(),
            flasher: flasher.clone(),
        })
        .with_bf_cloud(cloud.clone()),
    );
    // The device is saved and has a backup, as after a plug-in.
    core.gear_backup(&BackupParams {
        port: Some(PORT.into()),
        ..Default::default()
    })
    .unwrap();
    Bench {
        id: device_id(DeviceKind::Fc, &format!("bf-uid:{}", fc.uid_hex())),
        core,
        fc,
        cloud,
        flasher,
        image: img,
        extra_dfu,
        hide_dfu,
        _dir: dir,
    }
}

fn params(b: &Bench) -> FlashParams {
    FlashParams {
        device: b.id.clone(),
        version: Some(RELEASE.into()),
        splash: None,
    }
}

fn request(b: &Bench, plan: &ApplyPlan) -> FlashRequest {
    FlashRequest {
        params: params(b),
        digest: plan.digest.clone(),
        confirm: true,
    }
}

fn failed_check(plan: &ApplyPlan) -> Refusal {
    plan.checks
        .iter()
        .find_map(|c| c.refusal.clone())
        .expect("a check failed")
}

fn step<'a>(r: &'a ApplyReport, name: &str) -> &'a quadcam_lib::gear::apply::StepReport {
    r.steps
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no step {name} in {:?}", r.steps))
}

#[test]
fn a_plan_names_the_change_and_runs_every_guard() {
    let b = bench(Opts::default());
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    assert!(plan.checks.iter().all(|c| c.ok), "{:?}", plan.checks);
    assert!(!plan.digest.is_empty());
    let text = format!("{:?}", plan.diff);
    assert!(
        text.contains("2025.12.5-alpha") && text.contains("2026.6.0"),
        "{text}"
    );
    assert!(text.contains("SHA-256"), "{text}");
    assert!(
        plan.warnings.iter().any(|w| w.contains("boot button")),
        "the recovery path is in the plan: {:?}",
        plan.warnings
    );
    assert!(plan.warnings.iter().any(|w| w.contains("preview")));
    // The plan reboots and writes nothing.
    assert!(!b.fc.in_bootloader());
    assert_eq!(b.fc.saves(), 0);
    assert_eq!(b.flasher.opened.load(Ordering::SeqCst), 0);
    assert!(!b.fc.log().iter().any(|l| l == "bl"), "{:?}", b.fc.log());
    // The same plan twice gives the same digest, and the second reads the cache.
    let again = b.core.gear_flash_plan(&params(&b)).unwrap();
    assert_eq!(again.digest, plan.digest);
    assert_eq!(b.cloud.count(), 1);
}

#[test]
fn the_plan_refuses_what_it_cannot_prove() {
    // The preview is off.
    let b = bench(Opts {
        preview: false,
        ..Opts::default()
    });
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    assert!(plan.digest.is_empty());
    assert_eq!(failed_check(&plan).code, RefusalCode::Disabled);
    assert_eq!(b.cloud.count(), 0, "no download for a refused plan");

    // A release compat has not proven on this board: refused before any download.
    let b = bench(Opts::default());
    let mut p = params(&b);
    p.version = Some("4.3.2".into());
    let plan = b.core.gear_flash_plan(&p).unwrap();
    assert_eq!(failed_check(&plan).code, RefusalCode::UnknownVersion);
    assert_eq!(b.cloud.count(), 0);

    // A board QuadCam has no target for.
    let other = old_dump().replace("board_name BETAFPVG473_V2", "board_name BETAFPVF411");
    let b = bench(Opts {
        old: other,
        ..Opts::default()
    });
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    assert_eq!(failed_check(&plan).code, RefusalCode::UnknownBoard);
    assert_eq!(b.cloud.count(), 0);

    // A DFU device already attached: the FC could not be told apart.
    let b = bench(Opts::default());
    b.extra_dfu.store(1, Ordering::SeqCst);
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    assert_eq!(failed_check(&plan).code, RefusalCode::SeveralDevices);
    assert!(plan.digest.is_empty());

    // An unknown saved device, and a splash on an FC.
    let b = bench(Opts::default());
    let mut p = params(&b);
    p.device = "fc-nope".into();
    assert!(b.core.gear_flash_plan(&p).is_err());
    let mut p = params(&b);
    p.splash = Some(quadcam_lib::gear::splash::SplashParams {
        image: "x.png".into(),
        threshold: None,
        invert: false,
        board: None,
    });
    let e = b.core.gear_flash_plan(&p).unwrap_err();
    assert_eq!(
        e.downcast_ref::<Refusal>().unwrap().code,
        RefusalCode::Incompatible
    );
}

#[test]
fn a_bad_image_refuses_the_plan() {
    let b = bench(Opts::default());
    // Not a firmware: no vector table.
    let mut junk = image(300, 5);
    junk[0..4].copy_from_slice(&0u32.to_le_bytes());
    b.cloud.serve(
        TARGET,
        RELEASE,
        "x.hex",
        to_hex(FLASH_BASE, &junk).into_bytes(),
    );
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    assert_eq!(failed_check(&plan).code, RefusalCode::BadImage);
    assert!(plan.digest.is_empty());
    // Not HEX at all.
    let b = bench(Opts::default());
    b.cloud.serve(TARGET, RELEASE, "x.hex", b"hello".to_vec());
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    assert_eq!(failed_check(&plan).code, RefusalCode::BadImage);
    // The service has no such build: a refused plan, not a flash.
    let b = bench(Opts {
        serve: false,
        ..Opts::default()
    });
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    let r = failed_check(&plan);
    assert_eq!(r.code, RefusalCode::BadImage);
    assert!(r.reason.contains("no build"), "{}", r.reason);
    assert!(plan.digest.is_empty());
}

#[test]
fn the_plan_refuses_when_the_usb_timer_would_run_out() {
    let b = bench(Opts::default());
    b.fc.set_battery(7.6);
    // The battery has been in for 590 of the board's 600 seconds.
    let t0 = Instant::now() - Duration::from_secs(590);
    // A first look with no battery (it also identifies the FC), then the battery is in.
    b.fc.set_battery(0.0);
    // (The FC refuses a few opens after the backup's reboot.)
    for back in [150, 120, 90, 60] {
        b.core.gear_usb_tick(t0 - Duration::from_secs(back));
    }
    b.fc.set_battery(7.6);
    let t = b.core.gear_usb_tick(t0);
    assert!(t[0].battery, "{t:?}");
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    let r = failed_check(&plan);
    assert_eq!(r.code, RefusalCode::UsbHeat, "{r:?}");
    assert!(plan.digest.is_empty());
    // With the battery out the timer is not a reason.
    b.fc.set_battery(0.0);
    b.core.gear_usb_tick(t0 + Duration::from_secs(60));
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    assert!(plan.checks.iter().all(|c| c.ok), "{:?}", plan.checks);
}

#[test]
fn a_battery_with_no_usb_timer_refuses_the_flash() {
    let b = bench(Opts::default());
    // The reads are paused (or the first probe has not run): no timer counts.
    b.core
        .gear_poll_pause(&quadcam_lib::core::PollPauseParams {
            port: Some(PORT.into()),
            paused: true,
        })
        .unwrap();
    b.fc.set_battery(7.6);
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    assert!(!plan.digest.is_empty(), "{:?}", plan.checks);
    // The job reads the battery before its heat check, and an untimed battery refuses.
    let e = b.core.gear_flash(&request(&b, &plan)).unwrap_err();
    let r = e.downcast_ref::<Refusal>().expect("a refusal");
    assert_eq!(r.code, RefusalCode::UsbHeat, "{r:?}");
    assert!(r.reason.contains("not counting"), "{}", r.reason);
    assert!(!b.fc.log().iter().any(|l| l == "bl"));
    assert_eq!(b.flasher.opened.load(Ordering::SeqCst), 0);
}

#[test]
fn a_flash_replaces_the_firmware_and_puts_the_settings_back() {
    let b = bench(Opts::default());
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    let report = b.core.gear_flash(&request(&b, &plan)).unwrap();
    assert_eq!(report.status, ChangeStatus::Verified, "{}", report.message);

    // The device holds the image, byte for byte, and the FC left the bootloader.
    let dfu = b.flasher.dfu.lock().unwrap();
    assert_eq!(&dfu.flash[..b.image.len()], &b.image[..]);
    assert!(dfu.left);
    drop(dfu);
    assert!(!b.fc.in_bootloader());

    // The order of the steps: back up, bootloader, erase, write, read back, leave, restart,
    // read, re-apply.
    let names: Vec<&str> = report.steps.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Back up",
            "Restart into the bootloader",
            "Copy the current firmware",
            "Erase",
            "Write",
            "Read back",
            "Leave DFU",
            "Restart",
            "Read new settings",
            "Re-apply settings"
        ]
    );
    assert!(report
        .steps
        .iter()
        .all(|s| s.state == quadcam_lib::gear::apply::StepState::Done));
    // The old firmware was read twice and kept before anything was erased.
    assert!(
        step(&report, "Copy the current firmware")
            .detail
            .as_deref()
            .unwrap()
            .contains("read twice, saved as"),
        "{:?}",
        report.steps
    );

    // The old firmware's setting is back on the new one; the new firmware's version shows.
    assert_eq!(
        b.fc.saved_value(Section::Master, "small_angle").as_deref(),
        Some("160")
    );
    assert_eq!(
        b.fc.saved_value(Section::Master, "osd_cap_alarm")
            .as_deref(),
        Some("400")
    );
    assert!(b.fc.saved_dump().contains("2026.6.0-alpha"));
    // A setting the new version does not have is reported and skipped, never guessed.
    assert_eq!(b.fc.saved_value(Section::Master, "legacy_thing"), None);
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("not in this Betaflight version") && n.contains("legacy_thing")),
        "{:?}",
        report.notes
    );
    // The apply verified every line against `dump all`.
    assert!(report.verify.is_empty());
    assert!(report.sent.iter().any(|r| r.line.contains("small_angle")));

    // The old state is kept for good, taken before anything changed, and the change that put
    // the settings back is in the history.
    let backups = b.core.gear_backups(&BackupFilter::default()).unwrap();
    let before = backups
        .iter()
        .find(|s| s.trigger == Trigger::BeforeFlash)
        .expect("a backup before the flash");
    assert_eq!(report.backup.as_deref(), Some(before.id.as_str()));
    assert!(backups.iter().any(|s| s.trigger == Trigger::BeforeApply));
    let changes = b
        .core
        .gear_changes(&quadcam_lib::gear::changes::ChangeFilter {
            history: true,
            ..Default::default()
        })
        .unwrap();
    assert!(
        changes
            .iter()
            .any(|c| c.title.contains("after the Betaflight 2026.6.0 flash")
                && c.status == ChangeStatus::Verified),
        "{changes:?}"
    );
}

#[test]
fn the_old_active_profiles_come_back_after_the_flash() {
    // The person flew on PID profile 2 and rate profile 1.
    let old = old_dump()
        .replace(
            "# restore original profile selection\nprofile 0",
            "# restore original profile selection\nprofile 2",
        )
        .replace(
            "# restore original rateprofile selection\nrateprofile 0",
            "# restore original rateprofile selection\nrateprofile 1",
        );
    assert_ne!(old, old_dump());
    let b = bench(Opts {
        old,
        ..Opts::default()
    });
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    let report = b.core.gear_flash(&request(&b, &plan)).unwrap();
    assert_eq!(report.status, ChangeStatus::Verified, "{}", report.message);
    let saved = quadcam_lib::gear::bf::dump::Config::parse(&b.fc.saved_dump());
    let last = |verb: &str| {
        saved
            .lines
            .iter()
            .rev()
            .map(|l| l.text.trim().to_string())
            .find(|t| {
                t.split_whitespace().next() == Some(verb) && t.split_whitespace().count() == 2
            })
    };
    assert_eq!(last("profile").as_deref(), Some("profile 2"));
    assert_eq!(last("rateprofile").as_deref(), Some("rateprofile 1"));
}

#[test]
fn a_value_the_new_version_refuses_is_skipped_with_its_reason() {
    let b = bench(Opts {
        allowed: Some(("small_angle", "Allowed range: 0 - 90")),
        ..Opts::default()
    });
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    let report = b.core.gear_flash(&request(&b, &plan)).unwrap();
    assert_eq!(report.status, ChangeStatus::Verified, "{}", report.message);
    // The other setting came back; the refused one stayed at the new default.
    assert_eq!(
        b.fc.saved_value(Section::Master, "osd_cap_alarm")
            .as_deref(),
        Some("400")
    );
    assert_ne!(
        b.fc.saved_value(Section::Master, "small_angle").as_deref(),
        Some("160")
    );
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("does not take the old value")
                && n.contains("small_angle = 160")
                && n.contains("0-90")),
        "{:?}",
        report.notes
    );
}

#[test]
fn a_flash_needs_the_digest_the_confirm_and_the_click() {
    let b = bench(Opts::default());
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    // No confirm.
    let mut r = request(&b, &plan);
    r.confirm = false;
    let e = b.core.gear_flash(&r).unwrap_err();
    assert!(format!("{e}").starts_with("Refused"), "{e}");
    // A digest from another plan.
    let mut r = request(&b, &plan);
    r.digest = "0000000000000000".into();
    let e = b.core.gear_flash(&r).unwrap_err();
    assert_eq!(
        e.downcast_ref::<Refusal>().unwrap().code,
        RefusalCode::BeforeMismatch
    );
    assert!(!b.fc.log().iter().any(|l| l == "bl"));
    assert!(!b.fc.in_bootloader());

    // With the app running, the person must click Apply.
    let gui = Arc::new(Gui {
        deny: true,
        ..Gui::default()
    });
    let b = bench(Opts {
        hooks: gui.clone(),
        ..Opts::default()
    });
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    let e = b.core.gear_flash(&request(&b, &plan)).unwrap_err();
    assert!(format!("{e}").contains("cancelled"), "{e}");
    assert_eq!(gui.asked.load(Ordering::SeqCst), 1);
    assert!(!b.fc.in_bootloader());
    assert!(!b.fc.log().iter().any(|l| l == "bl"));
    // The sheet's own click is the confirm.
    let report = b.core.gear_flash_click(&request(&b, &plan)).unwrap();
    assert_eq!(report.status, ChangeStatus::Verified, "{}", report.message);
    assert_eq!(
        gui.asked.load(Ordering::SeqCst),
        1,
        "the click does not ask again"
    );
}

#[test]
fn a_flash_stops_when_the_plan_changed() {
    let b = bench(Opts::default());
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    // A DFU device is plugged in after the plan.
    b.extra_dfu.store(1, Ordering::SeqCst);
    let e = b.core.gear_flash(&request(&b, &plan)).unwrap_err();
    assert_eq!(
        e.downcast_ref::<Refusal>().unwrap().code,
        RefusalCode::SeveralDevices
    );
    assert!(!b.fc.log().iter().any(|l| l == "bl"));
}

#[test]
fn an_fc_that_never_enters_dfu_is_reported_with_the_way_back() {
    let b = bench(Opts::default());
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    b.hide_dfu.store(1, Ordering::SeqCst);
    let report = b.core.gear_flash(&request(&b, &plan)).unwrap();
    assert_eq!(report.status, ChangeStatus::Failed);
    assert!(
        report.message.contains("did not show up as a DFU device"),
        "{}",
        report.message
    );
    assert!(report.message.contains("boot button"), "{}", report.message);
    assert!(
        report.message.contains("Nothing was written"),
        "{}",
        report.message
    );
    assert_eq!(b.flasher.opened.load(Ordering::SeqCst), 0);
    assert!(!report.saved);
    // The old settings are in the backup the report names.
    assert!(report.backup.is_some());
}

#[test]
fn a_dropped_block_leaves_the_fc_in_its_bootloader_and_says_so() {
    let b = bench(Opts::default());
    b.flasher.dfu.lock().unwrap().faults.drop_write_block = Some(3);
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    let report = b.core.gear_flash(&request(&b, &plan)).unwrap();
    assert_eq!(report.status, ChangeStatus::Failed);
    assert!(
        report.message.contains("stays in its bootloader"),
        "{}",
        report.message
    );
    assert!(report.message.contains("boot button"), "{}", report.message);
    assert!(
        report.message.contains("Betaflight Configurator")
            && !report.message.contains("flash again"),
        "{}",
        report.message
    );
    assert!(report.message.contains(report.backup.as_deref().unwrap()));
    // The write reads each segment back before the next: the bad one stops the flash.
    use quadcam_lib::gear::apply::StepState;
    assert_eq!(step(&report, "Erase").state, StepState::Done);
    assert_eq!(step(&report, "Write").state, StepState::Failed);
    assert_eq!(step(&report, "Read back").state, StepState::Skipped);
    assert_eq!(step(&report, "Leave DFU").state, StepState::Skipped);
    // No settings were written to a half flashed FC.
    assert!(b.fc.in_bootloader());
    assert!(b
        .core
        .gear_changes(&quadcam_lib::gear::changes::ChangeFilter {
            history: true,
            ..Default::default()
        })
        .unwrap()
        .is_empty());
}

#[test]
fn a_flash_that_fails_before_the_erase_says_nothing_was_erased() {
    let b = bench(Opts::default());
    // The bootloader moves another block size than QuadCam writes.
    b.flasher.transfer.store(1024, Ordering::SeqCst);
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    let report = b.core.gear_flash(&request(&b, &plan)).unwrap();
    assert_eq!(report.status, ChangeStatus::Failed);
    assert!(
        report.message.contains("Nothing was erased"),
        "{}",
        report.message
    );
    assert!(!report.message.contains("half"), "{}", report.message);
    use quadcam_lib::gear::apply::StepState;
    for s in ["Erase", "Write", "Read back", "Leave DFU"] {
        assert_eq!(step(&report, s).state, StepState::Skipped, "{s}");
    }
    assert!(step(&report, "Erase")
        .detail
        .as_deref()
        .unwrap()
        .contains("1024"));
    assert!(b.flasher.dfu.lock().unwrap().erased.is_empty());
    // QuadCam left DFU: the FC runs its old firmware again.
    assert!(!b.fc.in_bootloader());
    assert!(!report.saved);
}

#[test]
fn a_chip_of_the_wrong_size_is_left_alone() {
    let b = bench(Opts {
        wrong_chip: true,
        ..Opts::default()
    });
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    let report = b.core.gear_flash(&request(&b, &plan)).unwrap();
    assert_eq!(report.status, ChangeStatus::Failed);
    assert!(
        report.message.contains("not the chip"),
        "{}",
        report.message
    );
    assert!(
        report.message.contains("Nothing was written"),
        "{}",
        report.message
    );
    // Nothing was erased, and the old firmware was started again.
    let d = b.flasher.dfu.lock().unwrap();
    assert!(d.erased.is_empty());
    drop(d);
    assert!(!b.fc.in_bootloader());
    assert!(b.fc.saved_dump().contains("2025.12.5-alpha"));
}

#[test]
fn a_failed_backup_stops_before_the_bootloader() {
    let b = bench(Opts::default());
    let plan = b.core.gear_flash_plan(&params(&b)).unwrap();
    // The port drops out when the backup reads `diff all`.
    let _ = b.fc.clone().lose_port_on("diff all");
    let e = b.core.gear_flash(&request(&b, &plan)).unwrap_err();
    assert_eq!(
        e.downcast_ref::<Refusal>().unwrap().code,
        RefusalCode::NoBackup,
        "{e:#}"
    );
    assert!(format!("{e}").contains("Nothing was changed"), "{e}");
    assert!(!b.fc.log().iter().any(|l| l == "bl"));
    assert!(!b.fc.in_bootloader());
    assert_eq!(b.flasher.opened.load(Ordering::SeqCst), 0);
}

#[test]
fn the_firmware_page_offers_the_flash_only_with_the_preview_on() {
    for (preview, flashable) in [(true, true), (false, false)] {
        let b = bench(Opts {
            preview,
            ..Opts::default()
        });
        let view = b
            .core
            .gear_firmware(&FirmwareParams { check: Some(true) })
            .unwrap();
        let fc = view.devices.iter().find(|d| d.device == b.id).unwrap();
        assert_eq!(fc.flashable, flashable, "{fc:?}");
        assert_eq!(fc.latest.as_deref(), Some(RELEASE));
        assert_eq!(fc.note.is_none(), flashable, "{:?}", fc.note);
    }
    // A board without a target says why, with the preview on.
    let other = old_dump().replace("board_name BETAFPVG473_V2", "board_name BETAFPVF411");
    let b = bench(Opts {
        old: other,
        ..Opts::default()
    });
    let view = b
        .core
        .gear_firmware(&FirmwareParams { check: Some(true) })
        .unwrap();
    let fc = view.devices.iter().find(|d| d.device == b.id).unwrap();
    assert!(!fc.flashable);
    assert_eq!(
        fc.note.as_deref(),
        Some("QuadCam will not flash it: Board BETAFPVF411 cannot be flashed by QuadCam yet.")
    );
}
