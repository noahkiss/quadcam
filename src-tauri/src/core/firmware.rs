//! `Core`'s firmware methods (design 6.5, 7.5, 8): the version check, the splash preview, and
//! the flash of an EdgeTX radio.
//!
//! - **Check.** `gear_firmware` shows each saved device against the newest release. It reads
//!   the network only when asked, or when `firmwareCheck` is `daily` and the last answer is a
//!   day old.
//! - **Flash.** The same path as every write: `gear_flash_plan` downloads the release (or reads
//!   the cache), picks the board binary, patches the splash, runs every guard and returns a
//!   plan with a digest. `gear_flash` runs the guards again, needs the digest and `confirm`,
//!   reads the radio's current firmware over DFU twice (`dfu::read_verified`), keeps it as a
//!   firmware copy (`gear::fwcopy`) and reads the saved file back, and only then erases,
//!   writes (each 16 KB segment read back), reads back, compares and leaves DFU. A mismatch
//!   stays in DFU. `dfu::flash` takes the verified copy as an argument, so an erase without
//!   one does not compile.
//! - **Read.** `gear_firmware_read` is the read-only trial: the same verified read, saved as a
//!   copy, with the version string in the image compared with the radio's known version. It
//!   goes through `dfu::ReadOnly`, which refuses erase, write and leave.
//! - **Confirm.** The sheet's own Apply calls `gear_flash_click`. Any other caller needs the
//!   plan's digest and `confirm`, and with the app running the person also clicks Apply in the
//!   sheet (`Hooks::confirm_apply`).
//! - **One job per chip.** The flash and the read each run as a job on `dfu:<serial>`, so
//!   Gear status shows them and a second flash or read of that chip refuses with `port_busy`.
//! - **Fail-safe.** `gear::firmware::Flasher` hands out the USB path; a process started by
//!   cargo gets a recorder unless `QUADCAM_FLASH=real`, and tests pass their own fake.

use super::Core;
use crate::api::{Event, FirmwareReadProgress};
use crate::gear::apply::{check, first_refusal, pass, ApplyReport, StepReport, StepState};
use crate::gear::blobs;
use crate::gear::compat::{self, Product};
use crate::gear::detect::{DfuInfo, STM32_DFU};
use crate::gear::dfu::{self, FLASH_BASE};
use crate::gear::firmware::check::{self as fwcheck, FirmwareStatus, Latest};
use crate::gear::firmware::edgetx::{self, HashSource};
use crate::gear::firmware::{Flasher, FwEnv};
use crate::gear::fwcopy::{self, CopyKind, FwCopy};
use crate::gear::model::{
    ApplyPlan, ChangeStatus, Check, Device, DeviceKind, DiffItem, Refusal, RefusalCode,
    StagedChange,
};
use crate::gear::splash::{self, SplashParams, SplashPreview};
use anyhow::{anyhow, bail, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use specta::Type;

/// The id of the stand-in change a flash's sheet and report carry.
pub const FLASH_CHANGE: &str = "flash";

/// `gear_firmware`: whether to read the network.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct FirmwareParams {
    /// True: check now. False: show the last answer only. Unset: check only when
    /// `firmwareCheck` is `daily` and the last answer is a day old.
    #[serde(default)]
    pub check: Option<bool>,
}

/// The Firmware page: one row per device, and what the check found.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct FirmwareView {
    pub devices: Vec<FirmwareStatus>,
    pub latest: Latest,
    /// The `firmwareCheck` setting: `manual` or `daily`.
    pub mode: String,
}

/// `gear_flash_plan`: which radio, which EdgeTX version, which splash.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct FlashParams {
    /// The saved radio's device id.
    pub device: String,
    /// The EdgeTX version to flash; default the version the radio reports.
    #[serde(default)]
    pub version: Option<String>,
    /// A splash picture to put in the firmware.
    #[serde(default)]
    pub splash: Option<SplashParams>,
}

/// `gear_flash`: the same params, the plan's digest and the confirm.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct FlashRequest {
    #[serde(flatten)]
    pub params: FlashParams,
    pub digest: String,
    #[serde(default)]
    pub confirm: bool,
}

/// `gear_firmware_read`: which saved radio the DFU device is, when known.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct FirmwareReadParams {
    /// The saved radio's device id. Without it the copy is kept under `dfu-<serial>`.
    #[serde(default)]
    pub device: Option<String>,
}

