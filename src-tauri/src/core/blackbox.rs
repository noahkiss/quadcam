//! `Core`'s blackbox jobs (design 7.12): pull an FC's flash, verify it, store it, erase it
//! when the setting and the checks allow, list, export, and erase by hand.
//!
//! - A pull is one FC job (`fc_job`): it holds the port, identifies the FC over MSP, reads
//!   the flash summary, reads only the used bytes, then verifies, stores and (maybe) erases.
//!   One cue plays at the end, so "safe to unplug" comes after the erase finished.
//! - Nothing is erased unless the image verified (its size is the used size, every log has
//!   a header, the stored blob reads back equal) and the person allowed it: the
//!   `gear_erase_blackbox` setting (a call can only turn it off for its run, `keep`). An erase that the USB heat timer
//!   could not let finish does not start.
//! - A pull refuses when it would outlast the USB heat timer (`force` overrides that and
//!   nothing else).
//! - The on-connect step skips a paused port and a port another program holds.

use super::{link_handle, Core, FcJob, HookFn, OnConnectHook, Skip};
use crate::gear::backup::BackupProgress;
use crate::gear::bf::blackbox::{self as bb, ImageCheck};
use crate::gear::bf::cli::{wait_for_port, CliSession, Timing, BAUD};
use crate::gear::bf::{self, FcInfo};
use crate::gear::blackbox::{self, Candidate, Linked, Method, Pull, Pulls};
use crate::gear::blobs::Blobs;
use crate::gear::model::{Connected, DeviceKind, Refusal, RefusalCode};
use crate::gear::serial::Ports;
use crate::gear::{Automation, GearSettings};
use anyhow::{anyhow, bail, Context, Result};
use chrono::{Local, Utc};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Seconds left on the USB timer that an erase must keep in hand beyond its estimate.
const ERASE_MARGIN_S: f64 = 10.0;

/// How a pull reads the flash.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PullMode {
    /// USB disk mode when the `gear_blackbox_msc` setting is on and the FC has it, else MSP.
    #[default]
    Auto,
    /// MSP only.
    Msp,
    /// USB disk mode only (unproven on real FCs); fails when the FC lacks it.
    Msc,
}

/// `gear_blackbox_pull`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct BlackboxPullParams {
    /// The FC's port; omitted when exactly one FC is plugged in.
    #[serde(default)]
    pub port: Option<String>,
    /// Keep the flash this run: do not erase it even when the `gear_erase_blackbox` setting
    /// is on. Nothing here turns the erase on; `gear_blackbox_erase` is its own call.
    #[serde(default)]
    pub keep: bool,
    #[serde(default)]
    pub mode: Option<PullMode>,
    /// Pull although the USB heat timer says the read would outlast it. An erase that
    /// could not finish still does not start.
    #[serde(default)]
    pub force: bool,
}

/// `gear_blackbox`: one device's pulls, or every device's.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct BlackboxFilter {
    #[serde(default)]
    pub device: Option<String>,
}

/// `gear_blackbox_export`: a pull to a folder.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct BlackboxExportParams {
    pub id: String,
    pub to: PathBuf,
    /// Also write each log as its own file.
    #[serde(default)]
    pub split: bool,
}

/// `gear_blackbox_erase`: erase the FC's flash by hand.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct BlackboxEraseParams {
    #[serde(default)]
    pub port: Option<String>,
    /// Required. The erase deletes the FC's logs for good.
    #[serde(default)]
    pub confirm: bool,
}

/// A pull with its guessed flights.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BlackboxEntry {
    pub pull: Pull,
    pub flights: Linked,
}

/// What a pull did about the erase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EraseState {
    /// Not asked for.
    Off,
    /// The flash was erased and reads empty.
    Done,
    /// Asked for and not done; `erase_note` says why. The pull itself is stored.
    Skipped,
}

/// `gear_blackbox_pull`'s answer.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BlackboxPullResult {
    /// None when the flash was empty.
    pub pull: Option<Pull>,
    /// False when the stored pull already holds these bytes.
    pub new: bool,
    pub method: Option<Method>,
    pub read_bytes: u64,
    pub read_secs: f64,
    pub erase: EraseState,
    pub erase_note: Option<String>,
    pub erase_secs: Option<f64>,
    pub notes: Vec<String>,
}

/// `gear_blackbox_export`'s answer.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BlackboxExported {
    pub files: Vec<PathBuf>,
    pub bytes: u64,
}

