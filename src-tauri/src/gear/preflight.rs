//! The "Pack up" check before a flying session: read-only rows that pass or warn. Each row
//! uses what QuadCam already knows; a row it cannot judge reads as unknown.
//!
//! Inputs come from `core/flights.rs`. Later packages fill more of them:
//! - the radio's selected model when it is not plugged in: TODO(WP4) from the latest radio
//!   backup's `radio.yml`;
//! - free space on a card that is not plugged in: TODO(WP4) from the space seen at its last
//!   backup;
//! - backup times come from `Device::last_backup` (WP4).

use super::packs::{ChargeState, PackView};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use specta::Type;

/// A backup older than this many days warns.
pub const BACKUP_MAX_DAYS: i64 = 14;
/// A card with less free space than this warns, in bytes.
pub const CARD_MIN_FREE: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum RowState {
    Pass,
    Warn,
    Unknown,
}

/// One check.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct CheckRow {
    /// `packs`, `radio`, `cards_space`, `backups`, `cards_in`.
    pub id: String,
    pub label: String,
    pub state: RowState,
    pub detail: String,
}

/// `gear_preflight`'s answer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct Preflight {
    pub rows: Vec<CheckRow>,
}

/// A saved radio, as the check sees it.
#[derive(Debug, Clone, Default)]
pub struct RadioSeen {
    pub name: String,
    pub connected: bool,
    /// The model the radio selects, when its card could be read.
    pub selected_model: Option<String>,
    /// The aircraft that model belongs to.
    pub aircraft: Option<String>,
    pub last_seen: Option<NaiveDateTime>,
}

/// A goggles, DVR or air-unit card plugged in now.
#[derive(Debug, Clone, Default)]
pub struct CardSpace {
    pub name: String,
    pub free: Option<u64>,
    pub total: Option<u64>,
}

/// A saved device and its latest backup.
#[derive(Debug, Clone, Default)]
pub struct BackupSeen {
    pub name: String,
    pub last_backup: Option<NaiveDateTime>,
}

/// Everything the check reads.
#[derive(Debug, Clone, Default)]
pub struct Input {
    pub packs: Vec<PackView>,
    pub radios: Vec<RadioSeen>,
    pub cards: Vec<CardSpace>,
    pub backups: Vec<BackupSeen>,
    /// Cards and radios still mounted or still inserted.
    pub cards_in: Vec<String>,
    pub now: NaiveDateTime,
}

fn gb(b: u64) -> String {
    format!("{:.1} GB", b as f64 / 1e9)
}

fn row(id: &str, label: &str, state: RowState, detail: String) -> CheckRow {
    CheckRow {
        id: id.into(),
        label: label.into(),
        state,
        detail,
    }
}

