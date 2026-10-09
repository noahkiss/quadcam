//! The Zone (Godot): `settings.cfg`, a Godot config file. Each `[rate_profile_N]` section has
//! `type="betaflight"` and one `Vector3(RC rate, super rate, expo)` per axis (`roll`,
//! `pitch`, `yaw`) as fractions. Other keys and sections stay as they are.

use super::{bf_rates, parse_f64, support, Doc, Sim, SimFile, SimProfile};
use anyhow::{bail, Result};
use std::ops::Range;
use std::path::{Path, PathBuf};

pub struct Zone;

impl Sim for Zone {
    fn id(&self) -> &'static str {
        "zone"
    }
    fn name(&self) -> &'static str {
        "The Zone"
    }
    fn process(&self) -> &'static str {
        "The Zone"
    }
    fn files(&self, home: &Path) -> Vec<PathBuf> {
        let p = support(home).join("Godot/app_userdata/The Zone/settings.cfg");
        p.is_file().then_some(p).into_iter().collect()
    }
    fn parse(&self, raw: &[u8]) -> Result<SimFile> {
        let text =
            std::str::from_utf8(raw).map_err(|_| anyhow::anyhow!("the file is not UTF-8 text"))?;
        /// One axis: its three numbers and their byte spans.
        type Axis = ([f64; 3], [Range<usize>; 3]);
        struct Sec {
            name: String,
            kind: Option<String>,
            axes: [Option<Axis>; 3],
        }
        let mut secs: Vec<Sec> = Vec::new();
        let mut cur: Option<usize> = None;
        let mut pos = 0;
        for line in text.split_inclusive('\n') {
            let start = pos;
            pos += line.len();
            let t = line.trim();
            if let Some(h) = t.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
                cur = h.starts_with("rate_profile_").then(|| {
                    secs.push(Sec {
                        name: h.into(),
                        kind: None,
                        axes: [None, None, None],
                    });
                    secs.len() - 1
                });
                continue;
            }
            let Some(i) = cur else { continue };
            let Some((k, v)) = t.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            if k == "type" {
                secs[i].kind = Some(v.trim_matches('"').to_string());
            } else if let Some(a) = ["roll", "pitch", "yaw"].iter().position(|n| *n == k) {
                let Some(inner) = v.strip_prefix("Vector3(").and_then(|r| r.strip_suffix(')'))
                else {
                    bail!("{} {k}: {v:?} is not a Vector3", secs[i].name);
                };
                // Byte offset of the first number in the original line.
                let open = line.find("Vector3(").unwrap() + "Vector3(".len();
                let mut vals = [0.0; 3];
                let mut spans: [Range<usize>; 3] = [0..0, 0..0, 0..0];
                let mut at = open;
                let parts: Vec<&str> = inner.split(',').collect();
                if parts.len() != 3 {
                    bail!("{} {k}: a Vector3 holds three numbers", secs[i].name);
                }
                for (n, part) in parts.iter().enumerate() {
                    let lead = part.len() - part.trim_start().len();
                    let s = start + at + lead;
                    let e = s + part.trim().len();
                    vals[n] = parse_f64(part, &format!("{} {k}", secs[i].name))?;
                    spans[n] = s..e;
                    at += part.len() + 1;
                }
                secs[i].axes[a] = Some((vals, spans));
            }
        }
        let profiles = secs
            .into_iter()
            .map(|s| {
                let name = s.name.trim_start_matches("rate_profile_");
                let name = format!("Profile {name}");
                let ty = s.kind.as_deref().unwrap_or("betaflight");
                if !ty.eq_ignore_ascii_case("betaflight") {
                    return SimProfile {
                        name,
                        rates: None,
                        throttle: None,
                        note: Some(format!("uses the {ty} type (QuadCam reads Betaflight)")),
                        spans: Vec::new(),
                    };
                }
                if s.axes.iter().any(|a| a.is_none()) {
                    return SimProfile {
                        name,
                        rates: None,
                        throttle: None,
                        note: Some("an axis is missing".into()),
                        spans: Vec::new(),
                    };
                }
                let mut v = [0.0; 9];
                let mut spans = Vec::new();
                for (a, ax) in s.axes.iter().enumerate() {
                    let (vals, sp) = ax.as_ref().unwrap();
                    v[a * 3..a * 3 + 3].copy_from_slice(vals);
                    spans.extend(sp.iter().cloned());
                }
                SimProfile {
                    name,
                    rates: Some(bf_rates(&v)),
                    throttle: None,
                    note: None,
                    spans,
                }
            })
            .collect();
        Ok(SimFile {
            doc: Doc::new(raw.to_vec()),
            profiles,
        })
    }
}

#[cfg(test)]
pub const FIXTURE: &str = include_str!("../../../tests/fixtures/sims/zone.settings.cfg");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_profiles_and_round_trips() {
        let f = Zone.parse(FIXTURE.as_bytes()).unwrap();
        assert_eq!(f.doc.render(), FIXTURE.as_bytes());
        assert_eq!(f.profiles.len(), 2);
        let r = f.profiles[0].rates.unwrap();
        assert_eq!(r.axes[0].rc_rate, 127.0);
        assert_eq!(r.axes[0].srate, 72.0);
        assert_eq!(r.axes[2].expo, 0.0);
        // The second profile uses another type.
        assert!(f.profiles[1].rates.is_none());
    }

    #[test]
    fn a_span_points_at_the_number() {
        let f = Zone.parse(FIXTURE.as_bytes()).unwrap();
        let s = f.profiles[0].spans[0].clone();
        assert_eq!(&FIXTURE.as_bytes()[s.clone()], b"1.27");
        let doc = f.doc.replaced(&s, b"1.5");
        let again = Zone.parse(doc.render()).unwrap();
        assert_eq!(again.profiles[0].rates.unwrap().axes[0].rc_rate, 150.0);
    }

    #[test]
    fn a_bad_vector_is_an_error() {
        assert!(Zone
            .parse(b"[rate_profile_0]\ntype=\"betaflight\"\nroll=Vector3(1, 2)\n")
            .is_err());
        assert!(Zone
            .parse(b"[rate_profile_0]\nroll=Vector3(a, b, c)\n")
            .is_err());
    }
}