/// `gear_blackbox_erase`'s answer.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BlackboxErased {
    /// The stored pull that holds what was erased.
    pub pull: String,
    pub secs: f64,
}

/// The on-connect step "Blackbox": pull the flash of an FC, and erase it when the setting
/// allows. Skips a paused port.
pub fn blackbox_hooks() -> Vec<OnConnectHook> {
    let run: HookFn = Arc::new(|core: &Core, c: &Connected| {
        let port = link_handle(&c.link);
        if core.gear_poll_paused(&port) {
            return Err(Skip.into());
        }
        core.gear_blackbox_pull(&BlackboxPullParams {
            port: Some(port),
            ..Default::default()
        })
        .map(|_| ())
    });
    vec![OnConnectHook {
        name: "Blackbox",
        automation: Automation::Blackbox,
        kinds: vec![DeviceKind::Fc],
        run,
    }]
}

fn minutes(s: f64) -> String {
    if s >= 90.0 {
        format!("{:.1} min", s / 60.0)
    } else {
        format!("{s:.0} s")
    }
}

/// Does a read of `read_s` fit in the USB time `remaining_s` (None: no timer)?
fn fits(read_s: f64, remaining_s: Option<u32>) -> bool {
    remaining_s.is_none_or(|r| read_s <= r as f64)
}

/// Does an erase of `erase_s` fit, with a margin?
fn erase_fits(erase_s: f64, remaining_s: Option<u32>) -> bool {
    remaining_s.is_none_or(|r| erase_s + ERASE_MARGIN_S <= r as f64)
}

struct PullArgs<'a> {
    timing: Timing,
    settings: &'a GearSettings,
    erase: bool,
    mode: PullMode,
    force: bool,
}

/// What a read through USB disk mode gave.
struct MscRead {
    image: Vec<u8>,
    /// The port came back after the disk was released.
    back: bool,
}

impl Core {
    fn pulls(&self) -> Pulls {
        Pulls::new(self.gear_store())
    }

    /// The seconds left on this port's USB timer; None when no battery session runs or the
    /// timer is off.
    fn usb_remaining(&self, port: &str) -> Option<u32> {
        self.gear_usb_timers()
            .into_iter()
            .find(|t| t.port == port && t.battery)
            .and_then(|t| t.remaining_s)
    }

    /// Pulls the FC's blackbox flash: verify, store, and erase when allowed. One cue at the
    /// end, after the erase finished.
    pub fn gear_blackbox_pull(&self, p: &BlackboxPullParams) -> Result<FcJob<BlackboxPullResult>> {
        let settings = self.gear_settings();
        let a = PullArgs {
            timing: self.fc_timing(),
            erase: settings.erase_blackbox && !p.keep,
            mode: p.mode.unwrap_or_default(),
            force: p.force,
            settings: &settings,
        };
        self.fc_job(p.port.as_deref(), "Blackbox", |ports, port| {
            self.blackbox_pull_inner(ports, port, &a)
        })
    }

