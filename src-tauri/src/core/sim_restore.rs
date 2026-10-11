//! `Core`'s restore of a sim's backup (design 6.6, 8): put a sim's rate file back as it was
//! before a sync.
//!
//! The same path as every write (design 8.1): a plan with checks, a diff, warnings and a
//! digest (`gear_sim_restore_plan`), then the apply (`gear_sim_restore`): the checks again,
//! the confirm, a backup of the file as it is now (kept, so a restore can be undone), one
//! atomic write per file, a byte-for-byte read back, and every file put back if one fails.
//! A process started by cargo writes only under the temporary folder, and the game must be
//! closed, as for a sync.

use super::Core;
use crate::gear::apply::sim::{
    self as simplan, backup_device, can_write, diff_lines, fail, tilde, write_atomic,
    writes_allowed, DEVICE,
};
use crate::gear::apply::{check, first_refusal, pass, ApplyReport, StepReport, StepState};
use crate::gear::backup::blob_of;
use crate::gear::blobs;
use crate::gear::model::{
    ApplyPlan, Backup, ChangeStatus, DiffItem, DiffLine, Identity, LineOp, Refusal, RefusalCode,
    StagedChange, Trigger,
};
use crate::gear::sims::{self, Sim};
use anyhow::{bail, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// `gear_sim_restore_plan`: which sim, and which of its backups (default the newest that
/// differs from the file now).
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct SimRestoreParams {
    /// A sim id: `liftoff`, `micro`, `uncrashed`, `zone`.
    pub sim: String,
    /// A backup id from `gear_backups` for `sim-<id>`.
    #[serde(default)]
    pub backup: Option<String>,
}

/// `gear_sim_restore`: the same params, the plan's digest and the confirm.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct SimRestoreRequest {
    #[serde(flatten)]
    pub params: SimRestoreParams,
    pub digest: String,
    #[serde(default)]
    pub confirm: bool,
}

/// One file to put back.
struct RestoreWrite {
    path: PathBuf,
    shown: String,
    rel: String,
    before: Vec<u8>,
    after: Vec<u8>,
}

struct RestorePlanned {
    plan: ApplyPlan,
    writes: Vec<RestoreWrite>,
    sim: &'static dyn Sim,
}

/// The stand-in change an agent's confirm request and the apply sheet carry.
fn pseudo_restore(plan: &ApplyPlan) -> StagedChange {
    StagedChange {
        id: RESTORE_ID.into(),
        title: "Restore a sim backup".into(),
        ..simplan::pseudo_change(plan)
    }
}

/// The id of the stand-in change a restore carries.
pub const RESTORE_ID: &str = "sim-restore";

/// A path inside the backup that stays inside the home folder.
fn safe_rel(rel: &str) -> bool {
    let p = Path::new(rel);
    !rel.is_empty()
        && p.is_relative()
        && !p
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
}

impl Core {
    /// What restoring would write, with every guard's result. Writes nothing.
    pub fn gear_sim_restore_plan(&self, p: &SimRestoreParams) -> Result<ApplyPlan> {
        let running = sims::running_processes();
        Ok(self
            .sim_restore_planned(p, &crate::paths::home_dir(), &|n| {
                running.iter().any(|r| r == n)
            })?
            .plan)
    }

    /// `gear_sim_restore_plan` with the home folder and the process check given (tests).
    pub fn gear_sim_restore_plan_at(
        &self,
        p: &SimRestoreParams,
        home: &Path,
        running: &dyn Fn(&str) -> bool,
    ) -> Result<ApplyPlan> {
        Ok(self.sim_restore_planned(p, home, running)?.plan)
    }

