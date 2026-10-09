//! `Core`'s staged changes and the FC apply (design 8): stage, list, update, discard,
//! restore, plan and apply.
//!
//! - Apply is one FC job (`fc_job`): one port hold, one cue at the end. Inside it: check
//!   again, back up (always kept), compare the fresh dump with the plan (the digest),
//!   range-check every `set` with `get`, write, save, wait through the reboot, read
//!   `dump all` back, verify, record.
//! - The plan reads no CLI (no reboot): it compares with the device's latest backup and
//!   asks each plugged-in FC for its identity over MSP.
//! - Confirm: the apply sheet's own Apply click calls `gear_apply_click`. Any other caller
//!   (CLI, MCP) needs the plan's digest and `confirm`, and with the app running the person
//!   also clicks Apply in the sheet (`Hooks::confirm_apply`).

use super::{link_handle, Core};
use crate::gear::apply::fc::{self as fcplan, Base, Cand};
use crate::gear::apply::{
    first_refusal, ApplyPlanParams, ApplyReport, ApplyRequest, StepReport, StepState,
};
use crate::gear::backup::BackupContent;
use crate::gear::bf::cli::CliSession;
use crate::gear::bf::dump::{parse_cmd, Cmd, Config};
use crate::gear::bf::{self, boards};
use crate::gear::changes::{
    render_fc, restore_lines, ChangeFilter, ChangeUpdate, Changes, NewChange,
};
use crate::gear::model::{
    ApplyPlan, ChangeStatus, DeviceKind, Edit, Refusal, RefusalCode, StagedChange, Trigger,
};
use crate::session::Editor;
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Mutex;
use std::time::Instant;

/// `gear_change_stage`: queue edits for a device.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct StageParams {
    pub device: String,
    #[serde(default)]
    pub title: Option<String>,
    pub edits: Vec<Edit>,
    #[serde(default)]
    pub note: Option<String>,
    /// Who staged it. The app and the CLI leave it out (the person); MCP says `agent`.
    #[serde(default)]
    pub editor: Option<Editor>,
    /// Keep it as a draft: the plug-in bar and Review skip drafts.
    #[serde(default)]
    pub draft: bool,
}

/// `gear_change_update`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct ChangeUpdateParams {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub edits: Option<Vec<Edit>>,
    #[serde(default)]
    pub status: Option<ChangeStatus>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub order: Option<u32>,
}

/// `gear_restore_stage`: put an FC backup's settings back, as a staged change.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct RestoreParams {
    pub backup: String,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub editor: Option<Editor>,
}

pub(super) fn refusal(e: Refusal) -> anyhow::Error {
    e.into()
}

impl Core {
    pub(super) fn changes(&self) -> Changes {
        Changes::new(self.gear_store())
    }

    /// The staged changes of each device (for `GearStatus` and the plug-in bar).
    pub fn gear_staged_counts(&self) -> std::collections::BTreeMap<String, usize> {
        self.changes().staged_counts()
    }

    pub fn gear_changes(&self, f: &ChangeFilter) -> Result<Vec<StagedChange>> {
        Ok(self.changes().list(f))
    }

    /// The device's latest backup with its `dump all`.
    pub(super) fn fc_base(&self, device: &str) -> Option<Base> {
        let snaps = self.snapshots();
        let backup = snaps.latest(device)?;
        let BackupContent { text, .. } = snaps.read(&backup.id, Some("dump all")).ok()?;
        Some(Base {
            backup,
            dump: text?,
        })
    }

    /// The edits with restores turned into the lines that make the device read as the
    /// backup.
    fn fc_edits(&self, edits: &[Edit], base: Option<&Base>) -> Result<Vec<Edit>> {
        let mut out = Vec::new();
        for e in edits {
            match (e, base) {
                (Edit::Restore { backup, paths }, Some(b)) => {
                    if !paths.iter().all(|p| p == "dump all" || p == "diff all") {
                        bail!("An FC restore takes `dump all` or `diff all`, not {paths:?}.");
                    }
                    let snaps = self.snapshots();
                    let text = snaps
                        .read(backup, Some("dump all"))
                        .with_context(|| format!("Backup {backup} has no `dump all`."))?
                        .text
                        .context("The backup's dump is not text.")?;
                    out.push(Edit::FcLines {
                        lines: restore_lines(&Config::parse(&text), &Config::parse(&b.dump)),
                    });
                }
                (e, _) => out.push(e.clone()),
            }
        }
        Ok(out)
    }

