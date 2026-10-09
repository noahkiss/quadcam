//! The model editors' ops and reads (design 6.3, WP9): logging, sensor logging, RSSI
//! alarms, spoken callouts, value screens, and the checks the timer editor shares. Each op
//! edits lines through `yaml::Doc` and leaves every other line as it was; each is
//! idempotent.
//!
//! Encodings this module keeps (each has a test):
//!
//! - `LOGS` special function: `def` is `"<period>,1"`, the period in 0.1 s.
//! - A callout is a `PLAY_TRACK` special function. Its track name owns it: a later
//!   `SetCallout` with that track replaces it, and the logical switch it made goes with
//!   it when nothing else uses that switch.
//! - A battery or sensor callout compares a sensor with `FUNC_VNEG` (below) or
//!   `FUNC_VPOS` (above). The value is typed as the sensor shows it (`3.5`), stored in the
//!   sensor's precision (`35` at precision 1).
//! - Value screens: `type: VALUES`, up to 4 lines of up to 3 sources; `{Label}` becomes the
//!   sensor's `tele(N)` slot.

use super::model::{
    bad, check_text, get, ls_index, ls_name, resolve_sensors, set_ls, sf_fields, sf_items,
    write_sfs, Field, LsDef, SfDef, MAX_LOGICAL_SWITCHES, MAX_SPECIAL_FUNCTIONS, MAX_TRACK_NAME,
};
use super::yaml::{unquote, Doc};
use crate::gear::model::Refusal;
use serde::{Deserialize, Serialize};
use specta::Type;

/// Lines on a value screen, and sources on a line.
pub const SCREEN_LINES: usize = 4;
pub const SCREEN_SOURCES: usize = 3;
/// The longest logging period, in 0.1 s.
pub const MAX_LOG_PERIOD: u32 = 255;

// ----- types -----

/// The switch that writes the radio's log, and how often.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct LoggingDef {
    /// `ON`, `SA2`, `L3`, ...
    pub swtch: String,
    /// 0.1 s.
    pub period_ds: u32,
}

/// Whether the radio's log records one telemetry sensor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct SensorLog {
    pub label: String,
    pub logs: bool,
}

/// When a callout plays.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case", tag = "when")]
pub enum CalloutWhen {
    /// While a switch is on.
    Switch { swtch: String },
    /// While a source reads below a value. `source` is `{RxBt}` or any source name.
    Below {
        source: String,
        value: String,
        #[serde(default)]
        delay_ds: u32,
    },
    /// While a source reads above a value.
    Above {
        source: String,
        value: String,
        #[serde(default)]
        delay_ds: u32,
    },
}

/// A spoken callout an op sets.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct CalloutDef {
    /// The track name (the file stem in `SOUNDS/<lang>/`, 8 characters at most). Owns the
    /// callout.
    pub track: String,
    #[serde(flatten)]
    pub when: CalloutWhen,
    /// `1x` (once), `!1x` (once, not at power-on) or seconds between repeats. Default `1x`.
    #[serde(default)]
    pub repeat: Option<String>,
}

/// A callout as the file holds it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct CalloutView {
    pub track: String,
    pub swtch: String,
    pub repeat: String,
    /// The condition, when it is one this editor writes (a switch, or a sensor against a
    /// value); None for anything else, which the editor shows but does not rewrite.
    #[serde(default)]
    pub when: Option<CalloutWhen>,
}

/// The radio's logging, as the file holds it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct LoggingView {
    /// The `LOGS` function, when there is one.
    #[serde(default)]
    pub logging: Option<LoggingDef>,
    /// Every telemetry sensor, with whether the log records it.
    pub sensors: Vec<SensorLog>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct RfAlarms {
    pub warning: u32,
    pub critical: u32,
}

/// A telemetry screen, in full.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct ScreenDetail {
    pub index: u32,
    pub kind: String,
    #[serde(default)]
    pub script: Option<String>,
    /// For a `VALUES` screen: each line's sources, as the file writes them (`tele(2)`,
    /// `Tmr1`), and the same with sensor labels: `labels`.
    pub lines: Vec<Vec<String>>,
    pub labels: Vec<Vec<String>>,
}

