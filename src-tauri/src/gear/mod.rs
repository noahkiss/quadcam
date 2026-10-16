//! Gear: the FPV bench next to the clip library. The radio, the flight controller, the
//! ELRS link, goggles and DVR cards, packs and sims (`docs/gear-design.md`).
//!
//! Every module here runs without Tauri. `core/gear.rs` holds the `Core` methods,
//! `api/gear.rs` their rows, `mcp/gear.rs` the three MCP tools, and the CLI's
//! `bin/quadcam-cli/gear/` the `gear` subcommands.
//!
//! - `model`: the shared types (devices, backups, staged changes, plans, refusals).
//! - `store`: the gear folder and `gear.json`, its only writer.
//! - `bf`: the Betaflight link: CLI, MSP, dump parsing, the FC simulator, board notes.
//! - `compat`: the versions QuadCam has proven it can write.
//! - `serial`: USB serial ports, with the per-port lock and the test fail-safe.
//! - `detect`: the gear plugged in now.
//! - `events`: connected, identified, unmounted-but-present and removed, from one look to
//!   the next.
//! - `edgetx`: the EdgeTX card engine: YAML line editor, model views and ops, card plans
//!   and the card writer.
//! - `cues`: spoken, sound and notification cues ("safe to unplug", "still inserted").
//! - `flights`: flight analysis from radio logs: hover, sag, resting voltage, mAh,
//!   dropouts, and the flight index (`flights.json`).
//! - `packs`: packs and pack types, pack history, the charging sheet, suggestions.
//! - `crashes`: the crash and repair log.
//! - `report`: the session report; `preflight`: the "Pack up" check.
//! - `osd`: Betaflight OSD layouts: decode, draw per profile, overlap and off-screen check.
//! - `sim_cal`: the sim's radio calibrations, keyed by Gear radio id, the resolve step, and
//!   the defaults an aircraft's switch map gives the sim.
//! - `switchmap`: the switch map: an EdgeTX model joined with the FC's `aux` and `adjrange`
//!   lines, per control and position; live positions from channel values.
//! - `radio_hid`: the radio as a USB joystick (hidapi): reports, channels, the watch.
//! - `blobs`: the content-addressed blob store under the backups.
//! - `blackbox`: pulled blackbox flash images: records, the pairing with flights.
//! - `backup`: snapshots: take, list, read, diff, retain, prune, export, import old folders.
//! - `radiologs`: each radio log kept once per radio, outside snapshots.
//! - `health`: the card check (`diskutil verifyVolume`) and repair, and their log.
//!
//! `Env` is what Gear reaches outside the process (serial ports, volumes, presence, the cue
//! sink); tests replace it.

pub mod apply;
pub mod backup;
pub mod bf;
pub mod blackbox;
pub mod blobs;
pub mod changes;
pub mod compat;
pub mod copy;
pub mod crashes;
pub mod cues;
pub mod detect;
pub mod dfu;
pub mod edgetx;
pub mod elrs;
pub mod events;
pub mod firmware;
pub mod flights;
pub mod fwcopy;
pub mod health;
pub mod model;
pub mod osd;
pub mod packs;
pub mod preflight;
pub mod radio_hid;
pub mod radiologs;
pub mod rates;
pub mod report;
pub mod serial;
pub mod sim_cal;
pub mod sims;
pub mod splash;
pub mod store;
pub mod switchmap;
pub mod voice;

