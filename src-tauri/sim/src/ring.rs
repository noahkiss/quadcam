//! What the physics core consumes each step: one post-link RC frame (sim-design 2.3).
//!
//! The radio side is [`crate::input`] (S4): the HID thread pushes raw samples into its
//! `InputRing`; the link model resamples them at the packet rate; the calibration turns a
//! sample into sticks. [`RadioSource`] chains those for the physics thread: for each step
//! it takes the newest sample at or before the step's time (sample and hold, as a receiver
//! does) and hands the core an [`RcFrame`]. A recording stores the frames, so replays never
//! depend on the ring, the link or the clock.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::input::{Calibration, InputRing, LinkModel, RcInput, AXES};

/// Channels per frame: AETR (roll, pitch, throttle, yaw) then AUX1.. (index 4 is AUX1).
pub const CHANNELS: usize = 16;

/// The link is in failsafe.
pub const FLAG_LINK_LOST: u16 = 1;
/// The sim's reset control is on.
pub const FLAG_RESET: u16 = 1 << 1;

/// The modes a radio calibration can drive directly, instead of the quad's `aux` ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Arm,
    Angle,
    Horizon,
    Turtle,
    Airmode,
}

impl Mode {
    /// (the "set" bit, the "on" bit).
    fn bits(self) -> (u16, u16) {
        let k = match self {
            Mode::Arm => 0,
            Mode::Angle => 1,
            Mode::Horizon => 2,
            Mode::Turtle => 3,
            Mode::Airmode => 4,
        };
        (1 << (2 + 2 * k), 1 << (3 + 2 * k))
    }
}

/// One RC frame, post-link: channel values in µs (1000-2000, centre 1500), as Betaflight
/// sees them, and the host time it applies from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RcFrame {
    /// Host monotonic time (ns): [`crate::input::now_ns`], the clock the runner steps on.
    pub t_ns: u64,
    pub ch: [u16; CHANNELS],
    /// `FLAG_*` bits and per-mode overrides.
    pub flags: u16,
}

impl Default for RcFrame {
    /// Sticks centred, throttle low, every aux low, link up.
    fn default() -> RcFrame {
        let mut ch = [1000; CHANNELS];
        ch[0] = 1500;
        ch[1] = 1500;
        ch[3] = 1500;
        RcFrame {
            t_ns: 0,
            ch,
            flags: 0,
        }
    }
}

impl RcFrame {
    /// Sticks as -1..1 (roll, pitch, yaw) and throttle 0..1, straight from the channels.
    pub fn sticks(&self) -> Sticks {
        let c = |v: u16| ((v as f64 - 1500.0) / 500.0).clamp(-1.0, 1.0);
        Sticks {
            roll: c(self.ch[0]),
            pitch: c(self.ch[1]),
            throttle: ((self.ch[2] as f64 - 1000.0) / 1000.0).clamp(0.0, 1.0),
            yaw: c(self.ch[3]),
        }
    }

    pub fn link_lost(&self) -> bool {
        self.flags & FLAG_LINK_LOST != 0
    }

    pub fn reset(&self) -> bool {
        self.flags & FLAG_RESET != 0
    }

    /// Drive `m` from this frame rather than from the quad's `aux` ranges.
    pub fn set_mode(&mut self, m: Mode, on: bool) {
        let (set, bit) = m.bits();
        self.flags |= set;
        if on {
            self.flags |= bit;
        } else {
            self.flags &= !bit;
        }
    }

    /// `Some(on)` when the frame drives `m`.
    pub fn mode(&self, m: Mode) -> Option<bool> {
        let (set, bit) = m.bits();
        (self.flags & set != 0).then_some(self.flags & bit != 0)
    }

    /// Build a frame from sticks (roll, pitch, yaw −1..1, throttle 0..1) and aux values.
    pub fn from_sticks(s: Sticks, aux: &[u16]) -> RcFrame {
        let mut f = RcFrame::default();
        let us = |x: f64| (1500.0 + 500.0 * x.clamp(-1.0, 1.0)).round() as u16;
        f.ch[0] = us(s.roll);
        f.ch[1] = us(s.pitch);
        f.ch[2] = (1000.0 + 1000.0 * s.throttle.clamp(0.0, 1.0)).round() as u16;
        f.ch[3] = us(s.yaw);
        for (i, v) in aux.iter().enumerate().take(CHANNELS - 4) {
            f.ch[4 + i] = *v;
        }
        f
    }

    /// A calibrated radio sample: the calibrated sticks on CH1-4, the radio's own channels
    /// from CH5 on (the quad's `aux` ranges read them), the reset control, and the modes the
    /// calibration assigns a control to.
    pub fn from_input(i: &RcInput, cal: &Calibration, failsafe: bool) -> RcFrame {
        let mut f = RcFrame::from_sticks(
            Sticks {
                roll: i.roll,
                pitch: i.pitch,
                throttle: i.throttle,
                yaw: i.yaw,
            },
            &i.channels[4..AXES],
        );
        f.t_ns = i.t_ns;
        if failsafe {
            f.flags |= FLAG_LINK_LOST;
        }
        if i.reset {
            f.flags |= FLAG_RESET;
        }
        for (m, set, on) in [
            (Mode::Arm, cal.arm.is_some(), i.arm),
            (Mode::Angle, cal.angle.is_some(), i.angle),
            (Mode::Horizon, cal.horizon.is_some(), i.horizon),
            (Mode::Turtle, cal.turtle.is_some(), i.turtle),
            (Mode::Airmode, cal.airmode.is_some(), i.airmode),
        ] {
            if set {
                f.set_mode(m, on);
            }
        }
        f
    }
}

