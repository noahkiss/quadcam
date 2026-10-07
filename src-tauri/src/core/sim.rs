//! `Core`'s sim input (sim design 7): which radio the joystick is, its saved calibration,
//! the defaults an aircraft gives the sim, and the calibration session the screen drives.
//!
//! - `gear_sim_calibration` resolves the joystick to a Gear radio (`sim_cal::resolve`) and
//!   returns its calibration. `gear_sim_calibration_save` writes one.
//! - `gear_sim_defaults` reads the aircraft's switch map (its radio model and the quad's
//!   `aux` lines) and says what the sim pre-fills, with each value's source.
//! - `gear_sim_calibrate` runs the guided auto-calibration: a radio subscription feeds every
//!   report to `AutoCal`, and `sim-calibration` events (at most one per 16 ms) carry its
//!   state to the screen.

use super::Core;
use super::SwitchMapParams;
use crate::api::{Event, SimCalibrationEvent};
use crate::gear::model::DeviceKind;
use crate::gear::radio_hid::{self, Feed, Subscription};
use crate::gear::sim_cal::{self, RadioResolution, SavedCalibration, SimDefaults};
use anyhow::{bail, Result};
use quadcam_sim::input::{AutoCal, CalPhase, Calibration, CaptureTarget, StickFunction};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The events' pace while the radio streams.
const VIEW_INTERVAL: Duration = Duration::from_millis(16);

/// The running calibration session.
#[derive(Default)]
pub struct SimState {
    session: Mutex<Option<CalSession>>,
}

struct CalSession {
    live: Arc<Mutex<Live>>,
    _sub: Subscription,
}

struct Live {
    auto: AutoCal,
    connected: bool,
    sent: Option<Instant>,
}

/// `gear_sim_calibration`: a radio by key, or the one in USB Joystick mode now.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct SimCalibrationParams {
    /// A Gear radio id or a provisional `usb-…` key; omitted: the joystick plugged in.
    #[serde(default)]
    pub radio: Option<String>,
}

/// A radio's calibration, and which radio it is.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SimCalibration {
    /// How the joystick was matched to a radio; None when a radio was named.
    #[serde(default)]
    pub resolution: Option<RadioResolution>,
    /// The saved calibration; None: the screen opens to make one.
    #[serde(default)]
    pub calibration: Option<SavedCalibration>,
    /// The joystick's firmware version, when one is plugged in.
    #[serde(default)]
    pub firmware: Option<String>,
}

/// `gear_sim_calibration_save`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct SimCalibrationSaveParams {
    /// A Gear radio id, or a provisional `usb-…` key.
    pub radio: String,
    pub calibration: Calibration,
    #[serde(default)]
    pub product: Option<String>,
    #[serde(default)]
    pub firmware: Option<String>,
    /// Remember this radio as the answer to "Which radio is this?" for its product name.
    #[serde(default)]
    pub remember: bool,
    /// A provisional key this save replaces: linking that radio to a saved one.
    #[serde(default)]
    pub replaces: Option<String>,
}

/// `gear_sim_defaults`: an aircraft (its saved radio's and FC's latest backups), or files.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct SimDefaultsParams {
    #[serde(default)]
    pub aircraft: Option<String>,
    /// An EdgeTX card or one model file, as for the switch map.
    #[serde(default)]
    pub radio: Option<std::path::PathBuf>,
    /// Betaflight dump, diff or CLI files.
    #[serde(default)]
    pub fc: Vec<std::path::PathBuf>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CalibrateAction {
    /// Starts a session from `calibration` (at Review with `review`).
    Start,
    /// Next, or Done in the Move step.
    Advance,
    /// No arm switch, or no reset control.
    Skip,
    /// The full flow again: edited ends go, deadzones stay.
    Recalibrate,
    /// Sets one control (`target`) by moving it, then back to Review.
    Capture,
    /// Replaces the draft (an edit in Review, a mode or channel change).
    Set,
    /// Ends the session.
    Stop,
    /// The state now.
    #[default]
    Get,
}

/// `gear_sim_calibrate`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct CalibrateParams {
    pub action: CalibrateAction,
    /// Start and Set: the calibration (saved, or `SimDefaults::calibration`).
    #[serde(default)]
    pub calibration: Option<Calibration>,
    /// Start: the short check, for a radio whose model gives every stick's channel.
    #[serde(default)]
    pub quick: bool,
    /// Start: the arm switch is known (the quad's `aux` lines); the Arm step is skipped.
    #[serde(default)]
    pub arm_known: bool,
    /// Start: open at Review (a saved calibration).
    #[serde(default)]
    pub review: bool,
    /// Capture: the control to set.
    #[serde(default)]
    pub target: Option<CaptureTarget>,
}