use crate::disk::Volume;
use model::DeviceKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// The Gear settings (Settings > Gear), each with its default. They live in
/// `settings.json` like every other setting (`settings::KEYS`); `from_values` reads them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct GearSettings {
    /// The gear folder: `gear.json`, backups, logs, staged changes.
    pub gear_dir: PathBuf,
    /// Back up a device when it is plugged in.
    pub auto_backup: bool,
    /// Plug-in and manual backups kept per device before thinning.
    pub keep_recent: u32,
    /// Then one a week for this many weeks.
    pub keep_weeks: u32,
    /// Then one a month, with no limit.
    pub keep_monthly: bool,
    /// Minutes an FC may run on USB power before the warning; 0 turns it off.
    pub usb_minutes: u32,
    /// `manual` or `daily`.
    pub firmware_check: String,
    /// The voice provider: `say` (macOS) or another a later version adds.
    pub tts_provider: String,
    /// Erase the FC's blackbox flash after a pull verified (`gearEraseBlackbox`). Off by
    /// default, like `delete_clips_after_import`.
    pub erase_blackbox: bool,
    /// Try the FC's USB disk mode before MSP for a blackbox pull (`gearBlackboxMsc`).
    /// Unproven on real FCs; off by default.
    pub blackbox_msc: bool,
    /// What runs when a device of each kind is plugged in (`gearOnConnect`). Backup also
    /// needs `auto_backup`.
    pub on_connect: BTreeMap<DeviceKind, Vec<Automation>>,
    /// Which cues play, and how (`gearCues`).
    pub cues: cues::CueSettings,
}

/// A step that may run on its own when a device is plugged in. Each is off unless the
/// device kind's `on_connect` list holds it; only `backup` is on by default.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, specta::Type,
)]
#[serde(rename_all = "snake_case")]
pub enum Automation {
    /// Back the device up.
    Backup,
    /// Import the clips on a card.
    Import,
    /// Apply the device's staged changes that are Ready.
    ApplyReady,
    /// Pull an FC's blackbox flash (and erase it, when `erase_blackbox` is on).
    Blackbox,
}

/// Values `firmwareCheck` takes.
pub const FIRMWARE_CHECKS: &[&str] = &["manual", "daily"];

impl GearSettings {
    /// The defaults, with the gear folder at `default_dir`.
    pub fn defaults(default_dir: PathBuf) -> Self {
        Self {
            gear_dir: default_dir,
            auto_backup: true,
            keep_recent: 10,
            keep_weeks: 8,
            keep_monthly: true,
            usb_minutes: 20,
            firmware_check: "manual".into(),
            tts_provider: "say".into(),
            erase_blackbox: false,
            blackbox_msc: false,
            on_connect: DeviceKind::ALL
                .iter()
                .map(|k| (*k, vec![Automation::Backup]))
                .collect(),
            cues: cues::CueSettings::default(),
        }
    }

    /// True when `automation` runs on its own for a device of this kind.
    pub fn runs(&self, kind: DeviceKind, automation: Automation) -> bool {
        (automation != Automation::Backup || self.auto_backup)
            && self
                .on_connect
                .get(&kind)
                .is_some_and(|l| l.contains(&automation))
    }

    /// The settings file's values on top of the defaults. A missing or bad value keeps the
    /// default.
    pub fn from_values(v: &crate::settings::Values, default_dir: PathBuf) -> Self {
        fn get<T: serde::de::DeserializeOwned>(v: &crate::settings::Values, k: &str) -> Option<T> {
            v.get(k)
                .filter(|x| !x.is_null())
                .and_then(|x| serde_json::from_value(x.clone()).ok())
        }
        let mut s = Self::defaults(default_dir);
        if let Some(p) = get::<PathBuf>(v, "gearDir").filter(|p| p.is_absolute()) {
            s.gear_dir = p;
        }
        if let Some(b) = get(v, "gearAutoBackup") {
            s.auto_backup = b;
        }
        if let Some(n) = get::<u32>(v, "gearKeepRecent").filter(|n| *n >= 1) {
            s.keep_recent = n;
        }
        if let Some(n) = get(v, "gearKeepWeeks") {
            s.keep_weeks = n;
        }
        if let Some(b) = get(v, "gearKeepMonthly") {
            s.keep_monthly = b;
        }
        if let Some(n) = get(v, "gearUsbMinutes") {
            s.usb_minutes = n;
        }
        if let Some(f) =
            get::<String>(v, "firmwareCheck").filter(|f| FIRMWARE_CHECKS.contains(&f.as_str()))
        {
            s.firmware_check = f;
        }
        if let Some(p) = get::<String>(v, "ttsProvider").filter(|p| !p.trim().is_empty()) {
            s.tts_provider = p;
        }
        if let Some(b) = get(v, "gearEraseBlackbox") {
            s.erase_blackbox = b;
        }
        if let Some(b) = get(v, "gearBlackboxMsc") {
            s.blackbox_msc = b;
        }
        // A kind in the file replaces that kind's list; other kinds keep the default.
        if let Some(m) = get::<BTreeMap<DeviceKind, Vec<Automation>>>(v, "gearOnConnect") {
            s.on_connect.extend(m);
        }
        if let Some(c) = get(v, "gearCues") {
            s.cues = c;
        }
        s
    }
}

