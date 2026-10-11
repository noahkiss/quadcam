//! `Core`'s ExpressLRS jobs (design 6.4), a preview behind the `elrsPreview` setting.
//!
//! - **Read.** `gear_elrs_read` hands a saved radio's USB port to its internal module, or a
//!   saved FC's to the receiver on its serial-receiver UART, pings the device over CRSF,
//!   reads its parameters and saves what it found: the device (kind, target name, version)
//!   and a snapshot of its options under `<gear>/elrs/`.
//! - **Options.** Staged like any change (`Edit::ElrsOptions`), applied by `gear_apply`: the
//!   job reads the device again, refuses when a value moved since the read, backs up the
//!   parameters, writes each one over CRSF, reads them back and compares.
//! - **Flash.** `gear_elrs_flash_plan` downloads the official release (the person's action;
//!   its SHA-256 is recorded and shown, or checked against a digest the person gives), picks
//!   the device's target, configures the image (the binding phrase becomes a UID; only a
//!   fingerprint of it is ever shown) and runs every guard. `gear_elrs_flash` needs the
//!   plan's digest and `confirm`, puts the device in its bootloader, saves its current
//!   firmware, writes with the `esptool` module and reads the verdict.
//!
//! A radio or an FC stays in passthrough after a job until it is restarted or unplugged; every
//! report says so. Nothing here has touched a real device: tests use `gear::elrs::fake`.

use super::apply::refusal;
use super::{link_handle, Core};
use crate::gear::apply::{check, first_refusal, pass, ApplyReport, StepReport, StepState};
use crate::gear::blobs;
use crate::gear::elrs::crsf::{self, Param, WriteValue};
use crate::gear::elrs::flash::{self, Built};
use crate::gear::elrs::image::{self, Side};
use crate::gear::elrs::link::{self, CrsfLink, ModuleStart, Timing};
use crate::gear::elrs::{self as elrs, ElrsSet, ElrsSnapshot};
use crate::gear::firmware::check::{self as fwcheck, FirmwareStatus};
use crate::gear::model::{
    ApplyPlan, ChangeStatus, Check, Device, DeviceKind, DiffItem, DiffLine, Edit, Identity, LineOp,
    Refusal, RefusalCode, StagedChange, Trigger,
};
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::time::Duration;

/// The id of the stand-in change a flash's sheet and report carry.
pub const ELRS_FLASH_CHANGE: &str = "elrs-flash";

const OFF: &str =
    "ELRS tools (preview) are off. Turn them on in Settings > Gear, or set elrs_preview=true.";

/// `gear_elrs`: the ELRS devices QuadCam knows, and what a job needs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct ElrsParams {
    /// True: read the newest ExpressLRS release from the network now.
    #[serde(default)]
    pub check: Option<bool>,
}

/// A radio or an FC an ExpressLRS device can be read through.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct ElrsHost {
    pub device: String,
    pub name: String,
    pub kind: DeviceKind,
}

/// A saved ELRS device with what the last read found.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct ElrsDeviceView {
    pub snapshot: ElrsSnapshot,
    pub name: String,
    pub status: Option<FirmwareStatus>,
    /// Staged changes waiting for this device.
    pub staged: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct ElrsView {
    /// The `elrsPreview` setting. Every job refuses while it is off.
    pub preview: bool,
    /// The installed `esptool` module's version, if any.
    pub esptool: Option<String>,
    pub latest: Option<String>,
    pub hosts: Vec<ElrsHost>,
    pub devices: Vec<ElrsDeviceView>,
    /// Whether a binding phrase is set. Never the phrase.
    pub phrase_set: bool,
    pub region: String,
    /// Seconds without a link before a flashed device starts its WiFi; 0 is never. 60 when unset.
    pub wifi_interval: u32,
    pub warnings: Vec<String>,
}

/// `gear_elrs_read`: the radio or FC the device sits behind.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct ElrsReadParams {
    /// The saved radio (its internal module) or FC (its receiver).
    pub host: String,
    /// The host's serial port; omit when only one is plugged in.
    #[serde(default)]
    pub port: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct ElrsReadReport {
    pub device: String,
    pub snapshot: ElrsSnapshot,
    pub notes: Vec<String>,
}

/// `gear_elrs_flash_plan`: which saved ELRS device, which release.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct ElrsFlashParams {
    pub device: String,
    /// The ExpressLRS version to flash; default the newest release the last check found.
    #[serde(default)]
    pub version: Option<String>,
    /// The release zip's SHA-256, when the person has it from a trusted place. Without it
    /// the plan shows the digest of what it downloaded.
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub port: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct ElrsFlashRequest {
    #[serde(flatten)]
    pub params: ElrsFlashParams,
    pub digest: String,
    #[serde(default)]
    pub confirm: bool,
}

struct Prefs {
    preview: bool,
    phrase: Option<String>,
    region: String,
    wifi: Option<u32>,
}

struct FlashPrep {
    plan: ApplyPlan,
    device: Device,
    host: Device,
    snapshot: ElrsSnapshot,
    built: Option<Built>,
    version: String,
}

fn fail(name: &str, code: RefusalCode, reason: impl Into<String>) -> Check {
    check(name, Err(Refusal::new(code, reason)))
}

fn step(name: &str, state: StepState, detail: Option<String>) -> StepReport {
    StepReport {
        name: name.into(),
        state,
        detail,
    }
}

fn side_of(kind: DeviceKind) -> Option<Side> {
    match kind {
        DeviceKind::ElrsTx => Some(Side::Tx),
        DeviceKind::ElrsRx => Some(Side::Rx),
        _ => None,
    }
}

