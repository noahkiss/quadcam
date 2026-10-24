//! `Core`'s EdgeTX card reads: the card view (models, the selected model, the radio
//! clock) and the preview of card edits (checks and diff, nothing written). Writing a
//! plan belongs to the apply engine (design 8.1); `gear_release_card` is the unmount that
//! comes before "safe to unplug".

use super::Core;
use crate::gear::edgetx::card::{AppleDoubleFile, Card, CardView, WriteOptions};
use crate::gear::model::{Check, Connected, DiffItem, Edit, Identity, Link};
use anyhow::{bail, Context, Result};
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
    /// Seconds at the link's measured speed, plus each file's own cost (a Pocket over USB
    /// writes about 0.3 MB/s, a GX12 about 0.36 MB/s; a card in a reader is fast).
    pub eta_s: u64,
    /// The card is in the radio, over USB (slow), not in a reader. Always true for a
    /// radio that keeps its files in its own flash (a GX12).
    pub radio_usb: bool,
}

/// `gear_model_import`: a model from another radio's card, a model file or a radio backup,
/// as a new model on this card. Nothing is written: the answer holds the edit to stage.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct ModelImportParams {
    /// The card that gets the model (its mount); default the one card plugged in.
    #[serde(default)]
    pub mount: Option<PathBuf>,
    /// A connected radio's device id, instead of `mount`.
    #[serde(default)]
    pub device: Option<String>,
    /// Where the model comes from: another card's mount or a copy of its folder, one model
    /// file, or a radio backup id (`<device>/<name>`).
    pub from: String,
    /// The model file in `from` (`model01.yml`); default its selected model.
    #[serde(default)]
    pub model: Option<String>,
    /// The source radio's board, when `from` is a model file with no `radio.yml` beside it.
    #[serde(default)]
    pub from_board: Option<String>,
    /// The new model file (`model03.yml`); default the first free one.
    #[serde(default)]
    pub to: Option<String>,
    /// The new model's name; default the source's.
    #[serde(default)]
    pub name: Option<String>,
    /// Links in place of the two boards' default links, per source control.
    #[serde(default)]
    pub links: Vec<crate::gear::edgetx::board::ControlLink>,
    /// The model on this card whose board-only setup (a GX12's custom switches) the new
    /// model takes; default the selected model.
    #[serde(default)]
    pub template: Option<String>,
}

/// What importing a model would do: the edit to stage, both radios' controls, the links
/// used and each control the model reads, and the card preview.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ModelImport {
    /// Stage it on the radio (`gear_change_stage`) to write it.
    pub edit: Edit,
    pub from_board: String,
    #[serde(default)]
    pub to_board: Option<String>,
    pub from_controls: Vec<crate::gear::edgetx::board::BoardControl>,
    pub to_controls: Vec<crate::gear::edgetx::board::BoardControl>,
    /// The links in effect: the boards' defaults with the ones given on top.
    pub links: Vec<crate::gear::edgetx::board::ControlLink>,
    pub translation: crate::gear::edgetx::board::Translation,
    pub preview: CardPreview,
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

