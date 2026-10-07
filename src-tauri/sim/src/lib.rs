//! `quadcam-sim`: QuadCam's FPV simulator core (`docs/sim-design.md`).
//!
//! Physics, the Betaflight-style flight controller, rates, the profile format and presets,
//! the real-time stepping thread, snapshots and recordings. No Tauri, window or audio
//! dependency: it builds and tests on its own.
//!
//! - Input plugs in through [`ring`]: [`ring::rc_ring`] gives the producer to the input
//!   thread and the consumer to [`runner::spawn`].
//! - The renderer and audio read [`snapshot::SnapshotReader`].
//! - [`sim::Sim`] is the deterministic core; [`record`] replays it exactly.
//!
//! Frames: world x, y horizontal, z up. Body x forward, y left, z up. SI units.

// Per-motor and per-axis physics reads clearer with indices.
#![allow(clippy::needless_range_loop)]

pub mod aero;
pub mod battery;
pub mod diff;
pub mod fc;
pub mod filters;
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
pub mod world;

pub use profile::{preset, presets, SimProfile};
pub use rapier3d_f64;
pub use ring::{rc_ring, RcConsumer, RcFrame, RcProducer, RcSource};
pub use sim::{Sim, SimSettings};
pub use snapshot::{Snapshot, SnapshotPair, SnapshotReader};
pub use world::WorldSpec;