impl Core {
    fn elrs_prefs(&self) -> Prefs {
        let values = self
            .settings_file
            .as_deref()
            .map(|f| crate::settings::read(f).unwrap_or_default())
            .unwrap_or_default();
        Prefs {
            preview: values
                .get("elrsPreview")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            phrase: values
                .get("elrsBindingPhrase")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            region: values
                .get("elrsRegion")
                .and_then(|v| v.as_str())
                .filter(|r| *r == "FCC" || *r == "LBT")
                .unwrap_or("FCC")
                .to_string(),
            wifi: match values.get("elrsWifiInterval").and_then(|v| v.as_u64()) {
                Some(0) => None,
                Some(n) => Some(n as u32),
                None => Some(60),
            },
        }
    }

    pub(super) fn elrs_require(&self) -> Result<()> {
        if self.elrs_prefs().preview {
            Ok(())
        } else {
            bail!("Refused: {OFF}")
        }
    }

    fn elrs_timing(&self) -> Timing {
        if self.fc_timing().quiet < Duration::from_millis(50) {
            Timing::fast()
        } else {
            Timing::default()
        }
    }

    fn elrs_root(&self) -> std::path::PathBuf {
        self.gear_store().root().to_path_buf()
    }

    /// The ELRS devices, the radios and FCs to read through, and what a flash needs.
    pub fn gear_elrs(&self, p: &ElrsParams) -> Result<ElrsView> {
        let prefs = self.elrs_prefs();
        let store = self.gear_store();
        let devices = store.devices()?;
        let last = fwcheck::cached(&self.cache);
        let latest = if prefs.preview && p.check == Some(true) {
            fwcheck::check(self.firmware.fetch.as_ref(), &self.cache, Utc::now())?
        } else {
            last
        };
        let counts = self.gear_staged_counts();
        let root = self.elrs_root();
        let mut views = Vec::new();
        for d in devices.iter().filter(|d| side_of(d.kind).is_some()) {
            let Some(snapshot) = elrs::load_snapshot(&root, &d.id) else {
                continue;
            };
            views.push(ElrsDeviceView {
                snapshot,
                name: d.display_name(),
                status: fwcheck::statuses(std::slice::from_ref(d), &latest)
                    .into_iter()
                    .next(),
                staged: counts.get(&d.id).copied().unwrap_or(0) as u32,
            });
        }
        let mut warnings = Vec::new();
        let majors: Vec<(String, u64)> = views
            .iter()
            .filter_map(|v| {
                Some((
                    v.snapshot.role.clone(),
                    elrs::major(v.snapshot.version.as_deref()?)?,
                ))
            })
            .collect();
        if let (Some(tx), Some(rx)) = (
            majors.iter().find(|m| m.0 == "tx"),
            majors.iter().find(|m| m.0 == "rx"),
        ) {
            if tx.1 != rx.1 {
                warnings.push(format!(
                    "A transmitter on ExpressLRS {} and a receiver on {} do not link. Flash the receiver first, then the transmitter.",
                    tx.1, rx.1
                ));
            }
        }
        Ok(ElrsView {
            preview: prefs.preview,
            esptool: self
                .modules
                .list()
                .into_iter()
                .find(|m| m.name == "esptool")
                .and_then(|m| m.installed.map(|i| i.version)),
            latest: latest.elrs,
            hosts: devices
                .iter()
                .filter(|d| matches!(d.kind, DeviceKind::Radio | DeviceKind::Fc))
                .map(|d| ElrsHost {
                    device: d.id.clone(),
                    name: d.display_name(),
                    kind: d.kind,
                })
                .collect(),
            devices: views,
            phrase_set: prefs.phrase.is_some(),
            region: prefs.region,
            wifi_interval: prefs.wifi.unwrap_or(0),
            warnings,
        })
    }

    /// The serial port of a saved FC: the one asked for, or the only FC. The FC must identify
    /// itself as `host`: another device, or one that does not answer MSP, refuses.
    fn elrs_fc_port(&self, host: &Device, port: Option<&str>) -> Result<String> {
        let c = self.gear_fc_pick(port)?;
        let handle = link_handle(&c.link);
        let mut id =
            c.id.clone()
                .or_else(|| self.gear_fc_seen(&handle).and_then(|i| i.id));
        if id.is_none() && (self.gear.holders)(&handle).is_empty() {
            let _hold = self.gear_hold(&handle);
            if let Ok(i) =
                crate::gear::bf::identify(self.gear.ports.as_ref(), &handle, self.fc_timing())
            {
                id = i.id.clone();
                self.fc_state.lock().unwrap().seen.insert(handle.clone(), i);
            }
        }
        match id {
            Some(i) if i == host.id || host.aliases.contains(&i) => Ok(handle),
            Some(_) => Err(refusal(Refusal::new(
                RefusalCode::DeviceChanged,
                format!(
                    "The FC on {handle} is not {}. Pick that FC's device, or unplug the other FC.",
                    host.display_name()
                ),
            ))),
            None => Err(refusal(Refusal::new(
                RefusalCode::DeviceChanged,
                format!(
                    "QuadCam cannot tell whether the FC on {handle} is {}: it did not identify itself over MSP. Unplug it and plug it in again (a passthrough from an earlier job ends then), or close the program that holds the port.",
                    host.display_name()
                ),
            ))),
        }
    }

    fn elrs_host_port(&self, host: &Device, port: Option<&str>) -> Result<String> {
        match host.kind {
            DeviceKind::Radio => self.radio_pick(port),
            DeviceKind::Fc => self.elrs_fc_port(host, port),
            other => bail!("A {} is not a host for ExpressLRS.", other.label()),
        }
    }

    fn elrs_host(&self, id: &str) -> Result<Device> {
        let host = self
            .gear_store()
            .device(id)?
            .with_context(|| format!("Unknown device {id:?}. See `gear devices`."))?;
        if !matches!(host.kind, DeviceKind::Radio | DeviceKind::Fc) {
            bail!(
                "{} is a {}. Pick the radio (for its internal module) or the FC (for its receiver).",
                host.display_name(),
                host.kind.label()
            );
        }
        Ok(host)
    }

