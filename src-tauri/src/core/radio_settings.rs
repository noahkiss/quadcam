//! `Core::gear_radio_settings` and `Core::gear_radio_settings_edit`: the radio's Settings
//! editor (`RADIO/radio.yml`, `Edit::Radio`). The settings are read from the radio's mounted
//! card, else from its latest backup, with the device's staged radio edits on top. An edit
//! joins the device's open "Radio settings" change, as the model editors do: a later edit of
//! a setting replaces the earlier one, and an edit the radio already has drops out. Only
//! QuadCam's own data is written; the apply sheet writes the card.

use super::model_edit::{Base, ModelEntry};
use super::Core;
use crate::gear::changes::ChangeFilter;
use crate::gear::edgetx::card::{
    apply_radio_op, identity_from_radio_yml, is_model_file, RadioOp, RADIO_FILE,
};
use crate::gear::edgetx::model as em;
use crate::gear::edgetx::radio::{self, RadioSettingsView, SettingKind};
use crate::gear::edgetx::yaml::Doc;
use crate::gear::model::{ChangeStatus, Edit, StagedChange};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// The title of the change the Settings editor keeps its edits in.
pub const RADIO_CHANGE: &str = "Radio settings";

/// `gear_radio_settings`: a radio.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct RadioSettingsParams {
    /// A saved radio's device id.
    pub device: String,
    /// Show the settings as they will be with the device's staged radio edits applied. On
    /// by default for the editor.
    #[serde(default = "yes")]
    pub staged: bool,
}

fn yes() -> bool {
    true
}

/// The radio's settings, for the editor.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RadioSettingsDetail {
    pub device: String,
    /// `card` (mounted now) or `backup <id> (<date>)`.
    pub source: String,
    /// `board` and `semver` from `radio.yml`.
    #[serde(default)]
    pub board: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    pub view: RadioSettingsView,
    /// The model files, for the selected model.
    pub models: Vec<ModelEntry>,
    /// Staged radio edits shown on top.
    pub staged: usize,
    pub notes: Vec<String>,
}

/// `gear_radio_settings_edit`: radio edits, staged for a radio.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct RadioSettingsEditParams {
    pub device: String,
    /// `set_scalar {key, value}` (a value the file has) or `select_model {file}`.
    #[serde(default)]
    pub ops: Vec<RadioOp>,
    /// Who stages it. The app and the CLI leave it out (the person); MCP says `agent`.
    #[serde(default)]
    pub editor: Option<crate::session::Editor>,
}

/// The setting an op sets: a later op with the same key replaces an earlier one.
fn op_key(op: &RadioOp) -> &str {
    match op {
        RadioOp::SetScalar { key, .. } => key,
        RadioOp::SelectModel { .. } => "currModel",
    }
}

/// Puts `fresh` after `kept`; an op replaces the earlier op of its setting.
pub fn merge_radio_ops(kept: &[RadioOp], fresh: &[RadioOp]) -> Vec<RadioOp> {
    let mut out: Vec<RadioOp> = kept.to_vec();
    for op in fresh {
        out.retain(|o| op_key(o) != op_key(op));
        out.push(op.clone());
    }
    out
}

fn has_model(base: &Base<'_>, f: &str) -> bool {
    base.reader.read(&format!("MODELS/{f}")).is_some()
}

impl Core {
    fn radio_doc(&self, base: &Base<'_>) -> Result<Doc> {
        let bytes = base
            .reader
            .read(RADIO_FILE)
            .with_context(|| format!("The {} has no {RADIO_FILE}.", base.source))?;
        Doc::parse("radio.yml", &bytes).map_err(|r| anyhow::anyhow!(r.reason))
    }

    /// The radio ops of the device's open changes, in order.
    fn staged_radio_ops(&self, device: &str) -> Vec<RadioOp> {
        self.changes()
            .list(&ChangeFilter {
                device: Some(device.to_string()),
                ..Default::default()
            })
            .into_iter()
            .filter(|c| matches!(c.status, ChangeStatus::Draft | ChangeStatus::Ready))
            .flat_map(|c| c.edits)
            .flat_map(|e| match e {
                Edit::Radio { ops } => ops,
                _ => vec![],
            })
            .collect()
    }

    /// Reads the radio's settings for the editor. Reads only.
    pub fn gear_radio_settings(&self, p: &RadioSettingsParams) -> Result<RadioSettingsDetail> {
        let snaps = self.snapshots();
        let base = self.model_base(&p.device, &snaps)?;
        let mut doc = self.radio_doc(&base)?;
        let id = identity_from_radio_yml(doc.original());
        let mut staged = 0;
        let mut notes = Vec::new();
        if p.staged {
            for op in self.staged_radio_ops(&p.device) {
                match apply_radio_op(&mut doc, &op, &|f| has_model(&base, f), &mut vec![]) {
                    Ok(()) => staged += 1,
                    Err(r) => notes.push(format!("A staged edit does not apply: {}", r.reason)),
                }
            }
            if staged > 0 {
                notes.push(format!(
                    "Shows {staged} staged {}: not on the radio until applied.",
                    if staged == 1 { "edit" } else { "edits" }
                ));
            }
        }
        let view = radio::view(&doc).map_err(|r| anyhow::anyhow!(r.reason))?;
        let mut models = Vec::new();
        for f in base.reader.files() {
            let Some(m) = f.strip_prefix("MODELS/").filter(|m| is_model_file(m)) else {
                continue;
            };
            let name = base
                .reader
                .read(&f)
                .and_then(|b| Doc::parse(m, &b).ok())
                .and_then(|d| em::model_name(&d).ok().flatten())
                .unwrap_or_default();
            models.push(ModelEntry {
                file: m.to_string(),
                name,
                selected: view.selected_model.as_deref() == Some(m),
            });
        }
        models.sort_by(|a, b| a.file.cmp(&b.file));
        Ok(RadioSettingsDetail {
            device: p.device.clone(),
            source: base.source,
            board: id.board,
            version: id.version,
            view,
            models,
            staged,
            notes,
        })
    }

