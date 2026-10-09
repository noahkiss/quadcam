//! `Core`'s EdgeTX card reads: the card view (models, the selected model, the radio
//! clock) and the preview of card edits (checks and diff, nothing written). Writing a
//! plan belongs to the apply engine (design 8.1); `gear_release_card` is the unmount that
//! comes before "safe to unplug".

use super::Core;
use crate::gear::edgetx::card::{AppleDoubleFile, Card, CardView, WriteOptions};
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

/// `gear_card_clean`: the `._` files macOS left on a card. Without `remove` it lists them.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct CardCleanParams {
    #[serde(default)]
    pub mount: Option<PathBuf>,
    #[serde(default)]
    pub device: Option<String>,
    /// Delete the listed files. Needs `confirm`.
    #[serde(default)]
    pub remove: bool,
    #[serde(default)]
    pub confirm: bool,
}

/// What `gear_card_clean` found and did.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct CardClean {
    pub root: PathBuf,
    /// The AppleDouble files on the card when the call began.
    pub files: Vec<AppleDoubleFile>,
    pub bytes: u64,
    pub removed: u32,
    /// The card is in the radio, over USB (slow).
    pub radio_usb: bool,
    pub notes: Vec<String>,
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

/// The card a request names, found mounted or mounted for it.
struct CardTarget {
    root: PathBuf,
    /// The detected card; none for a plain folder.
    c: Option<Connected>,
    /// This request mounted it.
    here: bool,
}

impl Core {
    /// The card a request names: `mount`, a device id, or the one radio plugged in. A card
    /// that is unmounted but still plugged in is mounted for the request; `here` says this
    /// call mounted it, so the caller releases it when done (`gear_target_done`).
    fn gear_card_target(
        &self,
        mount: Option<&PathBuf>,
        device: Option<&str>,
    ) -> Result<CardTarget> {
        let connected = self.gear_connected()?;
        let is_vol = |c: &&Connected| matches!(c.link, Link::Volume { .. });
        let mount_of = |c: &Connected| match &c.link {
            Link::Volume { mount, .. } => Some(mount.clone()),
            _ => None,
        };
        let mounted = |c: Connected| CardTarget {
            root: mount_of(&c).unwrap(),
            c: Some(c),
            here: false,
        };
        if let Some(m) = mount {
            let c = connected
                .iter()
                .find(|c| mount_of(c).as_ref() == Some(m))
                .cloned();
            return Ok(CardTarget {
                root: m.clone(),
                c,
                here: false,
            });
        }
        let unmounted = self.unmounted_cards(true);
        if let Some(id) = device.map(str::trim).filter(|s| !s.is_empty()) {
            if let Some(c) = connected
                .iter()
                .filter(is_vol)
                .find(|c| c.id.as_deref() == Some(id))
            {
                return Ok(mounted(c.clone()));
            }
            let Some(c) = unmounted.iter().find(|c| c.id.as_deref() == Some(id)) else {
                bail!("No card with id {id:?} is plugged in. Plug the radio in (USB Storage) or insert its card.");
            };
            return self.target_mounted(c);
        }
        let radios: Vec<&Connected> = connected
            .iter()
            .filter(is_vol)
            .filter(|c| c.kind == crate::gear::model::DeviceKind::Radio)
            .collect();
        match (radios.as_slice(), unmounted.as_slice()) {
            ([one], []) => Ok(mounted((*one).clone())),
            ([], [one]) => self.target_mounted(one),
            ([], []) => bail!(
                "No EdgeTX card is plugged in. Plug the radio in and pick USB Storage, or insert its card; or name a mount."
            ),
            _ => bail!("Several EdgeTX cards are plugged in; name one by mount or device."),
        }
    }

    fn target_mounted(&self, c: &Connected) -> Result<CardTarget> {
        let l = self.card_for_job(c, true)?;
        Ok(CardTarget {
            root: l.root,
            c: Some(l.connected),
            here: l.mounted_here,
        })
    }

    /// Mount, work, unmount: releases a card the request mounted, quietly (a read or a
    /// preview plays no cue). A card that was mounted already stays as it is.
    fn gear_target_done(&self, t: &CardTarget) {
        if let (true, Some(c)) = (t.here, &t.c) {
            self.card_quiet_unmount(&super::apply_card::Located {
                connected: c.clone(),
                root: t.root.clone(),
                mounted_here: true,
            });
        }
    }

