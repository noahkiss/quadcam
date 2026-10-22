//! `Core`'s restore of a saved EdgeTX firmware copy (`gear::fwcopy`) to its radio over DFU.
//!
//! - **List.** `gear_firmware_copies` lists each radio's copies, newest first: its own folder
//!   and the `dfu-<serial>` folder of the DFU chip linked to it. Reads files only.
//! - **Plan.** `gear_firmware_restore_plan` runs every guard and returns a plan with a digest:
//!   the copy reads back with its recorded SHA-256, it names the radio's board, it passes the
//!   image checks a release flash passes (size, the bootloader's and the firmware's vector
//!   tables, its own version string), the radio has a card backup, and exactly one radio is
//!   in DFU mode and it is this radio. Writes nothing.
//! - **Apply.** `gear_firmware_restore` runs the guards again, needs the digest and `confirm`,
//!   and with the app running the person's click (`Hooks::confirm_apply`), as a flash does.
//!   It then goes through the flash's own engine (`Core::dfu_write`): two reads of the
//!   firmware the radio runs now, kept as a copy unless a saved copy already holds it byte
//!   for byte, then erase, write, read back, compare and leave DFU. A mismatch stays in DFU.
//! - **Fail-safe.** The USB path is the flash's `Flasher`: a process started by cargo gets a
//!   recorder unless `QUADCAM_FLASH=real`.

use super::firmware::DfuWrite;
use super::Core;
use crate::gear::apply::{check, first_refusal, pass, ApplyReport};
use crate::gear::blobs;
use crate::gear::compat::{self, Product};
use crate::gear::detect::DfuInfo;
use crate::gear::firmware::edgetx;
use crate::gear::fwcopy::{self, FwCopy};
use crate::gear::model::{
    ApplyPlan, ChangeStatus, Check, Device, DeviceKind, DiffItem, Refusal, RefusalCode,
    StagedChange,
};
use crate::gear::radios;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use specta::Type;

/// The id of the stand-in change a restore's sheet and report carry.
pub const RESTORE_CHANGE: &str = "firmware-restore";

/// `gear_firmware_copies`: whose copies.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct FirmwareCopiesParams {
    /// A saved radio's device id. Omitted: every radio's copies, and copies of DFU devices
    /// not linked to a radio.
    #[serde(default)]
    pub device: Option<String>,
}

/// One saved copy and the radio it belongs to.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct FirmwareCopy {
    /// The saved radio; none for a copy of a DFU device not linked to a radio.
    pub radio: Option<String>,
    pub copy: FwCopy,
}

/// `gear_firmware_restore_plan`: which radio, which copy.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct FirmwareRestoreParams {
    /// The saved radio's device id.
    pub device: String,
    /// The copy's id (`<device>/<stamp>-<kind>`) from `gear_firmware_copies`.
    pub copy: String,
}

/// `gear_firmware_restore`: the same params, the plan's digest and the confirm.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct FirmwareRestoreRequest {
    #[serde(flatten)]
    pub params: FirmwareRestoreParams,
    pub digest: String,
    #[serde(default)]
    pub confirm: bool,
}

struct Prepared {
    plan: ApplyPlan,
    device: Device,
    copy: FwCopy,
    /// The copy's image; empty while a check fails.
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
    /// A radio's copies, newest first: its own folder and its linked DFU chip's.
    pub(super) fn saved_copies(&self, device: &Device) -> Vec<FwCopy> {
        let store = self.gear_store();
        let mut out = fwcopy::list(&store, &device.id);
        if let Some(s) = device.dfu_serial.as_deref().filter(|s| !s.is_empty()) {
            out.extend(fwcopy::list(&store, &format!("dfu-{s}")));
        }
        out.sort_by(|a, b| b.taken_at.cmp(&a.taken_at).then(b.id.cmp(&a.id)));
        out
    }