/// What the editors show of a model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct EditorView {
    pub logging: LoggingView,
    #[serde(default)]
    pub rf_alarms: Option<RfAlarms>,
    pub callouts: Vec<CalloutView>,
    pub screens: Vec<ScreenDetail>,
}

// ----- timers -----

/// Checks one timer field's value against what EdgeTX accepts.
pub fn check_timer_field(key: &str, value: &str) -> Result<(), Refusal> {
    let num = |max: u32| -> Result<(), Refusal> {
        match value.parse::<u32>() {
            Ok(n) if n <= max => Ok(()),
            _ => Err(bad(format!("Timer {key} takes 0-{max}, not {value:?}."))),
        }
    };
    match key {
        "minuteBeep" | "showElapsed" | "extraHaptic" => num(1),
        "countdownBeep" => num(3),
        "persistent" => num(2),
        "countdownStart" => match value.parse::<i32>() {
            Ok(n) if (-1..=3).contains(&n) => Ok(()),
            _ => Err(bad(format!(
                "Timer countdownStart takes -1 to 3, not {value:?}."
            ))),
        },
        "mode" => {
            if !value.is_empty()
                && value
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
            {
                Ok(())
            } else {
                Err(bad(format!(
                    "Timer mode {value:?} is not a mode name (OFF, ON, START, THR, ...)."
                )))
            }
        }
        _ => Ok(()),
    }
}

// ----- checklist -----

/// The most lines a checklist file holds.
pub const MAX_CHECKLIST_LINES: usize = 99;

/// Checks checklist text: Latin-1, at most the lines the radio reads, and each line within
/// the screen width when it is known (the `=` of a tick box counts).
pub fn check_checklist(text: &str, width: Option<usize>) -> Result<(), Refusal> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() > MAX_CHECKLIST_LINES {
        return Err(bad(format!(
            "A checklist holds {MAX_CHECKLIST_LINES} lines at most; this has {}.",
            lines.len()
        )));
    }
    for l in lines {
        if let Some(w) = width {
            if l.trim_end().chars().count() > w {
                return Err(bad(format!(
                    "Checklist line {l:?} is over {w} characters, the screen's width."
                )));
            }
        }
        if super::yaml::to_latin1(l).is_none() {
            return Err(bad("A checklist holds Latin-1 characters only."));
        }
    }
    Ok(())
}

// ----- logging -----

fn sf_item_fields(doc: &Doc) -> Result<Vec<Vec<Field>>, Refusal> {
    Ok(sf_items(doc)?.into_iter().map(|(_, f)| f).collect())
}

fn def_head(f: &[Field]) -> String {
    unquote(get(f, "def"))
        .split(',')
        .next()
        .unwrap_or("")
        .to_string()
}

pub(super) fn set_logging(doc: &mut Doc, logging: Option<&LoggingDef>) -> Result<(), Refusal> {
    let node = doc.top("customFn")?;
    let mut items = sf_item_fields(doc)?;
    let first = items.iter().position(|f| get(f, "func") == "LOGS");
    items.retain(|f| get(f, "func") != "LOGS");
    if let Some(l) = logging {
        if l.swtch.is_empty() {
            return Err(bad("Logging needs a switch (ON for always)."));
        }
        if l.period_ds == 0 || l.period_ds > MAX_LOG_PERIOD {
            return Err(bad(format!(
                "A logging period is 0.1-{:.1} s.",
                f64::from(MAX_LOG_PERIOD) / 10.0
            )));
        }
        let want = sf_fields(
            doc,
            &SfDef {
                swtch: l.swtch.clone(),
                func: "LOGS".into(),
                def: format!("{},1", l.period_ds),
            },
        )?;
        let at = first.unwrap_or(items.len()).min(items.len());
        items.insert(at, want);
    }
    if items.len() > MAX_SPECIAL_FUNCTIONS {
        return Err(bad(format!(
            "A model holds {MAX_SPECIAL_FUNCTIONS} special functions at most; this would make {}.",
            items.len()
        )));
    }
    write_sfs(doc, node, &items)
}

