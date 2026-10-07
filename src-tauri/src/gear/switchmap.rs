//! The switch map (design 7.2): one table per aircraft that joins an EdgeTX model with the
//! FC's `aux` and `adjrange` lines. A row is a physical control (a switch, a trim used as a
//! switch, a stick a logical switch reads); each of its positions lists the channel values
//! it sends, the FC modes and adjustments they select, and the radio's own effects
//! (logical switches, special functions, timers).
//!
//! How a position is worked out: every channel's mixes are evaluated with that control in
//! that position and everything else at rest (other switches in position 0, sticks
//! centred, throttle low, trims released). Mix switch conditions, `ADD`, `MUL` and `REPL`
//! lines, inputs, limits and logical switches all take part, in file order. What differs
//! between a control's positions is what the control does.
//!
//! - Channel values: `us = 1500 + percent * 5.12` (−100 % is 988 µs, +100 % 2012 µs).
//!   `AUXn` is `CH(n+4)`; Betaflight's `aux` index 0 is AUX1.
//! - `adjrange` select positions: `(us − 900) / 400` for a 3-position select.
//! - A logical switch that reads telemetry or time is "unknown" and lights nothing.
//! - Live: `live` matches channel values (from the FC's `MSP_RC` or the radio's USB
//!   joystick) to each control's positions and lists the active modes.
//!
//! Written from the EdgeTX file format and Betaflight's documented CLI, not from either
//! project's source.

use super::bf::dump::{Cmd, Config};
use super::edgetx::model::{ls_index, ModelView};
use super::edgetx::yaml::{unquote, Doc};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::{BTreeMap, BTreeSet};

/// Channel values within this many µs of a position's value count as that position.
pub const LIVE_TOLERANCE_US: u16 = 60;

/// One physical control and what each of its positions does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ControlRow {
    /// `SA`, `TrimThrDown`, `stick:Thr`.
    pub id: String,
    /// `SA`, `Throttle trim down`, `Throttle stick`.
    pub label: String,
    pub kind: ControlKind,
    /// `2POS`, `3POS` or `TOGGLE` for a switch.
    #[serde(default)]
    pub switch_type: Option<String>,
    pub positions: Vec<Position>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ControlKind {
    Switch,
    Trim,
    Stick,
}

/// One position of a control.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Position {
    /// `up`, `mid`, `down`, `pressed`, `low`...
    pub name: String,
    /// The EdgeTX source for it (`SA0`), when it has one.
    #[serde(default)]
    pub source: Option<String>,
    /// The channels the control moves, with their value here.
    pub channels: Vec<ChannelValue>,
    /// FC modes on and adjustments selected here (`ARM`, `Rate profile 2`).
    pub fc: Vec<String>,
    /// The radio's own effects here (`L1 on`, `Plays "armed"`, `Timer 1 (TOT) runs`).
    pub radio: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ChannelValue {
    /// 1 is CH1.
    pub ch: u32,
    pub us: u16,
}

/// A Betaflight `aux` line in use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct AuxMode {
    pub slot: u32,
    pub mode_id: u32,
    /// `ARM`, `ANGLE`; `mode 99` for an id QuadCam does not know.
    pub name: String,
    /// 1 is CH1 (AUX1 is CH5).
    pub ch: u32,
    pub start: u16,
    pub end: u16,
    /// The mode this one follows instead of a range (`linkedTo`).
    #[serde(default)]
    pub linked: Option<String>,
}

/// A Betaflight `adjrange` line in use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Adjustment {
    pub slot: u32,
    pub function_id: u32,
    /// `Rate profile`, `OSD profile`, `Roll rate`.
    pub name: String,
    /// The channel whose range turns it on, and the range.
    pub range_ch: u32,
    pub start: u16,
    pub end: u16,
    /// The channel that selects or adjusts.
    pub select_ch: u32,
    /// A 3-position select (rate, OSD and LED profile) rather than a step adjustment.
    pub select: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Stick {
    Roll,
    Pitch,
    Throttle,
    Yaw,
}

impl Stick {
    pub const ALL: [Stick; 4] = [Stick::Roll, Stick::Pitch, Stick::Throttle, Stick::Yaw];

    /// The EdgeTX stick source.
    pub fn source(self) -> &'static str {
        match self {
            Stick::Roll => "Ail",
            Stick::Pitch => "Ele",
            Stick::Throttle => "Thr",
            Stick::Yaw => "Rud",
        }
    }

    fn from_source(s: &str) -> Option<Stick> {
        Stick::ALL.into_iter().find(|k| k.source() == s)
    }

    fn label(self) -> &'static str {
        match self {
            Stick::Roll => "Roll",
            Stick::Pitch => "Pitch",
            Stick::Throttle => "Throttle",
            Stick::Yaw => "Yaw",
        }
    }
}

/// The channel a stick drives, and the weight its mix gives it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct StickChannel {
    pub stick: Stick,
    /// 1 is CH1.
    pub ch: u32,
    /// Percent; negative when reversed.
    pub weight: i32,
}

/// Channel values matched to the map: where each control is, and what the FC has on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Live {
    /// `fc` (`MSP_RC`) or `radio` (the USB joystick).
    pub source: String,
    /// µs, CH1 first.
    pub channels: Vec<u16>,
    /// Each row's position index, by row id; None when no position matches.
    pub positions: BTreeMap<String, Option<u32>>,
    /// Modes whose range holds their channel's value.
    pub modes: Vec<String>,
    /// Selections the adjustments make (`Rate profile 2`).
    pub adjustments: Vec<String>,
}

/// The switch map of one aircraft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SwitchMap {
    /// The model's header name.
    #[serde(default)]
    pub model: Option<String>,
    /// What was read: file names, with the backup date once backups exist.
    pub sources: Vec<String>,
    pub rows: Vec<ControlRow>,
    pub sticks: Vec<StickChannel>,
    pub modes: Vec<AuxMode>,
    pub adjustments: Vec<Adjustment>,
    /// Two modes on one range, a control with no effect, a mode no control reaches, a
    /// sound file the card does not have.
    pub conflicts: Vec<String>,
    pub notes: Vec<String>,
    #[serde(default)]
    pub live: Option<Live>,
}

// ----- inputs -----

