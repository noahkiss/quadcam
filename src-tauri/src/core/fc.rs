//! `Core`'s flight-controller jobs, over `gear::bf`: identify (MSP), read (the CLI), run
//! CLI lines (for the apply engine), the USB heat timer, and the board notes.
//!
//! Every job is one job in the cue sense (`docs/gear-design.md` 7.11): it holds the port
//! (its own open and close are `app_initiated`, silent), opens it, works, drops the link
//! (QuadCam never holds an FC port idle), then plays one cue: "<FC> done, safe to unplug."
//! or "<step> failed on <FC>.". A job that reads the identity caches it per port, so the
//! next poll fills the serial device's id (`identified`).

use super::Core;
use crate::gear::bf::{self, boards, cli, FcInfo, FcRead};
use crate::gear::cues::{Cue, CueEvent};
use crate::gear::model::{Connected, DeviceKind, Link, Refusal, RefusalCode};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// `gear_fc_identify`: the FC's port; omitted when exactly one FC is plugged in.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct FcPortParams {
    #[serde(default)]
    pub port: Option<String>,
}

/// `gear_fc_read`: CLI commands that only read (`version`, `status`, `get NAME`,
/// `diff all`, `dump all`, ...). Empty: a backup's set (`version`, `status`, `diff all`,
/// `dump all`). The FC reboots when the read ends.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct FcReadParams {
    #[serde(default)]
    pub port: Option<String>,
    #[serde(default)]
    pub commands: Vec<String>,
}

/// A finished FC job's answer: what it read, and the board notes to show after it.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct FcJob<T> {
    pub result: T,
    /// Known issues of this board and build that a USB session triggers.
    pub notes: Vec<String>,
}

/// `gear_board_notes`: a board and version to filter by; both empty lists every note.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct BoardNotesParams {
    #[serde(default)]
    pub board: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

/// One FC's USB heat timer. It runs while the FC is on USB with its battery in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct UsbTimer {
    pub port: String,
    pub id: Option<String>,
    /// A battery is in (the FC reads more than 1 V on its battery lead).
    pub battery: bool,
    pub volts: Option<f32>,
    /// Seconds on USB with the battery in.
    pub elapsed_s: u32,
    /// The limit in seconds; None when the timer is off (`gearUsbMinutes` 0).
    pub limit_s: Option<u32>,
    /// Seconds left; 0 once past the limit.
    pub remaining_s: Option<u32>,
    /// "Unplug now" played for this battery session.
    pub warned: bool,
}

/// How often the USB timer reads the battery (one short MSP exchange; the port is closed
/// between reads).
pub const USB_PROBE: Duration = Duration::from_secs(30);

/// One port's USB timer state.
#[derive(Debug, Clone, Default)]
pub struct UsbState {
    battery_since: Option<Instant>,
    last_probe: Option<Instant>,
    volts: Option<f32>,
    warned: bool,
}

/// Per-port FC state the core keeps between jobs and polls.
#[derive(Default)]
pub struct FcState {
    /// What each port's FC said last (identity, id).
    pub seen: HashMap<String, FcInfo>,
    pub usb: HashMap<String, UsbState>,
}

impl Core {
    fn fc_timing(&self) -> cli::Timing {
        *self.fc_timing.lock().unwrap()
    }

    /// Replaces the CLI and MSP timing (tests pass `Timing::fast()`).
    pub fn with_fc_timing(self, t: cli::Timing) -> Core {
        *self.fc_timing.lock().unwrap() = t;
        self
    }

    /// The FC ports plugged in now (serial devices `detect` takes for FCs).
    fn fc_ports(&self) -> Vec<Connected> {
        self.gear
            .detect()
            .into_iter()
            .filter(|c| c.kind == DeviceKind::Fc && matches!(c.link, Link::Serial { .. }))
            .collect()
    }

