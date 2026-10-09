//! `Core`'s Gear half: the gear folder, the devices QuadCam knows, and what is plugged in
//! now. Every surface reaches Gear through these methods and the rows in `api/gear.rs`.
//! Later packages add their methods here (backups, changes, apply, ...), each a thin call
//! into its `gear::` module.

use super::Core;
use crate::gear::cues::{self, Cue, CueEvent};
use crate::gear::events::{DeviceEvent, DeviceEventKind, Tracker};
use crate::gear::model::{Connected, Device, DeviceKind, Link};
use crate::gear::store::Store;
use crate::gear::{Automation, GearSettings};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    /// FCs on USB: battery in, time on USB, the limit (`core/fc.rs`).
    #[serde(default)]
    pub usb_timers: Vec<super::fc::UsbTimer>,
    /// The FC ports whose background reads are paused (`gear_poll_pause`).
    #[serde(default)]
    pub paused: Vec<String>,
    /// The links a job holds now (`link_handle`): their devices show as working.
    #[serde(default)]
    pub working: Vec<String>,
    /// The links with a "still inserted" reminder armed (`link_handle`).
    #[serde(default)]
    pub reminders: Vec<String>,
    /// Backups and card checks running now, with their progress (`core/backup.rs`).
    #[serde(default)]
    pub jobs: Vec<super::GearJob>,
    /// The latest card check of each card plugged in.
    #[serde(default)]
    pub card_checks: Vec<crate::gear::health::CardCheck>,
    /// The last failed step of each device plugged in (an unmount that failed, a backup
    /// that failed), with the reason.
    #[serde(default)]
    pub failures: Vec<StepFailure>,
    /// The radio cards the person mounted to browse, and when each unmounts.
    #[serde(default)]
    pub mounted: Vec<super::CardMounted>,
}

/// A step that failed on a device, and why.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct StepFailure {
    /// The link (`link_handle`).
    pub handle: String,
    pub device: Option<String>,
    /// `Unmount`, `Backup`, `Card check`, ...
    pub step: String,
    pub message: String,
    pub at: chrono::DateTime<chrono::Utc>,
}

/// An on-connect step returns this error when it had nothing to do (a card QuadCam does
/// not know yet): the step counts as off, and the run plays no cue for it.
#[derive(Debug)]
pub struct Skip;

impl std::fmt::Display for Skip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("nothing to do")
    }
}

impl std::error::Error for Skip {}

