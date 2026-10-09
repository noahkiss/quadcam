//! Device events: what changed between two looks at the connected gear.
//!
//! - `connected`: a device appeared;
//! - `identified`: a device on the same link got its id (an FC after its MSP identity);
//! - `unmounted_present`: a volume unmounted but its device is still plugged in, so the
//!   person has not pulled it yet;
//! - `removed`: a device is gone.
//!
//! **Cards are released with `diskutil unmountDisk`, not `eject`.** Tested on a USB reader
//! (2026-10-07): after `eject` the card's disk node disappears and the reader shows no
//! card-present flag, so an ejected card cannot be told from a pulled one. After
//! `unmountDisk` the whole-disk node (`/dev/diskN`) stays while the card is in and goes
//! within seconds of the pull. `Presence::Disk` reads that. Two more cases:
//!
//! - the built-in SD slot: `AppleSDXCSlot` reports `Card Present` in the IORegistry
//!   (`Presence::SdSlotCard`);
//! - a USB device that is more than its storage, such as a DJI air unit, stays on the USB
//!   bus after its volume unmounts (`Presence::Usb`, DJI's vendor id).
//!
//! QuadCam's own "safe to remove" (`disk::safe_remove`) unmounts a card this way. A card
//! another app ejects reads as `removed` at once.
//!
//! `Tracker` keeps the previous look and the devices unmounted but present, and turns each
//! new look into events. The app polls (`detect::POLL`) and runs the on-connect hooks for
//! `connected` and `identified` events that are not `app_initiated` (QuadCam's own mounts
//! and port opens during a job). Events play no cue; a job's end does (`cues`).

use super::model::{Connected, DeviceKind, Link};
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum DeviceEventKind {
    Connected,
    Identified,
    UnmountedPresent,
    Removed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct DeviceEvent {
    pub kind: DeviceEventKind,
    pub device: Connected,
    /// QuadCam's own mount, unmount or port open (a job holds the link): no hook, no cue.
    #[serde(default)]
    pub app_initiated: bool,
}

/// Something the system shows is still plugged in, though it has no volume.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Presence {
    /// A whole-disk node in `/dev` (`disk4`): its media is still in.
    Disk { disk: String },
    /// A card in the built-in SD slot.
    SdSlotCard,
    /// A USB device QuadCam knows by vendor id.
    Usb {
        vid: u16,
        pid: u16,
        #[serde(default)]
        name: Option<String>,
    },
}

/// DJI's USB vendor id (air units and goggles in storage mode).
pub const DJI_VID: u16 = 0x2ca3;

/// USB vendor ids whose devices `presence` reports.
pub const PRESENCE_VIDS: &[u16] = &[DJI_VID];

use super::detect::is_sd_slot;

/// True when a device that lost its volume is still plugged in.
pub fn still_present(c: &Connected, presence: &[Presence]) -> bool {
    let Link::Volume {
        bus_protocol,
        whole_disk,
        ..
    } = &c.link
    else {
        return false;
    };
    if let Some(d) = whole_disk {
        if presence.contains(&Presence::Disk { disk: d.clone() }) {
            return true;
        }
    }
    if is_sd_slot(bus_protocol.as_deref()) {
        return presence.contains(&Presence::SdSlotCard);
    }
    c.kind == DeviceKind::Goggles
        && presence
            .iter()
            .any(|p| matches!(p, Presence::Usb { vid, .. } if *vid == DJI_VID))
}

/// The previous look, and the devices unmounted but still plugged in.
#[derive(Debug, Clone, Default)]
pub struct Tracker {
    pub connected: Vec<Connected>,
    pub unmounted: Vec<Connected>,
}

impl Tracker {
    /// Events from the previous look to `now`. Keeps `now` as the previous look.
    pub fn update(&mut self, now: Vec<Connected>, presence: &[Presence]) -> Vec<DeviceEvent> {
        let mut out = Vec::new();
        let ev = |kind, device: &Connected| DeviceEvent {
            kind,
            device: device.clone(),
            app_initiated: false,
        };
        for c in &now {
            match self.connected.iter().find(|b| b.link == c.link) {
                None => out.push(ev(DeviceEventKind::Connected, c)),
                Some(b) if b.id.is_none() && c.id.is_some() => {
                    out.push(ev(DeviceEventKind::Identified, c))
                }
                Some(_) => {}
            }
        }
        for b in &self.connected {
            if now.iter().any(|c| c.link == b.link) {
                continue;
            }
            if still_present(b, presence) {
                out.push(ev(DeviceEventKind::UnmountedPresent, b));
                self.unmounted.push(b.clone());
            } else {
                out.push(ev(DeviceEventKind::Removed, b));
            }
        }
        // An unmounted device that mounted again, or that is gone now.
        let mut kept = Vec::new();
        for e in std::mem::take(&mut self.unmounted) {
            let back = now
                .iter()
                .any(|c| c.link == e.link || (c.id.is_some() && c.id == e.id));
            if back {
                continue;
            }
            if still_present(&e, presence) {
                kept.push(e);
            } else {
                out.push(ev(DeviceEventKind::Removed, &e));
            }
        }
        self.unmounted = kept;
        self.connected = now;
        out
    }
}

