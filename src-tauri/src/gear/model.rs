//! The types every Gear module shares: devices and their identity, what is connected now,
//! backups, staged changes and their edits, apply plans, and refusals. The data lives in
//! `gear.json` (devices) and the gear folder (backups, changes); `store.rs` reads and
//! writes it. Later packages add variants and fields here; each new field takes
//! `#[serde(default)]` so files written by an earlier version still load.

use crate::session::Editor;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::PathBuf;

/// What a device is.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash, Type,
)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    /// A flight controller (Betaflight) on USB serial.
    Fc,
    /// An EdgeTX radio: its SD card in USB Storage mode, or its USB serial port.
    Radio,
    /// An ExpressLRS transmitter module.
    ElrsTx,
    /// An ExpressLRS receiver.
    ElrsRx,
    /// Goggles, by their card.
    Goggles,
    /// An analog DVR, by its card.
    DvrCard,
}

impl DeviceKind {
    pub const ALL: [DeviceKind; 6] = [
        DeviceKind::Fc,
        DeviceKind::Radio,
        DeviceKind::ElrsTx,
        DeviceKind::ElrsRx,
        DeviceKind::Goggles,
        DeviceKind::DvrCard,
    ];

    /// The word the UI and the CLI show.
    pub fn label(self) -> &'static str {
        match self {
            DeviceKind::Fc => "FC",
            DeviceKind::Radio => "Radio",
            DeviceKind::ElrsTx => "ELRS TX",
            DeviceKind::ElrsRx => "ELRS RX",
            DeviceKind::Goggles => "Goggles",
            DeviceKind::DvrCard => "DVR card",
        }
    }

    /// The prefix of this kind's device ids.
    pub fn id_prefix(self) -> &'static str {
        match self {
            DeviceKind::Fc => "fc",
            DeviceKind::Radio => "radio",
            DeviceKind::ElrsTx => "elrs-tx",
            DeviceKind::ElrsRx => "elrs-rx",
            DeviceKind::Goggles => "goggles",
            DeviceKind::DvrCard => "dvr",
        }
    }
}

/// What a device reports about itself. Every field is optional: a device may not say.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct Identity {
    /// The board or target name (`pocket`, `STM32F411`).
    #[serde(default)]
    pub board: Option<String>,
    /// The firmware family (`EdgeTX`, `Betaflight`, `ExpressLRS`).
    #[serde(default)]
    pub firmware: Option<String>,
    /// The firmware version (`2.12.4`, `4.5.1`).
    #[serde(default)]
    pub version: Option<String>,
    /// The build id or date, when the firmware reports one.
    #[serde(default)]
    pub build: Option<String>,
    /// The flash target, for firmware that names one (ExpressLRS).
    #[serde(default)]
    pub target: Option<String>,
}

/// A device QuadCam knows, kept in `gear.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct Device {
    /// Stable: a hash of what the device says about itself (see `device_id`).
    pub id: String,
    pub kind: DeviceKind,
    /// The person's name for it. Empty until they give one ("Unnamed FC").
    #[serde(default)]
    pub name: String,
    /// The aircraft profile it belongs to.
    #[serde(default)]
    pub aircraft: Option<String>,
    #[serde(default)]
    pub identity: Identity,
    #[serde(default)]
    pub last_seen: Option<DateTime<Utc>>,
    /// The id of its newest backup.
    #[serde(default)]
    pub last_backup: Option<String>,
    /// The card's free space when the newest backup was taken.
    #[serde(default)]
    pub last_space: Option<SpaceSeen>,
    /// Other ids this device answers to: a radio card has one id in the built-in SD slot
    /// (its hardware serial) and another in the radio's USB Storage mode (its volume UUID).
    /// QuadCam records the second when it sees the first, so both are one saved radio.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// A radio's STM32 chip serial, as its DFU bootloader reports it. The person links it
    /// once; DFU mode is the only mode that shows it.
    #[serde(default)]
    pub dfu_serial: Option<String>,
}

/// A card's space at one moment, in bytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct SpaceSeen {
    pub free: u64,
    #[serde(default)]
    pub total: Option<u64>,
    pub at: DateTime<Utc>,
}

impl Device {
    /// The name to show: the person's, else "Unnamed <kind>".
    pub fn display_name(&self) -> String {
        if self.name.trim().is_empty() {
            format!("Unnamed {}", self.kind.label())
        } else {
            self.name.clone()
        }
    }
}

