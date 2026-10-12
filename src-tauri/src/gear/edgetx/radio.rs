//! The radio's own settings: a typed view of `RADIO/radio.yml` for the Settings editor, and
//! the one check every `RadioOp::SetScalar` passes (`setting_value`).
//!
//! - `SETTINGS` lists the settings the editor shows, grouped as EdgeTX's Radio Setup screen
//!   groups them, each with the values EdgeTX accepts. Every one is a top-level scalar, so
//!   the line-level editor rewrites one line and leaves every other line as it was.
//! - A key outside the table is not checked here: `SetScalar` still sets any top-level
//!   value the file has, as before, and unknown keys round-trip untouched.
//! - A key the file lacks is shown as absent; QuadCam never adds a line to `radio.yml`.
//!
//! Values are in the file's own units: a 5-step setting is 0-4 for -2..+2, `vBatWarn` is
//! tenths of a volt, `lightAutoOff` counts 5 s steps, `backlightBright` is 100 minus the
//! brightness. QuadCam learned them from EdgeTX's files and copies no EdgeTX code.

use super::yaml::{unquote, Doc};
use crate::gear::model::{Refusal, RefusalCode};
use serde::{Deserialize, Serialize};
use specta::Type;

/// The sections, in the order the editor shows them.
pub const SECTIONS: &[&str] = &[
    "Sound",
    "Haptic",
    "Display",
    "Alarms",
    "Time and GPS",
    "Sticks and USB",
    "Owner",
];

/// What kind of value a setting holds, and the values it may take.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum SettingKind {
    /// One of a list of values.
    Choice { choices: Vec<Choice> },
    /// A whole number from `min` to `max` in the file. The editor shows
    /// `value * mul / 10^decimals` in `unit`; with `invert` N it shows `N - value`.
    Number {
        min: i32,
        max: i32,
        unit: String,
        mul: i32,
        decimals: u8,
        #[serde(default)]
        invert: Option<i32>,
    },
    /// `0` or `1`. With `invert`, on is `0` (a `disable...` key).
    Toggle { invert: bool },
    /// Quoted text of at most `max_len` characters: letters, digits, space, `-`, `_`, `.`.
    Text { max_len: u32 },
}

/// One value of a choice.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct Choice {
    /// The value in the file.
    pub value: String,
    pub label: String,
}

/// One setting as the editor shows it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct RadioSetting {
    /// The `radio.yml` key.
    pub key: String,
    pub section: String,
    pub label: String,
    pub kind: SettingKind,
    /// The value in the file (text unquoted); None when the file has no such line.
    #[serde(default)]
    pub value: Option<String>,
}

/// The typed view of `radio.yml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct RadioSettingsView {
    pub sections: Vec<String>,
    pub settings: Vec<RadioSetting>,
    /// The model the radio selects (`model01.yml`).
    #[serde(default)]
    pub selected_model: Option<String>,
}

