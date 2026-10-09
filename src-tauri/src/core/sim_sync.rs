//! `Core`'s sim sync (design 6.6, 8): write a quad's rate profile into the sims on this Mac.
//!
//! The same path as every write (design 8.1): a plan with checks, a diff, warnings and a
//! digest (`gear_sim_sync_plan`), then the apply (`gear_sim_sync`): the checks again, the
//! confirm, a backup of every file (the gear store, device `sim-<id>`, kept), one atomic
//! write per file, a read back and a parse. Any failure puts every written file back.
//!
//! - **Refuses while the game runs.** The process list is `sims::running_processes`; under
//!   cargo `QUADCAM_SIMS_RUNNING` stands in for it, and a write refuses any path outside
//!   the temporary folder, so no test touches a real player's files.
//! - **Confirm.** The sheet's own Apply calls `gear_sim_sync_click`. Any other caller needs
//!   the plan's digest and `confirm`, and with the app running the person also clicks Apply
//!   in the sheet (`Hooks::confirm_apply`, with a stand-in change).

use super::Core;
use crate::gear::apply::sim::{
    self as simplan, backup_device, pseudo_change, reads_as, write_atomic, writes_allowed,
    SimSyncParams, SimSyncRequest,
};
use crate::gear::apply::{first_refusal, ApplyReport, StepReport, StepState};
use crate::gear::model::{ApplyPlan, ChangeStatus, Identity, Refusal, RefusalCode, Trigger};
use crate::gear::sims;
use anyhow::{bail, Result};
use chrono::Utc;
use std::path::Path;

impl Core {
    /// What syncing would write, with every guard's result. Writes nothing.
    pub fn gear_sim_sync_plan(&self, p: &SimSyncParams) -> Result<ApplyPlan> {
        let running = sims::running_processes();
        Ok(self
            .sim_planned(p, &crate::paths::home_dir(), &|n| {
                running.iter().any(|r| r == n)
            })?
            .plan)
    }

    /// `gear_sim_sync_plan` with the home folder and the process check given (tests).
    pub fn gear_sim_sync_plan_at(
        &self,
        p: &SimSyncParams,
        home: &Path,
        running: &dyn Fn(&str) -> bool,
    ) -> Result<ApplyPlan> {
        Ok(self.sim_planned(p, home, running)?.plan)
    }

    fn sim_planned(
        &self,
        p: &SimSyncParams,
        home: &Path,
        running: &dyn Fn(&str) -> bool,
    ) -> Result<simplan::SimPlanned> {
        let quad = self.quad_profile(&p.paths, &p.device, &p.backup, p.profile)?;
        Ok(simplan::plan(&quad, &p.sims, home, running))
    }

    /// Writes the planned files. Needs the plan's digest and `confirm`; with the app
    /// running, the person also clicks Apply in the sheet.
    pub fn gear_sim_sync(&self, req: &SimSyncRequest) -> Result<ApplyReport> {
        let running = sims::running_processes();
        self.sim_sync_at(req, false, &crate::paths::home_dir(), &|n| {
            running.iter().any(|r| r == n)
        })
    }

    /// The apply sheet's own Apply click: the click is the confirm.
    pub fn gear_sim_sync_click(&self, req: &SimSyncRequest) -> Result<ApplyReport> {
        let running = sims::running_processes();
        self.sim_sync_at(req, true, &crate::paths::home_dir(), &|n| {
            running.iter().any(|r| r == n)
        })
    }

