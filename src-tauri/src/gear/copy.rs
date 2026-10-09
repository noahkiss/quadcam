//! Copy settings between quads (design 7.7, WP5b): pick settings, or a section such as
//! rates, OSD or modes, from one FC's latest backup and stage them for another FC.
//!
//! Pure over two `dump all` texts. The result is a list of FC edits (`set` and CLI lines)
//! that make the target read as the source for what was picked, a line diff, and the
//! compatibility checks. Staging goes through `Core::gear_change_stage`, so the usual plan,
//! checks and confirm apply; nothing here writes.
//!
//! Rules:
//! - **Same firmware and release.** Betaflight settings change names and values between
//!   releases, so the two dumps must be the same firmware and the same year.month (a patch
//!   difference is fine). Otherwise the copy is refused (`incompatible`).
//! - **Same board, else only portable parts.** With different boards, settings tied to the
//!   board's chips and buses (`gyro_*`, `acc_*`, `baro_*`, `mag_*`, `*_bustype`, `*spibus`,
//!   `*i2c*`, `serial*`) are left out, and a note says so.
//! - **Per-quad values are never copied:** the craft and display names, accelerometer
//!   trims, battery and current calibration.
//! - **Only what the target has.** A setting or line the target's dump does not hold is
//!   left out and listed.

use super::apply::{check, pass};
use super::bf::dump::{render_set, Cmd, Config};
use super::changes::FcRender;
use super::model::{Check, DiffLine, Edit, LineOp, Refusal, RefusalCode, Section};
use serde::{Deserialize, Serialize};
use specta::Type;

/// A part of the configuration to copy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum CopyPart {
    /// Every rate profile's settings.
    Rates,
    /// Every PID profile's settings (PIDs, filters, and the rest of a profile).
    Pid,
    /// The `osd_*` settings: element positions, units and alarms.
    Osd,
    /// The mode ranges (`aux`).
    Modes,
    /// The adjustment ranges (`adjrange`).
    Adjustments,
    /// The VTX table and the band/channel/power ranges (`vtx`, `vtxtable`).
    Vtx,
    /// Features and beeper conditions.
    Features,
}

impl CopyPart {
    pub const ALL: [CopyPart; 7] = [
        CopyPart::Rates,
        CopyPart::Pid,
        CopyPart::Osd,
        CopyPart::Modes,
        CopyPart::Adjustments,
        CopyPart::Vtx,
        CopyPart::Features,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CopyPart::Rates => "rates",
            CopyPart::Pid => "PID profiles",
            CopyPart::Osd => "OSD",
            CopyPart::Modes => "modes",
            CopyPart::Adjustments => "adjustments",
            CopyPart::Vtx => "VTX",
            CopyPart::Features => "features and beeper",
        }
    }

    /// The non-`set` verbs this part copies as CLI lines.
    fn verbs(self) -> &'static [&'static str] {
        match self {
            CopyPart::Modes => &["aux"],
            CopyPart::Adjustments => &["adjrange"],
            CopyPart::Vtx => &["vtx", "vtxtable"],
            CopyPart::Features => &["feature", "beeper"],
            _ => &[],
        }
    }

    /// True when a `set` belongs to this part.
    fn takes(self, section: Section, name: &str) -> bool {
        match self {
            CopyPart::Rates => matches!(section, Section::RateProfile(_)),
            CopyPart::Pid => matches!(section, Section::Profile(_)),
            CopyPart::Osd => section == Section::Master && name.starts_with("osd_"),
            CopyPart::Vtx => section == Section::Master && name.starts_with("vtx_"),
            _ => false,
        }
    }
}

/// What to copy.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct CopySelect {
    #[serde(default)]
    pub parts: Vec<CopyPart>,
    /// Settings by name (in every section that holds them), on top of the parts.
    #[serde(default)]
    pub settings: Vec<String>,
}

/// Never copied: each quad has its own.
const PER_QUAD: &[&str] = &[
    "name",
    "display_name",
    "pilot_name",
    "acc_trim_pitch",
    "acc_trim_roll",
    "vbat_scale",
    "vbat_divider",
    "vbat_multiplier",
    "ibata_scale",
    "ibata_offset",
    "current_meter_scale",
    "current_meter_offset",
];

/// Tied to the board's chips and buses: copied only between quads of the same board.
const BOARD_BOUND: &[&str] = &["gyro_", "acc_", "baro_", "mag_", "serial", "spi", "i2c"];

fn board_bound(name: &str) -> bool {
    BOARD_BOUND.iter().any(|p| name.starts_with(p))
        || name.ends_with("_bustype")
        || name.contains("spibus")
        || name.contains("i2c")
}