pub(super) fn set_sensor_logs(doc: &mut Doc, sensors: &[SensorLog]) -> Result<(), Refusal> {
    for s in sensors {
        let n = doc.top("telemetrySensors")?;
        let child = n
            .as_ref()
            .and_then(|n| {
                n.children
                    .iter()
                    .find(|c| c.get("label").map(unquote) == Some(s.label.as_str()))
            })
            .ok_or_else(|| {
                bad(format!(
                    "{} has no telemetry sensor labelled {:?}; discover sensors on the radio first.",
                    doc.name, s.label
                ))
            })?;
        let l = child.child("logs").ok_or_else(|| {
            bad(format!(
                "Sensor {:?} in {} has no logs field.",
                s.label, doc.name
            ))
        })?;
        let line = l.line;
        let indent = l.indent;
        doc.set_line(
            line,
            format!("{}logs: {}", " ".repeat(indent), u8::from(s.logs)),
        );
    }
    Ok(())
}

// ----- alarms -----

pub(super) fn set_rf_alarms(doc: &mut Doc, warning: u32, critical: u32) -> Result<(), Refusal> {
    if warning == 0 || warning > 127 || critical == 0 || critical > 127 {
        return Err(bad("RSSI alarms take 1-127."));
    }
    if critical > warning {
        return Err(bad(format!(
            "The critical level ({critical}) is over the warning level ({warning}); signal falls from warning to critical."
        )));
    }
    let n = doc.top("rfAlarms")?.ok_or_else(|| {
        bad(format!(
            "{} has no rfAlarms block; nothing was written.",
            doc.name
        ))
    })?;
    for (key, v) in [("warning", warning), ("critical", critical)] {
        let c = n
            .child(key)
            .ok_or_else(|| bad(format!("rfAlarms in {} has no {key} field.", doc.name)))?;
        let (line, indent) = (c.line, c.indent);
        doc.set_line(line, format!("{}{key}: {v}", " ".repeat(indent)));
    }
    Ok(())
}

// ----- value screens -----

pub(super) fn set_screen_values(
    doc: &mut Doc,
    index: u32,
    lines: &[Vec<String>],
) -> Result<(), Refusal> {
    if index > 3 {
        return Err(bad("Telemetry screens are 1-4."));
    }
    if lines.len() > SCREEN_LINES {
        return Err(bad(format!(
            "A value screen holds {SCREEN_LINES} lines; this has {}.",
            lines.len()
        )));
    }
    let mut out = vec![
        format!("   {index}:"),
        "      type: VALUES".to_string(),
        "      u: ".to_string(),
    ];
    let mut body = Vec::new();
    for (li, line) in lines.iter().enumerate() {
        let sources: Vec<&String> = line.iter().filter(|s| !s.trim().is_empty()).collect();
        if sources.is_empty() {
            continue;
        }
        if sources.len() > SCREEN_SOURCES {
            return Err(bad(format!(
                "A value screen line holds {SCREEN_SOURCES} sources; line {} has {}.",
                li + 1,
                sources.len()
            )));
        }
        body.push(format!("            {li}:"));
        body.push("               sources: ".to_string());
        for (si, s) in sources.iter().enumerate() {
            let t = s.trim();
            let s = match t.strip_prefix('{').and_then(|x| x.strip_suffix('}')) {
                Some(_) => format!("tele({})", resolve_sensors(doc, t)?),
                None => t.to_string(),
            };
            if !s
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "()_-+".contains(c))
            {
                return Err(bad(format!("{s:?} is not a source name.")));
            }
            body.push(format!("                  {si}:"));
            body.push(format!("                     val: {s}"));
        }
    }
    if body.is_empty() {
        // A value screen with no source is a screen EdgeTX does not show.
        return Err(bad(
            "A value screen needs at least one source; remove the screen instead.",
        ));
    }
    out.push("         lines: ".to_string());
    out.extend(body);
    super::model::put_screen(doc, index, out)
}

// ----- callouts -----

