//! Sim adapters (design 6.6): read the rate profiles of the sims on this Mac, so the Rates
//! segment can show them next to the quad's ("sim differs from quad").
//!
//! One adapter per game, each a `Sim`: where its file lives under `$HOME`, how to read its
//! rate profiles, and the process that means the game runs. Every format is read with byte
//! spans (`Doc`): the parsed file renders back to the exact bytes it came from, and a value
//! replaced through its span changes only those bytes. The later sync package writes through
//! that, behind the plan and confirm of design 8; this package never writes.
//!
//! Sims take Betaflight-style rates. File values are stored as fractions (1.27 for the CLI's
//! 127); `FILE_SCALE` converts. The formats are the ones design 6.6 describes, read from
//! synthetic fixtures (`tests/fixtures/sims/`): never a real player's files in tests.
//!
//! Reads work while the game runs (the game may rewrite the file later). `running` is
//! reported so a later write refuses (`sim_running`, design 8.2).

pub mod liftoff;
pub mod micro;
pub mod uncrashed;
pub mod velocidrone;
pub mod zone;

use crate::gear::rates::{
    self, profile_of, RateProfileView, Rates, RatesType, ThrottleCurve, SAME_DEG_S,
};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::ops::Range;
use std::path::{Path, PathBuf};

/// A file value times this is the CLI value (a file's 1.27 is `rc_rate` 127).
pub const FILE_SCALE: f64 = 100.0;

/// A file's bytes with the spans of the values read from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doc {
    pub raw: Vec<u8>,
}

impl Doc {
    pub fn new(raw: Vec<u8>) -> Doc {
        Doc { raw }
    }

    /// The file with `span` replaced by `with`: every other byte stays as it was.
    pub fn replaced(&self, span: &Range<usize>, with: &[u8]) -> Doc {
        let mut raw = Vec::with_capacity(self.raw.len() + with.len());
        raw.extend_from_slice(&self.raw[..span.start]);
        raw.extend_from_slice(with);
        raw.extend_from_slice(&self.raw[span.end..]);
        Doc { raw }
    }

    /// The bytes of the file as read (a parsed file renders back unchanged).
    pub fn render(&self) -> &[u8] {
        &self.raw
    }
}

/// One rate profile of a sim.
#[derive(Debug, Clone, PartialEq)]
pub struct SimProfile {
    pub name: String,
    /// Betaflight-model rates; `None` when the file uses another model or holds no numbers.
    pub rates: Option<Rates>,
    /// Only Uncrashed has a throttle curve.
    pub throttle: Option<ThrottleCurve>,
    /// Why `rates` is empty, or another thing to know.
    pub note: Option<String>,
    /// The byte spans of the values read, in file order: nine rate values, then the
    /// throttle's two when present.
    pub spans: Vec<Range<usize>>,
}

/// A parsed sim file.
#[derive(Debug, Clone, PartialEq)]
pub struct SimFile {
    pub doc: Doc,
    pub profiles: Vec<SimProfile>,
}

/// One game.
pub trait Sim: Sync {
    /// The id used in the API: `liftoff`, `micro`, `uncrashed`, `zone`, `velocidrone`.
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    /// False for an adapter that ships off (Velocidrone, until a sample save exists).
    fn enabled(&self) -> bool {
        true
    }
    /// What to tell the person about the adapter when it has no files or is off.
    fn note(&self) -> Option<&'static str> {
        None
    }
    /// The process name that means the game runs (the executable's name).
    fn process(&self) -> &'static str;
    /// The rate files that exist under `home`.
    fn files(&self, home: &Path) -> Vec<PathBuf>;
    /// Reads one file.
    fn parse(&self, raw: &[u8]) -> Result<SimFile>;
    /// Reads the file at `path` (an adapter whose profile name is the file name overrides it).
    fn parse_file(&self, raw: &[u8], _path: &Path) -> Result<SimFile> {
        self.parse(raw)
    }
}

/// Every adapter, in the order the Rates segment lists them.
pub fn all() -> Vec<&'static dyn Sim> {
    vec![
        &liftoff::Liftoff,
        &micro::Micro,
        &uncrashed::Uncrashed,
        &zone::Zone,
        &velocidrone::Velocidrone,
    ]
}

/// Files under `~/Library/Application Support`.
pub fn support(home: &Path) -> PathBuf {
    home.join("Library/Application Support")
}

/// Parses a decimal text; an error names what was read.
pub fn parse_f64(text: &str, what: &str) -> Result<f64> {
    match text.trim().parse::<f64>() {
        Ok(v) if v.is_finite() => Ok(v),
        _ => bail!("{what}: {:?} is not a number", text.trim()),
    }
}

