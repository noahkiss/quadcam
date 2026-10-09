//! The import session's card and the mount cycle (the same cycle as `apply_card.rs`):
//! mount if needed, work, unmount when done.
//!
//! - **Stage.** A card named by device id (`SourceParams.device`) that is unmounted but
//!   still plugged in is mounted for the stage (`Core::mount_source`).
//! - **Done.** When an import ends (after "Delete clips after import"), the card is
//!   unmounted (`release_session_card`): the staged copies hold the clips. The outcome says
//!   whether it is safe to remove. A DJI device over USB (not a removable card) stays
//!   mounted: QuadCam never unmounts or ejects it on its own.
//! - **Later.** Format, a second deletion and "Safe to remove" mount the card again first
//!   (`ensure_session_card`).

use super::Core;
use crate::disk::{self, CardIdentity};
use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::time::Duration;

/// What the end of an import did with the session's card.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, specta::Type)]
pub struct CardRelease {
    /// The card is unmounted: safe to remove.
    pub released: bool,
    pub message: String,
}

impl Core {
    /// The mount point of the card with this device id, mounting it first when it is
    /// unmounted but still plugged in. Any card kind (a DVR card, goggles, a radio).
    pub(super) fn mount_source(&self, device: &str) -> Result<PathBuf> {
        let l = self
            .locate_volume(device.trim(), true, false)?
            .with_context(|| format!("Card {:?} is not plugged in.", device.trim()))?;
        Ok(l.root)
    }

    /// With no source named: the one mounted card, else the one unmounted card still
    /// plugged in (mounted now). None when there is neither.
    pub(super) fn mount_only_card(&self) -> Result<Option<PathBuf>> {
        let unmounted: Vec<_> = self
            .unmounted_cards(false)
            .into_iter()
            .filter(|c| {
                matches!(
                    c.kind,
                    crate::gear::model::DeviceKind::DvrCard
                        | crate::gear::model::DeviceKind::Goggles
                )
            })
            .collect();
        match unmounted.as_slice() {
            [one] => Ok(Some(self.card_for_job(one, false)?.root)),
            _ => Ok(None),
        }
    }

    /// The session's card: its mount point, mounting the card first when it is not mounted.
    /// None when the session came from a folder.
    pub(super) fn ensure_session_card(&self) -> Result<Option<PathBuf>> {
        let Some(s) = self.session() else {
            return Ok(None);
        };
        let Some(card) = s.card.clone() else {
            return Ok(None);
        };
        if let Some(m) = mount_of(&card) {
            return Ok(Some(m));
        }
        (self.gear.mount)(&card.whole_disk).with_context(|| {
            format!(
                "Mounting {} for the card step",
                card.volume_name.as_deref().unwrap_or("the card")
            )
        })?;
        for i in 0..40 {
            if let Some(m) = mount_of(&card) {
                // A card can come back under another path (a name clash): follow it.
                if let Some(mut latest) = self.session() {
                    if latest.source != m {
                        latest.source = m.clone();
                        self.commit(Some(latest))?;
                    }
                }
                return Ok(Some(m));
            }
            if i < 39 {
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        bail!(
            "{} did not mount. Take the card out and put it back.",
            card.volume_name.as_deref().unwrap_or("The card")
        )
    }

    /// False when the session has a card and it is not mounted now.
    pub(super) fn session_card_mounted(&self) -> bool {
        match self.session().and_then(|s| s.card) {
            Some(card) => mount_of(&card).is_some(),
            None => true,
        }
    }

    /// Unmounts the session's card when the work on it is done. None when the session has
    /// no card, or the card is not a removable one. Never fails the caller: the outcome
    /// says whether the card is safe to remove.
    pub(super) fn release_session_card(&self) -> Option<CardRelease> {
        let card = self.session()?.card?;
        let whole = format!("/dev/{}", card.whole_disk);
        let info = disk::info(&whole).ok()?;
        // A card only: never a DJI device over USB (an air unit, goggles in storage mode).
        let dji_device = info
            .media_name
            .as_deref()
            .is_some_and(|n| n.to_uppercase().contains("DJI"))
            || (!info.is_slot_card() && crate::gear::events::dji_usb_attached());
        if !disk::is_removable(&info) || dji_device {
            return None;
        }
        let name = card
            .volume_name
            .clone()
            .unwrap_or_else(|| "The card".into());
        let r = {
            let _hold = self.gear_hold(&card.whole_disk);
            (self.gear.unmount)(&card.whole_disk)
        };
        Some(match r {
            Ok(()) => CardRelease {
                released: true,
                message: format!("{name} is unmounted. It is safe to remove."),
            },
            Err(e) => CardRelease {
                released: false,
                message: format!(
                    "{name} did not unmount ({e:#}). Eject it in Finder before you pull it."
                ),
            },
        })
    }
}

/// The mount point of the card's volume, when it is mounted.
fn mount_of(card: &CardIdentity) -> Option<PathBuf> {
    let target = card
        .volume_uuid
        .clone()
        .unwrap_or_else(|| card.device_identifier.clone());
    disk::info(&target)
        .ok()
        .filter(|i| {
            card.volume_uuid.is_none()
                || i.volume_uuid
                    .as_deref()
                    .is_some_and(|u| Some(u) == card.volume_uuid.as_deref())
        })
        .and_then(|i| i.mount_point)
        .map(PathBuf::from)
}
