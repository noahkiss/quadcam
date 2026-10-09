//! The sim's in-process host (sim design 2.4, S5): the S1 physics on its own thread, fed by the
//! radio through S4's input ring, and the newest state for the Sim page to draw.
//!
//! - `sim_start` builds the sim from a built-in profile and the plain room, subscribes the radio's
//!   unthrottled feed to an input ring, and starts the physics thread (`quadcam_sim::runner`).
//! - `sim_frame` is the page's per-frame read: the two newest steps' poses and the HUD values.
//!   The page interpolates between them (`app/src/views/Gear/Sim/host/loop.ts`). It never waits
//!   for the physics thread and the physics thread never waits for it.
//! - These are GUI commands, not `api` rows: a sim is flown by hand, so an agent never starts one
//!   (design 2.6).

use super::Core;
use crate::gear::radio_hid::{Feed, Subscription};
use anyhow::{bail, Context, Result};
use quadcam_sim::fc::{ArmBlock, FlightMode};
use quadcam_sim::input::{now_ns, Calibration, InputRing, LinkConfig, LinkModel};
use quadcam_sim::ring::{RadioSource, RcFrame, RcSource, FLAG_LINK_LOST};
use quadcam_sim::runner::{self, HostClock, Runner, RunnerConfig};
use quadcam_sim::snapshot::{snapshot_buffer, Snapshot, SnapshotReader};
use quadcam_sim::world::{Shape, WorldSpec};
use quadcam_sim::{Sim, SimSettings};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The preset a new sim flies when the page names none.
pub const DEFAULT_PRESET: &str = "meteor75";

/// The input ring holds this many reports (a radio sends about 250 a second).
const RING_CAPACITY: usize = 2048;

/// The seed of the one deterministic run; a fresh flight does not need a fresh seed.
const SEED: u64 = 1;

/// One built-in profile the Sim page offers.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SimPreset {
    pub id: String,
    pub label: String,
    /// Motor-to-motor diagonal (mm).
    pub wheelbase_mm: f64,
    /// All-up mass (g).
    pub mass_g: f64,
}

/// The Sim page's settings, the `simSettings` key of the settings file. A missing field reads
/// as its default; the page writes the whole object on each change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SimUiSettings {
    /// A built-in profile id; None: `meteor75`.
    #[serde(default)]
    pub profile: Option<String>,
    /// `fpv` (default) or `chase`.
    #[serde(default = "fpv")]
    pub view: String,
    /// `16:9`, `4:3`, or None for the profile's.
    #[serde(default)]
    pub aspect: Option<String>,
    #[serde(default = "yes")]
    pub stick_display: bool,
    #[serde(default = "yes")]
    pub osd: bool,
    /// Camera tilt and field of view; None: the profile's.
    #[serde(default)]
    pub uptilt_deg: Option<f64>,
    #[serde(default)]
    pub fov_deg: Option<f64>,
}

fn fpv() -> String {
    "fpv".into()
}

fn yes() -> bool {
    true
}

impl Default for SimUiSettings {
    fn default() -> Self {
        Self {
            profile: None,
            view: fpv(),
            aspect: None,
            stick_display: true,
            osd: true,
            uptilt_deg: None,
            fov_deg: None,
        }
    }
}

impl SimUiSettings {
    /// The settings file's check: the right shapes and ranges.
    pub fn check(&self) -> Result<()> {
        if !["fpv", "chase"].contains(&self.view.as_str()) {
            bail!("the view is fpv or chase");
        }
        if self.aspect.as_deref().is_some_and(|a| !["16:9", "4:3"].contains(&a)) {
            bail!("the aspect is 16:9 or 4:3");
        }
        if self.uptilt_deg.is_some_and(|u| !(0.0..=60.0).contains(&u)) {
            bail!("the camera tilt is 0 to 60 degrees");
        }
        if self.fov_deg.is_some_and(|f| !(60.0..=170.0).contains(&f)) {
            bail!("the field of view is 60 to 170 degrees");
        }
        Ok(())
    }
}