/// The rows, in a fixed order.
pub fn check(i: &Input) -> Preflight {
    let mut rows = Vec::new();

    // Packs.
    let live: Vec<&PackView> = i.packs.iter().filter(|p| !p.pack.retired).collect();
    rows.push(if live.is_empty() {
        row(
            "packs",
            "Packs charged",
            RowState::Unknown,
            "No packs saved.".into(),
        )
    } else {
        let charged = live
            .iter()
            .filter(|p| p.state == ChargeState::Charged)
            .count();
        let not: Vec<&str> = live
            .iter()
            .filter(|p| p.state != ChargeState::Charged)
            .map(|p| p.pack.label.as_str())
            .collect();
        if not.is_empty() {
            row(
                "packs",
                "Packs charged",
                RowState::Pass,
                format!("{charged} of {} charged.", live.len()),
            )
        } else {
            row(
                "packs",
                "Packs charged",
                RowState::Warn,
                format!(
                    "{charged} of {} charged. Not charged: {}.",
                    live.len(),
                    not.join(", ")
                ),
            )
        }
    });

    // Radio.
    rows.push(match i.radios.iter().find(|r| r.connected) {
        Some(r) => match &r.selected_model {
            Some(m) => row(
                "radio",
                "Radio model",
                if r.aircraft.is_some() {
                    RowState::Pass
                } else {
                    RowState::Warn
                },
                match &r.aircraft {
                    Some(a) => format!("{}: model {m} selected ({a}).", r.name),
                    None => format!("{}: model {m} selected, not linked to an aircraft.", r.name),
                },
            ),
            None => row(
                "radio",
                "Radio model",
                RowState::Unknown,
                format!("{} is plugged in; its card could not be read.", r.name),
            ),
        },
        None => match i
            .radios
            .iter()
            .filter_map(|r| r.last_seen.map(|t| (r, t)))
            .max_by_key(|x| x.1)
        {
            Some((r, t)) => row(
                "radio",
                "Radio model",
                RowState::Unknown,
                format!(
                    "{} last seen {}. Plug it in to check the model.",
                    r.name,
                    t.format("%Y-%m-%d %H:%M")
                ),
            ),
            None => row(
                "radio",
                "Radio model",
                RowState::Unknown,
                "No radio saved.".into(),
            ),
        },
    });

    // Free space.
    rows.push(if i.cards.is_empty() {
        row(
            "cards_space",
            "Card space",
            RowState::Unknown,
            "Plug in the goggles, DVR or air unit card to check its free space.".into(),
        )
    } else {
        let low: Vec<&CardSpace> = i
            .cards
            .iter()
            .filter(|c| {
                c.free.is_some_and(|f| {
                    f < CARD_MIN_FREE || c.total.is_some_and(|t| t > 0 && f * 10 < t)
                })
            })
            .collect();
        let detail = i
            .cards
            .iter()
            .map(|c| match c.free {
                Some(f) => format!("{}: {} free", c.name, gb(f)),
                None => format!("{}: free space unknown", c.name),
            })
            .collect::<Vec<_>>()
            .join("; ");
        row(
            "cards_space",
            "Card space",
            if low.is_empty() {
                RowState::Pass
            } else {
                RowState::Warn
            },
            format!("{detail}."),
        )
    });

    // Backups.
    rows.push(if i.backups.is_empty() {
        row(
            "backups",
            "Backups",
            RowState::Unknown,
            "No radio or FC saved.".into(),
        )
    } else {
        let mut old = Vec::new();
        let mut parts = Vec::new();
        for b in &i.backups {
            match b.last_backup {
                Some(t) => {
                    let days = (i.now - t).num_days();
                    parts.push(format!("{}: {} days ago", b.name, days.max(0)));
                    if days > BACKUP_MAX_DAYS {
                        old.push(&b.name);
                    }
                }
                None => {
                    parts.push(format!("{}: never", b.name));
                    old.push(&b.name);
                }
            }
        }
        row(
            "backups",
            "Backups",
            if old.is_empty() {
                RowState::Pass
            } else {
                RowState::Warn
            },
            format!("{}.", parts.join("; ")),
        )
    });

    // Cards left in the Mac.
    rows.push(if i.cards_in.is_empty() {
        row(
            "cards_in",
            "Cards out",
            RowState::Pass,
            "No card in the Mac.".into(),
        )
    } else {
        row(
            "cards_in",
            "Cards out",
            RowState::Warn,
            format!("Still in the Mac: {}.", i.cards_in.join(", ")),
        )
    });

    Preflight { rows }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gear::packs::Pack;

    fn pack(label: &str, state: ChargeState) -> PackView {
        PackView {
            pack: Pack {
                label: label.into(),
                ..Default::default()
            },
            cycles: 0,
            state,
            last_flown: None,
            median_resting_v: None,
            median_secs: None,
            weak: None,
            history: vec![],
        }
    }

    fn now() -> NaiveDateTime {
        NaiveDateTime::parse_from_str("2026-10-07 09:00:00", "%Y-%m-%d %H:%M:%S").unwrap()
    }

    #[test]
    fn empty_input_is_unknown_except_cards_out() {
        let p = check(&Input {
            now: now(),
            ..Default::default()
        });
        let states: Vec<RowState> = p.rows.iter().map(|r| r.state).collect();
        use RowState::*;
        assert_eq!(states, vec![Unknown, Unknown, Unknown, Unknown, Pass]);
    }

    #[test]
    fn warns_and_passes() {
        let p = check(&Input {
            packs: vec![
                pack("A1", ChargeState::Charged),
                pack("A2", ChargeState::Flown),
            ],
            radios: vec![RadioSeen {
                name: "Radio".into(),
                connected: true,
                selected_model: Some("Whoop".into()),
                aircraft: Some("Whoop".into()),
                last_seen: None,
            }],
            cards: vec![CardSpace {
                name: "DVR".into(),
                free: Some(1_000_000_000),
                total: Some(32_000_000_000),
            }],
            backups: vec![
                BackupSeen {
                    name: "Radio".into(),
                    last_backup: Some(now() - chrono::Duration::days(3)),
                },
                BackupSeen {
                    name: "FC".into(),
                    last_backup: None,
                },
            ],
            cards_in: vec!["DVR".into()],
            now: now(),
        });
        let r = |id: &str| p.rows.iter().find(|r| r.id == id).unwrap().clone();
        assert_eq!(r("packs").state, RowState::Warn);
        assert_eq!(r("packs").detail, "1 of 2 charged. Not charged: A2.");
        assert_eq!(r("radio").state, RowState::Pass);
        assert_eq!(r("cards_space").state, RowState::Warn);
        assert_eq!(r("cards_space").detail, "DVR: 1.0 GB free.");
        assert_eq!(r("backups").state, RowState::Warn);
        assert_eq!(r("backups").detail, "Radio: 3 days ago; FC: never.");
        assert_eq!(r("cards_in").state, RowState::Warn);
    }
}