/// One line of the model's `expoData` (an input).
#[derive(Debug, Clone, PartialEq)]
pub struct Expo {
    /// 0 is I0.
    pub chn: u32,
    pub source: String,
    pub weight: f64,
    pub swtch: String,
}

/// A channel's limits (`limitData`), in percent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limit {
    pub min: f64,
    pub max: f64,
    pub offset: f64,
    pub revert: bool,
}

impl Default for Limit {
    fn default() -> Self {
        Self {
            min: -100.0,
            max: 100.0,
            offset: 0.0,
            revert: false,
        }
    }
}

/// What the map is built from.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    pub model: Option<ModelView>,
    pub expos: Vec<Expo>,
    pub limits: BTreeMap<u32, Limit>,
    /// `(SA, 2POS)` from `radio.yml`; empty when no radio file was read.
    pub switches: Vec<(String, String)>,
    /// The FC's dump, diff or CLI lines.
    pub fc: Option<Config>,
    /// The track names in `SOUNDS/<lang>/` (lower case, no extension), when a card was read.
    pub sounds: Option<BTreeSet<String>>,
    pub sources: Vec<String>,
}

/// The model's inputs (`expoData`), in file order.
pub fn expos(doc: &Doc) -> Vec<Expo> {
    let Ok(Some(n)) = doc.top("expoData") else {
        return Vec::new();
    };
    n.children
        .iter()
        .filter(|c| c.key.is_none())
        .map(|c| Expo {
            chn: c.get("chn").and_then(|v| v.parse().ok()).unwrap_or(0),
            source: unquote(c.get("srcRaw").unwrap_or("")).to_string(),
            weight: c
                .get("weight")
                .and_then(|v| v.parse().ok())
                .unwrap_or(100.0),
            swtch: unquote(c.get("swtch").unwrap_or("NONE")).to_string(),
        })
        .collect()
}

/// The model's channel limits (`limitData`). `min` and `max` count 0.1 % from ±100 %.
pub fn limits(doc: &Doc) -> BTreeMap<u32, Limit> {
    let mut out = BTreeMap::new();
    let Ok(Some(n)) = doc.top("limitData") else {
        return out;
    };
    for c in &n.children {
        let Some(i) = c.index() else { continue };
        let num = |k: &str| c.get(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        out.insert(
            i,
            Limit {
                min: -100.0 + num("min") / 10.0,
                max: 100.0 + num("max") / 10.0,
                offset: num("offset") / 10.0,
                revert: c.get("revert") == Some("1"),
            },
        );
    }
    out
}

/// The radio's switches and their types (`switchConfig` in `radio.yml`), `NONE` left out.
pub fn switch_types(radio: &Doc) -> Vec<(String, String)> {
    let Ok(Some(n)) = radio.top("switchConfig") else {
        return Vec::new();
    };
    n.children
        .iter()
        .filter_map(|c| {
            let name = c.key.clone()?;
            let t = c.get("type").unwrap_or("3POS").to_string();
            (t != "NONE").then_some((name, t))
        })
        .collect()
}

/// The radio's TTS language (`ttsLanguage`), for the sound folder.
pub fn tts_language(radio: &Doc) -> String {
    radio
        .top_value("ttsLanguage")
        .map(|v| unquote(&v).to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "en".into())
}

// ----- Betaflight -----

/// Betaflight's mode ids (the `aux` line's second number) and their names, from the
/// Configurator's Modes tab and the CLI's `aux` documentation.
pub const MODES: &[(u32, &str)] = &[
    (0, "ARM"),
    (1, "ANGLE"),
    (2, "HORIZON"),
    (3, "ALTHOLD"),
    (4, "ANTI GRAVITY"),
    (5, "MAG"),
    (6, "HEADFREE"),
    (7, "HEADADJ"),
    (8, "CAMSTAB"),
    (12, "PASSTHRU"),
    (13, "BEEPER"),
    (15, "LEDLOW"),
    (17, "CALIB"),
    (19, "OSD DISABLE"),
    (20, "TELEMETRY"),
    (23, "SERVO1"),
    (24, "SERVO2"),
    (25, "SERVO3"),
    (26, "BLACKBOX"),
    (27, "FAILSAFE"),
    (28, "AIR MODE"),
    (29, "3D"),
    (30, "FPV ANGLE MIX"),
    (31, "BLACKBOX ERASE"),
    (32, "CAMERA CONTROL 1"),
    (33, "CAMERA CONTROL 2"),
    (34, "CAMERA CONTROL 3"),
    (35, "FLIP OVER AFTER CRASH"),
    (36, "PREARM"),
    (37, "GPS BEEP SATELLITE COUNT"),
    (39, "VTX PIT MODE"),
    (40, "USER1"),
    (41, "USER2"),
    (42, "USER3"),
    (43, "USER4"),
    (44, "PID AUDIO"),
    (45, "PARALYZE"),
    (46, "GPS RESCUE"),
    (47, "ACRO TRAINER"),
    (48, "VTX CONTROL DISABLE"),
    (49, "LAUNCH CONTROL"),
    (50, "MSP OVERRIDE"),
    (51, "STICK COMMANDS DISABLE"),
    (52, "BEEPER MUTE"),
];

/// Betaflight's adjustment functions (the `adjrange` line's sixth number).
pub const ADJUSTMENTS: &[(u32, &str)] = &[
    (0, "None"),
    (1, "RC rate"),
    (2, "RC expo"),
    (3, "Throttle expo"),
    (4, "Pitch and roll rate"),
    (5, "Yaw rate"),
    (6, "Pitch and roll P"),
    (7, "Pitch and roll I"),
    (8, "Pitch and roll D"),
    (9, "Yaw P"),
    (10, "Yaw I"),
    (11, "Yaw D"),
    (12, "Rate profile"),
    (13, "Pitch rate"),
    (14, "Roll rate"),
    (15, "Pitch P"),
    (16, "Pitch I"),
    (17, "Pitch D"),
    (18, "Roll P"),
    (19, "Roll I"),
    (20, "Roll D"),
    (21, "Yaw RC rate"),
    (22, "Pitch and roll F"),
    (23, "Feedforward transition"),
    (24, "Horizon strength"),
    (25, "Roll RC rate"),
    (26, "Pitch RC rate"),
    (27, "Roll RC expo"),
    (28, "Pitch RC expo"),
    (29, "PID audio"),
    (30, "Pitch F"),
    (31, "Roll F"),
    (32, "Yaw F"),
    (33, "OSD profile"),
    (34, "LED profile"),
    (35, "LED dimmer"),
];

/// Adjustments that pick one of three profiles rather than step a value.
const SELECTS: &[u32] = &[12, 33, 34];

pub fn mode_name(id: u32) -> String {
    MODES
        .iter()
        .find(|(i, _)| *i == id)
        .map(|(_, n)| n.to_string())
        .unwrap_or_else(|| format!("mode {id}"))
}

fn adjustment_name(id: u32) -> String {
    ADJUSTMENTS
        .iter()
        .find(|(i, _)| *i == id)
        .map(|(_, n)| n.to_string())
        .unwrap_or_else(|| format!("adjustment {id}"))
}

/// The last line of each `verb N` key, in key order: a diff after a dump wins.
fn last_lines<'a>(c: &'a Config, verb: &str) -> BTreeMap<u32, Vec<&'a str>> {
    let mut out = BTreeMap::new();
    for l in &c.lines {
        if let Some(Cmd::Other { verb: v, .. }) = &l.cmd {
            if v == verb {
                let w: Vec<&str> = l.text.split_whitespace().collect();
                if let Some(i) = w.get(1).and_then(|x| x.parse().ok()) {
                    out.insert(i, w);
                }
            }
        }
    }
    out
}

