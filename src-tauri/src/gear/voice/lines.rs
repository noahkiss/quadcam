//! Voice lines and the spelling rules (design 7.4). `resources/voice/lines.csv` is QuadCam's
//! own text, written from what each sound means; it is not a copy of EdgeTX's voice list
//! (that file is GPL-2.0). `resources/voice/spelling.toml` turns the readable text into the
//! text a voice speaks. A user's own lines live in `gear.json`, never in the repo file and
//! never in a pack.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::BTreeMap;

/// The groups a line belongs to.
pub const GROUPS: &[&str] = &["system", "numbers", "units", "callouts", "extras"];

/// One sound on the card: its path, the text a voice reads, and why it exists.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct Line {
    /// From the card's root: `SOUNDS/en/armed.wav`, `SOUNDS/en/SYSTEM/hello.wav`.
    pub path: String,
    /// The readable text. The spelling rules turn it into what a voice speaks.
    pub text: String,
    pub group: String,
    #[serde(default)]
    pub why: String,
}

/// QuadCam's lines, from the file compiled in.
pub fn builtin() -> Result<Vec<Line>> {
    parse_csv(include_str!("../../../resources/voice/lines.csv"))
}

/// The sha256 of the compiled-in `lines.csv`, for a pack's index entry.
pub fn builtin_sha() -> String {
    super::sha256_hex(include_str!("../../../resources/voice/lines.csv").as_bytes())
}

/// Splits one CSV record into fields (quotes, doubled quotes inside).
pub(super) fn fields(line: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            ('"', true) => quoted = false,
            ('"', false) if cur.is_empty() => quoted = true,
            (',', false) => out.push(std::mem::take(&mut cur)),
            (c, _) => cur.push(c),
        }
    }
    if quoted {
        bail!("an unclosed quote");
    }
    out.push(cur);
    Ok(out)
}

/// Reads `path,text,group,why` records. A record that is wrong names its line.
pub fn parse_csv(text: &str) -> Result<Vec<Line>> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        if i == 0 || raw.trim().is_empty() {
            continue;
        }
        let f = fields(raw).with_context(|| format!("lines.csv line {}", i + 1))?;
        if f.len() != 4 {
            bail!("lines.csv line {} has {} fields, not 4", i + 1, f.len());
        }
        let line = Line {
            path: f[0].clone(),
            text: f[1].clone(),
            group: f[2].clone(),
            why: f[3].clone(),
        };
        check_path(&line.path).with_context(|| format!("lines.csv line {}", i + 1))?;
        if !GROUPS.contains(&line.group.as_str()) {
            bail!(
                "lines.csv line {}: group {:?} is not one of {GROUPS:?}",
                i + 1,
                line.group
            );
        }
        if line.text.trim().is_empty() {
            bail!("lines.csv line {}: no text", i + 1);
        }
        if out.iter().any(|o: &Line| o.path == line.path) {
            bail!("lines.csv line {}: {} appears twice", i + 1, line.path);
        }
        out.push(line);
    }
    Ok(out)
}

/// A sound's path on the card: `SOUNDS/<lang>/[SYSTEM/]<name>.wav`, the name 8 characters
/// at most, or `SOUNDS/<lang>/SCRIPTS/<dir>/<name>.wav`, which a script reads by its full
/// path (the name may be longer).
pub fn check_path(path: &str) -> Result<()> {
    let parts: Vec<&str> = path.split('/').collect();
    let ok_shape = match parts.as_slice() {
        ["SOUNDS", lang, name] => !lang.is_empty() && !name.is_empty(),
        ["SOUNDS", lang, "SYSTEM", name] => !lang.is_empty() && !name.is_empty(),
        // A telemetry script's prompts: `SOUNDS/en/SCRIPTS/YAAPU/armed.wav`.
        ["SOUNDS", lang, "SCRIPTS", dir, name] => {
            !lang.is_empty()
                && !name.is_empty()
                && !dir.is_empty()
                && dir.chars().all(|c| c.is_ascii_alphanumeric())
        }
        _ => false,
    };
    if !ok_shape {
        bail!(
            "{path:?} is not SOUNDS/<language>/<name>.wav or SOUNDS/<language>/SYSTEM/<name>.wav"
        );
    }
    let name = parts.last().unwrap();
    let max = if parts.len() == 5 {
        32
    } else {
        crate::gear::edgetx::model::MAX_TRACK_NAME
    };
    let Some(stem) = name.strip_suffix(".wav") else {
        bail!("{path:?} is not a .wav file");
    };
    if stem.is_empty()
        || stem.len() > max
        || !stem
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!("{path:?}: a sound name is 1-{max} letters, digits, - or _");
    }
    if parts[1]
        .chars()
        .any(|c| !c.is_ascii_alphanumeric() && c != '-')
    {
        bail!("{path:?}: the language folder holds letters only");
    }
    Ok(())
}

