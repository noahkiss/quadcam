//! `quadcam-sim`: QuadCam's FPV simulator core (`docs/sim-design.md`).
//!
//! Physics, the Betaflight-style flight controller, rates, the profile format and presets,
//! the real-time stepping thread, snapshots and recordings. No Tauri, window or audio
//! dependency: it builds and tests on its own.
//!
//! - [`input`] reads the radio: the raw sample ring, calibration and the link model (S4).
//!   [`ring::RadioSource`] chains them into the [`RcFrame`] each step consumes; give it to
//!   [`runner::spawn`].
//! - The renderer and audio read [`snapshot::SnapshotReader`].
//! - [`sim::Sim`] is the deterministic core; [`record`] replays it exactly.
//!
//! Frames: world x, y horizontal, z up. Body x forward, y left, z up. SI units.

// Per-motor and per-axis physics reads clearer with indices.
#![allow(clippy::needless_range_loop)]

pub mod aero;
pub mod battery;
pub mod bench;
pub mod diff;
pub mod fc;
pub mod filters;
pub mod input;
pub mod log;
pub mod mixer;
pub mod motor;
pub mod profile;
pub mod rates;
pub mod record;
pub mod ring;
pub mod rng;
pub mod runner;
pub mod sim;
pub mod snapshot;
pub mod validate;
pub mod world;

pub use profile::{preset, presets, SimProfile};
pub use rapier3d_f64;
pub use ring::{RadioSource, RcFrame, RcSource};
pub use sim::{Sim, SimSettings};
pub use snapshot::{Snapshot, SnapshotPair, SnapshotReader};
pub use world::WorldSpec;