/// The `aux` lines in use (a range wider than nothing, or a link).
pub fn aux_modes(c: &Config) -> Vec<AuxMode> {
    last_lines(c, "aux")
        .into_values()
        .filter_map(|w| {
            let n = |i: usize| w.get(i).and_then(|x| x.parse::<u32>().ok());
            let (slot, mode_id, aux, start, end) = (n(1)?, n(2)?, n(3)?, n(4)?, n(5)?);
            let linked = n(7).filter(|l| *l != 0).map(mode_name);
            (start < end || linked.is_some()).then(|| AuxMode {
                slot,
                mode_id,
                name: mode_name(mode_id),
                ch: aux + 5,
                start: start as u16,
                end: end as u16,
                linked,
            })
        })
        .collect()
}

/// The `adjrange` lines in use (a function other than None and a range).
pub fn adjustments(c: &Config) -> Vec<Adjustment> {
    last_lines(c, "adjrange")
        .into_values()
        .filter_map(|w| {
            let n = |i: usize| w.get(i).and_then(|x| x.parse::<u32>().ok());
            let (slot, range_aux, start, end, function_id, select_aux) =
                (n(1)?, n(3)?, n(4)?, n(5)?, n(6)?, n(7)?);
            (function_id != 0 && start < end).then(|| Adjustment {
                slot,
                function_id,
                name: adjustment_name(function_id),
                range_ch: range_aux + 5,
                start: start as u16,
                end: end as u16,
                select_ch: select_aux + 5,
                select: SELECTS.contains(&function_id),
            })
        })
        .collect()
}

/// Percent to µs.
pub fn us(pct: f64) -> u16 {
    (1500.0 + pct * 5.12).round().clamp(0.0, 3000.0) as u16
}

/// A 3-position select's position (0, 1 or 2) for a channel value.
pub fn select_position(us: u16) -> u32 {
    ((us.saturating_sub(900)) / 400).min(2) as u32
}

fn in_range(v: u16, start: u16, end: u16) -> bool {
    // Betaflight's top step is 2100, and a range ending there takes everything above.
    v >= start && (v < end || end >= 2100)
}

// ----- evaluation -----

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tri {
    On,
    Off,
    Unknown,
}

impl Tri {
    fn from(b: bool) -> Tri {
        if b {
            Tri::On
        } else {
            Tri::Off
        }
    }
    fn not(self) -> Tri {
        match self {
            Tri::On => Tri::Off,
            Tri::Off => Tri::On,
            Tri::Unknown => Tri::Unknown,
        }
    }
    fn and(self, o: Tri) -> Tri {
        match (self, o) {
            (Tri::Off, _) | (_, Tri::Off) => Tri::Off,
            (Tri::On, Tri::On) => Tri::On,
            _ => Tri::Unknown,
        }
    }
    fn or(self, o: Tri) -> Tri {
        self.not().and(o.not()).not()
    }
}

/// Where every control is.
#[derive(Debug, Clone, Default)]
struct State {
    switches: BTreeMap<String, u32>,
    trims: BTreeSet<String>,
    sticks: BTreeMap<Stick, f64>,
}

impl State {
    fn rest() -> State {
        let mut s = State::default();
        s.sticks.insert(Stick::Throttle, -100.0);
        s
    }
}

/// What the radio computes in one state.
#[derive(Debug, Clone, PartialEq)]
struct Outcome {
    /// Percent after limits, by channel index (0 is CH1).
    channels: BTreeMap<u32, f64>,
    ls: Vec<Tri>,
}

struct Eval<'a> {
    inputs: &'a Inputs,
    model: &'a ModelView,
    /// Switch types by name.
    types: BTreeMap<String, String>,
}

/// A switch source `SA0`..`SH2` split into name and position.
fn switch_pos(s: &str) -> Option<(String, u32)> {
    let b = s.as_bytes();
    (b.len() == 3 && b[0] == b'S' && b[1].is_ascii_uppercase())
        .then(|| (s[..2].to_string(), (b[2] as char).to_digit(10)))
        .and_then(|(n, p)| Some((n, p.filter(|p| *p <= 2)?)))
}

/// A switch name used as a mix source (`SA`).
fn is_switch_name(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 2 && b[0] == b'S' && b[1].is_ascii_uppercase()
}

fn is_trim(s: &str) -> bool {
    s.starts_with("Trim")
        && (s.ends_with("Up") || s.ends_with("Down") || s.ends_with("Left") || s.ends_with("Right"))
}

