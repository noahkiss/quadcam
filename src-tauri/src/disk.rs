//! Volumes and disks: `diskutil info`, card detection, and the guarded FAT32 format.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Largest disk the format step accepts. Analog DVRs usually take cards up to 32 GB.
pub const MAX_FORMAT_BYTES: u64 = 64_000_000_000;
/// Cards over this size ship as exFAT; analog DVRs want FAT32.
pub const FAT32_CARD_BYTES: u64 = 32 * 1_000_000_000 + 2_000_000_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct DiskInfo {
    pub device_identifier: String,
    pub parent_whole_disk: String,
    pub internal: bool,
    pub removable: bool,
    pub ejectable: bool,
    pub total_size: u64,
    pub volume_uuid: Option<String>,
    pub volume_name: Option<String>,
    pub mount_point: Option<String>,
    /// `FilesystemPersonality`, or `FilesystemName` where FSKit leaves the former empty.
    pub filesystem: Option<String>,
    pub media_name: Option<String>,
    pub bus_protocol: Option<String>,
}

impl DiskInfo {
    pub fn is_fat32(&self) -> bool {
        self.filesystem
            .as_deref()
            .is_some_and(|f| f.to_uppercase().contains("FAT32"))
    }
}

fn run_diskutil_plist(args: &[&str]) -> Result<plist::Dictionary> {
    let out = Command::new("/usr/sbin/diskutil")
        .args(args)
        .output()
        .context("running diskutil")?;
    if !out.status.success() {
        bail!(
            "diskutil {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let v: plist::Value = plist::from_bytes(&out.stdout).context("parsing diskutil plist")?;
    v.into_dictionary()
        .ok_or_else(|| anyhow!("diskutil returned no dictionary"))
}

/// `diskutil info -plist <target>`; `target` is a mount point, `/dev/diskN` or `diskN`.
pub fn info(target: &str) -> Result<DiskInfo> {
    let d = run_diskutil_plist(&["info", "-plist", target])?;
    Ok(parse_info(&d))
}

pub fn parse_info(d: &plist::Dictionary) -> DiskInfo {
    let s = |k: &str| {
        d.get(k)
            .and_then(|v| v.as_string())
            .map(str::to_string)
            .filter(|s| !s.is_empty())
    };
    let b = |k: &str| d.get(k).and_then(|v| v.as_boolean()).unwrap_or(false);
    let n = |k: &str| d.get(k).and_then(|v| v.as_unsigned_integer()).unwrap_or(0);
    DiskInfo {
        device_identifier: s("DeviceIdentifier").unwrap_or_default(),
        parent_whole_disk: s("ParentWholeDisk").unwrap_or_default(),
        // Missing Internal means diskutil could not tell; treat as internal (conservative).
        internal: d
            .get("Internal")
            .and_then(|v| v.as_boolean())
            .unwrap_or(true),
        removable: b("RemovableMedia") || b("Removable"),
        ejectable: b("Ejectable"),
        total_size: {
            let t = n("TotalSize");
            if t > 0 {
                t
            } else {
                n("Size")
            }
        },
        volume_uuid: s("VolumeUUID"),
        volume_name: s("VolumeName"),
        mount_point: s("MountPoint"),
        filesystem: s("FilesystemPersonality").or_else(|| s("FilesystemName")),
        media_name: s("MediaName"),
        bus_protocol: s("BusProtocol"),
    }
}

/// Whole disks that hold the running system: the boot volume's whole disk and, for an APFS
/// container, its physical stores.
pub fn boot_whole_disks() -> Vec<String> {
    let mut out = vec!["disk0".to_string()];
    if let Ok(d) = run_diskutil_plist(&["info", "-plist", "/"]) {
        if let Some(p) = d.get("ParentWholeDisk").and_then(|v| v.as_string()) {
            out.push(p.to_string());
        }
        if let Some(stores) = d.get("APFSPhysicalStores").and_then(|v| v.as_array()) {
            for s in stores {
                if let Some(id) = s
                    .as_dictionary()
                    .and_then(|x| x.get("APFSPhysicalStore"))
                    .and_then(|v| v.as_string())
                {
                    out.push(whole_disk_of(id));
                }
            }
        }
    }
    out
}

/// `disk4s1` -> `disk4`.
pub fn whole_disk_of(id: &str) -> String {
    let id = id.trim_start_matches("/dev/");
    match id[4.min(id.len())..].find('s') {
        Some(i) if id.starts_with("disk") => id[..4 + i].to_string(),
        _ => id.to_string(),
    }
}

/// A mounted volume the app may care about.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct Volume {
    pub mount: PathBuf,
    pub info: DiskInfo,
    /// Removable and holds DVR clips.
    pub is_card: bool,
    /// Holds `LOGS/` next to `MODELS/` or `RADIO/`: an EdgeTX radio in USB Storage mode.
    pub is_radio: bool,
    pub warnings: Vec<String>,
}

pub fn is_removable(info: &DiskInfo) -> bool {
    !info.internal && (info.removable || info.ejectable)
}

pub fn looks_like_radio(mount: &Path) -> bool {
    mount.join("LOGS").is_dir() && (mount.join("MODELS").is_dir() || mount.join("RADIO").is_dir())
}

pub fn card_warnings(info: &DiskInfo) -> Vec<String> {
    let mut w = Vec::new();
    if !info.is_fat32() {
        w.push(format!(
            "Card is {}, not FAT32. Most analog DVRs need a FAT32 card of 32 GB or less. Import works; you can format it to FAT32 at the end.",
            info.filesystem.as_deref().unwrap_or("an unknown format")
        ));
    }
    if info.total_size > FAT32_CARD_BYTES {
        w.push("Card is larger than 32 GB. Most analog DVRs take cards up to 32 GB.".into());
    }
    w
}

/// Inspects one mount point. Returns None when diskutil cannot describe it.
pub fn probe_volume(mount: &Path) -> Option<Volume> {
    let info = info(&mount.to_string_lossy()).ok()?;
    let removable = is_removable(&info);
    let is_radio = removable && looks_like_radio(mount);
    let is_card = removable && !is_radio && crate::scan::has_clips(mount);
    let warnings = if is_card {
        card_warnings(&info)
    } else {
        Vec::new()
    };
    Some(Volume {
        mount: mount.to_path_buf(),
        info,
        is_card,
        is_radio,
        warnings,
    })
}

/// Every removable volume under `/Volumes`.
pub fn list_volumes() -> Vec<Volume> {
    let Ok(rd) = std::fs::read_dir("/Volumes") else {
        return Vec::new();
    };
    let mut v: Vec<Volume> = rd
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| probe_volume(&e.path()))
        .filter(|v| is_removable(&v.info))
        .collect();
    v.sort_by(|a, b| a.mount.cmp(&b.mount));
    v
}