/// What the read-only trial found.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct FirmwareRead {
    pub copy: FwCopy,
    /// The saved radio the copy belongs to, when one was named.
    pub device: Option<String>,
    /// The version QuadCam knows for that radio (from its last card or serial read).
    pub known_version: Option<String>,
    /// The flash size the DFU device reports, in bytes.
    pub flash_bytes: u64,
    /// True: the image names the version the radio reported. False: it names another.
    /// None: one of the two is unknown.
    pub matches: Option<bool>,
    /// What to tell the person, in plain words.
    pub message: String,
    /// What the read did, step by step. None of them writes the radio.
    pub steps: Vec<StepReport>,
}

struct Prepared {
    plan: ApplyPlan,
    device: Device,
    /// The image to write; empty while a check fails.
    image: Vec<u8>,
    dfu: Option<DfuInfo>,
}

fn refuse(code: RefusalCode, reason: impl Into<String>) -> anyhow::Error {
    Refusal::new(code, reason).into()
}

fn fail(name: &str, code: RefusalCode, reason: impl Into<String>) -> Check {
    check(name, Err(Refusal::new(code, reason)))
}

/// The job handle of a radio in DFU mode: its chip's serial.
pub fn dfu_handle(info: &DfuInfo) -> String {
    format!("dfu:{}", info.serial.as_deref().unwrap_or_default())
}

impl Core {
    /// Registers a flash or a read as the one job on this DFU chip, and holds its link. A job
    /// already on the chip refuses with `port_busy`.
    fn dfu_job(
        &self,
        info: &DfuInfo,
        device: Option<&str>,
        step: &str,
    ) -> Result<(super::backup::JobGuard<'_>, super::gear::Hold<'_>)> {
        let handle = dfu_handle(info);
        let busy = |what: &str| {
            refuse(
                RefusalCode::PortBusy,
                format!(
                    "{what} is already running on the radio in DFU mode. Wait for it to end."
                ),
            )
        };
        if let Some(j) = self.gear_jobs().into_iter().find(|j| j.handle == handle) {
            return Err(busy(&j.step));
        }
        let (job, _) = self
            .job_start(&handle, device, step)
            .map_err(|_| busy("Another job"))?;
        let hold = self.gear_hold(&super::gear::link_handle(
            &crate::gear::model::Link::Dfu {
                vid: info.vid,
                pid: info.pid,
                serial: info.serial.clone(),
            },
        ));
        Ok((job, hold))
    }

    /// Replaces the network and the USB path the firmware code uses (tests pass fakes).
    pub fn with_firmware_env(mut self, env: FwEnv) -> Core {
        self.firmware = env;
        self
    }

    /// Each saved device against the newest release. Reads the network only as the params and
    /// the `firmwareCheck` setting say.
    pub fn gear_firmware(&self, p: &FirmwareParams) -> Result<FirmwareView> {
        let cache = self.cache.clone();
        let mode = self.gear_settings().firmware_check;
        let now = Utc::now();
        let last = fwcheck::cached(&cache);
        let latest = if p.check.unwrap_or_else(|| fwcheck::due(&mode, &last, now)) {
            fwcheck::check(self.firmware.fetch.as_ref(), &cache, now)?
        } else {
            last
        };
        let devices = self.gear_store().devices()?;
        let mut statuses = fwcheck::statuses(&devices, &latest);
        crate::gear::firmware::betaflight::adjust_statuses(&mut statuses, self.bf_flash_on());
        Ok(FirmwareView {
            devices: statuses,
            latest,
            mode,
        })
    }

    /// A picture at the radio's size and depth. Reads the file only.
    pub fn gear_splash(&self, p: &SplashParams) -> Result<SplashPreview> {
        Ok(splash::preview(p)?.1)
    }

    /// What a flash would do, with every guard's result. Downloads the release when it is not
    /// in the cache; writes no device.
    pub fn gear_flash_plan(&self, p: &FlashParams) -> Result<ApplyPlan> {
        if self.is_fc(&p.device)? {
            return self.bf_flash_plan(p);
        }
        Ok(self.flash_prepared(p)?.plan)
    }

    /// True when the saved device is an FC: its flash is Betaflight's (`core/bf_flash.rs`).
    fn is_fc(&self, device: &str) -> Result<bool> {
        Ok(self
            .gear_store()
            .devices()?
            .iter()
            .any(|d| d.id == device && d.kind == DeviceKind::Fc))
    }