impl Eval<'_> {
    fn cond(&self, c: &str, st: &State, ls: &[Tri]) -> Tri {
        let c = c.trim();
        if let Some(rest) = c.strip_prefix('!') {
            return self.cond(rest, st, ls).not();
        }
        match c {
            "" | "NONE" | "ON" => return Tri::On,
            "OFF" => return Tri::Off,
            _ => {}
        }
        if let Some((name, pos)) = switch_pos(c) {
            return Tri::from(st.switches.get(&name).copied().unwrap_or(0) == pos);
        }
        if let Some(i) = ls_index(c) {
            return ls.get(i as usize).copied().unwrap_or(Tri::Off);
        }
        if is_trim(c) {
            return Tri::from(st.trims.contains(c));
        }
        Tri::Unknown
    }

    fn switch_value(&self, name: &str, st: &State) -> f64 {
        match st.switches.get(name).copied().unwrap_or(0) {
            0 => -100.0,
            1 => 0.0,
            _ => 100.0,
        }
    }

    /// A source's value in percent; None when it depends on something the map cannot know.
    fn source(&self, s: &str, st: &State, ls: &[Tri], ch: &BTreeMap<u32, f64>) -> Option<f64> {
        let s = unquote(s.trim());
        let (neg, s) = match s.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, s),
        };
        let v = if let Some(k) = Stick::from_source(s) {
            st.sticks.get(&k).copied().unwrap_or(0.0)
        } else if is_switch_name(s) {
            self.switch_value(s, st)
        } else if s == "MAX" {
            100.0
        } else if let Some(i) = s.strip_prefix('I').and_then(|n| n.parse::<u32>().ok()) {
            self.input(i, st, ls, ch)?
        } else if let Some(n) = s
            .strip_prefix("ch(")
            .and_then(|r| r.strip_suffix(')'))
            .and_then(|n| n.parse::<u32>().ok())
        {
            ch.get(&n).copied().unwrap_or(0.0)
        } else if let Some(i) = ls_index(s) {
            match ls.get(i as usize).copied().unwrap_or(Tri::Off) {
                Tri::On => 100.0,
                Tri::Off => -100.0,
                Tri::Unknown => return None,
            }
        } else if s.starts_with("Trim")
            || s.starts_with('P')
            || s.starts_with("S1")
            || s.starts_with("S2")
        {
            0.0
        } else {
            return None;
        };
        Some(if neg { -v } else { v })
    }

    /// An input's value: its first line whose switch is on.
    fn input(&self, i: u32, st: &State, ls: &[Tri], ch: &BTreeMap<u32, f64>) -> Option<f64> {
        let e = self
            .inputs
            .expos
            .iter()
            .filter(|e| e.chn == i)
            .find(|e| self.cond(&e.swtch, st, ls) == Tri::On)?;
        Some(self.source(&e.source, st, ls, ch)? * e.weight / 100.0)
    }

    fn logical(&self, st: &State, ls: &[Tri], ch: &BTreeMap<u32, f64>) -> Vec<Tri> {
        let n = self
            .model
            .logical_switches
            .iter()
            .map(|l| l.index + 1)
            .max()
            .unwrap_or(0) as usize;
        let mut out = vec![Tri::Off; n];
        for l in &self.model.logical_switches {
            let a: Vec<&str> = l.def.split(',').map(str::trim).collect();
            let num = |i: usize| a.get(i).and_then(|x| x.parse::<f64>().ok());
            let src = |i: usize| a.get(i).and_then(|x| self.source(x, st, ls, ch));
            let cmp = |f: fn(f64, f64) -> bool| match (src(0), num(1)) {
                (Some(x), Some(y)) => Tri::from(f(x, y)),
                _ => Tri::Unknown,
            };
            let cmp2 = |f: fn(f64, f64) -> bool| match (src(0), src(1)) {
                (Some(x), Some(y)) => Tri::from(f(x, y)),
                _ => Tri::Unknown,
            };
            let sw = |i: usize| self.cond(a.get(i).copied().unwrap_or("NONE"), st, ls);
            let v = match l.func.as_str() {
                "FUNC_VPOS" => cmp(|x, y| x > y),
                "FUNC_VNEG" => cmp(|x, y| x < y),
                "FUNC_APOS" => cmp(|x, y| x.abs() > y),
                "FUNC_ANEG" => cmp(|x, y| x.abs() < y),
                "FUNC_VEQUAL" => cmp(|x, y| x == y),
                "FUNC_VALMOSTEQUAL" => cmp(|x, y| (x - y).abs() < 1.0),
                "FUNC_GREATER" => cmp2(|x, y| x > y),
                "FUNC_LESS" => cmp2(|x, y| x < y),
                "FUNC_EQUAL" => cmp2(|x, y| x == y),
                "FUNC_AND" => sw(0).and(sw(1)),
                "FUNC_OR" => sw(0).or(sw(1)),
                "FUNC_XOR" => match (sw(0), sw(1)) {
                    (Tri::Unknown, _) | (_, Tri::Unknown) => Tri::Unknown,
                    (x, y) => Tri::from(x != y),
                },
                // An edge pulses when its switch turns on; the map shows it with the switch.
                "FUNC_EDGE" => sw(0),
                _ => Tri::Unknown,
            };
            let and = if l.andsw.is_empty() || l.andsw == "NONE" {
                Tri::On
            } else {
                self.cond(&l.andsw, st, ls)
            };
            out[l.index as usize] = v.and(and);
        }
        out
    }

    fn channels(&self, st: &State, ls: &[Tri], prev: &BTreeMap<u32, f64>) -> BTreeMap<u32, f64> {
        let mut acc: BTreeMap<u32, f64> = BTreeMap::new();
        for m in &self.model.mixes {
            if self.cond(&m.swtch, st, ls) != Tri::On {
                continue;
            }
            let Some(src) = self.source(&m.source, st, ls, prev) else {
                continue;
            };
            let weight = m.weight.trim().parse::<f64>().unwrap_or(100.0);
            let offset = m
                .fields
                .iter()
                .find(|f| f.key == "offset")
                .and_then(|f| f.value.parse::<f64>().ok())
                .unwrap_or(0.0);
            let v = src * weight / 100.0 + offset;
            let a = acc.entry(m.dest_ch).or_insert(0.0);
            match m.mltpx.as_str() {
                "REPL" => *a = v,
                "MUL" => *a = *a * v / 100.0,
                _ => *a += v,
            }
        }
        let mut out = BTreeMap::new();
        for (ch, v) in acc {
            let lim = self.inputs.limits.get(&ch).copied().unwrap_or_default();
            let v = v.clamp(-100.0, 100.0);
            let v = if lim.revert { -v } else { v };
            out.insert(ch, (v + lim.offset).clamp(lim.min, lim.max));
        }
        out
    }

    /// Channels and logical switches settle within a few passes (a mix may read an LS that
    /// reads a channel).
    fn run(&self, st: &State) -> Outcome {
        let mut ch = BTreeMap::new();
        let mut ls = Vec::new();
        for _ in 0..4 {
            ls = self.logical(st, &ls, &ch);
            let next = self.channels(st, &ls, &ch);
            if next == ch {
                break;
            }
            ch = next;
        }
        Outcome { channels: ch, ls }
    }

    fn positions_of(&self, name: &str) -> Vec<(String, u32)> {
        match self.types.get(name).map(String::as_str) {
            Some("2POS") => vec![("up".into(), 0), ("down".into(), 2)],
            Some("TOGGLE") => vec![("released".into(), 0), ("pressed".into(), 2)],
            _ => vec![("up".into(), 0), ("mid".into(), 1), ("down".into(), 2)],
        }
    }
}

