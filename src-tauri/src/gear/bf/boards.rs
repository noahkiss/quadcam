//! What QuadCam knows about particular FC boards and builds: known issues to show after a
//! job and on the device page, how long a board may run on USB with its battery in, and
//! where a board's maker publishes its own builds (`VENDOR_SOURCES`). Data, reviewed per
//! release, like `compat`. A row names a board (`board_name`, any case) and optionally a
//! firmware version prefix (`compat::version_matches`).

use crate::gear::compat::version_matches;
use crate::gear::model::Identity;
use serde::{Deserialize, Serialize};
use specta::Type;

/// When a known issue matters.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum NoteWhen {
    /// Shown after every job on the FC, and on its page.
    AfterUsbSession,
    /// Shown on the device page only.
    Always,
}

/// One known issue of a board, or a board and build.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct BoardNote {
    pub board: String,
    /// A version prefix; None for every version.
    pub version: Option<String>,
    pub when: NoteWhen,
    pub text: String,
}

struct NoteRow {
    board: &'static str,
    version: Option<&'static str>,
    when: NoteWhen,
    text: &'static str,
}

const NOTES: &[NoteRow] = &[NoteRow {
    board: "BETAFPVG473",
    version: Some("2025.12.5"),
    when: NoteWhen::AfterUsbSession,
    text: "After a USB session the beeper and the lost-model finder stay silent until the \
           battery is unplugged and plugged in again.",
}];

/// Minutes a board may run on USB with its battery in before "Unplug now". Small quads
/// with no airflow heat fast, and an HD air unit adds its own heat.
const USB_LIMITS: &[(&str, u32)] = &[("BETAFPVG473", 10), ("BETAFPVG473_V2", 10)];

fn board_eq(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// Every known issue, or those for this board (and version, when given).
pub fn notes(board: Option<&str>, version: Option<&str>) -> Vec<BoardNote> {
    NOTES
        .iter()
        .filter(|n| board.is_none_or(|b| board_eq(n.board, b)))
        .filter(|n| match (n.version, version) {
            (Some(want), Some(have)) => version_matches(want, have),
            _ => true,
        })
        .map(|n| BoardNote {
            board: n.board.into(),
            version: n.version.map(str::to_string),
            when: n.when,
            text: n.text.into(),
        })
        .collect()
}

/// The notes to show after a job on this board and version.
pub fn after_job(board: Option<&str>, version: Option<&str>) -> Vec<String> {
    let Some(b) = board else {
        return Vec::new();
    };
    notes(Some(b), version)
        .into_iter()
        .filter(|n| {
            n.when == NoteWhen::AfterUsbSession && (n.version.is_none() || version.is_some())
        })
        .map(|n| n.text)
        .collect()
}

/// The board's own USB limit in minutes, when the table has one.
pub fn usb_minutes(board: Option<&str>) -> Option<u32> {
    let b = board?;
    USB_LIMITS
        .iter()
        .find(|(n, _)| board_eq(n, b))
        .map(|(_, m)| *m)
}

/// The limit in effect: the shorter of the setting and the board's own; 0 (the setting
/// off) turns the timer off.
pub fn usb_limit(setting_minutes: u32, board: Option<&str>) -> Option<u32> {
    if setting_minutes == 0 {
        return None;
    }
    Some(match usb_minutes(board) {
        Some(m) => m.min(setting_minutes),
        None => setting_minutes,
    })
}

/// Where a maker publishes its builds. Each kind has its own reader in
/// `gear::firmware::vendor`; a maker with another kind of feed (GitHub releases, a WordPress
/// category) adds a variant and its reader.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Feed {
    /// The attachments of one Zendesk Help Center article: `.hex` and `.txt` pairs, the
    /// `.txt` a `diff all` that starts with the build's version line.
    Zendesk { host: String, article: String },
}

/// Which of a maker's files a row takes: Betaflight cloud builds (a git revision) or the
/// maker's local builds (`norevision`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum BuildKind {
    Cloud,
    Local,
}

/// A maker's own builds for one quad: the vendor channel of an FC.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct VendorSource {
    /// The maker's name, as the page shows it (`BetaFPV`).
    pub vendor: String,
    /// The quad the files are for (`Meteor75 Pro II O4`).
    pub product: String,
    pub feed: Feed,
    /// The start of the file names that belong to this quad.
    pub prefix: String,
    pub build: BuildKind,
}

