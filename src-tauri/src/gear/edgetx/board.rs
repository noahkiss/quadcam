//! The radios' controls, per board, for the card engine: which switches, buttons, pots and
//! sliders a board has, and the model keys only that board's YAML schema holds. Storage
//! kind and speed are the radio profile's (`gear::radios`). Data, reviewed per board
//! (design 6.3, "Boards").
//!
//! - `spec` finds a board's facts; `inventory` joins them with what `radio.yml` and a model
//!   say (a switch's type, a pot's detent, a custom switch's per-model setup).
//! - `custom_switches` reads a model's custom switches (`SW1`..`SW6` on a GX12) in both
//!   layouts: the `customSwitches:` block EdgeTX 2.12 writes and the 2.11 bit fields
//!   (`functionSwitchConfig` and its siblings), which 2.12 still reads but never writes.
//! - `ControlLink` and `default_links` map one board's controls onto another's;
//!   `translate` rewrites a model's control references through such a map, for a model
//!   copied from one radio to another (`Edit::ModelImport`).
//!
//! Learned from EdgeTX's per-board YAML schema files and its documentation; no EdgeTX code
//! is copied.

use super::yaml::{unquote, Doc, Node};
use crate::gear::model::{Refusal, RefusalCode};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::{BTreeMap, BTreeSet};

/// What kind of control a radio has.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum BoardControlKind {
    /// A switch on the case (`SA`).
    Switch,
    /// A button whose type, group and LED colour each model sets (`SW1` on a GX12).
    Custom,
    /// A knob (`P1`).
    Pot,
    /// A slider (`SL1`).
    Slider,
}

/// Where a control's type was read.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum ControlSource {
    /// QuadCam's board table.
    Board,
    /// `radio.yml` (`switchConfig`, `potsConfig`).
    Radio,
    /// The model's custom-switch setup.
    Model,
}

/// One control of a radio.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct BoardControl {
    /// `SA`, `SW1`, `P1`, `SL1`.
    pub name: String,
    pub kind: BoardControlKind,
    /// `2POS`, `3POS` or `TOGGLE` (held while pressed) for a switch or a custom switch.
    #[serde(default)]
    pub switch_type: Option<String>,
    /// A pot with a centre detent.
    #[serde(default)]
    pub detent: bool,
    /// The name the radio or the model gives it, when it has one.
    #[serde(default)]
    pub label: Option<String>,
    pub source: ControlSource,
}

/// A board's facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlSpec {
    /// `radio.yml`'s `board:` value.
    pub board: &'static str,
    /// Switches on the case and their factory types.
    pub switches: &'static [(&'static str, &'static str)],
    /// Custom switches (per-model buttons) and their factory type.
    pub custom: &'static [&'static str],
    /// Pots and whether each has a centre detent.
    pub pots: &'static [(&'static str, bool)],
    pub sliders: &'static [&'static str],
    /// Characters per checklist line on the screen.
    pub checklist_width: usize,
    /// Top-level model keys only this board's schema holds. A model copied to a board
    /// without them leaves them out; one copied to this board takes them from a model
    /// already on the card.
    pub model_only_keys: &'static [&'static str],
}

/// The GX12's per-model custom-switch keys: the 2.12 blocks first, then the 2.11 bit
/// fields 2.12 still reads.
const GX12_CFS_KEYS: &[&str] = &[
    "customSwitches",
    "cfsGroupOn",
    "functionSwitchConfig",
    "functionSwitchGroup",
    "functionSwitchStartConfig",
    "functionSwitchLogicalState",
    "switchNames",
    "functionSwitchLedONColor",
    "functionSwitchLedOFFColor",
];

