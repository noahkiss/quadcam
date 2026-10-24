//! Volumes and disks: `diskutil info`, card detection, and the guarded format (after an
//! import, and card prep) with the file system of the source's `CardPolicy`.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Cards over this size ship as exFAT; analog DVRs want FAT32. A card's size is never a
/// reason to refuse a format: the source's `CardPolicy` turns it into advice
/// (`format_advice`).
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
    /// A card in the Mac's built-in SD slot. macOS reports the slot as internal
    /// (`Internal` true, "internal, physical" in `diskutil list`), but the card in it is
    /// removable media on the `Secure Digital` bus. An internal SSD is neither.
    pub fn is_slot_card(&self) -> bool {
        self.removable
            && self
                .bus_protocol
                .as_deref()
                .is_some_and(|b| b.eq_ignore_ascii_case("Secure Digital"))
    }

    /// Internal storage of the Mac: internal and not a card in the built-in SD slot.
    pub fn is_internal_storage(&self) -> bool {
        self.internal && !self.is_slot_card()
    }

    pub fn is_fat32(&self) -> bool {
        self.filesystem
            .as_deref()
            .is_some_and(|f| f.to_uppercase().contains("FAT32"))
    }
}

/// How long a `diskutil` read may run. A busy Disk Arbitration answers within seconds; a
/// hung one would otherwise block the caller (the Gear poll, a format guard) for good.
/// Only reads come through here, so stopping one changes nothing on a disk.
const DISKUTIL_READ_TIMEOUT: Duration = Duration::from_secs(30);

fn run_diskutil_plist(args: &[&str]) -> Result<plist::Dictionary> {
    let mut child = Command::new("/usr/sbin/diskutil")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running diskutil")?;
    let drain = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut v = Vec::new();
            if let Some(mut r) = r {
                let _ = r.read_to_end(&mut v);
            }
            v
        })
    };
    let stdout = drain(
        child
            .stdout
            .take()
            .map(|r| Box::new(r) as Box<dyn Read + Send>),
    );
    let stderr = drain(
        child
            .stderr
            .take()
            .map(|r| Box::new(r) as Box<dyn Read + Send>),
    );
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().context("waiting for diskutil")? {
            break s;
        }
        if start.elapsed() >= DISKUTIL_READ_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "diskutil {} did not answer within {} s.",
                args.join(" "),
                DISKUTIL_READ_TIMEOUT.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let stdout = stdout.join().unwrap_or_default();
    if !status.success() {
        bail!(
            "diskutil {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&stderr.join().unwrap_or_default()).trim()
        );
    }
    let v: plist::Value = plist::from_bytes(&stdout).context("parsing diskutil plist")?;
    v.into_dictionary()
        .ok_or_else(|| anyhow!("diskutil returned no dictionary"))
}

/// `diskutil info -plist <target>`; `target` is a mount point, `/dev/diskN` or `diskN`.
pub fn info(target: &str) -> Result<DiskInfo> {
    Ok(info_whole(target)?.0)
}

/// `info`, and whether the answer was whole: it named the device and said whether the
/// disk is internal. A partial answer reads as internal storage (`parse_info`).
fn info_whole(target: &str) -> Result<(DiskInfo, bool)> {
    let d = run_diskutil_plist(&["info", "-plist", target])?;
    let whole = d.contains_key("Internal") && d.contains_key("DeviceIdentifier");
    Ok((parse_info(&d), whole))
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
    /// Removable and holds clips of a source QuadCam reads.
    pub is_card: bool,
    /// The video system of the clips on a card.
    #[serde(default)]
    pub source: Option<crate::sources::SourceKind>,
    /// Holds `LOGS/` next to `MODELS/` or `RADIO/`: an EdgeTX radio in USB Storage mode.
    pub is_radio: bool,
    pub warnings: Vec<String>,
}

/// A disk a person can take out: external and removable or ejectable, or a card in the
/// built-in SD slot.
pub fn is_removable(info: &DiskInfo) -> bool {
    !info.is_internal_storage() && (info.removable || info.ejectable)
}

pub fn looks_like_radio(mount: &Path) -> bool {
    mount.join("LOGS").is_dir() && (mount.join("MODELS").is_dir() || mount.join("RADIO").is_dir())
}

