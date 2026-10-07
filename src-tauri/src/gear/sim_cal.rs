//! The sim's radio calibrations (sim design 7.5) and the defaults QuadCam already knows
//! (7.2).
//!
//! - Stored in `<gear>/sim/calibrations.json`, keyed by the Gear radio's device id, never
//!   by USB serial: every EdgeTX radio of one MCU type reports the same serial.
//! - In joystick mode the radio's card is not mounted, so `resolve` works out which saved
//!   radio it is from the USB product name: the one saved radio whose board the name
//!   holds; the one the user picked before; else ask. A radio never seen in storage mode
//!   gets a provisional key (`usb-…`) the user can link to a saved radio later.
//! - `defaults` reads an aircraft's switch map: the stick channels from its EdgeTX model,
//!   and the arm, angle, horizon, turtle and air mode switches from the quad's `aux` lines,
//!   each with where it came from. It suggests a reset control: one that moves a channel
//!   the sim sees but does nothing in the model or on the quad.

use super::model::{Device, DeviceKind};
use super::switchmap::{ControlKind, Stick, SwitchMap};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use quadcam_sim::input::{Calibration, RadioControl, SWITCH_MARGIN_US};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `<gear>/sim/calibrations.json`.
pub fn file(gear: &Path) -> PathBuf {
    gear.join("sim").join("calibrations.json")
}

/// One radio's saved calibration and what it was made with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SavedCalibration {
    /// The Gear radio's device id, or a provisional `usb-…` key.
    pub radio: String,
    #[serde(default)]
    pub provisional: bool,
    pub calibration: Calibration,
    /// The USB product name (`Radiomaster Pocket Joystick`).
    #[serde(default)]
    pub product: Option<String>,
    pub vid: u16,
    pub pid: u16,
    /// The firmware version the radio reported over USB (`2.12`).
    #[serde(default)]
    pub firmware: Option<String>,
    pub saved_at: DateTime<Utc>,
}

/// The file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CalibrationFile {
    #[serde(default)]
    pub radios: BTreeMap<String, SavedCalibration>,
    /// The radio the user picked for a product name ("Which radio is this?").
    #[serde(default)]
    pub products: BTreeMap<String, String>,
}

pub fn read(gear: &Path) -> Result<CalibrationFile> {
    let p = file(gear);
    match std::fs::read(&p) {
        Ok(b) => serde_json::from_slice(&b).with_context(|| format!("cannot read {}", p.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(CalibrationFile::default()),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", p.display())),
    }
}

/// Writes the whole file through a temp file and a rename.
pub fn write(gear: &Path, f: &CalibrationFile) -> Result<()> {
    let p = file(gear);
    let dir = p.parent().expect("has a parent");
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(".calibrations.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(f)?)?;
    std::fs::rename(&tmp, &p).with_context(|| format!("cannot write {}", p.display()))
}

/// What the save changes besides the calibration.
pub struct SaveOptions<'a> {
    /// Remember this radio as the answer for its product name.
    pub remember: bool,
    /// A provisional key whose calibration this one replaces (linking it to a saved radio).
    pub replaces: Option<&'a str>,
}

/// Saves one radio's calibration.
pub fn save(gear: &Path, c: SavedCalibration, o: SaveOptions) -> Result<SavedCalibration> {
    let mut f = read(gear)?;
    if let Some(old) = o.replaces.filter(|k| *k != c.radio) {
        f.radios.remove(old);
        f.products.retain(|_, v| v != old);
    }
    if o.remember {
        if let Some(p) = &c.product {
            f.products.insert(p.clone(), c.radio.clone());
        }
    }
    f.radios.insert(c.radio.clone(), c.clone());
    write(gear, &f)?;
    Ok(c)
}

// ----- which radio -----

/// A saved radio the user can pick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct RadioChoice {
    pub id: String,
    pub name: String,
}

/// Which radio a joystick is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct RadioResolution {
    pub product: String,
    /// The radio's key: a saved radio's id, or a provisional key. None: ask, from
    /// `choices`.
    #[serde(default)]
    pub radio: Option<String>,
    #[serde(default)]
    pub provisional: bool,
    /// The saved radios this product name could be.
    pub choices: Vec<RadioChoice>,
    /// How it was worked out.
    pub how: String,
}

fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Whether a USB product name (`Radiomaster Pocket Joystick`) names a radio board
/// (`pocket`).
pub fn product_names_board(product: &str, board: &str) -> bool {
    let b = squash(board);
    !b.is_empty() && squash(product).contains(&b)
}

/// The provisional key of a radio never seen in storage mode.
pub fn provisional_key(product: &str, vid: u16, pid: u16) -> String {
    format!(
        "usb-{:016x}",
        xxhash_rust::xxh64::xxh64(format!("{product}:{vid:04x}:{pid:04x}").as_bytes(), 0)
    )
}

/// Works out which radio the joystick `product` is (design 7.5).
pub fn resolve(
    product: &str,
    vid: u16,
    pid: u16,
    devices: &[Device],
    f: &CalibrationFile,
) -> RadioResolution {
    let choices: Vec<RadioChoice> = devices
        .iter()
        .filter(|d| d.kind == DeviceKind::Radio)
        .filter(|d| {
            d.identity
                .board
                .as_deref()
                .is_some_and(|b| product_names_board(product, b))
        })
        .map(|d| RadioChoice {
            id: d.id.clone(),
            name: d.display_name(),
        })
        .collect();
    let mk = |radio: Option<String>, provisional: bool, how: &str| RadioResolution {
        product: product.to_string(),
        radio,
        provisional,
        choices: choices.clone(),
        how: how.to_string(),
    };
    if choices.len() == 1 {
        return mk(
            Some(choices[0].id.clone()),
            false,
            "The only saved radio of this model.",
        );
    }
    if let Some(id) = f.products.get(product) {
        if choices.iter().any(|c| &c.id == id) {
            return mk(
                Some(id.clone()),
                false,
                "The radio you picked for this model.",
            );
        }
    }
    if choices.len() > 1 {
        return mk(
            None,
            false,
            "Several saved radios are this model: which one is this?",
        );
    }
    mk(
        Some(provisional_key(product, vid, pid)),
        true,
        "No saved radio is this model yet. Link it to one once its card has been read.",
    )
}

// ----- defaults from what QuadCam knows -----

/// A control the sim will use, and where the suggestion came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SuggestedControl {
    pub control: RadioControl,
    /// `SA down`, `CH5`.
    pub label: String,
    /// `your radio model`, `the quad's modes`.
    pub source: String,
}

/// The stick channels the sim starts from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct StickMap {
    /// 1 is CH1.
    pub roll: u8,
    pub pitch: u8,
    pub throttle: u8,
    pub yaw: u8,
    /// `your radio model`, or `EdgeTX's default order (AETR)`.
    pub source: String,
    /// The model gave every stick's channel, so a quick check is enough.
    pub known: bool,
}

/// What the sim pre-fills for an aircraft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SimDefaults {
    #[serde(default)]
    pub aircraft: Option<String>,
    pub map: StickMap,
    #[serde(default)]
    pub arm: Option<SuggestedControl>,
    #[serde(default)]
    pub angle: Option<SuggestedControl>,
    #[serde(default)]
    pub horizon: Option<SuggestedControl>,
    /// Betaflight's FLIP OVER AFTER CRASH.
    #[serde(default)]
    pub turtle: Option<SuggestedControl>,
    #[serde(default)]
    pub airmode: Option<SuggestedControl>,
    /// A free button or trim for reset: no function in the model or on the quad.
    #[serde(default)]
    pub reset: Option<SuggestedControl>,
    pub notes: Vec<String>,
    /// The calibration these give, in Mode 2: where a new calibration starts.
    pub calibration: Calibration,
}