/// Betaflight-model rates from nine file values (per axis rc rate, super rate, expo).
pub fn bf_rates(v: &[f64; 9]) -> Rates {
    let axis = |i: usize| rates::RateAxis {
        rc_rate: (v[i] * FILE_SCALE).round(),
        srate: (v[i + 1] * FILE_SCALE).round(),
        expo: (v[i + 2] * FILE_SCALE).round(),
    };
    Rates {
        rates_type: RatesType::Betaflight,
        axes: [axis(0), axis(3), axis(6)],
    }
}

// ----- status -----

/// One sim profile as the Rates segment shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SimProfileView {
    pub name: String,
    /// True when QuadCam could read rates from it.
    pub supported: bool,
    pub note: Option<String>,
    /// Roll, pitch, yaw (Betaflight model); empty when unsupported.
    pub axes: Vec<rates::AxisView>,
    /// Only Uncrashed has one.
    pub throttle: Option<rates::ThrottleView>,
    /// Against the quad, when a quad was given.
    pub diff: Option<SimDiff>,
}

/// How a sim profile compares with the quad's active rate profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SimDiff {
    /// Largest difference (deg/s) per axis from what a sync would write: the quad's rates
    /// as the Betaflight model (a quad on Actual or Quick is fitted first).
    pub max_diff: Vec<f64>,
    /// The same against the quad's own curve.
    pub quad_max_diff: Vec<f64>,
    /// The fit error per axis (0 when the quad is on the Betaflight model).
    pub fit_error: Vec<f64>,
    /// True when the throttle curves differ; null when the sim has none.
    pub throttle_differs: Option<bool>,
    /// True when every axis is within `SAME_DEG_S` and the throttle curves agree.
    pub same: bool,
}

/// One file of a sim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SimFileView {
    /// With `~` for the home folder.
    pub path: String,
    pub error: Option<String>,
    pub profiles: Vec<SimProfileView>,
}

/// One sim and what QuadCam read from it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SimStatus {
    pub id: String,
    pub name: String,
    /// False for an adapter that ships off.
    pub enabled: bool,
    /// True when a rate file exists.
    pub found: bool,
    /// True while the game runs (a write would refuse).
    pub running: bool,
    pub note: Option<String>,
    pub files: Vec<SimFileView>,
    /// With a quad: true when some profile equals the quad's active rates. Null without a
    /// quad or a readable profile.
    pub in_sync: Option<bool>,
}

fn tilde(home: &Path, p: &Path) -> String {
    match p.strip_prefix(home) {
        Ok(r) => format!("~/{}", r.display()),
        Err(_) => p.display().to_string(),
    }
}

/// Compares a sim profile with the quad's active profile.
pub fn diff(sim: &SimProfile, quad: &RateProfileView) -> Option<SimDiff> {
    let rates = sim.rates?;
    let q = profile_of(quad);
    let (target, fits) = rates::to_betaflight(&q);
    let s = rates::Profile {
        rates,
        limits: [rates::MAX_RATE_DEG_S; 3],
        throttle: sim.throttle.unwrap_or_default(),
    };
    let to_target = s.max_diff(&target);
    let to_quad = s.max_diff(&q);
    let throttle_differs = sim
        .throttle
        .map(|t| rates::throttle_differs(&t, &q.throttle));
    let same = to_target.iter().all(|&d| d <= SAME_DEG_S) && throttle_differs != Some(true);
    Some(SimDiff {
        max_diff: to_target.to_vec(),
        quad_max_diff: to_quad.to_vec(),
        fit_error: fits.iter().map(|f| f.max_diff).collect(),
        throttle_differs,
        same,
    })
}

fn profile_view(p: &SimProfile, quad: Option<&RateProfileView>) -> SimProfileView {
    let Some(r) = p.rates else {
        return SimProfileView {
            name: p.name.clone(),
            supported: false,
            note: p.note.clone(),
            axes: Vec::new(),
            throttle: None,
            diff: None,
        };
    };
    let prof = rates::Profile {
        rates: r,
        limits: [rates::MAX_RATE_DEG_S; 3],
        throttle: p.throttle.unwrap_or_default(),
    };
    let v = prof.view(0, Some(p.name.clone()), false, true);
    SimProfileView {
        name: p.name.clone(),
        supported: true,
        note: p.note.clone(),
        axes: v.axes,
        throttle: p.throttle.map(|_| v.throttle),
        diff: quad.and_then(|q| diff(p, q)),
    }
}