/// What a card's source would rather it were: its file system and size.
pub fn card_warnings(info: &DiskInfo, policy: &crate::sources::CardPolicy) -> Vec<String> {
    let mut w = Vec::new();
    let fs_ok = info
        .filesystem
        .as_deref()
        .is_some_and(|f| f.to_uppercase().contains(&policy.filesystem.to_uppercase()));
    if !fs_ok {
        w.push(format!(
            "Card is {}, not {}. {}",
            info.filesystem.as_deref().unwrap_or("an unknown format"),
            policy.filesystem,
            policy.filesystem_advice
        ));
    }
    if info.total_size > policy.warn_above_bytes && !policy.size_warning.is_empty() {
        w.push(policy.size_warning.into());
    }
    w
}

/// Inspects one mount point. Returns None when diskutil cannot describe it.
pub fn probe_volume(mount: &Path) -> Option<Volume> {
    let info = info(&mount.to_string_lossy()).ok()?;
    Some(volume_of(mount, info))
}

fn volume_of(mount: &Path, info: DiskInfo) -> Volume {
    let removable = is_removable(&info);
    let is_radio = removable && looks_like_radio(mount);
    let source = (removable && !is_radio)
        .then(|| crate::sources::detect(mount))
        .flatten();
    let is_card = source.is_some();
    let warnings = source
        .map(|s| card_warnings(&info, &s.card_policy()))
        .unwrap_or_default();
    Volume {
        mount: mount.to_path_buf(),
        info,
        is_card,
        source: source.map(|s| s.kind()),
        is_radio,
        warnings,
    }
}

/// One look at `/Volumes`.
#[derive(Debug, Clone, Default)]
pub struct VolumeScan {
    /// Every removable volume.
    pub volumes: Vec<Volume>,
    /// Every local mount point answered `diskutil` in full. False means a volume may be
    /// missing from `volumes` only for now, so the scan is not worth keeping.
    pub complete: bool,
}

/// Every removable volume under `/Volumes`.
pub fn list_volumes() -> Vec<Volume> {
    scan_volumes().volumes
}

/// `list_volumes`, and whether the look was complete. An entry `diskutil` cannot describe
/// counts against it only when it is a local mount point: a folder left behind under
/// `/Volumes` or a network share never answers, and is never a card either.
pub fn scan_volumes() -> VolumeScan {
    use std::os::unix::fs::MetadataExt;
    let Ok(rd) = std::fs::read_dir("/Volumes") else {
        return VolumeScan::default();
    };
    let parent = std::fs::metadata("/Volumes").map(|m| m.dev()).ok();
    let mut scan = VolumeScan {
        volumes: Vec::new(),
        complete: true,
    };
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let mount = e.path();
        let whole = match info_whole(&mount.to_string_lossy()) {
            Ok((info, whole)) => {
                let v = volume_of(&mount, info);
                if is_removable(&v.info) {
                    scan.volumes.push(v);
                }
                whole
            }
            Err(_) => false,
        };
        if !whole && is_local_mount(&mount, parent) {
            scan.complete = false;
        }
    }
    scan.volumes.sort_by(|a, b| a.mount.cmp(&b.mount));
    scan
}

/// A file system of this Mac mounted at `path` (not the one `/Volumes` itself is on).
fn is_local_mount(path: &Path, parent_dev: Option<u64>) -> bool {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    let Ok(m) = std::fs::metadata(path) else {
        return false;
    };
    if Some(m.dev()) == parent_dev {
        return false;
    }
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid C string and `st` a writable statfs the call fills in.
    let r = unsafe { libc::statfs(c.as_ptr(), &mut st) };
    r == 0 && st.f_flags & libc::MNT_LOCAL as u32 != 0
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

/// Volume label (FAT32 and exFAT alike): `DVR` by default, upper case, at most 11 characters.
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
        if d.is_internal_storage() {
            bail!("Refused: {} is an internal disk.", d.device_identifier);
        }
        if !(d.removable || d.ejectable) {
            bail!(
                "Refused: {} is not removable or ejectable.",
                d.device_identifier
            );
        }
    }
    if whole.total_size == 0 {
        bail!("Refused: {whole_id} reports no size.");
    }
    if !is_card_filesystem(volume.filesystem.as_deref()) {
        bail!(
            "Refused: the volume is {}, and DVRs and DJI units write only FAT or exFAT. This looks like a drive, not a card.",
            volume.filesystem.as_deref().unwrap_or("an unknown file system")
        );
    }
    Ok(())
}

/// A file system a DVR or a DJI unit writes: FAT (12, 16, 32) or exFAT. A volume that is
/// APFS, HFS+, NTFS or unknown is a drive, never a card.
pub fn is_card_filesystem(fs: Option<&str>) -> bool {
    fs.is_some_and(|f| {
        let f = f.to_ascii_uppercase();
        f.contains("FAT") || f.contains("MSDOS") || f.contains("MS-DOS")
    })
}

