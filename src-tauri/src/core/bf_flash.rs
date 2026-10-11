//! `Core`'s Betaflight flash (design 6.5, preview): plan and run the flash of an FC. It
//! shares `gear_flash_plan`, `gear_flash` and `gear_flash_click` with the EdgeTX flash; the
//! device's kind picks the path (`core/firmware.rs`).
//!
//! - **Plan.** Identify the FC over MSP, pick the official build for its board and the
//!   release (`firmware::betaflight`), run every guard and return a plan with a digest. A
//!   board not in `betaflight::TARGETS`, or a release `compat` has not proven on that board,
//!   refuses. Nothing is written and nothing reboots.
//! - **Apply.** Back up `diff all` and `dump all` (always kept), run `bl` so the FC shows as a
//!   DFU device, copy its flash twice (`dfu::read_verified`) and keep the copy, flash it over
//!   `gear::dfu` (erase, write, read back, compare, leave), wait for
//!   the FC, read the new version's `dump all`, then stage the old `diff all` as one change
//!   (`betaflight_config::carry`) and apply it through the FC apply engine, which backs up,
//!   writes, saves and verifies against `dump all`.
//! - **Half states.** Before `bl` nothing has changed. After it, a failure leaves the FC in
//!   its bootloader or on the new firmware; the report names which, and the recovery path.
//! - **Fail-safe.** `Flasher` hands out the USB path (a recorder under cargo), serial ports
//!   are fakes under cargo, and `Cloud` never reaches the network under cargo.

use super::firmware::{flash_change, FlashParams, FlashRequest, FLASH_CHANGE};
use super::Core;
use crate::gear::apply::fc::Cand;
use crate::gear::apply::{
    check, first_refusal, ApplyPlanParams, ApplyReport, ApplyRequest, StepReport, StepState,
};
use crate::gear::bf::cli::Timing;
use crate::gear::bf::dump::Config;
use crate::gear::bf::{self, boards};
use crate::gear::blobs;
use crate::gear::compat::{self, Product};
use crate::gear::detect::{DfuInfo, STM32_DFU};
use crate::gear::dfu::{self, Dfu, Layout, FLASH_BASE};
use crate::gear::firmware::betaflight::{self as bfw, Obtained, Target};
use crate::gear::firmware::betaflight_config as carryover;
use crate::gear::fwcopy::{self, CopyKind};
use crate::gear::model::{
    ApplyPlan, ChangeStatus, Check, Device, DeviceKind, DiffItem, Edit, Refusal, RefusalCode,
    Trigger,
};
use crate::session::Editor;
use anyhow::{anyhow, bail, Result};
use chrono::Utc;
use std::collections::HashMap;
use std::time::Instant;

/// Settings the report names before it adds "and N more".
const NOTE_CAP: usize = 12;

/// The recovery path every flash plan and every half state names.
const RECOVERY: &str = "If the FC does not start: unplug USB and the battery, hold the FC's boot button, plug USB in, and flash an official build from Betaflight Configurator. The ROM bootloader cannot be overwritten, so the FC can always be flashed again.";

struct Prepared {
    plan: ApplyPlan,
    device: Device,
    port: Option<String>,
    target: Option<&'static Target>,
    image: Option<Obtained>,
    release: String,
}

fn refuse(code: RefusalCode, reason: impl Into<String>) -> anyhow::Error {
    Refusal::new(code, reason).into()
}

/// A guard result from a call that may fail with a refusal or with plain text.
fn refusal_of(e: anyhow::Error, code: RefusalCode) -> Refusal {
    match e.downcast::<Refusal>() {
        Ok(r) => r,
        Err(e) => Refusal::new(code, format!("{e:#}")),
    }
}

fn step(name: &str, state: StepState, detail: Option<String>) -> StepReport {
    StepReport {
        name: name.into(),
        state,
        detail,
    }
}

impl Core {
    /// Whether the person turned on "Betaflight flashing (preview)".
    pub(super) fn bf_flash_on(&self) -> bool {
        self.settings_file
            .as_deref()
            .and_then(|f| crate::settings::read(f).ok())
            .and_then(|v| v.get("bfFlashPreview").and_then(|x| x.as_bool()))
            .unwrap_or(false)
    }

    /// Replaces the build service the Betaflight flash downloads from (tests pass a fixture).
    pub fn with_bf_cloud(mut self, cloud: std::sync::Arc<dyn bfw::Cloud>) -> Core {
        self.bf_cloud = cloud;
        self
    }

