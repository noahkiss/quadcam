//! Typed views of an EdgeTX model file and the operations QuadCam applies to it
//! (design 6.3). A view reads; a `ModelOp` edits the file's lines through `yaml::Doc` and
//! leaves every other line as it was.
//!
//! Encodings this module keeps (each has a test):
//!
//! - Logical switches: `L1` is index 0; EdgeTX omits empty ones; `delay` and `duration`
//!   count 0.1 s.
//! - Telemetry sources: `tele(N)` is slot N of `telemetrySensors`. An op names a sensor by
//!   its label, `{RxBt}`, and the slot is looked up; a missing label refuses.
//! - Switch sources: `SA0`..`SA2` (switch index × 3 + position); trims by name.
//! - Stick sources in a comparison take -100..100.
//! - Switch warnings: the 2.12 `switchWarning:` list. The legacy `switchWarningState:`
//!   line is replaced, never written.
//! - Special functions: 64 at most; `SET_SCREEN` only on an edge pulse (a `FUNC_EDGE`
//!   logical switch); `PLAY_TRACK` repeat `!1x` is "once, not at power-on"; track names
//!   8 characters at most; telemetry script names 6 at most.
//! - `header.modelId`: index 0 is the internal module; EdgeTX omits zero.
//! - Timer swap: trades every field, the stored value included, and every `TmrN`
//!   reference outside the timers block.

use super::yaml::{quote, shape_refusal, unquote, Doc, Node};
use crate::gear::model::{Refusal, RefusalCode};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::BTreeMap;

/// A key and its raw value, in file order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct Field {
    pub key: String,
    pub value: String,
}