/// A device id: the kind's prefix and an XXH64 of what identifies the device (a volume
/// UUID, a hash of the FC's MSP UID). The raw value never leaves the machine in an id.
pub fn device_id(kind: DeviceKind, raw: &str) -> String {
    format!(
        "{}-{:016x}",
        kind.id_prefix(),
        xxhash_rust::xxh64::xxh64(raw.as_bytes(), 0)
    )
}

/// How a connected device is reached.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Link {
    /// A mounted volume (a radio's SD card in USB Storage mode, a goggles or DVR card).
    Volume {
        mount: PathBuf,
        #[serde(default)]
        volume_uuid: Option<String>,
        /// The disk's protocol (`USB`, `Secure Digital` for the built-in SD slot).
        #[serde(default)]
        bus_protocol: Option<String>,
        /// The whole disk (`disk4`). After `diskutil unmountDisk` its node stays while the
        /// card is in, so its going away means the card was pulled.
        #[serde(default)]
        whole_disk: Option<String>,
    },
    /// A USB serial port.
    Serial {
        /// `/dev/cu.usbmodem...`
        port: String,
        vid: u16,
        pid: u16,
        #[serde(default)]
        product: Option<String>,
    },
    /// A USB DFU device (a radio in its bootloader).
    Dfu {
        vid: u16,
        pid: u16,
        /// The chip's unique id, which the bootloader reports. A saved radio's `dfu_serial`
        /// links to it.
        #[serde(default)]
        serial: Option<String>,
    },
}

/// A device plugged in now, as `detect` found it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct Connected {
    /// The device id, when the device could be identified without talking to it (a
    /// volume). A serial device gets its id once it is identified (MSP for an FC).
    pub id: Option<String>,
    pub kind: DeviceKind,
    pub link: Link,
    /// What detection could read without opening a port (the radio's `board` and `semver`).
    pub identity: Identity,
    /// The saved device with this id, when QuadCam knows it.
    #[serde(default)]
    pub device: Option<Device>,
    /// The USB device a volume sits on, when it is a radio in USB Storage mode (not a
    /// card in a reader). Its writes are slow (about 0.3 MB/s).
    #[serde(default)]
    pub usb: Option<UsbInfo>,
    /// Other device ids this volume answers to (its volume UUID and marker, when `id` came
    /// from the card's hardware serial). Matched against a saved device's `aliases`.
    #[serde(default)]
    pub also: Vec<String>,
}

/// What a USB device says about itself.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct UsbInfo {
    pub vid: u16,
    pub pid: u16,
    #[serde(default)]
    pub vendor: Option<String>,
    #[serde(default)]
    pub product: Option<String>,
    /// The USB serial number. EdgeTX radios report a generic one, so it is not an id.
    #[serde(default)]
    pub serial: Option<String>,
    /// `bcdDevice` as `major.minor` (an EdgeTX radio reports its firmware's, `2.12`).
    #[serde(default)]
    pub version: Option<String>,
}

/// Why a backup was taken.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// Plugged in, with "Back up on connect" on.
    Connect,
    /// The person asked.
    Manual,
    /// Taken by the apply engine before a write. Always kept.
    BeforeApply,
    /// Taken before a firmware flash. Always kept.
    BeforeFlash,
    /// Read back from an FC right after an apply, so the next plan compares with what the
    /// FC holds now. Thinned like plug-in backups.
    AfterApply,
    /// Taken from an old backup folder (`gear import-backups`). Thinned like plug-in
    /// backups.
    Import,
}

impl Trigger {
    /// The word in a snapshot's file name.
    pub fn slug(self) -> &'static str {
        match self {
            Trigger::Connect => "connect",
            Trigger::Manual => "manual",
            Trigger::BeforeApply => "before_apply",
            Trigger::BeforeFlash => "before_flash",
            Trigger::AfterApply => "after_apply",
            Trigger::Import => "import",
        }
    }

    /// Apply and flash snapshots are never pruned.
    pub fn always_kept(self) -> bool {
        matches!(self, Trigger::BeforeApply | Trigger::BeforeFlash)
    }
}

/// One file in a backup: its path inside the device, its size and its content hash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct BackupFile {
    /// For a card: the path from the card's root (`MODELS/model01.yml`). For an FC: the
    /// command (`diff all`, `dump all`).
    pub path: String,
    pub size: u64,
    /// XXH64 as 16 hex digits: the blob's name.
    pub xxh64: String,
    #[serde(default)]
    pub mtime: Option<DateTime<Utc>>,
}