struct VendorRow {
    /// `manufacturer_id`; an FC that did not report one passes.
    manufacturer: &'static str,
    board: &'static str,
    /// Words, one of which the linked aircraft profile's name or aircraft holds (letters and
    /// digits only, any case). Empty for any aircraft.
    aircraft: &'static [&'static str],
    /// Gyros this row is for. Empty for any gyro.
    gyro: &'static [&'static str],
    vendor: &'static str,
    product: &'static str,
    host: &'static str,
    article: &'static str,
    prefix: &'static str,
    build: BuildKind,
}

const BETAFPV_HELP: &str = "support.betafpv.com";

/// The makers' builds QuadCam follows. A board alone is not enough: the Air65 II and the
/// analog Meteor75 Pro II both report `BETAFPVG473_V2`, so a row also names the aircraft.
/// The Air65 II ships two gyros: BetaFPV's BMI270 files are cloud builds, its ICM/LSM files
/// local builds.
const VENDOR_SOURCES: &[VendorRow] = &[
    VendorRow {
        manufacturer: "BEFH",
        board: "BETAFPVG473",
        aircraft: &["meteor75"],
        gyro: &[],
        vendor: "BetaFPV",
        product: "Meteor75 Pro II O4",
        host: BETAFPV_HELP,
        article: "59855387002521",
        prefix: "M75 Pro II O4_GF 1811_ELRS",
        build: BuildKind::Cloud,
    },
    VendorRow {
        manufacturer: "BEFH",
        board: "BETAFPVG473_V2",
        aircraft: &["meteor75"],
        gyro: &[],
        vendor: "BetaFPV",
        product: "Meteor75 Pro II",
        host: BETAFPV_HELP,
        article: "62069650864921",
        prefix: "M75 Pro II_GF 1811_ELRS",
        build: BuildKind::Cloud,
    },
    VendorRow {
        manufacturer: "BEFH",
        board: "BETAFPVG473_V2",
        aircraft: &["air65"],
        gyro: &["BMI270"],
        vendor: "BetaFPV",
        product: "Air65 II",
        host: BETAFPV_HELP,
        article: "54654422544409",
        prefix: "A65 II F_0702_Freestyle",
        build: BuildKind::Cloud,
    },
    VendorRow {
        manufacturer: "BEFH",
        board: "BETAFPVG473_V2",
        aircraft: &["air65"],
        gyro: &["ICM42688P", "LSM6DSV16X", "LSM6DSO"],
        vendor: "BetaFPV",
        product: "Air65 II",
        host: BETAFPV_HELP,
        article: "54654422544409",
        prefix: "A65 II F_0702_Freestyle",
        build: BuildKind::Local,
    },
];