/// The partitions of a whole disk that can hold a volume, from `diskutil list -plist
/// <disk>`: every partition but an EFI system partition. A card formatted without a
/// partition table counts as 0.
pub fn volume_partitions(list: &plist::Dictionary) -> usize {
    list.get("AllDisksAndPartitions")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|d| d.as_dictionary())
        .and_then(|d| d.get("Partitions"))
        .and_then(|p| p.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter(|p| {
                    p.as_dictionary()
                        .and_then(|p| p.get("Content"))
                        .and_then(|c| c.as_string())
                        .is_none_or(|c| !c.eq_ignore_ascii_case("EFI"))
                })
                .count()
        })
        .unwrap_or(0)
}

/// The erase takes the whole disk, so a disk with more than one volume (a drive with
/// partitions) is refused. A card has one.
pub fn check_one_volume(whole_id: &str, partitions: usize) -> Result<()> {
    if partitions > 1 {
        bail!("Refused: {whole_id} holds {partitions} volumes, and the erase would take them all. A card holds one.");
    }
    Ok(())
}

/// What the format confirm says about a card's size for a source: advice, never a refusal.
pub fn format_advice(size: u64, policy: &crate::sources::CardPolicy) -> Vec<String> {
    if size > policy.warn_above_bytes && !policy.size_warning.is_empty() {
        vec![policy.size_warning.to_string()]
    } else {
        Vec::new()
    }
}

/// Re-reads diskutil and checks every guard. Returns the whole-disk info on success.
pub fn verify_card_for_format(expected: &CardIdentity, mount: &Path) -> Result<DiskInfo> {
    let volume = info(&mount.to_string_lossy()).context("the card is no longer mounted")?;
    let whole = info(&whole_disk_of(&volume.parent_whole_disk))?;
    check_format_guards(expected, &volume, &whole, &boot_whole_disks())?;
    let whole_id = whole_disk_of(&whole.device_identifier);
    let list = run_diskutil_plist(&["list", "-plist", &whole_id])
        .context("Refused: diskutil cannot list the card's volumes")?;
    check_one_volume(&whole_id, volume_partitions(&list))?;
    Ok(whole)
}

/// The `diskutil eraseDisk` personality for a `CardPolicy` file system: FAT32 (proven on
/// real cards) or exFAT (tested on disk images). Any other file system refuses.
pub fn erase_personality(filesystem: &str) -> Result<&'static str> {
    if filesystem.eq_ignore_ascii_case("FAT32") {
        Ok("FAT32")
    } else if filesystem.eq_ignore_ascii_case("exFAT") {
        Ok("ExFAT")
    } else {
        bail!("Refused: QuadCam formats cards only as FAT32 or exFAT, not {filesystem}.")
    }
}

/// Which format path asks: the format after an import, or card prep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EraseBy {
    Import,
    Prep,
}

/// Whether the source's policy offers a format on this path.
pub fn check_policy(policy: &crate::sources::CardPolicy, by: EraseBy) -> Result<()> {
    let offered = match by {
        EraseBy::Import => policy.format_offered,
        EraseBy::Prep => policy.prep_offered,
    };
    if !offered {
        bail!(match by {
            EraseBy::Import => "Refused: QuadCam does not format this source's cards after an import. Card prep can erase a removable card once every clip is in the library.",
            EraseBy::Prep => "Refused: card prep does not format this source's cards.",
        });
    }
    Ok(())
}