/// How many times `text` holds the token for logical switch index `n` (`L1` is 0).
fn mentions_ls(text: &str, n: u32) -> usize {
    let name = ls_name(n);
    let mut count = 0;
    let mut from = 0;
    while let Some(i) = text[from..].find(&name) {
        let at = from + i;
        let before = text[..at].chars().next_back();
        let after = text[at + name.len()..].chars().next();
        let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
        if !before.is_some_and(word) && !after.is_some_and(|c| c.is_ascii_digit()) {
            count += 1;
        }
        from = at + name.len();
    }
    count
}

/// How many times `L<n>` appears in the file outside the `logicalSw` block's own keys.
fn ls_refs(doc: &Doc, n: u32) -> usize {
    doc.lines().iter().map(|l| mentions_ls(l, n)).sum()
}

fn sf_ls(f: &[Field]) -> Option<u32> {
    let sw = unquote(get(f, "swtch"));
    ls_index(sw.trim_start_matches('!'))
}

fn sensor_prec(doc: &Doc, label: &str) -> Result<Option<u32>, Refusal> {
    let Some(n) = doc.top("telemetrySensors")? else {
        return Ok(None);
    };
    Ok(n.children
        .iter()
        .find(|c| c.get("label").map(unquote) == Some(label))
        .map(|c| c.get("prec").and_then(|p| p.parse().ok()).unwrap_or(0)))
}

/// A typed value stored at the sensor's precision.
fn scale(value: &str, prec: u32) -> Result<i64, Refusal> {
    let v: f64 = value
        .trim()
        .parse()
        .map_err(|_| bad(format!("{value:?} is not a number.")))?;
    Ok((v * 10f64.powi(prec as i32)).round() as i64)
}

/// A source as the file writes it: `{RxBt}` becomes `tele({RxBt})`, resolved by the op.
fn telesrc(source: &str) -> String {
    if source.starts_with('{') && source.ends_with('}') {
        format!("tele({source})")
    } else {
        source.to_string()
    }
}

fn check_track(track: &str) -> Result<(), Refusal> {
    check_text("A track name", track, MAX_TRACK_NAME)?;
    if track.is_empty()
        || !track
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(bad(format!(
            "Track {track:?}: a track name is 1-{MAX_TRACK_NAME} letters, digits, - or _."
        )));
    }
    Ok(())
}

fn owned(items: &[Vec<Field>], track: &str) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, f)| get(f, "func") == "PLAY_TRACK" && def_head(f) == track)
        .map(|(i, _)| i)
        .collect()
}

/// The logical switch only these callout functions use, when there is one.
fn sole_ls(doc: &Doc, items: &[Vec<Field>], mine: &[usize]) -> Option<u32> {
    let n = mine.first().and_then(|i| sf_ls(&items[*i]))?;
    let by_me = mine
        .iter()
        .filter(|i| sf_ls(&items[**i]) == Some(n))
        .count();
    (ls_refs(doc, n) == by_me).then_some(n)
}