/// Every board QuadCam knows the controls of.
pub const BOARDS: &[ControlSpec] = &[
    // RadioMaster Pocket: 128x64 schema; SA and SD latch, SE is a momentary button; P1 is
    // the S1 wheel. A card in a reader writes fast; over its USB about 0.3 MB/s.
    ControlSpec {
        board: "pocket",
        switches: &[
            ("SA", "2POS"),
            ("SB", "3POS"),
            ("SC", "3POS"),
            ("SD", "2POS"),
            ("SE", "TOGGLE"),
        ],
        custom: &[],
        pots: &[("P1", false)],
        sliders: &[],
        checklist_width: 20,
        model_only_keys: &[],
    },
    // RadioMaster GX12: its own schema (the 128x64 one plus custom switches); SG and SH
    // are momentary; SW1-SW6 are per-model buttons; 512 MB of flash over USB full speed,
    // about 0.36 MB/s.
    ControlSpec {
        board: "gx12",
        switches: &[
            ("SA", "2POS"),
            ("SB", "3POS"),
            ("SC", "3POS"),
            ("SD", "2POS"),
            ("SE", "3POS"),
            ("SF", "3POS"),
            ("SG", "TOGGLE"),
            ("SH", "TOGGLE"),
        ],
        custom: &["SW1", "SW2", "SW3", "SW4", "SW5", "SW6"],
        pots: &[("P1", true), ("P2", true)],
        sliders: &["SL1", "SL2"],
        checklist_width: 20,
        model_only_keys: GX12_CFS_KEYS,
    },
];

/// Default links between two boards beyond "same name, same type": `(from board, to
/// board, from control, to control)`.
const LINKS: &[(&str, &str, &str, &str)] = &[
    // The Pocket's momentary SE and the GX12's momentary SH.
    ("pocket", "gx12", "SE", "SH"),
    ("gx12", "pocket", "SH", "SE"),
];

/// True when the board keeps its files on a soldered chip (`radios::StorageKind::Chip`):
/// QuadCam reaches them only over the radio's USB, never in a card reader.
pub fn on_chip(board: Option<&str>) -> bool {
    board
        .and_then(|b| crate::gear::radios::profile(b.trim()))
        .is_some_and(|p| p.storage == crate::gear::radios::StorageKind::Chip)
}

/// The custom-switch type a model's 2.12 block names when the radio's own setup applies.
pub const GLOBAL: &str = "GLOBAL";

/// The facts of a board, by `radio.yml`'s `board:` value.
pub fn spec(board: Option<&str>) -> Option<&'static ControlSpec> {
    let b = board?.trim().to_ascii_lowercase();
    BOARDS.iter().find(|s| s.board == b)
}

/// A switch type as EdgeTX writes it in either case (`2pos`, `2POS`), upper case.
pub fn switch_type(raw: &str) -> String {
    unquote(raw.trim()).to_ascii_uppercase()
}

impl ControlSpec {
    /// The board's controls with their factory types.
    pub fn controls(&self) -> Vec<BoardControl> {
        let mut out: Vec<BoardControl> = self
            .switches
            .iter()
            .map(|(n, t)| control(n, BoardControlKind::Switch, Some(t), false))
            .collect();
        out.extend(
            self.custom
                .iter()
                .map(|n| control(n, BoardControlKind::Custom, Some("2POS"), false)),
        );
        out.extend(
            self.pots
                .iter()
                .map(|(n, d)| control(n, BoardControlKind::Pot, None, *d)),
        );
        out.extend(
            self.sliders
                .iter()
                .map(|n| control(n, BoardControlKind::Slider, None, false)),
        );
        out
    }
}

fn control(name: &str, kind: BoardControlKind, t: Option<&str>, detent: bool) -> BoardControl {
    BoardControl {
        name: name.into(),
        kind,
        switch_type: t.map(str::to_string),
        detent,
        label: None,
        source: ControlSource::Board,
    }
}

fn label(raw: Option<&str>) -> Option<String> {
    raw.map(|v| unquote(v.trim()).to_string())
        .filter(|v| !v.is_empty())
}