/// What a copy would do.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct CopyPlan {
    /// The source and target backups the plan read.
    pub from_backup: String,
    pub to_backup: String,
    pub checks: Vec<Check>,
    /// The edits that stage the copy; empty when a check failed or nothing differs.
    pub edits: Vec<Edit>,
    /// The before/after lines.
    pub diff: Vec<DiffLine>,
    /// Settings and lines already the same on the target.
    pub same: usize,
    /// Left out, each with why.
    pub skipped: Vec<String>,
    /// Things to know that do not refuse.
    pub notes: Vec<String>,
}

impl CopyPlan {
    pub fn ready(&self) -> bool {
        self.checks.iter().all(|c| c.ok) && !self.edits.is_empty()
    }
}

/// The year.month of a Betaflight version (`2025.12.5-alpha` is `2025.12`).
fn release(v: &str) -> Option<String> {
    let mut it = v.split(['.', '-']);
    let (a, b) = (it.next()?, it.next()?);
    (a.chars().all(|c| c.is_ascii_digit()) && b.chars().all(|c| c.is_ascii_digit()))
        .then(|| format!("{a}.{b}"))
}

/// Plans a copy from `source` (a `dump all`) to `target`.
pub fn plan(
    from_backup: &str,
    to_backup: &str,
    source: &str,
    target: &str,
    select: &CopySelect,
) -> CopyPlan {
    let (src, dst) = (Config::parse(source), Config::parse(target));
    let (si, di) = (src.identity(), dst.identity());
    let mut out = CopyPlan {
        from_backup: from_backup.into(),
        to_backup: to_backup.into(),
        checks: Vec::new(),
        edits: Vec::new(),
        diff: Vec::new(),
        same: 0,
        skipped: Vec::new(),
        notes: Vec::new(),
    };

    let fw = |i: &crate::gear::model::Identity| i.firmware.clone().unwrap_or_default();
    out.checks.push(check(
        "Same firmware",
        if !fw(&si).is_empty() && fw(&si) == fw(&di) {
            Ok(())
        } else {
            Err(Refusal::new(
                RefusalCode::Incompatible,
                format!(
                    "The quads run {} and {}; QuadCam copies settings between the same firmware only.",
                    or_unknown(&fw(&si)),
                    or_unknown(&fw(&di))
                ),
            ))
        },
    ));
    let (rs, rd) = (
        si.version.as_deref().and_then(release),
        di.version.as_deref().and_then(release),
    );
    out.checks.push(check(
        "Same release",
        if rs.is_some() && rs == rd {
            Ok(())
        } else {
            Err(Refusal::new(
                RefusalCode::Incompatible,
                format!(
                    "The quads run release {} and {}; settings change between releases, so QuadCam copies within one release.",
                    rs.as_deref().unwrap_or("unknown"),
                    rd.as_deref().unwrap_or("unknown")
                ),
            ))
        },
    ));
    let same_board = si.board.is_some() && si.board == di.board;
    out.checks.push(check(
        "Something picked",
        if select.parts.is_empty() && select.settings.is_empty() {
            Err(Refusal::new(
                RefusalCode::Incompatible,
                "Pick a part (rates, OSD, modes, ...) or name the settings to copy.",
            ))
        } else {
            Ok(())
        },
    ));
    if !out.checks.iter().all(|c| c.ok) {
        return out;
    }
    if !same_board {
        out.notes.push(format!(
            "The boards differ ({} and {}): settings tied to the board's chips and buses are left out.",
            si.board.as_deref().unwrap_or("unknown"),
            di.board.as_deref().unwrap_or("unknown")
        ));
    }
    let wanted: Vec<String> = select
        .settings
        .iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    for w in &wanted {
        if src.sections_of(w).is_empty() {
            out.skipped
                .push(format!("{w}: the source does not hold this setting"));
        }
    }

    // `set` lines, in the source's order. One edit per setting that differs.
    let mut seen: Vec<(Section, String)> = Vec::new();
    for (section, name, value) in src.sets() {
        let picked =
            select.parts.iter().any(|p| p.takes(section, name)) || wanted.iter().any(|w| w == name);
        if !picked || seen.contains(&(section, name.to_string())) {
            continue;
        }
        seen.push((section, name.to_string()));
        if PER_QUAD.contains(&name) {
            out.skipped.push(format!("{name}: each quad has its own"));
            continue;
        }
        if !same_board && board_bound(name) {
            out.skipped.push(format!("{name}: tied to the board"));
            continue;
        }
        let Some(now) = dst.get(section, name) else {
            out.skipped
                .push(format!("{name}: the target does not hold this setting"));
            continue;
        };
        if now.eq_ignore_ascii_case(value) {
            out.same += 1;
            continue;
        }
        if section != Section::Master && dst.sections_of(name).is_empty() {
            continue;
        }
        out.edits.push(Edit::FcSet {
            section,
            name: name.to_string(),
            value: value.to_string(),
        });
        let tag = section
            .select_line()
            .map(|s| format!("[{s}] "))
            .unwrap_or_default();
        out.diff.push(DiffLine {
            op: LineOp::Remove,
            text: format!("{tag}{}", render_set(name, now)),
        });
        out.diff.push(DiffLine {
            op: LineOp::Add,
            text: format!("{tag}{}", render_set(name, value)),
        });
    }

    // Lines of the list-like commands: the source's last line per key, where the target
    // has the key and holds something else.
    let mut lines: Vec<String> = Vec::new();
    for part in select.parts.iter().filter(|p| !p.verbs().is_empty()) {
        for l in &src.lines {
            let Some(Cmd::Other { verb, key }) = &l.cmd else {
                continue;
            };
            if !part.verbs().contains(&verb.as_str()) {
                continue;
            }
            let text = l.text.trim();
            if src.other(l.section, key) != Some(text) {
                continue;
            }
            match dst.other(l.section, key) {
                None => out
                    .skipped
                    .push(format!("{key}: the target does not hold this line")),
                Some(t) if t == text => out.same += 1,
                Some(t) => {
                    if !lines.contains(&text.to_string()) {
                        lines.push(text.to_string());
                        out.diff.push(DiffLine {
                            op: LineOp::Remove,
                            text: t.to_string(),
                        });
                        out.diff.push(DiffLine {
                            op: LineOp::Add,
                            text: text.to_string(),
                        });
                    }
                }
            }
        }
    }
    if !lines.is_empty() {
        out.edits.push(Edit::FcLines { lines });
    }
    out.checks.push(if out.edits.is_empty() {
        check(
            "Something differs",
            Err(Refusal::new(
                RefusalCode::Incompatible,
                "The target already holds everything picked, or none of it applies to it.",
            )),
        )
    } else {
        pass("Something differs")
    });
    out
}