// ----- describing effects -----

fn sf_text(model: &ModelView, func: &str, def: &str) -> String {
    let a: Vec<&str> = def.split(',').collect();
    let first = a.first().copied().unwrap_or("");
    let value = |s: &str| -> String {
        if let Some(n) = s
            .strip_prefix("tele(")
            .and_then(|r| r.strip_suffix(')'))
            .and_then(|n| n.parse::<u32>().ok())
        {
            return model
                .sensors
                .iter()
                .find(|x| x.slot == n)
                .map(|x| x.label.clone())
                .unwrap_or_else(|| s.to_string());
        }
        if let Some(n) = s.strip_prefix("Tmr") {
            return format!("Timer {n}");
        }
        s.to_string()
    };
    match func {
        "PLAY_TRACK" => format!("Plays \"{first}\""),
        "PLAY_VALUE" => format!("Reads {}", value(first)),
        "PLAY_SOUND" => format!("Sound {first}"),
        "HAPTIC" => "Vibrates".into(),
        "RESET" => format!("Resets {}", value(first)),
        "SET_SCREEN" => format!("Screen {first}"),
        "LOGS" => "Logs".into(),
        "VOLUME" => format!("Volume from {first}"),
        "BACKLIGHT" => format!("Backlight from {first}"),
        "SCREENSHOT" => "Screenshot".into(),
        "VARIO" => "Vario".into(),
        _ => format!("{func} {def}").trim().to_string(),
    }
}

/// The names of the switches, trims and sticks a model refers to, in order of first use.
fn referenced(model: &ModelView) -> (Vec<String>, Vec<String>, Vec<Stick>) {
    let mut switches = Vec::new();
    let mut trims = Vec::new();
    let mut sticks = Vec::new();
    let add_cond = |c: &str, switches: &mut Vec<String>, trims: &mut Vec<String>| {
        let c = unquote(c.trim()).trim_start_matches('!');
        if let Some((n, _)) = switch_pos(c) {
            if !switches.contains(&n) {
                switches.push(n);
            }
        } else if is_trim(c) && !trims.iter().any(|t| t == c) {
            trims.push(c.to_string());
        }
    };
    for m in &model.mixes {
        let s = m.source.trim_start_matches('-');
        if is_switch_name(s) && !switches.iter().any(|x| x == s) {
            switches.push(s.to_string());
        }
        add_cond(&m.swtch, &mut switches, &mut trims);
    }
    for l in &model.logical_switches {
        for (i, part) in l.def.split(',').enumerate() {
            if matches!(
                l.func.as_str(),
                "FUNC_AND" | "FUNC_OR" | "FUNC_XOR" | "FUNC_EDGE" | "FUNC_STICKY"
            ) && i < 2
            {
                add_cond(part, &mut switches, &mut trims);
            } else if i == 0 {
                if let Some(k) = Stick::from_source(part.trim()) {
                    if !sticks.contains(&k) {
                        sticks.push(k);
                    }
                }
            }
        }
        add_cond(&l.andsw, &mut switches, &mut trims);
    }
    for f in &model.special_functions {
        add_cond(&f.swtch, &mut switches, &mut trims);
    }
    for t in &model.timers {
        add_cond(&t.swtch, &mut switches, &mut trims);
    }
    (switches, trims, sticks)
}

fn trim_label(t: &str) -> String {
    let rest = t.trim_start_matches("Trim");
    let (axis, dir) = ["Up", "Down", "Left", "Right"]
        .iter()
        .find_map(|d| rest.strip_suffix(d).map(|a| (a, *d)))
        .unwrap_or((rest, ""));
    let axis = match axis {
        "Thr" => "Throttle",
        "Rud" => "Yaw",
        "Ele" => "Pitch",
        "Ail" => "Roll",
        a => a,
    };
    format!("{axis} trim {}", dir.to_lowercase())
}

/// A row before it is worked out: the control and the state at each position.
struct RowSpec {
    id: String,
    label: String,
    kind: ControlKind,
    switch_type: Option<String>,
    states: Vec<RowState>,
}

struct RowState {
    name: String,
    source: Option<String>,
    st: State,
}

// ----- the map -----