    /// Checks a change's edits against the device: an FC's lines against its latest dump,
    /// a radio's against what the card engine plans. Returns the backup the change is
    /// based on (empty when the device has none).
    fn check_stageable(&self, device: &str, edits: &[Edit]) -> Result<String> {
        let d = self
            .gear_store()
            .device(device)?
            .with_context(|| format!("Unknown device {device:?}. See `gear devices`."))?;
        match d.kind {
            DeviceKind::Fc => {
                let base = self.fc_base(device);
                let resolved = self.fc_edits(edits, base.as_ref())?;
                let cfg = base.as_ref().map(|b| Config::parse(&b.dump));
                if let Some(p) = render_fc(&resolved, cfg.as_ref())
                    .problems
                    .into_iter()
                    .next()
                {
                    return Err(refusal(p));
                }
                Ok(base.map(|b| b.backup.id).unwrap_or_default())
            }
            DeviceKind::Radio => {
                self.check_card_stageable(edits)?;
                Ok(self
                    .snapshots()
                    .latest(device)
                    .map(|b| b.id)
                    .unwrap_or_default())
            }
            other => bail!(
                "Only FC and radio changes can be staged for now; a {} change arrives with its package.",
                other.label()
            ),
        }
    }

    pub fn gear_change_stage(&self, p: &StageParams) -> Result<StagedChange> {
        self.stage_inner(p, None)
    }