    /// Stages radio edits. They join the device's open "Radio settings" change (made if
    /// none): a later edit of a setting replaces its earlier one, an edit the radio already
    /// has drops out, and a change left with nothing is discarded.
    pub fn gear_radio_settings_edit(&self, p: &RadioSettingsEditParams) -> Result<StagedChange> {
        if p.ops.is_empty() {
            bail!("Pass at least one radio op.");
        }
        let snaps = self.snapshots();
        let base = self.model_base(&p.device, &snaps)?;
        let base_doc = self.radio_doc(&base)?;
        let open = self
            .changes()
            .list(&ChangeFilter {
                device: Some(p.device.clone()),
                ..Default::default()
            })
            .into_iter()
            .find(|c| {
                c.title == RADIO_CHANGE
                    && matches!(c.status, ChangeStatus::Draft | ChangeStatus::Ready)
                    && c.edits.iter().all(|e| matches!(e, Edit::Radio { .. }))
            });
        let kept: Vec<RadioOp> = open
            .iter()
            .flat_map(|c| c.edits.iter())
            .flat_map(|e| match e {
                Edit::Radio { ops } => ops.clone(),
                _ => vec![],
            })
            .collect();
        let merged = merge_radio_ops(&kept, &p.ops);
        // Every op is checked on the radio's file; one the radio already holds drops out.
        let models = |f: &str| has_model(&base, f);
        let mut probe = base_doc.clone();
        for op in &merged {
            apply_radio_op(&mut probe, op, &models, &mut vec![])
                .map_err(|r| anyhow::anyhow!(r.reason))?;
        }
        let ops: Vec<RadioOp> = merged
            .into_iter()
            .filter(|op| {
                let mut one = base_doc.clone();
                apply_radio_op(&mut one, op, &models, &mut vec![]).is_ok()
                    && one.render() != base_doc.render()
            })
            .collect();
        let edits: Vec<Edit> = if ops.is_empty() {
            vec![]
        } else {
            vec![Edit::Radio { ops }]
        };
        match (open, edits.is_empty()) {
            (Some(c), true) => self.gear_change_discard(&c.id),
            (None, true) => bail!("Nothing changes: the radio already has that."),
            (Some(c), false) => self.gear_change_update(&super::ChangeUpdateParams {
                id: c.id,
                edits: Some(edits),
                ..Default::default()
            }),
            (None, false) => self.gear_change_stage(&super::StageParams {
                device: p.device.clone(),
                title: Some(RADIO_CHANGE.into()),
                edits,
                editor: p.editor,
                ..Default::default()
            }),
        }
    }
}

/// A setting's value as the radio shows it.
pub fn shown(kind: &SettingKind, value: &str) -> String {
    match kind {
        SettingKind::Choice { choices } => choices
            .iter()
            .find(|c| c.value == value)
            .map(|c| c.label.clone())
            .unwrap_or_else(|| value.to_string()),
        SettingKind::Number {
            unit,
            mul,
            decimals,
            invert,
            ..
        } => match value.trim().parse::<i32>() {
            Ok(n) => {
                let n = invert.map_or(n, |i| i - n);
                let v = f64::from(n * mul) / 10f64.powi(i32::from(*decimals));
                let v = format!("{v:.*}", usize::from(*decimals));
                if unit.is_empty() {
                    v
                } else {
                    format!("{v} {unit}")
                }
            }
            Err(_) => value.to_string(),
        },
        SettingKind::Toggle { invert } => {
            let on = (value.trim() == "1") != *invert;
            if on { "on" } else { "off" }.to_string()
        }
        SettingKind::Text { .. } => format!("{value:?}"),
    }
}

/// The settings as text, for the CLI and MCP.
pub fn render_text(d: &RadioSettingsDetail) -> String {
    use std::fmt::Write;
    let mut s = format!(
        "Radio settings ({} {}) from the {}\n",
        d.board.as_deref().unwrap_or("radio"),
        d.version.as_deref().unwrap_or(""),
        d.source
    );
    for n in &d.notes {
        let _ = writeln!(s, "  {n}");
    }
    for section in &d.view.sections {
        let _ = writeln!(s, "{section}:");
        for x in d.view.settings.iter().filter(|x| &x.section == section) {
            let v = match &x.value {
                Some(v) => format!("{} ({}: {v})", shown(&x.kind, v), x.key),
                None => format!("not on this card ({})", x.key),
            };
            let _ = writeln!(s, "  {}: {v}", x.label);
        }
    }
    let sel = d.models.iter().find(|m| m.selected);
    let _ = writeln!(
        s,
        "Selected model: {}",
        match sel {
            Some(m) => format!("{} ({})", m.name, m.file),
            None => "none".into(),
        }
    );
    s
}