    /// The guards that can change between the plan and the write, run in both: the port is
    /// free, no DFU device is attached, and the USB timer outlasts the flash.
    fn bf_live_checks(&self, cand: &Cand, need_s: u32) -> Vec<Check> {
        let dfu: Vec<DfuInfo> = (self.gear.dfu)()
            .into_iter()
            .filter(|d| (d.vid, d.pid) == STM32_DFU)
            .collect();
        vec![
            check(
                "Port free",
                match cand.holders.first() {
                    Some((_, name)) => Err(Refusal::new(
                        RefusalCode::PortBusy,
                        format!(
                            "{} is open in {name}. Close it there; QuadCam does not share a port.",
                            cand.port
                        ),
                    )),
                    None => Ok(()),
                },
            ),
            check(
                "No DFU device attached",
                if dfu.is_empty() {
                    Ok(())
                } else {
                    Err(Refusal::new(
                        RefusalCode::SeveralDevices,
                        format!(
                            "{} DFU device{} already attached. Unplug {}: QuadCam finds the FC in DFU mode as the one new device.",
                            dfu.len(),
                            if dfu.len() == 1 { " is" } else { "s are" },
                            if dfu.len() == 1 { "it" } else { "them" },
                        ),
                    ))
                },
            ),
            check(
                "USB heat",
                bfw::heat_check(cand.battery, cand.remaining_s, need_s),
            ),
        ]
    }

