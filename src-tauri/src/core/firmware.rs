//! `Core`'s firmware methods (design 6.5, 7.5, 8): the version check, the splash preview, and
//! the flash of an EdgeTX radio.
//!
//! - **Check.** `gear_firmware` shows each saved device against the newest release. It reads
//!   the network only when asked, or when `firmwareCheck` is `daily` and the last answer is a
//!   day old.
//! - **Flash.** The same path as every write: `gear_flash_plan` downloads the release (or reads
//!   the cache), picks the board binary, patches the splash, runs every guard and returns a
//!   plan with a digest. `gear_flash` runs the guards again, needs the digest and `confirm`,
//!   reads the radio's current firmware over DFU and keeps it as a backup, then erases,
//!   writes, reads back, compares and leaves DFU. A mismatch stays in DFU.
//! - **Confirm.** The sheet's own Apply calls `gear_flash_click`. Any other caller needs the
//!   plan's digest and `confirm`, and with the app running the person also clicks Apply in the
//!   sheet (`Hooks::confirm_apply`).
//! - **Fail-safe.** `gear::firmware::Flasher` hands out the USB path; a process started by
//!   cargo gets a recorder unless `QUADCAM_FLASH=real`, and tests pass their own fake.

use super::Core;
use crate::gear::apply::{check, first_refusal, pass, ApplyReport, StepReport, StepState};
use crate::gear::blobs;
use crate::gear::compat::{self, Product};
use crate::gear::detect::{DfuInfo, STM32_DFU};
use crate::gear::dfu::{self, FLASH_BASE};
use crate::gear::firmware::check::{self as fwcheck, FirmwareStatus, Latest};
use crate::gear::firmware::edgetx::{self, HashSource};
use crate::gear::firmware::{Flasher, FwEnv};
use crate::gear::model::{
    ApplyPlan, ChangeStatus, Check, Device, DeviceKind, DiffItem, Refusal, RefusalCode,
    StagedChange, Trigger,
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

impl Core {
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
        Ok(FirmwareView {
            devices: fwcheck::statuses(&devices, &latest),
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
        Ok(self.flash_prepared(p)?.plan)
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
        let known = compat::check_writable(Product::Edgetx, board.as_deref(), Some(&target));
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
                "No radio is in DFU mode. Turn the radio off, hold both trim buttons toward the centre and plug in the USB cable.",
            ),
            n => fail(
                "One radio in DFU mode",
                RefusalCode::SeveralDevices,
                format!("{n} radios are in DFU mode; unplug all but one."),
            ),
        });

        warnings.push(
            "A radio in DFU mode shows no name. QuadCam writes the one radio in DFU mode; check it is the radio you picked.".into(),
        );
        warnings.push(
            "QuadCam reads the radio's current firmware over DFU first and keeps it as a backup. Flashing has not been tried on a real radio yet.".into(),
        );

        let ready = checks.iter().all(|c| c.ok);
        let digest = if ready {
            blobs::hash(
                format!(
                    "flash|{}|{}|{}|{}|{}",
                    device.id,
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

        // Back up the firmware the radio runs now. A failed read stops everything.
        let name = prep.device.display_name();
        let current = dfu::flash_size(usb.as_mut(), quick)
            .and_then(|n| dfu::read_flash(usb.as_mut(), FLASH_BASE, n, quick));
        let mut current = current.map_err(|e| {
            refuse(
                RefusalCode::NoBackup,
                format!("Reading the radio's firmware failed: {e:#}. Nothing was written."),
            )
        })?;
        while current.len() > 1 && current.last() == Some(&0xFF) {
            current.pop();
        }
        let taken = self
            .snapshots()
            .take_files(
                &prep.device.id,
                &prep.device.identity,
                Trigger::BeforeFlash,
                Utc::now(),
                &[("firmware.bin".to_string(), current.clone())],
                false,
            )
            .map_err(|e| {
                refuse(
                    RefusalCode::NoBackup,
                    format!("The backup of {name}'s firmware failed: {e:#}. Nothing was written."),
                )
            })?;
        steps.push(step(
            "Back up the current firmware",
            StepState::Done,
            Some(format!("{} KB, {}", current.len() / 1024, taken.backup.id)),
        ));

        let mut seen: Vec<dfu::Step> = Vec::new();
        let outcome = dfu::flash(usb.as_mut(), FLASH_BASE, &prep.image, quick, &mut |s| {
            seen.push(s)
        });
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
                        "The flash failed: {e:#} The radio stays in DFU mode: unplug it, enter DFU mode again and retry. The firmware it ran before is kept in the backup ({}).",
                        taken.backup.id
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
            backup: Some(taken.backup.id),
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
