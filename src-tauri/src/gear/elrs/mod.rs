//! ExpressLRS (design 6.4, 7.10), a preview feature behind the `elrsPreview` setting.
//!
//! - `crsf`: the frame codec and the device-parameter messages.
//! - `link`: the serial passthrough through a radio or an FC, and a CRSF conversation.
//! - `fake`: simulators for tests (`FakeElrs`, `FakeHost`).
//! - `image`: the official release bundle, its targets, and the options block of an image.
//! - `flash`: the flash plan's pure pieces and the `esptool` argument lists.
//! - `uid`: the binding phrase's hash (MD5, written in QuadCam; the phrase is never shown).
//!
//! This module holds the shared types: the options QuadCam reads and stages, and what a read
//! leaves behind in `<gear>/elrs/<device>.json`.
//!
//! The ExpressLRS protocol parts were written from the public CRSF description and from
//! observing devices and releases. No ExpressLRS or esptool code is copied (both are GPL;
//! QuadCam is MIT). `esptool` runs only as the downloaded module, as a separate process.
//! Nothing here has touched a real device yet.

pub mod crsf;
pub mod fake;
pub mod flash;
pub mod image;
pub mod link;
pub mod uid;

use crate::gear::model::DeviceKind;
use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Utc};
use crsf::{Param, Value, WriteValue};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};

/// The options QuadCam reads and stages. The set stays small on purpose; every one is a
/// parameter the ExpressLRS Lua script shows, writable over CRSF.
pub const OPTIONS: &[(&str, &str, &[&str])] = &[
    ("packet_rate", "Packet rate", &["Packet Rate"]),
    (
        "telemetry_ratio",
        "Telemetry ratio",
        &["Telem Ratio", "Telemetry Ratio"],
    ),
    ("power", "Max power", &["Max Power"]),
    ("dynamic_power", "Dynamic power", &["Dynamic"]),
    ("switch_mode", "Switch mode", &["Switch Mode"]),
    ("model_match", "Model match", &["Model Match"]),
];

/// One option change: the option's name and the wanted choice, as the Lua script writes it
/// (`150Hz`, `1:16`, `250`, `On`). A choice matches an entry's text, or the text before its
/// `(`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct ElrsSet {
    pub option: String,
    pub value: String,
}

/// An option as the last read found it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct ElrsOption {
    /// QuadCam's name for it (`packet_rate`).
    pub key: String,
    pub label: String,
    /// The Lua parameter it is (`Packet Rate`).
    pub param: String,
    /// The parameter id on the device.
    pub id: u8,
    /// The chosen entry, or the number.
    pub value: String,
    /// What it can be set to; empty for a number (see `min` and `max`).
    pub choices: Vec<String>,
    #[serde(default)]
    pub min: Option<i64>,
    #[serde(default)]
    pub max: Option<i64>,
}

/// Any parameter the device listed, for the person to read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct ElrsParamView {
    pub id: u8,
    pub parent: u8,
    pub name: String,
    pub kind: String,
    pub value: String,
}

/// What a read of one ExpressLRS device left behind. Never holds a binding phrase: the phrase
/// is not readable over CRSF, and QuadCam stores only a hash of the one it flashes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct ElrsSnapshot {
    pub device: String,
    /// `tx` or `rx`.
    pub role: String,
    /// The saved radio or FC the device sits behind.
    pub host: String,
    pub name: String,
    pub target: Option<String>,
    pub version: Option<String>,
    /// Where `version` came from: `parameters` (a text the device lists) or `device_info`
    /// (the firmware id field). Both are unverified against a real device.
    pub version_source: Option<String>,
    pub options: Vec<ElrsOption>,
    pub params: Vec<ElrsParamView>,
    pub read_at: DateTime<Utc>,
    /// The board the radio's `ver` named at the read (a radio host only). A flash compares it
    /// with the radio plugged in before it holds the module's boot pin.
    #[serde(default)]
    pub host_board: Option<String>,
}

/// The device id of the ExpressLRS module in `host` (a saved radio) or the receiver behind
/// `host` (a saved FC), by the target name it reports. The same device gives the same id.
pub fn device_id(role: DeviceKind, host: &str, target: &str) -> String {
    crate::gear::model::device_id(role, &format!("{host}|{target}"))
}