    /// `gear_sim_sync` with the home folder and the process check given (tests).
    pub fn sim_sync_at(
        &self,
        req: &SimSyncRequest,
        from_gui: bool,
        home: &Path,
        running: &dyn Fn(&str) -> bool,
    ) -> Result<ApplyReport> {
        if !req.confirm {
            bail!("Refused: sim sync needs the plan's digest and confirm=true.");
        }
        let planned = self.sim_planned(&req.params, home, running)?;
        if let Some(r) = first_refusal(&planned.plan.checks) {
            return Err(r.into());
        }
        if planned.plan.digest != req.digest {
            return Err(Refusal::new(
                RefusalCode::BeforeMismatch,
                "The sim files changed since you read the plan; plan again.",
            )
            .into());
        }
        if !from_gui {
            self.hooks
                .confirm_apply(&pseudo_change(&planned.plan), &planned.plan)?;
        }
        let under_cargo = std::env::var_os("CARGO_MANIFEST_DIR").is_some();
        for w in &planned.writes {
            if !writes_allowed(&w.path, under_cargo) {
                return Err(Refusal::new(
                    RefusalCode::Disabled,
                    "Sim writes outside the temporary folder are off in this process (tests).",
                )
                .into());
            }
        }

        // The guards again, right before the first write.
        let mut steps: Vec<StepReport> = Vec::new();
        for w in &planned.writes {
            let proc = sims::all()
                .into_iter()
                .find(|s| s.id() == w.sim)
                .map(|s| s.process())
                .unwrap_or("");
            if running(proc) {
                return Err(Refusal::new(
                    RefusalCode::SimRunning,
                    format!("Quit {} first.", w.name),
                )
                .into());
            }
            if std::fs::read(&w.path).ok().as_deref() != Some(w.before.as_slice()) {
                return Err(Refusal::new(
                    RefusalCode::BeforeMismatch,
                    format!("{} changed since the plan; plan again.", w.shown),
                )
                .into());
            }
        }

        // Backup first; a failed backup stops everything.
        let snaps = self.snapshots();
        let mut backups = Vec::new();
        for w in &planned.writes {
            let taken = snaps
                .take_files(
                    &backup_device(&w.sim),
                    &Identity::default(),
                    Trigger::BeforeApply,
                    Utc::now(),
                    &[(w.rel.clone(), w.before.clone())],
                    false,
                )
                .map_err(|e| {
                    Refusal::new(
                        RefusalCode::NoBackup,
                        format!(
                            "The backup of {} failed: {e:#}. Nothing was written.",
                            w.name
                        ),
                    )
                })?;
            steps.push(StepReport {
                name: format!("Back up {}", w.name),
                state: StepState::Done,
                detail: Some(taken.backup.id.clone()),
            });
            backups.push(taken.backup.id);
        }

        // Write each file, read it back and parse it. A failure puts every file back.
        let mut done: Vec<usize> = Vec::new();
        let mut failure: Option<String> = None;
        for (i, w) in planned.writes.iter().enumerate() {
            let sim = sims::all().into_iter().find(|s| s.id() == w.sim).unwrap();
            let outcome = write_atomic(&w.path, &w.after);
            match outcome {
                Ok(true) => {
                    done.push(i);
                    steps.push(StepReport {
                        name: format!("Write {}", w.name),
                        state: StepState::Done,
                        detail: Some(w.shown.clone()),
                    });
                    let raw = std::fs::read(&w.path).unwrap_or_default();
                    if reads_as(sim, &w.path, &raw, w) {
                        steps.push(StepReport {
                            name: format!("Read back {}", w.name),
                            state: StepState::Done,
                            detail: Some(format!("{} holds the new rates", w.profile)),
                        });
                    } else {
                        steps.push(StepReport {
                            name: format!("Read back {}", w.name),
                            state: StepState::Failed,
                            detail: Some(format!("{} does not read as written", w.shown)),
                        });
                        failure = Some(format!("{} does not read back as written.", w.shown));
                        break;
                    }
                }
                Ok(false) => {
                    done.push(i);
                    steps.push(StepReport {
                        name: format!("Write {}", w.name),
                        state: StepState::Failed,
                        detail: Some("the bytes read back differ".into()),
                    });
                    failure = Some(format!("{} read back different bytes.", w.shown));
                    break;
                }
                Err(e) => {
                    steps.push(StepReport {
                        name: format!("Write {}", w.name),
                        state: StepState::Failed,
                        detail: Some(format!("{e:#}")),
                    });
                    failure = Some(format!("Writing {} failed: {e:#}", w.shown));
                    break;
                }
            }
        }
        let mut rolled_back = true;
        if failure.is_some() {
            for &i in &done {
                let w = &planned.writes[i];
                if !matches!(write_atomic(&w.path, &w.before), Ok(true)) {
                    rolled_back = false;
                }
            }
            steps.push(StepReport {
                name: "Roll back".into(),
                state: if rolled_back {
                    StepState::Done
                } else {
                    StepState::Failed
                },
                detail: Some(format!("{} file(s) put back", done.len())),
            });
        }
        let ok = failure.is_none();
        let message = match &failure {
            None => format!(
                "Verified: {} hold the new rates.",
                planned
                    .writes
                    .iter()
                    .map(|w| format!("{} ({})", w.name, w.profile))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Some(f) if rolled_back => format!("{f} Every file was put back as it was."),
            Some(f) => format!(
                "{f} Putting the old bytes back failed too: restore from the backup ({}).",
                backups.join(", ")
            ),
        };
        self.hooks.gear_changed();
        Ok(ApplyReport {
            change: simplan::CHANGE_ID.into(),
            device: simplan::DEVICE.into(),
            status: if ok {
                ChangeStatus::Verified
            } else {
                ChangeStatus::Failed
            },
            steps,
            backup: backups.first().cloned(),
            after_backup: None,
            sent: Vec::new(),
            failed_line: None,
            verify: Vec::new(),
            saved: ok,
            files: if ok {
                planned.writes.iter().map(|w| w.shown.clone()).collect()
            } else {
                Vec::new()
            },
            message,
            notes: planned.plan.warnings.clone(),
            at: Utc::now(),
        })
    }
}