    fn sim_restore_planned(
        &self,
        p: &SimRestoreParams,
        home: &Path,
        running: &dyn Fn(&str) -> bool,
    ) -> Result<RestorePlanned> {
        let id = p.sim.trim().to_ascii_lowercase();
        let Some(sim) = sims::all().into_iter().find(|s| s.id() == id) else {
            bail!(
                "{:?} is not a sim QuadCam knows (liftoff, micro, uncrashed, zone).",
                p.sim
            );
        };
        let who = sim.name();
        let device = backup_device(sim.id());
        let snaps = self.snapshots();
        let blobstore = snaps.blobs();
        let read = |b: &Backup| -> Result<Vec<(String, Vec<u8>)>> {
            b.files
                .iter()
                .map(|f| Ok((f.path.clone(), blobstore.get(&blob_of(f))?)))
                .collect()
        };
        let current = |rel: &str| std::fs::read(home.join(rel)).ok();

        let mut checks = Vec::new();
        let mut warnings = Vec::new();
        let mut diff = Vec::new();
        let mut writes: Vec<RestoreWrite> = Vec::new();

        // The backup: the one named, else the newest that differs from the files now.
        let list = snaps.list(&device);
        let backup = match p.backup.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
            Some(b) => match snaps.get(b) {
                Ok(x) if x.device == device => Some(x),
                _ => {
                    checks.push(fail(
                        "Backup found",
                        RefusalCode::NoBackup,
                        format!("{b:?} is not a backup of {who}. List them with gear backups."),
                    ));
                    None
                }
            },
            None => {
                let newest = list.iter().rev().find(|b| {
                    read(b).is_ok_and(|files| {
                        files
                            .iter()
                            .any(|(rel, bytes)| current(rel).as_deref() != Some(bytes))
                    })
                });
                if newest.is_none() {
                    checks.push(fail(
                        "Backup found",
                        RefusalCode::NoBackup,
                        if list.is_empty() {
                            format!("{who} has no backup: QuadCam backs a sim file up before each sync.")
                        } else {
                            format!("{who} already holds the file of every backup; there is nothing to restore.")
                        },
                    ));
                }
                newest.cloned()
            }
        };
        if let Some(b) = &backup {
            checks.push(pass("Backup found"));
            match read(b) {
                Err(e) => checks.push(fail(
                    "Backup readable",
                    RefusalCode::NoBackup,
                    format!("The backup cannot be read: {e:#}"),
                )),
                Ok(files) => {
                    checks.push(pass("Backup readable"));
                    checks.push(if running(sim.process()) {
                        fail(
                            &format!("Sim closed ({who})"),
                            RefusalCode::SimRunning,
                            format!("Quit {who} first."),
                        )
                    } else {
                        pass(&format!("Sim closed ({who})"))
                    });
                    for (rel, after) in files {
                        if !safe_rel(&rel) {
                            checks.push(fail(
                                "Path inside the home folder",
                                RefusalCode::NotWritable,
                                format!("{rel:?} in the backup does not name a file under the home folder."),
                            ));
                            continue;
                        }
                        let path = home.join(&rel);
                        let shown = tilde(home, &path);
                        let Some(before) = current(&rel) else {
                            checks.push(fail(
                                &format!("File present ({who})"),
                                RefusalCode::BeforeMismatch,
                                format!("{shown} is not on this Mac any more; QuadCam restores a file that is there."),
                            ));
                            continue;
                        };
                        checks.push(check(
                            &format!("Writable ({who})"),
                            can_write(&path).then_some(()).ok_or_else(|| {
                                Refusal::new(
                                    RefusalCode::NotWritable,
                                    format!("{shown} cannot be written: check its permissions and its folder."),
                                )
                            }),
                        ));
                        diff.push(file_diff(sim, who, &shown, &path, &before, &after));
                        writes.push(RestoreWrite {
                            path,
                            shown,
                            rel,
                            before,
                            after,
                        });
                    }
                    if writes.iter().all(|w| w.before == w.after) && checks.iter().all(|c| c.ok) {
                        checks.push(fail(
                            "Something differs",
                            RefusalCode::Incompatible,
                            "The file already holds what the backup holds; nothing to restore.",
                        ));
                    }
                    warnings.push(format!(
                        "This puts back the file as it was on {}. Changes made in the game or by a later sync are lost; QuadCam backs the file up first, so this can be undone.",
                        b.taken_at.format("%Y-%m-%d %H:%M UTC")
                    ));
                }
            }
        }