/// Builds the switch map. With no model, the rows are empty and only the FC side shows.
pub fn build(inputs: &Inputs) -> SwitchMap {
    let empty = ModelView {
        file: String::new(),
        name: String::new(),
        model_ids: vec![],
        timers: vec![],
        mixes: vec![],
        logical_switches: vec![],
        special_functions: vec![],
        switch_warnings: vec![],
        legacy_switch_warning: false,
        sensors: vec![],
        screens: vec![],
        checklist: false,
        checklist_interactive: false,
    };
    let model = inputs.model.as_ref().unwrap_or(&empty);
    let ev = Eval {
        inputs,
        model,
        types: inputs.switches.iter().cloned().collect(),
    };
    let modes = inputs.fc.as_ref().map(aux_modes).unwrap_or_default();
    let adjustments = inputs.fc.as_ref().map(adjustments).unwrap_or_default();
    let mut notes = Vec::new();
    let mut conflicts = Vec::new();
    if inputs.model.is_none() {
        notes.push("No EdgeTX model read: the rows need one.".into());
    }
    if inputs.fc.is_none() {
        notes.push("No Betaflight dump read: the FC column is empty.".into());
    }
    if inputs.model.is_some() && inputs.switches.is_empty() {
        notes.push("No radio.yml read: switches the model uses are taken as 3-position.".into());
    }

    let (used, trims, stick_ls) = referenced(model);
    let mut switch_names: Vec<String> = inputs.switches.iter().map(|(n, _)| n.clone()).collect();
    if inputs.model.is_some() {
        for s in used {
            if !switch_names.contains(&s) {
                switch_names.push(s);
            }
        }
    }
    if inputs.model.is_none() {
        switch_names.clear();
    }

    let rest = State::rest();
    let base = ev.run(&rest);

    // (row id, label, kind, type, positions)
    let mut specs: Vec<RowSpec> = Vec::new();
    for n in &switch_names {
        let positions = ev
            .positions_of(n)
            .into_iter()
            .map(|(name, p)| {
                let mut st = rest.clone();
                st.switches.insert(n.clone(), p);
                RowState {
                    name,
                    source: Some(format!("{n}{p}")),
                    st,
                }
            })
            .collect();
        specs.push(RowSpec {
            id: n.clone(),
            label: n.clone(),
            kind: ControlKind::Switch,
            switch_type: Some(ev.types.get(n).cloned().unwrap_or_else(|| "3POS".into())),
            states: positions,
        });
    }
    for t in &trims {
        let positions = [("released", false), ("pressed", true)]
            .into_iter()
            .map(|(name, on)| {
                let mut st = rest.clone();
                if on {
                    st.trims.insert(t.clone());
                }
                RowState {
                    name: name.into(),
                    source: on.then(|| t.clone()),
                    st,
                }
            })
            .collect();
        specs.push(RowSpec {
            id: t.clone(),
            label: trim_label(t),
            kind: ControlKind::Trim,
            switch_type: None,
            states: positions,
        });
    }
    for k in &stick_ls {
        let positions = [("low", -100.0), ("centre", 0.0), ("high", 100.0)]
            .into_iter()
            .map(|(name, v)| {
                let mut st = rest.clone();
                st.sticks.insert(*k, v);
                RowState {
                    name: name.into(),
                    source: None,
                    st,
                }
            })
            .collect();
        specs.push(RowSpec {
            id: format!("stick:{}", k.source()),
            label: format!("{} stick", k.label()),
            kind: ControlKind::Stick,
            switch_type: None,
            states: positions,
        });
    }

    let sticks = stick_channels(&ev, &rest);
    let stick_chs: BTreeSet<u32> = sticks.iter().map(|s| s.ch - 1).collect();
    let mut driven_all: BTreeSet<u32> = BTreeSet::new();
    let mut rows = Vec::new();
    for RowSpec {
        id,
        label,
        kind,
        switch_type,
        states,
    } in specs
    {
        let outs: Vec<Outcome> = states.iter().map(|s| ev.run(&s.st)).collect();
        let all_ch: BTreeSet<u32> = outs
            .iter()
            .flat_map(|o| o.channels.keys().copied())
            .collect();
        let driven: Vec<u32> = all_ch
            .into_iter()
            .filter(|c| kind != ControlKind::Stick || !stick_chs.contains(c))
            .filter(|c| {
                let vals: BTreeSet<u16> = outs
                    .iter()
                    .map(|o| us(o.channels.get(c).copied().unwrap_or(0.0)))
                    .collect();
                vals.len() > 1
            })
            .collect();
        driven_all.extend(driven.iter().copied());
        // A condition's value at every position: what changes with this control.
        let differs = |c: &str| {
            states
                .iter()
                .zip(&outs)
                .map(|(s, x)| ev.cond(c, &s.st, &x.ls))
                .collect::<BTreeSet<_>>()
                .len()
                > 1
        };
        let ls_differs = |i: usize| {
            outs.iter()
                .map(|o| o.ls.get(i).copied().unwrap_or(Tri::Off))
                .collect::<BTreeSet<_>>()
                .len()
                > 1
        };
        let mut positions = Vec::new();
        for (rs, o) in states.iter().zip(&outs) {
            let chv = |c: u32| us(o.channels.get(&c).copied().unwrap_or(0.0));
            let channels: Vec<ChannelValue> = driven
                .iter()
                .map(|c| ChannelValue {
                    ch: c + 1,
                    us: chv(*c),
                })
                .collect();
            let mut fc = Vec::new();
            for m in &modes {
                if m.linked.is_none()
                    && driven.contains(&(m.ch - 1))
                    && in_range(chv(m.ch - 1), m.start, m.end)
                    && !fc.contains(&m.name)
                {
                    fc.push(m.name.clone());
                }
            }
            for a in &adjustments {
                let active = in_range(chv(a.range_ch - 1), a.start, a.end);
                let mine =
                    driven.contains(&(a.select_ch - 1)) || driven.contains(&(a.range_ch - 1));
                if !active || !mine {
                    continue;
                }
                let t = if a.select {
                    format!("{} {}", a.name, select_position(chv(a.select_ch - 1)) + 1)
                } else if driven.contains(&(a.select_ch - 1)) {
                    format!("{} adjust", a.name)
                } else {
                    format!("{} adjust on CH{}", a.name, a.select_ch)
                };
                if !fc.contains(&t) {
                    fc.push(t);
                }
            }
            let mut radio = Vec::new();
            for l in &model.logical_switches {
                let i = l.index as usize;
                if ls_differs(i) && o.ls.get(i) == Some(&Tri::On) {
                    let pulse = if l.func == "FUNC_EDGE" {
                        " (pulse)"
                    } else {
                        ""
                    };
                    radio.push(format!("L{} on{pulse}", l.index + 1));
                }
            }
            for t in &model.timers {
                if differs(&t.swtch) && ev.cond(&t.swtch, &rs.st, &o.ls) == Tri::On {
                    let name = if t.name.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", t.name)
                    };
                    radio.push(format!("Timer {}{name} runs", t.index + 1));
                }
            }
            for f in &model.special_functions {
                if differs(&f.swtch) && ev.cond(&f.swtch, &rs.st, &o.ls) == Tri::On {
                    radio.push(sf_text(model, &f.func, &f.def));
                }
            }
            positions.push(Position {
                name: rs.name.clone(),
                source: rs.source.clone(),
                channels,
                fc,
                radio,
            });
        }
        // A mode on at every position does not change with this control.
        let everywhere: Vec<String> = positions
            .first()
            .map(|p: &Position| p.fc.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|m| modes.iter().any(|x| &x.name == m))
            .filter(|m| positions.iter().all(|p| p.fc.contains(m)))
            .collect();
        for p in &mut positions {
            p.fc.retain(|m| !everywhere.contains(m));
        }
        for m in everywhere {
            let n = format!("{m} is on in every position of {label}.");
            if !notes.contains(&n) {
                notes.push(n);
            }
        }
        if kind == ControlKind::Switch
            && positions
                .iter()
                .all(|p| p.channels.is_empty() && p.radio.is_empty() && p.fc.is_empty())
        {
            conflicts.push(format!(
                "{label} does nothing: no channel, logical switch or special function changes with it."
            ));
        }
        rows.push(ControlRow {
            id,
            label,
            kind,
            switch_type,
            positions,
        });
    }

    // FC modes against the model.
    if inputs.model.is_some() {
        for m in modes.iter().filter(|m| m.linked.is_none()) {
            let c = m.ch - 1;
            if !driven_all.contains(&c) {
                let at_rest = us(base.channels.get(&c).copied().unwrap_or(0.0));
                let state = if in_range(at_rest, m.start, m.end) {
                    "always on"
                } else {
                    "never on"
                };
                conflicts.push(format!(
                    "{} is on AUX{} (CH{}), which no control moves: {state}.",
                    m.name,
                    m.ch - 4,
                    m.ch
                ));
                continue;
            }
            let reached = rows.iter().any(|r| {
                r.positions.iter().any(|p| {
                    p.channels
                        .iter()
                        .any(|cv| cv.ch == m.ch && in_range(cv.us, m.start, m.end))
                })
            });
            if !reached {
                conflicts.push(format!(
                    "{} on AUX{} {}-{} is never reached: no position sends a value in that range.",
                    m.name,
                    m.ch - 4,
                    m.start,
                    m.end
                ));
            }
        }
    }
    let live_modes: Vec<&AuxMode> = modes.iter().filter(|m| m.linked.is_none()).collect();
    for (i, a) in live_modes.iter().enumerate() {
        for b in &live_modes[i + 1..] {
            if a.ch == b.ch && a.mode_id != b.mode_id && a.start == b.start && a.end == b.end {
                conflicts.push(format!(
                    "{} and {} share one range: AUX{} {}-{}.",
                    a.name,
                    b.name,
                    a.ch - 4,
                    a.start,
                    a.end
                ));
            }
        }
    }
    if let Some(sounds) = &inputs.sounds {
        let mut missing = BTreeSet::new();
        for f in &model.special_functions {
            if f.func == "PLAY_TRACK" {
                let track = f.def.split(',').next().unwrap_or("").trim();
                if !track.is_empty() && !sounds.contains(&track.to_ascii_lowercase()) {
                    missing.insert(track.to_string());
                }
            }
        }
        for t in missing {
            conflicts.push(format!(
                "Sound \"{t}\" is not on the card: a special function plays it."
            ));
        }
    }
    if model
        .mixes
        .iter()
        .any(|m| m.weight.trim().parse::<f64>().is_err())
    {
        notes.push("A mix weight is a global variable: it is read as 100 %.".into());
    }
    if model.logical_switches.iter().any(|l| {
        l.def.contains("tele(")
            || matches!(
                l.func.as_str(),
                "FUNC_TIMER" | "FUNC_STICKY" | "FUNC_DIFFEGREATER" | "FUNC_ADIFFEGREATER"
            )
    }) {
        notes.push(
            "Logical switches on telemetry, timers or sticky state are left out: the map cannot know them."
                .into(),
        );
    }

    SwitchMap {
        model: inputs.model.as_ref().map(|m| m.name.clone()),
        sources: inputs.sources.clone(),
        rows,
        sticks,
        modes,
        adjustments,
        conflicts,
        notes,
        live: None,
    }
}