/// The card the clips were read from, recorded at stage time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct CardIdentity {
    pub device_identifier: String,
    pub whole_disk: String,
    pub volume_uuid: Option<String>,
    pub volume_name: Option<String>,
    pub total_size: u64,
    pub media_name: Option<String>,
}

impl CardIdentity {
    pub fn from_info(info: &DiskInfo) -> Self {
        Self {
            device_identifier: info.device_identifier.clone(),
            whole_disk: whole_disk_of(&info.parent_whole_disk),
            volume_uuid: info.volume_uuid.clone(),
            volume_name: info.volume_name.clone(),
            total_size: info.total_size,
            media_name: info.media_name.clone(),
        }
    }
}

/// Volume name the format step uses when none is given.
pub const DEFAULT_LABEL: &str = "DVR";

/// FAT32 volume label: `DVR` by default, upper case, at most 11 characters.
pub fn fat_label(name: &str) -> Result<String> {
    let l: String = name.trim().to_uppercase();
    let l = if l.is_empty() {
        DEFAULT_LABEL.to_string()
    } else {
        l
    };
    if l.chars().count() > 11 {
        bail!("Volume name must be 11 characters or fewer");
    }
    if !l
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        bail!("Volume name may hold only A-Z, 0-9, _ and -");
    }
    Ok(l)
}