/// Percent per stick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PerStick {
    pub roll: i16,
    pub pitch: i16,
    pub throttle: i16,
    pub yaw: i16,
}

/// The session as the screen draws it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct CalibrateView {
    pub active: bool,
    pub connected: bool,
    pub phase: CalPhase,
    /// What Capture sets.
    #[serde(default)]
    pub target: Option<CaptureTarget>,
    pub quick: bool,
    pub arm_known: bool,
    pub calibration: Calibration,
    /// Each stick's coverage, the shorter side, 0-100.
    pub coverage: PerStick,
    /// The calibrated output: roll, pitch, yaw −100..100, throttle 0..100.
    #[serde(default)]
    pub sticks: Option<PerStick>,
    /// Raw channels in µs, CH1 first.
    pub channels: Vec<u16>,
    pub buttons: u32,
    #[serde(default)]
    pub message: Option<String>,
}

fn view(l: &Live, active: bool) -> CalibrateView {
    let a = &l.auto;
    let pct = |v: f64| (v * 100.0).round() as i16;
    let cov = |f: StickFunction| pct(a.coverage(f));
    let out = a.latest().map(|s| a.cal.apply(s));
    CalibrateView {
        active,
        connected: l.connected,
        phase: a.phase,
        target: a.target,
        quick: a.quick,
        arm_known: a.arm_known,
        calibration: a.cal.clone(),
        coverage: PerStick {
            roll: cov(StickFunction::Roll),
            pitch: cov(StickFunction::Pitch),
            throttle: cov(StickFunction::Throttle),
            yaw: cov(StickFunction::Yaw),
        },
        sticks: out.map(|o| PerStick {
            roll: pct(o.roll),
            pitch: pct(o.pitch),
            throttle: pct(o.throttle),
            yaw: pct(o.yaw),
        }),
        channels: out.map(|o| o.channels.to_vec()).unwrap_or_default(),
        buttons: out.map(|o| o.buttons).unwrap_or(0),
        message: a.message.clone(),
    }
}

fn check(c: &Calibration) -> Result<()> {
    if !(1..=4).contains(&c.mode) {
        bail!("Stick mode is 1 to 4.");
    }
    for f in StickFunction::ALL {
        let a = c.axis(f);
        if !(1..=8).contains(&a.ch) {
            bail!("The {} channel must be CH1 to CH8.", f.name());
        }
        if a.deadzone > 50 {
            bail!("The {} deadzone is at most 50 %.", f.name());
        }
        if a.low() >= a.high() {
            bail!("The {} ends are the wrong way round.", f.name());
        }
    }
    let mut m = c.map().to_vec();
    m.sort();
    m.dedup();
    if m.len() < 4 {
        bail!("Each stick needs its own channel.");
    }
    Ok(())
}

impl Core {
    fn sim_calibrations(&self) -> Result<sim_cal::CalibrationFile> {
        sim_cal::read(self.gear_store().root())
    }