/// The radio's controls: the board's table (when QuadCam knows the board), with the types
/// and names `radio.yml` gives, and the custom switches as `model` sets them. A switch
/// or pot set to none is left out.
pub fn inventory(
    board: Option<&str>,
    radio: Option<&Doc>,
    model: Option<&Doc>,
) -> Vec<BoardControl> {
    let mut out = spec(board).map(|s| s.controls()).unwrap_or_default();
    let mut drop: BTreeSet<String> = BTreeSet::new();
    if let Some(r) = radio {
        if let Ok(Some(n)) = r.top("switchConfig") {
            for c in &n.children {
                let Some(name) = c.key.clone() else { continue };
                let t = switch_type(c.get("type").unwrap_or("3POS"));
                if t == "NONE" {
                    drop.insert(name);
                    continue;
                }
                let kind = if is_custom_name(&name) {
                    BoardControlKind::Custom
                } else {
                    BoardControlKind::Switch
                };
                let slot = upsert(&mut out, &name, kind);
                if kind == BoardControlKind::Switch || t != GLOBAL {
                    slot.switch_type = Some(t);
                    slot.source = ControlSource::Radio;
                }
                slot.label = label(c.get("name"));
            }
        }
        if let Ok(Some(n)) = r.top("potsConfig") {
            for c in &n.children {
                let Some(name) = c.key.clone() else { continue };
                let t = unquote(c.get("type").unwrap_or("")).to_ascii_lowercase();
                if t == "none" {
                    drop.insert(name);
                    continue;
                }
                let kind = if t == "slider" || name.starts_with("SL") {
                    BoardControlKind::Slider
                } else {
                    BoardControlKind::Pot
                };
                let slot = upsert(&mut out, &name, kind);
                slot.detent = t == "with_detent";
                slot.label = label(c.get("name"));
                slot.source = ControlSource::Radio;
            }
        }
    }
    if let Some(m) = model {
        for cs in custom_switches(m) {
            if cs.switch_type == "NONE" {
                drop.insert(cs.switch.clone());
                continue;
            }
            let slot = upsert(&mut out, &cs.switch, BoardControlKind::Custom);
            if cs.switch_type != GLOBAL {
                slot.switch_type = Some(cs.switch_type.clone());
                slot.source = ControlSource::Model;
            }
            if cs.name.is_some() {
                slot.label = cs.name.clone();
            }
        }
    }
    out.retain(|c| !drop.contains(&c.name));
    out
}

fn upsert<'a>(
    out: &'a mut Vec<BoardControl>,
    name: &str,
    kind: BoardControlKind,
) -> &'a mut BoardControl {
    match out.iter().position(|c| c.name == name) {
        Some(i) => &mut out[i],
        None => {
            out.push(control(name, kind, None, false));
            out.last_mut().unwrap()
        }
    }
}

/// `SW1`..`SW9`: a custom switch's name.
pub fn is_custom_name(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 3 && b[0] == b'S' && b[1] == b'W' && (b'1'..=b'9').contains(&b[2])
}

// ----- custom switches -----

/// Which layout a model's custom-switch setup is in.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum CustomLayout {
    /// The `customSwitches:` block EdgeTX 2.12 writes.
    Blocks,
    /// The 2.11 bit fields (`functionSwitchConfig` and its siblings), which 2.12 reads and
    /// replaces on its next save of the model.
    Legacy,
}

/// One custom switch as a model sets it up.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct CustomSwitch {
    /// `SW1`; on a GX12 also `SA` and `SD`, whose LEDs a model may colour.
    pub switch: String,
    /// `2POS`, `TOGGLE`, `NONE`, or `GLOBAL` (the radio's own setup applies).
    pub switch_type: String,
    /// Its group: 0 none; in a group, turning one on turns the others off.
    pub group: u8,
    /// Its state at power-on: `off`, `on` or `previous`.
    pub start: String,
    /// On when the model was last saved.
    pub state: bool,
    #[serde(default)]
    pub name: Option<String>,
    /// LED colour when on and when off, `[r, g, b]`.
    #[serde(default)]
    pub on_color: Option<[u8; 3]>,
    #[serde(default)]
    pub off_color: Option<[u8; 3]>,
    pub layout: CustomLayout,
}

const TYPES_BY_CODE: [&str; 4] = ["NONE", "TOGGLE", "2POS", "3POS"];