/// Each stick's channel: the channel whose value follows that stick at rest.
fn stick_channels(ev: &Eval, rest: &State) -> Vec<StickChannel> {
    let base = ev.run(rest);
    let mut out = Vec::new();
    for k in Stick::ALL {
        let mut st = rest.clone();
        let from = st.sticks.get(&k).copied().unwrap_or(0.0);
        let to = if from <= -100.0 { 0.0 } else { from + 50.0 };
        st.sticks.insert(k, to);
        let moved = ev.run(&st);
        let best = moved
            .channels
            .iter()
            .map(|(c, v)| (*c, v - base.channels.get(c).copied().unwrap_or(0.0)))
            .filter(|(_, d)| d.abs() > 0.5)
            .min_by_key(|(c, _)| *c);
        if let Some((c, d)) = best {
            out.push(StickChannel {
                stick: k,
                ch: c + 1,
                weight: (d * 100.0 / (to - from)).round() as i32,
            });
        }
    }
    out
}

/// Matches channel values (µs, CH1 first) to the map: each row's position, the modes on,
/// the selections made.
pub fn live(map: &SwitchMap, source: &str, channels: &[u16]) -> Live {
    let at = |ch: u32| channels.get(ch as usize - 1).copied();
    let mut positions = BTreeMap::new();
    for r in &map.rows {
        let mut best: Option<(u32, u32)> = None;
        for (i, p) in r.positions.iter().enumerate() {
            if p.channels.is_empty() {
                continue;
            }
            let mut total = 0u32;
            let mut ok = true;
            for cv in &p.channels {
                match at(cv.ch) {
                    Some(v) if v.abs_diff(cv.us) <= LIVE_TOLERANCE_US => {
                        total += v.abs_diff(cv.us) as u32
                    }
                    _ => ok = false,
                }
            }
            if ok && best.is_none_or(|(_, t)| total < t) {
                best = Some((i as u32, total));
            }
        }
        positions.insert(r.id.clone(), best.map(|(i, _)| i));
    }
    let mut modes = Vec::new();
    for m in map.modes.iter().filter(|m| m.linked.is_none()) {
        if at(m.ch).is_some_and(|v| in_range(v, m.start, m.end)) && !modes.contains(&m.name) {
            modes.push(m.name.clone());
        }
    }
    let mut adjustments = Vec::new();
    for a in &map.adjustments {
        let (Some(r), Some(s)) = (at(a.range_ch), at(a.select_ch)) else {
            continue;
        };
        if in_range(r, a.start, a.end) {
            adjustments.push(if a.select {
                format!("{} {}", a.name, select_position(s) + 1)
            } else {
                format!("{} adjust", a.name)
            });
        }
    }
    Live {
        source: source.into(),
        channels: channels.to_vec(),
        positions,
        modes,
        adjustments,
    }
}

