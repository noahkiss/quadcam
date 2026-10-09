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

fn core(dir: &Path, volumes: Shared<Vec<Volume>>, cards: Vec<CardHw>, usb: Vec<UsbStorage>) -> Core {
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
    let vols = Arc::new(Mutex::new(vec![radio_card(&root, "Secure Digital", "disk42")]));
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
    assert!(root.join("MODELS/._notes.txt").exists(), "not AppleDouble: kept");
    assert!(root.join("MODELS/model01.yml").exists());
}