fn color(n: Option<&Node>) -> Option<[u8; 3]> {
    let n = n?;
    let v = |k: &str| n.get(k).and_then(|v| v.trim().parse::<u8>().ok());
    Some([v("r")?, v("g")?, v("b")?])
}

fn start_name(raw: &str) -> String {
    match unquote(raw.trim()) {
        "START_ON" => "on",
        "START_OFF" => "off",
        _ => "previous",
    }
    .into()
}

/// A model's custom switches, from the 2.12 block when it has one, else from the 2.11 bit
/// fields (the first six switches, `SW1`..`SW6`). Empty for a model without either.
pub fn custom_switches(doc: &Doc) -> Vec<CustomSwitch> {
    if let Ok(Some(n)) = doc.top("customSwitches") {
        return n
            .children
            .iter()
            .filter_map(|c| {
                let sw = c.key.clone()?;
                Some(CustomSwitch {
                    switch: sw,
                    switch_type: switch_type(c.get("type").unwrap_or("NONE")),
                    group: c
                        .get("group")
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0),
                    start: start_name(c.get("start").unwrap_or("")),
                    state: c.get("state").map(str::trim) == Some("1"),
                    name: label(c.get("name")),
                    on_color: color(c.child("onColor")),
                    off_color: color(c.child("offColor")),
                    layout: CustomLayout::Blocks,
                })
            })
            .collect();
    }
    let num = |k: &str| doc.top_value(k).and_then(|v| v.trim().parse::<u32>().ok());
    let Some(config) = num("functionSwitchConfig") else {
        return Vec::new();
    };
    let group = num("functionSwitchGroup").unwrap_or(0);
    let start = num("functionSwitchStartConfig").unwrap_or(0);
    let state = num("functionSwitchLogicalState").unwrap_or(0);
    let by_index = |key: &str| -> BTreeMap<u32, Node> {
        doc.top(key)
            .ok()
            .flatten()
            .map(|n| {
                n.children
                    .iter()
                    .filter_map(|c| Some((c.index()?, c.clone())))
                    .collect()
            })
            .unwrap_or_default()
    };
    let names = by_index("switchNames");
    let on = by_index("functionSwitchLedONColor");
    let off = by_index("functionSwitchLedOFFColor");
    (0..6u32)
        .map(|i| {
            let bits2 = |v: u32| (v >> (2 * i)) & 3;
            // 2.11 stored "on" as 0 and "off" as 1; 2 is "previous".
            let start = match bits2(start) {
                0 => "on",
                1 => "off",
                _ => "previous",
            };
            CustomSwitch {
                switch: format!("SW{}", i + 1),
                switch_type: TYPES_BY_CODE[bits2(config) as usize].into(),
                group: bits2(group) as u8,
                start: start.into(),
                state: (state >> i) & 1 == 1,
                name: label(names.get(&i).and_then(|n| n.get("val"))),
                on_color: color(on.get(&i)),
                off_color: color(off.get(&i)),
                layout: CustomLayout::Legacy,
            }
        })
        .collect()
}

// ----- one board's controls onto another's -----

/// One control of the source radio and the control of the target radio that takes its
/// place. The switch-assignment editor sets these.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct ControlLink {
    pub from: String,
    pub to: String,
}

/// How a model's controls map onto another radio's.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct ControlUse {
    /// The source radio's control the model reads.
    pub from: String,
    /// The target radio's control; None when nothing maps it.
    #[serde(default)]
    pub to: Option<String>,
    /// Why the link needs a look (a 3-position switch on a 2-position one, ...).
    #[serde(default)]
    pub note: Option<String>,
}

/// What a translation did, or would need.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct Translation {
    /// Each control the model reads, in name order.
    pub used: Vec<ControlUse>,
    /// Controls the model reads that no link maps.
    pub unmapped: Vec<String>,
    pub warnings: Vec<String>,
}

fn compatible(a: &BoardControl, b: &BoardControl) -> bool {
    use BoardControlKind::*;
    match (a.kind, b.kind) {
        (Switch | Custom, Switch | Custom) => a.switch_type == b.switch_type,
        (Pot | Slider, Pot | Slider) => true,
        _ => false,
    }
}