    fn blackbox_pull_inner(
        &self,
        ports: &dyn Ports,
        port: &str,
        a: &PullArgs,
    ) -> Result<(FcInfo, BlackboxPullResult)> {
        let t = a.timing;
        if let Some((_, name)) = (self.gear.holders)(port).into_iter().next() {
            return Err(Refusal::new(
                RefusalCode::PortBusy,
                format!("{port} is open in {name}. Close it there; QuadCam does not share a port."),
            )
            .into());
        }
        let info = bf::identify(ports, port, t)?;
        if info.identity.firmware.as_deref() != Some("Betaflight") {
            return Err(Refusal::new(
                RefusalCode::UnknownVersion,
                "QuadCam pulls the blackbox of Betaflight FCs only.",
            )
            .into());
        }
        let id = info.id.clone().context(
            "This FC gave no stable id (no MCU id and no USB serial), so QuadCam cannot keep its blackbox logs apart.",
        )?;
        if info.msp_api.as_deref().and_then(bb::api_minor) < Some(bb::MIN_API_MINOR) {
            bail!(
                "This FC's MSP API ({}) is older than 1.{}: QuadCam cannot read its blackbox flash.",
                info.msp_api.as_deref().unwrap_or("unknown"),
                bb::MIN_API_MINOR
            );
        }
        let (_job, stop) = self.job_start(port, Some(&id), "Pulling blackbox")?;
        let mut link = ports.open(port, BAUD)?;
        let sum = bb::summary(link.as_mut(), t.msp).context(
            "The FC did not answer the blackbox flash summary: it may log to an SD card, which QuadCam does not read.",
        )?;
        if !sum.supported || sum.total == 0 {
            bail!("This FC has no blackbox flash chip (it may log to an SD card or not at all).");
        }
        if !sum.ready {
            // Wait a moment: a flash that was just erased or is busy.
            let wait = Instant::now();
            while !bb::summary(link.as_mut(), t.msp)?.ready {
                if wait.elapsed() > t.reboot {
                    bail!("The blackbox flash is busy; try again in a minute.");
                }
                std::thread::sleep(t.poll);
            }
        }
        let sum = bb::summary(link.as_mut(), t.msp)?;
        let used = sum.used as u64;
        let mut res = BlackboxPullResult {
            pull: None,
            new: false,
            method: None,
            read_bytes: 0,
            read_secs: 0.0,
            erase: EraseState::Off,
            erase_note: None,
            erase_secs: None,
            notes: Vec::new(),
        };
        if used == 0 {
            res.notes.push("The blackbox flash is empty; nothing to pull.".into());
            return Ok((info, res));
        }

        // The USB heat timer: a pull that cannot finish is not started.
        let read_s = bb::read_seconds(used);
        let erase_s = bb::erase_seconds(sum.total as u64);
        let remaining = self.usb_remaining(port);
        if !fits(read_s, remaining) && !a.force {
            return Err(Refusal::new(
                RefusalCode::UsbHeat,
                format!(
                    "Reading {} KB takes about {} over MSP, and this FC has {} of USB time left. Unplug the battery and let it cool, then pull again (or force the pull).",
                    used / 1024,
                    minutes(read_s),
                    minutes(remaining.unwrap_or(0) as f64)
                ),
            )
            .into());
        }
        if !fits(read_s, remaining) {
            res.notes.push("Pulled although the USB timer is short (forced).".into());
        }

        // Read.
        let started = Instant::now();
        let mut method = Method::Msp;
        let mut image: Option<Vec<u8>> = None;
        let mut back = true;
        let use_msc = a.mode == PullMode::Msc || (a.mode == PullMode::Auto && a.settings.blackbox_msc);
        if use_msc {
            drop(link);
            match self.blackbox_msc_read(ports, port, t) {
                Ok(Some(m)) => {
                    method = Method::Msc;
                    back = m.back;
                    image = Some(m.image);
                }
                Ok(None) if a.mode == PullMode::Msc => {
                    bail!("This FC has no USB disk mode (its CLI does not list `msc`).")
                }
                Ok(None) => res
                    .notes
                    .push("This FC has no USB disk mode; read over MSP.".into()),
                Err(e) if a.mode == PullMode::Msc => return Err(e),
                Err(e) => res
                    .notes
                    .push(format!("USB disk mode failed ({e:#}); read over MSP.")),
            }
            link = wait_for_port(ports, port, t).context(
                "The FC did not come back after USB disk mode. Unplug USB, plug it in again and pull again.",
            )?;
        }
        let image = match image {
            Some(i) => i,
            None => {
                let progress = |done: u64| {
                    self.job_progress(
                        port,
                        &BackupProgress {
                            stage: "reading".into(),
                            files_done: 0,
                            files_total: 1,
                            bytes_done: done,
                            bytes_total: used,
                            path: "blackbox flash".into(),
                        },
                    );
                    !stop.load(Ordering::SeqCst)
                };
                let mut progress = progress;
                bb::read_used(link.as_mut(), sum.used, t.msp, &mut progress)?
            }
        };
        res.read_secs = started.elapsed().as_secs_f64();
        res.read_bytes = image.len() as u64;
        res.method = Some(method);

        // Verify, store, read back.
        let ImageCheck { logs, problems } = bb::check_image(&image, used);
        if !problems.is_empty() {
            bail!(
                "The blackbox read did not verify, so nothing was stored or erased: {}",
                problems.join(" ")
            );
        }
        self.job_step(port, "Storing blackbox");
        let store = self.gear_store();
        let blobs = Blobs::new(store.clone());
        let pulls = self.pulls();
        let pull = {
            let _lock = blobs.lock()?;
            let r = blobs.put(&image)?;
            if blobs.get(&r)? != image {
                bail!("The stored blackbox does not read back equal, so nothing was erased.");
            }
            let device = store.seen(&id, DeviceKind::Fc, &info.identity)?;
            match pulls.latest(&id).filter(|l| l.blob == r && !l.erased) {
                Some(same) => {
                    res.notes
                        .push("These bytes are already stored; no new record.".into());
                    same
                }
                None => {
                    let now = Utc::now();
                    let new = Pull {
                        id: format!("{id}/{}", blackbox::stamp(now)),
                        device: id.clone(),
                        aircraft: device.aircraft.clone(),
                        pulled_at: now,
                        day: now.with_timezone(&Local).date_naive(),
                        method,
                        blob: r,
                        used,
                        total: sum.total as u64,
                        firmware: logs.first().and_then(|l| l.firmware.clone()),
                        craft: logs.iter().find_map(|l| l.craft.clone()),
                        logs,
                        erased: false,
                        erase_note: None,
                    };
                    pulls.save(&new)?;
                    res.new = true;
                    new
                }
            }
        };
        self.hooks.gear_changed();

        // Erase, only now.
        let mut pull = pull;
        if a.erase {
            self.job_step(port, "Erasing blackbox");
            match self.blackbox_erase_after(link, &mut pull, erase_s, back, port, t) {
                Ok((state, note, secs)) => {
                    res.erase = state;
                    res.erase_note = note;
                    res.erase_secs = secs;
                }
                Err(e) => {
                    pull.erase_note = Some(format!("{e:#}"));
                    let _ = pulls.save(&pull);
                    return Err(e.context(
                        "The blackbox is stored, but the erase did not finish; the FC's flash may still hold logs",
                    ));
                }
            }
            pulls.save(&pull)?;
            self.hooks.gear_changed();
        }
        res.pull = Some(pull);
        Ok((info, res))
    }

