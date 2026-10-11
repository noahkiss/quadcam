//! Betaflight `dump all` and `diff all` text: a typed config with section context, the
//! version header, CLI lines rendered back, the line checks, and the transcript scrubber.
//!
//! Written from the CLI's observed output (the fixtures in `tests/fixtures/bf/`), not from
//! Betaflight's source.
//!
//! - A line belongs to the section the last `profile N`, `rateprofile N` or
//!   `battery_profile N` line selected; before the first, to `Master`. So a `set` shared by
//!   two rate profiles is found, and checked, in each.
//! - `set` lines become `Cmd::Set`; `batch`, `defaults`, `save` are control lines; other
//!   commands keep their verb and their key (the words that name what the line sets, such
//!   as `aux 3` or `resource MOTOR 1`), so a later line with the same key replaces it.
//! - `diff all` never shows a `set` equal to its default, so a check that a value is set
//!   reads `dump all` (`verify_lines`).

use crate::gear::model::{Identity, Section};
use serde::{Deserialize, Serialize};
use specta::Type;

/// What one CLI line does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    /// `set name = value`.
    Set { name: String, value: String },
    /// `profile N`, `rateprofile N`, `battery_profile N`.
    Select(Section),
    /// `batch start`, `batch end`, `defaults nosave`, `save`.
    Control,
    /// Any other command, with the words that name what it sets.
    Other { verb: String, key: String },
}

/// One line of a dump: its section, its text and what it does. Comments and blank lines
/// are kept as `None` commands so the text renders back unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub section: Section,
    pub text: String,
    pub cmd: Option<Cmd>,
}

/// The `# Betaflight / STM32G47X (G473) 2025.12.5-alpha ... (eb2bb5a33) MSP API: 1.47` line.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct VersionInfo {
    /// `Betaflight`.
    pub firmware: String,
    /// The MCU (`STM32G47X`).
    pub mcu: Option<String>,
    /// The short target in brackets (`G473`).
    pub target: Option<String>,
    /// `2025.12.5-alpha`.
    pub version: String,
    /// `Jun 25 2026 / 03:24:50`.
    pub build_date: Option<String>,
    /// The git revision in brackets (`eb2bb5a33`).
    pub git: Option<String>,
    /// `1.47`.
    pub msp_api: Option<String>,
}

/// A parsed `dump all`, `diff all` or other CLI listing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub lines: Vec<Line>,
    pub version: Option<VersionInfo>,
}

/// The words of a non-`set` line that name what it sets. Index-like words (numbers, the
/// first word after the verb for `resource`, `serial`, `timer`...) make a key; values do
/// not.
fn key_of(verb: &str, words: &[&str]) -> String {
    let n = match verb {
        // resource MOTOR 1 B00 | timer B00 AF2 | dma ADC 1 0 | serial UART1 ... | aux 0 ...
        "resource" | "dma" => 2,
        "timer" | "serial" | "aux" | "adjrange" | "rxrange" | "rxfail" | "led" | "color"
        | "mmix" | "smix" | "servo" | "vtx" => 1,
        "mode_color" => 2,
        // vtxtable band 3 ... | vtxtable powerlevels 5
        "vtxtable" if words.first() == Some(&"band") => 2,
        "vtxtable" => 1,
        // feature X / feature -X, beeper X / beeper -X: the name without the sign.
        "feature" | "beeper" | "beacon" => {
            return format!(
                "{verb} {}",
                words
                    .first()
                    .map(|w| w.trim_start_matches('-'))
                    .unwrap_or("")
            )
        }
        _ => 0,
    };
    let mut k = verb.to_string();
    for w in words.iter().take(n) {
        k.push(' ');
        k.push_str(w);
    }
    k
}