/// The default map from one radio's controls to another's, against the target's current
/// fit (`inventory`): the board pair's own links (a Pocket's momentary SE is a GX12's SH)
/// where the target control still has that type, else the same name with the same kind
/// and type. A control with neither stays unmapped.
pub fn default_links(
    from_board: &str,
    from: &[BoardControl],
    to_board: &str,
    to: &[BoardControl],
) -> Vec<ControlLink> {
    let mut out = Vec::new();
    for f in from {
        let same = to.iter().find(|t| t.name == f.name && compatible(f, t));
        let special = LINKS.iter().find_map(|(fb, tb, fc, tc)| {
            (fb.eq_ignore_ascii_case(from_board)
                && tb.eq_ignore_ascii_case(to_board)
                && *fc == f.name)
                .then_some(*tc)
                .filter(|tc| to.iter().any(|t| t.name == *tc && compatible(f, t)))
        });
        let pick = special.map(str::to_string).or(same.map(|t| t.name.clone()));
        if let Some(t) = pick {
            out.push(ControlLink {
                from: f.name.clone(),
                to: t,
            });
        }
    }
    out
}

/// The keys whose values name controls.
const SOURCE_KEYS: &[&str] = &["srcRaw", "swtch", "andsw", "def", "val", "thrTraceSrc"];

/// Special functions whose `def` names a file or a number, never a control.
const DEF_NOT_A_SOURCE: &[&str] = &[
    "PLAY_TRACK",
    "PLAY_SOUND",
    "PLAY_SCRIPT",
    "SET_SCREEN",
    "LOGS",
    "RGB_LED",
];

/// What a token names among `controls`: the control, and the switch position after it
/// (`SA2`, `SW12`), if any.
fn token_control<'a>(
    tok: &str,
    controls: &'a [BoardControl],
) -> Option<(&'a BoardControl, Option<char>)> {
    if let Some(c) = controls.iter().find(|c| c.name == tok) {
        return Some((c, None));
    }
    let (head, last) = tok.split_at(tok.len().checked_sub(1)?);
    let pos = last.chars().next().filter(|p| ('0'..='2').contains(p))?;
    controls
        .iter()
        .find(|c| {
            c.name == head && matches!(c.kind, BoardControlKind::Switch | BoardControlKind::Custom)
        })
        .map(|c| (c, Some(pos)))
}

/// Rewrites each identifier in `value` that names a control. `f` gets the control and
/// the position and answers the new name, or None to keep the token.
fn map_tokens(value: &str, mut f: impl FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(value.len());
    let mut tok = String::new();
    let flush = |tok: &mut String, out: &mut String, f: &mut dyn FnMut(&str) -> Option<String>| {
        if !tok.is_empty() {
            match f(tok) {
                Some(n) => out.push_str(&n),
                None => out.push_str(tok),
            }
            tok.clear();
        }
    };
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            tok.push(ch);
        } else {
            flush(&mut tok, &mut out, &mut f);
            out.push(ch);
        }
    }
    flush(&mut tok, &mut out, &mut f);
    out
}

/// The lines whose values may name controls, and the switch-warning entries (their key is
/// the switch). Skips `def` of special functions that play a file.
fn source_lines(doc: &Doc) -> Result<(Vec<usize>, Vec<usize>), Refusal> {
    let mut skip: BTreeSet<usize> = BTreeSet::new();
    if let Some(n) = doc.top("customFn")? {
        for c in &n.children {
            let func = c.get("func").unwrap_or("");
            if DEF_NOT_A_SOURCE.contains(&func) {
                if let Some(d) = c.child("def") {
                    skip.insert(d.line);
                }
            }
        }
    }
    let mut values = Vec::new();
    for (i, l) in doc.lines().iter().enumerate() {
        let t = l.trim_start();
        if skip.contains(&i) {
            continue;
        }
        if SOURCE_KEYS
            .iter()
            .any(|k| t.strip_prefix(k).is_some_and(|r| r.starts_with(": ")))
        {
            values.push(i);
        }
    }
    let warnings = doc
        .top("switchWarning")?
        .map(|n| n.children.iter().map(|c| c.line).collect())
        .unwrap_or_default();
    Ok((values, warnings))
}

