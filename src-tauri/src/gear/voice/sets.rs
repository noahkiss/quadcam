//! Line sets (design 7.4): the lists of lines a person picks to render. Each set is a data
//! file in `resources/voice/sets/` (`path,text,group,tone,why`, and for the EdgeTX lists a
//! sixth field `kinds`, the aircraft types that play the line). An aircraft set is the
//! radio's prompts, the units, the EdgeTX lines tagged with its kind, and its own file.
//! Sets combine, and a path in two sets renders once. The person's own lines (`custom`) come from
//! `gear.json`, not from here.
//!
//! The tone of a line decides its batch: `calm` status, `alert` warnings, `number`s and
//! `fun` easter eggs never share a take, so a calm word does not pick up a warning's voice.

use super::lines::{check_path, fields, Line, GROUPS};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use specta::Type;

/// The tones a line can have.
pub const TONES: &[&str] = &["calm", "alert", "number", "fun"];

/// The aircraft kinds a line can be tagged with.
pub const KINDS: &[&str] = &["quad", "heli", "plane", "glider", "car"];

/// A line with the tone that batches it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetLine {
    pub line: Line,
    pub tone: String,
    /// The aircraft kinds that play the line (empty: no aircraft set picks it by kind).
    pub kinds: Vec<String>,
}

/// What a person sees of a set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SetInfo {
    pub id: String,
    pub title: String,
    pub about: String,
    pub lines: u32,
}

struct Def {
    id: &'static str,
    title: &'static str,
    about: &'static str,
    /// The files that make up the set, in order.
    files: &'static [&'static str],
    /// Files that give a line only when it carries this kind.
    by_kind: Option<(&'static str, &'static [&'static str])>,
}

macro_rules! file {
    ($n:literal) => {
        include_str!(concat!("../../../resources/voice/sets/", $n, ".csv"))
    };
}

const FILES: &[(&str, &str)] = &[
    ("radio", file!("radio")),
    ("units", file!("units")),
    ("edgetx", file!("edgetx")),
    ("scripts", file!("scripts")),
    ("general", file!("general")),
    ("quad", file!("quad")),
    ("heli", file!("heli")),
    ("plane", file!("plane")),
    ("glider", file!("glider")),
    ("extras", file!("extras")),
    ("easter", file!("easter")),
    ("sample", file!("sample")),
];

const SETS: &[Def] = &[
    Def {
        id: "edgetx",
        title: "Full EdgeTX English",
        about: "Every prompt EdgeTX plays: the radio's own prompts, numbers and units, the model prompts, and the telemetry-script prompts.",
        files: &["radio", "units", "edgetx", "scripts"],
        by_kind: None,
    },
    Def {
        id: "quad",
        title: "FPV quad",
        about: "The radio's prompts, numbers and units, the EdgeTX prompts a quad's radio plays, and QuadCam's quad callouts.",
        files: &["radio", "units", "quad"],
        by_kind: Some(("quad", &["edgetx", "scripts"])),
    },
    Def {
        id: "heli",
        title: "Helicopter",
        about: "The radio's prompts, numbers and units, the EdgeTX prompts a helicopter's radio plays, and QuadCam's helicopter callouts.",
        files: &["radio", "units", "heli"],
        by_kind: Some(("heli", &["edgetx", "scripts"])),
    },
    Def {
        id: "plane",
        title: "Plane",
        about: "The radio's prompts, numbers and units, the EdgeTX prompts a plane's radio plays, and QuadCam's plane callouts.",
        files: &["radio", "units", "plane"],
        by_kind: Some(("plane", &["edgetx", "scripts"])),
    },
    Def {
        id: "glider",
        title: "Glider",
        about: "The radio's prompts, numbers and units, the EdgeTX prompts a glider's radio plays, and QuadCam's glider callouts.",
        files: &["radio", "units", "glider"],
        by_kind: Some(("glider", &["edgetx", "scripts"])),
    },
    Def {
        id: "car",
        title: "Car",
        about: "The radio's prompts, numbers and units, and the EdgeTX prompts a car's radio plays: drive modes, steering, gears, drift.",
        files: &["radio", "units"],
        by_kind: Some(("car", &["edgetx"])),
    },
    Def {
        id: "scripts",
        title: "Telemetry scripts",
        about: "The prompts the Betaflight, INAV and Yaapu telemetry scripts play.",
        files: &["scripts"],
        by_kind: None,
    },
    Def {
        id: "extras",
        title: "FPV extras",
        about: "More FPV callouts: arming states, battery stages, link stages, rate profiles, VTX, OSD, recording, race timing, finder, helper words.",
        files: &["extras", "general"],
        by_kind: None,
    },
    Def {
        id: "easter",
        title: "Easter eggs",
        about: "Short fun lines in original wording. Off unless a model plays them.",
        files: &["easter"],
        by_kind: None,
    },
    Def {
        id: "sample",
        title: "Sample",
        about: "A dozen hard lines for comparing voices: a bare number, short words, warnings.",
        files: &["sample"],
        by_kind: None,
    },
];

/// The set a person's own lines make.
pub const CUSTOM: &str = "custom";
/// QuadCam's own default lines (`lines.csv`).
pub const QUADCAM: &str = "quadcam";

/// The tone of a line of `lines.csv`, which has none: its group says enough.
fn tone_of_group(l: &Line) -> &'static str {
    match l.group.as_str() {
        "numbers" | "units" => "number",
        _ => {
            let t = l.text.to_lowercase();
            if [
                "low", "critical", "warning", "failsafe", "lost", "disabled", "inactive",
            ]
            .iter()
            .any(|w| t.contains(w))
            {
                "alert"
            } else {
                "calm"
            }
        }
    }
}