/// What one trimmed, non-comment line does.
pub fn parse_cmd(text: &str) -> Cmd {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("set ") {
        let (name, value) = match rest.split_once('=') {
            Some((n, v)) => (n.trim(), v.trim()),
            None => (rest.trim(), ""),
        };
        return Cmd::Set {
            name: name.to_string(),
            value: value.to_string(),
        };
    }
    let words: Vec<&str> = t.split_whitespace().collect();
    let verb = words.first().copied().unwrap_or("");
    let num = || words.get(1).and_then(|w| w.parse::<u8>().ok());
    match (verb, words.len()) {
        ("profile", 2) if num().is_some() => return Cmd::Select(Section::Profile(num().unwrap())),
        ("rateprofile", 2) if num().is_some() => {
            return Cmd::Select(Section::RateProfile(num().unwrap()))
        }
        ("battery_profile", 2) if num().is_some() => {
            return Cmd::Select(Section::BatteryProfile(num().unwrap()))
        }
        ("batch" | "defaults" | "save", _) => return Cmd::Control,
        _ => {}
    }
    Cmd::Other {
        verb: verb.to_string(),
        key: key_of(verb, &words[1.min(words.len())..]),
    }
}

/// Reads the version line (`# Betaflight / MCU (TARGET) VERSION DATE / TIME (GIT) MSP API: X`).
pub fn parse_version_line(line: &str) -> Option<VersionInfo> {
    let t = line.trim().trim_start_matches('#').trim();
    let (firmware, rest) = t.split_once(" / ")?;
    let firmware = firmware.trim();
    if firmware.is_empty() || firmware.contains(' ') {
        return None;
    }
    let mut v = VersionInfo {
        firmware: firmware.to_string(),
        ..Default::default()
    };
    let (rest, api) = match rest.split_once("MSP API:") {
        Some((r, a)) => (r.trim(), Some(a.trim().to_string())),
        None => (rest.trim(), None),
    };
    v.msp_api = api.filter(|a| !a.is_empty());
    let words: Vec<&str> = rest.split_whitespace().collect();
    let mut i = 0;
    if let Some(w) = words.first() {
        if !w.starts_with('(') {
            v.mcu = Some(w.to_string());
            i = 1;
        }
    }
    if let Some(w) = words.get(i) {
        if w.starts_with('(') && w.ends_with(')') {
            v.target = Some(w.trim_matches(['(', ')']).to_string());
            i += 1;
        }
    }
    v.version = words.get(i)?.to_string();
    if !v.version.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    i += 1;
    let tail = &words[i..];
    if let Some(last) = tail
        .last()
        .filter(|w| w.starts_with('(') && w.ends_with(')'))
    {
        v.git = Some(last.trim_matches(['(', ')']).to_string());
        let date = tail[..tail.len() - 1].join(" ");
        v.build_date = Some(date).filter(|d| !d.is_empty());
    } else if !tail.is_empty() {
        v.build_date = Some(tail.join(" "));
    }
    Some(v)
}

impl Config {
    /// Parses CLI output: a `dump all`, a `diff all`, or a list of CLI lines.
    pub fn parse(text: &str) -> Config {
        let mut section = Section::Master;
        let mut out = Config::default();
        for raw in text.lines() {
            let text = raw.trim_end_matches('\r');
            let t = text.trim();
            let cmd = if t.is_empty() {
                None
            } else if let Some(c) = t.strip_prefix('#') {
                if out.version.is_none() {
                    out.version = parse_version_line(c);
                }
                None
            } else {
                let c = parse_cmd(t);
                if let Cmd::Select(s) = c {
                    section = s;
                }
                Some(c)
            };
            out.lines.push(Line {
                section: match cmd {
                    Some(Cmd::Select(s)) => s,
                    _ => section,
                },
                text: text.to_string(),
                cmd,
            });
        }
        out
    }

    /// Every `set` as (section, name, value).
    pub fn sets(&self) -> impl Iterator<Item = (Section, &str, &str)> {
        self.lines.iter().filter_map(|l| match &l.cmd {
            Some(Cmd::Set { name, value }) => Some((l.section, name.as_str(), value.as_str())),
            _ => None,
        })
    }

