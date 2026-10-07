//! Input and calibration (sim design 7): the radio's samples, the stick map, each axis'
//! ends, centre, deadzone and Reverse, the guided auto-calibration, and the link model.
//!
//! - A sample is one USB joystick report: 8 axes of 0..2048 (axis i is CH(i+1), the
//!   radio's mixer output) and 24 buttons, stamped with `now_ns` when it arrived.
//! - `InputRing` carries samples from the HID thread to the physics thread without a lock.
//!   The physics thread takes, per step, the newest sample at or before the step's time.
//! - `LinkModel` sits between the ring and the flight controller, outside the
//!   deterministic core: the core consumes (and a recording stores) its output.
//! - `Calibration::apply` turns a sample into stick values: roll, pitch and yaw −1..1,
//!   throttle 0..1, with the deadzone cut from the middle and the rest rescaled.
//!
//! The auto-calibration's behaviour (per-side coverage, steady centres, the arm switch by
//! change) follows the spike's port of propwash's radio calibration (MIT), rewritten here.

use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::atomic::{fence, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

/// Axes in a report.
pub const AXES: usize = 8;
/// Buttons in a report.
pub const BUTTONS: usize = 24;
/// An axis' top value; 1024 is the middle.
pub const AXIS_MAX: u16 = 2048;
pub const AXIS_MID: u16 = 1024;

/// Nanoseconds on the sim's one monotonic clock (`mach_absolute_time` on macOS, through
/// `Instant`). The HID thread stamps samples with it and the physics thread steps on it.
pub fn now_ns() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

/// An axis value in µs: 0 is 988, 1024 is 1500, 2048 is 2012.
pub fn axis_us(raw: u16) -> u16 {
    988 + raw.min(AXIS_MAX) / 2
}

/// One report as it arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputSample {
    /// `now_ns` at arrival.
    pub t_ns: u64,
    /// Raw 0..2048, CH1 first.
    pub axes: [u16; AXES],
    /// Bit i is button i+1.
    pub buttons: u32,
}

impl InputSample {
    pub fn us(&self, ch: u8) -> Option<u16> {
        let i = (ch as usize).checked_sub(1)?;
        self.axes.get(i).map(|a| axis_us(*a))
    }
    pub fn button(&self, b: u8) -> bool {
        (1..=BUTTONS as u8).contains(&b) && (self.buttons >> (b - 1)) & 1 == 1
    }
}

// ----- the ring -----

struct Slot {
    /// Odd while being written; `2 * (n + 1)` once sample n is in.
    seq: AtomicU64,
    t: AtomicU64,
    buttons: AtomicU64,
    a: [AtomicU64; 2],
}

/// Samples from the HID thread (one writer) to any number of readers, lock-free. A slot is
/// a seqlock, so a reader never sees a half-written sample.
pub struct InputRing {
    slots: Box<[Slot]>,
    /// Samples pushed so far.
    head: AtomicU64,
}

impl InputRing {
    /// Room for `capacity` samples (rounded up to a power of two). At 1 kHz, 1024 is a
    /// second of history.
    pub fn new(capacity: usize) -> Self {
        let n = capacity.max(2).next_power_of_two();
        let slots = (0..n)
            .map(|_| Slot {
                seq: AtomicU64::new(0),
                t: AtomicU64::new(0),
                buttons: AtomicU64::new(0),
                a: [AtomicU64::new(0), AtomicU64::new(0)],
            })
            .collect();
        Self {
            slots,
            head: AtomicU64::new(0),
        }
    }

    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Samples pushed since the ring was made.
    pub fn pushed(&self) -> u64 {
        self.head.load(Ordering::Acquire)
    }

    /// Adds a sample. One thread writes; two writers would interleave.
    pub fn push(&self, s: InputSample) {
        let n = self.head.load(Ordering::Relaxed);
        let slot = &self.slots[(n as usize) & (self.slots.len() - 1)];
        slot.seq.store(2 * n + 1, Ordering::Relaxed);
        fence(Ordering::Release);
        slot.t.store(s.t_ns, Ordering::Relaxed);
        slot.buttons.store(s.buttons as u64, Ordering::Relaxed);
        for (k, a) in slot.a.iter().enumerate() {
            let mut w = 0u64;
            for j in 0..4 {
                w |= (s.axes[k * 4 + j] as u64) << (16 * j);
            }
            a.store(w, Ordering::Relaxed);
        }
        slot.seq.store(2 * (n + 1), Ordering::Release);
        self.head.store(n + 1, Ordering::Release);
    }

    /// Sample n, if the ring still holds it.
    fn get(&self, n: u64) -> Option<InputSample> {
        let slot = &self.slots[(n as usize) & (self.slots.len() - 1)];
        let s1 = slot.seq.load(Ordering::Acquire);
        if s1 != 2 * (n + 1) {
            return None;
        }
        let t_ns = slot.t.load(Ordering::Relaxed);
        let buttons = slot.buttons.load(Ordering::Relaxed) as u32;
        let mut axes = [0u16; AXES];
        for (k, a) in slot.a.iter().enumerate() {
            let w = a.load(Ordering::Relaxed);
            for j in 0..4 {
                axes[k * 4 + j] = (w >> (16 * j)) as u16;
            }
        }
        fence(Ordering::Acquire);
        (slot.seq.load(Ordering::Relaxed) == s1).then_some(InputSample {
            t_ns,
            axes,
            buttons,
        })
    }

    /// The newest sample.
    pub fn latest(&self) -> Option<InputSample> {
        self.latest_at(u64::MAX)
    }

    /// The newest sample stamped at or before `t_ns` (sample and hold).
    pub fn latest_at(&self, t_ns: u64) -> Option<InputSample> {
        let head = self.pushed();
        let oldest = head.saturating_sub(self.slots.len() as u64 - 1);
        (oldest..head)
            .rev()
            .filter_map(|n| self.get(n))
            .find(|s| s.t_ns <= t_ns)
    }

    /// Every sample pushed after the `from`th still in the ring, oldest first, and the
    /// count to pass next time.
    pub fn since(&self, from: u64) -> (Vec<InputSample>, u64) {
        let head = self.pushed();
        let oldest = head.saturating_sub(self.slots.len() as u64 - 1).max(from);
        ((oldest..head).filter_map(|n| self.get(n)).collect(), head)
    }
}