/// What the volume at `mount` holds, and what it is, as far as a format cares. A radio's SD
/// card is never erased. A DJI device over USB (an air unit, goggles in storage mode) is
/// never erased, whatever its storage holds (it may have been emptied in the goggles):
/// refused when the disk's media name names DJI, or when a DJI USB device is attached
/// (`dji_usb_attached`) and the card is not in the built-in SD slot. A volume that holds a
/// video file no source reads (`sources::unread_videos`) is refused: the erase would take it
/// without naming it. `disk` is the volume's or whole disk's `diskutil info`.
pub fn check_volume_contents(mount: &Path, disk: &DiskInfo, dji_usb_attached: bool) -> Result<()> {
    if looks_like_radio(mount) {
        bail!("Refused: this volume is a radio's SD card (LOGS with MODELS or RADIO).");
    }
    let named_dji = disk
        .media_name
        .as_deref()
        .is_some_and(|m| m.to_ascii_uppercase().contains("DJI"));
    if named_dji {
        bail!("Refused: QuadCam never formats a DJI device over USB (air unit or goggles storage). Format it in the device.");
    }
    if dji_usb_attached && !disk.is_slot_card() {
        bail!("Refused: QuadCam never formats a DJI device over USB (air unit or goggles storage), and one is attached. To format a card in a reader, unplug the DJI device first, or use the Mac's SD slot.");
    }
    let unread = crate::sources::unread_videos(mount);
    if !unread.is_empty() {
        let shown: Vec<&str> = unread.iter().take(5).map(String::as_str).collect();
        let more = if unread.len() > shown.len() {
            format!(" and {} more", unread.len() - shown.len())
        } else {
            String::new()
        };
        bail!(
            "Refused: {} video file{} on the card {} QuadCam reads ({}{more}). Copy {} off first, or format the card in its device.",
            unread.len(),
            if unread.len() == 1 { "" } else { "s" },
            if unread.len() == 1 { "is not a clip" } else { "are not clips" },
            shown.join(", "),
            if unread.len() == 1 { "it" } else { "them" }
        );
    }
    Ok(())
}

/// Erases the card's whole disk with the file system of `policy` (MBR), then unmounts it at
/// once (safe to remove). Every guard runs again here, immediately before the command: the
/// source's policy for `by`, the disk guards, the volume's contents, then `recheck` (the
/// caller's own guards, for example card prep's "every clip is in the library"). Err means
/// nothing was erased. Ok(Some(reason)) means the card was erased but would not unmount.
pub fn format_card(
    expected: &CardIdentity,
    mount: &Path,
    label: &str,
    policy: &crate::sources::CardPolicy,
    by: EraseBy,
    recheck: &dyn Fn() -> Result<()>,
) -> Result<Option<String>> {
    format_card_with(
        expected,
        mount,
        label,
        policy,
        by,
        &crate::gear::events::dji_usb_attached,
        recheck,
    )
}

/// `format_card`, asking `dji_usb` whether a DJI USB device is attached (`Env.dji_usb`; the
/// system's answer is `gear::events::dji_usb_attached`). It asks right before the erase.
pub fn format_card_with(
    expected: &CardIdentity,
    mount: &Path,
    label: &str,
    policy: &crate::sources::CardPolicy,
    by: EraseBy,
    dji_usb: &dyn Fn() -> bool,
    recheck: &dyn Fn() -> Result<()>,
) -> Result<Option<String>> {
    check_policy(policy, by)?;
    let personality = erase_personality(policy.filesystem)?;
    let label = fat_label(label)?;
    let whole = verify_card_for_format(expected, mount)?;
    check_volume_contents(mount, &whole, dji_usb())?;
    recheck()?;
    let disk = format!("/dev/{}", whole_disk_of(&whole.device_identifier));
    let out = Command::new("/usr/sbin/diskutil")
        .args(["eraseDisk", personality, &label, "MBRFormat", &disk])
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
        match unmount_disk(&disk) {
            Ok(()) => return Ok(None),
            Err(e) => last = Some(format!("{e:#}")),
        }
        std::thread::sleep(std::time::Duration::from_millis(750));
    }
    Ok(last)
}

/// Makes a disk safe to remove. A card (removable media: a reader's card, a card in the
/// built-in slot) and a DJI device over USB (an O4 air unit, goggles in storage mode) get
/// `diskutil unmountDisk`: every volume unmounts, and the disk stays listed until it is
/// pulled, so a later job can mount it again. Anything else gets `diskutil eject`. `target`
/// is a mount point, `/dev/diskN` or `diskN`; the whole disk is unmounted either way.
/// `dji_usb` says whether a DJI USB device is attached (`Env.dji_usb`). Unplugging
/// mid-write once wedged diskarbitrationd, so this always runs before a pull.
pub fn safe_remove(target: &str, dji_usb: &dyn Fn() -> bool) -> Result<()> {
    safe_remove_with(target, dji_usb, true)
}

