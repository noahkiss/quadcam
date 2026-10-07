//! Finds the gear plugged in now, without opening a port or writing anything:
//!
//! - EdgeTX radios in USB Storage mode: a removable volume `disk::looks_like_radio` takes,
//!   with `board` and `semver` read from `RADIO/radio.yml`;
//! - goggles and DVR cards: a card volume, by its source (DJI is goggles, analog a DVR);
//! - USB serial ports by VID and PID (`SERIAL_IDS`);
//! - DFU devices (a radio in its bootloader), once the DFU module lists them.
//!
//! `detect` is a pure function of those lists, so tests feed it synthetic volumes and fake
//! ports. The app calls it every `POLL` and sends `gear-changed` when the answer changes.

use super::model::{device_id, Connected, DeviceKind, Identity, Link, UsbInfo};
use super::serial::PortInfo;
use crate::disk::Volume;
use crate::sources::SourceKind;
use std::path::Path;
use std::time::Duration;

/// How often the app looks for gear.
pub const POLL: Duration = Duration::from_secs(2);

/// A USB DFU device as listed (filled in by the DFU module).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DfuInfo {
    pub vid: u16,
    pub pid: u16,
}

/// The STM32 ROM bootloader in DFU mode: a radio being flashed.
pub const STM32_DFU: (u16, u16) = (0x0483, 0xdf11);

/// A serial port's USB id and what it most likely is. Identification (MSP, the CLI)
/// confirms it before anything is written.
pub struct SerialId {
    pub vid: u16,
    pub pid: u16,
    pub kind: DeviceKind,
    pub what: &'static str,
}

/// USB ids of the serial ports Gear lists. Ports with other ids are not gear.
pub const SERIAL_IDS: &[SerialId] = &[
    SerialId {
        vid: 0x0483,
        pid: 0x5740,
        kind: DeviceKind::Fc,
        what: "STM32 virtual COM port",
    },
    SerialId {
        vid: 0x2e3c,
        pid: 0x5740,
        kind: DeviceKind::Fc,
        what: "AT32 virtual COM port",
    },
    SerialId {
        vid: 0x10c4,
        pid: 0xea60,
        kind: DeviceKind::ElrsTx,
        what: "CP210x USB-UART bridge",
    },
    SerialId {
        vid: 0x1a86,
        pid: 0x7523,
        kind: DeviceKind::ElrsTx,
        what: "CH340 USB-UART bridge",
    },
    SerialId {
        vid: 0x1a86,
        pid: 0x55d4,
        kind: DeviceKind::ElrsTx,
        what: "CH9102 USB-UART bridge",
    },
];

/// What a serial port most likely is. An STM32 port whose USB product names EdgeTX is the
/// radio's own serial port, not an FC.
pub fn classify_port(p: &PortInfo) -> Option<DeviceKind> {
    let id = SERIAL_IDS
        .iter()
        .find(|s| s.vid == p.vid && s.pid == p.pid)?;
    if id.kind == DeviceKind::Fc && is_radio_usb(p.manufacturer.as_deref(), p.product.as_deref()) {
        return Some(DeviceKind::Radio);
    }
    Some(id.kind)
}

/// Words in a USB vendor or product name that mean an EdgeTX radio. A 2.12 radio's serial
/// port says only "<Brand> <Model> Serial Port"; its vendor string says OpenTX.
pub const RADIO_USB_WORDS: &[&str] = &[
    "edgetx",
    "opentx",
    "radiomaster",
    "jumper",
    "frsky",
    "flysky",
    "betafpv lite radio",
];

/// True when a USB device's vendor or product names an EdgeTX radio.
pub fn is_radio_usb(vendor: Option<&str>, product: Option<&str>) -> bool {
    [vendor, product].iter().flatten().any(|s| {
        let s = s.to_ascii_lowercase();
        RADIO_USB_WORDS.iter().any(|w| s.contains(w))
    })
}