// ----- sticks and controls -----

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "lowercase")]
pub enum StickFunction {
    Roll,
    Pitch,
    Throttle,
    Yaw,
}

impl StickFunction {
    /// In EdgeTX's default channel order, AETR.
    pub const ALL: [StickFunction; 4] = [
        StickFunction::Roll,
        StickFunction::Pitch,
        StickFunction::Throttle,
        StickFunction::Yaw,
    ];
    /// Roll, pitch and yaw spring back to the middle; throttle stays where it is left.
    pub fn springs(self) -> bool {
        self != StickFunction::Throttle
    }
    pub fn name(self) -> &'static str {
        match self {
            StickFunction::Roll => "roll",
            StickFunction::Pitch => "pitch",
            StickFunction::Throttle => "throttle",
            StickFunction::Yaw => "yaw",
        }
    }
    /// The name of each end, low then high, before Reverse.
    fn end_names(self) -> (&'static str, &'static str) {
        match self {
            StickFunction::Roll | StickFunction::Yaw => ("Left end", "Right end"),
            StickFunction::Pitch => ("Back end", "Forward end"),
            StickFunction::Throttle => ("Bottom", "Top"),
        }
    }
}

/// One end of an axis' raw travel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum AxisEnd {
    Low,
    High,
}

/// A radio control the sim reads: a range of a channel (CH1-8, the axes), or a button
/// (CH9 and up reach the joystick as buttons).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RadioControl {
    /// On while the channel is within `min_us..=max_us`.
    Channel { ch: u8, min_us: u16, max_us: u16 },
    /// On while button `button` (1-24) is in the `pressed` state.
    Button { button: u8, pressed: bool },
}

impl RadioControl {
    pub fn active(&self, s: &InputSample) -> bool {
        match *self {
            RadioControl::Channel { ch, min_us, max_us } => {
                s.us(ch).is_some_and(|us| (min_us..=max_us).contains(&us))
            }
            RadioControl::Button { button, pressed } => s.button(button) == pressed,
        }
    }

    /// A Betaflight `aux` range on channel `ch` (1 is CH1). Channels past CH8 reach the
    /// joystick only as buttons, on above 1500 µs, so the range's middle picks the state.
    pub fn from_range(ch: u32, start: u16, end: u16) -> Option<RadioControl> {
        match ch {
            1..=8 => Some(RadioControl::Channel {
                ch: ch as u8,
                min_us: start,
                max_us: end,
            }),
            9..=32 => Some(RadioControl::Button {
                button: (ch - 8) as u8,
                pressed: (start as u32 + end as u32) / 2 > 1500,
            }),
            _ => None,
        }
    }

    /// `CH5 1700-2100 µs`, `Button 1 pressed`.
    pub fn describe(&self) -> String {
        match *self {
            RadioControl::Channel { ch, min_us, max_us } => format!("CH{ch} {min_us}-{max_us} µs"),
            RadioControl::Button { button, pressed } => {
                format!(
                    "Button {button} {}",
                    if pressed { "pressed" } else { "released" }
                )
            }
        }
    }
}

/// One stick axis' calibration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AxisCal {
    /// 1 is CH1.
    pub ch: u8,
    /// The ends auto-calibration found, raw 0..2048.
    pub auto_low: u16,
    pub auto_high: u16,
    /// Ends the user set; they win over the auto ones until Recalibrate.
    #[serde(default)]
    pub edited_low: Option<u16>,
    #[serde(default)]
    pub edited_high: Option<u16>,
    /// Raw rest value. Throttle's is unused.
    pub centre: u16,
    /// Percent of each side cut from the middle, 0-50. Roll, pitch and yaw only.
    #[serde(default)]
    pub deadzone: u8,
    #[serde(default)]
    pub reverse: bool,
}

impl AxisCal {
    /// Full travel, centred.
    pub fn full(ch: u8) -> Self {
        Self {
            ch,
            auto_low: 0,
            auto_high: AXIS_MAX,
            edited_low: None,
            edited_high: None,
            centre: AXIS_MID,
            deadzone: 0,
            reverse: false,
        }
    }
    pub fn low(&self) -> u16 {
        self.edited_low.unwrap_or(self.auto_low)
    }
    pub fn high(&self) -> u16 {
        self.edited_high.unwrap_or(self.auto_high)
    }
    /// The end's name for function `f`: an end follows Reverse, so with Reverse on the low
    /// raw end is the one the stick reaches pushed right (or forward, or up).
    pub fn end_name(&self, f: StickFunction, end: AxisEnd) -> &'static str {
        let (lo, hi) = f.end_names();
        match (end, self.reverse) {
            (AxisEnd::Low, false) | (AxisEnd::High, true) => lo,
            _ => hi,
        }
    }

    /// −1..1 around the centre, each side scaled to its own end, the deadzone cut from the
    /// middle and the rest rescaled so the ends still reach ±1 with no jump.
    pub fn centred(&self, raw: u16) -> f64 {
        let (lo, hi, c) = (self.low() as f64, self.high() as f64, self.centre as f64);
        let d = raw as f64 - c;
        let span = if d >= 0.0 { hi - c } else { c - lo };
        let t = if span > 0.0 {
            (d / span).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let dz = (self.deadzone.min(50) as f64) / 100.0;
        let a = t.abs();
        let t = if a <= dz {
            0.0
        } else {
            t.signum() * (a - dz) / (1.0 - dz)
        };
        if self.reverse {
            -t
        } else {
            t
        }
    }

    /// 0..1 from the low end to the high end.
    pub fn unit(&self, raw: u16) -> f64 {
        let (lo, hi) = (self.low() as f64, self.high() as f64);
        let t = if hi > lo {
            ((raw as f64 - lo) / (hi - lo)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if self.reverse {
            1.0 - t
        } else {
            t
        }
    }
}

/// A radio's calibration for the sim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Calibration {
    /// Stick mode 1-4: which stick carries which function. Labels only; the map decides.
    pub mode: u8,
    pub roll: AxisCal,
    pub pitch: AxisCal,
    pub throttle: AxisCal,
    pub yaw: AxisCal,
    /// The arm switch, when the sim arms from the calibration rather than the quad's `aux`
    /// lines.
    #[serde(default)]
    pub arm: Option<RadioControl>,
    /// The control that puts the quad back on the start pad.
    #[serde(default)]
    pub reset: Option<RadioControl>,
    /// The quad's mode switches as the sim reads them: pre-filled from its `aux` lines,
    /// changeable here.
    #[serde(default)]
    pub turtle: Option<RadioControl>,
    #[serde(default)]
    pub angle: Option<RadioControl>,
    #[serde(default)]
    pub horizon: Option<RadioControl>,
    #[serde(default)]
    pub airmode: Option<RadioControl>,
}

/// A control the user can set by moving it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum CaptureTarget {
    Arm,
    Reset,
    Turtle,
    Angle,
    Horizon,
    Airmode,
}

