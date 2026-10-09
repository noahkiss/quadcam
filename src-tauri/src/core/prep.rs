//! Card prep: format a card that has no session behind it, a new card or one whose clips
//! are all in the library: an analog DVR card as FAT32, a removable DJI goggles card as
//! exFAT (the source's `CardPolicy`). A DJI device over USB is never erased. The format after an import (`import.rs`) needs a verified session;
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

    /// `card_prep_plan` for a mount point or a device id (`CardPrepParams`).
    pub fn card_prep_plan_for(&self, p: &crate::api::CardPrepParams) -> Result<FormatPlan> {
        match (
            &p.mount,
            p.device.as_deref().map(str::trim).filter(|d| !d.is_empty()),
        ) {
            (Some(m), _) => self.card_prep_plan(m, p.label.as_deref()),
            (None, Some(d)) => self.card_prep_plan_device(d, p.label.as_deref()),
            (None, None) => bail!("Card prep needs the card's mount point or its device id."),
        }
    }

    /// `card_prep_plan` for a card named by device id (`gear_status`). Mount, work,
    /// unmount: an unmounted card that is still plugged in is mounted for the plan and
    /// released again; a card that is mounted stays as it is.
    pub fn card_prep_plan_device(&self, device: &str, label: Option<&str>) -> Result<FormatPlan> {
        let l = self
            .locate_volume(device.trim(), true, false)?
            .with_context(|| format!("Card {:?} is not plugged in.", device.trim()))?;
        let plan = self.prepare(&l.root, label).map(|p| p.plan);
        self.card_quiet_unmount(&l);
        plan
    }

    /// Erases the card that `req` names (its whole-disk device and volume UUID, from
    /// `card_prep_plan`) and makes it safe to remove. Unless the GUI's own button started it, the GUI (when
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
        // Mount, work, unmount: a card that is not mounted is mounted for the erase. A
        // successful erase unmounts it itself; any other end releases it here.
        let (mount, mounted_here) = self.mount_volume_for_job(uuid)?;
        let r = self.card_prep_on(&mount, req, from_gui_button);
        if r.is_err() && mounted_here {
            if let Ok(info) = disk::info(uuid) {
                let _ = (self.gear.unmount)(&disk::whole_disk_of(&info.parent_whole_disk));
            }
        }
        r
    }

    /// The mount point of the volume with this UUID, mounting its disk when it is
    /// unmounted but still plugged in. True when this call mounted it.
    fn mount_volume_for_job(&self, uuid: &str) -> Result<(PathBuf, bool)> {
        let info = disk::info(uuid)
            .with_context(|| format!("Refused: no volume with UUID {uuid} is attached."))?;
        if !info
            .volume_uuid
            .as_deref()
            .is_some_and(|u| u.eq_ignore_ascii_case(uuid))
        {
            bail!("Refused: no volume with UUID {uuid} is attached.");
        }
        if let Some(m) = info.mount_point {
            return Ok((PathBuf::from(m), false));
        }
        let whole = disk::whole_disk_of(&info.parent_whole_disk);
        (self.gear.mount)(&whole).with_context(|| format!("Mounting {whole} for card prep"))?;
        for i in 0..40 {
            if let Some(m) = disk::info(uuid).ok().and_then(|i| i.mount_point) {
                return Ok((PathBuf::from(m), true));
            }
            if i < 39 {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
        bail!("Refused: the volume with UUID {uuid} did not mount. Take the card out and put it back.")
    }

    fn card_prep_on(
        &self,
        mount: &Path,
        req: &FormatRequest,
        from_gui_button: bool,
    ) -> Result<FormatPlan> {
        let uuid = req.volume_uuid.trim();
        let mount = mount.to_path_buf();
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
        let eject_error = disk::format_card(
            &p.card,
            &p.mount,
            &p.plan.label,
            &p.policy,
            disk::EraseBy::Prep,
            &|| self.check_all_in_library(&p.mount).map(|_| ()),
        )?;
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
                "The card was erased, but it did not unmount ({e}). Eject it in Finder before you pull it."
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
        // What the card holds and is refuses first: a radio's card, a DJI device over USB,
        // then a source whose policy does not offer prep. A card with DJI folders (even
        // emptied) is prepared for DJI goggles; a card with no clips for an analog DVR.
        disk::check_volume_contents(&mount, &info, crate::gear::events::dji_usb_attached())?;
        let source: &dyn sources::Source = match sources::detect(&mount) {
            Some(s) => s,
            None if sources::dji::looks_like_dji_volume(&mount) => &sources::dji::Dji,
            None => &sources::analog::Analog,
        };
        let policy = source.card_policy();
        disk::check_policy(&policy, disk::EraseBy::Prep)?;
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
            filesystem: policy.filesystem.to_string(),
            warnings: disk::format_advice(whole.total_size, &policy),
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