    /// The port to use: the one asked for, or the only FC plugged in. Several and none
    /// asked refuses (design 6.1: ask, never guess).
    pub fn gear_fc_pick(&self, port: Option<&str>) -> Result<Connected> {
        let fcs = self.fc_ports();
        if let Some(p) = port.map(str::trim).filter(|p| !p.is_empty()) {
            if let Some(c) = fcs.iter().find(|c| super::gear::link_handle(&c.link) == p) {
                return Ok(c.clone());
            }
            bail!("No device: no FC on {p}. Plug USB in before the battery; check `gear status`.");
        }
        match fcs.len() {
            0 => bail!(
                "No device: no FC found. Plug USB in before the battery. If macOS asked to allow an accessory, click Allow."
            ),
            1 => Ok(fcs[0].clone()),
            _ => Err(Refusal::new(
                RefusalCode::SeveralDevices,
                format!(
                    "{} FCs are connected; pick one with port: {}.",
                    fcs.len(),
                    fcs.iter()
                        .map(|c| super::gear::link_handle(&c.link))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
            .into()),
        }
    }

    /// One FC job: hold the port, run `f`, cache the identity it read, one cue at the end.
    fn fc_job<T>(
        &self,
        port: Option<&str>,
        step: &str,
        f: impl FnOnce(&dyn crate::gear::serial::Ports, &str) -> Result<(FcInfo, T)>,
    ) -> Result<FcJob<T>> {
        let mut c = self.gear_fc_pick(port)?;
        let handle = super::gear::link_handle(&c.link);
        let out = {
            let _hold = self.gear_hold(&handle);
            f(self.gear.ports.as_ref(), &handle)
        };
        match out {
            Ok((info, result)) => {
                let notes = boards::after_job(
                    info.identity.board.as_deref(),
                    info.identity.version.as_deref(),
                );
                c.id = info.id.clone();
                c.identity = info.identity.clone();
                if let Some(id) = &info.id {
                    let store = self.gear_store();
                    if store.device(id)?.is_some() {
                        c.device = Some(store.seen(id, DeviceKind::Fc, &info.identity)?);
                        self.hooks.gear_changed();
                    }
                }
                self.fc_state.lock().unwrap().seen.insert(handle, info);
                self.gear_job_done(&c, None);
                Ok(FcJob { result, notes })
            }
            Err(e) => {
                // A refusal is an answer, not a failed step: no cue.
                let refused =
                    e.downcast_ref::<Refusal>().is_some() || format!("{e}").starts_with("Refused");
                if !refused {
                    self.gear_job_done(&c, Some(step));
                }
                Err(e)
            }
        }
    }

    /// Reads the FC's identity over MSP: no CLI, no reboot.
    pub fn gear_fc_identify(&self, p: &FcPortParams) -> Result<FcJob<FcInfo>> {
        let t = self.fc_timing();
        self.fc_job(p.port.as_deref(), "Identify", |ports, port| {
            let i = bf::identify(ports, port, t)?;
            Ok((i.clone(), i))
        })
    }

    /// Reads through the CLI (read-only commands; a backup's set by default). The FC
    /// reboots when the read ends. Writes nothing to the gear folder: the backup store
    /// (WP4) keeps what it reads.
    pub fn gear_fc_read(&self, p: &FcReadParams) -> Result<FcJob<FcRead>> {
        let t = self.fc_timing();
        let commands: Vec<String> = if p.commands.is_empty() {
            bf::BACKUP.iter().map(|s| s.to_string()).collect()
        } else {
            p.commands.iter().map(|c| c.trim().to_string()).collect()
        };
        self.fc_job(p.port.as_deref(), "Read", |ports, port| {
            let r = bf::read(ports, port, &commands, t)?;
            Ok((r.info.clone(), r))
        })
    }

    /// Writes CLI lines to an FC: the guard and the device check run again inside, right
    /// before the first write (`bf::run`). Only the apply engine calls this, after its plan,
    /// backup and confirm (design 8.1); it is not an `api` row.
    pub fn gear_fc_run(
        &self,
        port: Option<&str>,
        expect_id: Option<&str>,
        lines: &[String],
    ) -> Result<FcJob<cli::RunReport>> {
        let t = self.fc_timing();
        self.fc_job(port, "Apply", |ports, port| {
            bf::run(ports, port, expect_id, lines, t)
        })
    }

    /// What the last job read from the FC on `port`.
    pub fn gear_fc_seen(&self, port: &str) -> Option<FcInfo> {
        self.fc_state.lock().unwrap().seen.get(port).cloned()
    }

    /// Fills the id and identity of serial FCs that a job identified. Drops ports that
    /// are gone.
    pub(super) fn fc_fill(&self, found: &mut [Connected]) {
        let mut st = self.fc_state.lock().unwrap();
        let ports: Vec<String> = found
            .iter()
            .filter(|c| matches!(c.link, Link::Serial { .. }))
            .map(|c| super::gear::link_handle(&c.link))
            .collect();
        st.seen.retain(|p, _| ports.contains(p));
        st.usb.retain(|p, _| ports.contains(p));
        for c in found.iter_mut() {
            if let Some(i) = st.seen.get(&super::gear::link_handle(&c.link)) {
                if c.kind == DeviceKind::Fc {
                    c.id = i.id.clone();
                    c.identity = i.identity.clone();
                }
            }
        }
    }

    /// Board notes: every known issue, or those of one board and version.
    pub fn gear_board_notes(&self, p: &BoardNotesParams) -> Vec<boards::BoardNote> {
        let b = p.board.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let v = p
            .version
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        boards::notes(b, v)
    }

    /// For the app's poll: reads each FC's battery every `USB_PROBE` (a short MSP
    /// exchange, the port closed between reads; skipped while a job holds the port or
    /// another app has it open) and plays one "Unplug now" when an FC has run on USB with
    /// its battery in past its limit. A battery pulled ends the session and its warning.
    pub fn gear_usb_tick(&self, now: Instant) -> Vec<UsbTimer> {
        let settings = self.gear_settings();
        let t = self.fc_timing();
        for c in self.fc_ports() {
            let port = super::gear::link_handle(&c.link);
            let due = {
                let st = self.fc_state.lock().unwrap();
                st.usb
                    .get(&port)
                    .and_then(|u| u.last_probe)
                    .is_none_or(|l| now.duration_since(l) >= USB_PROBE)
            };
            if !due || self.held_by_job(&port) {
                continue;
            }
            let known = self.fc_state.lock().unwrap().seen.contains_key(&port);
            let volts = {
                let _hold = self.gear_hold(&port);
                // An FC not identified yet is identified first (MSP, no reboot), so its
                // board's limit applies and the next poll reports it `identified`.
                if !known {
                    if let Ok(i) = bf::identify(self.gear.ports.as_ref(), &port, t) {
                        self.fc_state.lock().unwrap().seen.insert(port.clone(), i);
                    }
                }
                bf::battery_volts(self.gear.ports.as_ref(), &port, t).ok()
            };
            let mut st = self.fc_state.lock().unwrap();
            let u = st.usb.entry(port.clone()).or_default();
            u.last_probe = Some(now);
            let Some(v) = volts else { continue };
            u.volts = Some(v);
            if v > bf::BATTERY_IN_VOLTS {
                u.battery_since.get_or_insert(now);
            } else {
                u.battery_since = None;
                u.warned = false;
            }
        }
        let timers = self.usb_timers(now);
        for tm in &timers {
            if tm.battery && !tm.warned && tm.remaining_s == Some(0) {
                if let Some(u) = self.fc_state.lock().unwrap().usb.get_mut(&tm.port) {
                    u.warned = true;
                }
                let c = self
                    .fc_ports()
                    .into_iter()
                    .find(|c| super::gear::link_handle(&c.link) == tm.port);
                let name = self
                    .gear_store()
                    .devices()
                    .ok()
                    .and_then(|ds| {
                        let id = tm.id.as_deref()?;
                        ds.into_iter().find(|d| d.id == id)
                    })
                    .map(|d| d.display_name())
                    .unwrap_or_else(|| {
                        c.map(|c| super::gear::connected_name(&c))
                            .unwrap_or("The FC".into())
                    });
                let mut e = CueEvent::new(Cue::UnplugNow, name);
                e.step = Some(format!("{} minutes", tm.elapsed_s / 60));
                self.gear
                    .cues
                    .fire(&settings.cues, e, now, chrono::Local::now().time());
            }
        }
        self.usb_timers(now)
    }

    fn usb_timers(&self, now: Instant) -> Vec<UsbTimer> {
        let minutes = self.gear_settings().usb_minutes;
        let st = self.fc_state.lock().unwrap();
        let mut out: Vec<UsbTimer> = st
            .usb
            .iter()
            .map(|(port, u)| {
                let info = st.seen.get(port);
                let limit_s =
                    boards::usb_limit(minutes, info.and_then(|i| i.identity.board.as_deref()))
                        .map(|m| m * 60);
                let elapsed_s = u
                    .battery_since
                    .map(|s| now.saturating_duration_since(s).as_secs() as u32)
                    .unwrap_or(0);
                UsbTimer {
                    port: port.clone(),
                    id: info.and_then(|i| i.id.clone()),
                    battery: u.battery_since.is_some(),
                    volts: u.volts,
                    elapsed_s,
                    limit_s,
                    remaining_s: limit_s.map(|l| l.saturating_sub(elapsed_s)),
                    warned: u.warned,
                }
            })
            .collect();
        out.sort_by(|a, b| a.port.cmp(&b.port));
        out
    }

    /// The USB timers now, without reading any FC.
    pub fn gear_usb_timers(&self) -> Vec<UsbTimer> {
        self.usb_timers(Instant::now())
    }
}