    fn flash_prepared(&self, p: &FlashParams) -> Result<Prepared> {
        let device = self
            .gear_store()
            .devices()?
            .into_iter()
            .find(|d| d.id == p.device)
            .ok_or_else(|| anyhow!("No saved device {}.", p.device))?;
        if device.kind != DeviceKind::Radio {
            return Err(refuse(
                RefusalCode::Incompatible,
                "QuadCam flashes the firmware of EdgeTX radios only.",
            ));
        }
        let board = device.identity.board.clone();
        let installed = device.identity.version.clone();
        let target = p
            .version
            .clone()
            .map(|v| v.trim().trim_start_matches(['v', 'V']).to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| installed.clone())
            .ok_or_else(|| anyhow!("The radio reports no version; name the version to flash."))?;

        let mut checks: Vec<Check> = Vec::new();
        let known = compat::check_writable(Product::EdgetxFlash, board.as_deref(), Some(&target));
        let known_ok = known.is_ok();
        checks.push(check("Known board and version", known));

        let mut splash_mono = None;
        if let Some(sp) = &p.splash {
            let board_name = board.clone().unwrap_or_default();
            let layout = if !splash::board_supported(&board_name) {
                Err(splash::unsupported(&board_name))
            } else {
                compat::check_writable(Product::Splash, board.as_deref(), Some(&target))
            };
            let layout_ok = layout.is_ok();
            checks.push(check("Splash layout", layout));
            if layout_ok {
                let (mono, _) = splash::preview(sp)?;
                splash_mono = Some(mono);
            }
        }
        let splash_ready = p.splash.is_none() || splash_mono.is_some();

        // Download and unpack only for a pair QuadCam has proven.
        let mut image: Vec<u8> = Vec::new();
        let mut diff = vec![DiffItem::Version {
            label: "EdgeTX firmware".into(),
            before: installed.clone(),
            after: target.clone(),
        }];
        let mut warnings: Vec<String> = Vec::new();
        let mut image_sha = String::new();
        if known_ok && splash_ready {
            let rel = edgetx::obtain(self.firmware.fetch.as_ref(), &self.cache, &target)?;
            match edgetx::board_binary(&rel, board.as_deref().unwrap_or_default()) {
                Ok(bin) => {
                    checks.push(pass("Firmware image"));
                    let mut bytes = bin.bytes.clone();
                    let mut put = vec![
                        format!("{} ({} KB)", bin.name, bytes.len() / 1024),
                        format!("SHA-256 {}", bin.sha256),
                    ];
                    warnings.push(match rel.hash_source {
                        HashSource::Release => {
                            "The download matches the SHA-256 the EdgeTX release lists.".into()
                        }
                        HashSource::FirstDownload => format!(
                            "The release lists no SHA-256. QuadCam recorded {} at the first download.",
                            rel.sha256
                        ),
                    });
                    if let Some(mono) = &splash_mono {
                        match splash::patch(&bytes, mono) {
                            Ok(patched) => {
                                checks.push(pass("Splash markers"));
                                bytes = patched;
                                put.push(format!(
                                    "Splash: {} dark pixels of 8192 (hash {})",
                                    mono.dark_count(),
                                    blobs::hash(&mono.pack())
                                ));
                            }
                            Err(e) => checks.push(check("Splash markers", Err(e.refusal()))),
                        }
                    }
                    image_sha = blobs::hash(&bytes);
                    diff.push(DiffItem::Files {
                        label: "Firmware image".into(),
                        put,
                        delete: Vec::new(),
                    });
                    image = bytes;
                }
                Err(e) => checks.push(check("Firmware image", Err(e.downcast::<Refusal>()?))),
            }
        }

        checks.push(if device.last_backup.is_some() {
            pass("Card backup")
        } else {
            fail(
                "Card backup",
                RefusalCode::NoBackup,
                "Back up this radio first: connect it in USB Storage mode and back it up. A flash can change how the radio reads its card.",
            )
        });

        let dfu: Vec<DfuInfo> = (self.gear.dfu)()
            .into_iter()
            .filter(|d| (d.vid, d.pid) == STM32_DFU)
            .collect();
        checks.push(match dfu.len() {
            1 => pass("One radio in DFU mode"),
            0 => fail(
                "One radio in DFU mode",
                RefusalCode::NoDevice,
                "No radio is in DFU mode. Turn the radio off, then plug in the USB cable (do not hold the trim buttons, which start the EdgeTX bootloader instead). Wait a few seconds.",
            ),
            n => fail(
                "One radio in DFU mode",
                RefusalCode::SeveralDevices,
                format!("{n} radios are in DFU mode; unplug all but one."),
            ),
        });

        if let [one] = dfu.as_slice() {
            let saved = self.gear_store().devices().unwrap_or_default();
            let owner = one
                .serial
                .as_deref()
                .and_then(|s| saved.iter().find(|d| d.dfu_serial.as_deref() == Some(s)));
            match (owner, device.dfu_serial.as_deref()) {
                (Some(o), _) if o.id == device.id => {
                    checks.push(pass("The radio in DFU mode is this radio"));
                }
                (Some(o), _) => checks.push(fail(
                    "The radio in DFU mode is this radio",
                    RefusalCode::DeviceChanged,
                    format!(
                        "The radio in DFU mode is linked to {}, not {}. Pick that radio, or unlink it (gear_dfu_link).",
                        o.display_name(),
                        device.display_name()
                    ),
                )),
                (None, Some(_)) => checks.push(fail(
                    "The radio in DFU mode is this radio",
                    RefusalCode::DeviceChanged,
                    format!(
                        "{} is linked to another DFU device than the one plugged in. Unplug the other radio, or unlink {} (gear_dfu_link) and plan again.",
                        device.display_name(),
                        device.display_name()
                    ),
                )),
                (None, None) => {
                    let last = super::dfu_link::last_seen_radio(&saved);
                    warnings.push(match last {
                        Some(l) if l.id != device.id => format!(
                            "This DFU device is not linked to a radio yet. {} was seen most recently, not {}; check you picked the radio in DFU mode. A verified flash links it to {}.",
                            l.display_name(),
                            device.display_name(),
                            device.display_name()
                        ),
                        _ => format!(
                            "This DFU device is not linked to a radio yet. A verified flash links it to {}.",
                            device.display_name()
                        ),
                    });
                }
            }
        }
        warnings.push(
            "A radio in DFU mode shows no name. QuadCam writes the one radio in DFU mode; check it is the radio you picked.".into(),
        );
        warnings.push(
            "QuadCam reads the radio's current firmware over DFU twice first and keeps it as a copy; it erases nothing until both reads agree and the saved file reads back. If a flash fails, the radio's ROM bootloader still answers over USB (turn it off, plug in USB) and you can flash again. Flashing has not been tried on a real radio yet.".into(),
        );

        let ready = checks.iter().all(|c| c.ok);
        let digest = if ready {
            blobs::hash(
                format!(
                    "flash|{}|{}|{}|{}|{}|{}",
                    device.id,
                    dfu.first()
                        .and_then(|d| d.serial.clone())
                        .unwrap_or_default(),
                    board.clone().unwrap_or_default(),
                    installed.clone().unwrap_or_default(),
                    target,
                    image_sha
                )
                .as_bytes(),
            )
        } else {
            String::new()
        };
        Ok(Prepared {
            plan: ApplyPlan {
                change: FLASH_CHANGE.into(),
                device: device.identity.clone(),
                checks,
                diff,
                digest,
                warnings,
            },
            device,
            image,
            dfu: dfu.into_iter().next(),
        })
    }

