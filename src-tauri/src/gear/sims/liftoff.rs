//! Liftoff (LuGus Studios): `UserData.xml`, each rate profile its own `<rateProfiles>`
//! element, checked against a real install:
//!
//! ```xml
//! <rateProfiles>
//!   <name>Freestyle</name>
//!   <rates xsi:type="BetaFlight">
//!     <Roll><Rate>127</Rate><Expo>40</Expo><SuperExpo>72</SuperExpo></Roll>
//!     <Pitch>...</Pitch>
//!     <Yaw>...</Yaw>
//!   </rates>
//! </rateProfiles>
//! ```
//!
//! The values are the CLI's whole numbers (`Rate` is `rc_rate`, `SuperExpo` is `srate`).
//! Betaflight model; no throttle curve. `micro` shares the reader: there one `<rateProfiles>`
//! holds `<FlightRatesProfile>` children.
//!
//! The scanner below reads tags and leaf text with byte spans and keeps everything else
//! (the declaration, comments, whitespace, other elements) untouched.

use super::{parse_f64, support, Doc, Sim, SimFile, SimProfile, Slot};
use crate::gear::rates::{RateAxis, Rates, RatesType};
use anyhow::{bail, Result};
use std::ops::Range;
use std::path::{Path, PathBuf};

pub struct Liftoff;

impl Sim for Liftoff {
    fn id(&self) -> &'static str {
        "liftoff"
    }
    fn name(&self) -> &'static str {
        "Liftoff"
    }
    fn process(&self) -> &'static str {
        "Liftoff"
    }
    fn files(&self, home: &Path) -> Vec<PathBuf> {
        let p = support(home).join("LuGus Studios/Liftoff/Saves/Player/UserData.xml");
        p.is_file().then_some(p).into_iter().collect()
    }
    fn parse(&self, raw: &[u8]) -> Result<SimFile> {
        parse_xml(raw)
    }
    fn slots(&self) -> Vec<Slot> {
        xml_slots()
    }
    fn encode(&self, cli: f64) -> Vec<u8> {
        xml_number(cli)
    }
}

/// The order `parse_xml` lists the spans in: per axis, Rate, Expo, SuperExpo.
pub fn xml_slots() -> Vec<Slot> {
    (0..3)
        .flat_map(|a| [Slot::Rc(a), Slot::Expo(a), Slot::Super(a)])
        .collect()
}

/// A CLI value as the file writes it: whole numbers plain.
pub fn xml_number(cli: f64) -> Vec<u8> {
    if cli.fract() == 0.0 {
        format!("{}", cli as i64).into_bytes()
    } else {
        format!("{cli}").into_bytes()
    }
}

/// A leaf element: its path from the root, the profile block it sits in (counted in file
/// order), and the span of its text.
#[derive(Debug)]
struct Leaf {
    path: Vec<String>,
    block: Option<usize>,
    text: Range<usize>,
}

/// What the scan found: the leaves and each block's `<rates xsi:type="...">`.
struct Scan {
    leaves: Vec<Leaf>,
    types: Vec<Option<String>>,
}

/// The element names that open a profile: Liftoff's `rateProfiles` and Micro Drones'
/// `FlightRatesProfile`. A leaf belongs to the innermost one it sits in.
fn is_block(name: &str) -> bool {
    name == "rateProfiles" || name == "FlightRatesProfile"
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let at = tag.find(&format!("{name}="))? + name.len() + 1;
    let rest = tag.get(at..)?;
    let q = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let inner = &rest[1..];
    Some(inner[..inner.find(q)?].to_string())
}