/// A snapshot of one device: a manifest of files in the blob store.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct Backup {
    pub id: String,
    pub device: String,
    pub trigger: Trigger,
    pub taken_at: DateTime<Utc>,
    pub identity: Identity,
    pub files: Vec<BackupFile>,
    /// The person's pin. Apply and flash backups are always kept anyway.
    #[serde(default)]
    pub pinned: bool,
}

/// A staged change's place in the bench queue.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    Draft,
    Ready,
    Try,
    ReadFirst,
    Applied,
    Verified,
    Failed,
    Reverted,
    Discarded,
}

impl ChangeStatus {
    /// Staged and not applied yet: the change still waits for the person.
    pub fn staged(self) -> bool {
        matches!(
            self,
            ChangeStatus::Draft | ChangeStatus::Ready | ChangeStatus::Try | ChangeStatus::ReadFirst
        )
    }
}

/// Where a Betaflight `set` lives.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case", tag = "kind", content = "index")]
pub enum Section {
    /// Not in a profile.
    Master,
    /// `profile N`.
    Profile(u8),
    /// `rateprofile N`.
    RateProfile(u8),
    /// `battery_profile N` (Betaflight 2026.6 and later).
    BatteryProfile(u8),
}

impl Section {
    /// The CLI line that selects this section; none for `Master`.
    pub fn select_line(self) -> Option<String> {
        match self {
            Section::Master => None,
            Section::Profile(n) => Some(format!("profile {n}")),
            Section::RateProfile(n) => Some(format!("rateprofile {n}")),
            Section::BatteryProfile(n) => Some(format!("battery_profile {n}")),
        }
    }
}

/// A file a change puts on a card.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct CardFile {
    /// From the card's root.
    pub path: String,
    /// XXH64 of the new bytes: the blob that holds them.
    pub xxh64: String,
    pub size: u64,
}

/// One edit in a staged change. Each package adds the variants it owns (rate profiles,
/// model operations); the apply engine refuses a variant it has no writer for.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Edit {
    /// Raw CLI lines, checked against the device's dump.
    FcLines { lines: Vec<String> },
    /// One `set`.
    FcSet {
        section: Section,
        name: String,
        value: String,
    },
    /// One `aux` line.
    FcAux {
        slot: u8,
        mode: u16,
        aux: u8,
        start: u16,
        end: u16,
    },
    /// One `adjrange` line.
    FcAdjrange {
        slot: u8,
        range_aux: u8,
        start: u16,
        end: u16,
        function: u8,
        select_aux: u8,
    },
    /// Moves an OSD element and picks the profiles that show it.
    OsdElement {
        element: String,
        x: u8,
        y: u8,
        profiles: Vec<u8>,
    },
    /// Files to put on and delete from a card.
    CardFiles {
        put: Vec<CardFile>,
        delete: Vec<String>,
    },
    /// Puts files of a backup back.
    Restore { backup: String, paths: Vec<String> },
    /// Ops on one EdgeTX model file (`model01.yml`). `name`, when set, must match the
    /// file's header name ("wrong card?").
    Model {
        file: String,
        #[serde(default)]
        name: Option<String>,
        ops: Vec<super::edgetx::model::ModelOp>,
    },
    /// Ops on `RADIO/radio.yml`. The selected model changes only through `select_model`.
    Radio {
        ops: Vec<super::edgetx::card::RadioOp>,
    },
    /// A model's power-on checklist text (`MODELS/<model name>.txt`). Turn it on or off
    /// with the model op `set_checklist`.
    Checklist { model: String, text: String },
    /// A new model file copied from another: its own name, every timer value 0, no model id.
    ModelCopy {
        from: String,
        to: String,
        name: String,
    },
    /// Deletes a model file; never the selected one.
    ModelDelete { file: String },
    /// ExpressLRS options of a saved ELRS device, set over CRSF (design 6.4). Never the
    /// binding phrase.
    ElrsOptions { options: Vec<super::elrs::ElrsSet> },
}

/// One entry in a change's history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct ChangeEvent {
    pub at: DateTime<Utc>,
    pub status: ChangeStatus,
    #[serde(default)]
    pub note: String,
}