    /// The read-only trial: reads all of the flash of the one radio in DFU mode twice, saves it
    /// as a firmware copy and compares the version the image names with the radio's known
    /// version. Nothing here erases, writes or restarts the radio: the transport is
    /// `dfu::ReadOnly`. Needs no digest and no confirm for that reason.
    pub fn gear_firmware_read(&self, p: &FirmwareReadParams) -> Result<FirmwareRead> {
        let saved = self.gear_store().devices()?;
        let device = match &p.device {
            Some(id) => {
                let d = saved
                    .iter()
                    .find(|d| &d.id == id)
                    .ok_or_else(|| anyhow!("No saved device {id}."))?;
                if d.kind != DeviceKind::Radio {
                    return Err(refuse(
                        RefusalCode::Incompatible,
                        "Reading firmware over DFU is for EdgeTX radios.",
                    ));
                }
                Some(d.clone())
            }
            None => None,
        };
        let dfus: Vec<DfuInfo> = (self.gear.dfu)()
            .into_iter()
            .filter(|d| (d.vid, d.pid) == STM32_DFU)
            .collect();
        let info = match dfus.as_slice() {
            [one] => one.clone(),
            [] => {
                return Err(refuse(
                    RefusalCode::NoDevice,
                    "No radio is in DFU mode. Turn the radio off, then plug in the USB cable (do not hold the trim buttons, which start the EdgeTX bootloader instead). Wait a few seconds and read again.",
                ))
            }
            n => {
                return Err(refuse(
                    RefusalCode::SeveralDevices,
                    format!("{} radios are in DFU mode; unplug all but one.", n.len()),
                ))
            }
        };
        let _job = self.dfu_job(&info, device.as_ref().map(|d| d.id.as_str()), "Reading firmware")?;
        let flasher: &dyn Flasher = self.firmware.flasher.as_ref();
        let quick = flasher.quick();
        let mut usb = flasher
            .open_dfu(info.vid, info.pid, info.serial.as_deref())
            .map_err(|e| {
                refuse(
                    RefusalCode::Disabled,
                    format!("Cannot open the radio's DFU device: {e:#}"),
                )
            })?;
        let mut steps = Vec::new();
        let flash_bytes = dfu::flash_size(usb.as_mut(), quick)? as u64;
        steps.push(StepReport {
            name: "Open the DFU device".into(),
            state: StepState::Done,
            detail: Some(format!("{} KB of flash", flash_bytes / 1024)),
        });
        let hooks = self.hooks.clone();
        let mut last = 0usize;
        let copy = dfu::read_verified(usb.as_mut(), quick, &mut |done, total| {
            // About 100 events for a 2 MB read.
            if done == total || done >= last + 20 * 1024 {
                last = done;
                hooks.event(Event::FirmwareRead(FirmwareReadProgress {
                    done: done as u64,
                    total: total as u64,
                }));
            }
        })
        .map_err(|e| {
            refuse(
                RefusalCode::NoBackup,
                format!(
                    "Reading the radio's firmware failed: {e:#}. Nothing was written to the radio."
                ),
            )
        })?;
        steps.push(StepReport {
            name: "Read the flash twice".into(),
            state: StepState::Done,
            detail: Some("both reads equal".into()),
        });
        if copy.is_blank() {
            return Err(refuse(
                RefusalCode::BadImage,
                "The radio's flash reads as blank. There is no firmware to copy. Flash firmware to bring it back.",
            ));
        }
        let bytes = copy.trimmed();
        let identity = edgetx::image_identity(&bytes);
        let key = device
            .as_ref()
            .map(|d| d.id.clone())
            .unwrap_or_else(|| format!("dfu-{}", info.serial.clone().unwrap_or_default()));
        let saved_copy = fwcopy::save(
            &self.gear_store(),
            &key,
            CopyKind::Read,
            Utc::now(),
            &bytes,
            identity.clone(),
        )
        .map_err(|e| anyhow!("Saving the copy failed: {e:#}. Nothing was written to the radio."))?;
        steps.push(StepReport {
            name: "Save the copy".into(),
            state: StepState::Done,
            detail: Some(format!("{} KB, {}", bytes.len() / 1024, saved_copy.id)),
        });
        let known = device.as_ref().and_then(|d| d.identity.version.clone());
        let image = identity.as_ref().map(|(_, v)| v.clone());
        // The board the image names, when it is not the named radio's board.
        let other_board = match (&device, &identity) {
            (Some(d), Some((b, _)))
                if !edgetx::same_board(d.identity.board.as_deref().unwrap_or_default(), b) =>
            {
                Some(b.clone())
            }
            _ => None,
        };
        let matches = match (&known, &image) {
            (Some(k), Some(i)) => Some(k.trim().trim_start_matches(['v', 'V']) == i),
            _ => None,
        };
        let message = match (&image, &known, matches) {
            (Some(i), Some(_), Some(true)) => {
                format!("The radio's firmware is EdgeTX {i}, the version QuadCam knows for it. The copy is saved.")
            }
            (Some(i), Some(k), _) => format!(
                "The firmware names EdgeTX {i}, but QuadCam knows the radio as {k}. Read the version in the radio's own About screen. The copy is saved."
            ),
            (Some(i), None, _) => format!(
                "The firmware names EdgeTX {i}. QuadCam has no version to compare it with. The copy is saved."
            ),
            (None, _, _) => "The copy is saved, but it holds no EdgeTX version string. This may not be EdgeTX firmware.".into(),
        };
        let message = match (&other_board, &device) {
            (Some(b), Some(d)) => format!(
                "The firmware is for board {b}, not {}: the radio in DFU mode may not be {}. A flash of {} refuses it. {message}",
                d.identity.board.as_deref().unwrap_or("(none)"),
                d.display_name(),
                d.display_name()
            ),
            _ => message,
        };
        self.hooks.gear_changed();
        Ok(FirmwareRead {
            copy: saved_copy,
            device: device.map(|d| d.id),
            known_version: known,
            flash_bytes,
            matches,
            message,
            steps,
        })
    }

