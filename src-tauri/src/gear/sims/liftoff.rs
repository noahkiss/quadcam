//! Liftoff (LuGus Studios): `UserData.xml`, each rate profile its own `<rateProfiles>`
//! element. A profile has `profileName` and three vectors, `rcRate`, `superRate` and `expo`,
//! each with `x` (roll), `y` (pitch) and `z` (yaw) as fractions (1.27 for the CLI's 127).
//! Betaflight model; no throttle curve.
//!
//! The scanner below reads tags and leaf text with byte spans and keeps everything else
//! (the declaration, comments, whitespace, other elements) untouched. `micro` shares it.

use super::{bf_rates, parse_f64, support, Doc, Sim, SimFile, SimProfile};
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
}

/// A leaf element: its path from the root, the `rateProfiles` block it sits in (counted in
/// file order), and the span of its text.
#[derive(Debug)]
struct Leaf {
    path: Vec<String>,
    block: Option<usize>,
    text: Range<usize>,
}

fn scan(raw: &[u8]) -> Result<Vec<Leaf>> {
    let s = std::str::from_utf8(raw).map_err(|_| anyhow::anyhow!("the file is not UTF-8 text"))?;
    let b = s.as_bytes();
    let mut stack: Vec<(String, usize)> = Vec::new(); // (name, text start)
    let mut leaves = Vec::new();
    let mut blocks = 0usize;
    let mut block: Option<usize> = None;
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
        if let Some(name) = tag.strip_prefix('/') {
            let name = name.trim();
            let Some((open, text_start)) = stack.pop() else {
                bail!("</{name}> closes nothing at byte {i}");
            };
            if open != name {
                bail!("</{name}> closes <{open}> at byte {i}");
            }
            if last_open_end == Some(text_start) {
                // Opened and closed with only text between: a leaf.
                let mut path: Vec<String> = stack.iter().map(|(n, _)| n.clone()).collect();
                path.push(open.clone());
                leaves.push(Leaf {
                    path,
                    block,
                    text: text_start..i,
                });
            }
            if open == "rateProfiles" {
                block = None;
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
                if name == "rateProfiles" {
                    block = Some(blocks);
                    blocks += 1;
                }
                last_open_end = Some(tag_end);
                stack.push((name, tag_end));
            }
        }
        i = tag_end;
    }
    if let Some((open, _)) = stack.last() {
        bail!("<{open}> is never closed");
    }
    Ok(leaves)
}

/// Reads every `<rateProfiles>` element of a Liftoff-style `UserData.xml`.
pub fn parse_xml(raw: &[u8]) -> Result<SimFile> {
    let leaves = scan(raw)?;
    let text = |l: &Leaf| std::str::from_utf8(&raw[l.text.clone()]).unwrap_or("");
    let n_blocks = leaves
        .iter()
        .filter_map(|l| l.block)
        .max()
        .map_or(0, |m| m + 1);
    let mut profiles = Vec::new();
    for blk in 0..n_blocks {
        let ls: Vec<&Leaf> = leaves.iter().filter(|l| l.block == Some(blk)).collect();
        let find = |parent: &str, axis: &str| {
            ls.iter().find(|l| {
                let n = l.path.len();
                n >= 2 && l.path[n - 1] == axis && l.path[n - 2] == parent
            })
        };
        let name = ls
            .iter()
            .find(|l| l.path.last().is_some_and(|n| n == "profileName"))
            .map(|l| text(l).trim().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("Profile {}", blk + 1));
        let mut vals = [0.0; 9];
        let mut spans = Vec::new();
        let mut missing = None;
        'axes: for (a, axis) in ["x", "y", "z"].iter().enumerate() {
            for (k, parent) in ["rcRate", "superRate", "expo"].iter().enumerate() {
                match find(parent, axis) {
                    Some(l) => {
                        vals[a * 3 + k] = parse_f64(text(l), &format!("{name} {parent}.{axis}"))?;
                        spans.push(l.text.clone());
                    }
                    None => {
                        missing = Some(format!("{parent}.{axis}"));
                        break 'axes;
                    }
                }
            }
        }
        profiles.push(match missing {
            None => SimProfile {
                name,
                rates: Some(bf_rates(&vals)),
                throttle: None,
                note: None,
                spans,
            },
            Some(m) => SimProfile {
                name,
                rates: None,
                throttle: None,
                note: Some(format!("{m} is missing")),
                spans: Vec::new(),
            },
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
        assert_eq!(f.profiles.len(), 2);
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
    }

    #[test]
    fn replacing_a_span_changes_only_those_bytes() {
        let f = Liftoff.parse(FIXTURE.as_bytes()).unwrap();
        let span = f.profiles[0].spans[0].clone();
        let doc = f.doc.replaced(&span, b"1.50");
        let again = Liftoff.parse(doc.render()).unwrap();
        assert_eq!(again.profiles[0].rates.unwrap().axes[0].rc_rate, 150.0);
        assert_eq!(again.profiles[0].rates.unwrap().axes[1].rc_rate, 127.0);
        let a = FIXTURE.as_bytes();
        let b = doc.render();
        assert_eq!(&a[..span.start], &b[..span.start]);
        assert_eq!(&a[span.end..], &b[span.start + 4..]);
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
    fn a_profile_without_a_vector_is_listed_but_not_read() {
        let xml =
            "<UserData><rateProfiles><profileName>Odd</profileName></rateProfiles></UserData>";
        let f = Liftoff.parse(xml.as_bytes()).unwrap();
        assert_eq!(f.profiles.len(), 1);
        assert!(f.profiles[0].rates.is_none());
        assert!(f.profiles[0].note.as_deref().unwrap().contains("missing"));
    }
}
