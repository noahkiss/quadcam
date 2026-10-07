//! What QuadCam knows about particular FC boards and builds: known issues to show after a
//! job and on the device page, and how long a board may run on USB with its battery in.
//! Data, reviewed per release, like `compat`. A row names a board (`board_name`, any case)
//! and optionally a firmware version prefix (`compat::version_matches`).

use crate::gear::compat::version_matches;
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

    #[test]
    fn usb_limit_is_the_shorter() {
        assert_eq!(usb_limit(20, Some("BETAFPVG473_V2")), Some(10));
        assert_eq!(usb_limit(5, Some("BETAFPVG473")), Some(5));
        assert_eq!(usb_limit(20, Some("OTHER")), Some(20));
        assert_eq!(usb_limit(20, None), Some(20));
        assert_eq!(usb_limit(0, Some("BETAFPVG473")), None);
    }
}