/// The controls of `from` a model reads.
pub fn controls_used(doc: &Doc, from: &[BoardControl]) -> Result<BTreeSet<String>, Refusal> {
    let (values, warnings) = source_lines(doc)?;
    let mut used = BTreeSet::new();
    for i in values {
        let l = &doc.lines()[i];
        let (_, v) = l.split_once(": ").unwrap_or((l, ""));
        map_tokens(v, |t| {
            if let Some((c, _)) = token_control(t, from) {
                used.insert(c.name.clone());
            }
            None
        });
    }
    for i in warnings {
        let k = doc.lines()[i].trim().trim_end_matches(':').to_string();
        if from.iter().any(|c| c.name == k) {
            used.insert(k);
        }
    }
    Ok(used)
}

/// What mapping the controls `doc` reads through `links` gives: each control used, those
/// no link maps, and warnings (a middle position the target cannot reach, a knob for a
/// switch). Changes nothing.
pub fn plan_translation(
    doc: &Doc,
    from: &[BoardControl],
    to: &[BoardControl],
    links: &[ControlLink],
) -> Result<Translation, Refusal> {
    let used = controls_used(doc, from)?;
    let mut t = Translation::default();
    for name in used {
        let src = from.iter().find(|c| c.name == name);
        let link = links.iter().find(|l| l.from == name);
        let Some(link) = link else {
            t.unmapped.push(name.clone());
            t.used.push(ControlUse {
                from: name,
                to: None,
                note: None,
            });
            continue;
        };
        let dst = to.iter().find(|c| c.name == link.to);
        let note = match (src, dst) {
            (_, None) => {
                t.unmapped.push(name.clone());
                Some(format!("The target radio has no {}.", link.to))
            }
            (Some(s), Some(d)) => link_note(s, d),
            (None, Some(_)) => None,
        };
        if let (Some(n), Some(_)) = (&note, dst) {
            t.warnings.push(format!("{} to {}: {n}", name, link.to));
        }
        t.used.push(ControlUse {
            from: name,
            to: dst.map(|d| d.name.clone()),
            note,
        });
    }
    Ok(t)
}

fn link_note(s: &BoardControl, d: &BoardControl) -> Option<String> {
    use BoardControlKind::*;
    let st = s.switch_type.as_deref().unwrap_or("3POS");
    let dt = d.switch_type.as_deref().unwrap_or("3POS");
    match (s.kind, d.kind) {
        (Switch | Custom, Switch | Custom) if st == dt => None,
        (Switch | Custom, Switch | Custom) => Some(match (st, dt) {
            ("3POS", _) => format!(
                "{} has a middle position, {} ({dt}) does not: what the middle does is never reached.",
                s.name, d.name
            ),
            ("TOGGLE", _) => format!(
                "{} springs back when let go; {} ({dt}) stays where it is put.",
                s.name, d.name
            ),
            (_, "TOGGLE") => format!(
                "{} stays where it is put; {} springs back when let go.",
                s.name, d.name
            ),
            _ => format!("{} is {st}, {} is {dt}.", s.name, d.name),
        }),
        (Pot | Slider, Pot | Slider) if s.detent != d.detent => Some(format!(
            "{} {} a centre detent, {} {}.",
            s.name,
            if s.detent { "has" } else { "has no" },
            d.name,
            if d.detent { "has one" } else { "has none" }
        )),
        (Pot | Slider, Pot | Slider) => None,
        _ => Some(format!(
            "{} is a {}, {} a {}: check what the model does with it.",
            s.name,
            kind_word(s.kind),
            d.name,
            kind_word(d.kind)
        )),
    }
}

