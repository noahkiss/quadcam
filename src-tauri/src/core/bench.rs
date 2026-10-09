//! `Core`'s Bench half (design 7.7): Keep and Revert for a Try change, copying settings
//! between quads, and the `apply_ready` on-connect step.
//!
//! - **Try** is a Ready change that waits for the flight: after it verifies it is
//!   `Applied`. **Keep** makes it `Verified`. **Revert** stages a restore of the backup the
//!   apply took (the files the apply wrote, for a card) and marks the change `Reverted`
//!   when that restore verifies.
//! - **Copy** reads two FCs' latest backups (or one named backup) and stages the settings
//!   that differ as one FC change (`gear/copy.rs`).
//! - **Apply ready** runs on connect only when `gearOnConnect` lists it for the kind (off by
//!   default). It plans each Ready change of the device, and a plan whose checks all pass
//!   goes to the apply sheet. Nothing is written until the person clicks Apply; with no
//!   window to click in, the step does nothing.

use super::{Core, OnConnectHook, Skip};
use crate::gear::apply::{ApplyPlanParams, ApplyReport, ApplyRequest};
use crate::gear::copy::{self, CopyPlan, CopySelect};
use crate::gear::model::{ChangeStatus, Connected, DeviceKind, Edit, StagedChange};
use crate::gear::Automation;
use crate::session::Editor;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;

/// `gear_copy_plan` and `gear_copy_stage`: where the settings come from and go to.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct CopyParams {
    /// An FC's device id (its latest backup), or a backup id (`<device>/<snapshot>`).
    pub from: String,
    /// The FC that gets the settings (its latest backup is the base).
    pub to: String,
    #[serde(flatten)]
    pub select: CopySelect,
    #[serde(default)]
    pub editor: Option<Editor>,
}

impl Core {
    /// Applied (a Try change that verified) to Verified.
    pub fn gear_change_keep(&self, id: &str) -> Result<StagedChange> {
        let ch = self.changes();
        let c = ch.get(id)?;
        if c.status != ChangeStatus::Applied {
            bail!(
                "Change {id} is {:?}; only an applied Try change waits for Keep or Revert.",
                c.status
            );
        }
        let c = ch.set_status(id, ChangeStatus::Verified, "Kept")?;
        self.hooks.gear_changed();
        Ok(c)
    }

    /// Stages a restore of the backup the change's apply took. The change becomes
    /// Reverted when that restore verifies.
    pub fn gear_change_revert(&self, id: &str) -> Result<StagedChange> {
        let ch = self.changes();
        let c = ch.get(id)?;
        if !matches!(c.status, ChangeStatus::Applied | ChangeStatus::Verified) {
            bail!(
                "Change {id} is {:?}; only an applied change can be reverted.",
                c.status
            );
        }
        if let Some(r) = ch
            .all()
            .into_iter()
            .find(|x| x.reverts.as_deref() == Some(id) && x.status.staged())
        {
            bail!("A revert of this change is already staged: {}.", r.id);
        }
        let report: ApplyReport =
            serde_json::from_slice(&ch.read_report(id).with_context(|| {
                format!("Change {id} has no apply report; nothing to revert.")
            })?)?;
        let backup = report
            .backup
            .clone()
            .context("The apply took no backup; nothing to revert to.")?;
        let paths = match self.change_kind(&c)? {
            DeviceKind::Fc => vec!["dump all".to_string()],
            _ if report.files.is_empty() => {
                bail!("The apply wrote no card files; nothing to revert.")
            }
            _ => report.files.clone(),
        };
        let edit = Edit::Restore { backup, paths };
        self.stage_inner(
            &super::StageParams {
                device: c.device.clone(),
                title: Some(format!("Revert: {}", c.title)),
                edits: vec![edit],
                note: Some(format!("Reverts {id}")),
                editor: None,
                draft: false,
            },
            Some(id.to_string()),
        )
    }

    /// The dump all behind `from`: a backup id, or a device's latest backup.
    fn copy_source(&self, from: &str) -> Result<(String, String)> {
        let snaps = self.snapshots();
        let id = if from.contains('/') {
            from.to_string()
        } else {
            snaps
                .latest(from)
                .with_context(|| format!("{from:?} has no backup to copy from."))?
                .id
        };
        let text = snaps
            .read(&id, Some("dump all"))
            .with_context(|| format!("Backup {id} has no `dump all`."))?
            .text
            .context("The backup's dump is not text.")?;
        Ok((id, text))
    }