impl Default for Calibration {
    /// AETR on CH1-4, full travel, Mode 2.
    fn default() -> Self {
        Self::with_map([1, 2, 3, 4])
    }
}

impl Calibration {
    /// Full travel on the channels given for roll, pitch, throttle and yaw.
    pub fn with_map(map: [u8; 4]) -> Self {
        Self {
            mode: 2,
            roll: AxisCal::full(map[0]),
            pitch: AxisCal::full(map[1]),
            throttle: AxisCal::full(map[2]),
            yaw: AxisCal::full(map[3]),
            arm: None,
            reset: None,
            turtle: None,
            angle: None,
            horizon: None,
            airmode: None,
        }
    }
    pub fn axis(&self, f: StickFunction) -> &AxisCal {
        match f {
            StickFunction::Roll => &self.roll,
            StickFunction::Pitch => &self.pitch,
            StickFunction::Throttle => &self.throttle,
            StickFunction::Yaw => &self.yaw,
        }
    }
    pub fn axis_mut(&mut self, f: StickFunction) -> &mut AxisCal {
        match f {
            StickFunction::Roll => &mut self.roll,
            StickFunction::Pitch => &mut self.pitch,
            StickFunction::Throttle => &mut self.throttle,
            StickFunction::Yaw => &mut self.yaw,
        }
    }
    pub fn control_mut(&mut self, t: CaptureTarget) -> &mut Option<RadioControl> {
        match t {
            CaptureTarget::Arm => &mut self.arm,
            CaptureTarget::Reset => &mut self.reset,
            CaptureTarget::Turtle => &mut self.turtle,
            CaptureTarget::Angle => &mut self.angle,
            CaptureTarget::Horizon => &mut self.horizon,
            CaptureTarget::Airmode => &mut self.airmode,
        }
    }

    /// The channel per function, roll first.
    pub fn map(&self) -> [u8; 4] {
        StickFunction::ALL.map(|f| self.axis(f).ch)
    }

    /// Clears edited ends; deadzones, Reverse, the mode and the map stay.
    pub fn clear_edits(&mut self) {
        for f in StickFunction::ALL {
            let a = self.axis_mut(f);
            a.edited_low = None;
            a.edited_high = None;
        }
    }

    /// The stick values for a sample.
    pub fn apply(&self, s: &InputSample) -> RcInput {
        let raw = |a: &AxisCal| s.axes[(a.ch.clamp(1, AXES as u8) - 1) as usize];
        let on = |c: Option<RadioControl>| c.is_some_and(|c| c.active(s));
        RcInput {
            t_ns: s.t_ns,
            roll: self.roll.centred(raw(&self.roll)),
            pitch: self.pitch.centred(raw(&self.pitch)),
            yaw: self.yaw.centred(raw(&self.yaw)),
            throttle: self.throttle.unit(raw(&self.throttle)),
            channels: s.axes.map(axis_us),
            buttons: s.buttons,
            arm: on(self.arm),
            reset: on(self.reset),
            turtle: on(self.turtle),
            angle: on(self.angle),
            horizon: on(self.horizon),
            airmode: on(self.airmode),
        }
    }
}

/// What the flight controller reads from the radio, after calibration.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RcInput {
    pub t_ns: u64,
    /// −1..1, right positive.
    pub roll: f64,
    /// −1..1, forward positive.
    pub pitch: f64,
    /// −1..1, right positive.
    pub yaw: f64,
    /// 0..1.
    pub throttle: f64,
    /// CH1-8 in µs, for the quad's `aux` ranges.
    pub channels: [u16; AXES],
    /// CH9 and up, as buttons.
    pub buttons: u32,
    /// The calibration's arm control is on.
    pub arm: bool,
    /// The reset control is on.
    pub reset: bool,
    /// The calibration's mode switches are on.
    pub turtle: bool,
    pub angle: bool,
    pub horizon: bool,
    pub airmode: bool,
}

// ----- auto-calibration -----

/// Each side of each stick axis must reach this share of its half travel.
pub const COVERAGE: f64 = 0.8;
/// ...and the sweep lasts at least this long, so the ends are reached, not just 80 %.
pub const MOVE_MIN_NS: u64 = 3_000_000_000;
/// Done accepts this much.
pub const COVERAGE_DONE: f64 = 0.5;
/// Let go: roll, pitch and yaw hold within this share of full travel...
pub const STEADY_SPAN: f64 = 0.015;
/// ...for this long.
pub const STEADY_NS: u64 = 600_000_000;
/// A non-stick axis that moves this share of full travel is a switch.
pub const SWITCH_TRAVEL: f64 = 0.25;
/// A switch found by change gets this margin around the value it moved to.
pub const SWITCH_MARGIN_US: u16 = 150;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CalPhase {
    /// Move both sticks around their full travel.
    Move,
    /// Let go of the sticks.
    LetGo,
    /// Flip the arm switch.
    Arm,
    /// Press the control to use for reset.
    Reset,
    /// Check, tune, save.
    Review,
    /// Move the control for `AutoCal::target`, then back to Review.
    Capture,
}