/// `gear_dismiss_reminder`: a device's link, as `link_handle` names it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct ReminderParams {
    pub handle: String,
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
        self.fc_fill(&mut found);
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
        let connected = self.gear_connected()?;
        let handles: Vec<String> = connected.iter().map(|c| link_handle(&c.link)).collect();
        let mut failures: Vec<StepFailure> = self
            .gear_failures
            .lock()
            .unwrap()
            .values()
            .filter(|f| handles.contains(&f.handle))
            .cloned()
            .collect();
        failures.sort_by(|a, b| a.handle.cmp(&b.handle));
        Ok(GearStatus {
            failures,
            mounted: self.gear_mounted_cards(),
            gear_dir: settings.gear_dir.clone(),
            card_checks: self.gear_latest_checks(&connected),
            jobs: self.gear_jobs(),
            connected,
            devices: store.devices()?.len(),
            staged: self.gear_staged_counts().values().sum(),
            sims_out_of_date: 0,
            usb_timers: self.gear_usb_timers(),
            paused: self.gear_paused_ports(),
            working: self.gear_working(),
            reminders: self.gear.cues.reminders.lock().unwrap().armed(),
            settings,
        })
    }

    /// The links a job holds now; a released hold in its grace time does not count.
    pub fn gear_working(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .gear_holds
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, until)| until.is_none())
            .map(|(k, _)| k.clone())
            .collect();
        v.sort();
        v
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

    /// Marks a device's link as QuadCam's own while the returned guard lives, and for
    /// `HOLD_GRACE` after: events on it (QuadCam's own mounts, unmounts and port opens) are
    /// `app_initiated`, run no hooks and play no cue. Every job holds its device.
    pub fn gear_hold(&self, handle: &str) -> Hold<'_> {
        self.gear_holds
            .lock()
            .unwrap()
            .insert(handle.to_string(), None);
        self.hooks.gear_changed();
        Hold {
            core: self,
            handle: handle.to_string(),
        }
    }

    /// True while a job holds the link (not counting the grace after it ends).
    pub(super) fn held_by_job(&self, handle: &str) -> bool {
        matches!(self.gear_holds.lock().unwrap().get(handle), Some(None))
    }

    pub(super) fn held(&self, handle: &str, now: Instant) -> bool {
        let mut holds = self.gear_holds.lock().unwrap();
        holds.retain(|_, until| until.is_none_or(|t| t > now));
        holds.contains_key(handle)
    }

    /// For the app's poll: the events since the tracker's last look (see `gear::events`).
    /// Events on a held link are `app_initiated`. A known device that connects or is
    /// identified is noted as seen (its last-seen time and what detection read). New
    /// devices are not saved here. The caller runs the on-connect hooks
    /// (`gear_on_connect`) for user `connected` and `identified` events.
    pub fn gear_poll(&self, tracker: &mut Tracker) -> Result<Vec<DeviceEvent>> {
        let found = self.gear_connected()?;
        let mut events = tracker.update(found, &(self.gear.presence)());
        let store = self.gear_store();
        let now = Instant::now();
        for e in &mut events {
            e.app_initiated = self.held(&link_handle(&e.device.link), now);
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
    /// turn its automation on for that kind. The run is one job: it holds the device, and
    /// plays one cue at its end ("done" or the first failed step), none when nothing ran.
    /// A step that returns `Skip` counts as off. A card a step read or checked is unmounted
    /// at the end (`diskutil unmountDisk`), and "done, safe to unplug" plays only when that
    /// worked; a failed unmount is a failed step ("Unmount") with its reason.
    pub fn gear_on_connect(&self, c: &Connected) -> Vec<HookRun> {
        let settings = self.gear_settings();
        let handle = link_handle(&c.link);
        self.gear_touched.lock().unwrap().remove(&handle);
        let hooks: Vec<OnConnectHook> = self
            .gear_hooks
            .lock()
            .unwrap()
            .iter()
            .filter(|h| h.kinds.contains(&c.kind))
            .cloned()
            .collect();
        let mut runs: Vec<HookRun> = {
            let _hold = self.gear_hold(&handle);
            hooks
                .into_iter()
                .map(|h| {
                    let outcome = if !settings.runs(c.kind, h.automation) {
                        HookOutcome::Off
                    } else {
                        match (h.run)(self, c) {
                            Ok(()) => HookOutcome::Ran,
                            Err(e) if e.downcast_ref::<Skip>().is_some() => HookOutcome::Off,
                            Err(e) => HookOutcome::Failed {
                                message: format!("{e:#}"),
                            },
                        }
                    };
                    HookRun {
                        name: h.name.to_string(),
                        automation: h.automation,
                        outcome,
                    }
                })
                .collect()
        };
        let touched = self.gear_touched.lock().unwrap().remove(&handle);
        let failed = runs.iter().find_map(|r| match &r.outcome {
            HookOutcome::Failed { message } => Some((r.name.clone(), message.clone())),
            _ => None,
        });
        if touched {
            if let Err(message) =
                self.gear_finish_card(c, failed.as_ref().map(|(s, m)| (s.as_str(), m.as_str())))
            {
                runs.push(HookRun {
                    name: "Unmount".into(),
                    automation: Automation::Backup,
                    outcome: HookOutcome::Failed { message },
                });
            }
        } else if let Some((step, message)) = failed {
            self.gear_note_failure(c, &step, &message);
            self.gear_job_done(c, Some(&step));
        } else if runs.iter().any(|r| r.outcome == HookOutcome::Ran) {
            self.gear_clear_failure(&handle);
            self.gear_job_done(c, None);
        }
        runs
    }

    /// Marks a card's link as read or checked by the running on-connect job.
    pub(super) fn gear_touch(&self, c: &Connected) {
        self.gear_touched
            .lock()
            .unwrap()
            .insert(link_handle(&c.link));
    }

    /// Records a failed step on a device, for `GearStatus.failures`.
    pub(super) fn gear_note_failure(&self, c: &Connected, step: &str, message: &str) {
        let handle = link_handle(&c.link);
        self.gear_failures.lock().unwrap().insert(
            handle.clone(),
            StepFailure {
                handle,
                device: c.id.clone(),
                step: step.to_string(),
                message: message.to_string(),
                at: chrono::Utc::now(),
            },
        );
        self.hooks.gear_changed();
    }

    pub(super) fn gear_clear_failure(&self, handle: &str) {
        if self.gear_failures.lock().unwrap().remove(handle).is_some() {
            self.hooks.gear_changed();
        }
    }

    /// The end of a job on a card: mount, work, unmount. The card is unmounted
    /// (`unmountDisk`) whatever the job did. With a failed step, its cue plays and its
    /// reason is kept. Otherwise "done, safe to unplug" plays only after the unmount
    /// worked; a failed unmount plays "Unmount failed" and returns its reason.
    pub(super) fn gear_finish_card(
        &self,
        c: &Connected,
        failed: Option<(&str, &str)>,
    ) -> std::result::Result<(), String> {
        match failed {
            Some((step, message)) => {
                if let Link::Volume {
                    whole_disk: Some(disk),
                    ..
                } = &c.link
                {
                    let _hold = self.gear_hold(&link_handle(&c.link));
                    if (self.gear.unmount)(disk).is_ok() {
                        self.card_note_released(c);
                    }
                }
                self.gear_note_failure(c, step, message);
                self.gear_job_done(c, Some(step));
                Ok(())
            }
            None => self.gear_release_card(c).map_err(|e| format!("{e:#}")),
        }
    }

    /// The end of one job on a device: one cue, "<device> done, safe to unplug." or
    /// "<step> failed on <device>.". A done arms the "still inserted" reminder for a card.
    pub fn gear_job_done(&self, c: &Connected, failed_step: Option<&str>) -> bool {
        let s = self.gear_settings().cues;
        let name = connected_name(c);
        let now = Instant::now();
        let event = match failed_step {
            Some(step) => CueEvent::failed(step, name.clone()),
            None => {
                if matches!(c.link, Link::Volume { .. }) {
                    self.gear.cues.reminders.lock().unwrap().arm(
                        &link_handle(&c.link),
                        &name,
                        &s,
                        now,
                    );
                }
                CueEvent::new(Cue::SafeToUnplug, name)
            }
        };
        let fired = self
            .gear
            .cues
            .fire(&s, event, now, chrono::Local::now().time());
        // The reminder state changed: the GUI shows it.
        self.hooks.gear_changed();
        fired
    }

    /// The end of a batch or an automation run over several devices: one cue for all of
    /// them. `done` and `failed` name devices; `failed` pairs each with its step.
    pub fn gear_batch_done(&self, done: &[String], failed: &[(String, String)]) -> bool {
        let Some(e) = cues::batch_cue(done, failed) else {
            return false;
        };
        self.gear.cues.fire(
            &self.gear_settings().cues,
            e,
            Instant::now(),
            chrono::Local::now().time(),
        )
    }

    /// Stops the "still inserted" reminder for a device (the person dismissed it).
    pub fn gear_dismiss_reminder(&self, c: &Connected) {
        self.gear_dismiss(&link_handle(&c.link));
    }

    /// Stops the "still inserted" reminder for a link (`link_handle`). True when one was
    /// armed.
    pub fn gear_dismiss(&self, handle: &str) -> bool {
        let was = {
            let mut r = self.gear.cues.reminders.lock().unwrap();
            let was = r.is_armed(handle);
            r.dismiss(handle);
            was
        };
        if was {
            self.hooks.gear_changed();
        }
        was
    }

    /// For the app's poll: plays the "still inserted" reminders that are due. A reminder
    /// runs only after a job's "done" and ends when the device is removed.
    pub fn gear_play_reminders(&self, tracker: &Tracker, now: Instant, local: chrono::NaiveTime) {
        let s = self.gear_settings().cues;
        let present: Vec<String> = tracker
            .unmounted
            .iter()
            .chain(tracker.connected.iter())
            .map(|c| link_handle(&c.link))
            .collect();
        let due = self
            .gear
            .cues
            .reminders
            .lock()
            .unwrap()
            .due(&present, &s, now);
        for e in due {
            self.gear.cues.fire(&s, e, now, local);
        }
    }
}

/// How long a released hold still marks events as QuadCam's own: the poll may see the
/// release a little later.
pub const HOLD_GRACE: Duration = Duration::from_secs(6);

/// A job's hold on a device's link (see `Core::gear_hold`).
pub struct Hold<'a> {
    core: &'a Core,
    handle: String,
}

impl Drop for Hold<'_> {
    fn drop(&mut self) {
        self.core
            .gear_holds
            .lock()
            .unwrap()
            .insert(self.handle.clone(), Some(Instant::now() + HOLD_GRACE));
        self.core.hooks.gear_changed();
    }
}

/// The name a cue says for a connected device: its saved name, else its kind.
pub fn connected_name(c: &Connected) -> String {
    match &c.device {
        Some(d) => d.display_name(),
        None => format!("The {}", c.kind.label()),
    }
}

/// What stays the same for a device's link across a mount and an unmount: the whole disk
/// of a volume (its mount point when unknown), a serial port's path, DFU.
pub fn link_handle(link: &Link) -> String {
    match link {
        Link::Volume {
            whole_disk: Some(d),
            ..
        } => d.clone(),
        Link::Volume { mount, .. } => mount.display().to_string(),
        Link::Serial { port, .. } => port.clone(),
        Link::Dfu { vid, pid } => format!("dfu-{vid:04x}:{pid:04x}"),
    }
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
