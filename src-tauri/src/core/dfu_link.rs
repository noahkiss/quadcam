//! Links a radio in DFU mode to its saved radio. DFU mode is the one mode that shows the
//! chip's unique serial (storage and serial modes do not), so the link is made once, by the
//! person's pick or by the last-seen order, and kept in the saved radio (`Device.dfu_serial`).

use super::Core;
use crate::gear::detect::STM32_DFU;
use crate::gear::model::{Device, DeviceKind};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// `gear_dfu_link`: link the radio in DFU mode to a saved radio, or remove a link.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct DfuLinkParams {
    /// The saved radio's id. Omitted: the radio seen most recently.
    #[serde(default)]
    pub device: Option<String>,
    /// The DFU chip serial. Omitted: the one radio in DFU mode now.
    #[serde(default)]
    pub serial: Option<String>,
    /// Remove the link of `device` (or of the radio that holds `serial`).
    #[serde(default)]
    pub unlink: bool,
}

/// What `gear_dfu_link` did.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct DfuLinked {
    pub device: Device,
    /// The DFU serial now linked; none after an unlink.
    #[serde(default)]
    pub serial: Option<String>,
    /// How the radio was chosen: `picked` (the person named it) or `last_seen`.
    pub how: String,
    pub notes: Vec<String>,
}

/// The saved radio seen most recently. With one saved radio, that one.
pub fn last_seen_radio(devices: &[Device]) -> Option<&Device> {
    let radios: Vec<&Device> = devices
        .iter()
        .filter(|d| d.kind == DeviceKind::Radio)
        .collect();
    match radios.as_slice() {
        [one] => Some(one),
        _ => radios
            .into_iter()
            .filter(|d| d.last_seen.is_some())
            .max_by_key(|d| d.last_seen),
    }
}

impl Core {
    /// The DFU serial of the one radio in DFU mode now, or why there is none.
    pub(super) fn dfu_serial_now(&self) -> Result<String> {
        let now: Vec<_> = (self.gear.dfu)()
            .into_iter()
            .filter(|d| (d.vid, d.pid) == STM32_DFU)
            .collect();
        match now.as_slice() {
            [] => bail!("No radio is in DFU mode. Turn the radio off, hold both trim buttons toward the centre and plug in the USB cable."),
            [one] => one.serial.clone().filter(|s| !s.is_empty()).ok_or_else(|| {
                anyhow::anyhow!("The radio in DFU mode reports no serial number, so it cannot be linked.")
            }),
            n => bail!("{} radios are in DFU mode; unplug all but one, or name the serial.", n.len()),
        }
    }

    /// Links the radio in DFU mode to a saved radio. The pick is `device`; without one the
    /// radio seen most recently is taken, and the answer says so. A DFU serial belongs to one
    /// radio: linking it to another moves it.
    pub fn gear_dfu_link(&self, p: &DfuLinkParams) -> Result<DfuLinked> {
        let store = self.gear_store();
        let devices = store.devices()?;
        let named = p.device.as_deref().map(str::trim).filter(|d| !d.is_empty());
        let mut notes = Vec::new();
        if p.unlink {
            let serial = p.serial.as_deref().map(str::trim).filter(|s| !s.is_empty());
            let target = devices
                .iter()
                .find(|d| match (named, serial) {
                    (Some(id), _) => d.id == id,
                    (None, Some(s)) => d.dfu_serial.as_deref() == Some(s),
                    (None, None) => false,
                })
                .ok_or_else(|| {
                    anyhow::anyhow!("Name the radio to unlink with device, or its DFU serial.")
                })?;
            let mut d = target.clone();
            if d.dfu_serial.take().is_none() {
                notes.push("This radio had no DFU link.".to_string());
            }
            store.save_device(&d)?;
            self.hooks.gear_changed();
            return Ok(DfuLinked {
                device: d,
                serial: None,
                how: "picked".into(),
                notes,
            });
        }
        let serial = match p.serial.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => s.to_string(),
            None => self.dfu_serial_now()?,
        };
        let (mut d, how) = match named {
            Some(id) => (
                devices
                    .iter()
                    .find(|d| d.id == id && d.kind == DeviceKind::Radio)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("No saved radio with id {id:?}. Pick one from gear_devices."))?,
                "picked",
            ),
            None => (
                last_seen_radio(&devices).cloned().ok_or_else(|| {
                    anyhow::anyhow!("No saved radio has been seen yet. Connect the radio in USB Storage mode first, or name it with device.")
                })?,
                "last_seen",
            ),
        };
        if how == "last_seen" {
            notes.push(format!(
                "Linked to {}, the radio seen most recently. If that is not the radio in DFU mode, link again with device.",
                d.display_name()
            ));
        }
        for other in devices
            .iter()
            .filter(|o| o.id != d.id && o.dfu_serial.as_deref() == Some(serial.as_str()))
        {
            let mut o = other.clone();
            o.dfu_serial = None;
            store.save_device(&o)?;
            notes.push(format!("Moved the link from {}.", other.display_name()));
        }
        if let Some(old) = d.dfu_serial.as_deref().filter(|o| *o != serial) {
            notes.push(format!("Replaced the earlier DFU link ({old})."));
        }
        d.dfu_serial = Some(serial.clone());
        store.save_device(&d)?;
        self.hooks.gear_changed();
        Ok(DfuLinked {
            device: d,
            serial: Some(serial),
            how: how.into(),
            notes,
        })
    }
}