    /// The erase after a verified pull. The flash must not have grown since the read, and
    /// the USB timer must leave time for the erase.
    fn blackbox_erase_after(
        &self,
        mut link: Box<dyn crate::gear::serial::SerialLink>,
        pull: &mut Pull,
        erase_s: f64,
        back: bool,
        port: &str,
        t: Timing,
    ) -> Result<(EraseState, Option<String>, Option<f64>)> {
        let skip = |pull: &mut Pull, why: String| {
            pull.erase_note = Some(why.clone());
            Ok((EraseState::Skipped, Some(why), None))
        };
        if !back {
            return skip(
                pull,
                "The FC did not return to serial after USB disk mode, so it was not erased. Unplug USB, plug it in again and erase by hand.".into(),
            );
        }
        let remaining = self.usb_remaining(port);
        if !erase_fits(erase_s, remaining) {
            return skip(
                pull,
                format!(
                    "Not erased: an erase takes about {} and the USB timer has {} left. Let the quad cool, then erase by hand.",
                    minutes(erase_s),
                    minutes(remaining.unwrap_or(0) as f64)
                ),
            );
        }
        let now = bb::summary(link.as_mut(), t.msp)?;
        if now.used as u64 != pull.used {
            return skip(
                pull,
                format!(
                    "Not erased: the flash holds {} bytes now, not the {} that were stored.",
                    now.used, pull.used
                ),
            );
        }
        bb::erase(link.as_mut(), t.msp)?;
        let limit = Duration::from_secs_f64(erase_s * 2.0).min(t.erase_max);
        let secs = bb::wait_erased(link.as_mut(), t.msp, limit, t.poll.max(Duration::from_millis(1)))?;
        pull.erased = true;
        pull.erase_note = None;
        Ok((EraseState::Done, None, Some(secs)))
    }

    /// Reads the flash through the FC's USB disk mode: enter the CLI, check `msc` is listed,
    /// send it, wait for a new volume with `.bbl` files, copy them in name order, release the
    /// disk. None when the FC has no `msc`. Needs a real-FC trial: the file names and layout
    /// the FC shows are not proven.
    fn blackbox_msc_read(&self, ports: &dyn Ports, port: &str, t: Timing) -> Result<Option<MscRead>> {
        let before: HashSet<PathBuf> = (self.gear.volumes)().into_iter().map(|v| v.mount).collect();
        let (mut s, _) = CliSession::enter(ports.open(port, BAUD)?, t)?;
        let help = s.command("help")?;
        if !help.text.split_whitespace().any(|w| w == "msc") {
            s.exit();
            return Ok(None);
        }
        s.msc()?;
        let deadline = Instant::now() + t.reboot;
        let vol = loop {
            if let Some(v) = (self.gear.volumes)()
                .into_iter()
                .find(|v| !before.contains(&v.mount) && !bbl_files(&v.mount).is_empty())
            {
                break v;
            }
            if Instant::now() >= deadline {
                bail!("No blackbox disk appeared after `msc`.");
            }
            std::thread::sleep(t.poll);
        };
        let mut image = Vec::new();
        for f in bbl_files(&vol.mount) {
            image.extend(std::fs::read(&f).with_context(|| format!("reading {}", f.display()))?);
        }
        let released = (self.gear.unmount)(&vol.info.parent_whole_disk).is_ok();
        let back = released && wait_for_port(ports, port, t).map(drop).is_ok();
        Ok(Some(MscRead { image, back }))
    }

