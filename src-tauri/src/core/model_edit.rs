//! `Core::gear_model` and `Core::gear_model_edit`: the radio's model editors (design 6.3,
//! WP9). A model is read from the radio's mounted card, else from its latest backup, with
//! the device's staged model edits on top. An edit joins the device's open "Model edits"
//! change, the way the OSD editor does: a later edit of a setting replaces the earlier one.
//! Only QuadCam's own data is written; the apply sheet writes the card.

use super::switchmap::{CardSource, Folder, InBackup};
use super::Core;
use crate::gear::changes::ChangeFilter;
use crate::gear::edgetx::card::{
    checklist_width, identity_from_radio_yml, is_model_file, selected_in,
};
use crate::gear::edgetx::editors::{self, EditorView};
use crate::gear::edgetx::model::{self as em, ModelOp, ModelView};
use crate::gear::edgetx::yaml::{latin1, Doc};
use crate::gear::model::{ChangeStatus, Edit, Link, StagedChange};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// The title of the change the model editors keep their edits in.
pub const MODEL_CHANGE: &str = "Model edits";

/// `gear_model`: a radio, and the model to read.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct ModelParams {
    /// A saved radio's device id.
    pub device: String,
    /// A model file (`model01.yml`); the radio's selected model when left out.
    #[serde(default)]
    pub model: Option<String>,
    /// Show the model as it will be with the device's staged model edits applied. On by
    /// default for the editors.
    #[serde(default = "yes")]
    pub staged: bool,
}

fn yes() -> bool {
    true
}

/// One model file on the card.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ModelEntry {
    pub file: String,
    pub name: String,
    pub selected: bool,
}

/// A model in full, for the editors.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ModelDetail {
    pub device: String,
    /// `card` (mounted now) or `backup <id> (<date>)`.
    pub source: String,
    pub models: Vec<ModelEntry>,
    pub view: ModelView,
    pub editors: EditorView,
    /// `MODELS/<name>.txt`, when the model has one.
    #[serde(default)]
    pub checklist: Option<String>,
    /// Characters per checklist line on this radio, when measured.
    #[serde(default)]
    pub checklist_width: Option<usize>,
    /// Sound files on the card a callout can name (the file stems, 8 characters at most).
    pub tracks: Vec<String>,
    /// Staged model edits shown on top.
    pub staged: usize,
    pub notes: Vec<String>,
}

/// `gear_model_edit`: edits of one model, staged for a radio.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct ModelEditParams {
    pub device: String,
    /// The model file (`model01.yml`).
    pub model: String,
    #[serde(default)]
    pub ops: Vec<ModelOp>,
    /// The power-on checklist text, one item per line (`=` starts a tick box). Turns the
    /// checklist on when the model has it off. Empty text removes the file's lines.
    #[serde(default)]
    pub checklist: Option<String>,
    /// Who stages it. The app and the CLI leave it out (the person); MCP says `agent`.
    #[serde(default)]
    pub editor: Option<crate::session::Editor>,
}

/// Where the model files are read from.
struct Base<'a> {
    reader: Box<dyn CardSource + 'a>,
    source: String,
}