/// Checks a whole number setting in a range (for `settings::KEYS`).
pub fn check_count(v: &Value, min: u64, max: u64) -> anyhow::Result<()> {
    match v.as_u64() {
        Some(n) if (min..=max).contains(&n) => Ok(()),
        _ => anyhow::bail!("a whole number from {min} to {max}"),
    }
}

/// The processes other than this one that have a serial port open, as `(pid, name)`.
pub type HoldersFn = Arc<dyn Fn(&str) -> Vec<(u32, String)> + Send + Sync>;

/// Unmounts a whole disk (`disk4`).
pub type UnmountFn = Arc<dyn Fn(&str) -> anyhow::Result<()> + Send + Sync>;

/// Mounts a whole disk (`disk4`) that an earlier job unmounted.
pub type MountFn = Arc<dyn Fn(&str) -> anyhow::Result<()> + Send + Sync>;

type VolumesFn = Arc<dyn Fn() -> Vec<Volume> + Send + Sync>;
type RefreshVolumesFn = Arc<dyn Fn() + Send + Sync>;
type VolumeScanFn = Arc<dyn Fn() -> crate::disk::VolumeScan + Send + Sync>;
type VolumeSignature = Vec<(std::path::PathBuf, u64, u64)>;
type VolumeSignatureFn = Arc<dyn Fn() -> VolumeSignature + Send + Sync>;
type ClockFn = Arc<dyn Fn() -> std::time::Instant + Send + Sync>;

/// How long a volume scan stays good while `/Volumes` looks the same. The signature cannot
/// see a card's contents (clips copied on or deleted, a reformat with the same name) or a
/// card put back at the same path, and the headless MCP server has no watcher. 30 s keeps an
/// idle Gear poll (every 2 s) to one `diskutil` round in 15 and lets such a change show
/// within half a minute. QuadCam's own deletes, formats and mounts invalidate at once.
const VOLUME_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

fn volume_signature() -> VolumeSignature {
    use std::os::unix::fs::MetadataExt;

    let Ok(entries) = std::fs::read_dir("/Volumes") else {
        return Vec::new();
    };
    let mut signature: VolumeSignature = entries
        .flatten()
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            Some((entry.path(), metadata.dev(), metadata.ino()))
        })
        .collect();
    signature.sort();
    signature
}

#[derive(Default)]
struct VolumeCache {
    /// Counts invalidations, so a scan that started before one does not store its result.
    generation: u64,
    kept: Option<(VolumeSignature, std::time::Instant, Vec<Volume>)>,
}