pub(super) fn set_callout(doc: &mut Doc, c: &CalloutDef) -> Result<(), Refusal> {
    check_track(&c.track)?;
    let repeat = c.repeat.clone().unwrap_or_else(|| "1x".into());
    let items = sf_item_fields(doc)?;
    let mine = owned(&items, &c.track);
    let old_ls = sole_ls(doc, &items, &mine);
    let swtch = match &c.when {
        CalloutWhen::Switch { swtch } => {
            if swtch.is_empty() {
                return Err(bad("A callout needs a switch."));
            }
            if let Some(n) = old_ls {
                set_ls(doc, n, None)?;
            }
            swtch.clone()
        }
        CalloutWhen::Below {
            source,
            value,
            delay_ds,
        }
        | CalloutWhen::Above {
            source,
            value,
            delay_ds,
        } => {
            let below = matches!(c.when, CalloutWhen::Below { .. });
            let prec = source
                .strip_prefix('{')
                .and_then(|s| s.strip_suffix('}'))
                .map(|label| {
                    sensor_prec(doc, label)?.ok_or_else(|| {
                        bad(format!(
                            "{} has no telemetry sensor labelled {label:?}; discover sensors on the radio first.",
                            doc.name
                        ))
                    })
                })
                .transpose()?
                .unwrap_or(0);
            let v = scale(value, prec)?;
            let slot = match old_ls {
                Some(n) => n,
                None => {
                    let used: Vec<u32> = match doc.top("logicalSw")? {
                        Some(n) => n.children.iter().filter_map(|c| c.index()).collect(),
                        None => Vec::new(),
                    };
                    (0..MAX_LOGICAL_SWITCHES)
                        .find(|i| !used.contains(i))
                        .ok_or_else(|| bad("All 64 logical switches are in use."))?
                }
            };
            set_ls(
                doc,
                slot,
                Some(&LsDef {
                    func: if below { "FUNC_VNEG" } else { "FUNC_VPOS" }.into(),
                    def: format!("{},{v}", telesrc(source)),
                    andsw: "NONE".into(),
                    delay: *delay_ds,
                    duration: 0,
                }),
            )?;
            ls_name(slot)
        }
    };
    let sf = SfDef {
        swtch,
        func: "PLAY_TRACK".into(),
        def: format!("{},1,{repeat}", c.track),
    };
    // check_sf runs inside `special_functions`; reuse it by validating the repeat here.
    if !(repeat == "1x" || repeat == "!1x" || repeat.parse::<u32>().is_ok()) {
        return Err(bad(format!(
            "Repeat {repeat:?}: use 1x, !1x (once, not at power-on) or seconds."
        )));
    }
    let node = doc.top("customFn")?;
    let mut items = sf_item_fields(doc)?;
    let mine = owned(&items, &c.track);
    let want = sf_fields(doc, &sf)?;
    match mine.split_first() {
        Some((first, rest)) => {
            items[*first] = want;
            for i in rest.iter().rev() {
                items.remove(*i);
            }
        }
        None => items.push(want),
    }
    if items.len() > MAX_SPECIAL_FUNCTIONS {
        return Err(bad(format!(
            "A model holds {MAX_SPECIAL_FUNCTIONS} special functions at most; this would make {}.",
            items.len()
        )));
    }
    write_sfs(doc, node, &items)
}

pub(super) fn remove_callout(doc: &mut Doc, track: &str) -> Result<(), Refusal> {
    check_track(track)?;
    let items = sf_item_fields(doc)?;
    let mine = owned(&items, track);
    if mine.is_empty() {
        return Ok(());
    }
    let old_ls = sole_ls(doc, &items, &mine);
    if let Some(n) = old_ls {
        set_ls(doc, n, None)?;
    }
    let node = doc.top("customFn")?;
    let mut items = sf_item_fields(doc)?;
    let mine = owned(&items, track);
    for i in mine.iter().rev() {
        items.remove(*i);
    }
    write_sfs(doc, node, &items)
}

// ----- reads -----

fn label_of(sensors: &[(u32, String)], source: &str) -> String {
    let inner = source
        .strip_prefix("tele(")
        .and_then(|s| s.strip_suffix(')'))
        .and_then(|s| s.parse::<u32>().ok());
    match inner.and_then(|n| sensors.iter().find(|(slot, _)| *slot == n)) {
        Some((_, l)) => format!("{{{l}}}"),
        None => source.to_string(),
    }
}