fn state_file(root: &Path, device: &str) -> PathBuf {
    root.join("elrs")
        .join(format!("{}.json", crate::gear::store::safe(device)))
}

pub fn save_snapshot(root: &Path, s: &ElrsSnapshot) -> Result<()> {
    let f = state_file(root, &s.device);
    std::fs::create_dir_all(f.parent().expect("has a parent"))?;
    let tmp = f.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(s)?)?;
    std::fs::rename(&tmp, &f)?;
    Ok(())
}

pub fn load_snapshot(root: &Path, device: &str) -> Option<ElrsSnapshot> {
    serde_json::from_slice(&std::fs::read(state_file(root, device)).ok()?).ok()
}

fn describe(p: &Param) -> (String, String) {
    match &p.value {
        Value::Number { value, .. } => ("number".into(), value.to_string()),
        Value::Select { options, index } => (
            "choice".into(),
            options.get(*index as usize).cloned().unwrap_or_default(),
        ),
        Value::Text(s) => ("text".into(), s.clone()),
        Value::Info(s) => ("info".into(), s.clone()),
        Value::Folder => ("folder".into(), String::new()),
        Value::Command => ("command".into(), String::new()),
        Value::Unread(_) => ("other".into(), String::new()),
    }
}

/// The parameters the person can read, without the hidden ones.
pub fn param_views(params: &[Param]) -> Vec<ElrsParamView> {
    params
        .iter()
        .filter(|p| !p.hidden)
        .map(|p| {
            let (kind, value) = describe(p);
            ElrsParamView {
                id: p.id,
                parent: p.parent,
                name: p.name.clone(),
                kind,
                value,
            }
        })
        .collect()
}

/// The options QuadCam knows among a device's parameters.
pub fn options_of(params: &[Param]) -> Vec<ElrsOption> {
    let mut out = Vec::new();
    for (key, label, names) in OPTIONS {
        let hit = params.iter().find(|p| {
            !p.hidden
                && names.iter().any(|n| n.eq_ignore_ascii_case(&p.name))
                && matches!(p.value, Value::Select { .. } | Value::Number { .. })
        });
        let Some(p) = hit else { continue };
        let (value, choices, min, max) = match &p.value {
            Value::Select { options, index } => (
                options.get(*index as usize).cloned().unwrap_or_default(),
                options.clone(),
                None,
                None,
            ),
            Value::Number {
                value, min, max, ..
            } => (value.to_string(), Vec::new(), Some(*min), Some(*max)),
            _ => continue,
        };
        out.push(ElrsOption {
            key: (*key).into(),
            label: (*label).into(),
            param: p.name.clone(),
            id: p.id,
            value,
            choices,
            min,
            max,
        });
    }
    out
}

/// A choice text without spaces, units and case: `150Hz(-112dBm)` and `150 hz` both read
/// `150hz`; `250mW` reads `250`.
fn norm(s: &str) -> String {
    let head = s.split('(').next().unwrap_or(s);
    let t: String = head
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    t.strip_suffix("mw").map(str::to_string).unwrap_or(t)
}

fn synonym(s: &str) -> String {
    match norm(s).as_str() {
        "true" | "yes" | "enabled" => "on".into(),
        "false" | "no" | "disabled" => "off".into(),
        o => o.to_string(),
    }
}