impl SimDefaults {
    /// The calibration to start from: the map, the arm switch and the reset control.
    pub fn calibration(&self, mode: u8) -> Calibration {
        let m = &self.map;
        let mut c = Calibration::with_map([m.roll, m.pitch, m.throttle, m.yaw]);
        c.mode = mode.clamp(1, 4);
        c.arm = self.arm.as_ref().map(|s| s.control);
        c.reset = self.reset.as_ref().map(|s| s.control);
        c.turtle = self.turtle.as_ref().map(|s| s.control);
        c.angle = self.angle.as_ref().map(|s| s.control);
        c.horizon = self.horizon.as_ref().map(|s| s.control);
        c.airmode = self.airmode.as_ref().map(|s| s.control);
        c
    }
}

const FROM_MODEL: &str = "your radio model";
const FROM_MODES: &str = "the quad's modes";

/// The sim's defaults from an aircraft's switch map. With no map, AETR and nothing else.
pub fn defaults(aircraft: Option<&str>, map: Option<&SwitchMap>) -> SimDefaults {
    let mut notes = Vec::new();
    let sticks = map.map(|m| m.sticks.as_slice()).unwrap_or_default();
    let ch = |k: Stick| {
        sticks
            .iter()
            .find(|s| s.stick == k)
            .and_then(|s| u8::try_from(s.ch).ok())
            .filter(|c| (1..=8).contains(c))
    };
    let known = Stick::ALL.iter().all(|k| ch(*k).is_some());
    let stick_map = if known {
        StickMap {
            roll: ch(Stick::Roll).unwrap_or(1),
            pitch: ch(Stick::Pitch).unwrap_or(2),
            throttle: ch(Stick::Throttle).unwrap_or(3),
            yaw: ch(Stick::Yaw).unwrap_or(4),
            source: FROM_MODEL.into(),
            known: true,
        }
    } else {
        if map.is_some_and(|m| m.model.is_some()) {
            notes.push("The radio model does not put every stick on CH1-8: the sticks start in EdgeTX's default order.".into());
        }
        StickMap {
            roll: 1,
            pitch: 2,
            throttle: 3,
            yaw: 4,
            source: "EdgeTX's default order (AETR)".into(),
            known: false,
        }
    };
    let mode = |name: &str| map.and_then(|m| mode_control(m, name));
    let arm = mode("ARM");
    let turtle = mode("FLIP OVER AFTER CRASH");
    let mut used: Vec<u8> = vec![
        stick_map.roll,
        stick_map.pitch,
        stick_map.throttle,
        stick_map.yaw,
    ];
    if let Some(m) = map {
        used.extend(m.modes.iter().filter_map(|x| u8::try_from(x.ch).ok()));
    }
    let reset = map.and_then(|m| free_control(m, &used));
    if map.is_some() && reset.is_none() {
        notes.push(
            "No free button or trim moves a channel the sim sees: setup asks you to press one."
                .into(),
        );
    }
    let mut d = SimDefaults {
        aircraft: aircraft.map(str::to_string),
        map: stick_map,
        arm,
        angle: mode("ANGLE"),
        horizon: mode("HORIZON"),
        turtle,
        airmode: mode("AIR MODE"),
        reset,
        notes,
        calibration: Calibration::default(),
    };
    d.calibration = d.calibration(2);
    d
}

/// The control that turns an `aux` mode on: the row whose position lists it, else the
/// channel alone.
fn mode_control(m: &SwitchMap, name: &str) -> Option<SuggestedControl> {
    let aux = m
        .modes
        .iter()
        .find(|x| x.name == name && x.linked.is_none())?;
    let control = RadioControl::from_range(aux.ch, aux.start, aux.end)?;
    let row = m.rows.iter().find_map(|r| {
        r.positions
            .iter()
            .find(|p| p.fc.iter().any(|f| f == name))
            .map(|p| (r, p))
    });
    Some(match row {
        Some((r, p)) => SuggestedControl {
            control,
            label: if r.positions.len() > 1 && r.kind == ControlKind::Switch {
                format!("{} {}", r.label, p.name)
            } else {
                r.label.clone()
            },
            source: FROM_MODEL.into(),
        },
        None => SuggestedControl {
            control,
            label: format!("CH{}", aux.ch),
            source: FROM_MODES.into(),
        },
    })
}