        let ready = checks.iter().all(|c| c.ok) && !writes.is_empty();
        let digest = if ready {
            let mut text = format!(
                "simrestore|{}",
                backup.as_ref().map_or("", |b| b.id.as_str())
            );
            for w in &writes {
                text.push_str(&format!(
                    "\n{}|{}|{}",
                    w.rel,
                    blobs::hash(&w.before),
                    blobs::hash(&w.after)
                ));
            }
            blobs::hash(text.as_bytes())
        } else {
            String::new()
        };
        Ok(RestorePlanned {
            plan: ApplyPlan {
                change: RESTORE_ID.into(),
                device: Identity::default(),
                checks,
                diff,
                digest,
                warnings,
            },
            writes: if ready { writes } else { Vec::new() },
            sim,
        })
    }

    /// Puts the planned files back. Needs the plan's digest and `confirm`; with the app
    /// running, the person also clicks Apply in the sheet.
    pub fn gear_sim_restore(&self, req: &SimRestoreRequest) -> Result<ApplyReport> {
        let running = sims::running_processes();
        self.agent_write(|| {
            self.sim_restore_at(req, false, &crate::paths::home_dir(), &|n| {
                running.iter().any(|r| r == n)
            })
        })
    }

    /// The apply sheet's own Apply click: the click is the confirm.
    pub fn gear_sim_restore_click(&self, req: &SimRestoreRequest) -> Result<ApplyReport> {
        let running = sims::running_processes();
        self.sim_restore_at(req, true, &crate::paths::home_dir(), &|n| {
            running.iter().any(|r| r == n)
        })
    }

    /// `gear_sim_restore` with the home folder and the process check given (tests).
    pub fn sim_restore_at(
        &self,
        req: &SimRestoreRequest,
        from_gui: bool,
        home: &Path,
        running: &dyn Fn(&str) -> bool,
    ) -> Result<ApplyReport> {
        if !req.confirm {
            bail!("Refused: a restore needs the plan's digest and confirm=true.");
        }
        let planned = self.sim_restore_planned(&req.params, home, running)?;
        if let Some(r) = first_refusal(&planned.plan.checks) {
            return Err(r.into());
        }
        if planned.plan.digest != req.digest {
            return Err(Refusal::new(
                RefusalCode::BeforeMismatch,
                "The sim files or the backup changed since you read the plan; plan again.",
            )
            .into());
        }
        if !from_gui {
            self.hooks
                .confirm_apply(&pseudo_restore(&planned.plan), &planned.plan)?;
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
        let who = planned.sim.name();
        // The guards again, right before the first write.
        if running(planned.sim.process()) {
            return Err(Refusal::new(RefusalCode::SimRunning, format!("Quit {who} first.")).into());
        }
        for w in &planned.writes {
            if std::fs::read(&w.path).ok().as_deref() != Some(w.before.as_slice()) {
                return Err(Refusal::new(
                    RefusalCode::BeforeMismatch,
                    format!("{} changed since the plan; plan again.", w.shown),
                )
                .into());
            }
        }

        // Back up the files as they are now (kept): a restore can be undone. A failed backup
        // stops everything.
        let mut steps: Vec<StepReport> = Vec::new();
        let mut backups = Vec::new();
        let snaps = self.snapshots();
        for w in &planned.writes {
            let taken = snaps
                .take_files(
                    &backup_device(planned.sim.id()),
                    &Identity::default(),
                    Trigger::BeforeApply,
                    Utc::now(),
                    &[(w.rel.clone(), w.before.clone())],
                    false,
                )
                .map_err(|e| {
                    Refusal::new(
                        RefusalCode::NoBackup,
                        format!("The backup of {who} failed: {e:#}. Nothing was written."),
                    )
                })?;
            steps.push(StepReport {
                name: format!("Back up {who}"),
                state: StepState::Done,
                detail: Some(taken.backup.id.clone()),
            });
            backups.push(taken.backup.id);
        }

        let mut done: Vec<usize> = Vec::new();
        let mut failure: Option<String> = None;
        for (i, w) in planned.writes.iter().enumerate() {
            let (state, detail) = match write_atomic(&w.path, &w.after) {
                Ok(true) => {
                    done.push(i);
                    (StepState::Done, w.shown.clone())
                }
                Ok(false) => {
                    done.push(i);
                    failure = Some(format!("{} read back different bytes.", w.shown));
                    (StepState::Failed, "the bytes read back differ".into())
                }
                Err(e) => {
                    failure = Some(format!("Writing {} failed: {e:#}", w.shown));
                    (StepState::Failed, format!("{e:#}"))
                }
            };
            steps.push(StepReport {
                name: format!("Write {who}"),
                state,
                detail: Some(detail),
            });
            if failure.is_some() {
                break;
            }
            steps.push(StepReport {
                name: format!("Read back {who}"),
                state: StepState::Done,
                detail: Some("the file holds the backup's bytes".into()),
            });
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
            None => format!("Verified: {who} holds the backed-up file again."),
            Some(f) if rolled_back => format!("{f} Every file was put back as it was."),
            Some(f) => format!(
                "{f} Putting the old bytes back failed too: restore from the backup ({}).",
                backups.join(", ")
            ),
        };
        self.hooks.gear_changed();
        Ok(ApplyReport {
            change: RESTORE_ID.into(),
            device: DEVICE.into(),
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

/// The diff of one file: the rates of each profile that differs, when both files parse as
/// the sim's; else the file named.
fn file_diff(
    sim: &dyn Sim,
    who: &str,
    shown: &str,
    path: &Path,
    before: &[u8],
    after: &[u8],
) -> DiffItem {
    let label = format!("{who}: {shown}");
    let (Ok(now), Ok(then)) = (sim.parse_file(before, path), sim.parse_file(after, path)) else {
        return DiffItem::Files {
            label,
            put: vec![shown.to_string()],
            delete: Vec::new(),
        };
    };
    let mut lines: Vec<DiffLine> = Vec::new();
    for p in &then.profiles {
        let Some(n) = now.profiles.iter().find(|n| n.name == p.name) else {
            continue;
        };
        if let (Some(have), Some(want)) = (n.rates, p.rates) {
            let d = diff_lines(&have, &want, n.throttle, p.throttle.as_ref());
            if !d.is_empty() {
                lines.push(DiffLine {
                    op: LineOp::Same,
                    text: format!("Profile {}", p.name),
                });
                lines.extend(d);
            }
        }
    }
    if lines.is_empty() {
        lines.push(DiffLine {
            op: LineOp::Same,
            text: "the rates are the same; other parts of the file differ".into(),
        });
    }
    DiffItem::Lines { label, lines }
}
