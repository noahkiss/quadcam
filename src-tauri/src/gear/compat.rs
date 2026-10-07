//! The versions QuadCam has proven it can write. Data, reviewed per release: a pair that is
//! not listed here is read-only, with a reason the UI shows. Each write path calls the guard
//! for its product right before it writes (design 8).
//!
//! A version entry matches that version and every version under it: `2.12` matches
//! `2.12.4`, and `2026.6` matches `2026.6.0-alpha`. A board of `None` matches any board.

use super::model::{Refusal, RefusalCode};
use serde::Serialize;
use specta::Type;

/// What a compatibility entry is about.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum Product {
    /// EdgeTX SD-card files (YAML models, `radio.yml`, sounds).
    Edgetx,
    /// The Betaflight CLI (backup, apply, verify).
    Betaflight,
    /// An ExpressLRS flash target.
    Elrs,
    /// The EdgeTX splash image inside a firmware binary.
    Splash,
    /// A sim's rate file.
    Sim,
}

impl Product {
    pub fn label(self) -> &'static str {
        match self {
            Product::Edgetx => "EdgeTX",
            Product::Betaflight => "Betaflight",
            Product::Elrs => "ExpressLRS",
            Product::Splash => "The splash of EdgeTX",
            Product::Sim => "The sim file",
        }
    }
}

/// One proven pair.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
pub struct Proven {
    pub product: Product,
    /// The board or target, lowercase. None: any board.
    pub board: Option<&'static str>,
    /// A version or version prefix (see the module docs).
    pub version: &'static str,
}

/// Every proven pair. Add a row only after a test on that firmware passes (design 11).
pub const PROVEN: &[Proven] = &[
    // EdgeTX 2.12 on a 128x64 B&W radio: the card patcher's revisions 1 to 9.
    Proven {
        product: Product::Edgetx,
        board: Some("pocket"),
        version: "2.12",
    },
    // Betaflight's CLI, by the serial runner: the 4.5 line and the 2026.6 line.
    Proven {
        product: Product::Betaflight,
        board: None,
        version: "4.5",
    },
    Proven {
        product: Product::Betaflight,
        board: None,
        version: "2026.6",
    },
    // The splash patch: markers checked on this release's binary.
    Proven {
        product: Product::Splash,
        board: Some("pocket"),
        version: "2.12.4",
    },
    // ExpressLRS targets and sim file versions are added by the packages that prove them.
];

/// True when `version` is `prefix` or starts with `prefix` and then `.` or `-`.
pub fn version_matches(prefix: &str, version: &str) -> bool {
    let v = version.trim().trim_start_matches(['v', 'V']);
    match v.strip_prefix(prefix) {
        Some("") => true,
        Some(rest) => rest.starts_with('.') || rest.starts_with('-'),
        None => false,
    }
}

/// The proven entry for this product, board and version, if any.
pub fn proven(product: Product, board: Option<&str>, version: &str) -> Option<&'static Proven> {
    let board = board.map(|b| b.trim().to_ascii_lowercase());
    PROVEN.iter().find(|p| {
        p.product == product
            && version_matches(p.version, version)
            && match (p.board, board.as_deref()) {
                (None, _) => true,
                (Some(want), Some(have)) => want == have,
                (Some(_), None) => false,
            }
    })
}

/// The guard: Ok for a proven pair, else the refusal the UI shows. A board QuadCam has
/// never proven any version on is `unknown_board`; a known board on another version is
/// `unknown_version`.
pub fn check_writable(
    product: Product,
    board: Option<&str>,
    version: Option<&str>,
) -> Result<(), Refusal> {
    let Some(version) = version.filter(|v| !v.trim().is_empty()) else {
        return Err(Refusal::new(
            RefusalCode::UnknownVersion,
            format!(
                "{} did not report its version; QuadCam reads it but does not write it.",
                product.label()
            ),
        ));
    };
    if proven(product, board, version).is_some() {
        return Ok(());
    }
    let board_known = PROVEN.iter().any(|p| {
        p.product == product
            && match (p.board, board) {
                (None, _) => true,
                (Some(want), Some(have)) => want.eq_ignore_ascii_case(have.trim()),
                (Some(_), None) => false,
            }
    });
    if !board_known {
        return Err(Refusal::new(
            RefusalCode::UnknownBoard,
            format!("Board {} is not proven.", board.unwrap_or("(none)")),
        ));
    }
    let on = board.map(|b| format!(" on board {b}")).unwrap_or_default();
    Err(Refusal::new(
        RefusalCode::UnknownVersion,
        format!(
            "{} {version}{on} is not proven; QuadCam reads it but does not write it.",
            product.label()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_prefixes() {
        assert!(version_matches("2.12", "2.12"));
        assert!(version_matches("2.12", "2.12.4"));
        assert!(version_matches("2.12", "v2.12.4"));
        assert!(version_matches("2026.6", "2026.6.0-alpha"));
        assert!(!version_matches("2.12", "2.120"));
        assert!(!version_matches("2.12", "2.1"));
        assert!(!version_matches("4.5", "4.50.1"));
    }

    #[test]
    fn edgetx_guard() {
        assert!(check_writable(Product::Edgetx, Some("pocket"), Some("2.12.4")).is_ok());
        assert!(check_writable(Product::Edgetx, Some("Pocket"), Some("2.12.0")).is_ok());
        let e = check_writable(Product::Edgetx, Some("pocket"), Some("2.11.3")).unwrap_err();
        assert_eq!(e.code, RefusalCode::UnknownVersion);
        assert_eq!(
            e.reason,
            "EdgeTX 2.11.3 on board pocket is not proven; QuadCam reads it but does not write it."
        );
        let e = check_writable(Product::Edgetx, Some("tx16s"), Some("2.12.4")).unwrap_err();
        assert_eq!(e.code, RefusalCode::UnknownBoard);
        let e = check_writable(Product::Edgetx, None, Some("2.12.4")).unwrap_err();
        assert_eq!(e.code, RefusalCode::UnknownBoard);
        let e = check_writable(Product::Edgetx, Some("pocket"), None).unwrap_err();
        assert_eq!(e.code, RefusalCode::UnknownVersion);
    }

    #[test]
    fn betaflight_guard() {
        assert!(check_writable(Product::Betaflight, Some("STM32F411"), Some("4.5.1")).is_ok());
        assert!(check_writable(Product::Betaflight, None, Some("2026.6.0-alpha")).is_ok());
        let e = check_writable(Product::Betaflight, None, Some("4.3.2")).unwrap_err();
        assert_eq!(e.code, RefusalCode::UnknownVersion);
        assert_eq!(
            e.reason,
            "Betaflight 4.3.2 is not proven; QuadCam reads it but does not write it."
        );
    }

    #[test]
    fn nothing_proven_refuses() {
        let e = check_writable(Product::Elrs, Some("any"), Some("3.5.0")).unwrap_err();
        assert_eq!(e.code, RefusalCode::UnknownBoard);
        assert!(check_writable(Product::Splash, Some("pocket"), Some("2.12.3")).is_err());
        assert!(check_writable(Product::Splash, Some("pocket"), Some("2.12.4")).is_ok());
    }
}