    /// Hands the host's port to the device behind it. Returns the CRSF link and, for a radio,
    /// the board its `ver` named; with `expect_board` a radio of another board refuses first.
    fn elrs_open(
        &self,
        host: &Device,
        port: &str,
        start: ModuleStart,
        expect_board: Option<&str>,
        notes: &mut Vec<String>,
    ) -> Result<(CrsfLink, Option<String>)> {
        let t = self.elrs_timing();
        let ports = self.gear.ports.as_ref();
        let (raw, board) = match host.kind {
            DeviceKind::Radio => {
                let baud = if start == ModuleStart::Run {
                    link::MODULE_BAUD
                } else {
                    link::FLASH_BAUD
                };
                notes.push(
                    "The radio's module has its pulses off until the radio restarts: restart the radio when you are done."
                        .into(),
                );
                link::radio_passthrough(ports, port, baud, start, expect_board, &t)?
            }
            _ => {
                let (l, r) = link::fc_passthrough(ports, port, link::RECEIVER_BAUD, &t)?;
                notes.push(format!(
                    "The FC passes its receiver UART ({}) through until you unplug USB: unplug it and plug it in again when you are done.",
                    r.uart
                ));
                (l, None)
            }
        };
        Ok((CrsfLink::new(raw, t), board))
    }

    /// Finds the device behind the host and reads its parameters.
    fn elrs_read_live(
        &self,
        host: &Device,
        port: &str,
        notes: &mut Vec<String>,
    ) -> Result<(crsf::DeviceInfo, Vec<Param>, Option<String>)> {
        let want = if host.kind == DeviceKind::Radio {
            crsf::ADDR_TX
        } else {
            crsf::ADDR_RX
        };
        let (mut c, board) = self.elrs_open(host, port, ModuleStart::Run, None, notes)?;
        let found = c.ping()?;
        let info = found.into_iter().find(|d| d.origin == want).ok_or_else(|| {
            anyhow!(
                "No ExpressLRS {} answered on {port}. {} Restart the {} and try again.",
                if want == crsf::ADDR_TX { "module" } else { "receiver" },
                if host.kind == DeviceKind::Fc {
                    "The receiver needs power from the FC (a battery may be needed) and a CRSF link."
                } else {
                    "The radio's internal module must be on."
                },
                if host.kind == DeviceKind::Fc { "FC" } else { "radio" },
            )
        })?;
        let params = c.read_params(&info)?;
        Ok((info, params, board))
    }

    /// Reads the ELRS device behind a saved radio or FC and saves what it found.
    pub fn gear_elrs_read(&self, p: &ElrsReadParams) -> Result<ElrsReadReport> {
        self.elrs_require()?;
        let host = self.elrs_host(&p.host)?;
        let port = self.elrs_host_port(&host, p.port.as_deref())?;
        let mut notes = Vec::new();
        let (info, params, host_board) = {
            let _hold = self.gear_hold(&port);
            self.elrs_read_live(&host, &port, &mut notes)?
        };
        let kind = if info.origin == crsf::ADDR_TX {
            DeviceKind::ElrsTx
        } else {
            DeviceKind::ElrsRx
        };
        let (version, source) = elrs::detect_version(&info, &params);
        if version.is_none() {
            notes.push(
                "The device did not list a version QuadCam can read. Enter it by hand with `gear devices save`."
                    .into(),
            );
        }
        let id = elrs::device_id(kind, &host.id, &info.name);
        let store = self.gear_store();
        let ident = Identity {
            board: None,
            firmware: Some("ExpressLRS".into()),
            version: version.clone(),
            build: None,
            target: Some(info.name.clone()),
        };
        let saved = store.seen(&id, kind, &ident)?;
        let snapshot = ElrsSnapshot {
            device: id.clone(),
            role: if kind == DeviceKind::ElrsTx {
                "tx"
            } else {
                "rx"
            }
            .into(),
            host: host.id.clone(),
            name: info.name.clone(),
            target: Some(info.name.clone()),
            version,
            version_source: source,
            options: elrs::options_of(&params),
            params: elrs::param_views(&params),
            read_at: Utc::now(),
            host_board,
        };
        elrs::save_snapshot(store.root(), &snapshot)?;
        self.hooks.gear_changed();
        notes.push(
            "The version and the options come from the device's parameters. Reading a device this way has not been checked on a real one yet."
                .into(),
        );
        Ok(ElrsReadReport {
            device: saved.id,
            snapshot,
            notes,
        })
    }

    // ----- staged options -----

    /// Checks a staged ELRS change against the last read.
    pub(super) fn check_elrs_stageable(&self, device: &str, edits: &[Edit]) -> Result<()> {
        self.elrs_require()?;
        let snap = elrs::load_snapshot(&self.elrs_root(), device).ok_or_else(|| {
            anyhow!("Read this device first (`gear elrs read`): its options are not known yet.")
        })?;
        let mut sets: Vec<ElrsSet> = Vec::new();
        for e in edits {
            match e {
                Edit::ElrsOptions { options } => sets.extend(options.iter().cloned()),
                _ => bail!("An ELRS device takes only ELRS option changes."),
            }
        }
        if sets.is_empty() {
            bail!("The change sets no option.");
        }
        elrs::resolve(&snap.options, &sets)
            .map(|_| ())
            .map_err(|e| refusal(Refusal::new(RefusalCode::BadSetting, format!("{e:#}"))))
    }

    fn elrs_sets(&self, change: &StagedChange) -> Vec<ElrsSet> {
        change
            .edits
            .iter()
            .flat_map(|e| match e {
                Edit::ElrsOptions { options } => options.clone(),
                _ => Vec::new(),
            })
            .collect()
    }