/// Pilot sticks: roll right, pitch forward, yaw right positive; throttle 0..1.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Sticks {
    pub roll: f64,
    pub pitch: f64,
    pub throttle: f64,
    pub yaw: f64,
}

/// What the physics thread reads input from.
pub trait RcSource: Send {
    /// The frame for a step at host time `t_ns` (sample and hold).
    fn frame_at(&mut self, t_ns: u64) -> RcFrame;
}

/// The radio: S4's input ring through the link model and the calibration.
pub struct RadioSource {
    pub ring: Arc<InputRing>,
    pub link: LinkModel,
    pub cal: Calibration,
}

impl RcSource for RadioSource {
    fn frame_at(&mut self, t_ns: u64) -> RcFrame {
        let ring = &self.ring;
        let out = self.link.step(t_ns, |t| ring.latest_at(t));
        match out.sample {
            Some(s) => RcFrame::from_input(&self.cal.apply(&s), &self.cal, out.failsafe),
            None => RcFrame {
                t_ns,
                ..RcFrame::default()
            },
        }
    }
}

/// A scripted input for tests and headless runs: frames by time.
#[derive(Debug, Clone, Default)]
pub struct FakeRc {
    frames: Vec<RcFrame>,
    next: usize,
    held: RcFrame,
}

impl FakeRc {
    pub fn new(mut frames: Vec<RcFrame>) -> FakeRc {
        frames.sort_by_key(|f| f.t_ns);
        FakeRc {
            frames,
            next: 0,
            held: RcFrame::default(),
        }
    }

    /// One frame held forever.
    pub fn constant(f: RcFrame) -> FakeRc {
        FakeRc::new(vec![RcFrame { t_ns: 0, ..f }])
    }
}

impl RcSource for FakeRc {
    fn frame_at(&mut self, t_ns: u64) -> RcFrame {
        while self.next < self.frames.len() && self.frames[self.next].t_ns <= t_ns {
            self.held = self.frames[self.next];
            self.next += 1;
        }
        self.held
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{InputSample, LinkConfig, RadioControl, AXIS_MAX, AXIS_MID};

    fn at(t: u64, roll: u16) -> RcFrame {
        let mut f = RcFrame {
            t_ns: t,
            ..RcFrame::default()
        };
        f.ch[0] = roll;
        f
    }

    #[test]
    fn fake_input_samples_and_holds_by_time() {
        let mut rx = FakeRc::new(vec![at(1_000, 1100), at(2_000, 1200), at(5_000, 1500)]);
        assert_eq!(rx.frame_at(999).ch[0], 1500);
        assert_eq!(rx.frame_at(1_500).ch[0], 1100);
        assert_eq!(rx.frame_at(4_999).ch[0], 1200);
        assert_eq!(rx.frame_at(5_000).ch[0], 1500);
    }

    #[test]
    fn sticks_round_trip() {
        let s = Sticks {
            roll: 0.5,
            pitch: -0.25,
            throttle: 0.3,
            yaw: 1.0,
        };
        let f = RcFrame::from_sticks(s, &[2000]);
        assert_eq!(f.ch[4], 2000);
        let b = f.sticks();
        assert!((b.roll - 0.5).abs() < 1e-9 && (b.pitch + 0.25).abs() < 1e-9);
        assert!((b.throttle - 0.3).abs() < 1e-9 && (b.yaw - 1.0).abs() < 1e-9);
    }

    #[test]
    fn mode_overrides_are_tri_state() {
        let mut f = RcFrame::default();
        assert_eq!(f.mode(Mode::Angle), None);
        f.set_mode(Mode::Angle, true);
        f.set_mode(Mode::Arm, false);
        assert_eq!(f.mode(Mode::Angle), Some(true));
        assert_eq!(f.mode(Mode::Arm), Some(false));
        assert_eq!(f.mode(Mode::Turtle), None);
        f.set_mode(Mode::Angle, false);
        assert_eq!(f.mode(Mode::Angle), Some(false));
    }

    fn sample(t_ns: u64, axes: [u16; AXES]) -> InputSample {
        InputSample {
            t_ns,
            axes,
            buttons: 0,
        }
    }

    #[test]
    fn radio_source_reads_the_input_ring_through_the_link_and_calibration() {
        let ring = Arc::new(InputRing::new(64));
        let mut cal = Calibration::default();
        // Reset on CH6 high, angle on CH5 high: the calibration drives them.
        cal.reset = RadioControl::from_range(6, 1700, 2100);
        cal.angle = RadioControl::from_range(5, 1700, 2100);
        let mut src = RadioSource {
            ring: ring.clone(),
            link: LinkModel::new(LinkConfig::default()),
            cal,
        };
        assert_eq!(src.frame_at(10).ch[0], 1500);
        let mut axes = [AXIS_MID; AXES];
        axes[0] = AXIS_MAX; // roll full right
        axes[2] = 0; // throttle low
        axes[4] = AXIS_MAX; // CH5 high
        axes[5] = AXIS_MAX; // CH6 high
        ring.push(sample(1_000, axes));
        let f = src.frame_at(2_000);
        assert!(f.ch[0] >= 1990, "{}", f.ch[0]);
        assert!(f.ch[2] <= 1010, "{}", f.ch[2]);
        assert!(f.reset());
        assert_eq!(f.mode(Mode::Angle), Some(true));
        assert_eq!(f.mode(Mode::Arm), None);
        assert!(f.ch[4] > 1900);
        assert!(!f.link_lost());
    }
}