fn squash(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The maker's builds for this FC, from its identity and the names of the aircraft profile
/// it is linked to (the profile's name and its aircraft). None when no row fits. With two
/// rows left (the Air65 II's gyros and no gyro read yet), the FC's build decides: a cloud
/// build key picks the cloud row.
pub fn vendor_source(id: &Identity, aircraft: &[&str]) -> Option<VendorSource> {
    let board = id.board.as_deref()?;
    let names: Vec<String> = aircraft.iter().map(|a| squash(a)).collect();
    let mut rows: Vec<&VendorRow> = VENDOR_SOURCES
        .iter()
        .filter(|r| board_eq(r.board, board))
        .filter(|r| {
            id.manufacturer_id
                .as_deref()
                .is_none_or(|m| m.trim().eq_ignore_ascii_case(r.manufacturer))
        })
        .filter(|r| {
            r.aircraft.is_empty()
                || r.aircraft
                    .iter()
                    .any(|w| names.iter().any(|n| n.contains(&squash(w))))
        })
        .collect();
    if let Some(g) = id.gyro.as_deref() {
        rows.retain(|r| r.gyro.is_empty() || r.gyro.iter().any(|x| x.eq_ignore_ascii_case(g)));
    }
    let want = if id.build_key().is_some() {
        BuildKind::Cloud
    } else {
        BuildKind::Local
    };
    let row = match rows.as_slice() {
        [] => return None,
        [one] => *one,
        many => many
            .iter()
            .find(|r| r.build == want)
            .copied()
            .unwrap_or(many[0]),
    };
    Some(VendorSource {
        vendor: row.vendor.into(),
        product: row.product.into(),
        feed: Feed::Zendesk {
            host: row.host.into(),
            article: row.article.into(),
        },
        prefix: row.prefix.into(),
        build: row.build,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_match_board_and_build() {
        assert_eq!(notes(None, None).len(), NOTES.len());
        assert_eq!(
            after_job(Some("betafpvg473"), Some("2025.12.5-alpha")).len(),
            1
        );
        assert!(after_job(Some("BETAFPVG473"), Some("2026.6.0-alpha")).is_empty());
        assert!(after_job(Some("BETAFPVG473_V2"), Some("2025.12.5-alpha")).is_empty());
        assert!(after_job(None, Some("2025.12.5")).is_empty());
        // Unknown version: the page lists it, a job does not claim it.
        assert_eq!(notes(Some("BETAFPVG473"), None).len(), 1);
        assert!(after_job(Some("BETAFPVG473"), None).is_empty());
    }

    fn fc(board: &str, mfr: Option<&str>, key: Option<&str>, gyro: Option<&str>) -> Identity {
        Identity {
            board: Some(board.into()),
            manufacturer_id: mfr.map(str::to_string),
            build_key: key.map(str::to_string),
            gyro: gyro.map(str::to_string),
            ..Identity::default()
        }
    }

    #[test]
    fn vendor_rows_need_board_maker_and_aircraft() {
        let m = vendor_source(
            &fc("BETAFPVG473", Some("BEFH"), None, None),
            &["Meteor75 Pro II", ""],
        )
        .unwrap();
        assert_eq!(m.product, "Meteor75 Pro II O4");
        assert_eq!(m.vendor, "BetaFPV");
        assert_eq!(
            m.feed,
            Feed::Zendesk {
                host: "support.betafpv.com".into(),
                article: "59855387002521".into()
            }
        );
        // An FC saved before QuadCam read the maker code still matches.
        assert!(vendor_source(&fc("betafpvg473", None, None, None), &["meteor 75"]).is_some());
        // Another maker, no aircraft, or another aircraft: no row.
        assert!(
            vendor_source(&fc("BETAFPVG473", Some("XXXX"), None, None), &["Meteor75"]).is_none()
        );
        assert!(vendor_source(&fc("BETAFPVG473", Some("BEFH"), None, None), &[]).is_none());
        assert!(vendor_source(&fc("BETAFPVG473", Some("BEFH"), None, None), &["Air65"]).is_none());
        assert!(vendor_source(&fc("STM32F411", None, None, None), &["Meteor75"]).is_none());
        // The shared V2 board: the aircraft decides.
        let a = vendor_source(
            &fc("BETAFPVG473_V2", Some("BEFH"), None, None),
            &["Air65 II"],
        )
        .unwrap();
        assert_eq!(a.product, "Air65 II");
        let m = vendor_source(
            &fc("BETAFPVG473_V2", Some("BEFH"), None, None),
            &["Meteor75 Pro II"],
        )
        .unwrap();
        assert_eq!(m.product, "Meteor75 Pro II");
    }

    #[test]
    fn the_air65_gyro_or_build_picks_its_files() {
        let air = |key, gyro| {
            vendor_source(
                &fc("BETAFPVG473_V2", Some("BEFH"), key, gyro),
                &["Air65 II"],
            )
            .unwrap()
            .build
        };
        assert_eq!(air(None, Some("BMI270")), BuildKind::Cloud);
        assert_eq!(air(None, Some("ICM42688P")), BuildKind::Local);
        // No gyro read: a build key means the cloud files, none the local ones.
        assert_eq!(
            air(Some("00112233445566778899aabbccddeeff"), None),
            BuildKind::Cloud
        );
        assert_eq!(air(None, None), BuildKind::Local);
        assert_eq!(air(Some(""), None), BuildKind::Local);
    }

    #[test]
    fn usb_limit_is_the_shorter() {
        assert_eq!(usb_limit(20, Some("BETAFPVG473_V2")), Some(10));
        assert_eq!(usb_limit(5, Some("BETAFPVG473")), Some(5));
        assert_eq!(usb_limit(20, Some("OTHER")), Some(20));
        assert_eq!(usb_limit(20, None), Some(20));
        assert_eq!(usb_limit(0, Some("BETAFPVG473")), None);
    }
}
