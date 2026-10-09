//! `Core`'s EdgeTX card reads: the card view (models, the selected model, the radio
//! clock) and the preview of card edits (checks and diff, nothing written). Writing a
//! plan belongs to the apply engine (design 8.1); `gear_release_card` is the unmount that
//! comes before "safe to unplug".

use super::Core;
use crate::gear::edgetx::card::{Card, CardView, WriteOptions};
use crate::gear::model::{Check, Connected, DiffItem, Edit, Identity, Link};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// `gear_card`: which card, and optionally one model's full view.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct CardParams {
    /// The card's mount point (or any folder holding a card's files).
    #[serde(default)]
    pub mount: Option<PathBuf>,
    /// A connected radio's device id, instead of `mount`.
    #[serde(default)]
    pub device: Option<String>,
    /// A model file (`model01.yml`) to read in full.
    #[serde(default)]
    pub model: Option<String>,
}

/// `gear_card_preview`: card edits to check and diff. Nothing is written.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct CardPreviewParams {
    #[serde(default)]
    pub mount: Option<PathBuf>,
    #[serde(default)]
    pub device: Option<String>,
    pub edits: Vec<Edit>,
}

/// What a card edit would do: every check, the diff, and the time it would take.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct CardPreview {
    pub identity: Identity,
    pub ready: bool,
    pub checks: Vec<Check>,
    pub diff: Vec<DiffItem>,
    pub warnings: Vec<String>,
    /// Files the edits would write or delete.
    pub files: Vec<String>,
    pub bytes: u64,
    /// Seconds at the link's usual speed (a radio over USB writes about 0.3 MB/s).
    pub eta_s: u64,
    /// The card is in the radio, over USB (slow), not in a reader.
    pub radio_usb: bool,
}

/// `gear_card`'s answer: the card, and the aircraft whose EdgeTX model the radio selects.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct GearCard {
    pub card: CardView,
    /// The aircraft profile that names the selected model (its model file, or its EdgeTX
    /// model names). Compare with the quad last connected to warn "Radio is set to X".
    #[serde(default)]
    pub selected_aircraft: Option<String>,
    pub radio_usb: bool,
}

impl Core {
    /// The card a request names: `mount`, a device id, or the one radio plugged in.
    fn gear_card_target(
        &self,
        mount: Option<&PathBuf>,
        device: Option<&str>,
    ) -> Result<(PathBuf, Option<Connected>)> {
        let connected = self.gear_connected()?;
        let is_vol = |c: &&Connected| matches!(c.link, Link::Volume { .. });
        let mount_of = |c: &Connected| match &c.link {
            Link::Volume { mount, .. } => Some(mount.clone()),
            _ => None,
        };
        if let Some(m) = mount {
            let c = connected
                .iter()
                .find(|c| mount_of(c).as_ref() == Some(m))
                .cloned();
            return Ok((m.clone(), c));
        }
        if let Some(id) = device.map(str::trim).filter(|s| !s.is_empty()) {
            let Some(c) = connected
                .iter()
                .filter(is_vol)
                .find(|c| c.id.as_deref() == Some(id))
            else {
                bail!("No card with id {id:?} is mounted. Plug the radio in (USB Storage) or insert its card.");
            };
            return Ok((mount_of(c).unwrap(), Some(c.clone())));
        }
        let radios: Vec<&Connected> = connected
            .iter()
            .filter(is_vol)
            .filter(|c| c.kind == crate::gear::model::DeviceKind::Radio)
            .collect();
        match radios.as_slice() {
            [one] => Ok((mount_of(one).unwrap(), Some((*one).clone()))),
            [] => bail!(
                "No EdgeTX card is mounted. Plug the radio in and pick USB Storage, or insert its card; or name a mount."
            ),
            _ => bail!("Several EdgeTX cards are mounted; name one by mount or device."),
        }
    }

    /// The card: identity, models, the selected model and its aircraft, the radio clock,
    /// and one model's full view when asked. Reads only.
    pub fn gear_card(&self, p: &CardParams) -> Result<GearCard> {
        let (root, c) = self.gear_card_target(p.mount.as_ref(), p.device.as_deref())?;
        let card = Card::open(&root)?;
        let view = card.view(p.model.as_deref(), chrono::Local::now().date_naive())?;
        let selected_aircraft = match (&view.selected_model, &view.selected_name) {
            (Some(file), name) => {
                let (profiles, _) = self.profiles()?;
                profiles
                    .iter()
                    .find(|pr| pr.gear.edgetx_model.as_deref() == Some(file.as_str()))
                    .or_else(|| {
                        name.as_deref().and_then(|n| {
                            profiles.iter().find(|pr| {
                                pr.edgetx_models.iter().any(|m| m.eq_ignore_ascii_case(n))
                            })
                        })
                    })
                    .map(|pr| pr.name.clone())
            }
            _ => None,
        };
        Ok(GearCard {
            card: view,
            selected_aircraft,
            radio_usb: c.is_some_and(|c| c.usb.is_some()),
        })
    }

    /// Checks and diffs card edits. Writes nothing.
    pub fn gear_card_preview(&self, p: &CardPreviewParams) -> Result<CardPreview> {
        let (root, c) = self.gear_card_target(p.mount.as_ref(), p.device.as_deref())?;
        let card = Card::open(&root)?;
        let plan = card.plan(&p.edits, None)?;
        let radio_usb = c.as_ref().is_some_and(|c| c.usb.is_some());
        let opts = if radio_usb {
            WriteOptions::radio_usb()
        } else {
            WriteOptions::reader()
        };
        Ok(CardPreview {
            identity: plan.identity.clone(),
            ready: plan.ready(),
            files: plan.files.iter().map(|f| f.path.clone()).collect(),
            bytes: plan.bytes(),
            eta_s: plan.bytes() / opts.bytes_per_s.max(1),
            checks: plan.checks,
            diff: plan.diff,
            warnings: plan.warnings,
            radio_usb,
        })
    }

    /// Unmounts a card after a job, then plays "<device> done, safe to unplug." Never the
    /// cue first: when the unmount fails or times out, the "failed" cue plays instead and
    /// the error says whether a reboot may be needed.
    pub fn gear_release_card(&self, c: &Connected) -> Result<()> {
        let Link::Volume {
            whole_disk: Some(disk),
            ..
        } = &c.link
        else {
            let e = anyhow::anyhow!("This device has no disk to unmount.");
            self.gear_note_failure(c, "Unmount", &e.to_string());
            self.gear_job_done(c, Some("Unmount"));
            return Err(e);
        };
        let r = {
            let _hold = self.gear_hold(&super::link_handle(&c.link));
            (self.gear.unmount)(disk)
        };
        match r {
            Ok(()) => {
                self.card_note_released(c);
                self.gear_clear_failure(&super::link_handle(&c.link));
                self.gear_job_done(c, None);
                Ok(())
            }
            Err(e) => {
                self.gear_note_failure(c, "Unmount", &format!("{e:#}"));
                self.gear_job_done(c, Some("Unmount"));
                Err(e)
            }
        }
    }
}