fn or_unknown(s: &str) -> &str {
    if s.is_empty() {
        "an unknown firmware"
    } else {
        s
    }
}

/// The CLI lines a copy's edits would send against the target's dump, for the check that
/// the target holds every named setting.
pub fn render(plan: &CopyPlan, target: &Config) -> FcRender {
    super::changes::render_fc(&plan.edits, Some(target))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = include_str!("../../tests/fixtures/bf/g473-2025.12.5.dump_all.txt");

    /// The same dump with changed values, as a second quad's would be.
    fn other(f: impl Fn(&str) -> String) -> String {
        f(SRC)
    }

    fn pick(parts: &[CopyPart]) -> CopySelect {
        CopySelect {
            parts: parts.to_vec(),
            settings: vec![],
        }
    }

    #[test]
    fn identical_dumps_have_nothing_to_copy() {
        let p = plan("a", "b", SRC, SRC, &pick(&[CopyPart::Rates, CopyPart::Osd]));
        assert!(
            p.checks.iter().filter(|c| !c.ok).count() == 1,
            "{:?}",
            p.checks
        );
        assert!(p.edits.is_empty());
        assert!(p.same > 0);
    }

    #[test]
    fn a_pick_stages_the_differences_and_a_diff() {
        let target = other(|s| {
            s.replace("set osd_vbat_pos = 14850", "set osd_vbat_pos = 2048")
                .replace("aux 0 0 0 1700 2100 0 0", "aux 0 0 0 900 1000 0 0")
        });
        let p = plan(
            "a",
            "b",
            SRC,
            &target,
            &pick(&[CopyPart::Osd, CopyPart::Modes]),
        );
        assert!(p.ready(), "{:?}", p.checks);
        assert_eq!(
            p.edits,
            vec![
                Edit::FcSet {
                    section: Section::Master,
                    name: "osd_vbat_pos".into(),
                    value: "14850".into(),
                },
                Edit::FcLines {
                    lines: vec!["aux 0 0 0 1700 2100 0 0".into()],
                },
            ]
        );
        let text: Vec<String> = p
            .diff
            .iter()
            .map(|l| {
                format!(
                    "{}{}",
                    match l.op {
                        LineOp::Add => "+",
                        LineOp::Remove => "-",
                        LineOp::Same => " ",
                    },
                    l.text
                )
            })
            .collect();
        assert_eq!(
            text,
            [
                "-set osd_vbat_pos = 2048",
                "+set osd_vbat_pos = 14850",
                "-aux 0 0 0 900 1000 0 0",
                "+aux 0 0 0 1700 2100 0 0"
            ]
        );
        // The edits render against the target without a problem.
        let r = render(&p, &Config::parse(&target));
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert!(r.lines.contains(&"set osd_vbat_pos = 14850".to_string()));
    }

    #[test]
    fn rates_and_pid_copy_per_profile() {
        let first_rate = SRC
            .lines()
            .find(|l| l.starts_with("set roll_rc_rate") || l.starts_with("set roll_srate"))
            .expect("a rate setting");
        let name = first_rate.split_whitespace().nth(1).unwrap();
        let target = other(|s| {
            let mut out = Vec::new();
            for l in s.lines() {
                if l.starts_with(&format!("set {name} =")) {
                    out.push(format!("set {name} = 1"));
                } else {
                    out.push(l.to_string());
                }
            }
            out.join("\n")
        });
        let p = plan("a", "b", SRC, &target, &pick(&[CopyPart::Rates]));
        assert!(p.ready());
        assert!(p.edits.iter().all(|e| matches!(
            e,
            Edit::FcSet {
                section: Section::RateProfile(_),
                ..
            }
        )));
        assert!(p
            .edits
            .iter()
            .any(|e| matches!(e, Edit::FcSet { name: n, .. } if n == name)));
        // PID profiles are a separate pick: the rate pick touched no profile setting.
        let pid = plan("a", "b", SRC, &target, &pick(&[CopyPart::Pid]));
        assert!(pid.edits.is_empty());
    }

    #[test]
    fn another_release_or_firmware_is_incompatible() {
        let newer = SRC.replace("2025.12.5-alpha", "2026.6.0");
        let p = plan("a", "b", SRC, &newer, &pick(&[CopyPart::Rates]));
        assert_eq!(
            p.checks
                .iter()
                .find(|c| !c.ok)
                .unwrap()
                .refusal
                .as_ref()
                .unwrap()
                .code,
            RefusalCode::Incompatible
        );
        assert!(p.edits.is_empty());
        // A patch difference is fine.
        let patch = SRC.replace("2025.12.5-alpha", "2025.12.9");
        let p = plan("a", "b", SRC, &patch, &pick(&[CopyPart::Rates]));
        assert!(p.checks.iter().take(2).all(|c| c.ok), "{:?}", p.checks);
        let inav = SRC.replace("# Betaflight /", "# INAV /");
        let p = plan("a", "b", SRC, &inav, &pick(&[CopyPart::Rates]));
        assert!(!p.checks[0].ok);
    }

    #[test]
    fn per_quad_values_and_board_bound_settings_are_left_out() {
        let target = other(|s| {
            s.replace("set acc_trim_pitch = 0", "set acc_trim_pitch = 9")
                .replace("set gyro_1_spibus = 1", "set gyro_1_spibus = 3")
                .replace("board_name BETAFPVG473", "board_name OTHERBOARD")
        });
        let named = CopySelect {
            parts: vec![],
            settings: vec![
                "acc_trim_pitch".into(),
                "gyro_1_spibus".into(),
                "no_such_setting".into(),
            ],
        };
        let p = plan("a", "b", SRC, &target, &named);
        assert!(p.edits.is_empty(), "{:?}", p.edits);
        let why = p.skipped.join("\n");
        assert!(
            why.contains("acc_trim_pitch: each quad has its own"),
            "{why}"
        );
        assert!(why.contains("gyro_1_spibus: tied to the board"), "{why}");
        assert!(why.contains("no_such_setting"), "{why}");
        assert!(p.notes.iter().any(|n| n.contains("boards differ")));
        // The same board copies a bus setting by name, but still not the trims.
        let same = other(|s| s.replace("set gyro_1_spibus = 1", "set gyro_1_spibus = 3"));
        let p = plan("a", "b", SRC, &same, &named);
        assert_eq!(p.edits.len(), 1, "{:?}", p.edits);
        assert!(p.skipped.iter().any(|s| s.starts_with("acc_trim_pitch")));
    }

    #[test]
    fn nothing_picked_and_missing_on_target() {
        let p = plan("a", "b", SRC, SRC, &CopySelect::default());
        assert!(p
            .checks
            .iter()
            .any(|c| c.name == "Something picked" && !c.ok));
        // A setting the target lacks is listed, not staged.
        let without = SRC
            .lines()
            .filter(|l| !l.starts_with("set osd_vbat_pos"))
            .collect::<Vec<_>>()
            .join("\n");
        let p = plan("a", "b", SRC, &without, &pick(&[CopyPart::Osd]));
        assert!(p.skipped.iter().any(|s| s.starts_with("osd_vbat_pos")));
    }

    #[test]
    fn features_copy_as_on_and_off_lines() {
        let target = other(|s| s.replace("\nfeature OSD\n", "\nfeature -OSD\n"));
        let p = plan("a", "b", SRC, &target, &pick(&[CopyPart::Features]));
        assert_eq!(
            p.edits,
            vec![Edit::FcLines {
                lines: vec!["feature OSD".into()]
            }]
        );
    }
}