    /// The plan of a staged ELRS options change. Reads the saved read, not the device.
    pub(super) fn plan_elrs_options(&self, change: &StagedChange) -> Result<ApplyPlan> {
        let device = self
            .gear_store()
            .device(&change.device)?
            .with_context(|| format!("Unknown device {:?}.", change.device))?;
        let mut checks = vec![];
        checks.push(if self.elrs_prefs().preview {
            pass("ELRS tools (preview) are on")
        } else {
            fail("ELRS tools (preview) are on", RefusalCode::Disabled, OFF)
        });
        let snap = elrs::load_snapshot(&self.elrs_root(), &change.device);
        let Some(snap) = snap else {
            checks.push(fail(
                "The device was read",
                RefusalCode::NoBackup,
                "Read this device first (`gear elrs read`).",
            ));
            return Ok(plan_of(
                change,
                &device,
                checks,
                Vec::new(),
                String::new(),
                Vec::new(),
            ));
        };
        checks.push(pass("The device was read"));
        let resolved = elrs::resolve(&snap.options, &self.elrs_sets(change));
        let mut lines = Vec::new();
        let mut digest = String::new();
        match resolved {
            Ok(w) => {
                checks.push(pass("The options exist and the values are in range"));
                for (o, _, to) in &w {
                    if &o.value == to {
                        lines.push(DiffLine {
                            op: LineOp::Same,
                            text: format!("{}: {} (already set)", o.label, o.value),
                        });
                    } else {
                        lines.push(DiffLine {
                            op: LineOp::Remove,
                            text: format!("{}: {}", o.label, o.value),
                        });
                        lines.push(DiffLine {
                            op: LineOp::Add,
                            text: format!("{}: {}", o.label, to),
                        });
                    }
                }
                let basis = format!(
                    "elrs-options|{}|{}|{}|{}",
                    device.id,
                    snap.read_at.to_rfc3339(),
                    snap.version.clone().unwrap_or_default(),
                    w.iter()
                        .map(|(o, _, to)| format!("{}={}", o.key, to))
                        .collect::<Vec<_>>()
                        .join(",")
                );
                digest = blobs::hash(basis.as_bytes());
            }
            Err(e) => checks.push(fail(
                "The options exist and the values are in range",
                RefusalCode::BadSetting,
                format!("{e:#}"),
            )),
        }
        let host = self.gear_store().device(&snap.host)?;
        checks.push(match &host {
            Some(h) => match self.elrs_host_port(h, None) {
                Ok(_) => pass(&format!("{} is plugged in on serial", h.display_name())),
                Err(e) => fail(
                    &format!("{} is plugged in on serial", h.display_name()),
                    RefusalCode::NoDevice,
                    format!("{e:#}"),
                ),
            },
            None => fail(
                "The radio or FC it sits behind is saved",
                RefusalCode::NoDevice,
                "The radio or FC this device was read through is no longer saved.",
            ),
        });
        let warnings = vec![
            "QuadCam reads the device again first and refuses when an option moved since the read.".into(),
            "Writing ExpressLRS options over a passthrough has not been checked on a real device yet.".into(),
        ];
        Ok(plan_of(
            change,
            &device,
            checks,
            vec![DiffItem::Lines {
                label: format!("{} options", device.display_name()),
                lines,
            }],
            digest,
            warnings,
        ))
    }

