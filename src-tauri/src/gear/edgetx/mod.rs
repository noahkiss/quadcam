//! The EdgeTX card engine (design 6.3): read and edit an EdgeTX SD card's YAML files line
//! by line, plan card edits with their checks and diff, and write them file by file.
//!
//! - `yaml`: the line-level reader and editor. Never a generic YAML round trip.
//! - `model`: typed views of a model file and the `ModelOp` edits.
//! - `editors`: the model editors' ops (logging, alarms, callouts, value screens, timer
//!   checks, checklist text) and the reads that show them.
//! - `card`: the card: identity, models, the selected model, the radio clock; plans
//!   (`Card::plan`, writes nothing) and the writer (`card::write`, called only by the
//!   apply engine), and `card::release` (unmount, before "safe to unplug").
//! - `radio`: the typed view of `radio.yml` for the radio's Settings editor, and the check
//!   of each value a `RadioOp::SetScalar` writes.
//! - `board`: each board's controls (switches, custom switches, pots, sliders), its
//!   model-only keys and storage speed; the control map a model copied between radios
//!   goes through.
//! - `setup`: a radio setup as data (jobs, assignments, template, aircraft) and its
//!   generator, QuadCam's own Lua screens and the steps after a flash.
//! - `synth`: the synthetic card for tests and the mock core.
//!
//! QuadCam learned the format from EdgeTX's files and documentation and copies no EdgeTX
//! code (EdgeTX is GPL-2.0; QuadCam is MIT).

pub mod board;
pub mod card;
pub mod cli;
pub mod editors;
pub mod model;
pub mod radio;
pub mod setup;
pub mod synth;
pub mod yaml;