fn scan(raw: &[u8]) -> Result<Scan> {
    let s = std::str::from_utf8(raw).map_err(|_| anyhow::anyhow!("the file is not UTF-8 text"))?;
    let b = s.as_bytes();
    // (name, text start, block id of this element)
    let mut stack: Vec<(String, usize, Option<usize>)> = Vec::new();
    let mut leaves = Vec::new();
    let mut types: Vec<Option<String>> = Vec::new();
    let mut i = 0;
    let mut last_open_end: Option<usize> = None;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &s[i..];
        if rest.starts_with("<?") {
            i += rest.find("?>").map(|n| n + 2).unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with("<!--") {
            i += rest.find("-->").map(|n| n + 3).unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            i += rest.find("]]>").map(|n| n + 3).unwrap_or(rest.len());
            continue;
        }
        let Some(end) = rest.find('>') else {
            bail!("a tag is not closed at byte {i}");
        };
        let tag = &rest[1..end];
        let tag_end = i + end + 1;
        let inner_block = |stack: &[(String, usize, Option<usize>)]| {
            stack.iter().rev().find_map(|(_, _, blk)| *blk)
        };
        if let Some(name) = tag.strip_prefix('/') {
            let name = name.trim();
            let Some((open, text_start, _)) = stack.pop() else {
                bail!("</{name}> closes nothing at byte {i}");
            };
            if open != name {
                bail!("</{name}> closes <{open}> at byte {i}");
            }
            if last_open_end == Some(text_start) {
                // Opened and closed with only text between: a leaf.
                let mut path: Vec<String> = stack.iter().map(|(n, _, _)| n.clone()).collect();
                path.push(open.clone());
                leaves.push(Leaf {
                    path,
                    block: inner_block(&stack),
                    text: text_start..i,
                });
            }
        } else {
            let self_closing = tag.ends_with('/');
            let name = tag
                .trim_end_matches('/')
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            if !self_closing {
                let mut blk = None;
                if is_block(&name) {
                    blk = Some(types.len());
                    types.push(None);
                }
                if name == "rates" {
                    if let Some(id) = inner_block(&stack) {
                        types[id] = attr(tag, "xsi:type");
                    }
                }
                last_open_end = Some(tag_end);
                stack.push((name, tag_end, blk));
            }
        }
        i = tag_end;
    }
    if let Some((open, _, _)) = stack.last() {
        bail!("<{open}> is never closed");
    }
    Ok(Scan { leaves, types })
}

const AXES: [&str; 3] = ["Roll", "Pitch", "Yaw"];
const FIELDS: [&str; 3] = ["Rate", "Expo", "SuperExpo"];

/// Reads every rate profile of a Liftoff-style `UserData.xml`.
pub fn parse_xml(raw: &[u8]) -> Result<SimFile> {
    let Scan { leaves, types } = scan(raw)?;
    let text = |l: &Leaf| std::str::from_utf8(&raw[l.text.clone()]).unwrap_or("");
    let mut profiles = Vec::new();
    for (blk, ty) in types.iter().enumerate() {
        let ls: Vec<&Leaf> = leaves.iter().filter(|l| l.block == Some(blk)).collect();
        // A wrapper (Micro Drones' outer `rateProfiles`) has no leaves of its own.
        if ls.is_empty() {
            continue;
        }
        let find = |axis: &str, field: &str| {
            ls.iter().find(|l| {
                let n = l.path.len();
                n >= 2 && l.path[n - 1] == field && l.path[n - 2] == axis
            })
        };
        let name = ls
            .iter()
            .find(|l| {
                l.path.last().is_some_and(|n| n == "name")
                    && l.path.len() >= 2
                    && is_block(&l.path[l.path.len() - 2])
            })
            .map(|l| text(l).trim().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("Profile {}", profiles.len() + 1));
        let mut vals = [0.0; 9];
        let mut spans = Vec::new();
        let mut missing = None;
        'axes: for (a, axis) in AXES.iter().enumerate() {
            for field in FIELDS {
                match find(axis, field) {
                    Some(l) => {
                        vals[a * 3 + FIELDS.iter().position(|f| *f == field).unwrap()] =
                            parse_f64(text(l), &format!("{name} {axis}.{field}"))?;
                        spans.push(l.text.clone());
                    }
                    None => {
                        missing = Some(format!("{axis}.{field}"));
                        break 'axes;
                    }
                }
            }
        }
        let other_type = ty
            .as_deref()
            .filter(|t| !t.eq_ignore_ascii_case("BetaFlight"));
        profiles.push(match (missing, other_type) {
            (Some(m), _) => SimProfile {
                name,
                rates: None,
                throttle: None,
                note: Some(format!("{m} is missing")),
                spans: Vec::new(),
            },
            (None, Some(t)) => SimProfile {
                name,
                rates: None,
                throttle: None,
                note: Some(format!("uses the {t} type (QuadCam reads Betaflight)")),
                spans,
            },
            (None, None) => {
                // File order per axis: Rate (rc), Expo, SuperExpo.
                let axis = |a: usize| RateAxis {
                    rc_rate: vals[a * 3].round(),
                    expo: vals[a * 3 + 1].round(),
                    srate: vals[a * 3 + 2].round(),
                };
                SimProfile {
                    name,
                    rates: Some(Rates {
                        rates_type: RatesType::Betaflight,
                        axes: [axis(0), axis(1), axis(2)],
                    }),
                    throttle: None,
                    note: None,
                    spans,
                }
            }
        });
    }
    Ok(SimFile {
        doc: Doc::new(raw.to_vec()),
        profiles,
    })
}