/// Every format guard. `volume` is the card's volume as diskutil sees it now,
/// `whole` is its whole disk now, `expected` is the card recorded when the clips were staged.
pub fn check_format_guards(
    expected: &CardIdentity,
    volume: &DiskInfo,
    whole: &DiskInfo,
    boot_disks: &[String],
) -> Result<()> {
    let whole_id = whole_disk_of(&whole.device_identifier);
    if whole_id != expected.whole_disk || whole_disk_of(&volume.parent_whole_disk) != whole_id {
        bail!(
            "Refused: the card at {whole_id} is not the card the clips were read from ({}).",
            expected.whole_disk
        );
    }
    if volume.device_identifier != expected.device_identifier {
        bail!(
            "Refused: device changed ({} is now {}).",
            expected.device_identifier,
            volume.device_identifier
        );
    }
    if expected.volume_uuid.is_none() || volume.volume_uuid != expected.volume_uuid {
        bail!("Refused: volume UUID does not match the card the clips were read from.");
    }
    if whole_id == "disk0" || boot_disks.iter().any(|b| b == &whole_id) {
        bail!("Refused: {whole_id} is the boot disk.");
    }
    for d in [volume, whole] {
        if d.internal {
            bail!("Refused: {} is an internal disk.", d.device_identifier);
        }
        if !(d.removable || d.ejectable) {
            bail!(
                "Refused: {} is not removable or ejectable.",
                d.device_identifier
            );
        }
    }
    if whole.total_size == 0 || whole.total_size > MAX_FORMAT_BYTES {
        bail!(
            "Refused: {whole_id} is {} bytes; the limit is 64 GB.",
            whole.total_size
        );
    }
    Ok(())
}

/// Re-reads diskutil and checks every guard. Returns the whole-disk info on success.
pub fn verify_card_for_format(expected: &CardIdentity, mount: &Path) -> Result<DiskInfo> {
    let volume = info(&mount.to_string_lossy()).context("the card is no longer mounted")?;
    let whole = info(&whole_disk_of(&volume.parent_whole_disk))?;
    check_format_guards(expected, &volume, &whole, &boot_whole_disks())?;
    Ok(whole)
}