    /// Applies a staged ELRS options change: read, compare, back up, write, read back, verify.
    pub(super) fn apply_elrs_options(
        &self,
        change: &StagedChange,
        digest: &str,
        from_gui: bool,
    ) -> Result<ApplyReport> {
        let plan = self.plan_elrs_options(change)?;
        if let Some(r) = first_refusal(&plan.checks) {
            return Err(refusal(r));
        }
        if plan.digest != digest {
            return Err(refusal(Refusal::new(
                RefusalCode::BeforeMismatch,
                "The plan changed since you read it; plan again.",
            )));
        }
        if !from_gui {
            self.hooks.confirm_apply(change, &plan)?;
        }
        let device = self
            .gear_store()
            .device(&change.device)?
            .context("The device is gone.")?;
        let snap =
            elrs::load_snapshot(&self.elrs_root(), &device.id).context("No read of the device.")?;
        let host = self.elrs_host(&snap.host)?;
        let port = self.elrs_host_port(&host, None)?;
        let writes = elrs::resolve(&snap.options, &self.elrs_sets(change))?;

        let mut steps: Vec<StepReport> = Vec::new();
        let mut notes = plan.warnings.clone();
        let _hold = self.gear_hold(&port);
        let (mut c, _) = self.elrs_open(&host, &port, ModuleStart::Run, None, &mut notes)?;
        let want = side_of(device.kind)
            .map(|s| {
                if s == Side::Tx {
                    crsf::ADDR_TX
                } else {
                    crsf::ADDR_RX
                }
            })
            .unwrap_or(crsf::ADDR_TX);
        let info = c
            .ping()?
            .into_iter()
            .find(|d| d.origin == want)
            .ok_or_else(|| anyhow!("The device did not answer; nothing was written."))?;
        if info.name != snap.name {
            return Err(refusal(Refusal::new(
                RefusalCode::DeviceChanged,
                format!(
                    "The device now calls itself `{}`, not `{}`. Nothing was written; read it again.",
                    info.name, snap.name
                ),
            )));
        }
        let mut live = c.read_params(&info)?;
        let live_opts = elrs::options_of(&live);
        // The value, the id and the list must be the read's: a choice is written as its index.
        for (o, _, _) in &writes {
            let now = live_opts.iter().find(|l| l.key == o.key);
            let Some(l) = now.filter(|l| (&l.value, l.id) == (&o.value, o.id)) else {
                return Err(refusal(Refusal::new(
                    RefusalCode::BeforeMismatch,
                    format!(
                        "{} is {} on the device, not {} as last read. Nothing was written; read it again.",
                        o.label,
                        now.map(|l| l.value.as_str()).unwrap_or("missing"),
                        o.value
                    ),
                )));
            };
            if (&l.choices, l.min, l.max) != (&o.choices, o.min, o.max) {
                return Err(refusal(Refusal::new(
                    RefusalCode::BeforeMismatch,
                    format!(
                        "{} offers other values on the device than at the read. Nothing was written; read it again.",
                        o.label
                    ),
                )));
            }
        }
        steps.push(step(
            "Read the device",
            StepState::Done,
            Some(format!("{} parameters", live.len())),
        ));

        let before = serde_json::to_vec_pretty(&elrs::param_views(&live))?;
        let taken = self
            .snapshots()
            .take_files(
                &device.id,
                &device.identity,
                Trigger::BeforeApply,
                Utc::now(),
                &[("elrs/parameters.json".to_string(), before)],
                false,
            )
            .map_err(|e| {
                refusal(Refusal::new(
                    RefusalCode::NoBackup,
                    format!("The backup of the parameters failed: {e:#}. Nothing was written."),
                ))
            })?;
        steps.push(step(
            "Back up the parameters",
            StepState::Done,
            Some(taken.backup.id.clone()),
        ));

        // The device saves a parameter a moment after the write.
        let save_wait = if self.elrs_timing().settle.is_zero() {
            Duration::ZERO
        } else {
            Duration::from_millis(800)
        };
        let mut failed: Option<String> = None;
        let mut wrote = false;
        for (o, w, to) in &writes {
            let set = format!("Set {}", o.label);
            // One write can change another parameter's list (a packet rate changes the switch
            // modes): after a write, read again and take the index from the list as it is now.
            if wrote {
                std::thread::sleep(save_wait);
                match c.read_params(&info) {
                    Ok(l) => live = l,
                    Err(e) => {
                        failed = Some(format!("The read after a write failed: {e:#}"));
                        steps.push(step(&set, StepState::Failed, failed.clone()));
                        break;
                    }
                }
            }
            let opts = elrs::options_of(&live);
            let Some(now) = opts.iter().find(|l| l.key == o.key) else {
                failed = Some(format!("The device no longer lists {}", o.label));
                steps.push(step(&set, StepState::Failed, failed.clone()));
                break;
            };
            if &now.value == to {
                continue;
            }
            let w = match w {
                WriteValue::Index(_) => match now.choices.iter().position(|x| x == to) {
                    Some(i) => WriteValue::Index(i as u8),
                    None => {
                        failed = Some(format!(
                            "{} no longer offers {to} after the earlier write",
                            o.label
                        ));
                        steps.push(step(&set, StepState::Failed, failed.clone()));
                        break;
                    }
                },
                n => n.clone(),
            };
            let Some(p) = live.iter().find(|p| p.id == now.id) else {
                failed = Some(format!("{} has no parameter {}", o.label, now.id));
                break;
            };
            let bytes = crsf::encode_value(p, &w)?;
            match c.write_value(want, now.id, &bytes) {
                Ok(()) => {
                    wrote = true;
                    steps.push(step(&set, StepState::Done, Some(to.clone())));
                }
                Err(e) => {
                    steps.push(step(&set, StepState::Failed, Some(format!("{e:#}"))));
                    failed = Some(format!("{e:#}"));
                    break;
                }
            }
        }
        // Give the last write time, then read back.
        std::thread::sleep(save_wait);
        let after = c.read_params(&info)?;
        drop(c);
        let after_opts = elrs::options_of(&after);
        let mut mismatches = Vec::new();
        for (o, _, to) in &writes {
            let now = after_opts
                .iter()
                .find(|l| l.key == o.key)
                .map(|l| l.value.clone());
            if now.as_deref() != Some(to.as_str()) {
                mismatches.push(format!(
                    "{}: wanted {}, found {}",
                    o.label,
                    to,
                    now.unwrap_or_else(|| "nothing".into())
                ));
            }
        }
        let verified = failed.is_none() && mismatches.is_empty();
        steps.push(step(
            "Read back",
            if failed.is_some() {
                StepState::Skipped
            } else {
                StepState::Done
            },
            Some(format!("{} parameters", after.len())),
        ));
        steps.push(step(
            "Verify",
            if verified {
                StepState::Done
            } else {
                StepState::Failed
            },
            (!mismatches.is_empty()).then(|| mismatches.join("; ")),
        ));
        let new_snap = ElrsSnapshot {
            options: after_opts,
            params: elrs::param_views(&after),
            read_at: Utc::now(),
            ..snap
        };
        elrs::save_snapshot(&self.elrs_root(), &new_snap)?;
        let name = device.display_name();
        let (status, message) = if verified {
            (
                ChangeStatus::Verified,
                format!("Verified: {name} reports the new options."),
            )
        } else {
            (
                ChangeStatus::Failed,
                format!(
                    "The change did not verify: {}. The parameters before it are in the backup ({}).",
                    failed.clone().unwrap_or_else(|| mismatches.join("; ")),
                    taken.backup.id
                ),
            )
        };
        let report = ApplyReport {
            change: change.id.clone(),
            device: device.id.clone(),
            status,
            steps,
            backup: Some(taken.backup.id),
            after_backup: None,
            sent: Vec::new(),
            failed_line: None,
            verify: Vec::new(),
            saved: verified,
            files: Vec::new(),
            message,
            notes,
            at: Utc::now(),
        };
        self.finish_change(change, &report)?;
        Ok(report)
    }

    // ----- flash -----

    pub fn gear_elrs_flash_plan(&self, p: &ElrsFlashParams) -> Result<ApplyPlan> {
        Ok(self.elrs_flash_prepared(p)?.plan)
    }

