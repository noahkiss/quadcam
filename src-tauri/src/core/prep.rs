//! Card prep: format a DVR card that has no session behind it, a new card or one whose clips
//! are all in the library. The format after an import (`import.rs`) needs a verified session;
//! prep needs instead that no clip on the card is missing from the library. Both erase through
//! `disk::format_card`, which runs every guard again right before `diskutil eraseDisk`.
//!
//! Gear design 7.9 (WP11). These methods belong with the Gear `Core` methods in
//! `core/gear.rs` once that file exists.

use super::{Core, FormatPlan, FormatRequest};
use crate::disk::{self, CardIdentity};
use crate::library as lib;
use crate::sources::{self, CardPolicy};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// What card prep reads from a volume before it plans the erase.
struct Prepared {
    mount: PathBuf,
    card: CardIdentity,
    policy: CardPolicy,
    plan: FormatPlan,
}

impl Core {
    /// Runs every card-prep guard on the volume at `mount` and names the disk that would be
    /// erased. Writes nothing.
    pub fn card_prep_plan(&self, mount: &Path, label: Option<&str>) -> Result<FormatPlan> {
        Ok(self.prepare(mount, label)?.plan)
    }

    /// Erases the card that `req` names (its whole-disk device and volume UUID, from
    /// `card_prep_plan`) and ejects it. Unless the GUI's own button started it, the GUI (when
    /// running) must also get a click on Erase. `disk::format_card` checks every guard once
    /// more, the library check included.
    pub fn card_prep(&self, req: &FormatRequest, from_gui_button: bool) -> Result<FormatPlan> {
        if !req.confirm {
            bail!("Refused: card prep needs an explicit confirm.");
        }
        let uuid = req.volume_uuid.trim();
        if uuid.is_empty() {
            bail!("Refused: card prep needs the card's volume UUID (see the plan).");
        }
        let mount = mount_of_volume(uuid)?;
        let p = self.prepare(&mount, req.label.as_deref())?;
        let device = req.device.trim();
        if device != p.plan.device {
            bail!(
                "Refused: --device {device} is not the card's whole disk {}.",
                p.plan.device
            );
        }
        if !uuid.eq_ignore_ascii_case(&p.plan.volume_uuid) {
            bail!(
                "Refused: volume UUID {uuid} does not match the card ({}).",
                p.plan.volume_uuid
            );
        }
        if !from_gui_button {
            self.hooks.confirm_format(&p.plan)?;
        }
        let _b = self.claim()?;
        let eject_error = disk::format_card(&p.card, &p.mount, &p.plan.label, &p.policy, &|| {
            self.check_all_in_library(&p.mount).map(|_| ())
        })?;
        // A session read from this card can no longer format it.
        if let Some(mut s) = self.session() {
            if s.card.as_ref().is_some_and(|c| {
                c.whole_disk == p.card.whole_disk && c.volume_uuid == p.card.volume_uuid
            }) {
                s.card = None;
                s.warnings
                    .push(format!("Card {} was erased by card prep.", p.plan.device));
                self.commit(Some(s))?;
            }
        }
        match eject_error {
            None => Ok(p.plan),
            Some(e) => bail!(
                "The card was erased, but it did not eject ({e}). Eject it before you pull it."
            ),
        }
    }

    fn prepare(&self, mount: &Path, label: Option<&str>) -> Result<Prepared> {
        let info = disk::info(&mount.to_string_lossy())
            .with_context(|| format!("{} is not a mounted volume", mount.display()))?;
        let mount = PathBuf::from(
            info.mount_point
                .clone()
                .with_context(|| format!("{} is not mounted", mount.display()))?,
        );
        // What the card holds refuses first: a radio or DJI card, then a source that does
        // not offer a format. A card with no clips is prepared for an analog DVR.
        disk::check_volume_contents(&mount)?;
        let source = sources::detect(&mount).unwrap_or(&sources::analog::Analog);
        let policy = source.card_policy();
        if !policy.format_offered {
            bail!(
                "Refused: QuadCam does not format {} cards; format them in the device.",
                source.kind().label()
            );
        }
        let clips = self.check_all_in_library(&mount)?;
        let card = CardIdentity::from_info(&info);
        let whole = disk::verify_card_for_format(&card, &mount)?;
        let plan = FormatPlan {
            disk: card.whole_disk.clone(),
            device: format!("/dev/{}", card.whole_disk),
            volume_uuid: card.volume_uuid.clone().unwrap_or_default(),
            volume_name: card.volume_name.clone().unwrap_or_default(),
            size: whole.total_size,
            media_name: whole.media_name.clone().unwrap_or_default(),
            clip_count: clips,
            label: disk::fat_label(
                label
                    .filter(|l| !l.trim().is_empty())
                    .unwrap_or(&self.defaults().format_label),
            )?,
        };
        Ok(Prepared {
            mount,
            card,
            policy,
            plan,
        })
    }

    /// The prep guard: every clip on the card (by any source) is in the library, by its
    /// current fingerprint or its 0.4 one. A clip that is only staged in the session does not
    /// count. Returns the number of clips.
    fn check_all_in_library(&self, mount: &Path) -> Result<usize> {
        let mut found: Vec<PathBuf> = sources::all()
            .iter()
            .flat_map(|s| s.list(mount))
            .map(|c| c.path)
            .collect();
        found.sort();
        found.dedup();
        if found.is_empty() {
            return Ok(0);
        }
        let known = self
            .with_index(|_, ix| Ok((lib::known_sources(ix), false)))
            .context("Refused: the library cannot be read, so QuadCam cannot tell whether the card's clips are in it")?;
        let missing: Vec<String> = found
            .iter()
            .filter(|p| {
                crate::identity::fingerprints(p)
                    .map(|(now, old)| !(known.contains(&now) || known.contains(&old)))
                    .unwrap_or(true)
            })
            .map(|p| {
                p.strip_prefix(mount)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        if !missing.is_empty() {
            let shown: Vec<&str> = missing.iter().take(5).map(String::as_str).collect();
            let more = if missing.len() > shown.len() {
                format!(" and {} more", missing.len() - shown.len())
            } else {
                String::new()
            };
            bail!(
                "Refused: {} of {} clips on the card are not in the library ({}{more}). Import them first.",
                missing.len(),
                found.len(),
                shown.join(", ")
            );
        }
        Ok(found.len())
    }
}

/// The mount point of the volume with this UUID. `diskutil info` takes a volume UUID.
fn mount_of_volume(uuid: &str) -> Result<PathBuf> {
    let info = disk::info(uuid)
        .with_context(|| format!("Refused: no volume with UUID {uuid} is attached."))?;
    if !info
        .volume_uuid
        .as_deref()
        .is_some_and(|u| u.eq_ignore_ascii_case(uuid))
    {
        bail!("Refused: no volume with UUID {uuid} is attached.");
    }
    info.mount_point
        .map(PathBuf::from)
        .with_context(|| format!("Refused: the volume with UUID {uuid} is not mounted."))
}