/// A control with no function: every position does nothing on the quad or the radio, and
/// one moves a channel the sim sees (CH1-32) that no stick or mode uses. Trims and
/// momentary switches first.
fn free_control(m: &SwitchMap, used: &[u8]) -> Option<SuggestedControl> {
    let rank = |r: &super::switchmap::ControlRow| match (r.kind, r.switch_type.as_deref()) {
        (ControlKind::Trim, _) => 0,
        (ControlKind::Switch, Some("TOGGLE")) => 1,
        (ControlKind::Switch, _) => 2,
        (ControlKind::Stick, _) => 9,
    };
    let mut rows: Vec<_> = m
        .rows
        .iter()
        .filter(|r| r.kind != ControlKind::Stick)
        .filter(|r| {
            r.positions
                .iter()
                .all(|p| p.fc.is_empty() && p.radio.is_empty())
        })
        .collect();
    rows.sort_by_key(|r| rank(r));
    rows.into_iter().find_map(|r| {
        let rest = r.positions.first()?;
        r.positions.iter().skip(1).find_map(|p| {
            p.channels.iter().find_map(|cv| {
                let ch = u8::try_from(cv.ch).ok().filter(|c| (1..=32).contains(c))?;
                if used.contains(&ch) {
                    return None;
                }
                let at_rest = rest
                    .channels
                    .iter()
                    .find(|x| x.ch == cv.ch)
                    .map(|x| x.us)
                    .unwrap_or(1500);
                let control = RadioControl::from_range(
                    cv.ch,
                    cv.us.saturating_sub(SWITCH_MARGIN_US),
                    cv.us + SWITCH_MARGIN_US,
                )?;
                let differs = match control {
                    RadioControl::Channel { min_us, max_us, .. } => {
                        !(min_us..=max_us).contains(&at_rest)
                    }
                    RadioControl::Button { pressed, .. } => (at_rest > 1500) != pressed,
                };
                differs.then(|| SuggestedControl {
                    control,
                    label: if r.kind == ControlKind::Switch && r.positions.len() > 2 {
                        format!("{} {}", r.label, p.name)
                    } else {
                        r.label.clone()
                    },
                    source: FROM_MODEL.into(),
                })
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gear::model::Identity;
    use crate::gear::switchmap::{AuxMode, ChannelValue, ControlRow, Position, StickChannel};

    fn radio(id: &str, name: &str, board: &str) -> Device {
        Device {
            id: id.into(),
            kind: DeviceKind::Radio,
            name: name.into(),
            aircraft: None,
            identity: Identity {
                board: Some(board.into()),
                ..Default::default()
            },
            last_seen: None,
            last_backup: None,
        }
    }

    fn saved(radio: &str) -> SavedCalibration {
        SavedCalibration {
            radio: radio.into(),
            provisional: false,
            calibration: Calibration::default(),
            product: Some("Test Pocket Joystick".into()),
            vid: 0x1209,
            pid: 0x4f54,
            firmware: Some("2.12".into()),
            saved_at: Utc::now(),
        }
    }

    #[test]
    fn calibrations_save_by_radio_id_and_the_resolve_step_asks_for_two() {
        let dir = tempfile::tempdir().unwrap();
        let p = "Test Pocket Joystick";
        let one = vec![
            radio("radio-aaaa", "Mine", "pocket"),
            radio("radio-tx", "Big", "tx16s"),
        ];
        let f = read(dir.path()).unwrap();
        let r = resolve(p, 0x1209, 0x4f54, &one, &f);
        assert_eq!(r.radio.as_deref(), Some("radio-aaaa"));
        assert!(!r.provisional);
        // Two radios of one model: the same product name (and the same USB serial).
        let two = vec![
            radio("radio-aaaa", "Mine", "pocket"),
            radio("radio-bbbb", "Spare", "pocket"),
        ];
        let r = resolve(p, 0x1209, 0x4f54, &two, &f);
        assert_eq!(r.radio, None, "asks");
        assert_eq!(r.choices.len(), 2);
        // The answer is remembered for the product name; each radio keeps its own.
        let mut b = saved("radio-bbbb");
        b.calibration.roll.deadzone = 7;
        save(
            dir.path(),
            b,
            SaveOptions {
                remember: true,
                replaces: None,
            },
        )
        .unwrap();
        save(
            dir.path(),
            saved("radio-aaaa"),
            SaveOptions {
                remember: false,
                replaces: None,
            },
        )
        .unwrap();
        let f = read(dir.path()).unwrap();
        assert_eq!(f.radios.len(), 2);
        assert_eq!(f.radios["radio-bbbb"].calibration.roll.deadzone, 7);
        assert_eq!(f.radios["radio-aaaa"].calibration.roll.deadzone, 0);
        let r = resolve(p, 0x1209, 0x4f54, &two, &f);
        assert_eq!(r.radio.as_deref(), Some("radio-bbbb"));
        let text = std::fs::read_to_string(file(dir.path())).unwrap();
        assert!(!text.contains("00000000001B"), "never keyed by serial");
    }

    #[test]
    fn an_unknown_radio_gets_a_provisional_key_it_can_link_later() {
        let dir = tempfile::tempdir().unwrap();
        let r = resolve(
            "Other Radio Joystick",
            0x1209,
            0x4f54,
            &[],
            &CalibrationFile::default(),
        );
        assert!(r.provisional);
        let key = r.radio.unwrap();
        assert!(key.starts_with("usb-"));
        assert_eq!(key, provisional_key("Other Radio Joystick", 0x1209, 0x4f54));
        let mut c = saved(&key);
        c.provisional = true;
        c.product = Some("Other Radio Joystick".into());
        save(
            dir.path(),
            c.clone(),
            SaveOptions {
                remember: false,
                replaces: None,
            },
        )
        .unwrap();
        // Linked: the calibration moves to the saved radio's id.
        c.radio = "radio-cccc".into();
        c.provisional = false;
        save(
            dir.path(),
            c,
            SaveOptions {
                remember: true,
                replaces: Some(&key),
            },
        )
        .unwrap();
        let f = read(dir.path()).unwrap();
        assert_eq!(f.radios.keys().collect::<Vec<_>>(), ["radio-cccc"]);
        assert_eq!(f.products["Other Radio Joystick"], "radio-cccc");
        assert!(product_names_board("RadioMaster TX16S Joystick", "tx16s"));
        assert!(!product_names_board("Radiomaster Pocket Joystick", ""));
    }

    fn pos(name: &str, ch: u32, us: u16, fc: &[&str]) -> Position {
        Position {
            name: name.into(),
            source: None,
            channels: vec![ChannelValue { ch, us }],
            fc: fc.iter().map(|s| s.to_string()).collect(),
            radio: vec![],
        }
    }

    fn row(id: &str, kind: ControlKind, ty: Option<&str>, positions: Vec<Position>) -> ControlRow {
        ControlRow {
            id: id.into(),
            label: id.into(),
            kind,
            switch_type: ty.map(str::to_string),
            positions,
        }
    }

    fn aux(name: &str, ch: u32, start: u16, end: u16) -> AuxMode {
        AuxMode {
            slot: 0,
            mode_id: 0,
            name: name.into(),
            ch,
            start,
            end,
            linked: None,
        }
    }

    /// A synthetic map: TAER sticks, arm on SA (CH5), angle on SB (CH6), turtle on a
    /// momentary SE that drives CH9, a free trim on CH10, a switch with a radio effect.
    fn map() -> SwitchMap {
        let sticks = [
            (Stick::Throttle, 1),
            (Stick::Roll, 2),
            (Stick::Pitch, 3),
            (Stick::Yaw, 4),
        ]
        .into_iter()
        .map(|(stick, ch)| StickChannel {
            stick,
            ch,
            weight: 100,
        })
        .collect();
        SwitchMap {
            model: Some("TEST".into()),
            sources: vec![],
            rows: vec![
                row(
                    "SA",
                    ControlKind::Switch,
                    Some("2POS"),
                    vec![pos("up", 5, 988, &[]), pos("down", 5, 2012, &["ARM"])],
                ),
                row(
                    "SB",
                    ControlKind::Switch,
                    Some("3POS"),
                    vec![
                        pos("up", 6, 988, &[]),
                        pos("mid", 6, 1500, &["ANGLE"]),
                        pos("down", 6, 2012, &["HORIZON"]),
                    ],
                ),
                row(
                    "SE",
                    ControlKind::Switch,
                    Some("TOGGLE"),
                    vec![
                        pos("up", 9, 988, &[]),
                        pos("down", 9, 2012, &["FLIP OVER AFTER CRASH"]),
                    ],
                ),
                row(
                    "SD",
                    ControlKind::Switch,
                    Some("2POS"),
                    vec![pos("up", 11, 988, &[]), {
                        let mut p = pos("down", 11, 2012, &[]);
                        p.radio = vec!["Plays \"armed\"".into()];
                        p
                    }],
                ),
                row(
                    "TrimRudLeft",
                    ControlKind::Trim,
                    None,
                    vec![pos("released", 10, 988, &[]), pos("pressed", 10, 2012, &[])],
                ),
            ],
            sticks,
            modes: vec![
                aux("ARM", 5, 1700, 2100),
                aux("ANGLE", 6, 1300, 1700),
                aux("HORIZON", 6, 1700, 2100),
                aux("FLIP OVER AFTER CRASH", 9, 1700, 2100),
            ],
            adjustments: vec![],
            conflicts: vec![],
            notes: vec![],
            live: None,
        }
    }

    #[test]
    fn defaults_come_from_the_model_and_the_quads_modes() {
        let d = defaults(Some("Whoop"), Some(&map()));
        assert_eq!(
            (d.map.roll, d.map.pitch, d.map.throttle, d.map.yaw),
            (2, 3, 1, 4)
        );
        assert!(d.map.known);
        assert_eq!(d.map.source, "your radio model");
        let arm = d.arm.clone().unwrap();
        assert_eq!(arm.label, "SA down");
        assert_eq!(
            arm.control,
            RadioControl::Channel {
                ch: 5,
                min_us: 1700,
                max_us: 2100
            }
        );
        assert_eq!(d.angle.clone().unwrap().label, "SB mid");
        assert_eq!(d.horizon.clone().unwrap().label, "SB down");
        let turtle = d.turtle.clone().unwrap();
        assert_eq!(turtle.label, "SE down");
        assert_eq!(
            turtle.control,
            RadioControl::Button {
                button: 1,
                pressed: true
            }
        );
        assert!(d.airmode.is_none());
        let reset = d.reset.clone().unwrap();
        assert_eq!(
            reset.label, "TrimRudLeft",
            "the free trim; SD plays a sound, so it is not free"
        );
        assert_eq!(
            reset.control,
            RadioControl::Button {
                button: 2,
                pressed: true
            }
        );
        let c = d.calibration(2);
        assert_eq!(c.map(), [2, 3, 1, 4]);
        assert_eq!(c.arm, Some(arm.control));
        assert_eq!(c.reset, Some(reset.control));
        // No map: AETR, nothing suggested.
        let none = defaults(None, None);
        assert_eq!((none.map.roll, none.map.throttle), (1, 3));
        assert!(!none.map.known && none.arm.is_none() && none.reset.is_none());
        // A mode on a channel no row reaches: the channel, from the quad's modes.
        let mut m = map();
        m.rows.remove(0);
        let d = defaults(None, Some(&m));
        assert_eq!(d.arm.unwrap().label, "CH5");
    }
}
