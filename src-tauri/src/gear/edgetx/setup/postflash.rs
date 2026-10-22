//! The steps after an EdgeTX or ELRS flash (`docs/radio-design.md` 3.1 steps 5, 11 and 12),
//! each planned before anything is written:
//!
//! - the storage pack: EdgeTX's SD-card files for the radio's screen (`bw128x64`), put on the
//!   card where they differ, never a model or `radio.yml`;
//! - the ELRS Lua: `SCRIPTS/TOOLS/elrs.lua` from the ExpressLRS release, with the old
//!   `elrsV3.lua` and its compiled `.luac` removed so the radio lists one tool;
//! - the volume label: a short name for the radio's volume, made from its saved name.
//!
//! Both downloads are GPL files from upstream, fetched at run time and never bundled.

use super::generate::Source;
use crate::gear::edgetx::yaml::latin1;
use crate::gear::model::{Edit, Refusal, RefusalCode};
use serde::{Deserialize, Serialize};
use specta::Type;

/// The pack file that names the pack's version.
pub const PACK_VERSION_FILE: &str = "edgetx.sdcard.version";
/// Where the ELRS Lua goes.
pub const ELRS_LUA: &str = "SCRIPTS/TOOLS/elrs.lua";
/// The Lua names older ELRS releases used; the radio would list both.
pub const OLD_ELRS_LUA: &[&str] = &["SCRIPTS/TOOLS/elrsV3.lua", "SCRIPTS/TOOLS/elrsV3.luac"];

/// The storage pack a board takes; None for a board QuadCam has not matched to one.
pub fn pack_name(board: &str) -> Option<&'static str> {
    match board.trim().to_ascii_lowercase().as_str() {
        "pocket" | "gx12" => Some("bw128x64"),
        _ => None,
    }
}

/// The storage pack release for a firmware version: the newest QuadCam has checked for that
/// series.
pub fn pack_for(firmware: &str) -> Option<&'static str> {
    let mut it = firmware.trim().trim_start_matches('v').split('.');
    match (it.next(), it.next()) {
        (Some("2"), Some("12")) => Some("2.12.3"),
        _ => None,
    }
}

/// The pack's download (GitHub release asset of EdgeTX/edgetx-sdcard).
pub fn pack_url(version: &str, name: &str) -> String {
    format!("https://github.com/EdgeTX/edgetx-sdcard/releases/download/v{version}/{name}.zip")
}

/// The ELRS Lua of a release (a GitHub release asset of ExpressLRS/ExpressLRS).
pub fn elrs_lua_url(version: &str) -> String {
    format!("https://github.com/ExpressLRS/ExpressLRS/releases/download/{version}/elrs.lua")
}

/// A version string fit for a URL: digits and dots.
pub fn valid_version(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 16
        && v.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && v.split('.').all(|p| !p.is_empty())
}

/// What putting a storage pack on the card would do.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct PackPlan {
    /// `edgetx.sdcard.version` on the card now.
    #[serde(default)]
    pub version_before: Option<String>,
    /// The pack's own version.
    #[serde(default)]
    pub version_after: Option<String>,
    /// Files the card lacks or holds with other bytes.
    pub put: Vec<String>,
    /// Files the card already holds as the pack has them.
    pub unchanged: u32,
    /// Files of the pack QuadCam never puts (a model or `radio.yml`).
    pub skipped: Vec<String>,
    /// Bytes to write.
    pub bytes: u64,
}

/// True for a pack file QuadCam may put on a card: inside it, no hidden part, and never a
/// YAML file (models and `radio.yml` are the person's).
pub fn pack_path_ok(path: &str) -> bool {
    crate::gear::edgetx::card::inside_card(path)
        && !path.split('/').any(|c| c.starts_with('.'))
        && !path.to_ascii_lowercase().ends_with(".yml")
}