    fn bf_prepared(&self, p: &FlashParams) -> Result<Prepared> {
        let device = self
            .gear_store()
            .devices()?
            .into_iter()
            .find(|d| d.id == p.device)
            .ok_or_else(|| anyhow!("No saved device {}.", p.device))?;
        if p.splash.is_some() {
            return Err(refuse(
                RefusalCode::Incompatible,
                "A splash image is for EdgeTX radios; an FC takes no splash.",
            ));
        }
        let mut checks: Vec<Check> = Vec::new();
        checks.push(check(
            "Betaflight flashing (preview) is on",
            if self.bf_flash_on() {
                Ok(())
            } else {
                Err(Refusal::new(
                    RefusalCode::Disabled,
                    "Betaflight flashing is a preview and is off. Turn on Betaflight flashing (preview) in Settings > Gear, or set betaflight_flash_preview=true.",
                ))
            },
        ));

        // The FC: the one named, or the only one; the saved device's id.
        let cands = self.fc_cands();
        let pool: Vec<&Cand> = cands.iter().collect();
        let chosen: Option<&Cand> = match pool.as_slice() {
            [one] => Some(*one),
            _ => pool
                .iter()
                .copied()
                .find(|c| c.id.as_deref() == Some(device.id.as_str())),
        };
        checks.push(check(
            "One FC plugged in",
            match (pool.len(), chosen) {
                (0, _) => Err(Refusal::new(
                    RefusalCode::NoDevice,
                    "No FC is plugged in. Plug USB in before the battery.",
                )),
                (_, Some(_)) => Ok(()),
                (n, None) => Err(Refusal::new(
                    RefusalCode::SeveralDevices,
                    format!(
                        "{n} FCs are connected and none is {}; unplug the others. Ports: {}.",
                        device.display_name(),
                        pool.iter()
                            .map(|c| c.port.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                )),
            },
        ));
        checks.push(check(
            "The FC is this device",
            match chosen {
                Some(c) if c.id.as_deref() == Some(device.id.as_str()) => Ok(()),
                Some(c) if c.id.is_none() => Err(Refusal::new(
                    RefusalCode::DeviceChanged,
                    format!(
                        "QuadCam could not read the FC on {}; close other programs that use it and plan again.",
                        c.port
                    ),
                )),
                Some(_) => Err(Refusal::new(
                    RefusalCode::DeviceChanged,
                    format!(
                        "The FC plugged in is not {}.",
                        device.display_name()
                    ),
                )),
                None => Ok(()),
            },
        ));

        let identity = chosen
            .filter(|c| c.identity.board.is_some() || c.identity.version.is_some())
            .map(|c| c.identity.clone())
            .unwrap_or_else(|| device.identity.clone());
        let board = identity.board.clone();
        let installed = identity.version.clone();
        checks.push(check(
            "The FC runs Betaflight",
            match identity.firmware.as_deref() {
                Some("Betaflight") | None => Ok(()),
                Some(f) => Err(Refusal::new(
                    RefusalCode::Incompatible,
                    format!("This FC runs {f}; QuadCam flashes Betaflight only."),
                )),
            },
        ));

        // The release: the one asked for, else the one installed (less its suffix).
        let release = p
            .version
            .as_deref()
            .map(bfw::release_of)
            .filter(|v| !v.is_empty())
            .or_else(|| installed.as_deref().map(bfw::release_of))
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                refuse(
                    RefusalCode::Incompatible,
                    "The FC reports no version; name the version to flash.",
                )
            })?;

        let target = board.as_deref().and_then(bfw::target_for);
        let known = match target {
            None => Err(Refusal::new(
                RefusalCode::UnknownBoard,
                format!(
                    "Board {} cannot be flashed by QuadCam yet.",
                    board.as_deref().unwrap_or("(none)")
                ),
            )),
            Some(_) => {
                compat::check_writable(Product::Betaflight, board.as_deref(), Some(&release))
            }
        };
        checks.push(check("Known board and version", known));
        // Download only for a plan that is otherwise sound.
        let proceed = checks.iter().all(|c| c.ok);

        let mut warnings: Vec<String> = Vec::new();
        let mut diff = vec![DiffItem::Version {
            label: "Betaflight firmware".into(),
            before: installed.clone(),
            after: release.clone(),
        }];
        let mut image: Option<Obtained> = None;
        if let (true, Some(t)) = (proceed, target) {
            match bfw::obtain(self.bf_cloud.as_ref(), &self.cache, t, &release) {
                Ok(o) => {
                    checks.push(check("Firmware image", bfw::check_image(&o.image, t)));
                    diff.push(DiffItem::Files {
                        label: "Firmware image".into(),
                        put: vec![
                            format!("{} ({} KB)", o.file, o.image.bytes.len() / 1024),
                            format!("SHA-256 {}", o.sha256),
                            format!("{} at {:#010x}", t.mcu, FLASH_BASE),
                        ],
                        delete: Vec::new(),
                    });
                    warnings.push(if o.cached {
                        "The image is the one downloaded before; its SHA-256 still matches the record.".into()
                    } else {
                        format!(
                            "The build service lists no SHA-256. QuadCam recorded {} at this download.",
                            o.sha256
                        )
                    });
                    image = Some(o);
                }
                Err(e) => checks.push(check(
                    "Firmware image",
                    Err(refusal_of(e, RefusalCode::BadImage)),
                )),
            }
        }

        let need = image
            .as_ref()
            .map(|o| bfw::flash_seconds(o.image.bytes.len()))
            .unwrap_or(bfw::flash_seconds(512 * 1024));
        match chosen {
            Some(c) => checks.extend(self.bf_live_checks(c, need)),
            None => checks.push(check("USB heat", Ok(()))),
        }

        warnings.push(
            "QuadCam saves `diff all` and `dump all` first, flashes over DFU, reads the flash back, and puts your settings back through the FC apply. A setting the new version no longer has is listed in the report and skipped. Board lines (resources, serial ports) are not carried over."
                .into(),
        );
        warnings.push(RECOVERY.into());
        warnings.push(
            "Betaflight flashing is a preview. It has not been tried on a real FC yet.".into(),
        );
        if chosen.is_some_and(|c| c.battery) {
            warnings.push("A battery is in. Unplug it before the flash.".into());
        }

        let ready = checks.iter().all(|c| c.ok);
        let digest = if ready {
            blobs::hash(
                format!(
                    "bfflash|{}|{}|{}|{}|{}|{}",
                    device.id,
                    chosen.map(|c| c.port.clone()).unwrap_or_default(),
                    board.clone().unwrap_or_default(),
                    installed.clone().unwrap_or_default(),
                    release,
                    image.as_ref().map(|o| o.sha256.clone()).unwrap_or_default(),
                )
                .as_bytes(),
            )
        } else {
            String::new()
        };
        Ok(Prepared {
            plan: ApplyPlan {
                change: FLASH_CHANGE.into(),
                device: identity,
                checks,
                diff,
                digest,
                warnings,
            },
            device,
            port: chosen.map(|c| c.port.clone()),
            target,
            image,
            release,
        })
    }

    /// What a Betaflight flash would do. Downloads the build when it is not in the cache;
    /// writes nothing and reboots nothing.
    pub(super) fn bf_flash_plan(&self, p: &FlashParams) -> Result<ApplyPlan> {
        Ok(self.bf_prepared(p)?.plan)
    }