/// Erases the card's whole disk as FAT32/MBR, then ejects it at once.
/// Guards run again here, immediately before the command. Err means nothing was erased.
/// Ok(Some(reason)) means the card was erased but would not eject.
pub fn format_card(expected: &CardIdentity, mount: &Path, label: &str) -> Result<Option<String>> {
    let label = fat_label(label)?;
    let whole = verify_card_for_format(expected, mount)?;
    let disk = format!("/dev/{}", whole_disk_of(&whole.device_identifier));
    let out = Command::new("/usr/sbin/diskutil")
        .args(["eraseDisk", "FAT32", &label, "MBRFormat", &disk])
        .output()
        .context("running diskutil eraseDisk")?;
    if !out.status.success() {
        bail!(
            "diskutil eraseDisk failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    // macOS mounts the fresh volume, and a volume watcher (this app's included) may be
    // scanning it for a moment and dissent. Retry briefly before giving up.
    let mut last = None;
    for _ in 0..8 {
        match eject(&disk) {
            Ok(()) => return Ok(None),
            Err(e) => last = Some(format!("{e:#}")),
        }
        std::thread::sleep(std::time::Duration::from_millis(750));
    }
    Ok(last)
}

/// `diskutil eject`: unplugging mid-write once wedged diskarbitrationd.
pub fn eject(target: &str) -> Result<()> {
    let out = Command::new("/usr/sbin/diskutil")
        .args(["eject", target])
        .output()?;
    if !out.status.success() {
        bail!(
            "diskutil eject failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card() -> (CardIdentity, DiskInfo, DiskInfo) {
        let vol = DiskInfo {
            device_identifier: "disk9s1".into(),
            parent_whole_disk: "disk9".into(),
            internal: false,
            removable: true,
            ejectable: true,
            total_size: 31_900_000_000,
            volume_uuid: Some("UUID-A".into()),
            volume_name: Some("FPVCARD".into()),
            filesystem: Some("MS-DOS FAT32".into()),
            ..Default::default()
        };
        let whole = DiskInfo {
            device_identifier: "disk9".into(),
            parent_whole_disk: "disk9".into(),
            volume_uuid: None,
            ..vol.clone()
        };
        (CardIdentity::from_info(&vol), vol, whole)
    }

    #[test]
    fn whole_disk_ids() {
        assert_eq!(whole_disk_of("disk4s1"), "disk4");
        assert_eq!(whole_disk_of("/dev/disk12s2"), "disk12");
        assert_eq!(whole_disk_of("disk4"), "disk4");
        assert_eq!(whole_disk_of("disk3s1s1"), "disk3");
    }

    #[test]
    fn guards_accept_the_same_small_removable_card() {
        let (id, v, w) = card();
        check_format_guards(&id, &v, &w, &["disk0".into(), "disk3".into()]).unwrap();
    }

    #[test]
    fn guards_refuse() {
        let boot = vec!["disk0".to_string(), "disk3".to_string()];
        let (id, v, w) = card();

        let mut w2 = w.clone();
        w2.internal = true;
        assert!(
            check_format_guards(&id, &v, &w2, &boot).is_err(),
            "internal"
        );

        let mut w2 = w.clone();
        w2.removable = false;
        w2.ejectable = false;
        assert!(
            check_format_guards(&id, &v, &w2, &boot).is_err(),
            "not removable"
        );

        let mut w2 = w.clone();
        w2.total_size = 128_000_000_000;
        assert!(
            check_format_guards(&id, &v, &w2, &boot).is_err(),
            "over 64 GB"
        );

        let mut v2 = v.clone();
        v2.volume_uuid = Some("UUID-B".into());
        assert!(
            check_format_guards(&id, &v2, &w, &boot).is_err(),
            "swapped card"
        );

        let mut id2 = id.clone();
        id2.volume_uuid = None;
        assert!(
            check_format_guards(&id2, &v, &w, &boot).is_err(),
            "no recorded uuid"
        );

        let mut w2 = w.clone();
        w2.device_identifier = "disk5".into();
        assert!(
            check_format_guards(&id, &v, &w2, &boot).is_err(),
            "different disk"
        );

        let boot2 = vec!["disk0".to_string(), "disk9".to_string()];
        assert!(
            check_format_guards(&id, &v, &w, &boot2).is_err(),
            "boot disk"
        );

        let (mut id0, mut v0, mut w0) = card();
        id0.whole_disk = "disk0".into();
        id0.device_identifier = "disk0s1".into();
        v0.device_identifier = "disk0s1".into();
        v0.parent_whole_disk = "disk0".into();
        w0.device_identifier = "disk0".into();
        assert!(check_format_guards(&id0, &v0, &w0, &[]).is_err(), "disk0");
    }

    #[test]
    fn labels() {
        assert_eq!(fat_label("").unwrap(), "DVR");
        assert_eq!(fat_label("fpvcard2").unwrap(), "FPVCARD2");
        assert!(fat_label("TWELVECHARSX").is_err());
        assert!(fat_label("A B").is_err());
    }

    #[test]
    fn fat32_detection() {
        let (_, v, _) = card();
        assert!(v.is_fat32());
        assert!(card_warnings(&v).is_empty());
        let mut x = v.clone();
        x.filesystem = Some("ExFAT".into());
        x.total_size = 64_000_000_000;
        assert_eq!(card_warnings(&x).len(), 2);
    }
}