impl Field {
    pub fn new(key: &str, value: &str) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

/// An ELRS (or other) model id for one module slot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct ModuleId {
    /// 0 is the internal module.
    pub module: u32,
    pub id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct Timer {
    /// 0 is Timer 1.
    pub index: u32,
    pub name: String,
    /// The switch that runs it (`L1`, `NONE`).
    pub swtch: String,
    pub mode: String,
    /// The stored count, in seconds.
    pub value: i64,
    /// 0 off, 1 per flight, 2 until a manual reset.
    pub persistent: String,
    /// Every field, raw, in file order.
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct Mix {
    /// 0 is CH1.
    pub dest_ch: u32,
    pub source: String,
    pub weight: String,
    pub swtch: String,
    /// `ADD`, `MUL` or `REPL`.
    pub mltpx: String,
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct LogicalSwitch {
    /// 0 is `L1`.
    pub index: u32,
    pub func: String,
    pub def: String,
    pub andsw: String,
    /// 0.1 s.
    pub delay: u32,
    /// 0.1 s.
    pub duration: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct SpecialFunction {
    pub index: u32,
    pub swtch: String,
    pub func: String,
    pub def: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct SwitchWarning {
    /// `SA`.
    pub switch: String,
    /// `up`, `mid` or `down`.
    pub pos: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct Sensor {
    pub slot: u32,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct Screen {
    /// 0 is telemetry screen 1.
    pub index: u32,
    /// `VALUES`, `BARS`, `SCRIPT`, ...
    pub kind: String,
    /// The script name for a `SCRIPT` screen.
    #[serde(default)]
    pub script: Option<String>,
}

/// What a model file says, as far as Gear reads it. Every list is in file order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct ModelView {
    pub file: String,
    pub name: String,
    pub model_ids: Vec<ModuleId>,
    pub timers: Vec<Timer>,
    pub mixes: Vec<Mix>,
    pub logical_switches: Vec<LogicalSwitch>,
    pub special_functions: Vec<SpecialFunction>,
    pub switch_warnings: Vec<SwitchWarning>,
    /// The file holds the legacy `switchWarningState:` line, which EdgeTX 2.12.4 misreads.
    pub legacy_switch_warning: bool,
    pub sensors: Vec<Sensor>,
    pub screens: Vec<Screen>,
    /// `displayChecklist`.
    pub checklist: bool,
    /// `checklistInteractive`.
    pub checklist_interactive: bool,
}

// ----- names and encodings -----

/// The most logical switches and special functions a model holds.
pub const MAX_LOGICAL_SWITCHES: u32 = 64;
pub const MAX_SPECIAL_FUNCTIONS: usize = 64;
/// Timers a model holds.
pub const MAX_TIMERS: u32 = 3;
/// A `PLAY_TRACK` file stem in `SOUNDS/<lang>/`.
pub const MAX_TRACK_NAME: usize = 8;
/// A telemetry script name in `SCRIPTS/TELEMETRY/`.
pub const MAX_SCRIPT_NAME: usize = 6;
/// A model name.
pub const MAX_MODEL_NAME: usize = 15;
/// Stick and pot sources compare in -100..100.
pub const STICKS: &[&str] = &["Rud", "Ele", "Thr", "Ail"];

/// `L1` for index 0.
pub fn ls_name(index: u32) -> String {
    format!("L{}", index + 1)
}

/// The index of `L13` (12), with or without a leading `!`.
pub fn ls_index(name: &str) -> Option<u32> {
    let n: u32 = name
        .trim_start_matches('!')
        .strip_prefix('L')?
        .parse()
        .ok()?;
    (1..=MAX_LOGICAL_SWITCHES).contains(&n).then(|| n - 1)
}

/// A switch position source (`SC1`) as EdgeTX numbers it: switch index × 3 + position.
pub fn switch_source_index(name: &str) -> Option<u32> {
    let b = name.trim_start_matches('!').as_bytes();
    if b.len() != 3 || b[0] != b'S' || !b[1].is_ascii_uppercase() {
        return None;
    }
    let pos = (b[2] as char).to_digit(10).filter(|p| *p <= 2)?;
    Some((b[1] - b'A') as u32 * 3 + pos)
}

/// The slot of each telemetry sensor, by label (the first slot when two share a label).
pub fn sensor_slots(doc: &Doc) -> Result<BTreeMap<String, u32>, Refusal> {
    let mut out = BTreeMap::new();
    if let Some(n) = doc.top("telemetrySensors")? {
        for c in &n.children {
            if let (Some(slot), Some(label)) = (c.index(), c.get("label")) {
                out.entry(unquote(label).to_string()).or_insert(slot);
            }
        }
    }
    Ok(out)
}

/// Replaces each `{Label}` with the slot of the sensor with that label.
pub fn resolve_sensors(doc: &Doc, text: &str) -> Result<String, Refusal> {
    if !text.contains('{') {
        return Ok(text.to_string());
    }
    let slots = sensor_slots(doc)?;
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('}') else {
            return Err(bad(format!("Unclosed {{ in {text:?}.")));
        };
        let label = &rest[i + 1..i + j];
        let slot = slots.get(label).ok_or_else(|| {
            bad(format!(
                "{} has no telemetry sensor labelled {label:?}; discover sensors on the radio first.",
                doc.name
            ))
        })?;
        out.push_str(&slot.to_string());
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn bad(reason: impl Into<String>) -> Refusal {
    Refusal::new(RefusalCode::BadSetting, reason)
}

fn shape(doc: &Doc, node: &Node, why: &str) -> Refusal {
    shape_refusal(&doc.name, node.line + 1, why)
}

/// Checks a name for a quoted EdgeTX string: printable ASCII, no `"` or `\`.
fn check_text(what: &str, s: &str, max: usize) -> Result<(), Refusal> {
    if s.chars().count() > max {
        return Err(bad(format!("{what} {s:?} is over {max} characters.")));
    }
    if !s
        .chars()
        .all(|c| (' '..='~').contains(&c) && c != '"' && c != '\\')
    {
        return Err(bad(format!(
            "{what} {s:?} may hold only printable ASCII, without quotes or backslashes."
        )));
    }
    Ok(())
}

// ----- reading -----

/// The `N:` children of a block, each with its scalar fields. Refuses a child that is not
/// an index or holds a nested block (unless `nested_ok`).
fn indexed(doc: &Doc, n: &Node, nested_ok: bool) -> Result<Vec<(u32, Vec<Field>)>, Refusal> {
    let mut out = Vec::new();
    for c in &n.children {
        let idx = c
            .index()
            .ok_or_else(|| shape(doc, c, "an index was expected"))?;
        if !nested_ok {
            if let Some(g) = c.children.iter().find(|g| !g.children.is_empty()) {
                return Err(shape(doc, g, "a nested block"));
            }
        }
        out.push((
            idx,
            c.fields()
                .into_iter()
                .map(|(key, value)| Field { key, value })
                .collect(),
        ));
    }
    Ok(out)
}

fn get<'a>(fields: &'a [Field], key: &str) -> &'a str {
    fields
        .iter()
        .find(|f| f.key == key)
        .map(|f| f.value.as_str())
        .unwrap_or("")
}

/// The mixes: ` -` items with scalar fields only.
/// The mixData block and each mix's fields.
type Mixes = (Node, Vec<Vec<Field>>);

fn mix_items(doc: &Doc) -> Result<Option<Mixes>, Refusal> {
    let Some(n) = doc.top("mixData")? else {
        return Ok(None);
    };
    let mut items = Vec::new();
    for c in &n.children {
        if c.key.is_some() {
            return Err(shape(doc, c, "a list item was expected"));
        }
        if let Some(g) = c.children.iter().find(|g| !g.children.is_empty()) {
            return Err(shape(doc, g, "a nested block in a mix"));
        }
        items.push(
            c.fields()
                .into_iter()
                .map(|(key, value)| Field { key, value })
                .collect(),
        );
    }
    Ok(Some((n, items)))
}

/// The model's header name.
pub fn model_name(doc: &Doc) -> Result<Option<String>, Refusal> {
    Ok(doc
        .find(&["header", "name"])?
        .map(|n| unquote(&n.value).to_string()))
}

/// Reads the typed view. Refuses a block it cannot read.
pub fn view(file: &str, doc: &Doc) -> Result<ModelView, Refusal> {
    let header = doc.top("header")?;
    let name = model_name(doc)?.unwrap_or_default();
    let mut model_ids = Vec::new();
    if let Some(m) = header.as_ref().and_then(|h| h.child("modelId")) {
        for (module, f) in indexed(doc, m, false)? {
            model_ids.push(ModuleId {
                module,
                id: get(&f, "val").parse().unwrap_or(0),
            });
        }
    }
    let mut timers = Vec::new();
    if let Some(t) = doc.top("timers")? {
        for (index, fields) in indexed(doc, &t, false)? {
            timers.push(Timer {
                index,
                name: unquote(get(&fields, "name")).into(),
                swtch: unquote(get(&fields, "swtch")).into(),
                mode: get(&fields, "mode").into(),
                value: get(&fields, "value").parse().unwrap_or(0),
                persistent: get(&fields, "persistent").into(),
                fields,
            });
        }
    }
    let mixes = mix_items(doc)?
        .map(|(_, items)| {
            items
                .into_iter()
                .map(|fields| Mix {
                    dest_ch: get(&fields, "destCh").parse().unwrap_or(0),
                    source: unquote(get(&fields, "srcRaw")).into(),
                    weight: get(&fields, "weight").into(),
                    swtch: unquote(get(&fields, "swtch")).into(),
                    mltpx: get(&fields, "mltpx").into(),
                    fields,
                })
                .collect()
        })
        .unwrap_or_default();
    let mut logical_switches = Vec::new();
    if let Some(n) = doc.top("logicalSw")? {
        for (index, f) in indexed(doc, &n, false)? {
            logical_switches.push(LogicalSwitch {
                index,
                func: get(&f, "func").into(),
                def: unquote(get(&f, "def")).into(),
                andsw: unquote(get(&f, "andsw")).into(),
                delay: get(&f, "delay").parse().unwrap_or(0),
                duration: get(&f, "duration").parse().unwrap_or(0),
            });
        }
    }
    let special_functions = sf_items(doc)?
        .into_iter()
        .map(|(index, f)| SpecialFunction {
            index,
            swtch: unquote(get(&f, "swtch")).into(),
            func: get(&f, "func").into(),
            def: unquote(get(&f, "def")).into(),
        })
        .collect();
    let mut switch_warnings = Vec::new();
    if let Some(n) = doc.top("switchWarning")? {
        for c in &n.children {
            switch_warnings.push(SwitchWarning {
                switch: c.key.clone().unwrap_or_default(),
                pos: c.get("pos").unwrap_or("").into(),
            });
        }
    }
    let sensors = sensor_slots(doc)?
        .into_iter()
        .map(|(label, slot)| Sensor { slot, label })
        .collect::<Vec<_>>();
    let mut sensors = sensors;
    sensors.sort_by_key(|s| s.slot);
    let mut screens = Vec::new();
    if let Some(n) = doc.top("screens")? {
        for c in &n.children {
            let Some(index) = c.index() else {
                return Err(shape(doc, c, "an index was expected"));
            };
            screens.push(Screen {
                index,
                kind: c.get("type").unwrap_or("").into(),
                script: c
                    .path(&["u", "script", "file"])
                    .map(|f| unquote(&f.value).to_string()),
            });
        }
    }
    Ok(ModelView {
        file: file.into(),
        name,
        model_ids,
        timers,
        mixes,
        logical_switches,
        special_functions,
        switch_warnings,
        legacy_switch_warning: doc.top("switchWarningState")?.is_some(),
        sensors,
        screens,
        checklist: doc.top_value("displayChecklist").as_deref() == Some("1"),
        checklist_interactive: doc.top_value("checklistInteractive").as_deref() == Some("1"),
    })
}

fn sf_items(doc: &Doc) -> Result<Vec<(u32, Vec<Field>)>, Refusal> {
    match doc.top("customFn")? {
        Some(n) => indexed(doc, &n, false),
        None => Ok(Vec::new()),
    }
}

// ----- operations -----

/// A mix line an op wants on a channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct MixLine {
    /// `SA`, `MAX`, `I2`, ...
    pub source: String,
    pub weight: i32,
    /// `NONE` for always.
    pub swtch: String,
    /// `ADD`, `MUL` or `REPL`.
    pub mltpx: String,
}

/// A logical switch an op sets. `def` may name sensors by label: `tele({RxBt}),35`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct LsDef {
    /// `FUNC_AND`, `FUNC_VPOS`, `FUNC_EDGE`, `FUNC_STICKY`, ...
    pub func: String,
    pub def: String,
    #[serde(default = "none")]
    pub andsw: String,
    /// 0.1 s.
    #[serde(default)]
    pub delay: u32,
    /// 0.1 s.
    #[serde(default)]
    pub duration: u32,
}

fn none() -> String {
    "NONE".into()
}

/// A special function as an op names it (values unquoted). `def` may name sensors by
/// label.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct SfDef {
    pub swtch: String,
    pub func: String,
    pub def: String,
}

/// One edit to a model file. Ops apply in order; a later op sees the earlier ones.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum ModelOp {
    /// The header name (15 characters at most).
    Rename {
        name: String,
    },
    /// A module's model id (index 0 is the internal module); 0 removes it.
    SetModelId {
        module: u32,
        id: u32,
    },
    /// Top-level values the file already has (`disableTelemetryWarning`, ...). Blocks
    /// and the header are not values.
    SetFlags {
        flags: Vec<Field>,
    },
    /// The power-on checklist on or off (`displayChecklist`, `checklistInteractive`).
    SetChecklist {
        enabled: bool,
    },
    /// The exact mix lines of one channel (0 is CH1); empty removes the channel's mixes.
    SetMixes {
        channel: u32,
        lines: Vec<MixLine>,
    },
    /// One logical switch (0 is L1); None empties it.
    SetLogicalSwitch {
        index: u32,
        ls: Option<LsDef>,
    },
    /// Removes these special functions (where present) and appends these (where absent).
    /// A change records what it added, so a later change can remove them.
    SpecialFunctions {
        remove: Vec<SfDef>,
        add: Vec<SfDef>,
    },
    /// Moves a special function to another switch, keeping its place.
    MoveSpecialFunction {
        from: SfDef,
        swtch: String,
    },
    /// Fields of one timer (0 is Timer 1), unquoted. The stored `value` is never set.
    SetTimer {
        index: u32,
        fields: Vec<Field>,
    },
    RemoveTimer {
        index: u32,
    },
    /// Trades two timers and every `TmrN` reference to them.
    SwapTimers {
        a: u32,
        b: u32,
    },
    /// The switch warnings, as the 2.12 list; replaces a legacy `switchWarningState:`.
    SetSwitchWarnings {
        warnings: Vec<SwitchWarning>,
    },
    /// A telemetry screen (0 is screen 1): a script screen, or None to remove it.
    SetScreen {
        index: u32,
        script: Option<String>,
    },
}

/// Applies ops in order. Refuses with the first problem; the doc is then unusable.
pub fn apply(doc: &mut Doc, ops: &[ModelOp]) -> Result<(), Refusal> {
    for op in ops {
        apply_one(doc, op)?;
    }
    Ok(())
}

fn apply_one(doc: &mut Doc, op: &ModelOp) -> Result<(), Refusal> {
    match op {
        ModelOp::Rename { name } => rename(doc, name),
        ModelOp::SetModelId { module, id } => set_model_id(doc, *module, *id),
        ModelOp::SetFlags { flags } => {
            for f in flags {
                if PROTECTED_KEYS.contains(&f.key.as_str()) {
                    return Err(bad(format!(
                        "{} is not a model value QuadCam sets this way.",
                        f.key
                    )));
                }
                doc.set_top_scalar(&f.key, &f.value)?;
            }
            Ok(())
        }
        ModelOp::SetChecklist { enabled } => {
            let v = if *enabled { "1" } else { "0" };
            doc.set_top_scalar("displayChecklist", v)?;
            doc.set_top_scalar("checklistInteractive", v)?;
            Ok(())
        }
        ModelOp::SetMixes { channel, lines } => set_mixes(doc, *channel, lines),
        ModelOp::SetLogicalSwitch { index, ls } => set_ls(doc, *index, ls.as_ref()),
        ModelOp::SpecialFunctions { remove, add } => special_functions(doc, remove, add),
        ModelOp::MoveSpecialFunction { from, swtch } => move_sf(doc, from, swtch),
        ModelOp::SetTimer { index, fields } => set_timer(doc, *index, fields),
        ModelOp::RemoveTimer { index } => {
            let Some(t) = doc.top("timers")? else {
                return Ok(());
            };
            if let Some(c) = t.children.iter().find(|c| c.index() == Some(*index)) {
                doc.splice(c.span(), vec![]);
            }
            Ok(())
        }
        ModelOp::SwapTimers { a, b } => swap_timers(doc, *a, *b),
        ModelOp::SetSwitchWarnings { warnings } => set_switch_warnings(doc, warnings),
        ModelOp::SetScreen { index, script } => set_screen(doc, *index, script.as_deref()),
    }
}

/// Keys `SetFlags` never writes: the blocks and values other ops own.
pub const PROTECTED_KEYS: &[&str] = &[
    "semver",
    "checksum",
    "header",
    "timers",
    "mixData",
    "logicalSw",
    "customFn",
    "switchWarning",
    "switchWarningState",
    "screens",
    "telemetrySensors",
    "moduleData",
    "modelRegistrationID",
];

fn rename(doc: &mut Doc, name: &str) -> Result<(), Refusal> {
    check_text("A model name", name, MAX_MODEL_NAME)?;
    let n = doc.find(&["header", "name"])?.ok_or_else(|| {
        bad(format!(
            "{} has no header name; nothing was written.",
            doc.name
        ))
    })?;
    doc.set_line(
        n.line,
        format!("{}name: {}", " ".repeat(n.indent), quote(name)),
    );
    Ok(())
}

fn set_model_id(doc: &mut Doc, module: u32, id: u32) -> Result<(), Refusal> {
    if module > 3 || id > 63 {
        return Err(bad("A model id is 0-63, on module 0-3."));
    }
    let header = doc
        .top("header")?
        .ok_or_else(|| bad(format!("{} has no header; nothing was written.", doc.name)))?;
    let pad = header.children.first().map(|c| c.indent).unwrap_or(3);
    let mut ids: BTreeMap<u32, u32> = BTreeMap::new();
    let existing = header.child("modelId").cloned();
    if let Some(m) = &existing {
        for (k, f) in indexed(doc, m, false)? {
            ids.insert(k, get(&f, "val").parse().unwrap_or(0));
        }
    }
    ids.insert(module, id);
    ids.retain(|_, v| *v != 0);
    let mut lines = Vec::new();
    if !ids.is_empty() {
        lines.push(format!("{}modelId: ", " ".repeat(pad)));
        for (k, v) in &ids {
            lines.push(format!("{}{k}:", " ".repeat(pad + 3)));
            lines.push(format!("{}val: {v}", " ".repeat(pad + 6)));
        }
    }
    let range = match &existing {
        Some(m) => m.span(),
        None => header.end..header.end,
    };
    if doc.slice(range.clone()) != lines.as_slice() {
        doc.splice(range, lines);
    }
    Ok(())
}

const MIX_REQUIRED: &[&str] = &["destCh", "srcRaw", "weight", "swtch", "mltpx"];
/// Values a new mix takes for fields the file's layout has.
const MIX_NEW: &[(&str, &str)] = &[
    ("carryTrim", "0"),
    ("mixWarn", "0"),
    ("delayPrec", "0"),
    ("speedPrec", "0"),
    ("flightModes", "000000000"),
    ("offset", "0"),
    ("delayUp", "0"),
    ("delayDown", "0"),
    ("speedUp", "0"),
    ("speedDown", "0"),
    ("name", "\"\""),
];

fn set_mixes(doc: &mut Doc, channel: u32, want: &[MixLine]) -> Result<(), Refusal> {
    if channel >= 32 {
        return Err(bad("A channel is 0-31 (CH1-CH32)."));
    }
    for l in want {
        if !["ADD", "MUL", "REPL"].contains(&l.mltpx.as_str()) {
            return Err(bad(format!(
                "Mix multiplex {:?} is not ADD, MUL or REPL.",
                l.mltpx
            )));
        }
        if !(-500..=500).contains(&l.weight) {
            return Err(bad("A mix weight is -500..500."));
        }
    }
    let (node, items) = mix_items(doc)?
        .ok_or_else(|| bad(format!("{} has no mixData; nothing was written.", doc.name)))?;
    let Some(first) = items.first() else {
        return Err(bad(format!(
            "{} has no mixes to copy a layout from; nothing was written.",
            doc.name
        )));
    };
    // The layout and quoting of the file's own first entry (2.10 hand edits put weight
    // first and leave srcRaw unquoted; 2.12 saves put destCh first and quote it).
    let layout: Vec<String> = first.iter().map(|f| f.key.clone()).collect();
    for (i, it) in items.iter().enumerate() {
        if it.iter().map(|f| &f.key).ne(layout.iter()) {
            return Err(shape_refusal(
                &doc.name,
                node.children[i].line + 1,
                "mixes do not share one field layout",
            ));
        }
    }
    if let Some(k) = MIX_REQUIRED
        .iter()
        .find(|k| !layout.iter().any(|l| l == *k))
    {
        return Err(shape_refusal(
            &doc.name,
            node.line + 1,
            &format!("mixes lack {k}"),
        ));
    }
    let src_quoted = get(first, "srcRaw").starts_with('"');
    let have: Vec<&Vec<Field>> = items
        .iter()
        .filter(|f| get(f, "destCh") == channel.to_string())
        .collect();
    let got: Vec<MixLine> = have
        .iter()
        .map(|f| MixLine {
            source: unquote(get(f, "srcRaw")).into(),
            weight: get(f, "weight").parse().unwrap_or(i32::MIN),
            swtch: unquote(get(f, "swtch")).into(),
            mltpx: get(f, "mltpx").into(),
        })
        .collect();
    if got == want {
        return Ok(());
    }
    let mut base: Vec<Field> = match have.first() {
        Some(h) => (*h).clone(),
        None => {
            let mut b = items.last().unwrap().clone();
            for f in &mut b {
                if let Some((_, v)) = MIX_NEW.iter().find(|(k, _)| *k == f.key) {
                    f.value = (*v).into();
                }
            }
            b
        }
    };
    let set = |b: &mut Vec<Field>, k: &str, v: String| {
        if let Some(f) = b.iter_mut().find(|f| f.key == k) {
            f.value = v;
        }
    };
    set(&mut base, "destCh", channel.to_string());
    set(&mut base, "offset", "0".into());
    set(&mut base, "name", "\"\"".into());
    let new: Vec<Vec<Field>> = want
        .iter()
        .map(|l| {
            let mut d = base.clone();
            let src = if src_quoted {
                quote(&l.source)
            } else {
                l.source.clone()
            };
            set(&mut d, "srcRaw", src);
            set(&mut d, "weight", l.weight.to_string());
            set(&mut d, "swtch", quote(&l.swtch));
            set(&mut d, "mltpx", l.mltpx.clone());
            d
        })
        .collect();
    let mut all: Vec<Vec<Field>> = items
        .into_iter()
        .filter(|f| get(f, "destCh") != channel.to_string())
        .collect();
    all.extend(new);
    // Stable: keeps the order inside a channel.
    all.sort_by_key(|f| get(f, "destCh").parse::<u32>().unwrap_or(u32::MAX));
    let (dash, field) = node
        .children
        .first()
        .map(|c| {
            (
                c.indent,
                c.children.first().map(|g| g.indent).unwrap_or(c.indent + 2),
            )
        })
        .unwrap_or((1, 3));
    let mut lines = Vec::new();
    for it in all {
        lines.push(format!("{}-", " ".repeat(dash)));
        for f in it {
            lines.push(format!("{}{}: {}", " ".repeat(field), f.key, f.value));
        }
    }
    doc.splice(node.body(), lines);
    Ok(())
}

const LS_FIELDS: &[&str] = &[
    "func",
    "def",
    "andsw",
    "lsPersist",
    "lsState",
    "delay",
    "duration",
];

fn check_comparison(doc: &Doc, func: &str, def: &str) -> Result<(), Refusal> {
    if !matches!(
        func,
        "FUNC_VPOS" | "FUNC_VNEG" | "FUNC_APOS" | "FUNC_ANEG" | "FUNC_VEQUAL" | "FUNC_VALMOSTEQUAL"
    ) {
        return Ok(());
    }
    let mut parts = def.split(',');
    let (Some(src), Some(v)) = (parts.next(), parts.next()) else {
        return Err(bad(format!(
            "{func} takes \"source,value\", not {def:?} ({}).",
            doc.name
        )));
    };
    if STICKS.contains(&src) {
        let n: i32 = v
            .trim()
            .parse()
            .map_err(|_| bad(format!("{v:?} is not a number.")))?;
        if !(-100..=100).contains(&n) {
            return Err(bad(format!(
                "A stick compares in -100..100; {src} {n} is out of range."
            )));
        }
    }
    Ok(())
}

fn set_ls(doc: &mut Doc, index: u32, ls: Option<&LsDef>) -> Result<(), Refusal> {
    if index >= MAX_LOGICAL_SWITCHES {
        return Err(bad(format!(
            "Logical switches are L1-L{MAX_LOGICAL_SWITCHES}."
        )));
    }
    let node = doc.top("logicalSw")?.ok_or_else(|| {
        bad(format!(
            "{} has no logicalSw block; nothing was written.",
            doc.name
        ))
    })?;
    let items = indexed(doc, &node, false)?;
    for (i, (_, f)) in items.iter().enumerate() {
        if f.iter()
            .map(|x| x.key.as_str())
            .ne(LS_FIELDS.iter().copied())
        {
            return Err(shape_refusal(
                &doc.name,
                node.children[i].line + 1,
                "a logical switch with another field layout",
            ));
        }
    }
    let (ipad, fpad) = node
        .children
        .first()
        .map(|c| {
            (
                c.indent,
                c.children.first().map(|g| g.indent).unwrap_or(c.indent + 3),
            )
        })
        .unwrap_or((3, 6));
    let new = match ls {
        None => vec![],
        Some(l) => {
            let def = resolve_sensors(doc, &l.def)?;
            check_comparison(doc, &l.func, &def)?;
            if !l.func.starts_with("FUNC_") {
                return Err(bad(format!(
                    "{:?} is not a logical switch function.",
                    l.func
                )));
            }
            let old = items.iter().find(|(i, _)| *i == index).map(|(_, f)| f);
            let keep = |k: &str| old.map(|f| get(f, k).to_string()).unwrap_or("0".into());
            let p = " ".repeat(fpad);
            vec![
                format!("{}{index}:", " ".repeat(ipad)),
                format!("{p}func: {}", l.func),
                format!("{p}def: {}", quote(&def)),
                format!("{p}andsw: {}", quote(&l.andsw)),
                format!("{p}lsPersist: {}", keep("lsPersist")),
                format!("{p}lsState: {}", keep("lsState")),
                format!("{p}delay: {}", l.delay),
                format!("{p}duration: {}", l.duration),
            ]
        }
    };
    let range = match node.children.iter().find(|c| c.index() == Some(index)) {
        Some(c) => c.span(),
        None => {
            // Keep index order: before the first higher index.
            let at = node
                .children
                .iter()
                .find(|c| c.index().is_some_and(|i| i > index))
                .map(|c| c.line)
                .unwrap_or(node.end);
            at..at
        }
    };
    if doc.slice(range.clone()) != new.as_slice() {
        doc.splice(range, new);
    }
    Ok(())
}

/// Checks one special function an op adds.
fn check_sf(doc: &Doc, sf: &SfDef) -> Result<(), Refusal> {
    let fields: Vec<&str> = sf.def.split(',').collect();
    match sf.func.as_str() {
        "PLAY_TRACK" => {
            let name = fields[0];
            if name.is_empty() || name.len() > MAX_TRACK_NAME {
                return Err(bad(format!(
                    "Track {name:?}: a track name is 1-{MAX_TRACK_NAME} characters."
                )));
            }
            if let Some(rep) = fields.get(2) {
                let ok = *rep == "1x" || *rep == "!1x" || rep.parse::<u32>().is_ok();
                if !ok {
                    return Err(bad(format!(
                        "Repeat {rep:?}: use 1x, !1x (once, not at power-on) or seconds."
                    )));
                }
            }
        }
        "SET_SCREEN" => {
            // It repeats while its switch is on, so it belongs on an edge pulse.
            let edge = ls_index(&sf.swtch).is_some_and(|i| {
                view(&doc.name, doc).is_ok_and(|v| {
                    v.logical_switches
                        .iter()
                        .any(|l| l.index == i && l.func == "FUNC_EDGE")
                })
            });
            if !edge || sf.swtch.starts_with('!') {
                return Err(bad(format!(
                    "SET_SCREEN on {} would repeat while it is on; put it on a FUNC_EDGE logical switch.",
                    sf.swtch
                )));
            }
        }
        _ => {}
    }
    if sf.swtch.is_empty() || sf.func.is_empty() {
        return Err(bad("A special function needs a switch and a function."));
    }
    Ok(())
}

fn sf_lines(doc: &Doc, node: Option<&Node>, items: &[Vec<Field>]) -> Vec<String> {
    let (ipad, fpad) = node
        .and_then(|n| n.children.first())
        .map(|c| {
            (
                c.indent,
                c.children.first().map(|g| g.indent).unwrap_or(c.indent + 3),
            )
        })
        .unwrap_or((3, 6));
    let _ = doc;
    let mut out = Vec::new();
    for (i, it) in items.iter().enumerate() {
        out.push(format!("{}{i}:", " ".repeat(ipad)));
        for f in it {
            out.push(format!("{}{}: {}", " ".repeat(fpad), f.key, f.value));
        }
    }
    out
}

fn sf_fields(doc: &Doc, sf: &SfDef) -> Result<Vec<Field>, Refusal> {
    Ok(vec![
        Field::new("swtch", &quote(&sf.swtch)),
        Field::new("func", &sf.func),
        Field::new("def", &quote(&resolve_sensors(doc, &sf.def)?)),
    ])
}

fn same_sf(f: &[Field], want: &[Field]) -> bool {
    want.iter().all(|w| get(f, &w.key) == w.value)
}

fn special_functions(doc: &mut Doc, remove: &[SfDef], add: &[SfDef]) -> Result<(), Refusal> {
    let node = doc.top("customFn")?;
    let mut items: Vec<Vec<Field>> = sf_items(doc)?.into_iter().map(|(_, f)| f).collect();
    for r in remove {
        let want = sf_fields(doc, r)?;
        items.retain(|f| !same_sf(f, &want));
    }
    for a in add {
        check_sf(doc, a)?;
        let want = sf_fields(doc, a)?;
        if !items.iter().any(|f| same_sf(f, &want)) {
            items.push(want);
        }
    }
    if items.len() > MAX_SPECIAL_FUNCTIONS {
        return Err(bad(format!(
            "A model holds {MAX_SPECIAL_FUNCTIONS} special functions at most; this would make {}.",
            items.len()
        )));
    }
    write_sfs(doc, node, &items)
}

fn write_sfs(doc: &mut Doc, node: Option<Node>, items: &[Vec<Field>]) -> Result<(), Refusal> {
    let body = sf_lines(doc, node.as_ref(), items);
    match node {
        Some(n) => {
            if doc.slice(n.body()) != body.as_slice() {
                doc.splice(n.body(), body);
            }
        }
        None if items.is_empty() => {}
        None => {
            let at = doc
                .top("flightModeData")?
                .or(doc.top("thrTraceSrc")?)
                .map(|n| n.line)
                .ok_or_else(|| {
                    bad(format!(
                        "{} has no customFn block and no place for one; nothing was written.",
                        doc.name
                    ))
                })?;
            let mut lines = vec!["customFn: ".to_string()];
            lines.extend(body);
            doc.splice(at..at, lines);
        }
    }
    Ok(())
}

fn move_sf(doc: &mut Doc, from: &SfDef, swtch: &str) -> Result<(), Refusal> {
    let node = doc.top("customFn")?;
    let mut items: Vec<Vec<Field>> = sf_items(doc)?.into_iter().map(|(_, f)| f).collect();
    let want = sf_fields(doc, from)?;
    for it in items.iter_mut().filter(|f| same_sf(f, &want)) {
        if let Some(f) = it.iter_mut().find(|f| f.key == "swtch") {
            f.value = quote(swtch);
        }
    }
    write_sfs(doc, node, &items)
}

/// Timer fields in the order EdgeTX 2.12 writes them, with a new timer's values.
const TIMER_NEW: &[(&str, &str)] = &[
    ("start", "0"),
    ("swtch", "\"NONE\""),
    ("value", "0"),
    ("mode", "OFF"),
    ("countdownBeep", "0"),
    ("minuteBeep", "0"),
    ("persistent", "0"),
    ("countdownStart", "0"),
    ("showElapsed", "0"),
    ("extraHaptic", "0"),
    ("name", "\"\""),
];
const TIMER_QUOTED: &[&str] = &["swtch", "name"];

fn timers_node(doc: &Doc) -> Result<Node, Refusal> {
    doc.top("timers")?.ok_or_else(|| {
        bad(format!(
            "{} has no timers block; nothing was written.",
            doc.name
        ))
    })
}

fn render_indexed(n: &Node, items: &BTreeMap<u32, Vec<Field>>) -> Vec<String> {
    let (ipad, fpad) = n
        .children
        .first()
        .map(|c| {
            (
                c.indent,
                c.children.first().map(|g| g.indent).unwrap_or(c.indent + 3),
            )
        })
        .unwrap_or((3, 6));
    let mut out = Vec::new();
    for (i, f) in items {
        out.push(format!("{}{i}:", " ".repeat(ipad)));
        for x in f {
            out.push(format!("{}{}: {}", " ".repeat(fpad), x.key, x.value));
        }
    }
    out
}

fn set_timer(doc: &mut Doc, index: u32, fields: &[Field]) -> Result<(), Refusal> {
    if index >= MAX_TIMERS {
        return Err(bad(format!("Timers are 1-{MAX_TIMERS}.")));
    }
    let n = timers_node(doc)?;
    let mut items: BTreeMap<u32, Vec<Field>> = indexed(doc, &n, false)?.into_iter().collect();
    let layout: Vec<Field> = items
        .values()
        .next()
        .map(|f| {
            f.iter()
                .map(|x| {
                    let v = TIMER_NEW
                        .iter()
                        .find(|(k, _)| *k == x.key)
                        .map(|(_, v)| *v)
                        .unwrap_or("0");
                    Field::new(&x.key, v)
                })
                .collect()
        })
        .unwrap_or_else(|| TIMER_NEW.iter().map(|(k, v)| Field::new(k, v)).collect());
    let t = items.entry(index).or_insert(layout);
    for f in fields {
        if f.key == "value" {
            return Err(bad(
                "A timer's stored value is never set; QuadCam keeps it.",
            ));
        }
        if f.key == "name" {
            check_text("A timer name", &f.value, 8)?;
        }
        let v = if TIMER_QUOTED.contains(&f.key.as_str()) {
            quote(&f.value)
        } else {
            f.value.clone()
        };
        match t.iter_mut().find(|x| x.key == f.key) {
            Some(x) => x.value = v,
            None => {
                return Err(bad(format!(
                    "Timers in {} have no {} field.",
                    doc.name, f.key
                )));
            }
        }
    }
    let body = render_indexed(&n, &items);
    if doc.slice(n.body()) != body.as_slice() {
        doc.splice(n.body(), body);
    }
    Ok(())
}

/// Replaces whole `TmrA` and `TmrB` tokens with each other.
pub fn swap_tmr_refs(line: &str, a: u32, b: u32) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < line.len() {
        if line[i..].starts_with("Tmr")
            && (i == 0 || !(bytes[i - 1] as char).is_ascii_alphanumeric())
        {
            let digits = line[i + 3..]
                .bytes()
                .take_while(|c| c.is_ascii_digit())
                .count();
            let after = i + 3 + digits;
            let ends = after == line.len() || !(bytes[after] as char).is_ascii_alphanumeric();
            if digits > 0 && ends {
                let n: u32 = line[i + 3..after].parse().unwrap_or(0);
                let m = if n == a + 1 {
                    b + 1
                } else if n == b + 1 {
                    a + 1
                } else {
                    n
                };
                out.push_str(&format!("Tmr{m}"));
                i = after;
                continue;
            }
        }
        let c = line[i..].chars().next().unwrap();
        out.push(c);
        i += c.len_utf8();
    }
    out
}