/// A USB device with the BSD disks below it, as `ioreg` lists them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsbStorage {
    pub info: UsbInfo,
    /// `disk13`, `disk13s1`.
    pub disks: Vec<String>,
}

/// USB devices in `ioreg -a -r -c IOUSBHostDevice -l` output, each with the BSD names of
/// the disks below it.
pub fn parse_ioreg_usb(xml: &[u8]) -> Vec<UsbStorage> {
    fn disks(v: &plist::Value, out: &mut Vec<String>) {
        if let Some(d) = v.as_dictionary() {
            if let Some(n) = d.get("BSD Name").and_then(|x| x.as_string()) {
                out.push(n.to_string());
            }
            if let Some(c) = d.get("IORegistryEntryChildren").and_then(|x| x.as_array()) {
                c.iter().for_each(|x| disks(x, out));
            }
        }
    }
    let Ok(v) = plist::Value::from_reader(std::io::Cursor::new(xml)) else {
        return Vec::new();
    };
    let list = match v {
        plist::Value::Array(a) => a,
        d @ plist::Value::Dictionary(_) => vec![d],
        _ => return Vec::new(),
    };
    list.iter()
        .filter_map(|dev| {
            let d = dev.as_dictionary()?;
            let n = |k: &str| d.get(k).and_then(|x| x.as_unsigned_integer());
            let s = |k: &str| d.get(k).and_then(|x| x.as_string()).map(str::to_string);
            let mut found = Vec::new();
            disks(dev, &mut found);
            Some(UsbStorage {
                info: UsbInfo {
                    vid: n("idVendor")? as u16,
                    pid: n("idProduct")? as u16,
                    vendor: s("USB Vendor Name"),
                    product: s("USB Product Name"),
                    serial: s("USB Serial Number").or_else(|| s("kUSBSerialNumberString")),
                    version: n("bcdDevice")
                        .map(|b| format!("{:x}.{:02x}", (b >> 8) & 0xff, b & 0xff)),
                },
                disks: found,
            })
        })
        .collect()
}

/// USB devices now, with their disks. Reads only; none in a process started by cargo
/// unless `QUADCAM_SERIAL=real`.
pub fn usb_storage() -> Vec<UsbStorage> {
    if !super::serial::serial_enabled(
        std::env::var("QUADCAM_SERIAL").ok().as_deref(),
        std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
    ) {
        return Vec::new();
    }
    std::process::Command::new("/usr/sbin/ioreg")
        .args(["-a", "-r", "-c", "IOUSBHostDevice", "-l"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_ioreg_usb(&o.stdout))
        .unwrap_or_default()
}

/// `board` and `semver` from a card's `RADIO/radio.yml`, read by the EdgeTX engine.
pub fn radio_identity(mount: &Path) -> Identity {
    match std::fs::read(mount.join(super::edgetx::card::RADIO_FILE)) {
        Ok(b) => super::edgetx::card::identity_from_radio_yml(&b),
        Err(_) => Identity {
            firmware: Some("EdgeTX".into()),
            ..Default::default()
        },
    }
}

/// A card's hardware identity, as the built-in SD card reader reports it
/// (`system_profiler SPCardReaderDataType`). USB card readers do not report it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CardHw {
    /// The card's whole disk (`disk4`).
    pub disk: String,
    pub manufacturer_id: Option<String>,
    pub serial: Option<String>,
    pub product: Option<String>,
    pub manufacturing_date: Option<String>,
}

impl CardHw {
    /// The raw value a card's id hashes: manufacturer id and serial number.
    pub fn id_source(&self) -> Option<String> {
        let serial = self.serial.as_deref().filter(|s| !s.trim().is_empty())?;
        Some(format!(
            "sdcard:{}:{serial}",
            self.manufacturer_id.as_deref().unwrap_or("")
        ))
    }
}

