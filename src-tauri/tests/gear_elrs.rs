//! ExpressLRS (design 6.4), a preview: read a transmitter module through a radio and a
//! receiver through an FC, stage and apply options, and plan and run a flash. Everything is
//! a fake: the host's CLI and the ELRS device are `gear::elrs::fake`, the release is served
//! from memory, `esptool` is a recorder. Nothing here opens a real port, downloads, or flashes.

use quadcam_lib::core::{
    Core, ElrsFlashParams, ElrsFlashRequest, ElrsParams, ElrsReadParams, Hooks, NoHooks,
    StageParams,
};
use quadcam_lib::gear::apply::{ApplyPlanParams, ApplyRequest};
use quadcam_lib::gear::elrs::fake::{FakeElrs, FakeHost};
use quadcam_lib::gear::elrs::image::{fixtures, INDEX_URL};
use quadcam_lib::gear::elrs::ElrsSet;
use quadcam_lib::gear::firmware::{FixtureFetch, FwEnv, Recorder as FlashRecorder};
use quadcam_lib::gear::model::{
    ApplyPlan, ChangeStatus, Device, DeviceKind, Edit, Identity, Refusal, RefusalCode,
};
use quadcam_lib::gear::serial::Ports;
use quadcam_lib::modules::manifest::Manifest;
use quadcam_lib::modules::run::{sha256_file, Runner};
use quadcam_lib::modules::Modules;
use quadcam_lib::photos::Recorder;
use serde_json::json;
use std::ffi::OsString;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const PHRASE: &str = "bench-phrase-not-real";
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

#[derive(Default)]
struct Gui {
    asked: AtomicUsize,
}

