//! The sim's integration tests: one binary, one module per area. `tests/bench.rs` stays a
//! binary of its own: see Cargo.toml.

mod common;
mod fc_behaviour;
mod fit;
mod physics;
mod runner;
mod validation;