/// `safe_remove`, but with `other_disks` false it refuses a disk that is neither a card nor
/// a DJI device, before it runs anything: a target someone named may be a backup drive.
/// While a DJI device is on USB, `removes_by_unmount` takes any external disk; a named
/// target must then prove itself (`named_target_ok`).
pub fn safe_remove_with(target: &str, dji_usb: &dyn Fn() -> bool, other_disks: bool) -> Result<()> {
    let vol = info(target)?;
    let whole_id = whole_disk_of(if vol.parent_whole_disk.is_empty() {
        &vol.device_identifier
    } else {
        &vol.parent_whole_disk
    });
    let whole = info(&whole_id).unwrap_or(vol);
    let dev = format!("/dev/{whole_id}");
    let dji_volume = || {
        list_volumes().iter().any(|v| {
            whole_disk_of(&v.info.device_identifier) == whole_id
                && v.source == Some(crate::sources::SourceKind::Dji)
        })
    };
    if !other_disks && !named_target_ok(&whole, dji_usb, &dji_volume) {
        bail!("{target} is not a card or a DJI device; QuadCam ejects only those. Nothing was ejected.")
    }
    if removes_by_unmount(&whole, dji_usb) {
        unmount_disk(&dev)
    } else {
        eject(&dev)
    }
}

/// A target someone named is a card or a DJI device: removable media, a disk whose media
/// name names DJI, or, while a DJI device is on USB, an external disk with a mounted DJI
/// volume (`DCIM/DJI_*`, `dji_volume`). Any other external disk is refused, a backup drive
/// plugged in beside the DJI device included.
pub fn named_target_ok(
    whole: &DiskInfo,
    dji_usb: &dyn Fn() -> bool,
    dji_volume: &dyn Fn() -> bool,
) -> bool {
    whole.removable
        || whole
            .media_name
            .as_deref()
            .is_some_and(|m| m.to_ascii_uppercase().contains("DJI"))
        || (!whole.internal && dji_usb() && dji_volume())
}

/// True when "safe to remove" unmounts the disk rather than ejecting it: a card, or a DJI
/// device over USB (its media name names DJI, or it is not a slot card while a DJI USB
/// device is attached). The DJI device stays on the bus, so it can be mounted again.
pub fn removes_by_unmount(whole: &DiskInfo, dji_usb: &dyn Fn() -> bool) -> bool {
    whole.removable
        || whole
            .media_name
            .as_deref()
            .is_some_and(|m| m.to_ascii_uppercase().contains("DJI"))
        || (!whole.internal && dji_usb())
}