    /// Flashes the planned image. Needs the plan's digest and `confirm`; with the app running,
    /// the person also clicks Apply in the sheet.
    pub fn gear_flash(&self, req: &FlashRequest) -> Result<ApplyReport> {
        self.flash_at(req, false)
    }

    /// The apply sheet's own Apply click: the click is the confirm.
    pub fn gear_flash_click(&self, req: &FlashRequest) -> Result<ApplyReport> {
        self.flash_at(req, true)
    }

    /// `gear_flash` with the confirm source given.
    pub fn flash_at(&self, req: &FlashRequest, from_gui: bool) -> Result<ApplyReport> {
        if self.is_fc(&req.params.device)? {
            return self.bf_flash_at(req, from_gui);
        }
        if !req.confirm {
            bail!("Refused: a flash needs the plan's digest and confirm=true.");
        }
        let prep = self.flash_prepared(&req.params)?;
        if let Some(r) = first_refusal(&prep.plan.checks) {
            return Err(r.into());
        }
        if prep.plan.digest != req.digest {
            return Err(refuse(
                RefusalCode::BeforeMismatch,
                "The firmware or the radio changed since the plan; plan again.",
            ));
        }
        if !from_gui {
            self.hooks
                .confirm_apply(&flash_change(&prep.device, &prep.plan), &prep.plan)?;
        }
        let dfu_info = prep.dfu.clone().expect("one DFU device passed its check");
        let _job = self.dfu_job(&dfu_info, Some(&prep.device.id), "Flashing")?;
        let flasher: &dyn Flasher = self.firmware.flasher.as_ref();
        let quick = flasher.quick();
        let mut usb = flasher
            .open_dfu(dfu_info.vid, dfu_info.pid, dfu_info.serial.as_deref())
            .map_err(|e| {
                refuse(
                    RefusalCode::Disabled,
                    format!("Cannot open the radio's DFU device: {e:#}"),
                )
            })?;
        let mut steps: Vec<StepReport> = Vec::new();
        let step = |name: &str, state: StepState, detail: Option<String>| StepReport {
            name: name.into(),
            state,
            detail,
        };

        let name = prep.device.display_name();
        let no_copy =
            |why: String| refuse(RefusalCode::NoBackup, format!("{why} Nothing was written."));
        // The device must be the chip the board expects, before any command that changes it.
        let want = edgetx::spec(prep.device.identity.board.as_deref().unwrap_or_default())
            .map(|s| s.flash_bytes)
            .unwrap_or_default();
        let have = dfu::flash_size(usb.as_mut(), quick)
            .map_err(|e| no_copy(format!("Reading the DFU memory layout failed: {e:#}.")))?;
        if want == 0 || have != want {
            return Err(refuse(
                RefusalCode::Incompatible,
                format!("The DFU device has {have} bytes of flash; this radio's chip has {want}. Nothing was written."),
            ));
        }

        // Copy the firmware the radio runs now: two reads that agree, saved, read back from
        // disk. Only then may anything be erased. A blank flash (an interrupted flash)
        // has nothing to copy and may be flashed.
        let copy = dfu::read_verified(usb.as_mut(), quick, &mut |_, _| {})
            .map_err(|e| no_copy(format!("Reading the radio's firmware failed: {e:#}.")))?;
        let current = copy.trimmed();
        let blank = copy.is_blank();
        let identity = edgetx::image_identity(&current);
        // A radio in DFU mode shows no name, and two radios can share a chip. The firmware it
        // runs names its board: another board is another radio, also before any link exists.
        if let Some((image_board, _)) = &identity {
            let board = prep.device.identity.board.as_deref().unwrap_or_default();
            if !edgetx::same_board(board, image_board) {
                return Err(refuse(
                    RefusalCode::DeviceChanged,
                    format!(
                        "The radio in DFU mode runs firmware for board {image_board}, not {board}: it is not {name}. Plug in {name}, or pick the radio in DFU mode. Nothing was erased."
                    ),
                ));
            }
        }
        let kept = if blank {
            None
        } else {
            Some(
                fwcopy::save(
                    &self.gear_store(),
                    &prep.device.id,
                    CopyKind::BeforeFlash,
                    Utc::now(),
                    &current,
                    identity,
                )
                .map_err(|e| {
                    no_copy(format!(
                        "Saving the copy of {name}'s firmware failed: {e:#}."
                    ))
                })?,
            )
        };
        steps.push(step(
            "Copy the current firmware",
            StepState::Done,
            Some(match &kept {
                Some(c) => format!("{} KB, read twice, saved as {}", current.len() / 1024, c.id),
                None => "the flash is blank; there is no firmware to keep".into(),
            }),
        ));

        let mut seen: Vec<dfu::Step> = Vec::new();
        let outcome = dfu::flash(
            usb.as_mut(),
            FLASH_BASE,
            &prep.image,
            &copy,
            quick,
            &mut |s| seen.push(s),
        );
        let labels = [
            (dfu::Step::Erase, "Erase"),
            (dfu::Step::Write, "Write"),
            (dfu::Step::ReadBack, "Read back"),
            (dfu::Step::Leave, "Leave DFU"),
        ];
        let (status, message) = match &outcome {
            Ok(o) => {
                for (s, label) in labels {
                    steps.push(step(
                        label,
                        StepState::Done,
                        match s {
                            dfu::Step::Erase => Some(format!("{} sectors", o.erased)),
                            dfu::Step::Write => Some(format!("{} KB", o.written / 1024)),
                            dfu::Step::ReadBack => Some(format!("{} KB, equal", o.verified / 1024)),
                            dfu::Step::Leave => None,
                        },
                    ));
                }
                if prep.device.dfu_serial.is_none() {
                    if let Some(serial) = dfu_info.serial.clone().filter(|s| !s.is_empty()) {
                        let mut d = prep.device.clone();
                        d.dfu_serial = Some(serial);
                        let _ = self.gear_store().save_device(&d);
                    }
                }
                (
                    ChangeStatus::Verified,
                    format!(
                        "Verified: {name} holds the new firmware byte for byte and restarted. Connect it in USB Storage mode to check the version."
                    ),
                )
            }
            Err(e) => {
                // The steps begun: the last one failed, the rest were not reached.
                let failed = seen.len().saturating_sub(1);
                for (i, (_, label)) in labels.iter().enumerate() {
                    let state = match i.cmp(&failed) {
                        std::cmp::Ordering::Less => StepState::Done,
                        std::cmp::Ordering::Equal => StepState::Failed,
                        std::cmp::Ordering::Greater => StepState::Skipped,
                    };
                    steps.push(step(
                        label,
                        state,
                        (state == StepState::Failed).then(|| format!("{e:#}")),
                    ));
                }
                (
                    ChangeStatus::Failed,
                    format!(
                        "The flash failed: {e:#} The radio stays in DFU mode. Flash again, or unplug it, turn it off, plug it in again and retry. The radio's ROM bootloader always answers over USB. The firmware it ran before is kept ({}).",
                        kept.as_ref().map_or("it was blank".to_string(), |c| c.id.clone())
                    ),
                )
            }
        };
        self.hooks.gear_changed();
        let ok = outcome.is_ok();
        Ok(ApplyReport {
            change: FLASH_CHANGE.into(),
            device: prep.device.id.clone(),
            status,
            steps,
            backup: kept.map(|c| c.id),
            after_backup: None,
            sent: Vec::new(),
            failed_line: None,
            verify: Vec::new(),
            saved: ok,
            files: Vec::new(),
            message,
            notes: prep.plan.warnings.clone(),
            at: Utc::now(),
        })
    }
}

/// The stand-in change the sheet and an agent's confirm request carry.
pub fn flash_change(device: &Device, plan: &ApplyPlan) -> StagedChange {
    StagedChange {
        id: FLASH_CHANGE.into(),
        device: device.id.clone(),
        title: "Flash firmware".into(),
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
