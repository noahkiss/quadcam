//! `Core`'s Gear half: the gear folder, the devices QuadCam knows, and what is plugged in
//! now. Every surface reaches Gear through these methods and the rows in `api/gear.rs`.
//! Later packages add their methods here (backups, changes, apply, ...), each a thin call
//! into its `gear::` module.

use super::Core;
use crate::gear::cues::{self, Cue, Reminders};
use crate::gear::events::{DeviceEvent, DeviceEventKind, Tracker};
use crate::gear::model::{Connected, Device, DeviceKind};
use crate::gear::store::Store;
use crate::gear::{Automation, GearSettings};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

/// `gear_status`' answer.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct GearStatus {
    /// The gear folder in use.
    pub gear_dir: PathBuf,
    /// The Gear settings in effect.
    pub settings: GearSettings,
    /// What is plugged in now, each with its saved record when QuadCam knows it.
    pub connected: Vec<Connected>,
    /// How many devices `gear.json` holds.
    pub devices: usize,
    /// Staged changes not yet applied (the Bench badge).
    pub staged: usize,
    /// Sims whose rates differ from their quad's.
    pub sims_out_of_date: usize,
}

/// `gear_device_save`: names a device or links it to an aircraft. A device QuadCam does
/// not know yet must be plugged in (its id from `gear_status`' `connected`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct DeviceSaveParams {
    pub id: String,
    /// The person's name for it; empty for none.
    #[serde(default)]
    pub name: Option<String>,
    /// An aircraft profile name; empty to unlink.
    #[serde(default)]
    pub aircraft: Option<String>,
}

impl Core {
    /// The Gear settings: the settings file's values on top of the defaults. The default
    /// gear folder sits next to the settings file (the support folder), so a core on a test
    /// settings file never reaches the real one.
    pub fn gear_settings(&self) -> GearSettings {
        let default_dir = self
            .settings_file
            .as_deref()
            .and_then(|f| f.parent())
            .map(|d| d.join("gear"))
            .unwrap_or_else(crate::paths::default_gear_dir);
        let values = self
            .settings_file
            .as_deref()
            .map(|f| crate::settings::read(f).unwrap_or_default())
            .unwrap_or_default();
        GearSettings::from_values(&values, default_dir)
    }

    /// The gear folder.
    pub fn gear_store(&self) -> Store {
        Store::new(self.gear_settings().gear_dir)
    }

    /// Replaces what Gear reaches outside the process (tests pass fakes).
    pub fn with_gear_env(mut self, env: crate::gear::Env) -> Core {
        self.gear = env;
        self
    }

    /// What is plugged in now, each with its saved record. Reads only.
    pub fn gear_connected(&self) -> Result<Vec<Connected>> {
        let saved = self.gear_store().devices()?;
        let mut found = self.gear.detect();
        for c in &mut found {
            c.device =
                c.id.as_deref()
                    .and_then(|id| saved.iter().find(|d| d.id == id).cloned());
        }
        Ok(found)
    }

    pub fn gear_status(&self) -> Result<GearStatus> {
        let settings = self.gear_settings();
        let store = Store::new(settings.gear_dir.clone());
        Ok(GearStatus {
            gear_dir: settings.gear_dir.clone(),
            connected: self.gear_connected()?,
            devices: store.devices()?.len(),
            staged: 0,
            sims_out_of_date: 0,
            settings,
        })
    }

    pub fn gear_devices(&self) -> Result<Vec<Device>> {
        self.gear_store().devices()
    }