impl Core {
    fn model_base<'a>(
        &self,
        device: &str,
        snaps: &'a crate::gear::backup::Snapshots,
    ) -> Result<Base<'a>> {
        let known = self
            .gear_store()
            .devices()?
            .into_iter()
            .find(|d| d.id == device);
        match known {
            None => bail!("No device {device:?} in QuadCam's list (quadcam-cli gear devices)."),
            Some(d) if d.kind != crate::gear::model::DeviceKind::Radio => {
                bail!("{device:?} is not a radio.")
            }
            _ => {}
        }
        let mounted = self
            .gear_connected()?
            .into_iter()
            .find_map(|c| match &c.link {
                Link::Volume { mount, .. } if c.id.as_deref() == Some(device) => {
                    Some(mount.clone())
                }
                _ => None,
            });
        if let Some(m) = mounted {
            return Ok(Base {
                reader: Box::new(Folder(m)),
                source: "card".into(),
            });
        }
        let b = snaps.latest(device).with_context(|| {
            format!("No backup of {device:?} yet: back it up, or plug the radio in (USB Storage).")
        })?;
        if !b.files.iter().any(|f| f.path.starts_with("MODELS/")) {
            bail!("The latest backup of {device:?} holds no models: back the card up again.");
        }
        let source = format!("backup {} ({})", b.id, b.taken_at.format("%Y-%m-%d %H:%M"));
        Ok(Base {
            reader: Box::new(InBackup { snaps, backup: b }),
            source,
        })
    }

    fn model_file(&self, base: &Base<'_>, model: Option<&str>) -> Result<String> {
        if let Some(m) = model.map(|m| m.trim().trim_start_matches("MODELS/")) {
            if !is_model_file(m) {
                bail!("{m:?} is not a model file name (model01.yml).");
            }
            return Ok(m.to_string());
        }
        let selected = base
            .reader
            .read("RADIO/radio.yml")
            .and_then(|b| Doc::parse("radio.yml", &b).ok())
            .and_then(|d| selected_in(&d));
        selected.context("The radio selects no model; name one (model01.yml).")
    }

    /// The device's staged model edits (model ops and checklist text), in order.
    fn staged_model_edits(&self, device: &str) -> Vec<Edit> {
        self.changes()
            .list(&ChangeFilter {
                device: Some(device.to_string()),
                ..Default::default()
            })
            .into_iter()
            .filter(|c| matches!(c.status, ChangeStatus::Draft | ChangeStatus::Ready))
            .flat_map(|c| c.edits)
            .filter(|e| matches!(e, Edit::Model { .. } | Edit::Checklist { .. }))
            .collect()
    }

    /// Reads a model for the editors. Reads only.
    pub fn gear_model(&self, p: &ModelParams) -> Result<ModelDetail> {
        let snaps = self.snapshots();
        let base = self.model_base(&p.device, &snaps)?;
        let file = self.model_file(&base, p.model.as_deref())?;
        let bytes = base
            .reader
            .read(&format!("MODELS/{file}"))
            .with_context(|| format!("The {} has no MODELS/{file}.", base.source))?;
        let mut doc = Doc::parse(&file, &bytes).map_err(|r| anyhow::anyhow!(r.reason))?;
        let name = em::model_name(&doc)
            .map_err(|r| anyhow::anyhow!(r.reason))?
            .unwrap_or_default();
        let board = base
            .reader
            .read("RADIO/radio.yml")
            .map(|b| identity_from_radio_yml(&b))
            .and_then(|i| i.board);
        let mut checklist = self.read_checklist(&base, &name);
        let mut staged = 0;
        let mut notes = Vec::new();
        if p.staged {
            for e in self.staged_model_edits(&p.device) {
                match e {
                    Edit::Model { file: f, ops, .. } if f == file => {
                        match em::apply(&mut doc, &ops) {
                            Ok(()) => staged += ops.len(),
                            Err(r) => {
                                notes.push(format!("A staged edit does not apply: {}", r.reason))
                            }
                        }
                    }
                    Edit::Checklist { model, text } if model == file => {
                        checklist = Some(text);
                        staged += 1;
                    }
                    _ => {}
                }
            }
            if staged > 0 {
                notes.push(format!(
                    "Shows {staged} staged {}: not on the radio until applied.",
                    if staged == 1 { "edit" } else { "edits" }
                ));
            }
        }
        let view = em::view(&file, &doc).map_err(|r| anyhow::anyhow!(r.reason))?;
        let editors = editors::editor_view(&doc).map_err(|r| anyhow::anyhow!(r.reason))?;
        let selected = base
            .reader
            .read("RADIO/radio.yml")
            .and_then(|b| Doc::parse("radio.yml", &b).ok())
            .and_then(|d| selected_in(&d));
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
                selected: selected.as_deref() == Some(m),
            });
        }
        models.sort_by(|a, b| a.file.cmp(&b.file));
        let mut tracks: Vec<String> = base
            .reader
            .files()
            .iter()
            .filter_map(|f| {
                let rest = f.strip_prefix("SOUNDS/")?;
                let (_lang, tail) = rest.split_once('/')?;
                if tail.contains('/') {
                    return None;
                }
                let stem = tail
                    .strip_suffix(".wav")
                    .or_else(|| tail.strip_suffix(".WAV"))?;
                (!stem.is_empty() && stem.len() <= em::MAX_TRACK_NAME).then(|| stem.to_string())
            })
            .collect();
        tracks.sort();
        tracks.dedup();
        Ok(ModelDetail {
            device: p.device.clone(),
            source: base.source,
            models,
            view,
            editors,
            checklist,
            checklist_width: checklist_width(board.as_deref()),
            tracks,
            staged,
            notes,
        })
    }

    /// `MODELS/<name>.txt` with spaces kept, else with `_`, as EdgeTX looks for it.
    fn read_checklist(&self, base: &Base<'_>, name: &str) -> Option<String> {
        if name.is_empty() {
            return None;
        }
        [name.to_string(), name.replace(' ', "_")]
            .iter()
            .find_map(|n| base.reader.read(&format!("MODELS/{n}.txt")))
            .map(|b| latin1(&b))
    }

    /// Stages edits of one model. The edits join the device's open "Model edits" change
    /// (made if none): a later edit of a setting replaces its earlier one, and a change
    /// that leaves the model as the radio has it is dropped.
    pub fn gear_model_edit(&self, p: &ModelEditParams) -> Result<StagedChange> {
        if p.ops.is_empty() && p.checklist.is_none() {
            bail!("Pass at least one model op, or a checklist.");
        }
        let snaps = self.snapshots();
        let base = self.model_base(&p.device, &snaps)?;
        let file = self.model_file(&base, Some(&p.model))?;
        let bytes = base
            .reader
            .read(&format!("MODELS/{file}"))
            .with_context(|| format!("The {} has no MODELS/{file}.", base.source))?;
        let base_doc = Doc::parse(&file, &bytes).map_err(|r| anyhow::anyhow!(r.reason))?;
        let name = em::model_name(&base_doc)
            .map_err(|r| anyhow::anyhow!(r.reason))?
            .unwrap_or_default();
        let board = base
            .reader
            .read("RADIO/radio.yml")
            .map(|b| identity_from_radio_yml(&b))
            .and_then(|i| i.board);
        let base_checklist = self.read_checklist(&base, &name);
        let open = self
            .changes()
            .list(&ChangeFilter {
                device: Some(p.device.clone()),
                ..Default::default()
            })
            .into_iter()
            .find(|c| {
                c.title == MODEL_CHANGE
                    && matches!(c.status, ChangeStatus::Draft | ChangeStatus::Ready)
                    && c.edits
                        .iter()
                        .all(|e| matches!(e, Edit::Model { .. } | Edit::Checklist { .. }))
            });
        let earlier = |f: &str| -> (Vec<ModelOp>, Option<String>) {
            let mut ops = Vec::new();
            let mut text = None;
            for e in open.iter().flat_map(|c| c.edits.iter()) {
                match e {
                    Edit::Model {
                        file: x, ops: o, ..
                    } if x == f => ops.extend(o.clone()),
                    Edit::Checklist { model, text: t } if model == f => text = Some(t.clone()),
                    _ => {}
                }
            }
            (ops, text)
        };
        let (kept, kept_text) = earlier(&file);
        let mut fresh = p.ops.clone();
        let text = p.checklist.clone().or(kept_text);
        if let Some(t) = &p.checklist {
            editors::check_checklist(t, checklist_width(board.as_deref()))
                .map_err(|r| anyhow::anyhow!(r.reason))?;
            if !t.trim().is_empty()
                && !fresh
                    .iter()
                    .any(|o| matches!(o, ModelOp::SetChecklist { .. }))
            {
                fresh.push(ModelOp::SetChecklist { enabled: true });
            }
        }
        let merged = editors::merge_ops(&kept, &fresh);
        // Does the result differ from what the radio has? An edit the radio already holds
        // drops out, and a checklist that equals its file drops out.
        let mut probe = base_doc.clone();
        em::apply(&mut probe, &merged).map_err(|r| anyhow::anyhow!(r.reason))?;
        let model_changes = probe.render() != base_doc.render();
        let text_changes = text
            .as_ref()
            .is_some_and(|t| normalise(t) != normalise(base_checklist.as_deref().unwrap_or("")));
        let mut edits: Vec<Edit> = Vec::new();
        for e in open.iter().flat_map(|c| c.edits.iter()) {
            let other = match e {
                Edit::Model { file: x, .. } => x != &file,
                Edit::Checklist { model, .. } => model != &file,
                _ => true,
            };
            if other {
                edits.push(e.clone());
            }
        }
        if model_changes {
            edits.push(Edit::Model {
                file: file.clone(),
                name: Some(name.clone()).filter(|n| !n.is_empty()),
                ops: merged,
            });
        }
        if text_changes {
            if let Some(t) = text {
                edits.push(Edit::Checklist {
                    model: file.clone(),
                    text: t,
                });
            }
        }
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
                title: Some(MODEL_CHANGE.into()),
                edits,
                editor: p.editor,
                ..Default::default()
            }),
        }
    }
}

/// Line endings and trailing blanks do not make a checklist differ.
fn normalise(t: &str) -> String {
    t.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}