#[cfg(test)]
pub const FIXTURE: &str = include_str!("../../../tests/fixtures/sims/liftoff.UserData.xml");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_each_rate_profile_and_round_trips_byte_exact() {
        let f = Liftoff.parse(FIXTURE.as_bytes()).unwrap();
        assert_eq!(f.doc.render(), FIXTURE.as_bytes());
        assert_eq!(f.profiles.len(), 3);
        let p = &f.profiles[0];
        assert_eq!(p.name, "Freestyle");
        let r = p.rates.unwrap();
        assert_eq!(r.axes[0].rc_rate, 127.0);
        assert_eq!(r.axes[0].srate, 72.0);
        assert_eq!(r.axes[0].expo, 40.0);
        assert_eq!(r.axes[2].rc_rate, 100.0);
        assert_eq!(r.axes[2].srate, 75.0);
        assert!(p.throttle.is_none());
        assert_eq!(f.profiles[1].name, "Race");
        assert_eq!(p.spans.len(), 9);
        // The third profile uses another model: listed, not read.
        assert!(f.profiles[2].rates.is_none());
        assert!(f.profiles[2].note.as_deref().unwrap().contains("Actual"));
    }

    #[test]
    fn replacing_a_span_changes_only_those_bytes() {
        let f = Liftoff.parse(FIXTURE.as_bytes()).unwrap();
        let span = f.profiles[0].spans[0].clone();
        let doc = f.doc.replaced(&span, b"150");
        let again = Liftoff.parse(doc.render()).unwrap();
        assert_eq!(again.profiles[0].rates.unwrap().axes[0].rc_rate, 150.0);
        assert_eq!(again.profiles[0].rates.unwrap().axes[1].rc_rate, 127.0);
        let a = FIXTURE.as_bytes();
        let b = doc.render();
        assert_eq!(&a[..span.start], &b[..span.start]);
        assert_eq!(&a[span.end..], &b[span.start + 3..]);
    }

    #[test]
    fn a_broken_file_is_an_error_not_a_guess() {
        assert!(Liftoff.parse(b"<a><b></a>").is_err());
        assert!(Liftoff.parse(b"<a>").is_err());
        assert!(Liftoff.parse(&[0xff, 0xfe]).is_err());
        let f = Liftoff.parse(b"<UserData></UserData>").unwrap();
        assert!(f.profiles.is_empty());
    }

    #[test]
    fn a_profile_without_an_axis_is_listed_but_not_read() {
        let xml = "<UserData><rateProfiles><name>Odd</name></rateProfiles></UserData>";
        let f = Liftoff.parse(xml.as_bytes()).unwrap();
        assert_eq!(f.profiles.len(), 1);
        assert!(f.profiles[0].rates.is_none());
        assert!(f.profiles[0].note.as_deref().unwrap().contains("missing"));
    }
}
