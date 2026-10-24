//! Device actions that are not staged changes: card repair, a blackbox pull that erases,
//! a blackbox erase and a volume rename. Asked for by an agent while the app runs, each
//! shows the apply sheet and waits for the person's click (`Hooks::confirm_apply`), as a
//! staged change's apply does, and its outcome goes back to that sheet (`Hooks::apply_done`).
//! The app's own buttons call the `_click` methods, which ask nothing: the click is the
//! confirm. A headless core's hooks approve at once.

use super::Core;
use crate::gear::apply::{ApplyReport, StepReport, StepState};
use crate::gear::model::{ApplyPlan, ChangeStatus, Check, DiffItem, Identity, StagedChange};
use anyhow::{anyhow, Result};

/// The stand-in change ids, which the sheet names its kind by.
pub const CARD_REPAIR: &str = "card-repair";
pub const BLACKBOX_PULL: &str = "blackbox-pull";
pub const BLACKBOX_ERASE: &str = "blackbox-erase";
pub const VOLUME_LABEL: &str = "volume-label";

/// What the sheet shows for one action.
pub(super) struct ActionAsk {
    pub id: &'static str,
    pub title: String,
    /// The saved device id, else the link handle (an FC port).
    pub device: String,
    pub identity: Identity,
    pub diff: Vec<DiffItem>,
    /// The guards that already passed.
    pub checks: Vec<Check>,
    pub warnings: Vec<String>,
}

impl ActionAsk {
    fn change(&self) -> StagedChange {
        StagedChange {
            id: self.id.into(),
            device: self.device.clone(),
            title: self.title.clone(),
            status: ChangeStatus::Ready,
            edits: Vec::new(),
            base_backup: String::new(),
            editor: crate::session::Editor::Agent,
            note: String::new(),
            order: 0,
            history: Vec::new(),
            reverts: None,
        }
    }

    fn plan(&self) -> ApplyPlan {
        ApplyPlan {
            change: self.id.into(),
            device: self.identity.clone(),
            checks: self.checks.clone(),
            diff: self.diff.clone(),
            digest: crate::gear::blobs::hash(format!("{}|{}", self.id, self.device).as_bytes()),
            warnings: self.warnings.clone(),
            dfu_link: None,
        }
    }

    fn report(&self, message: String) -> ApplyReport {
        ApplyReport {
            change: self.id.into(),
            device: self.device.clone(),
            status: ChangeStatus::Verified,
            steps: vec![StepReport {
                name: self.title.clone(),
                state: StepState::Done,
                detail: None,
            }],
            backup: None,
            after_backup: None,
            sent: Vec::new(),
            failed_line: None,
            verify: Vec::new(),
            saved: true,
            files: Vec::new(),
            message,
            notes: Vec::new(),
            at: chrono::Utc::now(),
        }
    }
}

impl Core {
    /// Runs `f`; with an `ask`, only after the person's click in the apply sheet, and the
    /// outcome (`said` of the answer, or the error) goes back to that sheet.
    pub(super) fn after_ask<T>(
        &self,
        ask: Option<ActionAsk>,
        f: impl FnOnce() -> Result<T>,
        said: impl Fn(&T) -> String,
    ) -> Result<T> {
        let Some(a) = ask else {
            return f();
        };
        self.hooks.confirm_apply(&a.change(), &a.plan())?;
        let out = f();
        let shown = match &out {
            Ok(v) => Ok(a.report(said(v))),
            Err(e) => Err(anyhow!("{e:#}")),
        };
        self.hooks.apply_done(&shown);
        out
    }
}