/// Keeps the relatively expensive `diskutil` volume scan while the cheap mount-point
/// signature stays unchanged, for at most `ttl`. Only a complete scan is kept. The scan runs
/// outside the lock, so an invalidation never waits for one, and a scan that an invalidation
/// overtook is returned but not kept. Serial ports and DFU devices are deliberately not
/// cached: the regular Gear poll must keep finding those promptly.
fn cached_volumes(
    signature: VolumeSignatureFn,
    source: VolumeScanFn,
    now: ClockFn,
    ttl: std::time::Duration,
) -> (VolumesFn, RefreshVolumesFn) {
    fn lock(cache: &std::sync::Mutex<VolumeCache>) -> std::sync::MutexGuard<'_, VolumeCache> {
        cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    let cache = Arc::new(std::sync::Mutex::new(VolumeCache::default()));
    let list_cache = cache.clone();
    let list = Arc::new(move || {
        let current = signature();
        let generation = {
            let cached = lock(&list_cache);
            if let Some((seen, at, volumes)) = cached.kept.as_ref() {
                if *seen == current && now().saturating_duration_since(*at) < ttl {
                    return volumes.clone();
                }
            }
            cached.generation
        };
        let started = now();
        let scan = source();
        if scan.complete {
            let mut cached = lock(&list_cache);
            if cached.generation == generation {
                cached.kept = Some((current, started, scan.volumes.clone()));
            }
        }
        scan.volumes
    });
    let refresh = Arc::new(move || {
        let mut cached = lock(&cache);
        cached.generation += 1;
        cached.kept = None;
    });
    (list, refresh)
}

/// What Gear reaches outside the process. The real one lists `/Volumes`, the system's
/// serial ports and what is still plugged in (none under cargo, see `serial`), and plays
/// cues (silent under cargo, see `cues`); tests pass synthetic volumes and fakes.
#[derive(Clone)]
pub struct Env {
    pub ports: Arc<dyn serial::Ports>,
    pub volumes: VolumesFn,
    /// Invalidates `volumes`; the `/Volumes` watcher and QuadCam's own mounts, clip deletes
    /// and formats use it.
    pub refresh_volumes: RefreshVolumesFn,
    pub dfu: Arc<dyn Fn() -> Vec<detect::DfuInfo> + Send + Sync>,
    pub presence: Arc<dyn Fn() -> Vec<events::Presence> + Send + Sync>,
    /// Cards in the built-in SD reader, with their hardware identity. Asked only when a
    /// volume sits in the slot.
    pub card_reader: Arc<dyn Fn() -> Vec<detect::CardHw> + Send + Sync>,
    /// USB devices with their disks (an EdgeTX radio in USB Storage mode). Asked only
    /// when a volume sits on USB.
    pub usb: Arc<dyn Fn() -> Vec<detect::UsbStorage> + Send + Sync>,
    /// The cue queue, its debounce and its reminders.
    pub cues: Arc<cues::CueService>,
    /// Unmounts a card's whole disk (`disk4`) under a timeout (`edgetx::card::release`).
    pub unmount: UnmountFn,
    /// Mounts a card's whole disk again (`edgetx::card::attach`) for a job or for the person.
    pub mount: MountFn,
    /// Tests: a card path whose read-back counts as a mismatch (`WriteOptions.fail_readback`).
    pub fail_readback: Option<String>,
    /// Tests: the card write's options in place of a reader's or a radio's (timeouts, a
    /// stalled file).
    pub card_write: Option<edgetx::card::WriteOptions>,
    /// Where real cards mount (`/Volumes`): a process started by cargo writes no card
    /// under it unless `QUADCAM_CARD_WRITE=real`. Tests name their own folder to see that
    /// refusal.
    pub real_cards: std::path::PathBuf,
    /// Whether a DJI USB device (an air unit, goggles in storage mode) is attached: the
    /// format guard asks it right before an erase (`disk::format_card_with`).
    pub dji_usb: Arc<dyn Fn() -> bool + Send + Sync>,
    /// Runs `diskutil` for the card check and repair (`health`).
    pub disk: Arc<dyn health::DiskRunner>,
    /// The other processes that have a serial port open, as `(pid, name)`.
    pub holders: HoldersFn,
    /// The voice providers (`voice::tts`): `say` and an OpenAI-compatible server.
    pub tts: Arc<dyn voice::tts::Providers>,
    /// Secrets (the voice provider's API key): the login Keychain.
    pub keys: Arc<dyn voice::keychain::KeyStore>,
    /// The Swift helper (`crate::intel`): the speech check of voice lines.
    pub intel: Arc<crate::intel::Intel>,
    /// GitHub's `gh`: a voice pack's Approve publishes through it.
    pub gh: Arc<voice::publish::Gh>,
}