/// Spelling rules: readable word -> spoken text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spelling {
    /// Longest word first.
    rules: Vec<(String, String)>,
}

#[derive(Deserialize)]
struct SpellingFile {
    #[serde(default)]
    spell: BTreeMap<String, String>,
    #[serde(default)]
    expand: BTreeMap<String, String>,
    #[serde(default)]
    plain: Plain,
}

#[derive(Deserialize, Default)]
struct Plain {
    #[serde(default)]
    words: Vec<String>,
}

impl Spelling {
    /// The rules compiled in.
    pub fn builtin() -> Result<Spelling> {
        Self::parse(include_str!("../../../resources/voice/spelling.toml"))
    }

    pub fn parse(text: &str) -> Result<Spelling> {
        let f: SpellingFile = toml::from_str(text).context("spelling.toml")?;
        let mut rules: Vec<(String, String)> = f.spell.into_iter().chain(f.expand).collect();
        for w in &f.plain.words {
            if rules.iter().any(|(k, _)| k == w) {
                bail!("spelling.toml: {w:?} is a plain word and also has a rule");
            }
        }
        for (k, _) in &rules {
            if k.is_empty() || !k.chars().all(|c| c.is_alphanumeric()) {
                bail!("spelling.toml: rule {k:?} is not a word");
            }
        }
        rules.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        Ok(Spelling { rules })
    }

    /// The text a voice speaks: each rule applies to whole words, case-sensitive.
    pub fn apply(&self, text: &str) -> String {
        let mut out = String::new();
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if !chars[i].is_alphanumeric() {
                out.push(chars[i]);
                i += 1;
                continue;
            }
            let start = i;
            while i < chars.len() && chars[i].is_alphanumeric() {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            match self.rules.iter().find(|(k, _)| *k == word) {
                Some((_, v)) => out.push_str(v),
                None => out.push_str(&word),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compiled_in_files_load_and_every_path_is_a_sound_path() {
        let lines = builtin().unwrap();
        assert!(lines.len() > 40);
        assert!(lines.iter().any(|l| l.path == "SOUNDS/en/armed.wav"));
        Spelling::builtin().unwrap();
    }

    #[test]
    fn csv_quotes_and_errors() {
        let l = parse_csv(
            "path,text,group,why\nSOUNDS/en/a.wav,\"Hi, there \"\"you\"\"\",extras,why\n",
        )
        .unwrap();
        assert_eq!(l[0].text, "Hi, there \"you\"");
        assert!(parse_csv("p\nSOUNDS/en/a.wav,x,extras\n")
            .unwrap_err()
            .to_string()
            .contains("3 fields"));
        assert!(parse_csv("p\nSOUNDS/en/waytoolongname.wav,x,extras,w\n").is_err());
        assert!(parse_csv("p\nSOUNDS/en/a.wav,x,nogroup,w\n").is_err());
        assert!(parse_csv("p\nSOUNDS/en/a.wav,x,extras,w\nSOUNDS/en/a.wav,y,extras,w\n").is_err());
        assert!(parse_csv("p\n../x.wav,x,extras,w\n").is_err());
    }
}