/// True for the built-in SD slot's protocol.
pub fn is_sd_slot(bus: Option<&str>) -> bool {
    bus.is_some_and(|b| b.eq_ignore_ascii_case("Secure Digital"))
}

/// The cards in `system_profiler -xml SPCardReaderDataType` output: every dict with a
/// `bsd_name` and `spcardreader_card_*` keys, at any depth.
pub fn parse_card_reader(xml: &[u8]) -> Vec<CardHw> {
    fn walk(v: &plist::Value, out: &mut Vec<CardHw>) {
        match v {
            plist::Value::Dictionary(d) => {
                let s = |k: &str| d.get(k).and_then(|v| v.as_string()).map(str::to_string);
                if let (Some(disk), true) = (
                    s("bsd_name"),
                    d.keys().any(|k| k.starts_with("spcardreader_card_")),
                ) {
                    out.push(CardHw {
                        disk,
                        manufacturer_id: s("spcardreader_card_manufacturer-id"),
                        serial: s("spcardreader_card_serialnumber"),
                        product: s("spcardreader_card_productname"),
                        manufacturing_date: s("spcardreader_card_manufacturing_date"),
                    });
                }
                for x in d.values() {
                    walk(x, out);
                }
            }
            plist::Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Ok(v) = plist::Value::from_reader(std::io::Cursor::new(xml)) {
        walk(&v, &mut out);
    }
    out
}

/// The cards in the built-in reader now. Reads only; none in a process started by cargo
/// unless `QUADCAM_SERIAL=real`.
pub fn card_reader() -> Vec<CardHw> {
    if !super::serial::serial_enabled(
        std::env::var("QUADCAM_SERIAL").ok().as_deref(),
        std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
    ) {
        return Vec::new();
    }
    std::process::Command::new("/usr/sbin/system_profiler")
        .args(["-xml", "SPCardReaderDataType"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_card_reader(&o.stdout))
        .unwrap_or_default()
}

/// The gear in these volumes, ports and DFU devices. A card in the built-in SD slot gets its
/// id from its hardware serial (`cards`); other volumes from their volume UUID. Devices
/// come back without their saved record (`Connected::device`); the caller adds it from the
/// store.
pub fn detect(
    volumes: &[Volume],
    ports: &[PortInfo],
    dfu: &[DfuInfo],
    cards: &[CardHw],
) -> Vec<Connected> {
    detect_all(volumes, ports, dfu, cards, &[])
}

/// `detect`, with the USB devices the volumes sit on. A volume on an EdgeTX radio's USB
/// device is the radio in USB Storage mode (`Connected::usb`), even when its card lacks
/// the radio folders. A radio card's id comes from its hardware serial (built-in slot),
/// else QuadCam's marker file, else the volume UUID.
pub fn detect_all(
    volumes: &[Volume],
    ports: &[PortInfo],
    dfu: &[DfuInfo],
    cards: &[CardHw],
    usb: &[UsbStorage],
) -> Vec<Connected> {
    let mut out = Vec::new();
    for v in volumes {
        let whole = crate::disk::whole_disk_of(&v.info.parent_whole_disk);
        let radio_usb = usb
            .iter()
            .find(|u| !whole.is_empty() && u.disks.iter().any(|d| d == &whole))
            .filter(|u| is_radio_usb(u.info.vendor.as_deref(), u.info.product.as_deref()))
            .map(|u| u.info.clone());
        let kind = if v.is_radio || radio_usb.is_some() {
            DeviceKind::Radio
        } else if v.is_card {
            match v.source {
                Some(SourceKind::Dji) => DeviceKind::Goggles,
                _ => DeviceKind::DvrCard,
            }
        } else {
            continue;
        };
        let identity = if kind == DeviceKind::Radio {
            radio_identity(&v.mount)
        } else {
            Identity::default()
        };
        let uuid = v.info.volume_uuid.clone();
        let hw = is_sd_slot(v.info.bus_protocol.as_deref())
            .then(|| cards.iter().find(|c| c.disk == whole))
            .flatten()
            .and_then(CardHw::id_source);
        let marker = (kind == DeviceKind::Radio)
            .then(|| super::edgetx::card::read_marker(&v.mount))
            .flatten();
        out.push(Connected {
            id: hw
                .or(marker)
                .or_else(|| uuid.clone())
                .map(|raw| device_id(kind, &raw)),
            kind,
            link: Link::Volume {
                mount: v.mount.clone(),
                volume_uuid: uuid,
                bus_protocol: v.info.bus_protocol.clone(),
                whole_disk: Some(crate::disk::whole_disk_of(&v.info.parent_whole_disk))
                    .filter(|d| !d.is_empty()),
            },
            identity,
            device: None,
            usb: radio_usb,
        });
    }
    for p in ports {
        let Some(kind) = classify_port(p) else {
            continue;
        };
        out.push(Connected {
            id: None,
            kind,
            link: Link::Serial {
                port: p.port.clone(),
                vid: p.vid,
                pid: p.pid,
                product: p.product.clone(),
            },
            identity: Identity::default(),
            device: None,
            usb: None,
        });
    }
    for d in dfu {
        if (d.vid, d.pid) == STM32_DFU {
            out.push(Connected {
                id: None,
                kind: DeviceKind::Radio,
                link: Link::Dfu {
                    vid: d.vid,
                    pid: d.pid,
                },
                identity: Identity::default(),
                device: None,
                usb: None,
            });
        }
    }
    out
}

/// DFU devices plugged in now. Empty until the DFU module (`gear/dfu.rs`) lists them.
pub fn dfu_devices() -> Vec<DfuInfo> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::DiskInfo;

    pub(crate) fn radio_volume(root: &Path, uuid: &str) -> Volume {
        std::fs::create_dir_all(root.join("RADIO")).unwrap();
        std::fs::create_dir_all(root.join("MODELS")).unwrap();
        std::fs::create_dir_all(root.join("LOGS")).unwrap();
        std::fs::write(
            root.join("RADIO/radio.yml"),
            "checksum: 0\r\nsemver: 2.12.4\r\nboard: pocket\r\ncurrModel: 0\r\nsomeList:\r\n  board: nested\r\n",
        )
        .unwrap();
        Volume {
            mount: root.to_path_buf(),
            info: DiskInfo {
                volume_uuid: Some(uuid.into()),
                removable: true,
                ..Default::default()
            },
            is_card: false,
            source: None,
            is_radio: crate::disk::looks_like_radio(root),
            warnings: vec![],
        }
    }

    #[test]
    fn lists_a_radio_volume_and_a_fake_port() {
        let d = tempfile::tempdir().unwrap();
        let radio = radio_volume(d.path(), "UUID-RADIO");
        assert!(radio.is_radio);
        let dvr = Volume {
            mount: "/Volumes/DVR".into(),
            info: DiskInfo {
                volume_uuid: Some("UUID-DVR".into()),
                ..Default::default()
            },
            is_card: true,
            source: Some(SourceKind::Analog),
            is_radio: false,
            warnings: vec![],
        };
        let other = Volume {
            is_card: false,
            ..dvr.clone()
        };
        let ports = vec![
            PortInfo {
                port: "/dev/cu.usbmodemFAKE1".into(),
                vid: 0x0483,
                pid: 0x5740,
                ..Default::default()
            },
            PortInfo {
                port: "/dev/cu.usbmodemFAKE2".into(),
                vid: 0x0483,
                pid: 0x5740,
                product: Some("EdgeTX Serial".into()),
                ..Default::default()
            },
            PortInfo {
                port: "/dev/cu.Bluetooth".into(),
                vid: 0x05ac,
                pid: 0x0001,
                ..Default::default()
            },
        ];
        let found = detect(
            &[radio, dvr, other],
            &ports,
            &[DfuInfo {
                vid: 0x0483,
                pid: 0xdf11,
            }],
            &[],
        );
        let kinds: Vec<DeviceKind> = found.iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            vec![
                DeviceKind::Radio,
                DeviceKind::DvrCard,
                DeviceKind::Fc,
                DeviceKind::Radio,
                DeviceKind::Radio
            ]
        );
        let r = &found[0];
        assert_eq!(r.identity.board.as_deref(), Some("pocket"));
        assert_eq!(r.identity.version.as_deref(), Some("2.12.4"));
        assert_eq!(r.identity.firmware.as_deref(), Some("EdgeTX"));
        assert_eq!(
            r.id.as_deref(),
            Some(device_id(DeviceKind::Radio, "UUID-RADIO").as_str())
        );
        assert!(
            matches!(&found[2].link, Link::Serial { port, .. } if port == "/dev/cu.usbmodemFAKE1")
        );
        assert_eq!(found[2].id, None, "an FC gets its id once identified");
        assert!(matches!(found[4].link, Link::Dfu { .. }));
    }

    #[test]
    fn a_card_in_the_sd_slot_is_known_by_its_serial() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><array><dict>
            <key>_items</key><array><dict>
              <key>_items</key><array><dict>
                <key>_name</key><string>SDHC Card</string>
                <key>bsd_name</key><string>disk4</string>
                <key>spcardreader_card_manufacturer-id</key><string>0x99</string>
                <key>spcardreader_card_serialnumber</key><string>0x0000TEST</string>
                <key>spcardreader_card_productname</key><string>TEST32G</string>
                <key>spcardreader_card_manufacturing_date</key><string>2026-01</string>
                <key>volumes</key><array><dict><key>bsd_name</key><string>disk4s1</string></dict></array>
              </dict></array>
              <key>spcardreader_vendor-id</key><string>0x17a0</string>
            </dict></array></dict></array></plist>"#;
        let cards = parse_card_reader(xml);
        assert_eq!(cards.len(), 1, "{cards:?}");
        assert_eq!(cards[0].disk, "disk4");
        assert_eq!(cards[0].product.as_deref(), Some("TEST32G"));
        let vol = |uuid: &str, bus: &str| Volume {
            mount: "/Volumes/DVR".into(),
            info: DiskInfo {
                volume_uuid: Some(uuid.into()),
                parent_whole_disk: "disk4".into(),
                bus_protocol: Some(bus.into()),
                ..Default::default()
            },
            is_card: true,
            source: Some(SourceKind::Analog),
            is_radio: false,
            warnings: vec![],
        };
        // A format changes the UUID; the slot's serial keeps the id.
        let a = detect(&[vol("U1", "Secure Digital")], &[], &[], &cards);
        let b = detect(&[vol("U2", "Secure Digital")], &[], &[], &cards);
        assert_eq!(a[0].id, b[0].id);
        assert_eq!(
            a[0].id.as_deref(),
            Some(device_id(DeviceKind::DvrCard, "sdcard:0x99:0x0000TEST").as_str())
        );
        // A USB reader: the UUID.
        let c = detect(&[vol("U1", "USB")], &[], &[], &cards);
        assert_eq!(
            c[0].id.as_deref(),
            Some(device_id(DeviceKind::DvrCard, "U1").as_str())
        );
        assert!(parse_card_reader(b"not a plist").is_empty());
        if std::env::var("QUADCAM_SERIAL").as_deref() != Ok("real") {
            assert!(card_reader().is_empty(), "none under cargo");
        }
    }

    #[test]
    fn a_radio_without_radio_yml_still_shows() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("LOGS")).unwrap();
        std::fs::create_dir_all(d.path().join("MODELS")).unwrap();
        let id = radio_identity(d.path());
        assert_eq!(id.board, None);
        assert_eq!(id.firmware.as_deref(), Some("EdgeTX"));
    }
}