impl Env {
    /// The system's volumes, ports and DFU devices. Port locks go in `<cache>/locks`.
    pub fn system(cache: &std::path::Path) -> Self {
        let (volumes, refresh_volumes) = cached_volumes(
            Arc::new(volume_signature),
            Arc::new(crate::disk::scan_volumes),
            Arc::new(std::time::Instant::now),
            VOLUME_CACHE_TTL,
        );
        let refresh_after_unmount = refresh_volumes.clone();
        let refresh_after_mount = refresh_volumes.clone();
        Self {
            ports: serial::system(cache.join("locks")),
            volumes,
            refresh_volumes,
            dfu: Arc::new(detect::dfu_devices),
            presence: Arc::new(events::presence),
            card_reader: Arc::new(detect::card_reader),
            usb: Arc::new(detect::usb_storage),
            cues: Arc::new(cues::CueService::system()),
            unmount: Arc::new(move |d| {
                let result = edgetx::card::release(d, edgetx::card::UNMOUNT_TIMEOUT);
                if result.is_ok() {
                    refresh_after_unmount();
                }
                result
            }),
            mount: Arc::new(move |d| {
                let result = edgetx::card::attach(d, edgetx::card::UNMOUNT_TIMEOUT);
                if result.is_ok() {
                    refresh_after_mount();
                }
                result
            }),
            fail_readback: None,
            card_write: None,
            real_cards: edgetx::card::REAL_CARDS.into(),
            dji_usb: Arc::new(events::dji_usb_attached),
            disk: health::system(),
            holders: Arc::new(serial::other_holders),
            tts: Arc::new(voice::tts::System {
                scratch: cache.join("voice").join("tmp"),
            }),
            keys: Arc::new(voice::keychain::Keychain),
            intel: Arc::new(crate::intel::Intel::system()),
            gh: Arc::new(voice::publish::Gh::system()),
        }
    }

    /// Fixed volumes and these ports; no DFU devices, nothing present, silent cues, a
    /// `diskutil` whose checks pass.
    pub fn fake(volumes: Vec<Volume>, ports: Arc<dyn serial::Ports>) -> Self {
        Self {
            ports,
            volumes: Arc::new(move || volumes.clone()),
            refresh_volumes: Arc::new(|| {}),
            dfu: Arc::new(Vec::new),
            presence: Arc::new(Vec::new),
            card_reader: Arc::new(Vec::new),
            usb: Arc::new(Vec::new),
            cues: Arc::new(cues::CueService::inline(Arc::new(
                cues::RecordedCues::default(),
            ))),
            unmount: Arc::new(|_| Ok(())),
            mount: Arc::new(|_| Ok(())),
            fail_readback: None,
            card_write: None,
            real_cards: edgetx::card::REAL_CARDS.into(),
            dji_usb: Arc::new(|| false),
            disk: Arc::new(health::FakeDisk::ok()),
            holders: Arc::new(|_| Vec::new()),
            tts: Arc::new(voice::tts::NoProviders),
            keys: Arc::new(voice::keychain::MemKeys::default()),
            intel: Arc::new(crate::intel::Intel::unavailable(
                "Tests have no quadcam-intel helper.",
            )),
            gh: Arc::new(voice::publish::Gh::unavailable("Tests have no gh.")),
        }
    }