    /// Pulls with their guessed flights, newest first.
    pub fn gear_blackbox(&self, f: &BlackboxFilter) -> Result<Vec<BlackboxEntry>> {
        let pulls = self.pulls();
        let mut list = match f.device.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
            Some(d) => pulls.list(d),
            None => pulls.all(),
        };
        list.reverse();
        let all = pulls.all();
        list.into_iter()
            .map(|p| {
                let flights = self.blackbox_flights(&p, &all)?;
                Ok(BlackboxEntry { pull: p, flights })
            })
            .collect()
    }

    /// The pairing of a pull's logs with the flights of its aircraft since the device's
    /// last erased pull, up to the pull. A guess (`blackbox::link`).
    fn blackbox_flights(&self, pull: &Pull, all: &[Pull]) -> Result<Linked> {
        let Some(aircraft) = pull.aircraft.clone() else {
            let mut l = blackbox::link(&pull.logs, &[]);
            l.note = "No aircraft is linked to this FC (gear devices), so its logs are not paired with flights.".into();
            return Ok(l);
        };
        let to_local = |t: chrono::DateTime<Utc>| t.with_timezone(&Local).naive_local();
        let end = to_local(pull.pulled_at);
        let since = all
            .iter()
            .filter(|p| p.device == pull.device && p.erased && p.pulled_at < pull.pulled_at)
            .map(|p| to_local(p.pulled_at))
            .max();
        let view = self.gear_flights(&super::FlightFilter {
            aircraft: Some(aircraft),
            ..Default::default()
        })?;
        // The view is newest first; the pairing wants oldest first.
        let mut flights: Vec<Candidate> = view
            .flights
            .iter()
            .filter(|r| r.flight.end <= end && since.is_none_or(|s| r.flight.start > s))
            .map(|r| Candidate {
                id: r.flight.id.clone(),
                secs: r.flight.secs,
            })
            .collect();
        flights.reverse();
        Ok(blackbox::link(&pull.logs, &flights))
    }

    /// Writes a pull's image (and with `split`, each log) to a folder. Never overwrites.
    pub fn gear_blackbox_export(&self, p: &BlackboxExportParams) -> Result<BlackboxExported> {
        let pull = self.pulls().get(p.id.trim())?;
        let image = Blobs::new(self.gear_store()).get(&pull.blob)?;
        std::fs::create_dir_all(&p.to)
            .with_context(|| format!("creating {}", p.to.display()))?;
        let stem = blackbox::export_stem(&pull);
        let mut files = Vec::new();
        let mut write = |name: String, bytes: &[u8]| -> Result<()> {
            let path = p.to.join(name);
            let mut f = std::fs::File::options()
                .write(true)
                .create_new(true)
                .open(&path)
                .with_context(|| format!("{} already exists or cannot be written", path.display()))?;
            std::io::Write::write_all(&mut f, bytes)?;
            files.push(path);
            Ok(())
        };
        write(format!("{stem}.bbl"), &image)?;
        if p.split {
            for l in &pull.logs {
                let (a, b) = (l.offset as usize, (l.offset + l.size) as usize);
                let part = image
                    .get(a..b)
                    .ok_or_else(|| anyhow!("log {} lies outside the stored image", l.index))?;
                write(format!("{stem}_log{:02}.bbl", l.index), part)?;
            }
        }
        Ok(BlackboxExported {
            bytes: files
                .iter()
                .filter_map(|f| std::fs::metadata(f).ok())
                .map(|m| m.len())
                .sum(),
            files,
        })
    }

    /// Erases the FC's flash by hand. It must hold exactly what the device's latest stored
    /// pull holds (the same used size: the flash only grows), and `confirm` must be true.
    pub fn gear_blackbox_erase(&self, p: &BlackboxEraseParams) -> Result<FcJob<BlackboxErased>> {
        if !p.confirm {
            bail!("Refused: erasing the blackbox deletes the FC's logs for good. Pass confirm=true to go ahead.");
        }
        let t = self.fc_timing();
        self.fc_job(p.port.as_deref(), "Blackbox erase", |ports, port| {
            if let Some((_, name)) = (self.gear.holders)(port).into_iter().next() {
                return Err(Refusal::new(
                    RefusalCode::PortBusy,
                    format!("{port} is open in {name}. Close it there; QuadCam does not share a port."),
                )
                .into());
            }
            let info = bf::identify(ports, port, t)?;
            let id = info
                .id
                .clone()
                .context("This FC gave no stable id, so QuadCam cannot find its stored blackbox.")?;
            let mut link = ports.open(port, BAUD)?;
            let sum = bb::summary(link.as_mut(), t.msp)?;
            let pulls = self.pulls();
            let latest = pulls.latest(&id).filter(|l| !l.erased);
            let Some(mut pull) = latest.filter(|l| l.used == sum.used as u64 && sum.used > 0) else {
                if sum.used == 0 {
                    bail!("The blackbox flash is already empty.");
                }
                bail!(
                    "Refused: the flash holds {} bytes and QuadCam has no stored pull of exactly that. Pull first.",
                    sum.used
                );
            };
            let blobs = Blobs::new(self.gear_store());
            if !blobs.verify(&pull.blob) {
                bail!("Refused: the stored pull {} does not read back; pull again first.", pull.id);
            }
            let erase_s = bb::erase_seconds(sum.total as u64);
            let remaining = self.usb_remaining(port);
            if !erase_fits(erase_s, remaining) {
                return Err(Refusal::new(
                    RefusalCode::UsbHeat,
                    format!(
                        "An erase takes about {} and the USB timer has {} left. Let the quad cool first.",
                        minutes(erase_s),
                        minutes(remaining.unwrap_or(0) as f64)
                    ),
                )
                .into());
            }
            bb::erase(link.as_mut(), t.msp)?;
            let limit = Duration::from_secs_f64(erase_s * 2.0).min(t.erase_max);
            let secs = bb::wait_erased(link.as_mut(), t.msp, limit, t.poll.max(Duration::from_millis(1)))?;
            pull.erased = true;
            pull.erase_note = None;
            pulls.save(&pull)?;
            self.hooks.gear_changed();
            Ok((info, BlackboxErased { pull: pull.id, secs }))
        })
    }
}