fn swap_timers(doc: &mut Doc, a: u32, b: u32) -> Result<(), Refusal> {
    if a >= MAX_TIMERS || b >= MAX_TIMERS || a == b {
        return Err(bad(format!("Swap two different timers, 1-{MAX_TIMERS}.")));
    }
    let n = timers_node(doc)?;
    let mut items: BTreeMap<u32, Vec<Field>> = indexed(doc, &n, false)?.into_iter().collect();
    let ta = items.remove(&a);
    let tb = items.remove(&b);
    if let Some(t) = ta {
        items.insert(b, t);
    }
    if let Some(t) = tb {
        items.insert(a, t);
    }
    let body = render_indexed(&n, &items);
    let span = n.span();
    for i in 0..doc.lines().len() {
        if span.contains(&i) {
            continue;
        }
        let s = swap_tmr_refs(&doc.lines()[i], a, b);
        doc.set_line(i, s);
    }
    doc.splice(n.body(), body);
    Ok(())
}

fn set_switch_warnings(doc: &mut Doc, want: &[SwitchWarning]) -> Result<(), Refusal> {
    for w in want {
        let b = w.switch.as_bytes();
        if b.len() != 2 || b[0] != b'S' || !b[1].is_ascii_uppercase() {
            return Err(bad(format!(
                "{:?} is not a switch (SA, SB, ...).",
                w.switch
            )));
        }
        if !["up", "mid", "down"].contains(&w.pos.as_str()) {
            return Err(bad(format!(
                "Warning position {:?} is not up, mid or down.",
                w.pos
            )));
        }
    }
    let mut body = Vec::new();
    for w in want {
        body.push(format!("   {}:", w.switch));
        body.push(format!("      pos: {}", w.pos));
    }
    let mut block = vec!["switchWarning: ".to_string()];
    block.extend(body.iter().cloned());
    if want.is_empty() {
        block.clear();
    }
    // Never keep the legacy line: EdgeTX 2.12.4 misreads it.
    let legacy = doc.top("switchWarningState")?;
    if let Some(n) = doc.top("switchWarning")? {
        if let Some(l) = &legacy {
            doc.splice(l.span(), vec![]);
        }
        let n = doc.top("switchWarning")?.unwrap_or(n);
        if doc.slice(n.span()) != block.as_slice() {
            doc.splice(n.span(), block);
        }
        return Ok(());
    }
    if let Some(l) = legacy {
        doc.splice(l.span(), block);
        return Ok(());
    }
    if block.is_empty() {
        return Ok(());
    }
    let at = doc
        .top("rssiSource")?
        .map(|n| n.line)
        .ok_or_else(|| {
            bad(format!(
                "{} has no switchWarning block and no rssiSource line to place one before; nothing was written.",
                doc.name
            ))
        })?;
    doc.splice(at..at, block);
    Ok(())
}

