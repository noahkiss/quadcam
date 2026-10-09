//! Velocidrone. Its save format is not known: the adapter ships off until a sample save
//! exists (design open question 1). It finds and reads nothing.

use super::{Sim, SimFile, Slot};
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

pub struct Velocidrone;

impl Sim for Velocidrone {
    fn id(&self) -> &'static str {
        "velocidrone"
    }
    fn name(&self) -> &'static str {
        "Velocidrone"
    }
    fn enabled(&self) -> bool {
        false
    }
    fn note(&self) -> Option<&'static str> {
        Some("Off until a sample save exists to read its format from.")
    }
    fn process(&self) -> &'static str {
        "Velocidrone"
    }
    fn files(&self, _home: &Path) -> Vec<PathBuf> {
        Vec::new()
    }
    fn parse(&self, _raw: &[u8]) -> Result<SimFile> {
        bail!("the Velocidrone adapter is off")
    }
    fn slots(&self) -> Vec<Slot> {
        Vec::new()
    }
    fn encode(&self, _cli: f64) -> Vec<u8> {
        Vec::new()
    }
}