/// Reads every enabled sim under `home`. `running` says whether a process name runs;
/// `quad` is the active rate profile to compare with.
pub fn status(
    home: &Path,
    running: &dyn Fn(&str) -> bool,
    quad: Option<&RateProfileView>,
) -> Vec<SimStatus> {
    all()
        .into_iter()
        .map(|sim| {
            let mut files = Vec::new();
            if sim.enabled() {
                for path in sim.files(home) {
                    let shown = tilde(home, &path);
                    let parsed = std::fs::read(&path)
                        .map_err(anyhow::Error::from)
                        .and_then(|raw| sim.parse_file(&raw, &path));
                    files.push(match parsed {
                        Ok(f) => SimFileView {
                            path: shown,
                            error: None,
                            profiles: f.profiles.iter().map(|p| profile_view(p, quad)).collect(),
                        },
                        Err(e) => SimFileView {
                            path: shown,
                            error: Some(format!("{e:#}")),
                            profiles: Vec::new(),
                        },
                    });
                }
            }
            let readable = files
                .iter()
                .flat_map(|f| &f.profiles)
                .any(|p| p.diff.is_some());
            let in_sync = (quad.is_some() && readable).then(|| {
                files
                    .iter()
                    .flat_map(|f| &f.profiles)
                    .any(|p| p.diff.as_ref().is_some_and(|d| d.same))
            });
            SimStatus {
                id: sim.id().into(),
                name: sim.name().into(),
                enabled: sim.enabled(),
                found: !files.is_empty(),
                running: sim.enabled() && running(sim.process()),
                note: sim.note().map(String::from),
                files,
                in_sync,
            }
        })
        .collect()
}

/// The names of the processes running now (`ps`), or, in a process started by cargo, the
/// ones `QUADCAM_SIMS_RUNNING` lists (comma separated) so no test reads the real process
/// table.
pub fn running_processes() -> Vec<String> {
    if std::env::var_os("CARGO_MANIFEST_DIR").is_some() {
        return std::env::var("QUADCAM_SIMS_RUNNING")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    let Ok(out) = std::process::Command::new("/bin/ps")
        .args(["-axco", "comm"])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .collect()
}

/// A text rendering for the CLI and MCP.
pub fn render_text(list: &[SimStatus]) -> String {
    let mut out = String::new();
    for s in list {
        let state = if !s.enabled {
            "off".to_string()
        } else if !s.found {
            "no rate file found".to_string()
        } else {
            match s.in_sync {
                Some(true) => "matches the quad".to_string(),
                Some(false) => "differs from the quad".to_string(),
                None => "read".to_string(),
            }
        };
        out.push_str(&format!(
            "{}{} - {state}\n",
            s.name,
            if s.running { " (running)" } else { "" }
        ));
        if let Some(n) = &s.note {
            out.push_str(&format!("  {n}\n"));
        }
        for f in &s.files {
            out.push_str(&format!("  {}\n", f.path));
            if let Some(e) = &f.error {
                out.push_str(&format!("    cannot read: {e}\n"));
            }
            for p in &f.profiles {
                if !p.supported {
                    out.push_str(&format!(
                        "    {} - {}\n",
                        p.name,
                        p.note.as_deref().unwrap_or("not read")
                    ));
                    continue;
                }
                let a = &p.axes;
                out.push_str(&format!(
                    "    {} - max {:.0}/{:.0}/{:.0} deg/s{}\n",
                    p.name,
                    a[0].max_deg_s,
                    a[1].max_deg_s,
                    a[2].max_deg_s,
                    match &p.diff {
                        Some(d) if d.same => ", same as the quad".to_string(),
                        Some(d) => format!(
                            ", differs by up to {:.0} deg/s",
                            d.max_diff.iter().cloned().fold(0.0, f64::max)
                        ),
                        None => String::new(),
                    }
                ));
            }
        }
    }
    out
}

#[cfg(test)]
pub mod testing {
    //! Synthetic sim files: the fixtures' builders.
    pub fn gvas(floats: &[f32]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"GVAS");
        b.extend_from_slice(&[3, 0, 0, 0, 10, 2, 0, 0, 5, 0, 0, 0]);
        for s in ["Rates", "ArrayProperty", "FloatProperty"] {
            b.extend_from_slice(&(s.len() as i32 + 1).to_le_bytes());
            b.extend_from_slice(s.as_bytes());
            b.push(0);
        }
        b.push(0);
        b.extend_from_slice(&(floats.len() as i32).to_le_bytes());
        for f in floats {
            b.extend_from_slice(&f.to_le_bytes());
        }
        b.extend_from_slice(&5i32.to_le_bytes());
        b.extend_from_slice(b"None\0");
        b
    }
}