/// The index of the entry `want` names.
pub fn choose(options: &[String], want: &str) -> Result<u8> {
    let w = synonym(want);
    let hits: Vec<usize> = options
        .iter()
        .enumerate()
        .filter(|(_, o)| synonym(o) == w || o.eq_ignore_ascii_case(want.trim()))
        .map(|(i, _)| i)
        .collect();
    match hits.as_slice() {
        [one] => Ok(*one as u8),
        [] => bail!("`{want}` is none of: {}.", options.join(", ")),
        many => bail!(
            "`{want}` matches several: {}. Say which one.",
            many.iter()
                .map(|i| options[*i].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Checks a change against a read and returns the parameter write for each option.
pub fn resolve(
    options: &[ElrsOption],
    sets: &[ElrsSet],
) -> Result<Vec<(ElrsOption, WriteValue, String)>> {
    let mut out: Vec<(ElrsOption, WriteValue, String)> = Vec::new();
    for s in sets {
        let key = s
            .option
            .trim()
            .to_ascii_lowercase()
            .replace([' ', '-'], "_");
        if key.contains("phrase") || key == "uid" || key.contains("bind") {
            bail!(
                "QuadCam does not stage the binding phrase. Set it as the elrs_binding_phrase setting; a flash plan then shows only its hash."
            );
        }
        let Some((_, _, _)) = OPTIONS.iter().find(|(k, _, _)| *k == key) else {
            bail!(
                "`{}` is not an option QuadCam stages. Options: {}.",
                s.option,
                OPTIONS
                    .iter()
                    .map(|(k, _, _)| *k)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        };
        let Some(o) = options.iter().find(|o| o.key == key) else {
            bail!(
                "The device has no {} parameter. Read it again, or it does not offer this option.",
                key
            );
        };
        if out.iter().any(|(p, _, _)| p.key == key) {
            bail!("{key} is set twice in one change.");
        }
        let (w, to) = if o.choices.is_empty() {
            let n: i64 = s
                .value
                .trim()
                .parse()
                .map_err(|_| anyhow!("{} takes a number, not `{}`.", o.label, s.value))?;
            if o.min.is_some_and(|m| n < m) || o.max.is_some_and(|m| n > m) {
                bail!(
                    "{} takes {} to {}; {n} is out of range.",
                    o.label,
                    o.min.unwrap_or(0),
                    o.max.unwrap_or(0)
                );
            }
            (WriteValue::Number(n), n.to_string())
        } else {
            let i = choose(&o.choices, &s.value).map_err(|e| anyhow!("{}: {e}", o.label))?;
            (WriteValue::Index(i), o.choices[i as usize].clone())
        };
        out.push((o.clone(), w, to));
    }
    Ok(out)
}

/// The version of a device: a text it lists (`ELRS 4.1.0 (...)`) in its info or text
/// parameters, else its device name, else the firmware id of its device info read as one byte
/// each of major, minor and patch. Returns where it came from; none of these is verified
/// against a real device.
pub fn detect_version(
    info: &crsf::DeviceInfo,
    params: &[Param],
) -> (Option<String>, Option<String>) {
    for p in params {
        if let Value::Info(t) | Value::Text(t) = &p.value {
            if let Some(v) = crsf::find_version(t) {
                return (Some(v), Some("parameters".into()));
            }
        }
    }
    if let Some(v) = crsf::find_version(&info.name) {
        return (Some(v), Some("parameters".into()));
    }
    let (a, b, c) = (
        (info.firmware >> 16) & 0xFF,
        (info.firmware >> 8) & 0xFF,
        info.firmware & 0xFF,
    );
    if (2..=9).contains(&a) && b < 100 && c < 100 {
        return (Some(format!("{a}.{b}.{c}")), Some("device_info".into()));
    }
    (None, None)
}

/// The binding phrase setting's check. The error never repeats the value.
pub fn check_phrase(v: &serde_json::Value) -> Result<()> {
    match v.as_str() {
        Some(s)
            if (1..=64).contains(&s.chars().count())
                && !s.chars().any(|c| c.is_control() || c == '"' || c == '\\') =>
        {
            Ok(())
        }
        _ => bail!("1 to 64 characters, without quotes, backslashes or control characters"),
    }
}

/// The major version of `x.y.z`.
pub fn major(v: &str) -> Option<u64> {
    v.trim()
        .trim_start_matches(['v', 'V'])
        .split('.')
        .next()?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fake::FakeElrs;

    fn tx_options() -> Vec<ElrsOption> {
        let d = FakeElrs::tx("RM Pocket 2.4GHz TX", "4.1.0");
        let params: Vec<Param> = (1..=9)
            .filter_map(|i| {
                let name = [
                    "Packet Rate",
                    "Telem Ratio",
                    "Switch Mode",
                    "Model Match",
                    "TX Power",
                    "Max Power",
                    "Dynamic",
                    "Bind",
                    "Version",
                ][i - 1];
                d.param(name)
            })
            .collect();
        options_of(&params)
    }

    #[test]
    fn options_are_found_by_lua_name() {
        let o = tx_options();
        let keys: Vec<&str> = o.iter().map(|o| o.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "packet_rate",
                "telemetry_ratio",
                "power",
                "dynamic_power",
                "switch_mode",
                "model_match"
            ]
        );
        assert_eq!(o[0].value, "150Hz(-112dBm)");
    }

    #[test]
    fn choices_match_loosely_and_refuse_when_unsure() {
        let o = tx_options();
        let set = |k: &str, v: &str| {
            vec![ElrsSet {
                option: k.into(),
                value: v.into(),
            }]
        };
        let r = resolve(&o, &set("packet_rate", "250hz")).unwrap();
        assert_eq!(r[0].1, WriteValue::Index(2));
        assert_eq!(r[0].2, "250Hz(-108dBm)");
        assert_eq!(
            resolve(&o, &set("power", "100mW")).unwrap()[0].1,
            WriteValue::Index(3)
        );
        assert_eq!(
            resolve(&o, &set("model_match", "true")).unwrap()[0].1,
            WriteValue::Index(1)
        );
        assert_eq!(
            resolve(&o, &set("Telemetry Ratio", "1:16")).unwrap()[0].1,
            WriteValue::Index(5)
        );
        assert!(resolve(&o, &set("packet_rate", "1000hz")).is_err());
        assert!(resolve(&o, &set("nonsense", "x")).is_err());
        assert!(resolve(
            &o,
            &[
                ElrsSet {
                    option: "power".into(),
                    value: "25".into()
                },
                ElrsSet {
                    option: "power".into(),
                    value: "50".into()
                },
            ]
        )
        .is_err());
    }

    #[test]
    fn the_binding_phrase_is_never_an_option() {
        let o = tx_options();
        for name in ["binding_phrase", "phrase", "uid", "bind"] {
            let e = resolve(
                &o,
                &[ElrsSet {
                    option: name.into(),
                    value: "secret".into(),
                }],
            )
            .unwrap_err();
            let text = e.to_string();
            assert!(text.contains("does not stage the binding phrase"), "{text}");
            assert!(!text.contains("secret"));
        }
    }

    #[test]
    fn a_snapshot_round_trips_and_the_id_is_stable() {
        let dir = tempfile::tempdir().unwrap();
        let id = device_id(DeviceKind::ElrsTx, "radio-1", "RM Pocket");
        assert_eq!(id, device_id(DeviceKind::ElrsTx, "radio-1", "RM Pocket"));
        assert_ne!(id, device_id(DeviceKind::ElrsTx, "radio-2", "RM Pocket"));
        assert!(id.starts_with("elrs-tx-"));
        let s = ElrsSnapshot {
            device: id.clone(),
            role: "tx".into(),
            host: "radio-1".into(),
            name: "RM Pocket".into(),
            target: None,
            version: Some("4.1.0".into()),
            version_source: Some("parameters".into()),
            options: tx_options(),
            params: Vec::new(),
            read_at: Utc::now(),
            host_board: Some("pocket".into()),
        };
        save_snapshot(dir.path(), &s).unwrap();
        assert_eq!(load_snapshot(dir.path(), &id), Some(s));
        assert_eq!(load_snapshot(dir.path(), "elrs-tx-none"), None);
    }

    #[test]
    fn the_version_comes_from_the_parameters_first() {
        let info = |fw| crsf::DeviceInfo {
            origin: crsf::ADDR_TX,
            name: "RM Pocket".into(),
            serial: 0,
            hardware: 0,
            firmware: fw,
            params: 0,
            protocol: 0,
        };
        let d = FakeElrs::tx("RM Pocket", "4.1.0");
        let v = d.param("Version").unwrap();
        assert_eq!(
            detect_version(&info(0x0003_0500), &[v]),
            (Some("4.1.0".into()), Some("parameters".into()))
        );
        assert_eq!(
            detect_version(&info(0x0003_0500), &[]),
            (Some("3.5.0".into()), Some("device_info".into()))
        );
        assert_eq!(detect_version(&info(0xDEAD_BEEF), &[]), (None, None));
        let mut named = info(0);
        named.name = "Tx 3.6.4".into();
        assert_eq!(detect_version(&named, &[]).0.as_deref(), Some("3.6.4"));
    }

    #[test]
    fn majors_read() {
        assert_eq!(major("4.1.0"), Some(4));
        assert_eq!(major("v3.6.4"), Some(3));
        assert_eq!(major("x"), None);
    }
}