/// What is plugged in without a volume: whole-disk nodes in `/dev`, and the IORegistry
/// (`ioreg -a`). Reads only. None in a process started by cargo unless
/// `QUADCAM_SERIAL=real`, like the serial ports.
pub fn presence() -> Vec<Presence> {
    if !super::serial::serial_enabled(
        std::env::var("QUADCAM_SERIAL").ok().as_deref(),
        std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
    ) {
        return Vec::new();
    }
    let ioreg = |class: &str| {
        std::process::Command::new("/usr/sbin/ioreg")
            .args(["-r", "-a", "-c", class])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| o.stdout)
            .unwrap_or_default()
    };
    let mut out = disk_nodes(std::path::Path::new("/dev"));
    out.extend(parse_sd_slots(&ioreg("AppleSDXCSlot")));
    out.extend(parse_usb(&ioreg("IOUSBHostDevice")));
    out
}

/// Whether a DJI USB device (an air unit, or goggles in storage mode) is attached, from
/// `ioreg`; true when `ioreg` fails. The format guard reads it before it erases a DJI
/// volume. False in a process
/// started by cargo unless `QUADCAM_SERIAL=real`, like `presence`.
pub fn dji_usb_attached() -> bool {
    if !super::serial::serial_enabled(
        std::env::var("QUADCAM_SERIAL").ok().as_deref(),
        std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
    ) {
        return false;
    }
    std::process::Command::new("/usr/sbin/ioreg")
        .args(["-r", "-a", "-c", "IOUSBHostDevice"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        // An ioreg that cannot answer counts as attached: the guard refuses.
        .is_none_or(|o| {
            parse_usb(&o.stdout)
                .iter()
                .any(|p| matches!(p, Presence::Usb { vid, .. } if *vid == DJI_VID))
        })
}

/// On demand only, never in a poll: whether the card behind `disk` is really in. After
/// `unmountDisk`, a microSD pulled out of a full-size adapter (in the built-in slot or a USB
/// reader) leaves the disk node in place: the adapter holds the card-detect switch. `diskutil mountDisk` then says it mounted, but no
/// partition gets a mount point. With the card in, a volume mounts, and this unmounts it
/// again. Refused in a process started by cargo unless `QUADCAM_SERIAL=real`.
pub fn probe_media(disk: &str) -> anyhow::Result<bool> {
    use anyhow::Context;
    if !super::serial::serial_enabled(
        std::env::var("QUADCAM_SERIAL").ok().as_deref(),
        std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
    ) {
        return Err(super::model::Refusal::new(
            super::model::RefusalCode::Disabled,
            "Disk probes are off in this process; set QUADCAM_SERIAL=real to use them.",
        )
        .into());
    }
    let disk = crate::disk::whole_disk_of(disk);
    if !disk.starts_with("disk") || !disk[4..].bytes().all(|b| b.is_ascii_digit()) {
        anyhow::bail!("{disk:?} is not a whole disk");
    }
    let run = |args: &[&str]| {
        std::process::Command::new("/usr/sbin/diskutil")
            .args(args)
            .output()
            .with_context(|| format!("running diskutil {}", args.join(" ")))
    };
    run(&["mountDisk", &disk])?;
    let list = run(&["list", "-plist", &disk])?;
    let present = media_mounted(&list.stdout);
    if present {
        run(&["unmountDisk", &disk])?;
    }
    Ok(present)
}

/// True when `diskutil list -plist <disk>` shows a partition with a mount point.
pub fn media_mounted(list_plist: &[u8]) -> bool {
    let Ok(v) = plist::Value::from_reader(std::io::Cursor::new(list_plist)) else {
        return false;
    };
    let parts = v
        .as_dictionary()
        .and_then(|d| d.get("AllDisksAndPartitions"))
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
        .filter_map(|d| d.as_dictionary())
        .flat_map(|d| {
            d.get("Partitions")
                .and_then(|p| p.as_array())
                .cloned()
                .unwrap_or_default()
        });
    parts.into_iter().any(|p| {
        p.as_dictionary()
            .and_then(|d| d.get("MountPoint"))
            .and_then(|m| m.as_string())
            .is_some_and(|m| !m.is_empty())
    })
}

/// `Presence::Disk` for each whole-disk node (`diskN`) in `dev`.
pub fn disk_nodes(dev: &std::path::Path) -> Vec<Presence> {
    let Ok(rd) = std::fs::read_dir(dev) else {
        return Vec::new();
    };
    let mut out: Vec<Presence> = rd
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| {
            n.strip_prefix("disk")
                .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(|disk| Presence::Disk { disk })
        .collect();
    out.sort_by_key(|p| format!("{p:?}"));
    out
}

fn entries(plist_bytes: &[u8]) -> Vec<plist::Dictionary> {
    if plist_bytes.is_empty() {
        return Vec::new();
    }
    plist::from_bytes::<Vec<plist::Dictionary>>(plist_bytes).unwrap_or_default()
}

/// `SdSlotCard` when a built-in SD slot reports `Card Present`.
pub fn parse_sd_slots(plist_bytes: &[u8]) -> Vec<Presence> {
    entries(plist_bytes)
        .iter()
        .filter(|d| d.get("Card Present").and_then(|v| v.as_boolean()) == Some(true))
        .map(|_| Presence::SdSlotCard)
        .collect()
}

/// USB devices with a vendor id in `PRESENCE_VIDS`.
pub fn parse_usb(plist_bytes: &[u8]) -> Vec<Presence> {
    let int = |d: &plist::Dictionary, k: &str| {
        d.get(k)
            .and_then(|v| v.as_unsigned_integer())
            .and_then(|n| u16::try_from(n).ok())
    };
    entries(plist_bytes)
        .iter()
        .filter_map(|d| {
            let vid = int(d, "idVendor")?;
            PRESENCE_VIDS.contains(&vid).then(|| Presence::Usb {
                vid,
                pid: int(d, "idProduct").unwrap_or(0),
                name: d
                    .get("USB Product Name")
                    .and_then(|v| v.as_string())
                    .map(str::to_string),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gear::model::Identity;

    fn vol(mount: &str, kind: DeviceKind, bus: &str) -> Connected {
        Connected {
            id: Some(format!("{}-1", kind.id_prefix())),
            kind,
            link: Link::Volume {
                mount: mount.into(),
                volume_uuid: None,
                bus_protocol: Some(bus.into()),
                whole_disk: Some("disk9".into()),
            },
            identity: Identity::default(),
            device: None,
            usb: None,
            also: Vec::new(),
        }
    }

    fn port(id: Option<&str>) -> Connected {
        Connected {
            id: id.map(str::to_string),
            kind: DeviceKind::Fc,
            link: Link::Serial {
                port: "/dev/cu.fake".into(),
                vid: 0x0483,
                pid: 0x5740,
                product: None,
            },
            identity: Identity::default(),
            device: None,
            usb: None,
            also: Vec::new(),
        }
    }

    fn kinds(e: &[DeviceEvent]) -> Vec<DeviceEventKind> {
        e.iter().map(|e| e.kind).collect()
    }

    #[test]
    fn connect_identify_disconnect() {
        let mut t = Tracker::default();
        assert_eq!(
            kinds(&t.update(vec![port(None)], &[])),
            [DeviceEventKind::Connected]
        );
        assert!(t.update(vec![port(None)], &[]).is_empty());
        assert_eq!(
            kinds(&t.update(vec![port(Some("fc-1"))], &[])),
            [DeviceEventKind::Identified]
        );
        assert_eq!(kinds(&t.update(vec![], &[])), [DeviceEventKind::Removed]);
    }

    #[test]
    fn an_unmounted_card_stays_until_its_disk_node_goes() {
        let card = vol("/Volumes/CARD", DeviceKind::DvrCard, "USB");
        let node = Presence::Disk {
            disk: "disk9".into(),
        };
        let mut t = Tracker::default();
        t.update(vec![card.clone()], std::slice::from_ref(&node));
        let e = t.update(vec![], std::slice::from_ref(&node));
        assert_eq!(kinds(&e), [DeviceEventKind::UnmountedPresent]);
        assert!(t.update(vec![], &[node]).is_empty(), "still in");
        assert_eq!(kinds(&t.update(vec![], &[])), [DeviceEventKind::Removed]);
    }

    #[test]
    fn disk_nodes_lists_whole_disks() {
        let d = tempfile::tempdir().unwrap();
        for n in ["disk4", "disk4s1", "disk12", "diskX", "rdisk4", "null"] {
            std::fs::write(d.path().join(n), b"").unwrap();
        }
        let got = disk_nodes(d.path());
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got.contains(&Presence::Disk {
            disk: "disk4".into()
        }));
        assert!(got.contains(&Presence::Disk {
            disk: "disk12".into()
        }));
    }

    #[test]
    fn a_card_unmounted_in_the_sd_slot_stays_until_pulled() {
        let card = vol("/Volumes/DVR", DeviceKind::DvrCard, "Secure Digital");
        let mut t = Tracker::default();
        t.update(vec![card.clone()], &[Presence::SdSlotCard]);
        let e = t.update(vec![], &[Presence::SdSlotCard]);
        assert_eq!(kinds(&e), [DeviceEventKind::UnmountedPresent]);
        assert!(t.update(vec![], &[Presence::SdSlotCard]).is_empty());
        assert_eq!(t.unmounted.len(), 1);
        let e = t.update(vec![], &[]);
        assert_eq!(kinds(&e), [DeviceEventKind::Removed]);
        assert!(t.unmounted.is_empty());
        // Mounted again instead of pulled: no disconnect.
        t.update(vec![card.clone()], &[Presence::SdSlotCard]);
        t.update(vec![], &[Presence::SdSlotCard]);
        let e = t.update(vec![card], &[Presence::SdSlotCard]);
        assert_eq!(kinds(&e), [DeviceEventKind::Connected]);
        assert!(t.unmounted.is_empty());
    }

    #[test]
    fn an_ejected_reader_card_and_a_dji_unit() {
        let reader = vol("/Volumes/CARD", DeviceKind::DvrCard, "USB");
        let mut t = Tracker::default();
        t.update(vec![reader], &[]);
        assert_eq!(
            kinds(&t.update(vec![], &[])),
            [DeviceEventKind::Removed],
            "an ejected card in a USB reader leaves no disk node"
        );
        let dji = vol("/Volumes/DJI", DeviceKind::Goggles, "USB");
        let unit = Presence::Usb {
            vid: DJI_VID,
            pid: 0x0020,
            name: None,
        };
        t.update(vec![dji], std::slice::from_ref(&unit));
        assert_eq!(
            kinds(&t.update(vec![], &[unit])),
            [DeviceEventKind::UnmountedPresent]
        );
        assert_eq!(kinds(&t.update(vec![], &[])), [DeviceEventKind::Removed]);
    }

    #[test]
    fn ioreg_plists() {
        let slot = br#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><array>
            <dict><key>Card Present</key><true/><key>Description</key><string>Port-SD Card@1</string></dict>
            </array></plist>"#;
        assert_eq!(parse_sd_slots(slot), [Presence::SdSlotCard]);
        let empty = br#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><array>
            <dict><key>Card Present</key><false/></dict></array></plist>"#;
        assert!(parse_sd_slots(empty).is_empty());
        assert!(parse_sd_slots(b"").is_empty());
        let usb = br#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><array>
            <dict><key>idVendor</key><integer>11427</integer><key>idProduct</key><integer>32</integer><key>USB Product Name</key><string>Air unit</string></dict>
            <dict><key>idVendor</key><integer>1452</integer><key>idProduct</key><integer>1</integer></dict>
            </array></plist>"#;
        assert_eq!(
            parse_usb(usb),
            [Presence::Usb {
                vid: DJI_VID,
                pid: 32,
                name: Some("Air unit".into())
            }]
        );
    }

    #[test]
    fn no_presence_under_cargo() {
        if std::env::var("QUADCAM_SERIAL").as_deref() != Ok("real") {
            assert!(presence().is_empty());
            let e = probe_media("disk4").unwrap_err();
            assert!(format!("{e:#}").starts_with("Refused"), "{e:#}");
        }
    }

    #[test]
    fn mounted_media_from_diskutil_list() {
        let list = |part: &str| {
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict>
                <key>AllDisksAndPartitions</key><array><dict>
                  <key>DeviceIdentifier</key><string>disk4</string>
                  <key>Partitions</key><array><dict><key>DeviceIdentifier</key><string>disk4s1</string>{part}</dict></array>
                </dict></array></dict></plist>"#
            )
        };
        assert!(media_mounted(
            list("<key>MountPoint</key><string>/Volumes/DVR</string>").as_bytes()
        ));
        assert!(
            !media_mounted(list("").as_bytes()),
            "no card behind the adapter"
        );
        assert!(!media_mounted(b"junk"));
    }
}