    /// The gear plugged in now.
    pub fn detect(&self) -> Vec<model::Connected> {
        let volumes = (self.volumes)();
        let cards = if volumes
            .iter()
            .any(|v| detect::is_sd_slot(v.info.bus_protocol.as_deref()))
        {
            (self.card_reader)()
        } else {
            Vec::new()
        };
        let usb = if volumes.iter().any(|v| {
            v.info
                .bus_protocol
                .as_deref()
                .is_some_and(|b| b.eq_ignore_ascii_case("USB"))
        }) {
            (self.usb)()
        } else {
            Vec::new()
        };
        detect::detect_all(&volumes, &self.ports.list(), &(self.dfu)(), &cards, &usb)
    }

    /// Notes that something mounted, unmounted or changed on a volume, so the next Gear look
    /// scans `/Volumes`.
    pub fn volumes_changed(&self) {
        (self.refresh_volumes)();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A clock tests move by hand.
    fn test_clock() -> (ClockFn, Arc<std::sync::Mutex<std::time::Duration>>) {
        let base = std::time::Instant::now();
        let offset = Arc::new(std::sync::Mutex::new(std::time::Duration::ZERO));
        let read = offset.clone();
        (Arc::new(move || base + *read.lock().unwrap()), offset)
    }

    fn scan(complete: bool) -> crate::disk::VolumeScan {
        crate::disk::VolumeScan {
            volumes: Vec::new(),
            complete,
        }
    }

    #[test]
    fn volume_cache_scans_once_until_invalidated() {
        let scans = Arc::new(AtomicUsize::new(0));
        let counted = scans.clone();
        let signature = Arc::new(std::sync::Mutex::new(vec![(PathBuf::from("card"), 1, 1)]));
        let read_signature = signature.clone();
        let (clock, _) = test_clock();
        let (volumes, refresh) = cached_volumes(
            Arc::new(move || read_signature.lock().unwrap().clone()),
            Arc::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                scan(true)
            }),
            clock,
            VOLUME_CACHE_TTL,
        );

        volumes();
        volumes();
        assert_eq!(scans.load(Ordering::SeqCst), 1);

        refresh();
        volumes();
        assert_eq!(scans.load(Ordering::SeqCst), 2);

        *signature.lock().unwrap() = vec![(PathBuf::from("other-card"), 2, 2)];
        volumes();
        assert_eq!(scans.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn volume_cache_keeps_no_incomplete_scan() {
        let scans = Arc::new(AtomicUsize::new(0));
        let counted = scans.clone();
        let (clock, _) = test_clock();
        let (volumes, _) = cached_volumes(
            Arc::new(Vec::new),
            // The first look has a volume diskutil could not describe; the second is whole.
            Arc::new(move || scan(counted.fetch_add(1, Ordering::SeqCst) > 0)),
            clock,
            VOLUME_CACHE_TTL,
        );

        volumes();
        volumes();
        assert_eq!(
            scans.load(Ordering::SeqCst),
            2,
            "an incomplete scan is looked at again"
        );
        volumes();
        assert_eq!(scans.load(Ordering::SeqCst), 2, "a complete scan is kept");
    }

    #[test]
    fn volume_cache_expires_after_its_ttl() {
        let scans = Arc::new(AtomicUsize::new(0));
        let counted = scans.clone();
        let (clock, offset) = test_clock();
        let (volumes, _) = cached_volumes(
            Arc::new(Vec::new),
            Arc::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                scan(true)
            }),
            clock,
            VOLUME_CACHE_TTL,
        );