    /// The aircraft profile a model file (or its name) belongs to.
    pub(super) fn aircraft_of_model(
        &self,
        file: Option<&str>,
        name: Option<&str>,
    ) -> Result<Option<String>> {
        let Some(file) = file else { return Ok(None) };
        let (profiles, _) = self.profiles()?;
        Ok(profiles
            .iter()
            .find(|pr| pr.gear.edgetx_model.as_deref() == Some(file))
            .or_else(|| {
                name.and_then(|n| {
                    profiles
                        .iter()
                        .find(|pr| pr.edgetx_models.iter().any(|m| m.eq_ignore_ascii_case(n)))
                })
            })
            .map(|pr| pr.name.clone()))
    }

    /// The card: identity, models, the selected model and its aircraft, the radio clock,
    /// and one model's full view when asked. Reads only.
    pub fn gear_card(&self, p: &CardParams) -> Result<GearCard> {
        let t = self.gear_card_target(p.mount.as_ref(), p.device.as_deref())?;
        let out = (|| {
            let card = Card::open(&t.root)?;
            let view = card.view(p.model.as_deref(), chrono::Local::now().date_naive())?;
            let selected_aircraft = self.aircraft_of_model(
                view.selected_model.as_deref(),
                view.selected_name.as_deref(),
            )?;
            Ok(GearCard {
                card: view,
                selected_aircraft,
                radio_usb: t.c.as_ref().is_some_and(|c| c.usb.is_some()),
            })
        })();
        self.gear_target_done(&t);
        out
    }

    /// Checks and diffs card edits. Writes nothing.
    pub fn gear_card_preview(&self, p: &CardPreviewParams) -> Result<CardPreview> {
        let t = self.gear_card_target(p.mount.as_ref(), p.device.as_deref())?;
        let out = (|| {
            let card = Card::open(&t.root)?;
            let plan = card.plan(&p.edits, None)?;
            let radio_usb = t.c.as_ref().is_some_and(|c| c.usb.is_some());
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
        })();
        self.gear_target_done(&t);
        out
    }

    /// Lists the AppleDouble (`._*`) files macOS left on a card, and with `remove` and
    /// `confirm` deletes them. Only files that start with the AppleDouble magic count.
    /// After a removal on a connected card the card unmounts, as after every card job.
    pub fn gear_card_clean(&self, p: &CardCleanParams) -> Result<CardClean> {
        if p.remove && !p.confirm {
            bail!("Refused: removing files from a card needs confirm=true. Call without remove to list them first.");
        }
        let t = self.gear_card_target(p.mount.as_ref(), p.device.as_deref())?;
        let mut released = false;
        let out = self.card_clean_on(p, &t, &mut released);
        if !released {
            self.gear_target_done(&t);
        }
        out
    }

    /// The body of `gear_card_clean`. `released` turns true once the job's own finish
    /// unmounted the card.
    fn card_clean_on(
        &self,
        p: &CardCleanParams,
        t: &CardTarget,
        released: &mut bool,
    ) -> Result<CardClean> {
        let (root, c) = (t.root.clone(), t.c.clone());
        let radio_usb = c.as_ref().is_some_and(|c| c.usb.is_some());
        let files = crate::gear::edgetx::card::find_apple_double(&root);
        let bytes = files.iter().map(|f| f.bytes).sum();
        let mut out = CardClean {
            root: root.clone(),
            bytes,
            removed: 0,
            radio_usb,
            notes: Vec::new(),
            files,
        };
        if !p.remove {
            return Ok(out);
        }
        if !crate::gear::edgetx::card::writes_allowed(
            &root,
            std::env::var("QUADCAM_CARD_WRITE").ok().as_deref(),
            std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
        ) {
            bail!("Refused: this process writes no real card under /Volumes (set QUADCAM_CARD_WRITE=real).");
        }
        if out.files.is_empty() {
            out.notes.push("The card holds no ._ files.".into());
            return Ok(out);
        }
        let _hold = c
            .as_ref()
            .map(|c| self.gear_hold(&super::link_handle(&c.link)));
        let timeout = std::time::Duration::from_secs(
            if radio_usb { 30 } else { 10 } + out.files.len() as u64,
        );
        let removed = crate::gear::edgetx::card::remove_apple_doubles(&root, &out.files, timeout);
        let failed = removed.as_ref().err().map(|e| format!("{e:#}"));
        out.removed = removed.as_ref().copied().unwrap_or(0) as u32;
        if let Some(c) = &c {
            *released = true;
            if let Err(why) = self.gear_finish_card(c, failed.as_deref().map(|m| ("Clean card", m)))
            {
                out.notes.push(format!("The card did not unmount: {why}"));
            }
        }
        match removed {
            Ok(_) => Ok(out),
            Err(e) => Err(e),
        }
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
