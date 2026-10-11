//! A radio over USB: the same saved radio in a reader and in USB Storage mode, cleaning
//! AppleDouble files, the EdgeTX serial link and linking a DFU device. Fakes only: no real
//! volume, port or USB device is touched.

use quadcam_lib::core::{Core, NoHooks};
use quadcam_lib::disk::{DiskInfo, Volume};
use quadcam_lib::gear::detect::{CardHw, UsbStorage};
use quadcam_lib::gear::model::{DeviceKind, UsbInfo};
use quadcam_lib::gear::serial::FakePorts;
use quadcam_lib::gear::Env;
use quadcam_lib::photos::Recorder;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// A synthetic EdgeTX card; `bus` and `disk` say where it sits.
fn radio_card(root: &Path, bus: &str, disk: &str) -> Volume {
    for d in ["RADIO", "MODELS", "LOGS"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    std::fs::write(
        root.join("RADIO/radio.yml"),
        "checksum: 0\r\nsemver: 2.12.4\r\nboard: pocket\r\ncurrModel: 0\r\n",
    )
    .unwrap();
    Volume {
        mount: root.to_path_buf(),
        info: DiskInfo {
            volume_uuid: Some("TEST-UUID-CARD".into()),
            parent_whole_disk: disk.into(),
            bus_protocol: Some(bus.into()),
            removable: true,
            ..Default::default()
        },
        is_card: false,
        source: None,
        is_radio: quadcam_lib::disk::looks_like_radio(root),
        warnings: vec![],
    }
}

type Shared<T> = Arc<Mutex<T>>;

fn core(
    dir: &Path,
    volumes: Shared<Vec<Volume>>,
    cards: Vec<CardHw>,
    usb: Vec<UsbStorage>,
) -> Core {
    let mut env = Env::fake(Vec::new(), Arc::new(FakePorts::new(vec![])));
    env.volumes = Arc::new(move || volumes.lock().unwrap().clone());
    env.card_reader = Arc::new(move || cards.clone());
    env.usb = Arc::new(move || usb.clone());
    Core::new(
        dir.join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(dir.join("support/settings.json"))
    .with_gear_env(env)
}

fn radio_usb_device(disk: &str) -> UsbStorage {
    UsbStorage {
        info: UsbInfo {
            vid: 0x0483,
            pid: 0x5720,
            vendor: Some("OpenTX".into()),
            product: Some("Pocket Mass Storage".into()),
            serial: Some("000000000000".into()),
            version: Some("2.12".into()),
        },
        disks: vec![disk.into()],
    }
}

#[test]
fn a_radio_card_is_one_saved_radio_in_the_slot_and_over_usb() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("card");
    let vols = Arc::new(Mutex::new(vec![radio_card(
        &root,
        "Secure Digital",
        "disk42",
    )]));
    let hw = CardHw {
        disk: "disk42".into(),
        manufacturer_id: Some("0x03".into()),
        serial: Some("0xDEADBEEF".into()),
        ..Default::default()
    };
    let in_slot = core(d.path(), vols.clone(), vec![hw], vec![]);
    let c = in_slot.gear_connected().unwrap();
    let slot_id = c[0].id.clone().unwrap();
    assert!(c[0].usb.is_none());
    assert_eq!(c[0].also.len(), 1, "the volume UUID is the other id");
    in_slot
        .gear_device_save(&quadcam_lib::core::DeviceSaveParams {
            id: slot_id.clone(),
            name: Some("Bench radio".into()),
            ..Default::default()
        })
        .unwrap();
    // The next look records the alias.
    in_slot.gear_connected().unwrap();

    // The same card, now in the radio over USB: no hardware serial, a volume UUID id.
    *vols.lock().unwrap() = vec![radio_card(&root, "USB", "disk43")];
    let over_usb = core(d.path(), vols, vec![], vec![radio_usb_device("disk43")]);
    let c = over_usb.gear_connected().unwrap();
    assert!(c[0].usb.is_some());
    assert_eq!(c[0].id.as_deref(), Some(slot_id.as_str()));
    assert_eq!(c[0].device.as_ref().unwrap().name, "Bench radio");
    assert_eq!(c[0].kind, DeviceKind::Radio);
}

/// AppleDouble header: magic, version, filler.
const AD: [u8; 8] = [0x00, 0x05, 0x16, 0x07, 0x00, 0x02, 0x00, 0x00];

#[test]
fn card_clean_lists_then_removes_only_apple_double_files() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("card");
    let vols = Arc::new(Mutex::new(vec![radio_card(&root, "USB", "disk43")]));
    std::fs::write(root.join("MODELS/._model01.yml"), AD).unwrap();
    std::fs::write(root.join("._radio"), AD).unwrap();
    // Not AppleDouble: somebody's file that happens to start with ._
    std::fs::write(root.join("MODELS/._notes.txt"), b"my notes").unwrap();
    std::fs::write(root.join("MODELS/model01.yml"), b"header:\n").unwrap();
    let core = core(d.path(), vols, vec![], vec![radio_usb_device("disk43")]);
    let p = quadcam_lib::core::CardCleanParams {
        mount: Some(root.clone()),
        ..Default::default()
    };
    let listed = core.gear_card_clean(&p).unwrap();
    let paths: Vec<_> = listed.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["._radio", "MODELS/._model01.yml"]);
    assert_eq!(listed.files.len(), 2);
    assert_eq!(listed.removed, 0);
    assert!(listed.radio_usb);

    let refused = core
        .gear_card_clean(&quadcam_lib::core::CardCleanParams {
            remove: true,
            ..p.clone()
        })
        .unwrap_err();
    assert!(format!("{refused:#}").contains("confirm"));
    assert!(root.join("._radio").exists());

    let done = core
        .gear_card_clean(&quadcam_lib::core::CardCleanParams {
            remove: true,
            confirm: true,
            ..p
        })
        .unwrap();
    assert_eq!(done.removed, 2);
    assert!(!root.join("._radio").exists());
    assert!(!root.join("MODELS/._model01.yml").exists());
    assert!(
        root.join("MODELS/._notes.txt").exists(),
        "not AppleDouble: kept"
    );
    assert!(root.join("MODELS/model01.yml").exists());
}