/// A change to one device, staged and not yet applied (or applied, as history).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct StagedChange {
    pub id: String,
    pub device: String,
    pub title: String,
    pub status: ChangeStatus,
    pub edits: Vec<Edit>,
    /// The backup the "before" state was read from.
    pub base_backup: String,
    pub editor: Editor,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub order: u32,
    #[serde(default)]
    pub history: Vec<ChangeEvent>,
    /// The change this one undoes (a Revert stages a restore). When this change verifies,
    /// that one becomes Reverted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverts: Option<String>,
}

/// Why a write was refused. Each is one row of the checks table (design 8.2).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum RefusalCode {
    UnknownVersion,
    UnknownBoard,
    DeviceChanged,
    BeforeMismatch,
    ShapeUnknown,
    RoundTrip,
    NoBackup,
    SeveralDevices,
    SimRunning,
    BadImage,
    BadSetting,
    /// The port is open in another QuadCam process (the app or the CLI).
    PortBusy,
    /// Serial and device access is off in this process (tests; see `serial::system`).
    Disabled,
    /// The device the change is for is not plugged in.
    NoDevice,
    /// The FC has run on USB with its battery in past its limit; it must cool first.
    UsbHeat,
    /// The card's last check found errors; repair it before a write.
    CardCheck,
    /// The change is marked "Read first": read the real value on the device, then mark it
    /// Ready.
    ReadFirst,
    /// Settings do not fit the device they would go to (board or firmware).
    Incompatible,
    /// A file QuadCam would write cannot be written (a sim file's folder or permissions).
    NotWritable,
}

impl RefusalCode {
    /// The code as written in JSON and on the CLI.
    pub fn as_str(self) -> &'static str {
        match self {
            RefusalCode::UnknownVersion => "unknown_version",
            RefusalCode::UnknownBoard => "unknown_board",
            RefusalCode::DeviceChanged => "device_changed",
            RefusalCode::BeforeMismatch => "before_mismatch",
            RefusalCode::ShapeUnknown => "shape_unknown",
            RefusalCode::RoundTrip => "round_trip",
            RefusalCode::NoBackup => "no_backup",
            RefusalCode::SeveralDevices => "several_devices",
            RefusalCode::SimRunning => "sim_running",
            RefusalCode::BadImage => "bad_image",
            RefusalCode::BadSetting => "bad_setting",
            RefusalCode::PortBusy => "port_busy",
            RefusalCode::Disabled => "disabled",
            RefusalCode::NoDevice => "no_device",
            RefusalCode::UsbHeat => "usb_heat",
            RefusalCode::CardCheck => "card_check",
            RefusalCode::ReadFirst => "read_first",
            RefusalCode::Incompatible => "incompatible",
            RefusalCode::NotWritable => "not_writable",
        }
    }
}

/// A guard said no. As an error its text starts with "Refused", which the CLI maps to exit
/// code 3 and MCP returns as the error. `anyhow::Error::downcast_ref::<Refusal>` gets the
/// code back.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct Refusal {
    pub code: RefusalCode,
    pub reason: String,
}

impl Refusal {
    pub fn new(code: RefusalCode, reason: impl Into<String>) -> Self {
        Self {
            code,
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Refused ({}): {}", self.code.as_str(), self.reason)
    }
}

impl std::error::Error for Refusal {}

/// One guard in a plan: passed, or the refusal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct Check {
    /// What was checked, in words ("Known version").
    pub name: String,
    pub ok: bool,
    /// Set when the check fails.
    #[serde(default)]
    pub refusal: Option<Refusal>,
}

/// A line in a line diff.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum LineOp {
    Same,
    Add,
    Remove,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct DiffLine {
    pub op: LineOp,
    pub text: String,
}

/// One item in a plan's before/after view.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DiffItem {
    /// CLI lines for an FC, or a line diff of one text file.
    Lines { label: String, lines: Vec<DiffLine> },
    /// Files put on and deleted from a card.
    Files {
        label: String,
        put: Vec<String>,
        delete: Vec<String>,
    },
    /// A firmware version pair.
    Version {
        label: String,
        before: Option<String>,
        after: String,
    },
}

/// What applying a change would do, with every guard's result. Writes nothing. `digest`
/// covers the identity, the before state and the edits; the apply needs it back.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct ApplyPlan {
    pub change: String,
    pub device: Identity,
    pub checks: Vec<Check>,
    pub diff: Vec<DiffItem>,
    pub digest: String,
    /// Things to know that do not refuse (a sim file shape not yet seen loading).
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl ApplyPlan {
    /// True when every check passed.
    pub fn ready(&self) -> bool {
        self.checks.iter().all(|c| c.ok)
    }
}

