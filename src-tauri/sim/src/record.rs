//! Recordings: the post-link input stream, step by step, plus everything needed to rebuild
//! the sim. Replaying one gives a bit-identical flight (sim-design 2.2, 2.3).

use serde::{Deserialize, Serialize};

use crate::profile::SimProfile;
use crate::ring::{RcFrame, CHANNELS};
use crate::sim::{Sim, SimSettings};
use crate::world::WorldSpec;

pub const RECORDING_VERSION: u32 = 1;

/// The input from step `step` on (until the next frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecFrame {
    pub step: u64,
    pub ch: [u16; CHANNELS],
    pub flags: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recording {
    pub version: u32,
    pub profile: SimProfile,
    pub world: WorldSpec,
    pub settings: SimSettings,
    pub seed: u64,
    /// Steps recorded.
    pub steps: u64,
    /// Input changes only.
    pub frames: Vec<RecFrame>,
    /// The state hash after the last step, to check a replay.
    pub final_hash: Option<u64>,
}

/// Records the frames a sim consumes.
pub struct Recorder {
    rec: Recording,
}

impl Recorder {
    pub fn new(
        profile: &SimProfile,
        world: &WorldSpec,
        settings: &SimSettings,
        seed: u64,
    ) -> Recorder {
        Recorder {
            rec: Recording {
                version: RECORDING_VERSION,
                profile: profile.clone(),
                world: world.clone(),
                settings: settings.clone(),
                seed,
                steps: 0,
                frames: Vec::new(),
                final_hash: None,
            },
        }
    }

    /// The frame step `step` consumes (call once per step, in order).
    pub fn push(&mut self, step: u64, f: &RcFrame) {
        let changed = match self.rec.frames.last() {
            Some(l) => l.ch != f.ch || l.flags != f.flags,
            None => true,
        };
        if changed {
            self.rec.frames.push(RecFrame {
                step,
                ch: f.ch,
                flags: f.flags,
            });
        }
        self.rec.steps = step + 1;
    }

    pub fn finish(mut self, sim: &Sim) -> Recording {
        self.rec.final_hash = Some(sim.state_hash());
        self.rec
    }
}

impl Recording {
    /// A fresh sim for this recording, before its first step.
    pub fn sim(&self) -> Sim {
        Sim::new(
            &self.profile,
            self.world.clone(),
            self.settings.clone(),
            self.seed,
        )
    }

    /// The frame for each step, in order.
    pub fn frames_by_step(&self) -> impl Iterator<Item = RcFrame> + '_ {
        let mut i = 0usize;
        let mut cur = RcFrame::default();
        (0..self.steps).map(move |step| {
            while i < self.frames.len() && self.frames[i].step <= step {
                cur.ch = self.frames[i].ch;
                cur.flags = self.frames[i].flags;
                i += 1;
            }
            cur
        })
    }

    /// Replay every step; returns the sim at the end.
    pub fn replay(&self) -> Sim {
        let mut sim = self.sim();
        for f in self.frames_by_step() {
            sim.step(&f);
        }
        sim
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("recording serialises")
    }

    pub fn from_json(s: &str) -> Result<Recording, String> {
        let r: Recording = serde_json::from_str(s).map_err(|e| e.to_string())?;
        if r.version != RECORDING_VERSION {
            return Err(format!(
                "recording version {} (expected {RECORDING_VERSION})",
                r.version
            ));
        }
        Ok(r)
    }
}