// --- The EdgeTX CLI over USB Serial -------------------------------------------------

use quadcam_lib::core::{RadioCliAction, RadioCliParams};
use quadcam_lib::gear::edgetx::cli::FakeRadioCli;

const SERIAL: &str = "/dev/cu.usbmodemRADIO1";

/// A core with a radio on serial (the fake) and, optionally, its card in a reader.
fn serial_core(dir: &Path, radio: &FakeRadioCli, volumes: Vec<Volume>) -> Core {
    let mut env = Env::fake(volumes, Arc::new(radio.ports(SERIAL)));
    env.card_reader = Arc::new(Vec::new);
    Core::new(
        dir.join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(dir.join("support/settings.json"))
    .with_gear_env(env)
    .with_fc_timing(quadcam_lib::gear::bf::cli::Timing::fast())
}

fn p(action: RadioCliAction) -> RadioCliParams {
    RadioCliParams {
        action,
        ..Default::default()
    }
}

#[test]
fn a_radio_on_serial_is_a_radio_and_identifies_itself() {
    let d = tempfile::tempdir().unwrap();
    let radio = FakeRadioCli::new("pocket", "2.12.4");
    let core = serial_core(d.path(), &radio, vec![]);
    let c = core.gear_connected().unwrap();
    assert_eq!(c.len(), 1);
    assert_eq!(
        c[0].kind,
        DeviceKind::Radio,
        "a radio's serial port is not an FC"
    );
    let r = core.gear_radio_cli(&p(RadioCliAction::Identify)).unwrap();
    let info = r.info.unwrap();
    assert_eq!(
        (info.board.as_deref(), info.version.as_deref()),
        (Some("pocket"), Some("2.12.4"))
    );
    // The next look shows what the radio runs.
    let c = core.gear_connected().unwrap();
    assert_eq!(c[0].identity.version.as_deref(), Some("2.12.4"));
    assert_eq!(c[0].identity.board.as_deref(), Some("pocket"));
}

#[test]
fn play_ls_beep_and_reboot_need_the_right_arguments() {
    let d = tempfile::tempdir().unwrap();
    let radio = FakeRadioCli::new("pocket", "2.12.4").with_file("/SOUNDS/en/hello.wav", 10);
    let core = serial_core(d.path(), &radio, vec![]);
    let r = core
        .gear_radio_cli(&RadioCliParams {
            path: Some("/SOUNDS/en".into()),
            ..p(RadioCliAction::Ls)
        })
        .unwrap();
    assert_eq!(r.entries[0].name, "hello.wav");
    core.gear_radio_cli(&RadioCliParams {
        path: Some("/SOUNDS/en/hello.wav".into()),
        ..p(RadioCliAction::Play)
    })
    .unwrap();
    // A voice line's card path has no leading slash.
    core.gear_radio_cli(&RadioCliParams {
        path: Some("SOUNDS/en/hello.wav".into()),
        ..p(RadioCliAction::Play)
    })
    .unwrap();
    assert_eq!(
        radio.played(),
        ["/SOUNDS/en/hello.wav", "/SOUNDS/en/hello.wav"]
    );
    assert!(
        core.gear_radio_cli(&p(RadioCliAction::Play)).is_err(),
        "play needs a path"
    );
    core.gear_radio_cli(&p(RadioCliAction::Beep)).unwrap();
    let refused = core.gear_radio_cli(&p(RadioCliAction::Reboot)).unwrap_err();
    assert!(format!("{refused:#}").contains("confirm"));
    assert!(!radio.rebooted());
    core.gear_radio_cli(&RadioCliParams {
        confirm: true,
        ..p(RadioCliAction::Reboot)
    })
    .unwrap();
    assert!(radio.rebooted());
}

#[test]
fn another_program_holding_the_port_is_a_refusal() {
    let d = tempfile::tempdir().unwrap();
    let radio = FakeRadioCli::new("pocket", "2.12.4");
    let mut env = Env::fake(vec![], Arc::new(radio.ports(SERIAL)));
    env.holders = Arc::new(|_| vec![(4242, "screen".to_string())]);
    let core = Core::new(
        d.path().join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(d.path().join("support/settings.json"))
    .with_gear_env(env);
    let e = core
        .gear_radio_cli(&p(RadioCliAction::Identify))
        .unwrap_err();
    assert!(format!("{e:#}").contains("open in screen"));
    assert_eq!(radio.opens(), 0, "QuadCam never opened the port");
}

#[test]
fn verify_compares_the_radio_with_the_latest_backup() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("card");
    let vol = radio_card(&root, "USB", "disk43");
    std::fs::write(root.join("MODELS/model01.yml"), "header:\n  name: A\n").unwrap();
    std::fs::write(root.join("MODELS/model02.yml"), "header:\n  name: B\n").unwrap();
    std::fs::write(root.join("LOGS/x.csv"), "not checked").unwrap();
    // A folder with a space: the CLI splits the argument, so it cannot list it.
    std::fs::create_dir_all(root.join("SOUNDS/My Pack")).unwrap();
    std::fs::write(root.join("SOUNDS/My Pack/a.wav"), "RIFF").unwrap();
    let size = |p: &str| std::fs::metadata(root.join(p)).unwrap().len();
    let radio = FakeRadioCli::new("pocket", "2.12.4")
        .with_file("/RADIO/radio.yml", size("RADIO/radio.yml"))
        .with_file("/MODELS/model01.yml", size("MODELS/model01.yml"));
    let core = serial_core(d.path(), &radio, vec![vol]);
    let id = core
        .gear_connected()
        .unwrap()
        .into_iter()
        .find(|c| {
            c.usb.is_none() && matches!(c.link, quadcam_lib::gear::model::Link::Volume { .. })
        })
        .unwrap()
        .id
        .unwrap();
    core.gear_device_save(&quadcam_lib::core::DeviceSaveParams {
        id: id.clone(),
        name: Some("Bench radio".into()),
        ..Default::default()
    })
    .unwrap();
    core.gear_backup(&quadcam_lib::core::BackupParams {
        device: Some(id.clone()),
        ..Default::default()
    })
    .unwrap();

    let r = core
        .gear_radio_cli(&RadioCliParams {
            device: Some(id.clone()),
            ..p(RadioCliAction::Verify)
        })
        .unwrap();
    let v = r.verify.unwrap();
    assert_eq!(v.missing, ["MODELS/model02.yml"]);
    assert!(v.differ.is_empty());
    assert!(!v.ok);
    assert_eq!(v.checked, 3, "LOGS are not checked");
    assert_eq!(
        v.not_checked,
        ["SOUNDS/My Pack/a.wav"],
        "a folder the CLI cannot list is not missing"
    );

    // Without a device the one saved radio of the board is used once identify saved a board.
    let r = core.gear_radio_cli(&p(RadioCliAction::Verify)).unwrap();
    assert_eq!(
        r.verify.unwrap().backup.split('/').next(),
        Some(id.as_str())
    );
}