    /// The flash, then the settings. Needs the plan's digest and `confirm`; with the app
    /// running, the person also clicks Apply in the sheet.
    pub(super) fn bf_flash_at(&self, req: &FlashRequest, from_gui: bool) -> Result<ApplyReport> {
        if !req.confirm {
            bail!("Refused: a flash needs the plan's digest and confirm=true.");
        }
        let prep = self.bf_prepared(&req.params)?;
        if let Some(r) = first_refusal(&prep.plan.checks) {
            return Err(r.into());
        }
        if prep.plan.digest != req.digest {
            return Err(refuse(
                RefusalCode::BeforeMismatch,
                "The firmware or the FC changed since the plan; plan again.",
            ));
        }
        if !from_gui {
            self.hooks
                .confirm_apply(&flash_change(&prep.device, &prep.plan), &prep.plan)?;
        }
        let port = prep
            .port
            .clone()
            .ok_or_else(|| anyhow!("No FC to flash."))?;
        let target = prep.target.expect("a known board passed its check");
        let image = prep.image.as_ref().expect("the image passed its check");

        // Part A: back up, flash, bring the FC back, read its new state. One job on the port.
        let a = {
            let (_job, _stop) = self.job_start(&port, Some(&prep.device.id), "Flashing")?;
            let _hold = self.gear_hold(&port);
            self.bf_flash_job(&prep, &port, target, image)?
        };
        let mut steps = a.steps;
        let mut notes = prep.plan.warnings.clone();
        notes.retain(|n| n != RECOVERY || a.failed.is_some());
        let mut report = ApplyReport {
            change: FLASH_CHANGE.into(),
            device: prep.device.id.clone(),
            status: ChangeStatus::Failed,
            steps: Vec::new(),
            backup: a.backup.clone(),
            after_backup: a.after_backup.clone(),
            sent: Vec::new(),
            failed_line: None,
            verify: Vec::new(),
            saved: a.flashed,
            files: Vec::new(),
            message: String::new(),
            notes: Vec::new(),
            at: Utc::now(),
        };
        if let Some(why) = a.failed {
            report.message = why;
            report.steps = steps;
            report.notes = notes;
            self.hooks.gear_changed();
            return Ok(report);
        }
        let carry = a.carry.expect("a finished flash read the new state");
        notes.extend(carryover::notes(&carry, NOTE_CAP));
        notes.extend(a.notes);

        // Part B: put the settings back through the FC apply.
        if carry.lines.is_empty() {
            steps.push(step(
                "Re-apply settings",
                StepState::Skipped,
                Some("Nothing to carry over: the new firmware already holds every setting.".into()),
            ));
            report.status = ChangeStatus::Verified;
            report.message = format!(
                "Verified: {} runs {} and the flash matches byte for byte. No setting was carried over.",
                prep.device.display_name(),
                prep.release
            );
        } else {
            let applied = self.bf_reapply(&prep, &port, &carry.lines);
            match applied {
                Ok((applied, id)) => {
                    report.sent = applied.sent.clone();
                    report.failed_line = applied.failed_line.clone();
                    report.verify = applied.verify.clone();
                    report.after_backup = applied.after_backup.clone().or(report.after_backup);
                    let ok = applied.status == ChangeStatus::Verified;
                    steps.push(step(
                        "Re-apply settings",
                        if ok {
                            StepState::Done
                        } else {
                            StepState::Failed
                        },
                        Some(if ok {
                            format!("{} lines, change {id}", carry.lines.len())
                        } else {
                            applied.message.clone()
                        }),
                    ));
                    if ok {
                        report.status = ChangeStatus::Verified;
                        report.message = format!(
                            "Verified: {} runs {}, the flash matches byte for byte, and its settings read back as saved.",
                            prep.device.display_name(),
                            prep.release
                        );
                    } else {
                        report.message = format!(
                            "The firmware is flashed and verified, but the settings were not put back: {} The old settings are in backup {}; change {id} still holds them in Changes.",
                            applied.message,
                            report.backup.clone().unwrap_or_default()
                        );
                    }
                }
                Err(e) => {
                    steps.push(step(
                        "Re-apply settings",
                        StepState::Failed,
                        Some(format!("{e:#}")),
                    ));
                    report.message = format!(
                        "The firmware is flashed and verified, but the settings were not put back: {e:#} The old settings are in backup {}.",
                        report.backup.clone().unwrap_or_default()
                    );
                }
            }
        }
        report.steps = steps;
        report.notes = notes;
        self.hooks.gear_changed();
        Ok(report)
    }