/// `sim_start`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct SimStartParams {
    /// A built-in profile id; default `meteor75`.
    #[serde(default)]
    pub profile: Option<String>,
    /// The radio's saved calibration (`gear_sim_calibration`); None: sticks only, in AETR.
    #[serde(default)]
    pub calibration: Option<Calibration>,
}

/// A box of the room, in the world frame (x, y horizontal, z up).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SimBox {
    pub centre: [f64; 3],
    pub half: [f64; 3],
    /// Rotation about the vertical axis (deg).
    pub yaw_deg: f64,
    /// `wall`, `floor`, `carpet`, `grass` or `gate`: the page colours by it.
    pub material: String,
}

/// What the page needs once to draw the sim.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SimStartInfo {
    pub profile: String,
    pub label: String,
    /// The physics step (s).
    pub dt: f64,
    pub world_name: String,
    pub boxes: Vec<SimBox>,
    /// The start pad: where a reset puts the quad.
    pub start: [f64; 3],
    pub start_yaw_deg: f64,
    pub camera: SimCamera,
    /// The frame, for the model the page draws.
    pub wheelbase_m: f64,
    pub body_half: [f64; 3],
    pub prop_radius_m: f64,
    /// The calibration's stick mode, 1 to 4.
    pub stick_mode: u8,
    /// A calibration reached the sim: the arm, turtle and reset controls work.
    pub calibrated: bool,
    /// The host clock now (ns): the page aligns its own clock to it.
    pub host_ns: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SimCamera {
    pub uptilt_deg: f64,
    pub fov_deg: f64,
    /// `4:3` or `16:9`.
    pub aspect: String,
    /// In the body frame (x forward, y left, z up), metres.
    pub position: [f64; 3],
}

/// One step's pose.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
pub struct SimPose {
    pub step: u64,
    /// Sim time (s).
    pub t: f64,
    /// The host time the step stands for (ns).
    pub host_ns: u64,
    /// The radio sample's arrival (ns); 0 before the first.
    pub input_ns: u64,
    pub pos: [f64; 3],
    /// Body to world (w, x, y, z).
    pub quat: [f64; 4],
}

/// The pilot's sticks after calibration: roll right, pitch forward and yaw right in -1..1,
/// throttle in 0..1.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
pub struct SimSticks {
    pub roll: f64,
    pub pitch: f64,
    pub yaw: f64,
    pub throttle: f64,
}

/// What the OSD and the stick display show.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SimHud {
    pub armed: bool,
    pub turtle: bool,
    pub airmode: bool,
    /// `acro`, `angle` or `horizon`.
    pub mode: String,
    /// Why the quad will not arm, in words; None when it can.
    pub arm_block: Option<String>,
    pub vbat: f64,
    pub mah: f64,
    pub low_battery: bool,
    pub sticks: SimSticks,
    /// The throttle the FC applied, 0..1.
    pub throttle: f64,
    /// Upside down or stopped with the motors idle: turtle or reset.
    pub stuck: bool,
    pub contacts: u32,
    pub dropped_steps: u64,
}

/// The page's read each frame.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SimFrame {
    /// The host clock when this was read (ns).
    pub host_ns: u64,
    pub prev: SimPose,
    pub cur: SimPose,
    pub hud: SimHud,
    /// The radio is plugged in and reporting.
    pub radio: bool,
}

/// What a stopped sim did.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SimStopInfo {
    pub steps: u64,
    pub dropped_steps: u64,
    /// The physics step's cost (µs): median and 99th percentile.
    pub step_p50_us: f64,
    pub step_p99_us: f64,
}

/// The running host.
pub(crate) struct Host {
    runner: Runner,
    reader: Mutex<SnapshotReader>,
    radio: Arc<AtomicBool>,
    _sub: Option<Subscription>,
}