    fn elrs_flash_prepared(&self, p: &ElrsFlashParams) -> Result<FlashPrep> {
        self.elrs_require()?;
        let prefs = self.elrs_prefs();
        let store = self.gear_store();
        let device = store
            .device(&p.device)?
            .with_context(|| format!("No saved device {}.", p.device))?;
        let Some(side) = side_of(device.kind) else {
            return Err(refusal(Refusal::new(
                RefusalCode::Incompatible,
                "QuadCam flashes ExpressLRS transmitter modules and receivers only.",
            )));
        };
        let snapshot = elrs::load_snapshot(store.root(), &device.id).ok_or_else(|| {
            anyhow!("Read this device first (`gear elrs read`): its target name decides the image.")
        })?;
        let host = self.elrs_host(&snapshot.host)?;
        let installed = device.identity.version.clone();
        let version = p
            .version
            .clone()
            .map(|v| v.trim().trim_start_matches(['v', 'V']).to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| fwcheck::cached(&self.cache).elrs)
            .ok_or_else(|| anyhow!("Name the version to flash (the newest release is not known yet; check firmware first)."))?;

        let mut checks: Vec<Check> = vec![pass("ELRS tools (preview) are on")];
        let mut warnings: Vec<String> = Vec::new();
        let mut diff: Vec<DiffItem> = vec![DiffItem::Version {
            label: "ExpressLRS firmware".into(),
            before: installed.clone(),
            after: version.clone(),
        }];

        let esptool = self.modules.tool("esptool").ok().flatten();
        checks.push(if esptool.is_some() {
            pass("The esptool module is installed")
        } else {
            fail(
                "The esptool module is installed",
                RefusalCode::Disabled,
                "Install esptool in Settings > Modules (quadcam-cli modules install esptool).",
            )
        });
        let uid = prefs.phrase.as_deref().map(elrs::uid::uid_of);
        checks.push(match uid {
            Some(_) => pass("A binding phrase is set"),
            None => fail(
                "A binding phrase is set",
                RefusalCode::BadSetting,
                "Set the binding phrase first (quadcam-cli settings set elrs_binding_phrase=...). A device flashed without one would not bind to your others.",
            ),
        });

        if host.kind == DeviceKind::Radio {
            checks.push(match &snapshot.host_board {
                Some(b) => pass(&format!("The read names the radio's board ({b})")),
                None => fail(
                    "The read names the radio's board",
                    RefusalCode::ReadFirst,
                    "Read this device again (`gear elrs read`): the read did not record the radio's board, which a flash checks first.",
                ),
            });
        }

        let port = match self.elrs_host_port(&host, p.port.as_deref()) {
            Ok(port) => {
                checks.push(pass(&format!(
                    "{} is plugged in on serial",
                    host.display_name()
                )));
                Some(port)
            }
            Err(e) => {
                checks.push(fail(
                    &format!("{} is plugged in on serial", host.display_name()),
                    RefusalCode::NoDevice,
                    format!("{e:#}"),
                ));
                None
            }
        };

        let mut built: Option<Built> = None;
        let mut bundle_sha = String::new();
        let can_build = esptool.is_some() && uid.is_some();
        if can_build {
            match image::obtain(
                self.firmware.fetch.as_ref(),
                &self.cache,
                &version,
                p.sha256.as_deref(),
            ) {
                Ok(bundle) => {
                    bundle_sha = bundle.sha256.clone();
                    warnings.push(if bundle.pinned {
                        "The download matches the SHA-256 you gave.".into()
                    } else {
                        format!(
                            "ExpressLRS publishes no SHA-256 for its releases. QuadCam recorded {} at the download; compare it with a trusted copy, or plan again with sha256.",
                            bundle.sha256
                        )
                    });
                    let targets = std::fs::read(bundle.root.join("hardware").join("targets.json"))
                        .map_err(|e| anyhow!("The release has no targets.json: {e}"))
                        .and_then(|b| image::parse_targets(&b));
                    match targets {
                        Ok(ts) => match image::find_target(&ts, &snapshot.name, side)
                            .and_then(|t| flash::check_target(&t, &version).map(|pl| (t, pl)))
                        {
                            Ok((t, platform)) => {
                                checks.push(pass("A known target and version"));
                                // The discriminator only tells builds apart. It comes from
                                // the release, the device and the UID, so a plan and its apply
                                // build the same image.
                                let disc = xxhash_rust::xxh64::xxh64(
                                    format!(
                                        "{}|{}|{}",
                                        bundle.sha256,
                                        device.id,
                                        elrs::uid::fingerprint(&uid.expect("checked above"))
                                    )
                                    .as_bytes(),
                                    0,
                                ) as u32;
                                match flash::build(
                                    &bundle,
                                    &t,
                                    platform,
                                    &prefs.region,
                                    uid.expect("checked above"),
                                    prefs.wifi,
                                    disc,
                                ) {
                                    Ok(b) => {
                                        checks.push(pass("The release image"));
                                        built = Some(b);
                                    }
                                    Err(r) => checks.push(check("The release image", Err(r))),
                                }
                            }
                            Err(r) => checks.push(check("A known target and version", Err(r))),
                        },
                        Err(e) => checks.push(fail(
                            "A known target and version",
                            RefusalCode::BadImage,
                            format!("{e:#}"),
                        )),
                    }
                }
                Err(e) => checks.push(fail(
                    "The release download",
                    RefusalCode::BadImage,
                    format!("{e:#}"),
                )),
            }
        }

        // The pair must stay on one major.
        if let Some(new_major) = elrs::major(&version) {
            let other_role = if side == Side::Tx { "rx" } else { "tx" };
            for d in store
                .devices()?
                .iter()
                .filter(|d| side_of(d.kind).is_some() && d.id != device.id)
            {
                if let Some(s) =
                    elrs::load_snapshot(store.root(), &d.id).filter(|s| s.role == other_role)
                {
                    if let Some(m) = s.version.as_deref().and_then(elrs::major) {
                        if m != new_major {
                            warnings.push(format!(
                                "{} runs ExpressLRS {m}; with {version} on {} they will not link. Flash both in one sitting, receiver first.",
                                d.display_name(),
                                device.display_name()
                            ));
                        }
                    }
                }
            }
        }

        let mut image_sha = String::new();
        if let Some(b) = &built {
            image_sha.clone_from(&b.image_sha);
            let mut put = vec![format!(
                "{} ({} on {}), {}",
                b.target.product_name, b.target.firmware, b.target.platform, b.region
            )];
            for f in &b.files {
                put.push(format!(
                    "{} at {} ({} KB, SHA-256 {})",
                    f.name,
                    f.offset,
                    f.bytes.len() / 1024,
                    f.sha256
                ));
            }
            if let Some(u) = uid {
                put.push(format!(
                    "Binding: UID fingerprint {} (the phrase is not shown)",
                    elrs::uid::fingerprint(&u)
                ));
            }
            put.push(match prefs.wifi {
                Some(s) => format!("WiFi starts after {s} s without a link"),
                None => "WiFi stays off".into(),
            });
            diff.push(DiffItem::Files {
                label: "Firmware image".into(),
                put,
                delete: Vec::new(),
            });
        }

        warnings.push(
            "QuadCam reads the chip's current flash with esptool first and keeps it as a backup. Flashing ExpressLRS this way has not been tried on a real device yet.".into(),
        );
        warnings.push("ExpressLRS 4 wipes a receiver's Options page; note your settings first, and read the device again afterwards.".into());
        warnings.push(match host.kind {
            DeviceKind::Radio => "Afterwards, restart the radio.".into(),
            _ => "Afterwards, unplug the FC's USB cable and power-cycle the quad.".into(),
        });

        let ready = checks.iter().all(|c| c.ok);
        let digest = if ready {
            blobs::hash(
                format!(
                    "elrs-flash|{}|{}|{}|{}|{}|{}|{}|{}",
                    device.id,
                    host.id,
                    snapshot.host_board.clone().unwrap_or_default(),
                    snapshot.name,
                    version,
                    bundle_sha,
                    image_sha,
                    snapshot.read_at.to_rfc3339(),
                )
                .as_bytes(),
            )
        } else {
            String::new()
        };
        let _ = port;
        Ok(FlashPrep {
            plan: ApplyPlan {
                change: ELRS_FLASH_CHANGE.into(),
                device: device.identity.clone(),
                checks,
                diff,
                digest,
                warnings,
            },
            device,
            host,
            snapshot,
            built,
            version,
        })
    }