    /// The saved firmware copies of one radio, or of every radio and of DFU devices not
    /// linked to one, newest first. Reads files only.
    pub fn gear_firmware_copies(&self, p: &FirmwareCopiesParams) -> Result<Vec<FirmwareCopy>> {
        let devices = self.gear_store().devices()?;
        let radios: Vec<&Device> = match p.device.as_deref().map(str::trim) {
            Some(id) if !id.is_empty() => {
                let d = devices
                    .iter()
                    .find(|d| d.id == id)
                    .ok_or_else(|| anyhow!("No saved device {id}."))?;
                if d.kind != DeviceKind::Radio {
                    bail!("Firmware copies are kept for EdgeTX radios only.");
                }
                vec![d]
            }
            _ => devices
                .iter()
                .filter(|d| d.kind == DeviceKind::Radio)
                .collect(),
        };
        let mut out: Vec<FirmwareCopy> = Vec::new();
        for r in &radios {
            out.extend(self.saved_copies(r).into_iter().map(|copy| FirmwareCopy {
                radio: Some(r.id.clone()),
                copy,
            }));
        }
        if p.device
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
        {
            // Copies of a DFU chip no radio holds the link to.
            let linked: Vec<String> = devices
                .iter()
                .filter_map(|d| d.dfu_serial.as_deref().map(|s| format!("dfu-{s}")))
                .collect();
            let store = self.gear_store();
            for f in fwcopy::folders(&store) {
                if f.starts_with("dfu-") && !linked.contains(&f) {
                    out.extend(
                        fwcopy::list(&store, &f)
                            .into_iter()
                            .map(|copy| FirmwareCopy { radio: None, copy }),
                    );
                }
            }
        }
        out.sort_by(|a, b| {
            b.copy
                .taken_at
                .cmp(&a.copy.taken_at)
                .then(b.copy.id.cmp(&a.copy.id))
        });
        Ok(out)
    }

    /// What restoring a saved copy would do, with every guard's result. Writes nothing.
    pub fn gear_firmware_restore_plan(&self, p: &FirmwareRestoreParams) -> Result<ApplyPlan> {
        Ok(self.restore_prepared(p)?.plan)
    }

    fn restore_prepared(&self, p: &FirmwareRestoreParams) -> Result<Prepared> {
        let device = self
            .gear_store()
            .devices()?
            .into_iter()
            .find(|d| d.id == p.device)
            .ok_or_else(|| anyhow!("No saved device {}.", p.device))?;
        if device.kind != DeviceKind::Radio {
            return Err(refuse(
                RefusalCode::Incompatible,
                "QuadCam restores the firmware of EdgeTX radios only.",
            ));
        }
        let name = device.display_name();
        let copy = self
            .saved_copies(&device)
            .into_iter()
            .find(|c| c.id == p.copy.trim())
            .ok_or_else(|| {
                anyhow!(
                    "{name} has no firmware copy {}. gear_firmware_copies lists its copies.",
                    p.copy
                )
            })?;
        let board = device.identity.board.clone().unwrap_or_default();
        let installed = device.identity.version.clone();

        let mut checks: Vec<Check> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        let bytes = match fwcopy::read(&self.gear_store(), &copy) {
            Ok(b) => {
                checks.push(pass("Copy matches its SHA-256"));
                Some(b)
            }
            Err(e) => {
                checks.push(fail(
                    "Copy matches its SHA-256",
                    RefusalCode::BadImage,
                    format!("{e:#} QuadCam does not write it."),
                ));
                None
            }
        };
        let spec = edgetx::spec(&board);
        checks.push(match spec {
            Some(_) => pass("Known board"),
            None => fail(
                "Known board",
                RefusalCode::UnknownBoard,
                format!(
                    "Board {} is not proven.",
                    if board.is_empty() { "(none)" } else { &board }
                ),
            ),
        });
        // The board and version the image names itself, read from the bytes, not the record.
        let identity = bytes.as_deref().and_then(edgetx::image_identity);
        let version = identity.as_ref().map(|(_, v)| v.clone());
        let board_ok = matches!(&identity, Some((b, _)) if edgetx::same_board(&board, b));
        if bytes.is_some() {
            checks.push(match &identity {
                Some((b, _)) if edgetx::same_board(&board, b) => {
                    pass("Copy is for this radio's board")
                }
                Some((b, _)) => fail(
                    "Copy is for this radio's board",
                    RefusalCode::Incompatible,
                    format!(
                        "The copy is firmware for board {b}, not {}: it is not {name}'s firmware.",
                        if board.is_empty() { "(none)" } else { &board }
                    ),
                ),
                None => fail(
                    "Copy is for this radio's board",
                    RefusalCode::BadImage,
                    "The copy holds no EdgeTX version string; it may not be EdgeTX firmware.",
                ),
            });
        }
        let mut image = Vec::new();
        // The image checks name the board's own version string: only for the radio's board.
        if let (Some(b), Some(s), Some(v), true) = (&bytes, spec, &version, board_ok) {
            match edgetx::check_image(b, s, v) {
                Ok(()) => {
                    checks.push(pass("Firmware image"));
                    image = b.clone();
                }
                Err(r) => checks.push(check("Firmware image", Err(r))),
            }
        }
        checks.push(if device.last_backup.is_some() {
            pass("Card backup")
        } else {
            fail(
                "Card backup",
                RefusalCode::NoBackup,
                "Back up this radio first: connect it in USB Storage mode and back it up. Other firmware can change how the radio reads its card.",
            )
        });
        let dfu = self.radio_dfu_checks(&device, "restore", &mut checks, &mut warnings);

        if let Some(v) = &version {
            if compat::check_writable(Product::EdgetxFlash, Some(&board), Some(v)).is_err() {
                warnings.push(format!(
                    "QuadCam has not proven EdgeTX {v} on this board for a release flash. A restore writes the copy's bytes back as they were read."
                ));
            }
        }
        warnings.push(
            "A radio in DFU mode shows no name. QuadCam writes the one radio in DFU mode; check it is the radio you picked.".into(),
        );
        warnings.push(format!(
            "QuadCam reads the radio's current firmware over DFU twice first. If a saved copy already holds it, that copy is the backup; otherwise QuadCam saves a new copy and reads it back. It erases nothing until then. If a restore fails, the radio's ROM bootloader still answers over USB and you can restore again. To put the radio in DFU mode: {} Restoring has not been tried on a real radio yet.",
            radios::dfu_hint(Some(&board))
        ));

        let diff = vec![
            DiffItem::Version {
                label: "EdgeTX firmware".into(),
                before: installed.clone(),
                after: version.clone().unwrap_or_else(|| "unknown".into()),
            },
            DiffItem::Files {
                label: "Firmware image".into(),
                put: vec![
                    format!("{} ({} KB)", copy.id, copy.size / 1024),
                    format!("SHA-256 {}", copy.sha256),
                ],
                delete: Vec::new(),
            },
        ];
        let ready = checks.iter().all(|c| c.ok);
        let digest = if ready {
            blobs::hash(
                format!(
                    "restore|{}|{}|{}|{}|{}|{}",
                    device.id,
                    dfu.first()
                        .and_then(|d| d.serial.clone())
                        .unwrap_or_default(),
                    board,
                    installed.clone().unwrap_or_default(),
                    copy.id,
                    copy.sha256
                )
                .as_bytes(),
            )
        } else {
            String::new()
        };
        Ok(Prepared {
            plan: ApplyPlan {
                change: RESTORE_CHANGE.into(),
                device: device.identity.clone(),
                checks,
                diff,
                digest,
                warnings,
            },
            device,
            copy,
            image,
            dfu: dfu.into_iter().next(),
        })
    }