    /// Stages a change; `reverts` names the change a restore undoes.
    pub(super) fn stage_inner(
        &self,
        p: &StageParams,
        reverts: Option<String>,
    ) -> Result<StagedChange> {
        let base = self.check_stageable(&p.device, &p.edits)?;
        let title = p
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| default_title(&p.edits));
        let c = self.changes().stage(NewChange {
            device: p.device.clone(),
            title,
            edits: p.edits.clone(),
            base_backup: base,
            editor: p.editor.unwrap_or(Editor::User),
            note: p.note.clone().unwrap_or_default(),
            status: if p.draft {
                ChangeStatus::Draft
            } else {
                ChangeStatus::Ready
            },
            reverts,
        })?;
        self.hooks.gear_changed();
        Ok(c)
    }

    pub fn gear_change_update(&self, p: &ChangeUpdateParams) -> Result<StagedChange> {
        let ch = self.changes();
        if let Some(e) = &p.edits {
            let c = ch.get(&p.id)?;
            self.check_stageable(&c.device, e)?;
        }
        let c = ch.update(
            &p.id,
            ChangeUpdate {
                title: p.title.clone(),
                edits: p.edits.clone(),
                status: p.status,
                note: p.note.clone(),
                order: p.order,
            },
        )?;
        self.hooks.gear_changed();
        Ok(c)
    }

    pub fn gear_change_discard(&self, id: &str) -> Result<StagedChange> {
        let c = self.changes().discard(id)?;
        self.hooks.gear_changed();
        Ok(c)
    }

    /// Stages a restore of an FC backup's settings.
    pub fn gear_restore_stage(&self, p: &RestoreParams) -> Result<StagedChange> {
        let b = self.snapshots().get(&p.backup)?;
        let paths = match crate::gear::backup::kind_of_id(&b.device) {
            Some(DeviceKind::Fc) if p.paths.is_empty() => vec!["dump all".to_string()],
            Some(DeviceKind::Fc | DeviceKind::Radio) if !p.paths.is_empty() => p.paths.clone(),
            Some(DeviceKind::Radio) => bail!(
                "A card restore names the files to put back (paths): restoring a whole card is not offered."
            ),
            _ => bail!("Only an FC or a radio card backup can be restored."),
        };
        self.gear_change_stage(&StageParams {
            device: b.device.clone(),
            title: Some(format!(
                "Restore backup {}",
                p.backup.rsplit('/').next().unwrap_or(&p.backup)
            )),
            edits: vec![Edit::Restore {
                backup: p.backup.clone(),
                paths,
            }],
            note: None,
            editor: p.editor,
            draft: false,
        })
    }

    /// Each plugged-in FC as the plan sees it. An FC not identified yet is identified
    /// over MSP (no reboot) unless another program holds its port.
    pub(super) fn fc_cands(&self) -> Vec<Cand> {
        let t = self.fc_timing();
        let now = Instant::now();
        let timers = self.gear_usb_timers();
        let mut out = Vec::new();
        for c in self.fc_ports() {
            let port = link_handle(&c.link);
            let holders = (self.gear.holders)(&port);
            let mut info = self.gear_fc_seen(&port);
            if info.is_none() && holders.is_empty() && !self.held_by_job(&port) {
                let _hold = self.gear_hold(&port);
                if let Ok(i) = bf::identify(self.gear.ports.as_ref(), &port, t) {
                    self.fc_state
                        .lock()
                        .unwrap()
                        .seen
                        .insert(port.clone(), i.clone());
                    info = Some(i);
                }
            }
            let timer = timers.iter().find(|x| x.port == port);
            let _ = now;
            out.push(Cand {
                port,
                id: info.as_ref().and_then(|i| i.id.clone()),
                identity: info.map(|i| i.identity).unwrap_or_default(),
                battery: timer.is_some_and(|x| x.battery),
                remaining_s: timer.and_then(|x| x.remaining_s),
                holders,
            });
        }
        out
    }

    fn plan_change(&self, id: &str, port: Option<&str>) -> Result<(StagedChange, fcplan::Planned)> {
        let change = self.changes().get(id)?;
        if !change.status.staged() {
            bail!(
                "Change {id} is {:?}; only a staged change can be applied.",
                change.status
            );
        }
        let base = self.fc_base(&change.device);
        let edits = self.fc_edits(&change.edits, base.as_ref())?;
        let mut planned = fcplan::plan(&change, &edits, base.as_ref(), &self.fc_cands(), port);
        planned
            .plan
            .checks
            .insert(0, crate::gear::apply::card::read_first(&change));
        Ok((change, planned))
    }

    /// The kind of device a staged change is for.
    pub(super) fn change_kind(&self, change: &StagedChange) -> Result<DeviceKind> {
        Ok(self
            .gear_store()
            .device(&change.device)?
            .with_context(|| format!("Unknown device {:?}.", change.device))?
            .kind)
    }

    fn staged_change(&self, id: &str) -> Result<StagedChange> {
        let change = self.changes().get(id)?;
        if !change.status.staged() {
            bail!(
                "Change {id} is {:?}; only a staged change can be applied.",
                change.status
            );
        }
        Ok(change)
    }

    /// Runs every guard and builds the diff and digest. Writes nothing and reboots
    /// nothing.
    pub fn gear_apply_plan(&self, p: &ApplyPlanParams) -> Result<ApplyPlan> {
        let change = self.staged_change(&p.id)?;
        match self.change_kind(&change)? {
            DeviceKind::Fc => Ok(self.plan_change(&p.id, p.port.as_deref())?.1.plan),
            DeviceKind::Radio => Ok(self.plan_card(&change, false)?.0.plan),
            other => bail!("A {} change cannot be applied yet.", other.label()),
        }
    }

    /// Applies a staged change. Needs the plan's digest and `confirm`; with the app
    /// running, the person also clicks Apply in the sheet.
    pub fn gear_apply(&self, req: &ApplyRequest) -> Result<ApplyReport> {
        self.apply_inner(req, false)
    }

    /// The apply sheet's own Apply click: the click is the confirm.
    pub fn gear_apply_click(&self, req: &ApplyRequest) -> Result<ApplyReport> {
        self.apply_inner(req, true)
    }

    pub(super) fn apply_inner(&self, req: &ApplyRequest, from_gui: bool) -> Result<ApplyReport> {
        if !req.confirm {
            bail!("Refused: apply needs the plan's digest and confirm=true.");
        }
        let change = self.staged_change(&req.id)?;
        match self.change_kind(&change)? {
            DeviceKind::Fc => {}
            DeviceKind::Radio => return self.apply_card_change(req, from_gui),
            other => bail!("A {} change cannot be applied yet.", other.label()),
        }
        let (change, planned) = self.plan_change(&req.id, req.port.as_deref())?;
        if let Some(r) = first_refusal(&planned.plan.checks) {
            return Err(refusal(r));
        }
        if planned.plan.digest != req.digest {
            return Err(refusal(Refusal::new(
                RefusalCode::BeforeMismatch,
                "The plan changed since you read it; plan again.",
            )));
        }
        if !from_gui {
            self.hooks.confirm_apply(&change, &planned.plan)?;
        }
        let port = planned.port.clone().context("No FC to write to.")?;
        let base = self.fc_base(&change.device);
        let edits = self.fc_edits(&change.edits, base.as_ref())?;
        let (_job, _stop) = self.job_start(&port, Some(&change.device), "Applying")?;
        let stash: Mutex<Option<ApplyReport>> = Mutex::new(None);
        let out = self.fc_job(Some(&port), "Apply", |ports, port| {
            self.apply_fc(ports, port, &change, &edits, &req.digest, &stash)
        });
        match out {
            Ok(job) => {
                let mut r = job.result;
                r.notes = job.notes;
                Ok(r)
            }
            Err(e) => match stash.lock().unwrap().take() {
                Some(r) => Ok(r),
                None => Err(e),
            },
        }
    }

    fn apply_fc(
        &self,
        ports: &dyn crate::gear::serial::Ports,
        port: &str,
        change: &StagedChange,
        edits: &[Edit],
        digest: &str,
        stash: &Mutex<Option<ApplyReport>>,
    ) -> Result<(bf::FcInfo, ApplyReport)> {
        let t = self.fc_timing();
        let snaps = self.snapshots();
        let step = |s: &str| self.job_step(port, s);

        // The guards again, right before the write (design 8.1).
        step("Checking");
        // An earlier job may have left the FC rebooting.
        drop(crate::gear::bf::cli::wait_for_port(ports, port, t)?);
        let info = bf::identify(ports, port, t)?;
        if info.id.as_deref() != Some(change.device.as_str()) {
            return Err(refusal(Refusal::new(
                RefusalCode::DeviceChanged,
                "This is not the FC the change was planned for.",
            )));
        }
        bf::writable(&info.identity).map_err(refusal)?;
        if let Some((_, name)) = (self.gear.holders)(port).into_iter().next() {
            return Err(refusal(Refusal::new(
                RefusalCode::PortBusy,
                format!("{port} is open in {name}. Close it there; QuadCam does not share a port."),
            )));
        }
        if let Ok(v) = bf::battery_volts(ports, port, t) {
            let timer = self.gear_usb_timers().into_iter().find(|x| x.port == port);
            if v > bf::BATTERY_IN_VOLTS && timer.is_some_and(|x| x.remaining_s == Some(0)) {
                return Err(refusal(Refusal::new(
                    RefusalCode::UsbHeat,
                    "This FC has run on USB with its battery in past its limit. Unplug the battery and let it cool.",
                )));
            }
        }

        // Backup first; a failed backup stops everything.
        step("Backing up");
        let nobackup = |why: String| {
            refusal(Refusal::new(
                RefusalCode::NoBackup,
                format!("The backup failed: {why}. Nothing was written."),
            ))
        };
        let read = bf::read(
            ports,
            port,
            &bf::BACKUP.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            t,
        )
        .map_err(|e| nobackup(format!("{e:#}")))?;
        let files: Vec<(String, Vec<u8>)> = bf::BACKUP
            .iter()
            .filter_map(|c| read.file(c).map(|x| (c.to_string(), x.into_bytes())))
            .collect();
        let taken = snaps
            .take_files(
                &change.device,
                &read.info.identity,
                Trigger::BeforeApply,
                Utc::now(),
                &files,
                false,
            )
            .map_err(|e| nobackup(format!("{e:#}")))?;
        let backup_id = taken.backup.id.clone();
        let _ = self.after_backup(
            &change.device,
            DeviceKind::Fc,
            &read.info.identity,
            taken,
            Vec::new(),
            None,
        );

        // Same state as planned: the fresh dump gives the same digest.
        let fresh = Config::parse(&read.file("dump all").unwrap_or_default());
        let render = render_fc(edits, Some(&fresh));
        if let Some(p) = render.problems.first() {
            return Err(refusal(p.clone()));
        }
        if fcplan::digest(&change.device, &fresh, &render) != digest {
            return Err(refusal(Refusal::new(
                RefusalCode::BeforeMismatch,
                "The FC changed since the plan; plan again.",
            )));
        }

        // Write. Every `set` is range-checked on the FC before the first line is sent.
        step("Writing");
        let mut section = crate::gear::model::Section::Master;
        let mut precheck = |s: &mut CliSession| -> Result<()> {
            for l in &render.lines {
                match parse_cmd(l) {
                    Cmd::Select(sel) => {
                        section = sel;
                        s.command(l)?;
                    }
                    Cmd::Set { name, value } => {
                        let r = s.command(&format!("get {name}"))?;
                        let set = crate::gear::changes::SetRef {
                            section,
                            name,
                            value,
                        };
                        if let Some(p) = fcplan::range_problem(&set, &r.text) {
                            return Err(refusal(p));
                        }
                    }
                    _ => {}
                }
            }
            Ok(())
        };
        let (fcinfo, run) = bf::run_with(
            ports,
            port,
            Some(&change.device),
            &render.lines,
            t,
            &["version", "status", "diff all"],
            &mut precheck,
        )?;

        // Record.
        step("Verifying");
        let mut steps = vec![StepReport {
            name: "Back up".into(),
            state: StepState::Done,
            detail: Some(backup_id.clone()),
        }];
        let mut report = ApplyReport {
            change: change.id.clone(),
            device: change.device.clone(),
            status: ChangeStatus::Failed,
            steps: Vec::new(),
            backup: Some(backup_id),
            after_backup: None,
            sent: run.sent.clone(),
            failed_line: run.failed.clone(),
            verify: run.verify.clone(),
            saved: run.saved,
            files: Vec::new(),
            message: String::new(),
            notes: Vec::new(),
            at: Utc::now(),
        };
        let skipped = |n: &str| StepReport {
            name: n.into(),
            state: StepState::Skipped,
            detail: None,
        };
        if let Some(f) = &run.failed {
            steps.push(StepReport {
                name: "Write".into(),
                state: StepState::Failed,
                detail: Some(format!("{}: {}", f.line, f.text)),
            });
            steps.push(skipped("Read back"));
            steps.push(skipped("Verify"));
            report.message = format!(
                "The FC refused `{}`. Nothing was saved; the FC is as it was.",
                f.line
            );
        } else {
            steps.push(StepReport {
                name: "Write".into(),
                state: StepState::Done,
                detail: None,
            });
            steps.push(StepReport {
                name: "Read back".into(),
                state: StepState::Done,
                detail: None,
            });
            if run.verify.is_empty() {
                steps.push(StepReport {
                    name: "Verify".into(),
                    state: StepState::Done,
                    detail: None,
                });
                report.status = ChangeStatus::Verified;
                report.message = "Verified: the FC holds every line as written.".into();
            } else {
                steps.push(StepReport {
                    name: "Verify".into(),
                    state: StepState::Failed,
                    detail: Some(format!("{} lines differ", run.verify.len())),
                });
                report.message = format!(
                    "Saved, but {} lines do not read back as written. Restore the backup to undo.",
                    run.verify.len()
                );
            }
            // The state now, as a backup, so the next plan compares with it.
            let mut files: Vec<(String, Vec<u8>)> = run
                .after_replies
                .iter()
                .map(|r| {
                    (
                        r.line.clone(),
                        format!("{}\n{}\n", r.line, r.text).into_bytes(),
                    )
                })
                .collect();
            if let Some(d) = &run.after_dump {
                files.push(("dump all".into(), format!("dump all\n{d}\n").into_bytes()));
            }
            if let Ok(rep) = snaps.take_files(
                &change.device,
                &fcinfo.identity,
                Trigger::AfterApply,
                Utc::now(),
                &files,
                false,
            ) {
                report.after_backup = Some(rep.backup.id.clone());
                let _ = self.after_backup(
                    &change.device,
                    DeviceKind::Fc,
                    &fcinfo.identity,
                    rep,
                    Vec::new(),
                    None,
                );
            }
        }
        report.steps = steps;
        self.finish_change(change, &report)?;
        if report.status == ChangeStatus::Verified {
            Ok((fcinfo, report))
        } else {
            let msg = report.message.clone();
            *stash.lock().unwrap() = Some(report);
            Err(anyhow!("{msg}"))
        }
    }

    /// Records an apply: the change's status (a Try change that verified waits as Applied
    /// for Keep or Revert), its report, and, for a restore that verified, the change it
    /// undoes.
    pub(super) fn finish_change(&self, change: &StagedChange, report: &ApplyReport) -> Result<()> {
        let ch = self.changes();
        let verified = report.status == ChangeStatus::Verified;
        let (status, note) = if verified && change.status == ChangeStatus::Try {
            (
                ChangeStatus::Applied,
                format!("{} Keep it or revert it.", report.message),
            )
        } else {
            (report.status, report.message.clone())
        };
        ch.set_status(&change.id, status, &note)?;
        ch.write_report(&change.id, &serde_json::to_vec_pretty(report)?)?;
        if let (true, Some(orig)) = (verified, &change.reverts) {
            let _ = ch.set_status(
                orig,
                ChangeStatus::Reverted,
                &format!("Reverted by {}", change.id),
            );
        }
        self.hooks.gear_changed();
        Ok(())
    }

    /// Board notes for the sheet's header.
    pub fn gear_apply_notes(&self, board: Option<&str>, version: Option<&str>) -> Vec<String> {
        boards::after_job(board, version)
    }
}

/// A title from the first edit.
fn default_title(edits: &[Edit]) -> String {
    match edits.first() {
        Some(Edit::FcSet { name, value, .. }) => format!("Set {name} = {value}"),
        Some(Edit::FcLines { lines }) => {
            match lines.iter().filter(|l| !l.trim().is_empty()).count() {
                1 => format!(
                    "Run `{}`",
                    lines.iter().find(|l| !l.trim().is_empty()).unwrap().trim()
                ),
                n => format!("{n} CLI lines"),
            }
        }
        Some(Edit::Restore { .. }) => "Restore a backup".into(),
        _ => "FC change".into(),
    }
}