    /// The value of `name` in `section`. A master setting is found from any section.
    pub fn get(&self, section: Section, name: &str) -> Option<&str> {
        let mut found = None;
        for (s, n, v) in self.sets() {
            if n == name && (s == section || s == Section::Master) {
                found = Some(v);
            }
        }
        found
    }

    /// The sections a setting appears in.
    pub fn sections_of(&self, name: &str) -> Vec<Section> {
        let mut out: Vec<Section> = Vec::new();
        for (s, n, _) in self.sets() {
            if n == name && !out.contains(&s) {
                out.push(s);
            }
        }
        out
    }

    /// The last non-`set` line with this key in this section (`aux 3`, `feature OSD`):
    /// a dump lists every feature off, then the ones on.
    pub fn other(&self, section: Section, key: &str) -> Option<&str> {
        self.lines.iter().rev().find_map(|l| match &l.cmd {
            Some(Cmd::Other { key: k, .. }) if k == key && l.section == section => {
                Some(l.text.trim())
            }
            _ => None,
        })
    }

    /// A top-level word value: `board_name X`, `manufacturer_id X`, `mcu_id X`.
    pub fn word(&self, verb: &str) -> Option<&str> {
        self.lines.iter().find_map(|l| {
            let t = l.text.trim();
            let rest = t.strip_prefix(verb)?.strip_prefix(' ')?;
            Some(rest.trim()).filter(|r| !r.is_empty())
        })
    }

    /// `board_name`, else the `# board: ... board_name: X` comment that `version` prints.
    pub fn board_name(&self) -> Option<String> {
        if let Some(b) = self.word("board_name") {
            return Some(b.to_string());
        }
        self.lines.iter().find_map(|l| {
            let t = l.text.trim().strip_prefix('#')?.trim();
            let rest = t.strip_prefix("board:")?;
            rest.split(',').find_map(|kv| {
                let (k, v) = kv.split_once(':')?;
                (k.trim() == "board_name")
                    .then(|| v.trim().to_string())
                    .filter(|v| !v.is_empty())
            })
        })
    }

    /// The MCU's unique id (`mcu_id`), lowercase hex. The same value MSP_UID reports.
    pub fn mcu_id(&self) -> Option<String> {
        self.word("mcu_id")
            .filter(|v| v.chars().all(|c| c.is_ascii_hexdigit()))
            .map(|v| v.to_ascii_lowercase())
    }

    /// The identity this text shows: board, firmware, version, build.
    pub fn identity(&self) -> Identity {
        let v = self.version.clone().unwrap_or_default();
        Identity {
            board: self.board_name(),
            firmware: Some(v.firmware.clone()).filter(|f| !f.is_empty()),
            version: Some(v.version.clone()).filter(|f| !f.is_empty()),
            build: v.git.clone(),
            target: v.target.clone(),
        }
    }

    /// The text back, one line each, `\n` endings.
    pub fn render(&self) -> String {
        let mut s = String::new();
        for l in &self.lines {
            s.push_str(&l.text);
            s.push('\n');
        }
        s
    }
}

/// Renders one `set` line.
pub fn render_set(name: &str, value: &str) -> String {
    format!("set {name} = {value}")
}

/// The CLI lines of a file: trimmed, without comments and blank lines.
pub fn cli_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Lines of `expected` missing verbatim from `diff` (the bench check: every expected line
/// is in the `diff all`). Only right for values that differ from the default; a default
/// value never shows in a diff, so use `verify_lines` on a `dump all` for those.
pub fn missing_lines(diff: &str, expected: &str) -> Vec<String> {
    let have: std::collections::HashSet<&str> = diff.lines().map(|l| l.trim_end()).collect();
    cli_lines(expected)
        .into_iter()
        .filter(|l| !have.contains(l.as_str()))
        .collect()
}

/// A line a dump does not show as written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct VerifyFail {
    pub line: String,
    pub section: Section,
    /// What the dump holds instead, when it holds the setting at all.
    pub found: Option<String>,
}