    pub fn gear_elrs_flash(&self, req: &ElrsFlashRequest) -> Result<ApplyReport> {
        self.elrs_flash_at(req, false)
    }

    /// The apply sheet's own Apply click: the click is the confirm.
    pub fn gear_elrs_flash_click(&self, req: &ElrsFlashRequest) -> Result<ApplyReport> {
        self.elrs_flash_at(req, true)
    }

    fn run_esptool(&self, args: &[std::ffi::OsString], what: &str) -> Result<String> {
        let out = self
            .modules
            .run("esptool", args, Duration::from_secs(900))
            .with_context(|| format!("{what}: esptool did not run"))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if !out.status.success() {
            let tail: String = text
                .chars()
                .rev()
                .take(400)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            bail!("{what}: esptool stopped ({}). {}", out.status, tail.trim());
        }
        Ok(text)
    }

    fn elrs_flash_at(&self, req: &ElrsFlashRequest, from_gui: bool) -> Result<ApplyReport> {
        if !req.confirm {
            bail!("Refused: a flash needs the plan's digest and confirm=true.");
        }
        let prep = self.elrs_flash_prepared(&req.params)?;
        if let Some(r) = first_refusal(&prep.plan.checks) {
            return Err(refusal(r));
        }
        if prep.plan.digest != req.digest {
            return Err(refusal(Refusal::new(
                RefusalCode::BeforeMismatch,
                "The release or the device changed since the plan; plan again.",
            )));
        }
        if !from_gui {
            self.hooks
                .confirm_apply(&elrs_flash_change(&prep.device, &prep.plan), &prep.plan)?;
        }
        let built = prep.built.as_ref().expect("a ready plan has an image");
        let port = self.elrs_host_port(&prep.host, req.params.port.as_deref())?;
        let name = prep.device.display_name();
        let mut steps: Vec<StepReport> = Vec::new();
        let mut notes = prep.plan.warnings.clone();
        let _hold = self.gear_hold(&port);

        // 1. The device must be the one planned, then its bootloader. A radio must be the board
        // the read went through (its `ver`, before the pulses stop); an ESP32 in its ROM
        // bootloader cannot answer CRSF, and the radio stays in passthrough, so its module
        // cannot be pinged first. A receiver answers a CRSF ping on the same passthrough, then
        // prints its target as it restarts into its bootloader.
        let expect = (prep.host.kind == DeviceKind::Radio)
            .then(|| prep.snapshot.host_board.clone())
            .flatten();
        let (mut c, board) = self.elrs_open(
            &prep.host,
            &port,
            ModuleStart::Bootloader,
            expect.as_deref(),
            &mut notes,
        )?;
        if prep.host.kind == DeviceKind::Fc {
            let info = c
                .ping()?
                .into_iter()
                .find(|d| d.origin == crsf::ADDR_RX)
                .ok_or_else(|| {
                    refusal(Refusal::new(
                        RefusalCode::DeviceChanged,
                        "The receiver did not answer a CRSF ping, so QuadCam cannot tell it is the one planned. Nothing was written.",
                    ))
                })?;
            if info.name != prep.snapshot.name {
                return Err(refusal(Refusal::new(
                    RefusalCode::DeviceChanged,
                    format!(
                        "The receiver now calls itself `{}`, not `{}`. Nothing was written; read it again.",
                        info.name, prep.snapshot.name
                    ),
                )));
            }
            steps.push(step("Ping the receiver", StepState::Done, Some(info.name)));
            let said = c.enter_bootloader()?;
            if !flash::bootloader_names(&said, &built.target) {
                return Err(refusal(Refusal::new(
                    RefusalCode::DeviceChanged,
                    format!(
                        "The receiver in its bootloader says `{}`, which does not name {}. Nothing was written.",
                        said.trim(),
                        built.target.product_name
                    ),
                )));
            }
            steps.push(step(
                "Restart the receiver into its bootloader",
                StepState::Done,
                Some(said.trim().to_string()),
            ));
        } else {
            steps.push(step(
                "Check the radio",
                StepState::Done,
                board.map(|b| format!("board {b}")),
            ));
            steps.push(step(
                "Start the module with its boot pin held",
                StepState::Done,
                None,
            ));
        }
        drop(c);

        // 2. Stage the files and keep the current flash.
        let work = self.cache.join("run").join(format!(
            "elrs-{}",
            crate::gear::store::safe(&prep.device.id)
        ));
        let _ = std::fs::remove_dir_all(&work);
        flash::stage_files(built, &work)?;
        // esptool's `change_baud` must land on the speed the host's UART runs: the radio's
        // module UART is set to FLASH_BAUD by its passthrough, the FC's receiver UART stays at
        // RECEIVER_BAUD.
        let esp_baud = if prep.host.kind == DeviceKind::Radio {
            link::FLASH_BAUD
        } else {
            link::RECEIVER_BAUD
        };
        let baud = esp_baud.to_string();
        let current = work.join("current.bin");
        let read_args: Vec<std::ffi::OsString> = [
            "--chip",
            built.chip,
            "--port",
            &port,
            "--baud",
            &baud,
            "--before",
            "no-reset",
            "--after",
            "no-reset",
            "read-flash",
            "0",
            "ALL",
        ]
        .iter()
        .map(std::ffi::OsString::from)
        .chain([current.clone().into_os_string()])
        .collect();
        let backup_id = match self.run_esptool(&read_args, "Reading the current flash") {
            Ok(_) => {
                let bytes = std::fs::read(&current).map_err(|e| {
                    refusal(Refusal::new(
                        RefusalCode::NoBackup,
                        format!("esptool read no flash file: {e}. Nothing was written."),
                    ))
                })?;
                let taken = self
                    .snapshots()
                    .take_files(
                        &prep.device.id,
                        &prep.device.identity,
                        Trigger::BeforeFlash,
                        Utc::now(),
                        &[("firmware.bin".to_string(), bytes.clone())],
                        false,
                    )
                    .map_err(|e| {
                        refusal(Refusal::new(
                            RefusalCode::NoBackup,
                            format!("The backup of {name}'s firmware failed: {e:#}. Nothing was written."),
                        ))
                    })?;
                steps.push(step(
                    "Back up the current firmware",
                    StepState::Done,
                    Some(format!("{} KB, {}", bytes.len() / 1024, taken.backup.id)),
                ));
                taken.backup.id
            }
            Err(e) => {
                return Err(refusal(Refusal::new(
                    RefusalCode::NoBackup,
                    format!("{e:#} Nothing was written."),
                )));
            }
        };

        // 3. Write. esptool compares the flash with the data it sent before it finishes.
        let write_args = flash::esptool_args(built, &port, esp_baud, &work);
        let outcome = self.run_esptool(&write_args, "Writing the firmware");
        let _ = std::fs::remove_dir_all(&work);
        let (status, message) = match &outcome {
            Ok(text) if text.to_ascii_lowercase().contains("hash of data verified") => {
                steps.push(step(
                    "Write",
                    StepState::Done,
                    Some(format!("{} files", built.files.len())),
                ));
                steps.push(step(
                    "Verify",
                    StepState::Done,
                    Some("esptool: hash of data verified".into()),
                ));
                let mut d = prep.device.clone();
                d.identity.version = Some(prep.version.clone());
                let _ = self.gear_store().save_device(&d);
                (
                    ChangeStatus::Verified,
                    format!(
                        "Verified: {name} holds ExpressLRS {} (esptool compared the flash with the image). {} Then read it again.",
                        prep.version,
                        if prep.host.kind == DeviceKind::Radio {
                            "Restart the radio."
                        } else {
                            "Unplug the FC and power-cycle the quad."
                        }
                    ),
                )
            }
            Ok(_) => {
                steps.push(step("Write", StepState::Done, None));
                steps.push(step(
                    "Verify",
                    StepState::Failed,
                    Some("esptool finished but did not report its hash check".into()),
                ));
                (
                    ChangeStatus::Failed,
                    format!(
                        "esptool finished but did not report that it verified the flash. Read {name} again before you trust it. The firmware it ran before is in the backup ({backup_id})."
                    ),
                )
            }
            Err(e) => {
                steps.push(step("Write", StepState::Failed, Some(format!("{e:#}"))));
                (
                    ChangeStatus::Failed,
                    format!(
                        "The flash failed: {e:#} The device may stay in its bootloader: power-cycle it and flash again. The firmware it ran before is in the backup ({backup_id})."
                    ),
                )
            }
        };
        self.hooks.gear_changed();
        let ok = status == ChangeStatus::Verified;
        Ok(ApplyReport {
            change: ELRS_FLASH_CHANGE.into(),
            device: prep.device.id.clone(),
            status,
            steps,
            backup: Some(backup_id),
            after_backup: None,
            sent: Vec::new(),
            failed_line: None,
            verify: Vec::new(),
            saved: ok,
            files: Vec::new(),
            message,
            notes,
            at: Utc::now(),
        })
    }
}

fn plan_of(
    change: &StagedChange,
    device: &Device,
    checks: Vec<Check>,
    diff: Vec<DiffItem>,
    digest: String,
    warnings: Vec<String>,
) -> ApplyPlan {
    let ready = checks.iter().all(|c| c.ok);
    ApplyPlan {
        change: change.id.clone(),
        device: device.identity.clone(),
        checks,
        diff,
        digest: if ready { digest } else { String::new() },
        warnings,
    }
}

/// The stand-in change the sheet and an agent's confirm request carry.
pub fn elrs_flash_change(device: &Device, plan: &ApplyPlan) -> StagedChange {
    StagedChange {
        id: ELRS_FLASH_CHANGE.into(),
        device: device.id.clone(),
        title: "Flash ExpressLRS".into(),
        status: ChangeStatus::Ready,
        edits: Vec::new(),
        base_backup: String::new(),
        editor: crate::session::Editor::User,
        note: plan.warnings.join(" "),
        order: 0,
        history: Vec::new(),
        reverts: None,
    }
}