/// `.bbl` and `.bfl` files in a folder and its direct subfolders, by name.
fn bbl_files(dir: &std::path::Path) -> Vec<PathBuf> {
    fn here(dir: &std::path::Path, out: &mut Vec<PathBuf>, depth: u32) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() && depth == 0 {
                here(&p, out, 1);
            } else if p.extension().is_some_and(|x| {
                x.eq_ignore_ascii_case("bbl") || x.eq_ignore_ascii_case("bfl")
            }) && !e.file_name().to_string_lossy().starts_with('.')
            {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    here(dir, &mut out, 0);
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_fits() {
        assert!(fits(200.0, None), "no timer");
        assert!(fits(200.0, Some(200)));
        assert!(!fits(200.0, Some(199)));
        assert!(erase_fits(64.0, Some(74)));
        assert!(!erase_fits(64.0, Some(73)));
        assert!(erase_fits(64.0, None));
        assert_eq!(minutes(30.0), "30 s");
        assert_eq!(minutes(120.0), "2.0 min");
    }

    #[test]
    fn bbl_files_by_name_one_level_down() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("BTFL_002.BBL"), b"b").unwrap();
        std::fs::write(d.path().join("BTFL_001.bbl"), b"a").unwrap();
        std::fs::write(d.path().join(".BTFL_000.BBL"), b"x").unwrap();
        std::fs::write(d.path().join("readme.txt"), b"x").unwrap();
        std::fs::create_dir(d.path().join("LOGS")).unwrap();
        std::fs::write(d.path().join("LOGS").join("BTFL_003.BFL"), b"c").unwrap();
        let names: Vec<String> = bbl_files(d.path())
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, ["BTFL_001.bbl", "BTFL_002.BBL", "BTFL_003.BFL"]);
    }
}
