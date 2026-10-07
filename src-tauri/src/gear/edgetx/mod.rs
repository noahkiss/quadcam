//! The EdgeTX card engine (design 6.3): read and edit an EdgeTX SD card's YAML files line
//! by line, plan card edits with their checks and diff, and write them file by file.
//!
//! - `yaml`: the line-level reader and editor. Never a generic YAML round trip.
//! - `model`: typed views of a model file and the `ModelOp` edits.
//! - `card`: the card: identity, models, the selected model, the radio clock; plans
//!   (`Card::plan`, writes nothing) and the writer (`card::write`, called only by the
//!   apply engine), and `card::release` (unmount, before "safe to unplug").
//! - `synth`: the synthetic card for tests and the mock core.
//!
//! QuadCam learned the format from EdgeTX's files and documentation and copies no EdgeTX
//! code (EdgeTX is GPL-2.0; QuadCam is MIT).

pub mod card;
pub mod model;
pub mod synth;
pub mod yaml;