/// An aircraft profile's links to its gear (`Profile.gear`). Every field is optional, so
/// profiles from before Gear load unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct ProfileGear {
    /// The FC's device id.
    #[serde(default)]
    pub fc: Option<String>,
    /// The radio's device id.
    #[serde(default)]
    pub radio: Option<String>,
    /// The EdgeTX model file, for example `model01.yml`.
    #[serde(default)]
    pub edgetx_model: Option<String>,
    /// The receiver's device id.
    #[serde(default)]
    pub rx: Option<String>,
    #[serde(default)]
    pub pack_type: Option<String>,
}

impl ProfileGear {
    pub fn is_empty(&self) -> bool {
        self == &ProfileGear::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn device_ids_hash_the_raw_value() {
        let id = device_id(DeviceKind::Radio, "0A1B2C3D-UUID");
        assert!(id.starts_with("radio-"));
        assert_eq!(id.len(), "radio-".len() + 16);
        assert!(!id.contains("0A1B2C3D"));
        assert_eq!(id, device_id(DeviceKind::Radio, "0A1B2C3D-UUID"));
        assert_ne!(id, device_id(DeviceKind::Radio, "other"));
    }

    /// Every code. The match has no wildcard, so a new code fails to compile until it is
    /// listed here and so reaches the checks below.
    fn all_codes() -> Vec<RefusalCode> {
        use RefusalCode::*;
        let all = vec![
            UnknownVersion,
            UnknownBoard,
            DeviceChanged,
            BeforeMismatch,
            ShapeUnknown,
            RoundTrip,
            NoBackup,
            SeveralDevices,
            SimRunning,
            BadImage,
            BadSetting,
            PortBusy,
            Disabled,
            NoDevice,
            UsbHeat,
            CardCheck,
            ReadFirst,
            Incompatible,
            NotWritable,
        ];
        for c in &all {
            match c {
                UnknownVersion | UnknownBoard | DeviceChanged | BeforeMismatch | ShapeUnknown
                | RoundTrip | NoBackup | SeveralDevices | SimRunning | BadImage | BadSetting
                | PortBusy | Disabled | NoDevice | UsbHeat | CardCheck | ReadFirst
                | Incompatible | NotWritable => {}
            }
        }
        all
    }

    /// The app strips `Refused (code): ` from an error before it shows it
    /// (`errText` in `app/src/ipc/api.ts`), so the frame must stay `[a-z_]+`, and a code
    /// must not leak into the reason.
    #[test]
    fn a_refusal_prints_a_frame_the_app_can_strip() {
        let mut seen = std::collections::HashSet::new();
        for code in all_codes() {
            let name = code.as_str();
            assert!(seen.insert(name), "{name} is used twice");
            assert!(
                name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
                "{name}"
            );
            let json = serde_json::to_value(code).unwrap();
            assert_eq!(json, name, "the JSON code is the printed code");
            let r = Refusal::new(code, "Do the thing first.");
            assert_eq!(
                r.to_string(),
                format!("Refused ({name}): Do the thing first.")
            );
        }
    }

    #[test]
    fn a_minimal_device_loads() {
        let d: Device = serde_json::from_value(json!({"id": "fc-1", "kind": "fc"})).unwrap();
        assert_eq!(d.display_name(), "Unnamed FC");
        assert_eq!(d.identity, Identity::default());
    }

    #[test]
    fn a_refusal_reads_as_refused() {
        let e = anyhow::Error::new(Refusal::new(
            RefusalCode::UnknownVersion,
            "Betaflight 4.3.2 is not proven; QuadCam reads it but does not write it.",
        ));
        assert!(format!("{e:#}").starts_with("Refused (unknown_version): "));
        assert_eq!(
            e.downcast_ref::<Refusal>().unwrap().code,
            RefusalCode::UnknownVersion
        );
    }

    #[test]
    fn edits_are_tagged() {
        let e = Edit::FcSet {
            section: Section::RateProfile(2),
            name: "roll_rc_rate".into(),
            value: "7".into(),
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            json!({"kind": "fc_set", "section": {"kind": "rate_profile", "index": 2}, "name": "roll_rc_rate", "value": "7"})
        );
    }
}