    /// Restores the planned copy. Needs the plan's digest and `confirm`; with the app running,
    /// the person also clicks Apply in the sheet.
    pub fn gear_firmware_restore(&self, req: &FirmwareRestoreRequest) -> Result<ApplyReport> {
        self.agent_write(|| self.firmware_restore_at(req, false))
    }

    /// The apply sheet's own Apply click: the click is the confirm.
    pub fn gear_firmware_restore_click(&self, req: &FirmwareRestoreRequest) -> Result<ApplyReport> {
        self.firmware_restore_at(req, true)
    }

    fn firmware_restore_at(
        &self,
        req: &FirmwareRestoreRequest,
        from_gui: bool,
    ) -> Result<ApplyReport> {
        if !req.confirm {
            bail!("Refused: a firmware restore needs the plan's digest and confirm=true.");
        }
        let prep = self.restore_prepared(&req.params)?;
        if let Some(r) = first_refusal(&prep.plan.checks) {
            return Err(r.into());
        }
        if prep.plan.digest != req.digest {
            return Err(refuse(
                RefusalCode::BeforeMismatch,
                "The copy or the radio changed since the plan; plan again.",
            ));
        }
        if !from_gui {
            self.hooks
                .confirm_apply(&restore_change(&prep.device, &prep.plan), &prep.plan)?;
        }
        let dfu_info = prep.dfu.clone().expect("one DFU device passed its check");
        self.dfu_write(&DfuWrite {
            device: &prep.device,
            dfu: &dfu_info,
            image: &prep.image,
            change: RESTORE_CHANGE,
            notes: prep.plan.warnings.clone(),
            restore: Some(&prep.copy),
        })
    }
}

/// The stand-in change the sheet and an agent's confirm request carry.
pub fn restore_change(device: &Device, plan: &ApplyPlan) -> StagedChange {
    StagedChange {
        id: RESTORE_CHANGE.into(),
        device: device.id.clone(),
        title: "Restore firmware".into(),
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
