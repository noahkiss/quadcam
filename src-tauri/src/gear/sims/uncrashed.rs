//! Uncrashed: one Unreal GVAS file per profile, `rates/<NAME>.sav`, the name being the file
//! name. After the property type `FloatProperty\0` and one byte comes an i32 count (12) and
//! 12 little-endian f32: per axis (roll, pitch, yaw) super rate, RC rate and expo; then the
//! rates type (0 = Betaflight), throttle mid and throttle expo. Values are fractions, as in
//! Liftoff. An empty profile has no floats. The only sim with a throttle curve.

use super::{support, Doc, Sim, SimFile, SimProfile, FILE_SCALE};
use crate::gear::rates::{RateAxis, Rates, RatesType, ThrottleCurve};
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

pub struct Uncrashed;

const MARKER: &[u8] = b"FloatProperty\0";
const COUNT: usize = 12;

impl Sim for Uncrashed {
    fn id(&self) -> &'static str {
        "uncrashed"
    }
    fn name(&self) -> &'static str {
        "Uncrashed"
    }
    fn process(&self) -> &'static str {
        "Uncrashed"
    }
    fn files(&self, home: &Path) -> Vec<PathBuf> {
        let root = support(home).join("Uncrashed");
        let mut out = Vec::new();
        for id in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            for f in std::fs::read_dir(id.path().join("rates"))
                .into_iter()
                .flatten()
                .flatten()
            {
                if f.path().extension().is_some_and(|e| e == "sav") {
                    out.push(f.path());
                }
            }
        }
        out.sort();
        out
    }
    fn parse(&self, raw: &[u8]) -> Result<SimFile> {
        parse_gvas(raw, "Profile")
    }
    fn parse_file(&self, raw: &[u8], path: &Path) -> Result<SimFile> {
        named(raw, path)
    }
}

/// Reads a GVAS rates file, naming its one profile.
pub fn parse_gvas(raw: &[u8], name: &str) -> Result<SimFile> {
    if !raw.starts_with(b"GVAS") {
        bail!("not an Unreal save (no GVAS header)");
    }
    let at = raw
        .windows(MARKER.len())
        .position(|w| w == MARKER)
        .ok_or_else(|| anyhow::anyhow!("no float array in the save"))?;
    let count_at = at + MARKER.len() + 1;
    let Some(c) = raw.get(count_at..count_at + 4) else {
        bail!("the save ends inside the float array");
    };
    let count = i32::from_le_bytes([c[0], c[1], c[2], c[3]]);
    let profile = if count == 0 {
        SimProfile {
            name: name.into(),
            rates: None,
            throttle: None,
            note: Some("empty profile".into()),
            spans: Vec::new(),
        }
    } else if count as usize == COUNT {
        let mut v = [0.0f64; COUNT];
        let mut spans = Vec::new();
        for (i, slot) in v.iter_mut().enumerate() {
            let s = count_at + 4 + i * 4;
            let Some(b) = raw.get(s..s + 4) else {
                bail!("the save ends inside the float array");
            };
            *slot = f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64;
            spans.push(s..s + 4);
        }
        // File order per axis: super, RC rate, expo. Rates take rc, super, expo.
        let axis = |i: usize| RateAxis {
            rc_rate: (v[i + 1] * FILE_SCALE).round(),
            srate: (v[i] * FILE_SCALE).round(),
            expo: (v[i + 2] * FILE_SCALE).round(),
        };
        if v.iter().any(|x| !x.is_finite()) {
            bail!("the save holds a value that is not a number");
        }
        if v[9] == 0.0 {
            SimProfile {
                name: name.into(),
                rates: Some(Rates {
                    rates_type: RatesType::Betaflight,
                    axes: [axis(0), axis(3), axis(6)],
                }),
                throttle: Some(ThrottleCurve {
                    mid: (v[10] * FILE_SCALE).round(),
                    expo: (v[11] * FILE_SCALE).round(),
                    ..ThrottleCurve::default()
                }),
                note: None,
                spans,
            }
        } else {
            SimProfile {
                name: name.into(),
                rates: None,
                throttle: None,
                note: Some(format!(
                    "uses rates type {} (QuadCam reads the Betaflight type)",
                    v[9]
                )),
                spans,
            }
        }
    } else {
        bail!("{count} floats where {COUNT} are expected");
    };
    Ok(SimFile {
        doc: Doc::new(raw.to_vec()),
        profiles: vec![profile],
    })
}

/// A profile file's name: the file stem.
pub fn named(raw: &[u8], path: &Path) -> Result<SimFile> {
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Profile".into());
    parse_gvas(raw, &name)
}

#[cfg(test)]
mod tests {
    use super::super::testing::gvas;
    use super::*;

    fn floats(kind: f32) -> Vec<f32> {
        // roll, pitch, yaw: super, rc, expo; type; throttle mid, expo.
        vec![
            0.72, 1.27, 0.40, 0.72, 1.27, 0.40, 0.75, 1.00, 0.0, kind, 0.30, 0.50,
        ]
    }

    #[test]
    fn reads_rates_and_throttle_and_round_trips() {
        let raw = gvas(&floats(0.0));
        let f = parse_gvas(&raw, "Freestyle").unwrap();
        assert_eq!(f.doc.render(), raw.as_slice());
        let p = &f.profiles[0];
        assert_eq!(p.name, "Freestyle");
        let r = p.rates.unwrap();
        assert_eq!(r.axes[0].rc_rate, 127.0);
        assert_eq!(r.axes[0].srate, 72.0);
        assert_eq!(r.axes[0].expo, 40.0);
        assert_eq!(r.axes[2].srate, 75.0);
        let t = p.throttle.unwrap();
        assert_eq!((t.mid, t.expo), (30.0, 50.0));
        assert_eq!(p.spans.len(), 12);
    }

    #[test]
    fn replacing_a_float_changes_only_its_four_bytes() {
        let raw = gvas(&floats(0.0));
        let f = parse_gvas(&raw, "x").unwrap();
        let span = f.profiles[0].spans[1].clone();
        let doc = f.doc.replaced(&span, &1.5f32.to_le_bytes());
        let again = parse_gvas(doc.render(), "x").unwrap();
        assert_eq!(again.profiles[0].rates.unwrap().axes[0].rc_rate, 150.0);
        let diff = raw.iter().zip(doc.render()).filter(|(a, b)| a != b).count();
        assert!(diff <= 4 && diff > 0);
    }

    #[test]
    fn another_rates_type_and_an_empty_profile_are_listed_not_read() {
        let f = parse_gvas(&gvas(&floats(1.0)), "Actual").unwrap();
        assert!(f.profiles[0].rates.is_none());
        assert!(f.profiles[0].note.as_deref().unwrap().contains("type"));
        let f = parse_gvas(&gvas(&[]), "Empty").unwrap();
        assert_eq!(f.profiles[0].note.as_deref(), Some("empty profile"));
    }

    #[test]
    fn a_bad_save_is_an_error() {
        assert!(parse_gvas(b"nope", "x").is_err());
        assert!(parse_gvas(&gvas(&[1.0, 2.0]), "x").is_err());
        let mut cut = gvas(&floats(0.0));
        cut.truncate(60);
        assert!(parse_gvas(&cut, "x").is_err());
    }
}