impl Hooks for Gui {
    fn has_gui(&self) -> bool {
        true
    }
    fn confirm_apply(
        &self,
        _change: &quadcam_lib::gear::model::StagedChange,
        _plan: &ApplyPlan,
    ) -> anyhow::Result<()> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// `esptool` as a recorder: every call, and a flash read that writes a file.
#[derive(Default)]
struct FakeEsptool {
    calls: Mutex<Vec<Vec<String>>>,
    /// What a write prints.
    write_says: Mutex<String>,
    fail_write: Mutex<bool>,
    fail_read: Mutex<bool>,
}

impl Runner for FakeEsptool {
    fn run(
        &self,
        _program: &Path,
        args: &[OsString],
        _cwd: &Path,
        _t: Duration,
    ) -> anyhow::Result<Output> {
        let a: Vec<String> = args
            .iter()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        self.calls.lock().unwrap().push(a.clone());
        let ok = |stdout: &str| Output {
            status: ExitStatus::from_raw(0),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        let bad = |why: &str| Output {
            status: ExitStatus::from_raw(1 << 8),
            stdout: Vec::new(),
            stderr: why.as_bytes().to_vec(),
        };
        if a.iter().any(|x| x == "read-flash") {
            if *self.fail_read.lock().unwrap() {
                return Ok(bad("A fatal error occurred: Failed to connect"));
            }
            std::fs::write(a.last().unwrap(), vec![0xA5u8; 4096])?;
            return Ok(ok("Read 4096 bytes"));
        }
        if *self.fail_write.lock().unwrap() {
            return Ok(bad("A fatal error occurred: write failed"));
        }
        Ok(ok(&self.write_says.lock().unwrap()))
    }
}

struct Bench {
    dir: tempfile::TempDir,
    core: Arc<Core>,
    esptool: Arc<FakeEsptool>,
    fetch: Arc<FixtureFetch>,
    gui: Arc<Gui>,
    radio: String,
    fc: String,
}

fn bundle_zip(dir: &Path) -> PathBuf {
    let src = dir.join("bundle-src");
    let _ = std::fs::remove_dir_all(&src);
    let root = src.join("firmware");
    let fw = root.join("FCC").join("Unified_ESP8285_2400_RX");
    std::fs::create_dir_all(&fw).unwrap();
    std::fs::write(fw.join("firmware.bin"), fixtures::stock_8285(2)).unwrap();
    std::fs::create_dir_all(root.join("hardware/RX")).unwrap();
    std::fs::write(root.join("hardware/targets.json"), fixtures::targets_json()).unwrap();
    std::fs::write(
        root.join("hardware/RX/Generic 2400.json"),
        fixtures::layout().to_string(),
    )
    .unwrap();
    let zip = dir.join("firmware.zip");
    let st = std::process::Command::new("/usr/bin/ditto")
        .args(["-c", "-k", "--sequesterRsrc"])
        .arg(&src)
        .arg(&zip)
        .status()
        .unwrap();
    assert!(st.success());
    zip
}

fn install_fake_esptool(root: &Path) {
    let v = root.join("esptool").join("5.4.0");
    std::fs::create_dir_all(&v).unwrap();
    let tool = v.join("esptool");
    std::fs::write(&tool, b"#!/bin/sh\n").unwrap();
    let sha = sha256_file(&tool).unwrap();
    std::fs::write(
        v.join("installed.json"),
        json!({
            "name": "esptool", "version": "5.4.0", "installed_at": "2026-10-09T00:00:00Z",
            "license": "GPL-2.0-or-later", "license_url": "", "source": "", "homepage": "",
            "assets": [], "size": 10,
            "tools": {"esptool": {"path": "esptool", "sha256": sha, "signing": "not_binary"}}
        })
        .to_string(),
    )
    .unwrap();
}

fn saved(id: &str, kind: DeviceKind, name: &str) -> Device {
    Device {
        id: id.into(),
        kind,
        name: name.into(),
        aircraft: None,
        identity: Identity::default(),
        last_seen: None,
        last_backup: None,
        last_space: None,
        aliases: Vec::new(),
        dfu_serial: None,
    }
}

fn set_settings(dir: &Path, v: serde_json::Value) {
    let map: serde_json::Map<String, serde_json::Value> = v.as_object().unwrap().clone();
    quadcam_lib::settings::set(&dir.join("support/settings.json"), &map).unwrap();
}

/// A bench with a radio (a Pocket-like transmitter module behind it) and an FC (a receiver
/// behind it), both on serial.
fn bench(preview: bool, esptool: bool) -> (Bench, FakeHost, FakeHost) {
    let dir = tempfile::tempdir().unwrap();
    let tx = FakeHost::radio(FakeElrs::tx("RM Radio", "4.1.0"));
    let rx = FakeHost::fc(FakeElrs::rx("Vendor 2.4GHz AIO RX", "3.5.3"));
    let radio_ports = tx.ports("/dev/cu.radio");
    let fc_ports = rx.ports("/dev/cu.fc");
    let both = quadcam_lib::gear::serial::FakePorts::new(
        radio_ports
            .list()
            .into_iter()
            .chain(fc_ports.list())
            .collect(),
    )
    .with_opener({
        let (tx, rx) = (tx.clone(), rx.clone());
        move |port, baud| {
            if port == "/dev/cu.radio" {
                tx.ports(port).open(port, baud)
            } else {
                rx.ports(port).open(port, baud)
            }
        }
    });
    let env = quadcam_lib::gear::Env::fake(vec![], Arc::new(both));
    let fetch = Arc::new(FixtureFetch::new());
    let zip = bundle_zip(dir.path());
    fetch.serve(
        INDEX_URL,
        json!({"tags": {"4.1.0": COMMIT, "3.5.3": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}})
            .to_string()
            .into_bytes(),
    );
    fetch.serve(
        &format!("https://artifactory.expresslrs.org/ExpressLRS/{COMMIT}/firmware.zip"),
        std::fs::read(&zip).unwrap(),
    );
    let runner = Arc::new(FakeEsptool::default());
    *runner.write_says.lock().unwrap() = "Hash of data verified.\nHard resetting...".into();
    let mods_root = dir.path().join("support/modules");
    if esptool {
        install_fake_esptool(&mods_root);
    }
    let modules = Modules::new(
        mods_root,
        dir.path().join("cache/modules"),
        Manifest::built_in(),
        "http://127.0.0.1:1/modules.json".into(),
        fetch.clone(),
    )
    .with_runner(runner.clone());
    let gui = Arc::new(Gui::default());
    let core = Arc::new(
        Core::new(
            dir.path().join("cache"),
            None,
            gui.clone() as Arc<dyn Hooks>,
            Arc::new(Recorder::default()),
        )
        .with_settings(dir.path().join("support/settings.json"))
        .with_gear_env(env)
        .with_fc_timing(quadcam_lib::gear::bf::cli::Timing::fast())
        .with_modules(modules)
        .with_firmware_env(FwEnv {
            fetch: fetch.clone(),
            flasher: Arc::new(FlashRecorder::empty()),
        }),
    );
    if preview {
        set_settings(
            dir.path(),
            json!({"elrs_preview": true, "elrs_binding_phrase": PHRASE}),
        );
    }
    let store = core.gear_store();
    store
        .save_device(&saved(
            "radio-0000000000000001",
            DeviceKind::Radio,
            "Pocket",
        ))
        .unwrap();
    store
        .save_device(&saved("fc-0000000000000001", DeviceKind::Fc, "Air"))
        .unwrap();
    let b = Bench {
        dir,
        core,
        esptool: runner,
        fetch,
        gui,
        radio: "radio-0000000000000001".into(),
        fc: "fc-0000000000000001".into(),
    };
    (b, tx, rx)
}

fn read(b: &Bench, host: &str) -> anyhow::Result<quadcam_lib::core::ElrsReadReport> {
    b.core.gear_elrs_read(&ElrsReadParams {
        host: host.into(),
        port: None,
    })
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

fn sets(v: &[(&str, &str)]) -> Vec<Edit> {
    vec![Edit::ElrsOptions {
        options: v
            .iter()
            .map(|(o, x)| ElrsSet {
                option: (*o).into(),
                value: (*x).into(),
            })
            .collect(),
    }]
}

#[test]
fn every_job_refuses_while_the_preview_is_off() {
    let (b, _, _) = bench(false, true);
    let e = read(&b, &b.radio).unwrap_err();
    assert!(
        format!("{e:#}").contains("ELRS tools (preview) are off"),
        "{e:#}"
    );
    let e = b
        .core
        .gear_elrs_flash_plan(&ElrsFlashParams {
            device: "elrs-tx-0".into(),
            ..Default::default()
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("are off"));
    let v = b.core.gear_elrs(&ElrsParams::default()).unwrap();
    assert!(!v.preview);
    assert!(v.devices.is_empty());
}

#[test]
fn the_view_reports_the_wifi_delay_in_effect() {
    let (b, _, _) = bench(true, true);
    let wifi = |b: &Bench| {
        b.core
            .gear_elrs(&ElrsParams::default())
            .unwrap()
            .wifi_interval
    };
    assert_eq!(wifi(&b), 60, "unset: the default");
    set_settings(b.dir.path(), json!({"elrs_wifi_interval": 0}));
    assert_eq!(wifi(&b), 0, "0 means never");
    set_settings(b.dir.path(), json!({"elrs_wifi_interval": 90}));
    assert_eq!(wifi(&b), 90);
}

#[test]
fn the_transmitter_module_is_read_through_the_radio() {
    let (b, tx, _) = bench(true, true);
    let r = read(&b, &b.radio).unwrap();
    assert_eq!(r.snapshot.role, "tx");
    assert_eq!(r.snapshot.name, "RM Radio");
    assert_eq!(r.snapshot.version.as_deref(), Some("4.1.0"));
    assert_eq!(r.snapshot.version_source.as_deref(), Some("parameters"));
    let keys: Vec<&str> = r.snapshot.options.iter().map(|o| o.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "packet_rate",
            "telemetry_ratio",
            "power",
            "dynamic_power",
            "switch_mode",
            "model_match"
        ]
    );
    // The CLI at 115200 stopped the pulses and started the passthrough; CRSF came after.
    assert_eq!(tx.opens(), [115_200, 400_000]);
    let log = tx.log();
    assert!(log.contains(&"set pulses 0".to_string()), "{log:?}");
    assert!(
        log.contains(&"serialpassthrough rfmod 0 400000".to_string()),
        "{log:?}"
    );
    assert!(!log.iter().any(|l| l.contains("bootpin")));
    assert!(r.notes.iter().any(|n| n.contains("restart the radio")));
    // The device is saved with its target and version.
    let d = b.core.gear_store().device(&r.device).unwrap().unwrap();
    assert_eq!(d.kind, DeviceKind::ElrsTx);
    assert_eq!(d.identity.version.as_deref(), Some("4.1.0"));
    assert_eq!(d.identity.target.as_deref(), Some("RM Radio"));
    // The radio stays in passthrough: a second job needs it restarted.
    assert!(read(&b, &b.radio).is_err());
    tx.restart();
    // The same device reads to the same id.
    assert_eq!(read(&b, &b.radio).unwrap().device, r.device);
    let v = b.core.gear_elrs(&ElrsParams::default()).unwrap();
    assert_eq!(v.devices.len(), 1);
    assert_eq!(v.devices[0].snapshot.device, r.device);
    assert!(v.phrase_set);
}

#[test]
fn the_receiver_is_read_through_the_fc_after_its_settings_pass() {
    let (b, _, rx) = bench(true, true);
    let r = read(&b, &b.fc).unwrap();
    assert_eq!(r.snapshot.role, "rx");
    assert_eq!(r.snapshot.version.as_deref(), Some("3.5.3"));
    assert!(
        rx.log().contains(&"serialpassthrough 2 420000".to_string()),
        "{:?}",
        rx.log()
    );
    assert_eq!(rx.opens().last(), Some(&420_000));
    assert!(r.notes.iter().any(|n| n.contains("unplug")));
    let d = b.core.gear_store().device(&r.device).unwrap().unwrap();
    assert_eq!(d.kind, DeviceKind::ElrsRx);
}

#[test]
fn an_fc_with_the_wrong_receiver_settings_never_starts_a_passthrough() {
    let (b, _, _) = bench(true, true);
    // Replace the FC with one whose receiver is SBUS and inverted.
    let bad = FakeHost::fc(FakeElrs::rx("Vendor 2.4GHz AIO RX", "4.1.0"))
        .with_receiver_settings("SBUS", "ON", "OFF");
    let ports = bad.ports("/dev/cu.fc");
    let env = quadcam_lib::gear::Env::fake(vec![], Arc::new(ports));
    let core = Core::new(
        b.dir.path().join("cache2"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(b.dir.path().join("support/settings.json"))
    .with_gear_env(env)
    .with_fc_timing(quadcam_lib::gear::bf::cli::Timing::fast());
    let e = core
        .gear_elrs_read(&ElrsReadParams {
            host: b.fc.clone(),
            port: None,
        })
        .unwrap_err();
    let text = format!("{e:#}");
    assert!(text.contains("serialrx_provider is SBUS"), "{text}");
    assert!(text.contains("serialrx_inverted is ON"), "{text}");
    assert!(!bad.log().iter().any(|l| l.starts_with("serialpassthrough")));
}

#[test]
fn a_device_that_does_not_answer_is_an_error_with_a_hint() {
    let (b, _, _) = bench(true, true);
    let dead = FakeHost::radio(FakeElrs::tx("RM Radio", "4.1.0").mute());
    let env = quadcam_lib::gear::Env::fake(vec![], Arc::new(dead.ports("/dev/cu.radio")));
    let core = Core::new(
        b.dir.path().join("cache3"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(b.dir.path().join("support/settings.json"))
    .with_gear_env(env)
    .with_fc_timing(quadcam_lib::gear::bf::cli::Timing::fast());
    let e = core
        .gear_elrs_read(&ElrsReadParams {
            host: b.radio.clone(),
            port: None,
        })
        .unwrap_err();
    assert!(
        format!("{e:#}").contains("No ExpressLRS module answered"),
        "{e:#}"
    );
}

#[test]
fn options_are_staged_planned_applied_and_read_back() {
    let (b, tx, _) = bench(true, true);
    let id = read(&b, &b.radio).unwrap().device;
    // A phrase is never an option.
    let e = b
        .core
        .gear_change_stage(&StageParams {
            device: id.clone(),
            edits: sets(&[("binding_phrase", "secret")]),
            ..Default::default()
        })
        .unwrap_err();
    assert!(!format!("{e:#}").contains("secret"));
    assert!(format!("{e:#}").contains("binding phrase"));
    // A value the device does not offer is refused at staging.
    assert!(b
        .core
        .gear_change_stage(&StageParams {
            device: id.clone(),
            edits: sets(&[("packet_rate", "9000hz")]),
            ..Default::default()
        })
        .is_err());

    tx.restart();
    let c = b
        .core
        .gear_change_stage(&StageParams {
            device: id.clone(),
            edits: sets(&[
                ("packet_rate", "250hz"),
                ("telemetry_ratio", "1:16"),
                ("model_match", "on"),
            ]),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(c.title, "3 ELRS options");
    let plan = b
        .core
        .gear_apply_plan(&ApplyPlanParams {
            id: c.id.clone(),
            port: None,
        })
        .unwrap();
    assert!(failed(&plan).is_empty(), "{:?}", failed(&plan));
    assert!(!plan.digest.is_empty());
    // Without the digest and confirm nothing is written.
    assert!(b
        .core
        .gear_apply(&ApplyRequest {
            id: c.id.clone(),
            digest: "wrong".into(),
            confirm: true,
            port: None,
        })
        .is_err());
    assert!(tx.device().writes().is_empty());

    let report = b
        .core
        .gear_apply(&ApplyRequest {
            id: c.id.clone(),
            digest: plan.digest.clone(),
            confirm: true,
            port: None,
        })
        .unwrap();
    assert_eq!(report.status, ChangeStatus::Verified, "{}", report.message);
    assert_eq!(b.gui.asked.load(Ordering::SeqCst), 1);
    assert_eq!(tx.device().writes().len(), 3);
    let names: Vec<&str> = report.steps.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"Back up the parameters"));
    assert!(names.contains(&"Verify"));
    assert!(report.backup.is_some());
    // The snapshot holds what the device holds now.
    let v = b.core.gear_elrs(&ElrsParams::default()).unwrap();
    let o = &v.devices[0].snapshot.options;
    assert_eq!(
        o.iter().find(|x| x.key == "packet_rate").unwrap().value,
        "250Hz(-108dBm)"
    );
    assert_eq!(
        o.iter().find(|x| x.key == "model_match").unwrap().value,
        "On"
    );
    let after = b
        .core
        .gear_changes(&quadcam_lib::gear::changes::ChangeFilter {
            history: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(after[0].status, ChangeStatus::Verified);
}

#[test]
fn an_apply_refuses_when_the_device_moved_since_the_read() {
    let (b, tx, _) = bench(true, true);
    let id = read(&b, &b.radio).unwrap().device;
    let c = b
        .core
        .gear_change_stage(&StageParams {
            device: id,
            edits: sets(&[("switch_mode", "Hybrid")]),
            ..Default::default()
        })
        .unwrap();
    let plan = b
        .core
        .gear_apply_plan(&ApplyPlanParams {
            id: c.id.clone(),
            port: None,
        })
        .unwrap();
    // Someone sets the switch mode on the radio after the read, and restarts it.
    tx.restart();
    tx.device().set_index(3, 0);
    let e = b
        .core
        .gear_apply(&ApplyRequest {
            id: c.id,
            digest: plan.digest,
            confirm: true,
            port: None,
        })
        .unwrap_err();
    let r = refusal(e);
    assert_eq!(r.code, RefusalCode::BeforeMismatch);
    assert!(r.reason.contains("Nothing was written"));
    assert!(tx.device().writes().is_empty());
}

fn planned(b: &Bench, device: &str) -> (ElrsFlashParams, ApplyPlan) {
    let p = ElrsFlashParams {
        device: device.into(),
        version: Some("4.1.0".into()),
        sha256: None,
        port: None,
    };
    let plan = b.core.gear_elrs_flash_plan(&p).unwrap();
    (p, plan)
}

fn rx_device(b: &Bench) -> String {
    read(b, &b.fc).unwrap().device
}

#[test]
fn a_flash_plan_shows_the_image_and_only_a_fingerprint_of_the_phrase() {
    let (b, _, _) = bench(true, true);
    let id = rx_device(&b);
    let (_, plan) = planned(&b, &id);
    assert!(failed(&plan).is_empty(), "{:?}", failed(&plan));
    assert!(!plan.digest.is_empty());
    let text = serde_json::to_string(&plan).unwrap();
    assert!(text.contains("Binding: UID fingerprint"));
    assert!(text.contains("Vendor 2.4GHz AIO RX"));
    assert!(!text.contains(PHRASE));
    let uid = quadcam_lib::gear::elrs::uid::uid_of(PHRASE);
    assert!(!text.contains(&format!("{:?}", uid)));
    // The download is recorded, not trusted: the plan says so.
    assert!(plan.warnings.iter().any(|w| w.contains("recorded")));
    // The same plan twice agrees.
    assert_eq!(planned(&b, &id).1.digest, plan.digest);
}

#[test]
fn a_flash_runs_esptool_after_a_backup_and_reports_its_hash_check() {
    let (b, _, rx) = bench(true, true);
    let id = rx_device(&b);
    let (p, plan) = planned(&b, &id);
    rx.restart();
    let req = ElrsFlashRequest {
        params: p,
        digest: plan.digest.clone(),
        confirm: true,
    };
    let report = b.core.gear_elrs_flash(&req).unwrap();
    assert_eq!(report.status, ChangeStatus::Verified, "{}", report.message);
    assert_eq!(b.gui.asked.load(Ordering::SeqCst), 1);
    // The receiver was asked to restart into its bootloader.
    assert_eq!(rx.device().bootloader_requests(), 1);
    let calls = b.esptool.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 2);
    let read_call = calls[0].join(" ");
    assert!(
        read_call.contains("--chip esp8266") && read_call.contains("read-flash 0 ALL"),
        "{read_call}"
    );
    let write_call = calls[1].join(" ");
    assert!(
        write_call.contains("--before no-reset") && write_call.contains("write-flash 0x0000"),
        "{write_call}"
    );
    // The backup of the chip's flash is kept.
    assert!(report.backup.is_some());
    assert!(report
        .steps
        .iter()
        .any(|s| s.name == "Back up the current firmware"));
    // The binding phrase appears nowhere in the arguments or the report.
    let all = format!("{calls:?}{}", serde_json::to_string(&report).unwrap());
    assert!(!all.contains(PHRASE));
    let d = b.core.gear_store().device(&id).unwrap().unwrap();
    assert_eq!(d.identity.version.as_deref(), Some("4.1.0"));
}

#[test]
fn a_flash_needs_the_digest_and_confirm() {
    let (b, _, _) = bench(true, true);
    let id = rx_device(&b);
    let (p, plan) = planned(&b, &id);
    let mut req = ElrsFlashRequest {
        params: p,
        digest: plan.digest.clone(),
        confirm: false,
    };
    assert!(b.core.gear_elrs_flash(&req).is_err());
    req.confirm = true;
    req.digest = "stale".into();
    assert_eq!(
        refusal(b.core.gear_elrs_flash(&req).unwrap_err()).code,
        RefusalCode::BeforeMismatch
    );
    assert!(b.esptool.calls.lock().unwrap().is_empty());
}

#[test]
fn a_failed_backup_stops_before_anything_is_written() {
    let (b, _, rx) = bench(true, true);
    let id = rx_device(&b);
    let (p, plan) = planned(&b, &id);
    rx.restart();
    *b.esptool.fail_read.lock().unwrap() = true;
    let e = b
        .core
        .gear_elrs_flash(&ElrsFlashRequest {
            params: p,
            digest: plan.digest,
            confirm: true,
        })
        .unwrap_err();
    assert_eq!(refusal(e).code, RefusalCode::NoBackup);
    assert_eq!(
        b.esptool.calls.lock().unwrap().len(),
        1,
        "only the read ran"
    );
}

#[test]
fn a_write_that_esptool_does_not_verify_is_not_called_verified() {
    let (b, _, rx) = bench(true, true);
    let id = rx_device(&b);
    let (p, plan) = planned(&b, &id);
    rx.restart();
    *b.esptool.write_says.lock().unwrap() = "Writing at 0x00000000... (100 %)".into();
    let r = b
        .core
        .gear_elrs_flash(&ElrsFlashRequest {
            params: p.clone(),
            digest: plan.digest.clone(),
            confirm: true,
        })
        .unwrap();
    assert_eq!(r.status, ChangeStatus::Failed);
    assert!(r.message.contains("did not report"));
    rx.restart();
    *b.esptool.fail_write.lock().unwrap() = true;
    let r = b
        .core
        .gear_elrs_flash(&ElrsFlashRequest {
            params: p,
            digest: plan.digest,
            confirm: true,
        })
        .unwrap();
    assert_eq!(r.status, ChangeStatus::Failed);
    assert!(r.message.contains("backup"));
    let d = b.core.gear_store().device(&id).unwrap().unwrap();
    assert_eq!(
        d.identity.version.as_deref(),
        Some("3.5.3"),
        "an unverified flash keeps the old version"
    );
}

#[test]
fn the_plan_refuses_without_esptool_a_phrase_a_known_target_or_a_good_checksum() {
    // No esptool module.
    let (b, _, _) = bench(true, false);
    let id = rx_device(&b);
    let plan = b
        .core
        .gear_elrs_flash_plan(&ElrsFlashParams {
            device: id,
            version: Some("4.1.0".into()),
            ..Default::default()
        })
        .unwrap();
    assert!(failed(&plan)
        .iter()
        .any(|(n, _)| *n == "The esptool module is installed"));
    assert!(plan.digest.is_empty());

    // No phrase.
    let (b, _, _) = bench(true, true);
    set_settings(b.dir.path(), json!({"elrs_binding_phrase": null}));
    let id = rx_device(&b);
    let plan = b
        .core
        .gear_elrs_flash_plan(&ElrsFlashParams {
            device: id,
            version: Some("4.1.0".into()),
            ..Default::default()
        })
        .unwrap();
    assert!(failed(&plan)
        .iter()
        .any(|(n, c)| *n == "A binding phrase is set" && *c == RefusalCode::BadSetting));

    // A target the release does not name.
    let (b, _, _) = bench(true, true);
    let id = rx_device(&b);
    let mut snap = quadcam_lib::gear::elrs::load_snapshot(b.core.gear_store().root(), &id).unwrap();
    snap.name = "Some Other Receiver".into();
    quadcam_lib::gear::elrs::save_snapshot(b.core.gear_store().root(), &snap).unwrap();
    let plan = b
        .core
        .gear_elrs_flash_plan(&ElrsFlashParams {
            device: id.clone(),
            version: Some("4.1.0".into()),
            ..Default::default()
        })
        .unwrap();
    assert!(
        failed(&plan)
            .iter()
            .any(|(n, c)| *n == "A known target and version" && *c == RefusalCode::UnknownBoard),
        "{:?}",
        failed(&plan)
    );

    // A checksum that does not match deletes the download.
    let (b, _, _) = bench(true, true);
    let id = rx_device(&b);
    let plan = b
        .core
        .gear_elrs_flash_plan(&ElrsFlashParams {
            device: id,
            version: Some("4.1.0".into()),
            sha256: Some("00".repeat(32)),
            ..Default::default()
        })
        .unwrap();
    assert!(plan.checks.iter().any(|c| !c.ok
        && c.refusal
            .as_ref()
            .is_some_and(|r| r.reason.contains("expected checksum"))));
    let _ = (&b.fetch, &b.radio);
}

#[test]
fn a_pair_on_different_majors_is_warned_about() {
    let (b, _, _) = bench(true, true);
    let rx = rx_device(&b);
    let tx = read(&b, &b.radio).unwrap();
    let v = b.core.gear_elrs(&ElrsParams::default()).unwrap();
    assert!(
        v.warnings.iter().any(|w| w.contains("do not link")),
        "{:?}",
        v.warnings
    );
    // With the transmitter on 3, a flash of the receiver to 4 would break the link.
    let root = b.core.gear_store().root().to_path_buf();
    let mut snap = quadcam_lib::gear::elrs::load_snapshot(&root, &tx.device).unwrap();
    snap.version = Some("3.6.4".into());
    quadcam_lib::gear::elrs::save_snapshot(&root, &snap).unwrap();
    let (_, plan) = planned(&b, &rx);
    assert!(
        plan.warnings.iter().any(|w| w.contains("will not link")),
        "{:?}",
        plan.warnings
    );
}