/// Reads `path,text,group,tone,why` records. A record that is wrong names its line.
pub fn parse(text: &str, file: &str) -> Result<Vec<SetLine>> {
    let mut out: Vec<SetLine> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        if i == 0 || raw.trim().is_empty() {
            continue;
        }
        let at = || format!("sets/{file}.csv line {}", i + 1);
        let f = fields(raw).with_context(at)?;
        let kinds: Vec<String> = f
            .get(5)
            .map(|k| k.split_whitespace().map(String::from).collect())
            .unwrap_or_default();
        if let Some(k) = kinds.iter().find(|k| !KINDS.contains(&k.as_str())) {
            bail!("{}: kind {k:?} is not one of {KINDS:?}", at());
        }
        if f.len() != 5 && f.len() != 6 {
            bail!("{} has {} fields, not 5 or 6", at(), f.len());
        }
        check_path(&f[0]).with_context(at)?;
        if !GROUPS.contains(&f[2].as_str()) {
            bail!("{}: group {:?} is not one of {GROUPS:?}", at(), f[2]);
        }
        if !TONES.contains(&f[3].as_str()) {
            bail!("{}: tone {:?} is not one of {TONES:?}", at(), f[3]);
        }
        if f[1].trim().is_empty() {
            bail!("{}: no text", at());
        }
        if out.iter().any(|o| o.line.path == f[0]) {
            bail!("{}: {} appears twice", at(), f[0]);
        }
        out.push(SetLine {
            line: Line {
                path: f[0].clone(),
                text: f[1].clone(),
                group: f[2].clone(),
                why: f[4].clone(),
            },
            tone: f[3].clone(),
            kinds,
        });
    }
    Ok(out)
}

fn file_lines(name: &str) -> Result<Vec<SetLine>> {
    let (_, text) = FILES
        .iter()
        .find(|(n, _)| *n == name)
        .with_context(|| format!("no set file {name}"))?;
    parse(text, name)
}

/// The lines of one set, in file order, repeats within it dropped.
pub fn lines_of(id: &str) -> Result<Vec<SetLine>> {
    if id == QUADCAM {
        return Ok(super::lines::builtin()?
            .into_iter()
            .map(|l| SetLine {
                tone: tone_of_group(&l).into(),
                line: l,
                kinds: Vec::new(),
            })
            .collect());
    }
    let Some(def) = SETS.iter().find(|d| d.id == id) else {
        bail!("no line set {id:?}: the sets are {}", ids().join(", "));
    };
    let mut out: Vec<SetLine> = Vec::new();
    let mut take = |l: SetLine| {
        if !out.iter().any(|o| o.line.path == l.line.path) {
            out.push(l);
        }
    };
    for f in def.files {
        file_lines(f)?.into_iter().for_each(&mut take);
    }
    if let Some((kind, files)) = def.by_kind {
        for f in files {
            file_lines(f)?
                .into_iter()
                .filter(|l| l.kinds.iter().any(|k| k == kind))
                .for_each(&mut take);
        }
    }
    Ok(out)
}