        volumes();
        *offset.lock().unwrap() = VOLUME_CACHE_TTL - std::time::Duration::from_secs(1);
        volumes();
        assert_eq!(scans.load(Ordering::SeqCst), 1);
        *offset.lock().unwrap() = VOLUME_CACHE_TTL;
        volumes();
        assert_eq!(scans.load(Ordering::SeqCst), 2);
        volumes();
        assert_eq!(scans.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn volume_cache_drops_a_scan_an_invalidation_overtook() {
        let scans = Arc::new(AtomicUsize::new(0));
        let counted = scans.clone();
        let refresh_slot: Arc<std::sync::OnceLock<RefreshVolumesFn>> = Default::default();
        let during_scan = refresh_slot.clone();
        let (clock, _) = test_clock();
        let (volumes, refresh) = cached_volumes(
            Arc::new(Vec::new),
            Arc::new(move || {
                // Something changes while the first scan runs. The cache lock is free, so
                // this does not wait for the scan.
                if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                    (during_scan.get().unwrap())();
                }
                scan(true)
            }),
            clock,
            VOLUME_CACHE_TTL,
        );
        let _ = refresh_slot.set(refresh);

        volumes();
        volumes();
        assert_eq!(
            scans.load(Ordering::SeqCst),
            2,
            "the overtaken scan was not kept"
        );
        volumes();
        assert_eq!(scans.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn volume_cache_survives_a_panicking_scan() {
        let scans = Arc::new(AtomicUsize::new(0));
        let counted = scans.clone();
        let (clock, _) = test_clock();
        let (volumes, refresh) = cached_volumes(
            Arc::new(Vec::new),
            Arc::new(move || {
                if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                    panic!("scan failed");
                }
                scan(true)
            }),
            clock,
            VOLUME_CACHE_TTL,
        );

        let first = volumes.clone();
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || first())).is_err());
        volumes();
        refresh();
        volumes();
        assert_eq!(scans.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn settings_defaults_and_overrides() {
        let def = PathBuf::from("/support/gear");
        let empty = crate::settings::Values::new();
        let s = GearSettings::from_values(&empty, def.clone());
        assert_eq!(s, GearSettings::defaults(def.clone()));
        assert!(s.auto_backup);
        assert_eq!((s.keep_recent, s.keep_weeks, s.usb_minutes), (10, 8, 20));
        let v: crate::settings::Values = serde_json::from_value(json!({
            "gearDir": "/elsewhere/gear", "gearAutoBackup": false, "gearKeepRecent": 0,
            "gearKeepWeeks": 4, "gearUsbMinutes": 0, "firmwareCheck": "hourly", "ttsProvider": ""
        }))
        .unwrap();
        let s = GearSettings::from_values(&v, def);
        assert_eq!(s.gear_dir, PathBuf::from("/elsewhere/gear"));
        assert!(!s.auto_backup);
        assert_eq!(s.keep_recent, 10, "0 is not a valid count");
        assert_eq!(s.keep_weeks, 4);
        assert_eq!(s.usb_minutes, 0);
        assert_eq!(s.firmware_check, "manual", "a bad value keeps the default");
        assert_eq!(s.tts_provider, "say");
        assert!(
            !s.runs(DeviceKind::Fc, Automation::Backup),
            "auto_backup off"
        );
    }

    #[test]
    fn automations_are_off_but_backup() {
        let def = PathBuf::from("/g");
        let s = GearSettings::defaults(def.clone());
        for k in DeviceKind::ALL {
            assert!(s.runs(k, Automation::Backup));
            assert!(!s.runs(k, Automation::Import));
            assert!(!s.runs(k, Automation::ApplyReady));
            assert!(!s.runs(k, Automation::Blackbox));
        }
        assert!(!s.erase_blackbox && !s.blackbox_msc, "both off by default");
        let v: crate::settings::Values = serde_json::from_value(json!({
            "gearOnConnect": {"dvr_card": ["backup", "import"], "fc": ["blackbox"]},
            "gearEraseBlackbox": true,
            "gearCues": {"speech": false}
        }))
        .unwrap();
        let s = GearSettings::from_values(&v, def);
        assert!(s.runs(DeviceKind::DvrCard, Automation::Import));
        assert!(!s.runs(DeviceKind::Fc, Automation::Backup));
        assert!(s.runs(DeviceKind::Fc, Automation::Blackbox));
        assert!(s.erase_blackbox);
        assert!(
            s.runs(DeviceKind::Radio, Automation::Backup),
            "other kinds keep theirs"
        );
        assert!(!s.cues.speech);
        assert!(s.cues.notification);
    }
}