    /// Names a device or links it to an aircraft. A new device is saved from what is
    /// plugged in now.
    pub fn gear_device_save(&self, p: &DeviceSaveParams) -> Result<Device> {
        let id = p.id.trim();
        if id.is_empty() {
            bail!("id is required: a device id from gear_devices or gear_status");
        }
        let store = self.gear_store();
        let mut d = match store.device(id)? {
            Some(d) => d,
            None => {
                let c = self
                    .gear_connected()?
                    .into_iter()
                    .find(|c| c.id.as_deref() == Some(id))
                    .with_context(|| {
                        format!(
                            "No device with id {id:?}. Plug it in, or pick one from gear_devices."
                        )
                    })?;
                Device {
                    id: id.to_string(),
                    kind: c.kind,
                    name: String::new(),
                    aircraft: None,
                    identity: c.identity,
                    last_seen: Some(chrono::Utc::now()),
                    last_backup: None,
                }
            }
        };
        if let Some(name) = &p.name {
            let n = name.trim();
            if n.chars().count() > 80 {
                bail!("A device name is at most 80 characters.");
            }
            d.name = n.to_string();
        }
        if let Some(a) = &p.aircraft {
            let a = a.trim();
            d.aircraft = if a.is_empty() {
                None
            } else {
                let (profiles, _) = self.profiles()?;
                let p = profiles
                    .iter()
                    .find(|p| p.name.trim().eq_ignore_ascii_case(a))
                    .with_context(|| {
                        format!(
                            "No aircraft profile {a:?}. Profiles: {}",
                            if profiles.is_empty() {
                                "none".to_string()
                            } else {
                                profiles
                                    .iter()
                                    .map(|p| p.name.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            }
                        )
                    })?;
                Some(p.name.clone())
            };
        }
        let d = store.save_device(&d)?;
        self.hooks.gear_changed();
        Ok(d)
    }

    /// Removes a device from `gear.json`. Its backups stay in the gear folder.
    pub fn gear_device_forget(&self, id: &str) -> Result<Device> {
        let d = self.gear_store().forget_device(id.trim())?;
        self.hooks.gear_changed();
        Ok(d)
    }

    /// For the app's poll: the events since the tracker's last look (see `gear::events`).
    /// A known device that connects or is identified is noted as seen (its last-seen time
    /// and what detection read). New devices are not saved here. The caller runs the
    /// on-connect hooks (`gear_on_connect`) for `connected` and `identified`.
    pub fn gear_poll(&self, tracker: &mut Tracker) -> Result<Vec<DeviceEvent>> {
        let found = self.gear_connected()?;
        let mut events = tracker.update(found, &(self.gear.presence)());
        let store = self.gear_store();
        for e in &mut events {
            if !matches!(
                e.kind,
                DeviceEventKind::Connected | DeviceEventKind::Identified
            ) {
                continue;
            }
            let c = &mut e.device;
            if let (Some(id), Some(_)) = (c.id.clone(), &c.device) {
                let d = store.seen(&id, c.kind, &c.identity)?;
                c.device = Some(d.clone());
                if let Some(t) = tracker.connected.iter_mut().find(|t| t.link == c.link) {
                    t.device = Some(d);
                }
            }
        }
        Ok(events)
    }

    /// Adds a step that may run when a device is plugged in. Later packages register
    /// theirs (backup, import, apply ready changes) when the core is built.
    pub fn gear_add_hook(&self, hook: OnConnectHook) {
        self.gear_hooks.lock().unwrap().push(hook);
    }

    /// Runs the on-connect hooks for this device's kind, each only when the Gear settings
    /// turn its automation on for that kind. A failure plays the `step_failed` cue.
    pub fn gear_on_connect(&self, c: &Connected) -> Vec<HookRun> {
        let settings = self.gear_settings();
        let hooks: Vec<OnConnectHook> = self
            .gear_hooks
            .lock()
            .unwrap()
            .iter()
            .filter(|h| h.kinds.contains(&c.kind))
            .cloned()
            .collect();
        hooks
            .into_iter()
            .map(|h| {
                let outcome = if !settings.runs(c.kind, h.automation) {
                    HookOutcome::Off
                } else {
                    match (h.run)(self, c) {
                        Ok(()) => HookOutcome::Ran,
                        Err(e) => {
                            self.gear_cue(Cue::StepFailed, &connected_name(c));
                            HookOutcome::Failed {
                                message: format!("{e:#}"),
                            }
                        }
                    }
                };
                HookRun {
                    name: h.name.to_string(),
                    automation: h.automation,
                    outcome,
                }
            })
            .collect()
    }

    /// Plays a cue for a device, as the Gear settings say. False when that cue is off.
    pub fn gear_cue(&self, cue: Cue, device: &str) -> bool {
        cues::fire(
            self.gear.cues.as_ref(),
            &self.gear_settings().cues,
            cue,
            device,
        )
    }

    /// Plays `safe_to_unplug` for each device that unmounted but is still in, and
    /// `still_inserted` for each one whose reminder is due. For the app's poll.
    pub fn gear_play_cues(
        &self,
        events: &[DeviceEvent],
        tracker: &Tracker,
        reminders: &mut Reminders,
        now: std::time::Instant,
    ) {
        for e in events {
            if e.kind == DeviceEventKind::UnmountedPresent {
                self.gear_cue(Cue::SafeToUnplug, &connected_name(&e.device));
            }
        }
        let every = std::time::Duration::from_secs(u64::from(
            self.gear_settings().cues.still_inserted_every_s,
        ));
        let keys: Vec<String> = tracker.unmounted.iter().map(link_key).collect();
        for k in reminders.due(&keys, every, now) {
            if let Some(c) = tracker.unmounted.iter().find(|c| link_key(c) == k) {
                self.gear_cue(Cue::StillInserted, &connected_name(c));
            }
        }
    }
}

/// The name a cue says for a connected device: its saved name, else its kind.
pub fn connected_name(c: &Connected) -> String {
    match &c.device {
        Some(d) => d.display_name(),
        None => format!("The {}", c.kind.label()),
    }
}

/// A connected device's link as text, the key of its reminder.
fn link_key(c: &Connected) -> String {
    serde_json::to_string(&c.link).unwrap_or_default()
}

/// A step that runs on its own when a device is plugged in.
pub type HookFn = Arc<dyn Fn(&Core, &Connected) -> Result<()> + Send + Sync>;

/// An on-connect step: its name, the automation setting that gates it, the device kinds
/// it is for, and what it runs.
#[derive(Clone)]
pub struct OnConnectHook {
    pub name: &'static str,
    pub automation: Automation,
    pub kinds: Vec<DeviceKind>,
    pub run: HookFn,
}

/// What one hook did.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum HookOutcome {
    Ran,
    /// Its automation is off for this kind.
    Off,
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct HookRun {
    pub name: String,
    pub automation: Automation,
    pub outcome: HookOutcome,
}