enum Def {
    Choice(&'static [(&'static str, &'static str)]),
    /// A 5-step setting: 0-4 in the file, -2..+2 on the radio.
    Five,
    Num {
        min: i32,
        max: i32,
        unit: &'static str,
        mul: i32,
        decimals: u8,
        invert: Option<i32>,
    },
    Toggle {
        invert: bool,
    },
    Text(u32),
    /// `templateSetup`: the 24 orders of R, E, T, A.
    ChannelOrder,
}

const MODES: &[(&str, &str)] = &[
    ("mode_quiet", "Quiet"),
    ("mode_alarms", "Alarms only"),
    ("mode_nokeys", "No keys"),
    ("mode_all", "All"),
];

const fn num(min: i32, max: i32, unit: &'static str) -> Def {
    Def::Num {
        min,
        max,
        unit,
        mul: 1,
        decimals: 0,
        invert: None,
    }
}

/// `(key, section, label, kind)`, in the editor's order.
const SETTINGS: &[(&str, &str, &str, Def)] = &[
    ("beepMode", "Sound", "Sound mode", Def::Choice(MODES)),
    ("speakerVolume", "Sound", "Volume", num(0, 23, "")),
    ("beepVolume", "Sound", "Beep volume", Def::Five),
    ("beepLength", "Sound", "Beep length", Def::Five),
    ("wavVolume", "Sound", "Sound file volume", Def::Five),
    ("backgroundVolume", "Sound", "Background volume", Def::Five),
    ("varioVolume", "Sound", "Vario volume", Def::Five),
    ("hapticMode", "Haptic", "Haptic mode", Def::Choice(MODES)),
    ("hapticLength", "Haptic", "Haptic length", Def::Five),
    ("hapticStrength", "Haptic", "Haptic strength", Def::Five),
    ("contrast", "Display", "Contrast", num(10, 30, "")),
    (
        "backlightMode",
        "Display",
        "Backlight",
        Def::Choice(&[
            ("backlight_mode_off", "Off"),
            ("backlight_mode_keys", "Keys"),
            ("backlight_mode_sticks", "Sticks"),
            ("backlight_mode_all", "Keys and sticks"),
            ("backlight_mode_on", "On"),
        ]),
    ),
    (
        "lightAutoOff",
        "Display",
        "Backlight off after",
        Def::Num {
            min: 1,
            max: 120,
            unit: "s",
            mul: 5,
            decimals: 0,
            invert: None,
        },
    ),
    (
        "backlightBright",
        "Display",
        "Brightness",
        Def::Num {
            min: 0,
            max: 95,
            unit: "",
            mul: 1,
            decimals: 0,
            invert: Some(100),
        },
    ),
    (
        "vBatWarn",
        "Alarms",
        "Battery low",
        Def::Num {
            min: 30,
            max: 120,
            unit: "V",
            mul: 1,
            decimals: 1,
            invert: None,
        },
    ),
    (
        "inactivityTimer",
        "Alarms",
        "Inactivity alarm",
        num(0, 250, "min"),
    ),
    (
        "disableAlarmWarning",
        "Alarms",
        "Sound off warning",
        Def::Toggle { invert: true },
    ),
    (
        "disableRssiPoweroffAlarm",
        "Alarms",
        "RSSI shutdown alarm",
        Def::Toggle { invert: true },
    ),
    ("timezone", "Time and GPS", "Time zone", num(-12, 14, "h")),
    (
        "adjustRTC",
        "Time and GPS",
        "Set the clock from GPS",
        Def::Toggle { invert: false },
    ),
    (
        "gpsFormat",
        "Time and GPS",
        "GPS coordinates",
        Def::Choice(&[("0", "DMS"), ("1", "NMEA")]),
    ),
    (
        "imperial",
        "Time and GPS",
        "Units",
        Def::Choice(&[("0", "Metric"), ("1", "Imperial")]),
    ),
    (
        "stickMode",
        "Sticks and USB",
        "Stick mode",
        Def::Choice(&[
            ("0", "Mode 1"),
            ("1", "Mode 2"),
            ("2", "Mode 3"),
            ("3", "Mode 4"),
        ]),
    ),
    (
        "templateSetup",
        "Sticks and USB",
        "Default channel order",
        Def::ChannelOrder,
    ),
    (
        "USBMode",
        "Sticks and USB",
        "USB mode",
        Def::Choice(&[
            ("0", "Ask"),
            ("1", "Joystick"),
            ("2", "Storage"),
            ("3", "Serial"),
        ]),
    ),
    ("ownerRegistrationID", "Owner", "Owner ID", Def::Text(8)),
];

/// The 24 channel orders, in `templateSetup` order: the permutations of R, E, T, A in
/// lexicographic order of their positions (21 is AETR).
pub fn channel_orders() -> Vec<String> {
    let mut out = Vec::with_capacity(24);
    let letters = ['R', 'E', 'T', 'A'];
    let mut p = [0usize, 1, 2, 3];
    loop {
        out.push(p.iter().map(|&i| letters[i]).collect());
        // The next permutation.
        let Some(i) = (0..3).rev().find(|&i| p[i] < p[i + 1]) else {
            break;
        };
        let j = (i + 1..4).rev().find(|&j| p[j] > p[i]).unwrap();
        p.swap(i, j);
        p[i + 1..].reverse();
    }
    out
}

fn kind(d: &Def) -> SettingKind {
    let choices = |list: Vec<(String, String)>| SettingKind::Choice {
        choices: list
            .into_iter()
            .map(|(value, label)| Choice { value, label })
            .collect(),
    };
    match d {
        Def::Choice(list) => choices(
            list.iter()
                .map(|(v, l)| (v.to_string(), l.to_string()))
                .collect(),
        ),
        Def::Five => choices(
            ["-2", "-1", "0", "+1", "+2"]
                .iter()
                .enumerate()
                .map(|(i, l)| (i.to_string(), l.to_string()))
                .collect(),
        ),
        Def::ChannelOrder => choices(
            channel_orders()
                .into_iter()
                .enumerate()
                .map(|(i, l)| (i.to_string(), l))
                .collect(),
        ),
        Def::Num {
            min,
            max,
            unit,
            mul,
            decimals,
            invert,
        } => SettingKind::Number {
            min: *min,
            max: *max,
            unit: unit.to_string(),
            mul: *mul,
            decimals: *decimals,
            invert: *invert,
        },
        Def::Toggle { invert } => SettingKind::Toggle { invert: *invert },
        Def::Text(n) => SettingKind::Text { max_len: *n },
    }
}

/// The keys the editor knows.
pub fn keys() -> impl Iterator<Item = &'static str> {
    SETTINGS.iter().map(|s| s.0)
}