/// Reads what the editors show. Refuses a block it cannot read.
pub fn editor_view(doc: &Doc) -> Result<EditorView, Refusal> {
    let mut slots: Vec<(u32, String)> = Vec::new();
    let mut precs: Vec<(u32, u32)> = Vec::new();
    let mut sensors = Vec::new();
    if let Some(n) = doc.top("telemetrySensors")? {
        for c in &n.children {
            let (Some(slot), Some(label)) = (c.index(), c.get("label")) else {
                continue;
            };
            slots.push((slot, unquote(label).to_string()));
            precs.push((
                slot,
                c.get("prec").and_then(|p| p.parse().ok()).unwrap_or(0),
            ));
            sensors.push(SensorLog {
                label: unquote(label).to_string(),
                logs: c.get("logs") == Some("1"),
            });
        }
    }
    let items = sf_item_fields(doc)?;
    let logging = items
        .iter()
        .find(|f| get(f, "func") == "LOGS")
        .map(|f| LoggingDef {
            swtch: unquote(get(f, "swtch")).into(),
            period_ds: def_head(f).parse().unwrap_or(0),
        });
    let ls = match doc.top("logicalSw")? {
        Some(n) => n
            .children
            .iter()
            .filter_map(|c| {
                Some((
                    c.index()?,
                    c.get("func")?.to_string(),
                    unquote(c.get("def")?).to_string(),
                    c.get("delay")
                        .and_then(|d| d.parse::<u32>().ok())
                        .unwrap_or(0),
                    unquote(c.get("andsw").unwrap_or("NONE")).to_string(),
                ))
            })
            .collect::<Vec<_>>(),
        None => Vec::new(),
    };
    let mut callouts = Vec::new();
    for f in items.iter().filter(|f| get(f, "func") == "PLAY_TRACK") {
        let def = unquote(get(f, "def"));
        let mut parts = def.split(',');
        let track = parts.next().unwrap_or("").to_string();
        let repeat = parts.nth(1).unwrap_or("1x").to_string();
        let swtch = unquote(get(f, "swtch")).to_string();
        let when = match sf_ls(f).filter(|_| !swtch.starts_with('!')) {
            None => Some(CalloutWhen::Switch {
                swtch: swtch.clone(),
            }),
            Some(n) => {
                ls.iter()
                    .find(|(i, ..)| *i == n)
                    .and_then(|(_, func, def, delay, andsw)| {
                        let (src, v) = def.split_once(',')?;
                        let below = match func.as_str() {
                            "FUNC_VNEG" => true,
                            "FUNC_VPOS" => false,
                            _ => return None,
                        };
                        if andsw != "NONE" {
                            return None;
                        }
                        let prec = src
                            .strip_prefix("tele(")
                            .and_then(|s| s.strip_suffix(')'))
                            .and_then(|s| s.parse::<u32>().ok())
                            .and_then(|slot| precs.iter().find(|(s, _)| *s == slot))
                            .map(|(_, p)| *p)
                            .unwrap_or(0);
                        let raw: f64 = v.trim().parse().ok()?;
                        let value = format!("{}", raw / 10f64.powi(prec as i32));
                        let source = label_of(&slots, src);
                        Some(if below {
                            CalloutWhen::Below {
                                source,
                                value,
                                delay_ds: *delay,
                            }
                        } else {
                            CalloutWhen::Above {
                                source,
                                value,
                                delay_ds: *delay,
                            }
                        })
                    })
            }
        };
        callouts.push(CalloutView {
            track,
            swtch,
            repeat,
            when,
        });
    }
    let rf_alarms = doc.top("rfAlarms")?.and_then(|n| {
        Some(RfAlarms {
            warning: n.get("warning")?.parse().ok()?,
            critical: n.get("critical")?.parse().ok()?,
        })
    });
    let mut screens = Vec::new();
    if let Some(n) = doc.top("screens")? {
        for c in &n.children {
            let Some(index) = c.index() else { continue };
            let mut lines: Vec<Vec<String>> = Vec::new();
            if let Some(l) = c.path(&["u", "lines"]) {
                for line in &l.children {
                    let Some(li) = line.index() else { continue };
                    while lines.len() <= li as usize {
                        lines.push(Vec::new());
                    }
                    if let Some(src) = line.child("sources") {
                        lines[li as usize] = src
                            .children
                            .iter()
                            .filter_map(|s| s.get("val").map(|v| unquote(v).to_string()))
                            .collect();
                    }
                }
            }
            let labels = lines
                .iter()
                .map(|l| l.iter().map(|s| label_of(&slots, s)).collect())
                .collect();
            screens.push(ScreenDetail {
                index,
                kind: c.get("type").unwrap_or("").into(),
                script: c
                    .path(&["u", "script", "file"])
                    .map(|f| unquote(&f.value).to_string()),
                lines,
                labels,
            });
        }
    }
    Ok(EditorView {
        logging: LoggingView { logging, sensors },
        rf_alarms,
        callouts,
        screens,
    })
}