/// Checks CLI lines against a `dump all`, in their section (a `profile N` line in `lines`
/// selects, as on the FC). A `set` must hold the value (case does not matter: the CLI
/// prints `ON` for `on`); another command must appear verbatim, or its key with the same
/// text. Control lines and selections are not checked.
pub fn verify_lines(dump: &Config, lines: &[String]) -> Vec<VerifyFail> {
    let mut section = Section::Master;
    let mut out = Vec::new();
    for l in lines {
        let t = l.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        match parse_cmd(t) {
            Cmd::Select(s) => section = s,
            Cmd::Control => {}
            Cmd::Set { name, value } => {
                let found = dump.get(section, &name);
                if !found.is_some_and(|f| f.eq_ignore_ascii_case(&value)) {
                    out.push(VerifyFail {
                        line: t.to_string(),
                        section,
                        found: found.map(str::to_string),
                    });
                }
            }
            Cmd::Other { key, .. } => {
                let found = dump
                    .other(section, &key)
                    .or_else(|| dump.other(Section::Master, &key));
                if found != Some(t) {
                    out.push(VerifyFail {
                        line: t.to_string(),
                        section,
                        found: found.map(str::to_string),
                    });
                }
            }
        }
    }
    out
}

// ----- the transcript scrubber -----

/// Replaces what identifies a quad or its owner in CLI output, before a transcript can be
/// committed: the MCU id (zeros of the same length), the signature, the craft and pilot
/// names, every other `*_name` value, the `# name:` header and an ELRS bind UID.
/// Everything else stays byte for byte, line endings included.
pub fn scrub(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, piece) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let (body, cr) = match piece.strip_suffix('\r') {
            Some(b) => (b, "\r"),
            None => (piece, ""),
        };
        out.push_str(&scrub_line(body));
        out.push_str(cr);
    }
    out
}

fn scrub_line(l: &str) -> String {
    if let Some(v) = l.strip_prefix("mcu_id ") {
        if !v.is_empty() && v.chars().all(|c| c.is_ascii_hexdigit()) {
            return format!("mcu_id {}", "0".repeat(v.len()));
        }
    }
    if l == "signature" || l.starts_with("signature ") {
        return "signature ".into();
    }
    if let Some(v) = l.strip_prefix("# name: ") {
        if !matches!(v.trim(), "" | "-") {
            return "# name: CRAFT".into();
        }
    }
    if let Some(rest) = l.strip_prefix("set ") {
        if let Some((k, v)) = rest.split_once(" = ") {
            if k == "expresslrs_uid" {
                return format!(
                    "set {k} = {}",
                    v.split(',').map(|_| "0").collect::<Vec<_>>().join(",")
                );
            }
            if k.ends_with("_name") && !matches!(v.trim(), "" | "-") {
                let p = match k {
                    "craft_name" => "CRAFT",
                    "pilot_name" => "PILOT",
                    _ => "NAME",
                };
                return format!("set {k} = {p}");
            }
        }
    }
    l.to_string()
}