    /// Stages the carried lines as one change and applies it through the FC apply engine.
    /// The person's flash confirm covers it: the plan said the settings come back.
    fn bf_reapply(
        &self,
        prep: &Prepared,
        port: &str,
        lines: &[String],
    ) -> Result<(ApplyReport, String)> {
        let change = self.gear_change_stage(&super::apply::StageParams {
            device: prep.device.id.clone(),
            title: Some(format!(
                "Settings after the Betaflight {} flash",
                prep.release
            )),
            edits: vec![Edit::FcLines {
                lines: lines.to_vec(),
            }],
            note: Some("Staged by the Betaflight flash from the old firmware's diff all.".into()),
            editor: Some(Editor::User),
            draft: false,
        })?;
        let plan = self.gear_apply_plan(&ApplyPlanParams {
            id: change.id.clone(),
            port: Some(port.to_string()),
        })?;
        if let Some(r) = first_refusal(&plan.checks) {
            bail!("{} Change {} waits in Changes.", r.reason, change.id);
        }
        let report = self.apply_inner(
            &ApplyRequest {
                id: change.id.clone(),
                digest: plan.digest,
                confirm: true,
                port: Some(port.to_string()),
            },
            true,
        )?;
        Ok((report, change.id))
    }

    /// Part A of a flash, on the held port. `Err` means nothing was changed.
    fn bf_flash_job(
        &self,
        prep: &Prepared,
        port: &str,
        target: &Target,
        image: &Obtained,
    ) -> Result<PartA> {
        let t: Timing = self.fc_timing();
        let ports = self.gear.ports.as_ref();
        let name = prep.device.display_name();
        let mut out = PartA::default();

        // The guards again, right before the first step.
        self.job_step(port, "Checking");
        drop(bf::cli::wait_for_port(ports, port, t)?);
        let mut cand = self
            .fc_cands()
            .into_iter()
            .find(|c| c.port == port)
            .ok_or_else(|| refuse(RefusalCode::NoDevice, "The FC is no longer plugged in."))?;
        if cand.id.as_deref() != Some(prep.device.id.as_str()) {
            return Err(refuse(
                RefusalCode::DeviceChanged,
                "This is not the FC the flash was planned for.",
            ));
        }
        // The battery as it is now: a battery in while no timer counts refuses.
        let heat = self.usb_heat(port, bf::battery_volts(ports, port, t).ok());
        if heat.untimed {
            return Err(refuse(
                RefusalCode::UsbHeat,
                "A battery is in, and QuadCam's USB timer is not counting for this FC (its reads are paused, or it was plugged in moments ago), so it cannot tell whether the flash would outlast it. Unplug the battery, then flash again.",
            ));
        }
        cand.battery = heat.battery;
        cand.remaining_s = heat.remaining_s;
        for c in self.bf_live_checks(&cand, bfw::flash_seconds(image.image.bytes.len())) {
            if let Some(r) = c.refusal {
                return Err(r.into());
            }
        }

        // Back up first; a failed backup stops everything.
        self.job_step(port, "Backing up");
        let commands: Vec<String> = bf::BACKUP.iter().map(|s| s.to_string()).collect();
        let nobackup = |why: String| {
            refuse(
                RefusalCode::NoBackup,
                format!("The backup failed: {why}. Nothing was changed."),
            )
        };
        let read = bf::read(ports, port, &commands, t).map_err(|e| nobackup(format!("{e:#}")))?;
        let files: Vec<(String, Vec<u8>)> = bf::BACKUP
            .iter()
            .filter_map(|c| read.file(c).map(|x| (c.to_string(), x.into_bytes())))
            .collect();
        let taken = self
            .snapshots()
            .take_files(
                &prep.device.id,
                &read.info.identity,
                Trigger::BeforeFlash,
                Utc::now(),
                &files,
                false,
            )
            .map_err(|e| nobackup(format!("{e:#}")))?;
        let backup_id = taken.backup.id.clone();
        let _ = self.after_backup(
            &prep.device.id,
            DeviceKind::Fc,
            &read.info.identity,
            taken,
            Vec::new(),
            None,
        );
        out.backup = Some(backup_id.clone());
        out.steps.push(step(
            "Back up",
            StepState::Done,
            Some(format!("diff all and dump all, {backup_id}")),
        ));
        let old_diff = Config::parse(
            &read
                .replies
                .iter()
                .find(|r| r.line == "diff all")
                .map(|r| r.text.clone())
                .unwrap_or_default(),
        );

        // Into the bootloader. From here the FC may be in a half state.
        self.job_step(port, "Restarting into the bootloader");
        let id = prep.device.id.clone();
        let boot = bf::to_bootloader(
            ports,
            port,
            Some(&id),
            &|b| {
                bfw::target_for(b).map(|_| ()).ok_or_else(|| {
                    Refusal::new(
                        RefusalCode::UnknownBoard,
                        format!("Board {b} cannot be flashed."),
                    )
                })
            },
            t,
        );
        if let Err(e) = boot {
            if e.downcast_ref::<Refusal>().is_some() {
                return Err(e);
            }
            out.steps.push(step(
                "Restart into the bootloader",
                StepState::Failed,
                Some(format!("{e:#}")),
            ));
            out.failed = Some(format!(
                "{name} did not restart into its bootloader: {e:#} Nothing was written. Unplug USB and the battery, plug USB in again, and plan again. Your settings are in backup {backup_id}."
            ));
            return Ok(out);
        }
        let dfu_info = match self.bf_wait_dfu(t) {
            Ok(d) => d,
            Err(e) => {
                out.steps.push(step(
                    "Restart into the bootloader",
                    StepState::Failed,
                    Some(e.reason.clone()),
                ));
                out.failed = Some(format!(
                    "{} Nothing was written. {RECOVERY} Your settings are in backup {backup_id}.",
                    e.reason
                ));
                return Ok(out);
            }
        };
        out.steps.push(step(
            "Restart into the bootloader",
            StepState::Done,
            dfu_info.serial.clone().map(|s| format!("DFU device {s}")),
        ));

        // Flash.
        self.job_step(port, "Flashing");
        let flasher = self.firmware.flasher.as_ref();
        let quick = flasher.quick();
        let mut usb = match flasher.open_dfu(dfu_info.vid, dfu_info.pid, dfu_info.serial.as_deref())
        {
            Ok(u) => u,
            Err(e) => {
                out.steps
                    .push(step("Erase", StepState::Failed, Some(format!("{e:#}"))));
                out.failed = Some(format!(
                    "QuadCam could not open the FC's DFU device: {e:#} Nothing was written; the FC waits in its bootloader. Unplug USB and plug it in again to start the old firmware. {RECOVERY}"
                ));
                return Ok(out);
            }
        };
        let layout_problem = usb
            .layout()
            .and_then(|s| Layout::parse(&s))
            .map(|l| {
                let kb = (l.end() - l.start()) / 1024;
                (l.start() != FLASH_BASE || kb != target.flash_kb).then(|| {
                    format!(
                        "The bootloader reports {kb} KB of flash at {:#010x}; a {} has {} KB at {FLASH_BASE:#010x}",
                        l.start(),
                        target.target,
                        target.flash_kb
                    )
                })
            });
        match layout_problem {
            Ok(None) => {}
            Ok(Some(why)) => {
                // Nothing was erased: leaving DFU starts the old firmware again.
                let _ = if quick {
                    Dfu::quick(usb.as_mut()).leave(FLASH_BASE)
                } else {
                    Dfu::new(usb.as_mut()).leave(FLASH_BASE)
                };
                out.steps
                    .push(step("Erase", StepState::Failed, Some(why.clone())));
                out.failed = Some(format!(
                    "{why}. This is not the chip the image is for. Nothing was written, and QuadCam asked the FC to start its old firmware. Your settings are in backup {backup_id}."
                ));
                return Ok(out);
            }
            Err(e) => {
                out.steps
                    .push(step("Erase", StepState::Failed, Some(format!("{e:#}"))));
                out.failed = Some(format!(
                    "QuadCam could not read the DFU memory layout: {e:#} Nothing was written; the FC waits in its bootloader. Unplug USB and plug it in again. {RECOVERY}"
                ));
                return Ok(out);
            }
        }
        // Copy the firmware the FC runs now: two reads that agree, saved and read back from
        // disk. Only then may anything be erased. A blank flash (an interrupted flash) has
        // nothing to copy and may be flashed.
        self.job_step(port, "Copying the firmware");
        let stuck = |why: String| {
            format!(
                "{why} Nothing was written; the FC waits in its bootloader. Unplug USB and the battery and plug USB in again to start its old firmware. Your settings are in backup {backup_id}."
            )
        };
        let copy = match dfu::read_verified(usb.as_mut(), quick, &mut |_, _| {}) {
            Ok(c) => c,
            Err(e) => {
                out.steps.push(step(
                    "Copy the current firmware",
                    StepState::Failed,
                    Some(format!("{e:#}")),
                ));
                out.failed = Some(stuck(format!("Reading the FC's firmware failed: {e:#}.")));
                return Ok(out);
            }
        };
        let kept = if copy.is_blank() {
            None
        } else {
            match fwcopy::save(
                &self.gear_store(),
                &prep.device.id,
                CopyKind::BeforeFlash,
                Utc::now(),
                &copy.trimmed(),
                None,
            ) {
                Ok(c) => Some(c),
                Err(e) => {
                    out.steps.push(step(
                        "Copy the current firmware",
                        StepState::Failed,
                        Some(format!("{e:#}")),
                    ));
                    out.failed = Some(stuck(format!(
                        "Saving the copy of the FC's firmware failed: {e:#}."
                    )));
                    return Ok(out);
                }
            }
        };
        out.steps.push(step(
            "Copy the current firmware",
            StepState::Done,
            Some(match &kept {
                Some(c) => format!(
                    "{} KB, read twice, saved as {}",
                    copy.trimmed().len() / 1024,
                    c.id
                ),
                None => "the flash is blank; there is no firmware to keep".into(),
            }),
        ));

        self.job_step(port, "Flashing");
        let mut seen: Vec<dfu::Step> = Vec::new();
        let outcome = dfu::flash(
            usb.as_mut(),
            FLASH_BASE,
            &image.image.bytes,
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
        match &outcome {
            Ok(o) => {
                for (s, label) in labels {
                    out.steps.push(step(
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
                out.flashed = true;
            }
            Err(e) if seen.is_empty() => {
                // It failed before the erase: the old firmware is whole. Leaving DFU starts it.
                let left = if quick {
                    Dfu::quick(usb.as_mut()).leave(FLASH_BASE)
                } else {
                    Dfu::new(usb.as_mut()).leave(FLASH_BASE)
                }
                .is_ok();
                for (i, (_, label)) in labels.iter().enumerate() {
                    out.steps.push(step(
                        label,
                        StepState::Skipped,
                        (i == 0).then(|| format!("{e:#}")),
                    ));
                }
                out.failed = Some(format!(
                    "The flash did not start: {e:#} Nothing was erased, so the FC still holds its old firmware. {} Your settings are in backup {backup_id}.",
                    if left {
                        "QuadCam asked the FC to start it again."
                    } else {
                        "The FC waits in its bootloader: unplug USB and the battery and plug USB in again to start it."
                    }
                ));
                return Ok(out);
            }
            Err(e) => {
                let failed = seen.len().saturating_sub(1);
                for (i, (_, label)) in labels.iter().enumerate() {
                    let state = match i.cmp(&failed) {
                        std::cmp::Ordering::Less => StepState::Done,
                        std::cmp::Ordering::Equal => StepState::Failed,
                        std::cmp::Ordering::Greater => StepState::Skipped,
                    };
                    out.steps.push(step(
                        label,
                        state,
                        (state == StepState::Failed).then(|| format!("{e:#}")),
                    ));
                }
                out.failed = Some(format!(
                    "The flash failed: {e:#} The FC has half a firmware and stays in its bootloader. QuadCam cannot flash an FC that is in its bootloader. Leave USB plugged in and flash an official {} build from Betaflight Configurator, which finds the FC in DFU mode. {RECOVERY} Your settings are in backup {backup_id}: put them back with Restore once the FC runs Betaflight.",
                    target.target
                ));
                return Ok(out);
            }
        }

        // The FC comes back on the new firmware.
        self.job_step(port, "Waiting for the FC");
        let back = (|| -> Result<bf::FcInfo> {
            drop(bf::cli::wait_for_port(ports, port, t)?);
            let mut last = None;
            for _ in 0..8 {
                match bf::identify(ports, port, t) {
                    Ok(i) => return Ok(i),
                    Err(e) => {
                        last = Some(e);
                        std::thread::sleep(t.poll);
                    }
                }
            }
            Err(last.unwrap_or_else(|| anyhow!("no answer")))
        })();
        let info = match back {
            Ok(i) => i,
            Err(e) => {
                out.steps
                    .push(step("Restart", StepState::Failed, Some(format!("{e:#}"))));
                out.failed = Some(format!(
                    "The firmware is flashed and verified, but the FC did not answer on {port}: {e:#} Unplug USB and the battery, plug USB in again, then re-apply your settings from backup {backup_id} (Restore). {RECOVERY}"
                ));
                return Ok(out);
            }
        };
        out.steps.push(step(
            "Restart",
            StepState::Done,
            info.identity.version.clone(),
        ));
        if info.id.as_deref() != Some(prep.device.id.as_str()) {
            out.failed = Some(format!(
                "The firmware is flashed and verified, but the FC that answered on {port} is not {name}. Settings were not touched. Your old settings are in backup {backup_id}."
            ));
            return Ok(out);
        }
        if let Some(v) = info.identity.version.as_deref() {
            if !compat::version_matches(&prep.release, bfw::release_of(v).as_str()) {
                out.notes.push(format!(
                    "The FC reports Betaflight {v}, not the {} that was flashed.",
                    prep.release
                ));
            }
        }
        if let Err(r) = bf::writable(&info.identity) {
            out.failed = Some(format!(
                "The firmware is flashed and verified, but QuadCam does not write settings on it: {} Your old settings are in backup {backup_id}.",
                r.reason
            ));
            return Ok(out);
        }

        // Read the new firmware: the backup set and one `get` for each old setting.
        self.job_step(port, "Reading the new settings");
        let mut names: Vec<String> = Vec::new();
        for (_, n, _) in old_diff.sets() {
            if !names.iter().any(|x| x == n) {
                names.push(n.to_string());
            }
        }
        let mut commands = commands.clone();
        commands.extend(names.iter().map(|n| format!("get {n}")));
        let fresh = match bf::read(ports, port, &commands, t) {
            Ok(r) => r,
            Err(e) => {
                out.steps.push(step(
                    "Read new settings",
                    StepState::Failed,
                    Some(format!("{e:#}")),
                ));
                out.failed = Some(format!(
                    "The firmware is flashed and verified, but reading the new settings failed: {e:#} Re-apply your settings from backup {backup_id} (Restore) once the FC answers."
                ));
                return Ok(out);
            }
        };
        self.fc_state
            .lock()
            .unwrap()
            .seen
            .insert(port.to_string(), fresh.info.clone());
        let after: Vec<(String, Vec<u8>)> = bf::BACKUP
            .iter()
            .filter_map(|c| fresh.file(c).map(|x| (c.to_string(), x.into_bytes())))
            .collect();
        if let Ok(rep) = self.snapshots().take_files(
            &prep.device.id,
            &fresh.info.identity,
            Trigger::AfterApply,
            Utc::now(),
            &after,
            false,
        ) {
            out.after_backup = Some(rep.backup.id.clone());
            let _ = self.after_backup(
                &prep.device.id,
                DeviceKind::Fc,
                &fresh.info.identity,
                rep,
                Vec::new(),
                None,
            );
        }
        let gets: HashMap<String, String> = fresh
            .replies
            .iter()
            .filter_map(|r| {
                r.line
                    .strip_prefix("get ")
                    .map(|n| (n.trim().to_string(), r.text.clone()))
            })
            .collect();
        let dump = fresh
            .replies
            .iter()
            .find(|r| r.line == "dump all")
            .map(|r| r.text.clone())
            .unwrap_or_default();
        let carry = carryover::carry(&old_diff, &Config::parse(&dump), &gets);
        out.steps.push(step(
            "Read new settings",
            StepState::Done,
            Some(format!(
                "{} to carry over, {} skipped",
                carry.lines.iter().filter(|l| l.starts_with("set ")).count(),
                carry.missing.len() + carry.refused.len()
            )),
        ));
        for n in boards::after_job(
            fresh.info.identity.board.as_deref(),
            fresh.info.identity.version.as_deref(),
        ) {
            out.notes.push(n);
        }
        out.carry = Some(carry);
        Ok(out)
    }

    /// Waits for exactly one STM32 DFU device to appear.
    fn bf_wait_dfu(&self, t: Timing) -> std::result::Result<DfuInfo, Refusal> {
        let deadline = Instant::now() + t.reboot;
        loop {
            let found: Vec<DfuInfo> = (self.gear.dfu)()
                .into_iter()
                .filter(|d| (d.vid, d.pid) == STM32_DFU)
                .collect();
            match found.len() {
                1 => return Ok(found.into_iter().next().unwrap()),
                0 => {}
                n => {
                    return Err(Refusal::new(
                        RefusalCode::SeveralDevices,
                        format!(
                            "{n} DFU devices appeared; QuadCam cannot tell which one is the FC."
                        ),
                    ))
                }
            }
            if Instant::now() >= deadline {
                return Err(Refusal::new(
                    RefusalCode::NoDevice,
                    format!(
                        "The FC did not show up as a DFU device within {} s.",
                        t.reboot.as_secs()
                    ),
                ));
            }
            std::thread::sleep(t.poll);
        }
    }
}

/// What part A found: the steps so far, the backups, and what to carry over.
#[derive(Default)]
struct PartA {
    steps: Vec<StepReport>,
    backup: Option<String>,
    after_backup: Option<String>,
    flashed: bool,
    failed: Option<String>,
    notes: Vec<String>,
    carry: Option<carryover::Carry>,
}
