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
//! - `backup`: snapshots: take, list, read, diff, retain, prune, export, import old folders.
//! - `radiologs`: each radio log kept once per radio, outside snapshots.
//! - `health`: the card check (`diskutil verifyVolume`) and repair, and their log.
//!
//! `Env` is what Gear reaches outside the process (serial ports, volumes, presence, the cue
//! sink); tests replace it.

pub mod backup;
pub mod bf;
pub mod blobs;
pub mod compat;
pub mod crashes;
pub mod cues;
pub mod detect;
pub mod edgetx;
pub mod events;
pub mod flights;
pub mod health;
pub mod model;
pub mod osd;
pub mod packs;
pub mod preflight;
pub mod radio_hid;
pub mod radiologs;
pub mod report;
pub mod serial;
pub mod sim_cal;
pub mod store;
pub mod switchmap;

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

/// Unmounts a whole disk (`disk4`).
pub type UnmountFn = Arc<dyn Fn(&str) -> anyhow::Result<()> + Send + Sync>;

/// What Gear reaches outside the process. The real one lists `/Volumes`, the system's
/// serial ports and what is still plugged in (none under cargo, see `serial`), and plays
/// cues (silent under cargo, see `cues`); tests pass synthetic volumes and fakes.
#[derive(Clone)]
pub struct Env {
    pub ports: Arc<dyn serial::Ports>,
    pub volumes: Arc<dyn Fn() -> Vec<Volume> + Send + Sync>,
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
    /// Runs `diskutil` for the card check and repair (`health`).
    pub disk: Arc<dyn health::DiskRunner>,
}

impl Env {
    /// The system's volumes, ports and DFU devices. Port locks go in `<cache>/locks`.
    pub fn system(cache: &std::path::Path) -> Self {
        Self {
            ports: serial::system(cache.join("locks")),
            volumes: Arc::new(crate::disk::list_volumes),
            dfu: Arc::new(detect::dfu_devices),
            presence: Arc::new(events::presence),
            card_reader: Arc::new(detect::card_reader),
            usb: Arc::new(detect::usb_storage),
            cues: Arc::new(cues::CueService::system()),
            unmount: Arc::new(|d| edgetx::card::release(d, edgetx::card::UNMOUNT_TIMEOUT)),
            disk: health::system(),
        }
    }

    /// Fixed volumes and these ports; no DFU devices, nothing present, silent cues, a
    /// `diskutil` whose checks pass.
    pub fn fake(volumes: Vec<Volume>, ports: Arc<dyn serial::Ports>) -> Self {
        Self {
            ports,
            volumes: Arc::new(move || volumes.clone()),
            dfu: Arc::new(Vec::new),
            presence: Arc::new(Vec::new),
            card_reader: Arc::new(Vec::new),
            usb: Arc::new(Vec::new),
            cues: Arc::new(cues::CueService::inline(Arc::new(
                cues::RecordedCues::default(),
            ))),
            unmount: Arc::new(|_| Ok(())),
            disk: Arc::new(health::FakeDisk::ok()),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        }
        let v: crate::settings::Values = serde_json::from_value(json!({
            "gearOnConnect": {"dvr_card": ["backup", "import"], "fc": []},
            "gearCues": {"speech": false}
        }))
        .unwrap();
        let s = GearSettings::from_values(&v, def);
        assert!(s.runs(DeviceKind::DvrCard, Automation::Import));
        assert!(!s.runs(DeviceKind::Fc, Automation::Backup));
        assert!(
            s.runs(DeviceKind::Radio, Automation::Backup),
            "other kinds keep theirs"
        );
        assert!(!s.cues.speech);
        assert!(s.cues.notification);
    }
}