impl Core {
    /// The built-in profiles, for the page's aircraft list.
    pub fn sim_presets(&self) -> Vec<SimPreset> {
        quadcam_sim::presets()
            .into_iter()
            .map(|p| SimPreset {
                wheelbase_mm: p.frame.wheelbase.value * 1000.0,
                mass_g: p.frame.mass.value * 1000.0,
                id: p.id,
                label: p.label,
            })
            .collect()
    }

    /// Starts the sim on a built-in profile in the plain room. A sim already running stops
    /// first.
    pub fn sim_start(&self, p: &SimStartParams) -> Result<SimStartInfo> {
        let mut slot = self.sim.host.lock().unwrap();
        // The old runner (and its radio subscription) go before the new ones start.
        *slot = None;
        let id = p.profile.as_deref().unwrap_or(DEFAULT_PRESET);
        let profile = quadcam_sim::preset(id).with_context(|| {
            let ids: Vec<String> = quadcam_sim::presets().into_iter().map(|p| p.id).collect();
            format!("No sim profile {id:?}. Built-in profiles: {}.", ids.join(", "))
        })?;
        let cal = p.calibration.clone();
        if let Some(c) = &cal {
            super::sim::check(c)?;
        }
        let calibrated = cal.is_some();
        let cal = cal.unwrap_or_default();
        let stick_mode = cal.mode;
        let world = WorldSpec::reference_room();
        let boxes = boxes_of(&world);
        let (start, start_yaw_deg, world_name) =
            (world.start, world.start_yaw_deg, world.name.clone());
        let sim = Sim::new(&profile, world, SimSettings::default(), SEED);

        let ring = Arc::new(InputRing::new(RING_CAPACITY));
        let radio = Arc::new(AtomicBool::new(false));
        let (r2, g2) = (ring.clone(), radio.clone());
        let sub = self.radio_subscribe(Arc::new(move |f: &Feed| match f {
            Feed::Sample(s) => {
                g2.store(true, Ordering::Relaxed);
                r2.push(*s);
            }
            Feed::Connected(_) => g2.store(true, Ordering::Relaxed),
            Feed::Gone => g2.store(false, Ordering::Relaxed),
        }))?;
        let source = HostSource {
            inner: RadioSource {
                ring,
                link: LinkModel::new(LinkConfig::default()),
                cal,
            },
            radio: radio.clone(),
        };
        let (writer, reader) = snapshot_buffer();
        let dt = sim.dt;
        let runner = runner::spawn(
            sim,
            source,
            writer,
            HostClock::new(),
            RunnerConfig::default(),
            None,
            Some(Box::new(|d| {
                eprintln!(
                    "sim: dropped {} steps ({:.0} ms behind)",
                    d.steps,
                    d.behind_ns as f64 / 1e6
                );
            })),
        );
        let cam = &profile.camera;
        let (body_half, wheelbase) = (
            profile.frame.body_half.map(|v| v.value),
            profile.frame.wheelbase.value,
        );
        *slot = Some(Host {
            runner,
            reader: Mutex::new(reader),
            radio,
            _sub: Some(sub),
        });
        Ok(SimStartInfo {
            profile: profile.id.clone(),
            label: profile.label.clone(),
            dt,
            world_name,
            boxes,
            start,
            start_yaw_deg,
            camera: SimCamera {
                uptilt_deg: cam.uptilt_deg.value,
                fov_deg: cam.fov_deg.value,
                aspect: cam.aspect.clone(),
                // On top of the frame, ahead of its centre. The real position comes with the
                // fitted profile (S3).
                position: [wheelbase * 0.2, 0.0, body_half[2]],
            },
            wheelbase_m: wheelbase,
            body_half,
            prop_radius_m: profile.prop.diameter.value / 2.0,
            stick_mode,
            calibrated,
            host_ns: now_ns(),
        })
    }