fn kind_word(k: BoardControlKind) -> &'static str {
    match k {
        BoardControlKind::Switch => "switch",
        BoardControlKind::Custom => "button",
        BoardControlKind::Pot => "knob",
        BoardControlKind::Slider => "slider",
    }
}

/// Rewrites every control `doc` reads through `links`. Refuses, changing nothing, when a
/// control the model reads has no link or links to a control the target lacks.
pub fn translate(
    doc: &mut Doc,
    from: &[BoardControl],
    to: &[BoardControl],
    links: &[ControlLink],
) -> Result<Translation, Refusal> {
    let t = plan_translation(doc, from, to, links)?;
    if !t.unmapped.is_empty() {
        return Err(Refusal::new(
            RefusalCode::Incompatible,
            format!(
                "The model uses {} with no control on the target radio; map {} first. Nothing was written.",
                t.unmapped.join(", "),
                if t.unmapped.len() == 1 { "it" } else { "them" }
            ),
        ));
    }
    let map: BTreeMap<&str, &str> = links
        .iter()
        .map(|l| (l.from.as_str(), l.to.as_str()))
        .collect();
    let rename = |tok: &str| -> Option<String> {
        let (c, pos) = token_control(tok, from)?;
        let to = map.get(c.name.as_str())?;
        Some(match pos {
            Some(p) => format!("{to}{p}"),
            None => to.to_string(),
        })
    };
    let (values, warnings) = source_lines(doc)?;
    for i in values {
        let l = doc.lines()[i].clone();
        let Some((k, v)) = l.split_once(": ") else {
            continue;
        };
        let nv = map_tokens(v, &rename);
        doc.set_line(i, format!("{k}: {nv}"));
    }
    for i in warnings {
        let l = doc.lines()[i].clone();
        let indent = l.len() - l.trim_start().len();
        let k = l.trim().trim_end_matches(':');
        if let Some(to) = map.get(k) {
            doc.set_line(i, format!("{}{to}:", " ".repeat(indent)));
        }
    }
    Ok(t)
}