/// The guided auto-calibration (design 7.3): feed it every sample.
#[derive(Debug, Clone)]
pub struct AutoCal {
    pub phase: CalPhase,
    /// The calibration so far: the map, Reverse, deadzones and edits are kept from the one
    /// it started from.
    pub cal: Calibration,
    /// The short check for a radio whose model QuadCam knows: the map and ends are exact,
    /// so it skips Let go, and Arm when the arm switch is known.
    pub quick: bool,
    /// The arm switch came from the quad's `aux` lines: the Arm step is skipped.
    pub arm_known: bool,
    min: [u16; AXES],
    max: [u16; AXES],
    seen: bool,
    move_start: Option<u64>,
    window: Vec<InputSample>,
    base: Option<InputSample>,
    latest: Option<InputSample>,
    /// Why the last step was refused.
    pub message: Option<String>,
    /// What Capture sets.
    pub target: Option<CaptureTarget>,
}

impl AutoCal {
    /// Starts at Move from `start` (a saved calibration, or one pre-filled from what
    /// QuadCam knows).
    pub fn new(start: Calibration, quick: bool, arm_known: bool) -> Self {
        Self {
            phase: CalPhase::Move,
            cal: start,
            quick,
            arm_known,
            min: [u16::MAX; AXES],
            max: [0; AXES],
            seen: false,
            move_start: None,
            window: vec![],
            base: None,
            latest: None,
            message: None,
            target: None,
        }
    }

    /// Opens on a saved calibration at Review.
    pub fn review(saved: Calibration) -> Self {
        let mut a = Self::new(saved, false, true);
        a.phase = CalPhase::Review;
        a
    }

    pub fn latest(&self) -> Option<&InputSample> {
        self.latest.as_ref()
    }

    /// How far each side of axis `i` has been pushed, the shorter side, 0..1 of half travel.
    fn axis_coverage(&self, i: usize) -> f64 {
        if !self.seen || self.max[i] < self.min[i] {
            return 0.0;
        }
        let m = AXIS_MID as f64;
        let hi = (self.max[i] as f64 - m) / m;
        let lo = (m - self.min[i] as f64) / m;
        hi.min(lo).clamp(0.0, 1.0)
    }

    /// Coverage of a function's channel, the shorter side.
    pub fn coverage(&self, f: StickFunction) -> f64 {
        let ch = self.cal.axis(f).ch as usize;
        if (1..=AXES).contains(&ch) {
            self.axis_coverage(ch - 1)
        } else {
            0.0
        }
    }

    fn stick_axes(&self) -> Vec<usize> {
        self.cal
            .map()
            .iter()
            .map(|c| (*c as usize).saturating_sub(1))
            .collect()
    }

    pub fn feed(&mut self, s: InputSample) {
        self.latest = Some(s);
        match self.phase {
            CalPhase::Move => {
                self.track(&s);
                self.find_map();
                let start = *self.move_start.get_or_insert(s.t_ns);
                if (self.quick || s.t_ns - start >= MOVE_MIN_NS)
                    && StickFunction::ALL
                        .iter()
                        .all(|f| self.coverage(*f) >= COVERAGE)
                {
                    self.end_move();
                }
            }
            CalPhase::LetGo => {
                // A stick still on its way to an end extends it.
                self.track(&s);
                self.window.push(s);
                let cut = s.t_ns.saturating_sub(STEADY_NS);
                let first = self.window[0].t_ns;
                if first > cut {
                    return;
                }
                self.window.retain(|w| w.t_ns >= cut);
                let springs: Vec<usize> = StickFunction::ALL
                    .iter()
                    .filter(|f| f.springs())
                    .map(|f| self.cal.axis(*f).ch as usize - 1)
                    .collect();
                let span = (STEADY_SPAN * AXIS_MAX as f64) as u16;
                let steady = springs.iter().all(|&i| {
                    let lo = self.window.iter().map(|w| w.axes[i]).min().unwrap_or(0);
                    let hi = self.window.iter().map(|w| w.axes[i]).max().unwrap_or(0);
                    hi - lo <= span
                });
                if steady {
                    self.take_centres();
                    self.after_let_go();
                }
            }
            CalPhase::Arm | CalPhase::Reset | CalPhase::Capture => {
                let Some(base) = self.base else {
                    self.base = Some(s);
                    return;
                };
                if let Some(c) = self.changed(&base, &s) {
                    match self.phase {
                        CalPhase::Arm => {
                            self.cal.arm = Some(c);
                            self.go(CalPhase::Reset);
                        }
                        CalPhase::Reset if Some(c) != self.cal.arm => {
                            self.cal.reset = Some(c);
                            self.go(CalPhase::Review);
                        }
                        CalPhase::Capture => {
                            if let Some(t) = self.target.take() {
                                *self.cal.control_mut(t) = Some(c);
                            }
                            self.go(CalPhase::Review);
                        }
                        _ => {}
                    }
                }
            }
            CalPhase::Review => {}
        }
    }

    fn track(&mut self, s: &InputSample) {
        self.seen = true;
        for i in 0..AXES {
            self.min[i] = self.min[i].min(s.axes[i]);
            self.max[i] = self.max[i].max(s.axes[i]);
        }
    }

    /// The ends seen so far (the quick check keeps full travel: its values are exact).
    fn set_auto_ends(&mut self) {
        for f in StickFunction::ALL {
            let i = self.cal.axis(f).ch as usize - 1;
            let (lo, hi) = if self.quick {
                (0, AXIS_MAX)
            } else {
                (self.min[i], self.max[i])
            };
            let a = self.cal.axis_mut(f);
            a.auto_low = lo;
            a.auto_high = hi;
        }
    }

    /// A stick function whose channel did not move takes an unmapped axis that did.
    fn find_map(&mut self) {
        for f in StickFunction::ALL {
            if self.coverage(f) >= COVERAGE_DONE {
                continue;
            }
            let used = self.cal.map();
            let moved = (0..AXES)
                .find(|&i| !used.contains(&((i + 1) as u8)) && self.axis_coverage(i) >= COVERAGE);
            if let Some(i) = moved {
                self.cal.axis_mut(f).ch = (i + 1) as u8;
            }
        }
    }

    fn end_move(&mut self) {
        self.set_auto_ends();
        if self.quick {
            for f in StickFunction::ALL {
                self.cal.axis_mut(f).centre = AXIS_MID;
            }
            self.after_let_go();
        } else {
            self.go(CalPhase::LetGo);
        }
    }