    /// The two newest steps and the HUD values. Errors when no sim runs.
    pub fn sim_frame(&self) -> Result<SimFrame> {
        let slot = self.sim.host.lock().unwrap();
        let Some(h) = slot.as_ref() else {
            bail!("The sim is not running.");
        };
        let pair = *h.reader.lock().unwrap().read();
        Ok(SimFrame {
            host_ns: now_ns(),
            prev: pose_of(&pair.prev),
            cur: pose_of(&pair.cur),
            hud: hud_of(&pair.cur),
            radio: h.radio.load(Ordering::Relaxed),
        })
    }

    /// Back to the start pad.
    pub fn sim_reset(&self) -> Result<()> {
        let slot = self.sim.host.lock().unwrap();
        let Some(h) = slot.as_ref() else {
            bail!("The sim is not running.");
        };
        h.runner.send(runner::Control::Reset);
        Ok(())
    }

    /// Stops the sim and says what it did. None when none ran.
    pub fn sim_stop(&self) -> Option<SimStopInfo> {
        let h = self.sim.host.lock().unwrap().take()?;
        let st = h.runner.stats();
        Some(SimStopInfo {
            steps: st.steps,
            dropped_steps: st.dropped_steps,
            step_p50_us: st.cost.quantile_us(0.5),
            step_p99_us: st.cost.quantile_us(0.99),
        })
    }
}

/// The radio, with the link marked lost while the radio is gone: a held last sample would
/// keep the sticks where they were.
pub(crate) struct HostSource<S: RcSource = RadioSource> {
    pub(crate) inner: S,
    pub(crate) radio: Arc<AtomicBool>,
}

impl<S: RcSource> RcSource for HostSource<S> {
    fn frame_at(&mut self, t_ns: u64) -> RcFrame {
        let mut f = self.inner.frame_at(t_ns);
        if !self.radio.load(Ordering::Relaxed) {
            f.flags |= FLAG_LINK_LOST;
        }
        f
    }
}

fn boxes_of(w: &WorldSpec) -> Vec<SimBox> {
    w.colliders
        .iter()
        .filter_map(|c| match &c.shape {
            Shape::Box { half } => Some(SimBox {
                centre: c.position,
                half: *half,
                yaw_deg: c.yaw_deg,
                material: format!("{:?}", c.material).to_lowercase(),
            }),
            Shape::Mesh { .. } => None,
        })
        .collect()
}

fn pose_of(s: &Snapshot) -> SimPose {
    SimPose {
        step: s.step,
        t: s.t,
        host_ns: s.host_ns,
        input_ns: s.input_ns,
        pos: s.pos,
        quat: s.quat,
    }
}

/// The OSD's words for an arming refusal (Betaflight's arming-disable flags, in plain text).
pub fn arm_block_text(b: ArmBlock) -> &'static str {
    match b {
        ArmBlock::ArmSwitchAtStart => "Arm switch is on: turn it off first",
        ArmBlock::ArmSwitch => "Turn the arm switch off and on again",
        ArmBlock::Throttle => "Throttle is up: lower it to arm",
        ArmBlock::Angle => "Level the quad to arm",
        ArmBlock::Failsafe => "No radio link",
        ArmBlock::NoArmRange => "This quad has no arm switch set",
    }
}

fn hud_of(s: &Snapshot) -> SimHud {
    SimHud {
        armed: s.armed,
        turtle: s.turtle,
        airmode: s.airmode,
        mode: match s.mode {
            FlightMode::Acro => "acro",
            FlightMode::Angle => "angle",
            FlightMode::Horizon => "horizon",
        }
        .into(),
        arm_block: s.arm_block.map(|b| arm_block_text(b).to_string()),
        vbat: s.vbat,
        mah: s.mah,
        low_battery: s.low_battery,
        sticks: SimSticks {
            roll: s.sticks.roll,
            pitch: s.sticks.pitch,
            yaw: s.sticks.yaw,
            throttle: s.sticks.throttle,
        },
        throttle: s.throttle,
        stuck: s.stuck,
        contacts: s.contacts,
        dropped_steps: s.dropped_steps,
    }
}