/// `diskutil unmountDisk`: unmounts every volume of the whole disk; the disk stays attached.
pub fn unmount_disk(target: &str) -> Result<()> {
    let out = Command::new("/usr/sbin/diskutil")
        .args(["unmountDisk", target])
        .output()?;
    if !out.status.success() {
        bail!(
            "diskutil unmountDisk failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// `diskutil eject`: for a disk that is not a card.
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

    fn analog_policy() -> crate::sources::CardPolicy {
        crate::sources::Source::card_policy(&crate::sources::analog::Analog)
    }

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

    /// A `diskutil info -plist` answer with the keys QuadCam reads. The values are made up.
    fn plist_info(
        id: &str,
        internal: bool,
        removable: bool,
        ejectable: bool,
        bus: &str,
        size: u64,
        uuid: Option<&str>,
    ) -> DiskInfo {
        let whole = whole_disk_of(id);
        let uuid = uuid
            .map(|u| format!("<key>VolumeUUID</key><string>{u}</string><key>VolumeName</key><string>CARD</string><key>MountPoint</key><string>/Volumes/CARD</string><key>FilesystemPersonality</key><string>MS-DOS FAT32</string>"))
            .unwrap_or_default();
        let b = |v: bool| if v { "<true/>" } else { "<false/>" };
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>DeviceIdentifier</key><string>{id}</string>
<key>ParentWholeDisk</key><string>{whole}</string>
<key>Internal</key>{}
<key>RemovableMedia</key>{}
<key>Removable</key>{}
<key>Ejectable</key>{}
<key>BusProtocol</key><string>{bus}</string>
<key>TotalSize</key><integer>{size}</integer>
{uuid}
</dict></plist>"#,
            b(internal),
            b(removable),
            b(removable),
            b(ejectable)
        );
        let v: plist::Value = plist::from_bytes(xml.as_bytes()).unwrap();
        parse_info(&v.into_dictionary().unwrap())
    }

    /// (expected identity, volume, whole disk) as diskutil reports them for one kind of disk.
    fn plist_card(
        internal: bool,
        removable: bool,
        ejectable: bool,
        bus: &str,
        size: u64,
    ) -> (CardIdentity, DiskInfo, DiskInfo) {
        let vol = plist_info(
            "disk4s1",
            internal,
            removable,
            ejectable,
            bus,
            size,
            Some("UUID-SLOT"),
        );
        let whole = plist_info("disk4", internal, removable, ejectable, bus, size, None);
        (CardIdentity::from_info(&vol), vol, whole)
    }

    #[test]
    fn slot_card_usb_reader_card_and_internal_ssd() {
        let boot = vec!["disk0".to_string(), "disk3".to_string()];

        // The built-in SD slot: Internal true, but removable media on the Secure Digital bus.
        let (id, v, w) = plist_card(true, true, true, "Secure Digital", 31_900_000_000);
        assert!(v.internal && v.is_slot_card() && !v.is_internal_storage());
        assert!(is_removable(&v) && is_removable(&w));
        check_format_guards(&id, &v, &w, &boot).unwrap();
        // A card's size never refuses: a 128 GB card passes, with advice from its policy.
        let (id2, v2, w2) = plist_card(true, true, true, "Secure Digital", 128_000_000_000);
        check_format_guards(&id2, &v2, &w2, &boot).unwrap();
        assert_eq!(
            format_advice(w2.total_size, &analog_policy()),
            [analog_policy().size_warning]
        );
        assert!(format_advice(31_900_000_000, &analog_policy()).is_empty());
        let dji = crate::sources::Source::card_policy(&crate::sources::dji::Dji);
        assert!(format_advice(1_000_000_000_000, &dji).is_empty());
        // Every other guard still holds for a slot card.
        assert!(
            check_format_guards(&id, &v, &w, &["disk0".into(), "disk4".into()]).is_err(),
            "boot disk"
        );
        let mut swapped = v.clone();
        swapped.volume_uuid = Some("UUID-OTHER".into());
        assert!(
            check_format_guards(&id, &swapped, &w, &boot).is_err(),
            "swapped card"
        );

        // A card in a USB reader: external, removable.
        let (id, v, w) = plist_card(false, true, true, "USB", 31_900_000_000);
        assert!(!v.is_slot_card() && is_removable(&v));
        check_format_guards(&id, &v, &w, &boot).unwrap();

        // The internal SSD: internal, not removable media, another bus. Refused.
        let (id, v, w) = plist_card(true, false, false, "Apple Fabric", 31_900_000_000);
        assert!(v.is_internal_storage() && !is_removable(&v));
        let e = check_format_guards(&id, &v, &w, &[])
            .unwrap_err()
            .to_string();
        assert!(e.contains("internal"), "{e}");
        // Internal storage on the SD bus that is not removable media stays internal.
        let (id, v, w) = plist_card(true, false, true, "Secure Digital", 31_900_000_000);
        assert!(!v.is_slot_card() && v.is_internal_storage());
        assert!(check_format_guards(&id, &v, &w, &[]).is_err());
    }

    #[test]
    fn whole_disk_ids() {
        assert_eq!(whole_disk_of("disk4s1"), "disk4");
        assert_eq!(whole_disk_of("/dev/disk12s2"), "disk12");
        assert_eq!(whole_disk_of("disk4"), "disk4");
        assert_eq!(whole_disk_of("disk3s1s1"), "disk3");
    }

    #[test]
    fn guards_refuse() {
        let boot = vec!["disk0".to_string(), "disk3".to_string()];
        let (id, v, w) = card();
        // The baseline: the same small removable card passes.
        check_format_guards(&id, &v, &w, &boot).unwrap();

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
        w2.total_size = 0;
        assert!(check_format_guards(&id, &v, &w2, &boot).is_err(), "no size");
        w2.total_size = 512_000_000_000;
        check_format_guards(&id, &v, &w2, &boot).unwrap();

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
    fn fat32_and_exfat_are_formatted() {
        assert_eq!(erase_personality("FAT32").unwrap(), "FAT32");
        assert_eq!(erase_personality("exFAT").unwrap(), "ExFAT");
        assert!(erase_personality("APFS").is_err());
        assert!(erase_personality("HFS+").is_err());
    }

    #[test]
    fn policy_offers_per_path() {
        let dji = crate::sources::Source::card_policy(&crate::sources::dji::Dji);
        check_policy(&analog_policy(), EraseBy::Import).unwrap();
        check_policy(&analog_policy(), EraseBy::Prep).unwrap();
        assert!(check_policy(&dji, EraseBy::Import).is_err());
        check_policy(&dji, EraseBy::Prep).unwrap();
    }

    #[test]
    fn safe_remove_unmounts_a_card_and_a_dji_device_over_usb() {
        let card = DiskInfo {
            removable: true,
            ..Default::default()
        };
        assert!(removes_by_unmount(&card, &|| false));
        // An O4 air unit over USB: ejectable, not removable media. Unmounted, so it stays
        // on the bus and a later job can mount it again.
        let o4 = DiskInfo {
            ejectable: true,
            media_name: Some("DJI Air Unit".into()),
            bus_protocol: Some("USB".into()),
            ..Default::default()
        };
        assert!(removes_by_unmount(&o4, &|| false));
        let unnamed = DiskInfo {
            media_name: Some("Storage".into()),
            ..o4.clone()
        };
        assert!(removes_by_unmount(&unnamed, &|| true));
        // Any other external disk is ejected.
        assert!(!removes_by_unmount(&unnamed, &|| false));
    }

    #[test]
    fn a_named_target_must_be_a_card_or_a_dji_device() {
        let card = DiskInfo {
            removable: true,
            ..Default::default()
        };
        assert!(named_target_ok(&card, &|| true, &|| false));
        let o4 = DiskInfo {
            ejectable: true,
            media_name: Some("DJI Air Unit".into()),
            bus_protocol: Some("USB".into()),
            ..Default::default()
        };
        assert!(named_target_ok(&o4, &|| false, &|| false));
        // A DJI device whose media name does not say so: only with its DJI volume mounted.
        let unnamed = DiskInfo {
            media_name: Some("Storage".into()),
            ..o4.clone()
        };
        assert!(named_target_ok(&unnamed, &|| true, &|| true));
        // A backup drive beside a DJI device on USB: refused, never unmounted.
        let backup = DiskInfo {
            ejectable: true,
            media_name: Some("Backup Plus".into()),
            bus_protocol: Some("USB".into()),
            ..Default::default()
        };
        assert!(removes_by_unmount(&backup, &|| true));
        assert!(!named_target_ok(&backup, &|| true, &|| false));
        assert!(!named_target_ok(&backup, &|| false, &|| true));
        let internal = DiskInfo {
            internal: true,
            ..Default::default()
        };
        assert!(!named_target_ok(&internal, &|| true, &|| true));
    }

    #[test]
    fn radio_volumes_and_dji_devices_refuse() {
        let (_, reader_card, _) = card();
        let d = tempfile::tempdir().unwrap();
        check_volume_contents(d.path(), &reader_card, false).unwrap();
        std::fs::create_dir_all(d.path().join("DCIM")).unwrap();
        std::fs::write(d.path().join("DCIM/PICT0001.AVI"), b"x").unwrap();
        check_volume_contents(d.path(), &reader_card, false).unwrap();

        let radio = tempfile::tempdir().unwrap();
        for x in ["LOGS", "MODELS"] {
            std::fs::create_dir(radio.path().join(x)).unwrap();
        }
        let e = check_volume_contents(radio.path(), &reader_card, false)
            .unwrap_err()
            .to_string();
        assert!(e.contains("radio"), "{e}");

        // A DJI volume, even emptied: a removable card in a reader passes while no DJI
        // device is on USB; a card in the built-in slot passes either way.
        let dji = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dji.path().join("DCIM/DJI_001")).unwrap();
        check_volume_contents(dji.path(), &reader_card, false).unwrap();
        let (_, slot_card, _) = plist_card(true, true, true, "Secure Digital", 128_000_000_000);
        check_volume_contents(dji.path(), &slot_card, true).unwrap();
        // A DJI device over USB refuses: a DJI USB device attached, or a DJI media name.
        let e = check_volume_contents(dji.path(), &reader_card, true)
            .unwrap_err()
            .to_string();
        assert!(e.contains("DJI device over USB"), "{e}");
        let mut air_unit = reader_card.clone();
        air_unit.media_name = Some("DJI Air Unit".into());
        let e = check_volume_contents(dji.path(), &air_unit, false)
            .unwrap_err()
            .to_string();
        assert!(e.contains("DJI device over USB"), "{e}");
        // A DJI device over USB refuses whatever it holds: an air unit whose storage was
        // emptied in the goggles has `DCIM/` and `MISC/` but no `DJI_*` folder.
        let emptied = tempfile::tempdir().unwrap();
        for x in ["DCIM", "MISC"] {
            std::fs::create_dir(emptied.path().join(x)).unwrap();
        }
        for (disk, usb) in [(&air_unit, false), (&reader_card, true)] {
            let e = check_volume_contents(emptied.path(), disk, usb)
                .unwrap_err()
                .to_string();
            assert!(e.contains("DJI device over USB"), "{e}");
        }
        // An analog card in a reader refuses too while a DJI device is attached; in the
        // built-in slot it passes.
        let e = check_volume_contents(d.path(), &reader_card, true)
            .unwrap_err()
            .to_string();
        assert!(e.contains("unplug the DJI device"), "{e}");
        check_volume_contents(d.path(), &slot_card, true).unwrap();
    }

    #[test]
    fn video_files_no_source_reads_refuse() {
        let (_, reader_card, _) = card();
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("DCIM/100MEDIA")).unwrap();
        std::fs::write(d.path().join("DCIM/100MEDIA/DJI_0001.MP4"), b"x").unwrap();
        let e = check_volume_contents(d.path(), &reader_card, false)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("1 video file on the card is not a clip QuadCam reads")
                && e.contains("DJI_0001.MP4"),
            "{e}"
        );
    }

    #[test]
    fn drives_refuse_a_format() {
        let boot = vec!["disk0".to_string()];
        let (id, v, w) = card();
        // An external SSD: APFS or HFS+, ejectable, not removable media.
        for fs in [
            Some("APFS"),
            Some("Mac OS Extended (Journaled)"),
            Some("NTFS"),
            None,
        ] {
            let mut v2 = v.clone();
            v2.filesystem = fs.map(str::to_string);
            let e = check_format_guards(&id, &v2, &w, &boot)
                .unwrap_err()
                .to_string();
            assert!(e.contains("not a card"), "{fs:?}: {e}");
        }
        for fs in ["MS-DOS FAT32", "MS-DOS FAT16", "ExFAT", "exfat", "msdos"] {
            assert!(is_card_filesystem(Some(fs)), "{fs}");
        }
        // A disk with two volumes: the erase would take both.
        let xml = |parts: &[&str]| {
            let parts: String = parts
                .iter()
                .map(|c| format!("<dict><key>Content</key><string>{c}</string></dict>"))
                .collect();
            let x = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>AllDisksAndPartitions</key><array><dict><key>DeviceIdentifier</key><string>disk9</string><key>Partitions</key><array>{parts}</array></dict></array></dict></plist>"#
            );
            let v: plist::Value = plist::from_bytes(x.as_bytes()).unwrap();
            volume_partitions(&v.into_dictionary().unwrap())
        };
        assert_eq!(xml(&["DOS_FAT_32"]), 1);
        assert_eq!(xml(&["EFI", "Microsoft Basic Data"]), 1);
        assert_eq!(xml(&[]), 0);
        assert_eq!(xml(&["EFI", "Apple_HFS", "Microsoft Basic Data"]), 2);
        check_one_volume("disk9", 1).unwrap();
        check_one_volume("disk9", 0).unwrap();
        let e = check_one_volume("disk9", 2).unwrap_err().to_string();
        assert!(e.contains("2 volumes"), "{e}");
    }

    #[test]
    fn format_card_refuses_a_source_that_does_not_offer_it() {
        let dji = crate::sources::Source::card_policy(&crate::sources::dji::Dji);
        let (id, _, _) = card();
        // Refused on the policy, before diskutil is ever asked about the disk.
        let e = format_card(
            &id,
            Path::new("/nonexistent"),
            "DVR",
            &dji,
            EraseBy::Import,
            &|| Ok(()),
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("does not format"), "{e}");
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
        let policy = analog_policy();
        let (_, v, _) = card();
        assert!(v.is_fat32());
        assert!(card_warnings(&v, &policy).is_empty());
        let mut x = v.clone();
        x.filesystem = Some("ExFAT".into());
        x.total_size = policy.warn_above_bytes + 1;
        let w = card_warnings(&x, &policy);
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w[0].starts_with("Card is ExFAT, not FAT32."), "{}", w[0]);
        assert!(w[0].ends_with(policy.filesystem_advice), "{}", w[0]);
        assert_eq!(w[1], policy.size_warning);
        // The size warning starts one byte above the policy's limit.
        x.filesystem = v.filesystem.clone();
        x.total_size = policy.warn_above_bytes;
        assert!(card_warnings(&x, &policy).is_empty());
    }

    #[test]
    fn dji_card_warnings_follow_its_policy() {
        let dji = crate::sources::Source::card_policy(&crate::sources::dji::Dji);
        let (_, v, _) = card();
        let mut x = v.clone();
        x.filesystem = Some("ExFAT".into());
        x.total_size = 512_000_000_000;
        assert!(card_warnings(&x, &dji).is_empty(), "exFAT of any size");
        let w = card_warnings(&v, &dji);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(
            w[0].starts_with("Card is MS-DOS FAT32, not exFAT."),
            "{}",
            w[0]
        );
        assert!(w[0].ends_with(dji.filesystem_advice), "{}", w[0]);
        assert!(!w[0].contains("analog"));
    }
}