    /// Centres from the steady window (or the latest sample when Next skips the wait).
    /// A spring channel resting near an end while throttle's rests in the middle means
    /// the two were swapped: throttle is the one that stays where it was left.
    fn take_centres(&mut self) {
        let n = self.window.len().max(1) as u64;
        let mean = |i: usize, w: &[InputSample]| {
            (w.iter().map(|s| s.axes[i] as u64).sum::<u64>() / n) as u16
        };
        let w = if self.window.is_empty() {
            self.latest.into_iter().collect()
        } else {
            self.window.clone()
        };
        if w.is_empty() {
            return;
        }
        let thr = self.cal.throttle.ch as usize - 1;
        let off = |v: u16| (v as i32 - AXIS_MID as i32).unsigned_abs() as f64 / AXIS_MID as f64;
        if off(mean(thr, &w)) < 0.05 {
            if let Some(f) = StickFunction::ALL
                .into_iter()
                .filter(|f| f.springs())
                .find(|f| off(mean(self.cal.axis(*f).ch as usize - 1, &w)) > 0.5)
            {
                let (a, b) = (self.cal.axis(f).ch, self.cal.throttle.ch);
                self.cal.axis_mut(f).ch = b;
                self.cal.throttle.ch = a;
            }
        }
        self.set_auto_ends();
        for f in StickFunction::ALL.into_iter().filter(|f| f.springs()) {
            let i = self.cal.axis(f).ch as usize - 1;
            let a = self.cal.axis_mut(f);
            a.centre = mean(i, &w).clamp(a.low(), a.high());
        }
    }

    fn after_let_go(&mut self) {
        if self.arm_known {
            self.go(CalPhase::Reset);
        } else {
            self.go(CalPhase::Arm);
        }
    }

    /// A non-stick axis that moved far, or a button that toggled.
    fn changed(&self, base: &InputSample, s: &InputSample) -> Option<RadioControl> {
        let sticks = self.stick_axes();
        let travel = (SWITCH_TRAVEL * AXIS_MAX as f64) as i32;
        let axis = (0..AXES)
            .filter(|i| !sticks.contains(i))
            .map(|i| (i, (s.axes[i] as i32 - base.axes[i] as i32).abs()))
            .filter(|(_, d)| *d >= travel)
            .max_by_key(|(_, d)| *d)
            .map(|(i, _)| {
                let us = axis_us(s.axes[i]);
                RadioControl::Channel {
                    ch: (i + 1) as u8,
                    min_us: us.saturating_sub(SWITCH_MARGIN_US).max(900),
                    max_us: (us + SWITCH_MARGIN_US).min(2100),
                }
            });
        axis.or_else(|| {
            let diff = base.buttons ^ s.buttons;
            (diff != 0).then(|| {
                let b = diff.trailing_zeros() as u8 + 1;
                RadioControl::Button {
                    button: b,
                    pressed: s.button(b),
                }
            })
        })
    }

    /// The Next or Done button. Move accepts 50 % per side; Let go takes the sticks as
    /// they are; Arm and Reset keep what they have (a pre-filled reset, or none).
    pub fn advance(&mut self) -> bool {
        self.message = None;
        match self.phase {
            CalPhase::Move => {
                let short: Vec<&str> = StickFunction::ALL
                    .iter()
                    .filter(|f| self.coverage(**f) < COVERAGE_DONE)
                    .map(|f| f.name())
                    .collect();
                if !short.is_empty() {
                    self.message = Some(format!(
                        "Not enough movement yet: {}. Push each stick all the way both ways.",
                        short.join(", ")
                    ));
                    return false;
                }
                self.end_move();
            }
            CalPhase::LetGo => {
                self.take_centres();
                self.after_let_go();
            }
            CalPhase::Arm => self.go(CalPhase::Reset),
            CalPhase::Reset => self.go(CalPhase::Review),
            CalPhase::Capture => {
                self.target = None;
                self.go(CalPhase::Review);
            }
            CalPhase::Review => {}
        }
        true
    }

    /// No arm switch: the sim arms from the quad's `aux` lines or the menu.
    pub fn skip(&mut self) {
        match self.phase {
            CalPhase::Arm => {
                self.cal.arm = None;
                self.go(CalPhase::Reset);
            }
            CalPhase::Reset => {
                self.cal.reset = None;
                self.go(CalPhase::Review);
            }
            CalPhase::Capture => {
                if let Some(t) = self.target.take() {
                    *self.cal.control_mut(t) = None;
                }
                self.go(CalPhase::Review);
            }
            _ => {}
        }
    }

    /// Sets one control by moving it: Capture, then back to Review.
    pub fn capture(&mut self, t: CaptureTarget) {
        self.message = None;
        self.target = Some(t);
        self.go(CalPhase::Capture);
    }

    /// Recalibrate: the full flow again. Edited ends go; deadzones, Reverse, the mode and
    /// the map stay.
    pub fn recalibrate(&mut self) {
        self.cal.clear_edits();
        self.quick = false;
        self.min = [u16::MAX; AXES];
        self.max = [0; AXES];
        self.seen = false;
        self.move_start = None;
        self.go(CalPhase::Move);
    }

    fn go(&mut self, p: CalPhase) {
        self.phase = p;
        self.window.clear();
        self.base = None;
    }
}

// ----- link model -----

/// The radio link (design 7.6). Off: each sample reaches the FC as it arrives.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LinkConfig {
    /// ELRS packet rate (50, 150, 250, 500 Hz); None is off.
    pub rate_hz: Option<u32>,
    /// Each packet arrives up to this late, uniformly.
    pub jitter_us: u32,
    /// Share of packets lost, 0..1.
    pub loss: f64,
    /// No packet for this long: failsafe, the quad disarms.
    pub failsafe_ms: u32,
    pub seed: u64,
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            rate_hz: None,
            jitter_us: 0,
            loss: 0.0,
            failsafe_ms: 1000,
            seed: 1,
        }
    }
}

/// What the FC sees at a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkOut {
    /// The last sample delivered, held between packets.
    pub sample: Option<InputSample>,
    /// No packet within the failsafe time.
    pub failsafe: bool,
}

