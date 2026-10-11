//! What a Betaflight flash carries over (design 6.5, preview): the old firmware's `diff all`
//! against the new firmware's `dump all`. Pure over text; `core/bf_flash.rs` reads both from
//! the FC and stages the lines it keeps through the FC apply.
//!
//! - A `set` the new version still has, with a different value than its new default, is
//!   carried (`changes::restore_lines`: sets, modes, adjustments, features, beepers). The old
//!   active PID and rate profile are selected again last.
//! - A `set` the new version no longer has is **reported and skipped**. So is a carried `set`
//!   whose value the new FC's `get` answer does not allow. Nothing is guessed or renamed.
//! - Board lines (`resource`, `serial`, `timer`, `dma`, `mixer`, `map`...) are not carried:
//!   the new image brings its board's own, and writing them across versions is a guess. They
//!   are listed so the person can check them.

use crate::gear::apply::fc::range_problem;
use crate::gear::bf::dump::{parse_cmd, Cmd, Config};
use crate::gear::changes::{restore_lines, selection_lines, SetRef};
use crate::gear::model::Section;
use std::collections::HashMap;

/// A carried `set` the new version refuses, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub name: String,
    pub value: String,
    pub why: String,
}

/// What to stage after the flash, and what to tell the person.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Carry {
    /// CLI lines to stage, in order.
    pub lines: Vec<String>,
    /// Settings of the old diff the new version does not have.
    pub missing: Vec<String>,
    /// Carried settings whose value the new version does not allow.
    pub refused: Vec<Refused>,
    /// Old diff lines QuadCam does not carry (board lines), as written.
    pub left_out: Vec<String>,
}

/// Lines of a `diff all` that are not settings of the person: headers and batch control.
const FRAMING: &[&str] = &[
    "batch",
    "defaults",
    "board_name",
    "manufacturer_id",
    "mcu_id",
    "signature",
    "save",
    "diff",
    "dump",
    "version",
];

/// The verbs `restore_lines` carries.
const CARRIED: &[&str] = &[
    "aux",
    "adjrange",
    "rxrange",
    "feature",
    "beeper",
    "beacon",
    "led",
    "color",
    "mode_color",
    "vtx",
    "mmix",
    "smix",
    "servo",
    "rxfail",
];

/// `old` is the old firmware's `diff all`, `new` the new firmware's `dump all`, `gets` the new
/// FC's `get NAME` answer for each name in the old diff.
pub fn carry(old: &Config, new: &Config, gets: &HashMap<String, String>) -> Carry {
    let mut out = Carry::default();

    for (section, name, _) in old.sets() {
        if new.get(section, name).is_none() && !out.missing.iter().any(|m| m == name) {
            out.missing.push(name.to_string());
        }
    }

    // Walk the carried lines, dropping a `set` the new version refuses. A profile select
    // line stays only when a `set` follows it.
    let mut section = Section::Master;
    let mut pending: Option<String> = None;
    for l in restore_lines(old, new) {
        match parse_cmd(&l) {
            Cmd::Select(s) => {
                section = s;
                pending = Some(l);
            }
            Cmd::Set { name, value } => {
                let problem = gets
                    .get(&name)
                    .and_then(|reply| {
                        range_problem(
                            &SetRef {
                                section,
                                name: name.clone(),
                                value: value.clone(),
                            },
                            reply,
                        )
                    })
                    .map(|r| r.reason);
                match problem {
                    Some(why) => out.refused.push(Refused { name, value, why }),
                    None => {
                        if let Some(p) = pending.take() {
                            out.lines.push(p);
                        }
                        out.lines.push(l);
                    }
                }
            }
            _ => out.lines.push(l),
        }
    }
    // The old active profiles last; a select line left pending above was only a section.
    out.lines.extend(selection_lines(old, new));

    for l in &old.lines {
        let Some(Cmd::Other { verb, .. }) = &l.cmd else {
            continue;
        };
        if CARRIED.contains(&verb.as_str()) || FRAMING.contains(&verb.as_str()) {
            continue;
        }
        let t = l.text.trim();
        if !t.is_empty() && !out.left_out.iter().any(|x| x == t) {
            out.left_out.push(t.to_string());
        }
    }
    out
}