/// Gives `doc` the board-only model keys of the target: drops those of `from` that `to`
/// lacks, and copies those `to` has from `template` (a model already on the target card),
/// before the first of `before` the model has, else at the end.
pub fn swap_board_keys(
    doc: &mut Doc,
    from: &ControlSpec,
    to: &ControlSpec,
    template: Option<&Doc>,
) -> Result<Vec<String>, Refusal> {
    let mut notes = Vec::new();
    for k in from.model_only_keys {
        if to.model_only_keys.contains(k) {
            continue;
        }
        if let Some(n) = doc.top(k)? {
            doc.splice(n.span(), vec![]);
        }
    }
    let wanted: Vec<&str> = to
        .model_only_keys
        .iter()
        .filter(|k| !from.model_only_keys.contains(k))
        .copied()
        .collect();
    if wanted.is_empty() {
        return Ok(notes);
    }
    for k in &wanted {
        if let Some(n) = doc.top(k)? {
            doc.splice(n.span(), vec![]);
        }
    }
    let Some(t) = template else {
        return Err(Refusal::new(
            RefusalCode::Incompatible,
            format!(
                "The {} card has no model to take its custom switch setup from; nothing was written.",
                to.board
            ),
        ));
    };
    let mut lines = Vec::new();
    for n in t.tree()? {
        if n.key.as_deref().is_some_and(|k| wanted.contains(&k)) {
            lines.extend(t.slice(n.span()).iter().cloned());
        }
    }
    if lines.is_empty() {
        return Err(Refusal::new(
            RefusalCode::Incompatible,
            format!(
                "{} has no custom switch setup to copy; nothing was written.",
                t.name
            ),
        ));
    }
    notes.push(format!(
        "The custom switches (SW1-SW6, their groups and LED colours) come from {}.",
        t.name
    ));
    let tree = doc.tree()?;
    let at = tree
        .iter()
        .find(|n| {
            n.key.as_deref().is_some_and(|k| {
                ["usbJoystickExtMode", "usbJoystickIfMode", "radioGFDisabled"].contains(&k)
            })
        })
        .map(|n| n.line)
        .unwrap_or(doc.lines().len());
    doc.splice(at..at, lines);
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(s: &str) -> Doc {
        Doc::parse("m.yml", s.as_bytes()).unwrap()
    }

    #[test]
    fn tokens_map_switches_with_positions_and_leave_the_rest() {
        let from = spec(Some("pocket")).unwrap().controls();
        let mut seen = Vec::new();
        let out = map_tokens("\"!SE2,Thr,-90,SA,tele(3),P1,6P12,SAX\"", |t| {
            let (c, p) = token_control(t, &from)?;
            seen.push((c.name.clone(), p));
            Some(format!("<{t}>"))
        });
        assert_eq!(out, "\"!<SE2>,Thr,-90,<SA>,tele(3),<P1>,6P12,SAX\"");
        assert_eq!(
            seen,
            vec![
                ("SE".into(), Some('2')),
                ("SA".into(), None),
                ("P1".into(), None)
            ]
        );
    }

    #[test]
    fn custom_names_and_the_board_table() {
        assert!(is_custom_name("SW1") && !is_custom_name("SW") && !is_custom_name("SA1"));
        let g = spec(Some("GX12")).unwrap();
        assert!(on_chip(Some("gx12")) && !on_chip(Some("pocket")));
        assert_eq!(g.controls().len(), 8 + 6 + 2 + 2);
        assert!(spec(Some("tx16s")).is_none());
    }

    #[test]
    fn legacy_bit_fields_read_as_six_custom_switches() {
        let d = doc("functionSwitchConfig: 2730\nfunctionSwitchGroup: 9557\nfunctionSwitchStartConfig: 2730\nfunctionSwitchLogicalState: 1\nfunctionSwitchLedONColor: \n   0:\n      r: 248\n      g: 0\n      b: 0\n");
        let cs = custom_switches(&d);
        assert_eq!(cs.len(), 6);
        assert_eq!(cs[0].switch, "SW1");
        assert!(cs.iter().all(|c| c.switch_type == "2POS" && c.group == 1));
        assert!(cs.iter().all(|c| c.start == "previous"));
        assert!(cs[0].state && !cs[1].state);
        assert_eq!(cs[0].on_color, Some([248, 0, 0]));
        assert_eq!(cs[1].on_color, None);
        assert_eq!(cs[0].layout, CustomLayout::Legacy);
    }

    #[test]
    fn the_212_block_wins_over_the_legacy_fields() {
        let d = doc("functionSwitchConfig: 0\ncustomSwitches: \n   SW1:\n      name: \"ARM\"\n      type: TOGGLE\n      group: 0\n      start: START_OFF\n      state: 0\n      onColor: \n         r: 0\n         g: 255\n         b: 0\n   SA:\n      type: GLOBAL\n");
        let cs = custom_switches(&d);
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].name.as_deref(), Some("ARM"));
        assert_eq!(cs[0].switch_type, "TOGGLE");
        assert_eq!(cs[0].start, "off");
        assert_eq!(cs[0].on_color, Some([0, 255, 0]));
        assert_eq!(cs[1].switch_type, GLOBAL);
    }

    #[test]
    fn default_links_pocket_to_gx12_and_back() {
        let p = spec(Some("pocket")).unwrap().controls();
        let g = spec(Some("gx12")).unwrap().controls();
        let l = default_links("pocket", &p, "gx12", &g);
        let get = |l: &[ControlLink], f: &str| l.iter().find(|x| x.from == f).map(|x| x.to.clone());
        assert_eq!(get(&l, "SA").as_deref(), Some("SA"));
        assert_eq!(get(&l, "SE").as_deref(), Some("SH"));
        assert_eq!(get(&l, "P1").as_deref(), Some("P1"));
        let back = default_links("gx12", &g, "pocket", &p);
        assert_eq!(get(&back, "SH").as_deref(), Some("SE"));
        // A 3-position SE on the GX12 has no default on the Pocket's momentary SE.
        assert_eq!(get(&back, "SE"), None);
        assert_eq!(get(&back, "SW1"), None);
    }
}
