//! The apply engine's shared types and helpers (design 8). Every write to a device goes
//! stage, plan, checks, confirm, backup, write, read back, verify, record. `apply/fc.rs`
//! holds the FC's plan and checks; `core/apply.rs` runs the job (port, backup, write).

pub mod card;
pub mod fc;

use super::bf::cli::Reply;
use super::bf::dump::VerifyFail;
use super::model::{ChangeStatus, Check, Refusal};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use specta::Type;

/// `gear_apply_plan`: the change to plan. `port` picks the FC when several are plugged in.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct ApplyPlanParams {
    pub id: String,
    #[serde(default)]
    pub port: Option<String>,
}

/// `gear_apply`: the plan's digest and the confirm.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct ApplyRequest {
    pub id: String,
    pub digest: String,
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub port: Option<String>,
}

/// How one step of an apply ended.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Done,
    Failed,
    /// Not reached: an earlier step failed.
    Skipped,
}

/// One step of an apply: Back up, Write, Read back, Verify.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct StepReport {
    pub name: String,
    pub state: StepState,
    #[serde(default)]
    pub detail: Option<String>,
}

/// What an apply did. It is also written to the change's `report.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct ApplyReport {
    pub change: String,
    pub device: String,
    /// `Verified`, or `Failed`.
    pub status: ChangeStatus,
    pub steps: Vec<StepReport>,
    /// The backup taken before the write (always kept). "Restore backup" stages it back.
    pub backup: Option<String>,
    /// The state read back after the save.
    pub after_backup: Option<String>,
    /// Each line sent with the FC's answer, up to a failed one.
    pub sent: Vec<Reply>,
    /// The line the FC refused; nothing was saved.
    pub failed_line: Option<Reply>,
    /// Lines the FC does not hold as written after the save.
    pub verify: Vec<VerifyFail>,
    pub saved: bool,
    /// Card files the apply wrote, and files it deleted: what a Revert restores.
    #[serde(default)]
    pub files: Vec<String>,
    /// One sentence for the person.
    pub message: String,
    /// Known issues of this board and build.
    #[serde(default)]
    pub notes: Vec<String>,
    pub at: DateTime<Utc>,
}

/// A passed check.
pub fn pass(name: &str) -> Check {
    Check {
        name: name.into(),
        ok: true,
        refusal: None,
    }
}

/// A check from a guard's answer.
pub fn check(name: &str, r: Result<(), Refusal>) -> Check {
    match r {
        Ok(()) => pass(name),
        Err(refusal) => Check {
            name: name.into(),
            ok: false,
            refusal: Some(refusal),
        },
    }
}

/// The first failed check as an error (`Refused (code): reason`).
pub fn first_refusal(checks: &[Check]) -> Option<Refusal> {
    checks.iter().find_map(|c| c.refusal.clone())
}
