//! The physics thread's input: timestamped RC frames in a lock-free ring (sim-design 2.3).
//!
//! The input side (S4: the radio HID reader, mapping, calibration and the link model) pushes
//! post-link frames with `RcProducer::push`. The physics thread drains the ring and, for each
//! step, holds the newest frame whose time is at or before the step's time, as a receiver
//! does at its packet rate. The deterministic core consumes that held frame; a recording
//! stores it (`record`), so replays never depend on the ring or the clock.

use serde::{Deserialize, Serialize};

/// Channels per frame: AETR (roll, pitch, throttle, yaw) then AUX1.. (index 4 is AUX1).
pub const CHANNELS: usize = 16;

/// The link reports a lost link (failsafe).
pub const FLAG_LINK_LOST: u8 = 1;

/// One RC frame, post-link: channel values in µs (1000-2000, centre 1500), as Betaflight
/// sees them, and the host time it applies from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RcFrame {
    /// Host monotonic time (ns), the same clock as `runner::Clock::now_ns`.
    pub t_ns: u64,
    pub ch: [u16; CHANNELS],
    pub flags: u8,
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
}

/// Pilot sticks: roll right, pitch forward, yaw right positive; throttle 0..1.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Sticks {
    pub roll: f64,
    pub pitch: f64,
    pub throttle: f64,
    pub yaw: f64,
}

/// The producer end: S4's input thread owns it.
pub struct RcProducer(rtrb::Producer<RcFrame>);

/// The consumer end: the physics thread owns it.
pub struct RcConsumer {
    rx: rtrb::Consumer<RcFrame>,
    held: RcFrame,
    pending: Option<RcFrame>,
    /// Frames taken from the ring so far.
    pub received: u64,
}

/// A single-producer, single-consumer ring of `capacity` frames.
pub fn rc_ring(capacity: usize) -> (RcProducer, RcConsumer) {
    let (tx, rx) = rtrb::RingBuffer::new(capacity);
    (
        RcProducer(tx),
        RcConsumer {
            rx,
            held: RcFrame::default(),
            pending: None,
            received: 0,
        },
    )
}

impl RcProducer {
    /// Push a frame. Frames must arrive in time order. Returns false when the ring is full
    /// (the physics thread has stalled); the frame is dropped.
    pub fn push(&mut self, f: RcFrame) -> bool {
        self.0.push(f).is_ok()
    }
}

/// What the physics thread reads input from.
pub trait RcSource: Send {
    /// The newest frame with `t_ns` at or before `t_ns` (sample and hold).
    fn frame_at(&mut self, t_ns: u64) -> RcFrame;
}

impl RcSource for RcConsumer {
    fn frame_at(&mut self, t_ns: u64) -> RcFrame {
        loop {
            let next = match self.pending.take() {
                Some(f) => f,
                None => match self.rx.pop() {
                    Ok(f) => {
                        self.received += 1;
                        f
                    }
                    Err(_) => break,
                },
            };
            if next.t_ns <= t_ns {
                self.held = next;
            } else {
                self.pending = Some(next);
                break;
            }
        }
        self.held
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

    fn at(t: u64, roll: u16) -> RcFrame {
        let mut f = RcFrame {
            t_ns: t,
            ..RcFrame::default()
        };
        f.ch[0] = roll;
        f
    }

    #[test]
    fn ring_samples_and_holds_by_time() {
        let (mut tx, mut rx) = rc_ring(64);
        assert_eq!(rx.frame_at(0), RcFrame::default());
        for (t, r) in [(1_000, 1100), (2_000, 1200), (5_000, 1500)] {
            assert!(tx.push(at(t, r)));
        }
        assert_eq!(rx.frame_at(999).ch[0], 1500);
        assert_eq!(rx.frame_at(1_500).ch[0], 1100);
        assert_eq!(rx.frame_at(4_999).ch[0], 1200);
        assert_eq!(rx.frame_at(5_000).ch[0], 1500);
        assert_eq!(rx.received, 3);
    }

    #[test]
    fn full_ring_refuses_and_keeps_order() {
        let (mut tx, mut rx) = rc_ring(2);
        assert!(tx.push(at(1, 1001)));
        assert!(tx.push(at(2, 1002)));
        assert!(!tx.push(at(3, 1003)));
        assert_eq!(rx.frame_at(10).ch[0], 1002);
    }

    #[test]
    fn ring_works_across_threads() {
        let (mut tx, mut rx) = rc_ring(1024);
        let h = std::thread::spawn(move || {
            for i in 0..500u64 {
                while !tx.push(at(i * 10, 1000 + i as u16)) {
                    std::thread::yield_now();
                }
            }
        });
        h.join().unwrap();
        assert_eq!(rx.frame_at(4_990).ch[0], 1499);
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
}