/// SplitMix64: small, seeded, the same on every machine.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// 0..1.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Resamples the radio stream to a packet rate with seeded jitter and loss. Each packet
/// draws its jitter and its loss whether or not they are used, so the same seed and the
/// same input always give the same output.
#[derive(Debug, Clone)]
pub struct LinkModel {
    cfg: LinkConfig,
    rng: Rng,
    /// The next packet's nominal send time.
    next_send: Option<u64>,
    /// Packets sent but not yet arrived: (arrival, sample).
    flight: Vec<(u64, Option<InputSample>)>,
    held: Option<InputSample>,
    last_rx: Option<u64>,
}

impl LinkModel {
    pub fn new(cfg: LinkConfig) -> Self {
        Self {
            rng: Rng::new(cfg.seed),
            cfg,
            next_send: None,
            flight: vec![],
            held: None,
            last_rx: None,
        }
    }

    pub fn config(&self) -> &LinkConfig {
        &self.cfg
    }

    /// The FC's view at step time `t_ns`; `ring` gives the newest sample at or before a
    /// time (`InputRing::latest_at`, or a recording).
    pub fn step(&mut self, t_ns: u64, ring: impl Fn(u64) -> Option<InputSample>) -> LinkOut {
        let failsafe_ns = self.cfg.failsafe_ms as u64 * 1_000_000;
        let Some(rate) = self.cfg.rate_hz.filter(|r| *r > 0) else {
            let s = ring(t_ns);
            if s.is_some() {
                self.last_rx = Some(t_ns);
            }
            return LinkOut {
                sample: s,
                failsafe: false,
            };
        };
        let period = 1_000_000_000 / rate as u64;
        let mut send = *self.next_send.get_or_insert(t_ns);
        while send <= t_ns {
            let jitter = (self.rng.unit() * self.cfg.jitter_us as f64 * 1000.0) as u64;
            let lost = self.rng.unit() < self.cfg.loss;
            if !lost {
                self.flight.push((send + jitter, ring(send)));
            }
            send += period;
        }
        self.next_send = Some(send);
        let mut i = 0;
        while i < self.flight.len() {
            if self.flight[i].0 <= t_ns {
                let (at, s) = self.flight.remove(i);
                if s.is_some() && self.last_rx.is_none_or(|r| at >= r) {
                    self.held = s;
                    self.last_rx = Some(at);
                }
            } else {
                i += 1;
            }
        }
        let since = self.last_rx.map(|r| t_ns.saturating_sub(r));
        LinkOut {
            sample: self.held,
            failsafe: since.is_some_and(|d| d > failsafe_ns),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const MS: u64 = 1_000_000;

    fn s(t_ms: u64, axes: [u16; 8], buttons: u32) -> InputSample {
        InputSample {
            t_ns: t_ms * MS,
            axes,
            buttons,
        }
    }

    /// Sticks at rest: roll, pitch, yaw centred, throttle low, AETR.
    fn rest() -> [u16; 8] {
        [1024, 1024, 0, 1024, 0, 0, 0, 0]
    }

    #[test]
    fn the_ring_holds_samples_and_answers_by_time() {
        let r = InputRing::new(8);
        assert_eq!(r.capacity(), 8);
        assert!(r.latest().is_none());
        for t in 1..=20 {
            r.push(s(t, [t as u16; 8], t as u32));
        }
        assert_eq!(r.pushed(), 20);
        assert_eq!(r.latest().unwrap().t_ns, 20 * MS);
        assert_eq!(r.latest_at(15 * MS + 500).unwrap().axes[0], 15);
        assert!(r.latest_at(5 * MS).is_none(), "gone from the ring");
        let (got, next) = r.since(17);
        assert_eq!(
            got.iter().map(|x| x.buttons).collect::<Vec<_>>(),
            [18, 19, 20]
        );
        assert_eq!(next, 20);
    }

    #[test]
    fn the_ring_never_tears_across_threads() {
        let r = Arc::new(InputRing::new(64));
        let w = r.clone();
        let t = std::thread::spawn(move || {
            for n in 0..200_000u64 {
                let v = (n % 2048) as u16;
                w.push(InputSample {
                    t_ns: n,
                    axes: [v; 8],
                    buttons: v as u32,
                });
            }
        });
        let mut reads = 0;
        while !t.is_finished() || reads == 0 {
            if let Some(x) = r.latest() {
                assert!(x.axes.iter().all(|a| *a == x.axes[0]), "torn: {x:?}");
                assert_eq!(x.buttons, x.axes[0] as u32);
                assert_eq!(x.t_ns % 2048, x.axes[0] as u64);
                reads += 1;
            }
        }
        t.join().unwrap();
    }

    #[test]
    fn deadzone_rescales_with_no_jump() {
        let mut a = AxisCal::full(1);
        a.deadzone = 10;
        assert_eq!(a.centred(1024), 0.0);
        assert_eq!(a.centred(2048), 1.0);
        assert_eq!(a.centred(0), -1.0);
        // Inside the deadzone: zero. Just past its edge: just above zero, not 0.1.
        let edge = 1024 + 102; // 10 % of the upper half, rounded down
        assert_eq!(a.centred(edge), 0.0);
        let past = a.centred(edge + 2);
        assert!(past > 0.0 && past < 0.01, "{past}");
        // Monotonic, with no step bigger than one raw count's share.
        let mut prev = -1.0;
        for raw in 0..=2048u16 {
            let v = a.centred(raw);
            assert!(
                v >= prev && v - prev < 0.0025,
                "step at {raw}: {prev} -> {v}"
            );
            prev = v;
        }
        a.reverse = true;
        assert_eq!(a.centred(2048), -1.0);
    }

    #[test]
    fn ends_scale_each_side_and_names_follow_reverse() {
        let mut a = AxisCal::full(2);
        a.auto_low = 100;
        a.auto_high = 1900;
        a.centre = 1000;
        assert_eq!(a.centred(1900), 1.0);
        assert_eq!(a.centred(100), -1.0);
        assert!((a.centred(1450) - 0.5).abs() < 1e-9);
        assert!((a.centred(550) + 0.5).abs() < 1e-9);
        assert_eq!(a.end_name(StickFunction::Pitch, AxisEnd::Low), "Back end");
        assert_eq!(
            a.end_name(StickFunction::Pitch, AxisEnd::High),
            "Forward end"
        );
        a.reverse = true;
        assert_eq!(
            a.end_name(StickFunction::Pitch, AxisEnd::Low),
            "Forward end"
        );
        assert_eq!(a.end_name(StickFunction::Throttle, AxisEnd::High), "Bottom");
        let mut t = AxisCal::full(3);
        t.auto_low = 48;
        t.auto_high = 2000;
        assert_eq!(t.unit(48), 0.0);
        assert_eq!(t.unit(2048), 1.0);
        t.edited_high = Some(1024);
        assert_eq!(t.unit(1024), 1.0, "an edited end wins");
    }

    #[test]
    fn controls_read_channels_and_buttons() {
        let x = s(0, [0, 0, 0, 0, 2048, 0, 0, 0], 0b100);
        let arm = RadioControl::from_range(5, 1700, 2100).unwrap();
        assert!(arm.active(&x));
        let turtle = RadioControl::from_range(9, 1700, 2100).unwrap();
        assert_eq!(
            turtle,
            RadioControl::Button {
                button: 1,
                pressed: true
            }
        );
        assert!(!turtle.active(&x));
        assert!(RadioControl::from_range(11, 1800, 2100).unwrap().active(&x));
        assert!(RadioControl::from_range(11, 900, 1200)
            .unwrap()
            .active(&s(0, rest(), 0)));
        assert_eq!(arm.describe(), "CH5 1700-2100 µs");
    }

    /// Sweeps every stick around its travel, `low` and `high` as the reach on each side.
    fn sweep(a: &mut AutoCal, t: &mut u64, low: u16, high: u16, map: [usize; 4]) {
        for k in 0..=40u16 {
            for &i in &map {
                let mut ax = [1024, 1024, 1024, 1024, 0, 0, 0, 0];
                ax[map[2]] = 0;
                let v = if k <= 20 {
                    AXIS_MID - (AXIS_MID - low) * k / 20
                } else {
                    AXIS_MID + (high - AXIS_MID) * (k - 20) / 20
                };
                ax[i] = v;
                *t += 20;
                a.feed(s(*t, ax, 0));
            }
        }
    }

    fn hold(a: &mut AutoCal, t: &mut u64, ax: [u16; 8], buttons: u32, ms: u64) {
        let end = *t + ms;
        while *t < end {
            *t += 5;
            a.feed(s(*t, ax, buttons));
        }
    }

    #[test]
    fn the_flow_finds_map_ends_centres_and_the_arm_switch() {
        let mut a = AutoCal::new(Calibration::default(), false, false);
        let mut t = 0;
        sweep(&mut a, &mut t, 40, 2010, [0, 1, 2, 3]);
        assert_eq!(a.phase, CalPhase::LetGo);
        // Let go: the springs rest a little off the middle; throttle stays low.
        let mut ax = [1030, 1018, 0, 1027, 0, 0, 0, 0];
        hold(&mut a, &mut t, ax, 0, 700);
        assert_eq!(a.phase, CalPhase::Arm);
        // The ends include the reach after Move advanced, at 80 %.
        assert_eq!(a.cal.roll.auto_low, 40);
        assert_eq!(a.cal.yaw.auto_high, 2010);
        assert_eq!(
            (a.cal.roll.centre, a.cal.pitch.centre, a.cal.yaw.centre),
            (1030, 1018, 1027)
        );
        // Arm switch on CH5.
        hold(&mut a, &mut t, ax, 0, 20);
        ax[4] = 2048;
        hold(&mut a, &mut t, ax, 0, 20);
        assert_eq!(
            a.cal.arm,
            Some(RadioControl::Channel {
                ch: 5,
                min_us: 1862,
                max_us: 2100
            })
        );
        assert_eq!(a.phase, CalPhase::Reset);
        // Reset: a momentary on CH9, button 1. The arm switch moving again is not it.
        hold(&mut a, &mut t, ax, 0, 20);
        hold(&mut a, &mut t, ax, 1, 20);
        assert_eq!(a.phase, CalPhase::Review);
        assert_eq!(
            a.cal.reset,
            Some(RadioControl::Button {
                button: 1,
                pressed: true
            })
        );
        let out = a.cal.apply(&s(t, [2010, 1018, 2010, 40, 2048, 0, 0, 0], 1));
        assert_eq!((out.roll, out.pitch, out.yaw), (1.0, 0.0, -1.0));
        assert_eq!(out.throttle, 1.0);
        assert!(out.arm && out.reset);
    }

    #[test]
    fn capture_changes_one_control_and_returns_to_review() {
        let mut cal = Calibration::default();
        cal.turtle = RadioControl::from_range(9, 1700, 2100);
        let mut a = AutoCal::review(cal);
        let mut t = 0;
        a.capture(CaptureTarget::Turtle);
        assert_eq!(a.phase, CalPhase::Capture);
        hold(&mut a, &mut t, rest(), 0, 20);
        hold(&mut a, &mut t, rest(), 0b10, 20);
        assert_eq!(a.phase, CalPhase::Review);
        let turtle = Some(RadioControl::Button {
            button: 2,
            pressed: true,
        });
        assert_eq!(a.cal.turtle, turtle);
        assert!(a.cal.apply(&s(t, rest(), 0b10)).turtle);
        // Skip clears it; Next keeps it.
        a.capture(CaptureTarget::Angle);
        a.skip();
        assert_eq!((a.phase, a.cal.angle), (CalPhase::Review, None));
        a.capture(CaptureTarget::Turtle);
        a.advance();
        assert_eq!(a.cal.turtle, turtle);
    }

    #[test]
    fn the_map_follows_the_axes_that_moved() {
        // Throttle on CH1 (TAER). Starting from AETR the sweep moves all four, so no axis is
        // unmapped; throttle is told apart at let go, as the one that stays where it was left.
        // Roll, pitch and yaw look alike; the review lets the user swap them.
        let mut a = AutoCal::new(Calibration::default(), false, true);
        let mut t = 0;
        sweep(&mut a, &mut t, 0, 2048, [1, 2, 0, 3]);
        assert_eq!(a.phase, CalPhase::LetGo);
        hold(&mut a, &mut t, [0, 1024, 1024, 1024, 0, 0, 0, 0], 0, 700);
        assert_eq!(a.cal.throttle.ch, 1);
        let mut springs = [a.cal.roll.ch, a.cal.pitch.ch, a.cal.yaw.ch];
        springs.sort();
        assert_eq!(springs, [2, 3, 4]);
        // A stick on an unmapped axis: yaw on CH6 instead of CH4.
        let mut b = AutoCal::new(Calibration::default(), false, true);
        let mut t = 0;
        sweep(&mut b, &mut t, 0, 2048, [0, 1, 2, 5]);
        assert_eq!(b.cal.yaw.ch, 6);
        assert_eq!(b.phase, CalPhase::LetGo);
    }

    #[test]
    fn per_side_coverage_refuses_a_one_sided_sweep() {
        let mut a = AutoCal::new(Calibration::default(), false, false);
        let mut t = 0;
        // Pitch reaches only 40 % back; everything else goes all the way.
        for k in 0..=40u16 {
            for i in 0..4 {
                let mut ax = rest();
                let full = if k <= 20 {
                    1024 - 1024 * k / 20
                } else {
                    1024 + 1024 * (k - 20) / 20
                };
                ax[i] = if i == 1 && k <= 20 {
                    1024 - 410 * k / 20
                } else {
                    full
                };
                t += 20;
                a.feed(s(t, ax, 0));
            }
        }
        assert_eq!(a.phase, CalPhase::Move, "pitch back is short");
        assert!(a.coverage(StickFunction::Pitch) < 0.5 && a.coverage(StickFunction::Roll) >= 0.99);
        assert!(!a.advance(), "Done needs half of each side");
        assert!(a.message.as_deref().unwrap().contains("pitch"));
        // Pushed to 60 % back: Done accepts it, auto does not.
        a.feed(s(t + 5, [1024, 410, 0, 1024, 0, 0, 0, 0], 0));
        assert_eq!(a.phase, CalPhase::Move);
        assert!(a.advance());
        assert_eq!(a.phase, CalPhase::LetGo);
    }

    #[test]
    fn edited_ends_survive_until_recalibrate_and_deadzones_stay() {
        let mut cal = Calibration::default();
        cal.roll.deadzone = 5;
        cal.pitch.edited_high = Some(1900);
        cal.pitch.reverse = true;
        // A quick check keeps the edit: it sets only the auto ends.
        let mut a = AutoCal::new(cal.clone(), true, true);
        let mut t = 0;
        sweep(&mut a, &mut t, 0, 2048, [0, 1, 2, 3]);
        assert_eq!(
            a.phase,
            CalPhase::Reset,
            "the quick check skips Let go and Arm"
        );
        a.advance();
        assert_eq!(a.phase, CalPhase::Review);
        assert_eq!(a.cal.pitch.high(), 1900);
        assert_eq!(a.cal.pitch.auto_high, 2048);
        // Survives a save and a load.
        let json = serde_json::to_string(&a.cal).unwrap();
        let back: Calibration = serde_json::from_str(&json).unwrap();
        assert_eq!(back.pitch.edited_high, Some(1900));
        // Recalibrate clears it; the deadzone and Reverse stay.
        a.recalibrate();
        assert_eq!(a.phase, CalPhase::Move);
        assert_eq!(a.cal.pitch.edited_high, None);
        assert_eq!(a.cal.roll.deadzone, 5);
        assert!(a.cal.pitch.reverse);
    }

    fn stream(seed: u64, rate: u32, loss: f64) -> Vec<LinkOut> {
        let ring = InputRing::new(4096);
        for t in 0..2000u64 {
            ring.push(s(t, [(t % 2048) as u16; 8], 0));
        }
        let mut m = LinkModel::new(LinkConfig {
            rate_hz: Some(rate),
            jitter_us: 800,
            loss,
            failsafe_ms: 100,
            seed,
        });
        (0..4000u64)
            .map(|k| m.step(k * MS / 2, |t| ring.latest_at(t)))
            .collect()
    }

    #[test]
    fn the_link_model_is_seeded_and_replays_exactly() {
        let a = stream(7, 250, 0.1);
        assert_eq!(a, stream(7, 250, 0.1), "same seed, same output");
        assert_ne!(a, stream(8, 250, 0.1), "another seed differs");
        // At 250 Hz the FC sees about 250 distinct values a second, fewer with 10 % loss.
        let changes = |v: &[LinkOut]| v.windows(2).filter(|w| w[0].sample != w[1].sample).count();
        let lossless = stream(7, 250, 0.0);
        let (n0, n1) = (changes(&lossless), changes(&a));
        assert!((480..=510).contains(&n0), "{n0}");
        assert!(n1 < n0 && n1 > n0 * 8 / 10, "{n1} of {n0}");
        // Values lag their source by at most a period plus jitter.
        for (k, o) in lossless.iter().enumerate().skip(20) {
            let t = k as u64 * MS / 2;
            let lag = t - o.sample.unwrap().t_ns;
            assert!(lag <= 4 * MS + 800_000 + MS, "lag {lag} at {t}");
        }
        assert!(lossless.iter().all(|o| !o.failsafe));
    }

    #[test]
    fn a_lost_link_goes_to_failsafe_and_off_passes_through() {
        let out = stream(3, 50, 1.0);
        assert!(out.iter().all(|o| o.sample.is_none()));
        let ring = InputRing::new(16);
        ring.push(s(1, rest(), 0));
        let mut m = LinkModel::new(LinkConfig {
            rate_hz: Some(150),
            loss: 0.0,
            failsafe_ms: 100,
            ..Default::default()
        });
        assert!(!m.step(10 * MS, |t| ring.latest_at(t)).failsafe);
        // Every packet lost from now on: failsafe after 100 ms.
        let mut m2 = m.clone();
        m2.cfg.loss = 1.0;
        assert!(!m2.step(60 * MS, |t| ring.latest_at(t)).failsafe);
        assert!(m2.step(200 * MS, |t| ring.latest_at(t)).failsafe);
        let mut off = LinkModel::new(LinkConfig::default());
        let o = off.step(5 * MS, |t| ring.latest_at(t));
        assert_eq!(o.sample.unwrap().t_ns, MS);
        assert!(!o.failsafe);
    }
}
