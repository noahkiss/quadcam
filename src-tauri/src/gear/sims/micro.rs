//! Liftoff: Micro Drones. The same `UserData.xml` as Liftoff, inside the game's app bundle
//! (`Contents/Saves/Player/UserData.xml`) with the profiles wrapped in `<FlightRatesProfile>`.
//! The game is a Steam app, so the bundle sits under Steam's `steamapps/common`.

use super::liftoff::parse_xml;
use super::{support, Sim, SimFile};
use anyhow::Result;
use std::path::{Path, PathBuf};

pub struct Micro;

impl Sim for Micro {
    fn id(&self) -> &'static str {
        "micro"
    }
    fn name(&self) -> &'static str {
        "Liftoff: Micro Drones"
    }
    fn process(&self) -> &'static str {
        "Liftoff Micro Drones"
    }
    fn note(&self) -> Option<&'static str> {
        Some("The file lives inside the game's app bundle.")
    }
    fn files(&self, home: &Path) -> Vec<PathBuf> {
        let common = support(home).join("Steam/steamapps/common");
        let mut out = Vec::new();
        for dir in std::fs::read_dir(&common).into_iter().flatten().flatten() {
            if !dir.file_name().to_string_lossy().contains("Micro") {
                continue;
            }
            for app in std::fs::read_dir(dir.path())
                .into_iter()
                .flatten()
                .flatten()
            {
                let p = app.path().join("Contents/Saves/Player/UserData.xml");
                if app.path().extension().is_some_and(|e| e == "app") && p.is_file() {
                    out.push(p);
                }
            }
        }
        out.sort();
        out
    }
    fn parse(&self, raw: &[u8]) -> Result<SimFile> {
        parse_xml(raw)
    }
}

#[cfg(test)]
pub const FIXTURE: &str = include_str!("../../../tests/fixtures/sims/micro.UserData.xml");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_profiles_inside_the_wrapper_and_round_trips() {
        let f = Micro.parse(FIXTURE.as_bytes()).unwrap();
        assert_eq!(f.doc.render(), FIXTURE.as_bytes());
        assert_eq!(f.profiles.len(), 2);
        assert_eq!(f.profiles[0].name, "Micro");
        assert_eq!(f.profiles[0].rates.unwrap().axes[1].srate, 70.0);
    }
}