/// The sentence lists for the report's notes: each list capped at `cap` names.
pub fn notes(c: &Carry, cap: usize) -> Vec<String> {
    fn list(items: &[String], cap: usize) -> String {
        let shown = items
            .iter()
            .take(cap)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        match items.len().saturating_sub(cap) {
            0 => shown,
            n => format!("{shown}, and {n} more"),
        }
    }
    let mut out = Vec::new();
    if !c.missing.is_empty() {
        out.push(format!(
            "Skipped, not in this Betaflight version ({}): {}.",
            c.missing.len(),
            list(&c.missing, cap)
        ));
    }
    if !c.refused.is_empty() {
        let items: Vec<String> = c
            .refused
            .iter()
            .map(|r| format!("{} = {} ({})", r.name, r.value, r.why))
            .collect();
        out.push(format!(
            "Skipped, the new version does not take the old value ({}): {}.",
            items.len(),
            list(&items, cap)
        ));
    }
    if !c.left_out.is_empty() {
        out.push(format!(
            "Not carried over, board lines ({}): {}. They stay in the backup taken before the flash; check them.",
            c.left_out.len(),
            list(&c.left_out, cap)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW: &str = "\
# Betaflight / STM32G47X (G473) 2026.6.0 Jun 25 2026 / 03:24:50 (abc) MSP API: 1.47
feature -RX_SERIAL
feature RX_SPI
aux 0 0 0 1700 2100 0 0
set gyro_lpf1_static_hz = 0
set motor_pwm_protocol = DSHOT300
set small_angle = 25
profile 0
set p_pitch = 46
set p_roll = 44
";

    const OLD: &str = "\
# version
# Betaflight / STM32G47X (G473) 2025.12.5 Jun 25 2026 / 03:24:50 (abc) MSP API: 1.47
batch start
defaults nosave
board_name BETAFPVG473
resource MOTOR 1 A00
serial 0 64 115200 57600 0 115200
feature -RX_SPI
feature RX_SERIAL
aux 0 0 0 1300 2100 0 0
set motor_pwm_protocol = DSHOT600
set small_angle = 180
set removed_setting = 9
profile 0
set p_pitch = 60
set p_roll = 44
batch end
";

    fn gets() -> HashMap<String, String> {
        let mut g = HashMap::new();
        g.insert(
            "small_angle".to_string(),
            "small_angle = 25\nAllowed range: 0 - 180".to_string(),
        );
        g.insert(
            "p_pitch".to_string(),
            "p_pitch = 46\nAllowed range: 0 - 200".to_string(),
        );
        g
    }

    #[test]
    fn carries_what_differs_and_reports_the_rest() {
        let c = carry(&Config::parse(OLD), &Config::parse(NEW), &gets());
        assert!(c
            .lines
            .contains(&"set motor_pwm_protocol = DSHOT600".to_string()));
        assert!(c.lines.contains(&"set small_angle = 180".to_string()));
        assert!(c.lines.contains(&"set p_pitch = 60".to_string()));
        // p_roll is the new default already.
        assert!(!c.lines.iter().any(|l| l.contains("p_roll")));
        assert!(c.lines.iter().any(|l| l.starts_with("aux 0 ")));
        assert!(c.lines.iter().any(|l| l == "feature RX_SERIAL"));
        assert_eq!(c.missing, vec!["removed_setting".to_string()]);
        assert!(c.refused.is_empty());
        // Board lines are listed, framing is not.
        assert!(c.left_out.iter().any(|l| l.starts_with("resource MOTOR")));
        assert!(c.left_out.iter().any(|l| l.starts_with("serial 0")));
        assert!(!c
            .left_out
            .iter()
            .any(|l| l.contains("batch") || l.contains("defaults")));
        assert!(!c.lines.iter().any(|l| l.contains("removed_setting")));
    }

    #[test]
    fn a_value_the_new_version_refuses_is_skipped_with_its_reason() {
        let mut g = gets();
        g.insert(
            "small_angle".to_string(),
            "small_angle = 25\nAllowed range: 0 - 90".to_string(),
        );
        g.insert(
            "motor_pwm_protocol".to_string(),
            "motor_pwm_protocol = DSHOT300\nAllowed values: PWM, DSHOT300".to_string(),
        );
        let c = carry(&Config::parse(OLD), &Config::parse(NEW), &g);
        assert!(!c.lines.iter().any(|l| l.contains("small_angle")));
        assert!(!c.lines.iter().any(|l| l.contains("motor_pwm_protocol")));
        let names: Vec<&str> = c.refused.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["motor_pwm_protocol", "small_angle"]);
        assert!(c.refused[1].why.contains("0-90"), "{}", c.refused[1].why);
        // The profile line stays with its set.
        let at = c.lines.iter().position(|l| l == "profile 0").unwrap();
        assert_eq!(c.lines[at + 1], "set p_pitch = 60");
    }

    #[test]
    fn a_select_without_sets_is_dropped() {
        let mut g = gets();
        g.insert(
            "p_pitch".to_string(),
            "p_pitch = 46\nAllowed range: 0 - 50".to_string(),
        );
        let c = carry(&Config::parse(OLD), &Config::parse(NEW), &g);
        assert!(!c.lines.iter().any(|l| l.starts_with("profile")));
        assert_eq!(c.refused.len(), 1);
    }

    #[test]
    fn the_old_active_profiles_are_selected_again() {
        let old = format!("{OLD}profile 2\nrateprofile 1\n");
        let new = format!("{NEW}profile 0\nrateprofile 0\n");
        let c = carry(&Config::parse(&old), &Config::parse(&new), &gets());
        let n = c.lines.len();
        assert_eq!(c.lines[n - 2..], ["profile 2", "rateprofile 1"]);
        // The same selection is not sent again.
        let c = carry(&Config::parse(&new), &Config::parse(&new), &gets());
        assert!(c.lines.is_empty(), "{:?}", c.lines);
    }

    #[test]
    fn notes_cap_long_lists() {
        let c = Carry {
            missing: (0..5).map(|i| format!("s{i}")).collect(),
            ..Carry::default()
        };
        let n = notes(&c, 3);
        assert_eq!(n.len(), 1);
        assert!(n[0].contains("s0, s1, s2, and 2 more"), "{}", n[0]);
        assert!(notes(&Carry::default(), 3).is_empty());
    }
}