/// The ids of the sets that ship, then `quadcam` and `custom`.
pub fn ids() -> Vec<&'static str> {
    SETS.iter().map(|d| d.id).chain([QUADCAM, CUSTOM]).collect()
}

/// The sets that ship, with their line counts. `custom` is the number of the person's lines.
pub fn infos(custom: usize) -> Result<Vec<SetInfo>> {
    let mut out = Vec::new();
    for d in SETS {
        out.push(SetInfo {
            id: d.id.into(),
            title: d.title.into(),
            about: d.about.into(),
            lines: lines_of(d.id)?.len() as u32,
        });
    }
    out.push(SetInfo {
        id: QUADCAM.into(),
        title: "QuadCam default".into(),
        about: "The lines QuadCam's packs hold.".into(),
        lines: lines_of(QUADCAM)?.len() as u32,
    });
    out.push(SetInfo {
        id: CUSTOM.into(),
        title: "Your lines".into(),
        about: "The lines you added.".into(),
        lines: custom as u32,
    });
    Ok(out)
}

/// Several sets as one list. The first set to hold a path wins it; `custom` takes the
/// person's own lines (tone `calm`).
pub fn combine(ids: &[String], custom: &[Line]) -> Result<Vec<SetLine>> {
    let mut out: Vec<SetLine> = Vec::new();
    for id in ids {
        let lines = if id == CUSTOM {
            custom
                .iter()
                .map(|l| SetLine {
                    line: l.clone(),
                    tone: "calm".into(),
                    kinds: Vec::new(),
                })
                .collect()
        } else {
            lines_of(id)?
        };
        for l in lines {
            if !out.iter().any(|o| o.line.path == l.line.path) {
                out.push(l);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gear::voice::lines::Spelling;

    #[test]
    fn every_set_loads_and_has_lines() {
        for id in ids() {
            if id == CUSTOM {
                continue;
            }
            assert!(!lines_of(id).unwrap().is_empty(), "{id}");
        }
        let all = infos(0).unwrap();
        assert_eq!(all.len(), ids().len());
        assert!(all.iter().find(|s| s.id == "edgetx").unwrap().lines > 250);
        assert!(lines_of("nope")
            .unwrap_err()
            .to_string()
            .contains("no line set"));
    }

    /// EdgeTX plays the number and unit prompts, and the sounds it plays by itself, from
    /// `SOUNDS/en/SYSTEM/`; a model's own tracks sit in `SOUNDS/en/`.
    #[test]
    fn number_and_unit_prompts_sit_in_the_system_folder() {
        let mut paths: Vec<String> = super::super::lines::builtin()
            .unwrap()
            .into_iter()
            .map(|l| l.path)
            .collect();
        for id in ids() {
            if id != CUSTOM {
                paths.extend(lines_of(id).unwrap().into_iter().map(|l| l.line.path));
            }
        }
        let mut numbers = 0;
        for p in &paths {
            let name = p.rsplit('/').next().unwrap();
            let numbered = name.len() == 8 && name[..4].bytes().all(|b| b.is_ascii_digit());
            if numbered || name.starts_with("volt") {
                numbers += 1;
                assert!(p.starts_with("SOUNDS/en/SYSTEM/"), "{p}");
            }
            assert!(p.starts_with("SOUNDS/en/"), "{p}");
        }
        assert!(numbers > 80, "{numbers}");
    }

    #[test]
    fn a_path_means_one_text_in_every_file() {
        let mut seen: std::collections::BTreeMap<String, String> = Default::default();
        let builtin = lines_of(QUADCAM).unwrap();
        let sets: Vec<SetLine> = FILES
            .iter()
            .flat_map(|(n, _)| file_lines(n).unwrap())
            .chain(
                builtin
                    .into_iter()
                    .filter(|l| l.line.path.contains("/SYSTEM/") || !l.line.path.contains("/00")),
            )
            .collect();
        for l in sets {
            let t = l.line.text.to_lowercase();
            if let Some(prev) = seen.insert(l.line.path.clone(), t.clone()) {
                assert_eq!(prev, t, "{} says two different things", l.line.path);
            }
        }
    }

    #[test]
    fn the_aircraft_sets_share_the_radio_and_differ_in_callouts() {
        let quad = lines_of("quad").unwrap();
        let heli = lines_of("heli").unwrap();
        assert!(quad.iter().any(|l| l.line.path.ends_with("/hello.wav")));
        assert!(heli.iter().any(|l| l.line.path.ends_with("/hello.wav")));
        assert!(quad.iter().any(|l| l.line.path.ends_with("/turtle.wav")));
        assert!(!heli.iter().any(|l| l.line.path.ends_with("/turtle.wav")));
        assert!(heli.iter().any(|l| l.line.path.ends_with("/idleu1.wav")));
        assert!(!quad.iter().any(|l| l.line.path.ends_with("/idleu1.wav")));
    }

    #[test]
    fn sets_combine_and_a_line_in_two_sets_comes_once() {
        let both = combine(&["quad".into(), "heli".into()], &[]).unwrap();
        let paths: Vec<&str> = both.iter().map(|l| l.line.path.as_str()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), paths.len());
        assert_eq!(
            paths.iter().filter(|p| p.ends_with("/armed.wav")).count(),
            1
        );
        let n = lines_of("quad").unwrap().len() + lines_of("heli").unwrap().len();
        assert!(both.len() < n);
        // The first set keeps a shared path.
        let c = combine(
            &["sample".into(), CUSTOM.into()],
            &[Line {
                path: "SOUNDS/en/armed.wav".into(),
                text: "Mine".into(),
                group: "extras".into(),
                why: String::new(),
            }],
        )
        .unwrap();
        assert_eq!(
            c.iter()
                .filter(|l| l.line.path.ends_with("/armed.wav"))
                .count(),
            1
        );
        assert_eq!(
            c.iter()
                .find(|l| l.line.path.ends_with("/armed.wav"))
                .unwrap()
                .line
                .text,
            "Armed"
        );
    }

    #[test]
    fn a_tone_outside_the_list_is_refused() {
        assert!(parse("h\nSOUNDS/en/a.wav,x,extras,loud,w\n", "t").is_err());
        assert!(parse("h\nSOUNDS/en/a.wav,x,extras,calm\n", "t").is_err());
    }

    #[test]
    fn the_sample_set_holds_the_hard_lines() {
        let s: Vec<String> = lines_of("sample")
            .unwrap()
            .into_iter()
            .map(|l| l.line.text)
            .collect();
        assert_eq!(s.len(), 12);
        for want in [
            "Six",
            "Armed",
            "Turtle mode",
            "Battery low",
            "Signal critical",
        ] {
            assert!(s.iter().any(|t| t == want), "{want}");
        }
    }

    /// The full EdgeTX set is the upstream English list: 568 prompts plus 179 script prompts.
    #[test]
    fn the_full_edgetx_set_holds_every_prompt() {
        let all = lines_of("edgetx").unwrap();
        assert_eq!(all.len(), 747);
        let system = all
            .iter()
            .filter(|l| l.line.path.contains("/SYSTEM/"))
            .count();
        let scripts = all
            .iter()
            .filter(|l| l.line.path.contains("/SCRIPTS/"))
            .count();
        assert_eq!((system, scripts), (212, 179));
        for want in [
            "SOUNDS/en/SYSTEM/0000.wav",
            "SOUNDS/en/SYSTEM/0112.wav",
            "SOUNDS/en/SYSTEM/0176.wav",
            "SOUNDS/en/SYSTEM/telemco.wav",
            "SOUNDS/en/SYSTEM/dbm1.wav",
            "SOUNDS/en/vtxfr8.wav",
            "SOUNDS/en/drifon.wav",
            "SOUNDS/en/SCRIPTS/YAAPU/armed.wav",
            "SOUNDS/en/SCRIPTS/INAV/batcrt.wav",
        ] {
            assert!(all.iter().any(|l| l.line.path == want), "{want}");
        }
    }

    /// Every set resolves, every path is valid and unique inside it, and the counts hold.
    #[test]
    fn every_set_resolves_with_valid_unique_paths_and_counts() {
        let want = [
            ("edgetx", 747),
            ("scripts", 179),
            ("sample", 12),
            ("easter", 20),
        ];
        for (id, n) in want {
            assert_eq!(lines_of(id).unwrap().len(), n, "{id}");
        }
        for id in ["quad", "heli", "plane", "glider", "car"] {
            let l = lines_of(id).unwrap();
            assert!(l.len() > 250, "{id}: {}", l.len());
            assert!(l.len() < lines_of("edgetx").unwrap().len(), "{id}");
        }
        assert!(lines_of("extras").unwrap().len() >= 150);
        for id in ids() {
            if id == CUSTOM {
                continue;
            }
            let l = lines_of(id).unwrap();
            let mut paths: Vec<&str> = l.iter().map(|x| x.line.path.as_str()).collect();
            for p in &paths {
                check_path(p).unwrap();
            }
            paths.sort();
            paths.dedup();
            assert_eq!(paths.len(), l.len(), "{id}");
        }
    }

    /// A path is valid for EdgeTX: the file names are lower case, and nothing leaves
    /// `SOUNDS/en/`.
    #[test]
    fn paths_are_edgetx_paths() {
        for (n, _) in FILES {
            for l in file_lines(n).unwrap() {
                let p = &l.line.path;
                assert!(p.starts_with("SOUNDS/en/") && p.ends_with(".wav"), "{p}");
                assert!(!p.contains(" ") && !p.contains("//"), "{p}");
            }
        }
        assert!(check_path("SOUNDS/en/SCRIPTS/YAAPU/264977348.wav").is_ok());
        assert!(check_path("SOUNDS/en/toolongname.wav").is_err());
        assert!(check_path("SOUNDS/en/SCRIPTS/x/y/z.wav").is_err());
        assert!(check_path("SOUNDS/en/../a.wav").is_err());
    }

    /// Each tone is used, and the numbers and units are all `number`.
    #[test]
    fn the_tone_groups_are_all_present() {
        for (id, tones) in [
            ("edgetx", &["calm", "alert", "number"][..]),
            ("extras", &["calm", "alert"][..]),
        ] {
            let l = lines_of(id).unwrap();
            for t in tones {
                assert!(l.iter().any(|x| x.tone == *t), "{id} has no {t}");
            }
        }
        assert!(lines_of("easter").unwrap().iter().all(|l| l.tone == "fun"));
        for l in lines_of("edgetx").unwrap() {
            if matches!(l.line.group.as_str(), "numbers" | "units") {
                assert_eq!(l.tone, "number", "{}", l.line.path);
            } else {
                assert_ne!(l.tone, "number", "{}", l.line.path);
                assert_ne!(l.tone, "fun", "{}", l.line.path);
            }
        }
    }

    /// The aircraft sets pick the EdgeTX lines of their kind, and the kinds are real.
    #[test]
    fn the_aircraft_sets_pick_their_own_edgetx_lines() {
        let has = |id: &str, name: &str| {
            lines_of(id)
                .unwrap()
                .iter()
                .any(|l| l.line.path.ends_with(&format!("/{name}.wav")))
        };
        assert!(has("quad", "turton") && !has("heli", "turton") && !has("car", "turton"));
        assert!(has("plane", "geardn") && has("glider", "geardn") && !has("quad", "geardn"));
        assert!(has("glider", "thmmod") && !has("plane", "thmmod"));
        assert!(has("car", "drifon") && !has("quad", "drifon"));
        assert!(has("heli", "auro") && has("car", "volt1") && has("quad", "0042"));
        for (n, _) in FILES {
            for l in file_lines(n).unwrap() {
                assert!(l.kinds.iter().all(|k| KINDS.contains(&k.as_str())));
            }
        }
    }

    #[test]
    fn a_kind_outside_the_list_is_refused() {
        assert!(parse("h\nSOUNDS/en/a.wav,x,callouts,calm,w,boat\n", "t").is_err());
        let ok = parse("h\nSOUNDS/en/a.wav,x,callouts,calm,w,quad car\n", "t").unwrap();
        assert_eq!(ok[0].kinds, ["quad", "car"]);
    }

    #[test]
    fn spoken_text_has_no_stray_dots_and_no_dotted_acronyms() {
        let sp = Spelling::builtin().unwrap();
        for (n, _) in FILES {
            for l in file_lines(n).unwrap() {
                assert!(
                    !l.line.text.contains("D.V.R.") && !l.line.text.contains("G.P.S."),
                    "{}: write {:?} readable; the spelling rules dot it",
                    l.line.path,
                    l.line.text
                );
                let _ = sp.apply(&l.line.text);
            }
        }
        assert_eq!(sp.apply("DVR on"), "D.V.R. on");
    }
}