    /// What copying would stage, with the compatibility checks and a diff. Writes nothing.
    pub fn gear_copy_plan(&self, p: &CopyParams) -> Result<CopyPlan> {
        let to = self
            .gear_store()
            .device(p.to.trim())?
            .with_context(|| format!("Unknown device {:?}. See `gear devices`.", p.to))?;
        if to.kind != DeviceKind::Fc {
            bail!(
                "Settings copy to an FC; {:?} is a {}.",
                p.to,
                to.kind.label()
            );
        }
        let (from_id, source) = self.copy_source(p.from.trim())?;
        if from_id.split('/').next() == Some(to.id.as_str()) {
            bail!("That is the same FC; pick another quad to copy from.");
        }
        let base = self
            .fc_base(&to.id)
            .with_context(|| format!("{:?} has no backup yet. Back it up first.", p.to))?;
        Ok(copy::plan(
            &from_id,
            &base.backup.id,
            &source,
            &base.dump,
            &p.select,
        ))
    }

    /// Stages the copy as one FC change. Refused (`incompatible`) when a check fails.
    pub fn gear_copy_stage(&self, p: &CopyParams) -> Result<StagedChange> {
        let plan = self.gear_copy_plan(p)?;
        if let Some(r) = plan.checks.iter().find_map(|c| c.refusal.clone()) {
            return Err(super::apply::refusal(r));
        }
        let from_name = self
            .gear_store()
            .device(p.from.split('/').next().unwrap_or(&p.from))?
            .map(|d| d.display_name())
            .unwrap_or_else(|| p.from.clone());
        let parts: Vec<&str> = p.select.parts.iter().map(|x| x.label()).collect();
        let what = if parts.is_empty() {
            format!("{} settings", p.select.settings.len())
        } else {
            parts.join(", ")
        };
        self.stage_inner(
            &super::StageParams {
                device: p.to.trim().to_string(),
                title: Some(format!("Copy {what} from {from_name}")),
                edits: plan.edits,
                note: Some(format!("Copied from backup {}", plan.from_backup)),
                editor: p.editor,
                draft: false,
            },
            None,
        )
    }

    /// The `apply_ready` on-connect step for one device.
    fn apply_ready_run(&self, c: &Connected) -> Result<()> {
        if !self.hooks.has_gui() {
            return Err(Skip.into());
        }
        let Some(id) = c.id.clone() else {
            return Err(Skip.into());
        };
        let ready: Vec<StagedChange> = self
            .changes()
            .all()
            .into_iter()
            .filter(|x| x.device == id && x.status == ChangeStatus::Ready)
            .collect();
        if ready.is_empty() {
            return Err(Skip.into());
        }
        for ch in ready {
            let plan = self.gear_apply_plan(&ApplyPlanParams {
                id: ch.id.clone(),
                port: None,
            })?;
            if let Some(r) = plan.checks.iter().find_map(|k| k.refusal.clone()) {
                return Err(anyhow!("{}: {}", ch.title, r.reason));
            }
            let report = self.gear_apply(&ApplyRequest {
                id: ch.id.clone(),
                digest: plan.digest,
                confirm: true,
                port: None,
            })?;
            if report.status != ChangeStatus::Verified {
                return Err(anyhow!("{}: {}", ch.title, report.message));
            }
        }
        Ok(())
    }
}

/// The on-connect step "Apply ready changes" (`Automation::ApplyReady`), registered by the
/// app. A radio card is unmounted at the end of its apply; the run's own cue is the
/// apply's.
pub fn apply_ready_hooks() -> Vec<OnConnectHook> {
    let run: super::HookFn = Arc::new(|core: &Core, c: &Connected| core.apply_ready_run(c));
    vec![OnConnectHook {
        name: "Apply ready changes",
        automation: Automation::ApplyReady,
        kinds: vec![DeviceKind::Fc, DeviceKind::Radio],
        run,
    }]
}