/// The map as text, one block per control, for the CLI's `--text` and golden tests.
pub fn render_text(m: &SwitchMap) -> String {
    let mut s = String::new();
    s += &format!(
        "Switch map: {}\nRead: {}\n",
        m.model.as_deref().unwrap_or("(no model)"),
        if m.sources.is_empty() {
            "-".to_string()
        } else {
            m.sources.join(", ")
        }
    );
    if !m.sticks.is_empty() {
        let st: Vec<String> = m
            .sticks
            .iter()
            .map(|k| format!("{} CH{} {}%", k.stick.label(), k.ch, k.weight))
            .collect();
        s += &format!("Sticks: {}\n", st.join(", "));
    }
    for r in &m.rows {
        s += &format!(
            "\n{}{}\n",
            r.label,
            r.switch_type
                .as_deref()
                .map(|t| format!(" ({t})"))
                .unwrap_or_default()
        );
        let live = m
            .live
            .as_ref()
            .and_then(|l| l.positions.get(&r.id).copied().flatten());
        for (i, p) in r.positions.iter().enumerate() {
            let mark = if live == Some(i as u32) { "*" } else { " " };
            let ch: Vec<String> = p
                .channels
                .iter()
                .map(|c| format!("CH{} {}", c.ch, c.us))
                .collect();
            let join = |v: &[String]| {
                if v.is_empty() {
                    "-".to_string()
                } else {
                    v.join("; ")
                }
            };
            s += &format!(
                "{mark} {:<9} {:<22} FC: {:<34} Radio: {}\n",
                p.name,
                join(&ch),
                join(&p.fc),
                join(&p.radio)
            );
        }
    }
    if !m.modes.is_empty() {
        s += "\nModes\n";
        for x in &m.modes {
            match &x.linked {
                Some(l) => s += &format!("  {:<22} linked to {l}\n", x.name),
                None => {
                    s += &format!(
                        "  {:<22} AUX{} (CH{}) {}-{}\n",
                        x.name,
                        x.ch - 4,
                        x.ch,
                        x.start,
                        x.end
                    )
                }
            }
        }
    }
    if !m.adjustments.is_empty() {
        s += "\nAdjustments\n";
        for a in &m.adjustments {
            s += &format!(
                "  {:<22} on while AUX{} {}-{}, {} by AUX{}\n",
                a.name,
                a.range_ch - 4,
                a.start,
                a.end,
                if a.select { "selected" } else { "adjusted" },
                a.select_ch - 4
            );
        }
    }
    if let Some(l) = &m.live {
        s += &format!(
            "\nLive ({}): {}\n  modes: {}\n",
            l.source,
            l.channels
                .iter()
                .enumerate()
                .map(|(i, v)| format!("CH{} {v}", i + 1))
                .collect::<Vec<_>>()
                .join(", "),
            if l.modes.is_empty() {
                "-".into()
            } else {
                l.modes.join(", ")
            }
        );
        if !l.adjustments.is_empty() {
            s += &format!("  adjustments: {}\n", l.adjustments.join(", "));
        }
    }
    if !m.conflicts.is_empty() {
        s += "\nConflicts\n";
        for c in &m.conflicts {
            s += &format!("  - {c}\n");
        }
    }
    if !m.notes.is_empty() {
        s += "\nNotes\n";
        for n in &m.notes {
            s += &format!("  - {n}\n");
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microseconds_and_selects() {
        assert_eq!(us(-100.0), 988);
        assert_eq!(us(0.0), 1500);
        assert_eq!(us(100.0), 2012);
        assert_eq!(select_position(988), 0);
        assert_eq!(select_position(1500), 1);
        assert_eq!(select_position(2012), 2);
        assert_eq!(select_position(2100), 2);
        assert!(in_range(2012, 1700, 2100));
        assert!(!in_range(1500, 1700, 2100));
        assert!(in_range(2150, 1700, 2100), "2100 is the top step");
    }

    #[test]
    fn tri_logic() {
        assert_eq!(Tri::On.and(Tri::Unknown), Tri::Unknown);
        assert_eq!(Tri::Off.and(Tri::Unknown), Tri::Off);
        assert_eq!(Tri::On.or(Tri::Unknown), Tri::On);
        assert_eq!(Tri::Off.or(Tri::Off), Tri::Off);
    }

    #[test]
    fn aux_and_adjrange_lines() {
        let c = Config::parse(
            "aux 0 0 0 1700 2100 0 0\naux 1 1 1 900 1300 0 0\naux 2 0 0 900 900 0 0\naux 1 2 1 1300 1700 0 0\nadjrange 0 0 2 900 2100 12 2 0 0\nadjrange 1 0 0 900 900 0 0 0 0\n",
        );
        let m = aux_modes(&c);
        assert_eq!(m.len(), 2, "an empty range is not in use");
        assert_eq!((m[0].name.as_str(), m[0].ch), ("ARM", 5));
        assert_eq!(m[1].name, "HORIZON", "a later line with the same slot wins");
        let a = adjustments(&c);
        assert_eq!(a.len(), 1);
        assert_eq!(
            (
                a[0].name.as_str(),
                a[0].range_ch,
                a[0].select_ch,
                a[0].select
            ),
            ("Rate profile", 7, 7, true)
        );
        assert_eq!(mode_name(99), "mode 99");
    }

    #[test]
    fn trims_have_plain_labels() {
        assert_eq!(trim_label("TrimThrDown"), "Throttle trim down");
        assert_eq!(trim_label("TrimRudRight"), "Yaw trim right");
    }
}