/// The largest model file `gear_model_import` reads (EdgeTX models are tens of KB).
const MODEL_FILE_MAX: u64 = 1024 * 1024;

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

    /// The aircraft profile a model file (or its name) belongs to. With a `radio`, the
    /// aircraft on that radio (their profiles name it) come first, and a model file never
    /// matches a profile that names another radio: a file name is a slot on one card.
    pub(super) fn aircraft_of_model(
        &self,
        file: Option<&str>,
        name: Option<&str>,
        radio: Option<&str>,
    ) -> Result<Option<String>> {
        let Some(file) = file else { return Ok(None) };
        let (mut profiles, _) = self.profiles()?;
        // A stable sort: the radio's own aircraft first, each group in profile order.
        profiles.sort_by_key(|pr| radio.is_none() || pr.gear.radio.as_deref() != radio);
        Ok(profiles
            .iter()
            .find(|pr| {
                pr.gear.edgetx_model.as_deref() == Some(file)
                    && (radio.is_none()
                        || pr.gear.radio.is_none()
                        || pr.gear.radio.as_deref() == radio)
            })
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
                t.c.as_ref().and_then(|c| c.id.as_deref()),
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
            card_preview(&card, &t, &p.edits)
        })();
        self.gear_target_done(&t);
        out
    }

    /// A model from another radio as a new model on this card: reads the source, maps its
    /// controls and previews the edit. Writes nothing.
    pub fn gear_model_import(&self, p: &ModelImportParams) -> Result<ModelImport> {
        use super::switchmap::{CardSource, Folder, InBackup};
        use crate::gear::edgetx::{
            board, card as ec, model as em,
            yaml::{latin1, Doc},
        };
        let from = p.from.trim();
        if from.is_empty() {
            bail!(
                "from is required: a card's mount or folder, a model file, or a radio backup id."
            );
        }
        let path = PathBuf::from(from);
        let snaps = self.snapshots();
        let (source, file_bytes): (Box<dyn CardSource + '_>, Option<NamedBytes>) = if path.is_dir()
        {
            Card::open(&path)?;
            (Box::new(Folder(path.clone())), None)
        } else if path.is_file() {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            // Only a model file is read: its text comes back in the answer.
            let size = std::fs::metadata(&path)?.len();
            if !name.to_ascii_lowercase().ends_with(".yml") || size > MODEL_FILE_MAX {
                bail!("{from:?} is not an EdgeTX model file (a .yml file in MODELS/).");
            }
            let bytes = std::fs::read(&path)?;
            // MODELS/modelNN.yml on a card copy: radio.yml is two folders up.
            let root = path
                .parent()
                .and_then(std::path::Path::parent)
                .map(std::path::Path::to_path_buf)
                .unwrap_or_default();
            (Box::new(Folder(root)), Some((name, bytes)))
        } else {
            let backup = snaps.get(from).map_err(|_| {
                anyhow::anyhow!("{from:?} is not a folder, a model file or a backup id.")
            })?;
            (
                Box::new(InBackup {
                    snaps: &snaps,
                    backup,
                }),
                None,
            )
        };
        let radio = source
            .read(ec::RADIO_FILE)
            .and_then(|b| Doc::parse("radio.yml", &b).ok());
        let from_board = p
            .from_board
            .clone()
            .filter(|b| !b.trim().is_empty())
            .or_else(|| radio.as_ref().and_then(|r| r.top_value("board")))
            .map(|b| b.trim().to_ascii_lowercase())
            .ok_or_else(|| anyhow::anyhow!("The source names no board; pass from_board."))?;
        let (file, bytes) = match file_bytes {
            Some(fb) => fb,
            None => {
                let f = match p.model.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
                    Some(m) => m.trim_start_matches("MODELS/").to_string(),
                    None => radio.as_ref().and_then(ec::selected_in).ok_or_else(|| {
                        anyhow::anyhow!("The source names no selected model; pass model.")
                    })?,
                };
                let b = source
                    .read(&format!("MODELS/{f}"))
                    .ok_or_else(|| anyhow::anyhow!("The source has no MODELS/{f}."))?;
                (f, b)
            }
        };
        let src = Doc::parse(&file, &bytes).map_err(anyhow::Error::new)?;
        let src_name = em::model_name(&src)
            .map_err(anyhow::Error::new)?
            .with_context(|| {
                format!("{file} is not an EdgeTX model file: it has no header name.")
            })?;

        let t = self.gear_card_target(p.mount.as_ref(), p.device.as_deref())?;
        let out = (|| {
            let card = Card::open(&t.root)?;
            let view = card.view(p.template.as_deref(), chrono::Local::now().date_naive())?;
            let to_board = view.identity.board.clone();
            let to = match p.to.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(f) => f.trim_start_matches("MODELS/").to_string(),
                None => (0..100)
                    .map(|n| format!("model{n:02}.yml"))
                    .find(|f| !view.models.iter().any(|m| &m.file == f))
                    .ok_or_else(|| anyhow::anyhow!("The card has no free model slot."))?,
            };
            let from_controls = board::inventory(Some(&from_board), None, Some(&src));
            let to_controls = view.controls.clone();
            let links = ec::effective_links(
                &from_board,
                &from_controls,
                to_board.as_deref().unwrap_or(""),
                &to_controls,
                &p.links,
            );
            let translation = board::plan_translation(&src, &from_controls, &to_controls, &links)
                .map_err(anyhow::Error::new)?;
            let edit = Edit::ModelImport {
                from_board: from_board.clone(),
                text: latin1(&bytes),
                to,
                name: p
                    .name
                    .clone()
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or(src_name.clone()),
                links: p.links.clone(),
                template: p.template.clone(),
            };
            let preview = card_preview(&card, &t, std::slice::from_ref(&edit))?;
            Ok(ModelImport {
                edit,
                from_board: from_board.clone(),
                to_board,
                from_controls,
                to_controls,
                links,
                translation,
                preview,
            })
        })();
        self.gear_target_done(&t);
        out
    }
}

/// A file's name and bytes.
type NamedBytes = (String, Vec<u8>);

/// The preview of `edits` on `card`: the plan's checks and diff, and the write's time at
/// the link's speed.
fn card_preview(card: &Card, t: &CardTarget, edits: &[Edit]) -> Result<CardPreview> {
    let plan = card.plan(edits, None)?;
    let board = plan.identity.board.clone();
    let radio_usb = t.c.as_ref().is_some_and(|c| c.usb.is_some())
        || crate::gear::edgetx::board::on_chip(board.as_deref());
    let opts = WriteOptions::for_card(radio_usb, board.as_deref());
    Ok(CardPreview {
        identity: plan.identity.clone(),
        ready: plan.ready(),
        files: plan.files.iter().map(|f| f.path.clone()).collect(),
        bytes: plan.bytes(),
        eta_s: opts.eta_s(plan.bytes(), plan.files.len()),
        checks: plan.checks,
        diff: plan.diff,
        warnings: plan.warnings,
        radio_usb,
    })
}

impl Core {
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
        // A removal needs a card: one Gear detected, or a folder that is an EdgeTX card.
        if c.is_none() {
            if let Err(e) = crate::gear::edgetx::card::Card::open(&root) {
                bail!("Refused: {e:#} QuadCam removes ._ files only from a card it detects or an EdgeTX card.");
            }
        }
        if !crate::gear::edgetx::card::process_writes_allowed(&root, &self.gear.real_cards) {
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