fn set_screen(doc: &mut Doc, index: u32, script: Option<&str>) -> Result<(), Refusal> {
    if index > 3 {
        return Err(bad("Telemetry screens are 1-4."));
    }
    let n = doc.top("screens")?.ok_or_else(|| {
        bad(format!(
            "{} has no screens block; nothing was written.",
            doc.name
        ))
    })?;
    let existing = n
        .children
        .iter()
        .find(|c| c.index() == Some(index))
        .cloned();
    let new = match script {
        None => vec![],
        Some(s) => {
            if s.is_empty() || s.len() > MAX_SCRIPT_NAME {
                return Err(bad(format!(
                    "Script {s:?}: a telemetry script name is 1-{MAX_SCRIPT_NAME} characters."
                )));
            }
            check_text("A script name", s, MAX_SCRIPT_NAME)?;
            vec![
                format!("   {index}:"),
                "      type: SCRIPT".into(),
                "      u: ".into(),
                "         script: ".into(),
                format!("            file: {}", quote(s)),
            ]
        }
    };
    let range = match &existing {
        Some(c) => c.span(),
        None => {
            if new.is_empty() {
                return Ok(());
            }
            if index > 0 && !n.children.iter().any(|c| c.index() == Some(index - 1)) {
                return Err(bad(format!(
                    "Telemetry screen {index} is missing; screen {} cannot come after it.",
                    index + 1
                )));
            }
            let at = n
                .children
                .iter()
                .find(|c| c.index().is_some_and(|i| i > index))
                .map(|c| c.line)
                .unwrap_or(n.end);
            at..at
        }
    };
    if doc.slice(range.clone()) != new.as_slice() {
        doc.splice(range, new);
    }
    Ok(())
}