/// Values in `text` that look like a hardware unique id: 24 or more hex digits in a row,
/// not all zeros. A committed fixture must have none.
pub fn uid_shaped(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_hexdigit())
        .filter(|w| w.len() >= 24 && w.chars().any(|c| c != '0'))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP_G473: &str = include_str!("../../../tests/fixtures/bf/g473-2025.12.5.dump_all.txt");
    const AFTER_G473: &str =
        include_str!("../../../tests/fixtures/bf/g473-2025.12.5.after.dump_all.txt");
    const DIFF_G473: &str = include_str!("../../../tests/fixtures/bf/g473-2025.12.5.diff_all.txt");
    const DUMP_V2: &str = include_str!("../../../tests/fixtures/bf/g473v2-2026.6.0.dump_all.txt");
    const DIFF_V2: &str = include_str!("../../../tests/fixtures/bf/g473v2-2026.6.0.diff_all.txt");

    #[test]
    fn version_line() {
        let v = parse_version_line(
            "# Betaflight / STM32G47X (G473) 2025.12.5-alpha Jun 25 2026 / 03:24:50 (eb2bb5a33) MSP API: 1.47",
        )
        .unwrap();
        assert_eq!(v.firmware, "Betaflight");
        assert_eq!(v.mcu.as_deref(), Some("STM32G47X"));
        assert_eq!(v.target.as_deref(), Some("G473"));
        assert_eq!(v.version, "2025.12.5-alpha");
        assert_eq!(v.build_date.as_deref(), Some("Jun 25 2026 / 03:24:50"));
        assert_eq!(v.git.as_deref(), Some("eb2bb5a33"));
        assert_eq!(v.msp_api.as_deref(), Some("1.47"));
        // 4.x style.
        let v = parse_version_line(
            "# Betaflight / STM32F411 (S411) 4.5.1 Nov 14 2024 / 12:00:00 (77d01ba3b) MSP API: 1.46",
        )
        .unwrap();
        assert_eq!(v.version, "4.5.1");
        assert!(parse_version_line("# config rev: 1359bbe").is_none());
        assert!(parse_version_line("# master").is_none());
    }

    #[test]
    fn identity_from_both_fixtures() {
        let c = Config::parse(DUMP_G473);
        let id = c.identity();
        assert_eq!(id.board.as_deref(), Some("BETAFPVG473"));
        assert_eq!(id.firmware.as_deref(), Some("Betaflight"));
        assert_eq!(id.version.as_deref(), Some("2025.12.5-alpha"));
        assert_eq!(id.build.as_deref(), Some("eb2bb5a33"));
        assert_eq!(c.mcu_id().unwrap(), "0".repeat(24));
        let c = Config::parse(DIFF_V2);
        assert_eq!(c.identity().board.as_deref(), Some("BETAFPVG473_V2"));
        assert_eq!(c.identity().version.as_deref(), Some("2026.6.0-alpha"));
        // The `version` command's own output.
        let c = Config::parse(
            "# Betaflight / STM32G47X (G473) 2026.6.0-alpha May 15 2026 / 06:14:55 (e92c10887) MSP API: 1.48\r\n# board: manufacturer_id: BEFH, board_name: BETAFPVG473_V2\r\n",
        );
        assert_eq!(c.board_name().as_deref(), Some("BETAFPVG473_V2"));
    }

    #[test]
    fn sections_follow_the_select_lines() {
        let c = Config::parse(DUMP_V2);
        // A rate setting exists once per rate profile; a PID setting once per profile.
        assert_eq!(c.sections_of("roll_rc_rate").len(), 4);
        assert!(c
            .sections_of("roll_rc_rate")
            .iter()
            .all(|s| matches!(s, Section::RateProfile(_))));
        assert_eq!(c.sections_of("p_roll").len(), 4);
        assert!(matches!(c.sections_of("p_roll")[0], Section::Profile(0)));
        // 2026.6 has battery profiles.
        assert_eq!(c.sections_of("battery_profile_name").len(), 3);
        assert_eq!(c.sections_of("osd_ah_pos"), vec![Section::Master]);
        assert_eq!(c.get(Section::Master, "osd_ah_pos"), Some("4174"));
        // A master setting reads from any section.
        assert_eq!(c.get(Section::RateProfile(2), "osd_ah_pos"), Some("4174"));
        assert!(c.other(Section::Master, "aux 1").is_some());
        assert_eq!(c.other(Section::Master, "feature OSD"), Some("feature OSD"));
    }

    #[test]
    fn a_dump_renders_back_unchanged() {
        for t in [DUMP_G473, DIFF_G473, DUMP_V2, DIFF_V2, AFTER_G473] {
            let c = Config::parse(t);
            assert_eq!(c.render().trim_end(), t.trim_end());
        }
    }

    #[test]
    fn verify_reads_the_dump_not_the_diff() {
        let after = Config::parse(AFTER_G473);
        let before = Config::parse(DUMP_G473);
        let lines: Vec<String> = [
            "set vbat_min_cell_voltage = 310",
            "set vbat_warning_cell_voltage = 330",
            "set osd_cap_alarm = 400",
            "set osd_ah_pos = 2233",
            // A default value the diff never shows, and a profile setting.
            "set vbat_max_cell_voltage = 430",
            "profile 0",
            "set p_roll = 48",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let p_roll = after
            .get(Section::Profile(0), "p_roll")
            .unwrap()
            .to_string();
        // A master setting at its default: in the dump, never in the diff.
        let (_, name, value) = after
            .sets()
            .find(|(s, n, _)| *s == Section::Master && !DIFF_G473.contains(&format!("set {n} =")))
            .unwrap();
        let mut lines = lines;
        lines[4] = render_set(name, value);
        lines[6] = render_set("p_roll", &p_roll);
        assert_eq!(verify_lines(&after, &lines), vec![]);
        let fails = verify_lines(&before, &lines);
        assert_eq!(fails.len(), 4, "{fails:?}");
        assert_eq!(fails[0].found.as_deref(), Some("330"));
        // A name that does not exist, and case of values.
        let f = verify_lines(&after, &["set no_such_thing = 1".into()]);
        assert_eq!(f[0].found, None);
        let osd = after.get(Section::Master, "osd_craftname_msgs").unwrap();
        assert!(verify_lines(
            &after,
            &[render_set("osd_craftname_msgs", &osd.to_ascii_lowercase())]
        )
        .is_empty());
    }

    #[test]
    fn missing_lines_is_the_bench_check() {
        let expected = "# a comment\nfeature OSD\n\nset osd_ah_pos = 4174\nset osd_ah_pos = 1\n";
        assert_eq!(missing_lines(DIFF_V2, expected), vec!["set osd_ah_pos = 1"]);
    }

    #[test]
    fn scrubber_removes_what_identifies() {
        let raw = "mcu_id 0123456789abcdef01234567\r\nsignature abc\r\n# name: My Quad\r\nset craft_name = My Quad\r\nset pilot_name = Someone\r\nset profile_name = -\r\nset rateprofile_name = HOME\r\nset expresslrs_uid = 1,2,3,4,5,6\r\nset osd_ah_pos = 4174\r\n";
        let s = scrub(raw);
        assert_eq!(
            s,
            "mcu_id 000000000000000000000000\r\nsignature \r\n# name: CRAFT\r\nset craft_name = CRAFT\r\nset pilot_name = PILOT\r\nset profile_name = -\r\nset rateprofile_name = NAME\r\nset expresslrs_uid = 0,0,0,0,0,0\r\nset osd_ah_pos = 4174\r\n"
        );
        assert_eq!(uid_shaped(raw), vec!["0123456789abcdef01234567"]);
        assert!(uid_shaped(&s).is_empty());
        assert_eq!(scrub(&s), s, "scrubbing is idempotent");
    }

    /// Every committed transcript is scrubbed: no UID-shaped value, no name left.
    #[test]
    fn fixtures_are_scrubbed() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bf");
        let mut n = 0;
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            let t = std::fs::read_to_string(&p).unwrap();
            assert!(uid_shaped(&t).is_empty(), "{} holds a UID", p.display());
            assert_eq!(scrub(&t), t, "{} is not scrubbed", p.display());
            n += 1;
        }
        assert!(n >= 5);
    }
}