    /// A radio's calibration; with no radio named, the joystick plugged in, resolved to a
    /// saved radio. Reads only.
    pub fn gear_sim_calibration(&self, p: &SimCalibrationParams) -> Result<SimCalibration> {
        let f = self.sim_calibrations()?;
        if let Some(r) = p.radio.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
            return Ok(SimCalibration {
                resolution: None,
                calibration: f.radios.get(r).cloned(),
                firmware: None,
            });
        }
        let snap =
            radio_hid::snapshot(self.radio.hub.source().as_ref(), Duration::from_millis(300))?;
        let Some(product) = snap.product.filter(|_| snap.connected) else {
            bail!(
                "{}",
                snap.message
                    .unwrap_or_else(|| "No radio in USB Joystick mode.".into())
            );
        };
        let devices = self.gear_store().devices()?;
        let res = sim_cal::resolve(&product, radio_hid::VID, radio_hid::PID, &devices, &f);
        Ok(SimCalibration {
            calibration: res.radio.as_ref().and_then(|r| f.radios.get(r).cloned()),
            resolution: Some(res),
            firmware: snap.version,
        })
    }

    /// Saves a radio's calibration, keyed by its Gear radio id (or a provisional key).
    pub fn gear_sim_calibration_save(
        &self,
        p: &SimCalibrationSaveParams,
    ) -> Result<SavedCalibration> {
        check(&p.calibration)?;
        let radio = p.radio.trim();
        let provisional = radio.starts_with("usb-");
        if !provisional {
            let known = self
                .gear_store()
                .device(radio)?
                .is_some_and(|d| d.kind == DeviceKind::Radio);
            if !known {
                bail!("{radio:?} is not a saved radio. Pass a radio's id from `gear devices`, or the provisional key the calibration screen gave.");
            }
        }
        sim_cal::save(
            self.gear_store().root(),
            SavedCalibration {
                radio: radio.to_string(),
                provisional,
                calibration: p.calibration.clone(),
                product: p.product.clone(),
                vid: radio_hid::VID,
                pid: radio_hid::PID,
                firmware: p.firmware.clone(),
                saved_at: chrono::Utc::now(),
            },
            sim_cal::SaveOptions {
                remember: p.remember,
                replaces: p.replaces.as_deref(),
            },
        )
    }

    /// What the sim pre-fills for an aircraft: the stick channels from its radio model, the
    /// arm, angle, horizon, turtle and air mode switches from the quad's modes, a reset
    /// control. Reads only.
    pub fn gear_sim_defaults(&self, p: &SimDefaultsParams) -> Result<SimDefaults> {
        let aircraft = p
            .aircraft
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty());
        if aircraft.is_none() && p.radio.is_none() && p.fc.is_empty() {
            return Ok(sim_cal::defaults(None, None));
        }
        match self.gear_switch_map(&SwitchMapParams {
            aircraft: aircraft.map(str::to_string),
            radio: p.radio.clone(),
            fc: p.fc.clone(),
            ..Default::default()
        }) {
            Ok(m) => Ok(sim_cal::defaults(aircraft, Some(&m))),
            Err(e) => {
                let mut d = sim_cal::defaults(aircraft, None);
                d.notes.push(format!("{e}"));
                Ok(d)
            }
        }
    }

    /// Whether the radio's reader thread runs (the page stream or a sim sink).
    pub fn gear_radio_reading(&self) -> bool {
        self.radio.hub.running()
    }

    /// Drives the calibration session (`sim-calibration` events while it runs).
    pub fn gear_sim_calibrate(&self, p: &CalibrateParams) -> Result<CalibrateView> {
        let mut guard = self.sim.session.lock().unwrap();
        if p.action == CalibrateAction::Start {
            *guard = None;
            let start = p.calibration.clone().unwrap_or_default();
            check(&start)?;
            let auto = if p.review {
                AutoCal::review(start)
            } else {
                AutoCal::new(start, p.quick, p.arm_known)
            };
            let live = Arc::new(Mutex::new(Live {
                auto,
                connected: false,
                sent: None,
            }));
            let (l2, hooks) = (live.clone(), self.hooks.clone());
            let sub = self.radio.hub.subscribe(Arc::new(move |f: &Feed| {
                let mut l = l2.lock().unwrap();
                let before = l.auto.phase;
                match f {
                    Feed::Sample(s) => {
                        l.connected = true;
                        l.auto.feed(*s);
                    }
                    Feed::Connected(_) => l.connected = true,
                    Feed::Gone => l.connected = false,
                }
                let due = l.sent.is_none_or(|t| t.elapsed() >= VIEW_INTERVAL)
                    || l.auto.phase != before
                    || !matches!(f, Feed::Sample(_));
                if due {
                    l.sent = Some(Instant::now());
                    hooks.event(Event::SimCalibration(SimCalibrationEvent(view(&l, true))));
                }
            }))?;
            *guard = Some(CalSession { live, _sub: sub });
        }
        let Some(s) = guard.as_ref() else {
            if matches!(p.action, CalibrateAction::Stop | CalibrateAction::Get) {
                let l = Live {
                    auto: AutoCal::new(Calibration::default(), false, false),
                    connected: false,
                    sent: None,
                };
                return Ok(view(&l, false));
            }
            bail!("No calibration is running: start one first.");
        };
        let mut l = s.live.lock().unwrap();
        match p.action {
            CalibrateAction::Start | CalibrateAction::Get => {}
            CalibrateAction::Advance => {
                l.auto.advance();
            }
            CalibrateAction::Skip => l.auto.skip(),
            CalibrateAction::Recalibrate => l.auto.recalibrate(),
            CalibrateAction::Capture => {
                let Some(t) = p.target else {
                    bail!("Capture needs a target: arm, reset, turtle, angle, horizon or airmode.");
                };
                l.auto.capture(t);
            }
            CalibrateAction::Set => {
                let Some(c) = &p.calibration else {
                    bail!("Set needs a calibration.");
                };
                check(c)?;
                l.auto.cal = c.clone();
            }
            CalibrateAction::Stop => {}
        }
        let v = view(&l, p.action != CalibrateAction::Stop);
        drop(l);
        if p.action == CalibrateAction::Stop {
            *guard = None;
        }
        Ok(v)
    }
}