/// Reads the settings the editor shows. Reads only.
pub fn view(d: &Doc) -> Result<RadioSettingsView, Refusal> {
    let tree = d.tree()?;
    let mut settings = Vec::new();
    for (key, section, label, def) in SETTINGS {
        let node = tree.iter().find(|n| n.key.as_deref() == Some(*key));
        let value = node
            .filter(|n| n.children.is_empty())
            .map(|n| unquote(n.value.trim()).to_string());
        let mut kind = kind(def);
        // A value the table does not list still shows, as itself.
        if let (SettingKind::Choice { choices }, Some(v)) = (&mut kind, &value) {
            if !choices.iter().any(|c| &c.value == v) {
                choices.push(Choice {
                    value: v.clone(),
                    label: v.clone(),
                });
            }
        }
        settings.push(RadioSetting {
            key: key.to_string(),
            section: section.to_string(),
            label: label.to_string(),
            kind,
            value,
        });
    }
    Ok(RadioSettingsView {
        sections: SECTIONS.iter().map(|s| s.to_string()).collect(),
        settings,
        selected_model: super::card::selected_in(d),
    })
}

fn bad(reason: impl Into<String>) -> Refusal {
    Refusal::new(RefusalCode::BadSetting, reason)
}

/// The line value to write for `key`, checked against the values EdgeTX accepts. None for
/// a key the editor does not know (it is written as given). Text is quoted here.
pub fn setting_value(key: &str, value: &str) -> Result<Option<String>, Refusal> {
    let Some((_, _, label, def)) = SETTINGS.iter().find(|s| s.0 == key) else {
        return Ok(None);
    };
    let v = value.trim();
    let int = |min: i32, max: i32| -> Result<String, Refusal> {
        match v.parse::<i32>() {
            Ok(n) if (min..=max).contains(&n) => Ok(n.to_string()),
            _ => Err(bad(format!(
                "{key} ({label}) is a whole number from {min} to {max}, not {v:?}."
            ))),
        }
    };
    let out = match def {
        Def::Choice(list) => {
            if !list.iter().any(|(x, _)| *x == v) {
                return Err(bad(format!(
                    "{key} ({label}) is one of {}, not {v:?}.",
                    list.iter().map(|(x, _)| *x).collect::<Vec<_>>().join(", ")
                )));
            }
            v.to_string()
        }
        Def::Five => int(0, 4)?,
        Def::ChannelOrder => int(0, 23)?,
        Def::Num { min, max, .. } => int(*min, *max)?,
        Def::Toggle { .. } => int(0, 1)?,
        Def::Text(n) => {
            let t = unquote(value);
            if t.chars().count() > *n as usize {
                return Err(bad(format!(
                    "{key} ({label}) holds {n} characters at most."
                )));
            }
            if !t
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
            {
                return Err(bad(format!(
                    "{key} ({label}) holds letters, digits, spaces, -, _ and . only."
                )));
            }
            format!("\"{t}\"")
        }
    };
    Ok(Some(out))
}

/// What to know about a value that does not refuse.
pub fn warning(key: &str, value: &str) -> Option<String> {
    match (key, value) {
        ("hapticMode", "mode_alarms") => {
            Some("hapticMode mode_alarms drops every special-function haptic.".into())
        }
        ("stickMode", _) => Some(
            "stickMode changes which stick is throttle; check the sticks on the radio before you fly."
                .into(),
        ),
        ("USBMode", v) if v != "0" && v != "2" => Some(
            "With this USB mode the radio no longer shows its card when plugged in; QuadCam reads the card in Storage mode.".into(),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_channel_orders_run_in_edgetx_order() {
        let o = channel_orders();
        assert_eq!(o.len(), 24);
        assert_eq!(o[0], "RETA");
        assert_eq!(o[21], "AETR");
        assert_eq!(o[23], "ATER");
        let mut u = o.clone();
        u.sort();
        u.dedup();
        assert_eq!(u.len(), 24);
    }

    #[test]
    fn values_are_checked_by_kind() {
        assert_eq!(
            setting_value("beepVolume", "4").unwrap().as_deref(),
            Some("4")
        );
        assert!(setting_value("beepVolume", "5").is_err());
        assert!(setting_value("vBatWarn", "29").is_err());
        assert_eq!(
            setting_value("vBatWarn", " 66 ").unwrap().as_deref(),
            Some("66")
        );
        assert!(setting_value("backlightMode", "backlight_mode_x").is_err());
        assert!(setting_value("disableRssiPoweroffAlarm", "2").is_err());
        assert_eq!(
            setting_value("ownerRegistrationID", "abc")
                .unwrap()
                .as_deref(),
            Some("\"abc\"")
        );
        assert_eq!(
            setting_value("ownerRegistrationID", "\"abc\"")
                .unwrap()
                .as_deref(),
            Some("\"abc\"")
        );
        assert!(setting_value("ownerRegistrationID", "123456789").is_err());
        assert!(setting_value("ownerRegistrationID", "a\"b").is_err());
        assert_eq!(setting_value("view", "3").unwrap(), None);
    }

    #[test]
    fn every_key_has_a_section() {
        for (k, s, _, _) in SETTINGS {
            assert!(SECTIONS.contains(s), "{k}");
        }
    }
}