/// The pack's files that differ from the card's. Never deletes: files the pack lacks stay.
pub fn plan_pack(
    src: &dyn Source,
    files: &[(String, Vec<u8>)],
) -> (PackPlan, Vec<(String, Vec<u8>)>) {
    let mut plan = PackPlan {
        version_before: src
            .read(PACK_VERSION_FILE)
            .map(|b| latin1(&b).trim().to_string()),
        version_after: None,
        put: vec![],
        unchanged: 0,
        skipped: vec![],
        bytes: 0,
    };
    let mut out = Vec::new();
    for (path, bytes) in files {
        if path == PACK_VERSION_FILE {
            plan.version_after = Some(latin1(bytes).trim().to_string());
        }
        if !pack_path_ok(path) {
            plan.skipped.push(path.clone());
            continue;
        }
        if src.read(path).as_deref() == Some(bytes.as_slice()) {
            plan.unchanged += 1;
            continue;
        }
        plan.put.push(path.clone());
        plan.bytes += bytes.len() as u64;
        out.push((path.clone(), bytes.clone()));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    plan.put.sort();
    (plan, out)
}

/// The card edits that put the ELRS Lua on and take the old names off.
pub fn plan_elrs_lua(src: &dyn Source, lua: &[u8]) -> Result<Vec<Edit>, Refusal> {
    if lua.is_empty() || !lua.is_ascii() && std::str::from_utf8(lua).is_err() {
        return Err(Refusal::new(
            RefusalCode::BadImage,
            "The ELRS Lua is empty or not text; nothing was planned.",
        ));
    }
    let mut out = Vec::new();
    if src.read(ELRS_LUA).as_deref() != Some(lua) {
        out.push(Edit::CardText {
            path: ELRS_LUA.into(),
            text: Some(latin1(lua)),
        });
    }
    for old in OLD_ELRS_LUA {
        if src.read(old).is_some() {
            out.push(Edit::CardText {
                path: old.to_string(),
                text: None,
            });
        }
    }
    Ok(out)
}

/// The most characters a FAT volume label holds.
pub const MAX_LABEL: usize = 11;

/// A volume label made from a radio's saved name: upper case, letters, digits, `_` and `-`
/// kept, spaces dropped, 11 characters at most (`Pocket 2` -> `POCKET2`).
pub fn volume_label(name: &str) -> Result<String, Refusal> {
    let label: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        .map(|c| c.to_ascii_uppercase())
        .take(MAX_LABEL)
        .collect();
    if label.is_empty() {
        return Err(Refusal::new(
            RefusalCode::BadSetting,
            format!("{name:?} has no letters or digits for a volume label."),
        ));
    }
    Ok(label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Mem(BTreeMap<String, Vec<u8>>);

    impl Source for Mem {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            self.0.get(path).cloned()
        }
        fn files(&self) -> Vec<String> {
            self.0.keys().cloned().collect()
        }
    }

    fn card(files: &[(&str, &[u8])]) -> Mem {
        Mem(files
            .iter()
            .map(|(p, b)| (p.to_string(), b.to_vec()))
            .collect())
    }

    #[test]
    fn a_pack_puts_only_what_differs_and_never_yaml() {
        let src = card(&[
            ("edgetx.sdcard.version", b"2.10.0\n"),
            ("SCRIPTS/TOOLS/readme.txt", b"same"),
            ("SCRIPTS/snake.lua", b"old"),
        ]);
        let files = vec![
            ("edgetx.sdcard.version".to_string(), b"2.12.3\n".to_vec()),
            ("SCRIPTS/TOOLS/readme.txt".into(), b"same".to_vec()),
            ("SCRIPTS/snake.lua".into(), b"new".to_vec()),
            ("RADIO/radio.yml".into(), b"x".to_vec()),
            ("SCRIPTS/RGBLED/red.lua".into(), b"red".to_vec()),
        ];
        let (plan, put) = plan_pack(&src, &files);
        assert_eq!(plan.version_before.as_deref(), Some("2.10.0"));
        assert_eq!(plan.version_after.as_deref(), Some("2.12.3"));
        assert_eq!(
            plan.put,
            [
                "SCRIPTS/RGBLED/red.lua",
                "SCRIPTS/snake.lua",
                "edgetx.sdcard.version"
            ]
        );
        assert_eq!(plan.unchanged, 1);
        assert_eq!(plan.skipped, ["RADIO/radio.yml"]);
        assert_eq!(put.len(), 3);
        assert_eq!(plan.bytes, 3 + 3 + 7);
    }

    #[test]
    fn the_elrs_lua_replaces_the_old_names() {
        let src = card(&[
            ("SCRIPTS/TOOLS/elrsV3.lua", b"old"),
            ("SCRIPTS/TOOLS/elrsV3.luac", b"\x1bLua"),
        ]);
        let edits = plan_elrs_lua(&src, b"-- elrs").unwrap();
        assert_eq!(edits.len(), 3);
        assert!(matches!(&edits[0], Edit::CardText { path, text: Some(_) } if path == ELRS_LUA));
        assert!(
            matches!(&edits[2], Edit::CardText { path, text: None } if path.ends_with(".luac"))
        );
        let done = card(&[(ELRS_LUA, b"-- elrs")]);
        assert!(plan_elrs_lua(&done, b"-- elrs").unwrap().is_empty());
        assert!(plan_elrs_lua(&done, b"").is_err());
    }

    #[test]
    fn labels_and_versions() {
        assert_eq!(volume_label("Pocket 2").unwrap(), "POCKET2");
        assert_eq!(
            volume_label("RadioMaster GX12 Crush").unwrap(),
            "RADIOMASTER"
        );
        assert!(volume_label("  !! ").is_err());
        assert_eq!(pack_for("2.12.5"), Some("2.12.3"));
        assert_eq!(pack_for("2.11.0"), None);
        assert_eq!(pack_name("GX12"), Some("bw128x64"));
        assert!(valid_version("4.1.0"));
        assert!(!valid_version("4.1.0/../x"));
        assert_eq!(
            pack_url("2.12.3", "bw128x64"),
            "https://github.com/EdgeTX/edgetx-sdcard/releases/download/v2.12.3/bw128x64.zip"
        );
    }
}
