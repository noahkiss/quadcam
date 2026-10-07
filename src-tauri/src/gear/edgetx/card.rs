//! An EdgeTX SD card: `RADIO/radio.yml`, `MODELS/`, `SOUNDS/`, `SCRIPTS/`, `LOGS/`.
//!
//! - `Card::view` reads the card: identity, models, the selected model, the radio clock.
//! - `Card::plan` turns card edits into new file bytes, a diff and the checks. It writes
//!   nothing.
//! - `write` puts a plan on the card, one file at a time: the guards again, a backup of
//!   every touched file, then per file a temporary name, `F_FULLFSYNC`, a rename and a
//!   read-back. Only `gear/apply` calls it (design 8.1).
//! - `release` unmounts the card. The "safe to unplug" cue comes only after it succeeds.
//!
//! Slow links: a radio in USB Storage mode writes about 0.3 MB/s. A file's write is never
//! stopped once it starts; a stop request takes effect between files, so the card is
//! always whole. Every file write and the unmount run under a timeout. Pulling a radio
//! mid-write once wedged macOS's disk service until a reboot, so a timeout reports "a
//! reboot may be needed" instead of hanging.

use super::model::{self as em, ModelOp};
use super::yaml::{latin1, to_latin1, unquote, Doc};
use crate::gear::compat::{self, Product};
use crate::gear::model::{Check, DiffItem, DiffLine, Edit, Identity, LineOp, Refusal, RefusalCode};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// `RADIO/radio.yml`.
pub const RADIO_FILE: &str = "RADIO/radio.yml";
/// QuadCam's marker file: one line, the raw value the card's device id hashes. QuadCam
/// adds no keys to EdgeTX files; this file is its own.
pub const MARKER_FILE: &str = ".quadcam-id";
/// An empty file at a volume's root that tells Spotlight not to index it.
pub const NEVER_INDEX: &str = ".metadata_never_index";

/// The bootloader route, for a radio whose firmware stops at an error.
pub const BOOTLOADER_HINT: &str = "If the radio's firmware stops at an error, hold both horizontal trims inward while powering it on: the bootloader shows the SD card over USB.";

/// An edit to `radio.yml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum RadioOp {
    /// A top-level value the file already has (`hapticMode`, ...). Never the selected model:
    /// that is `SelectModel`.
    SetScalar { key: String, value: String },
    /// Selects a model (`model01.yml`): `currModel` and, when present, `currModelFilename`.
    SelectModel { file: String },
}

/// `radio.yml` keys `SetScalar` refuses.
pub const RADIO_PROTECTED: &[&str] = &[
    "currModel",
    "currModelFilename",
    "semver",
    "board",
    "checksum",
    "manuallyEdited",
];

/// `hapticMode` values.
pub const HAPTIC_MODES: &[&str] = &["mode_quiet", "mode_alarms", "mode_nokeys", "mode_all"];

/// A model file on the card.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct ModelSummary {
    /// `model01.yml`.
    pub file: String,
    /// The header name; empty when the file cannot be read.
    pub name: String,
    /// `radio.yml` selects it.
    pub selected: bool,
    /// Set when QuadCam cannot read the file (the reason).
    #[serde(default)]
    pub problem: Option<String>,
}

/// What the radio's clock looks like, from its log names.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct ClockCheck {
    pub ok: bool,
    /// The newest log by date in its name.
    #[serde(default)]
    pub newest_log: Option<String>,
    /// Set when the clock looks wrong.
    #[serde(default)]
    pub message: Option<String>,
}

/// A card as Gear reads it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct CardView {
    pub root: PathBuf,
    /// Board and version from `radio.yml`.
    pub identity: Identity,
    /// None when QuadCam may write this card; else why not.
    #[serde(default)]
    pub read_only: Option<Refusal>,
    pub models: Vec<ModelSummary>,
    /// The model `radio.yml` selects (`model01.yml`), and its name.
    #[serde(default)]
    pub selected_model: Option<String>,
    #[serde(default)]
    pub selected_name: Option<String>,
    pub clock: ClockCheck,
    /// QuadCam's marker, when the card has one.
    #[serde(default)]
    pub marker: Option<String>,
    /// The typed view of the model asked for.
    #[serde(default)]
    pub model: Option<em::ModelView>,
}

/// One file a plan changes. `before` None: a new file; `after` None: a delete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub before: Option<Vec<u8>>,
    pub after: Option<Vec<u8>>,
}

/// What a set of card edits would do. Writes nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct CardPlan {
    pub identity: Identity,
    pub checks: Vec<Check>,
    pub files: Vec<FileChange>,
    pub diff: Vec<DiffItem>,
    /// Things to know that do not refuse (a haptic mode that drops special-function
    /// haptics).
    pub warnings: Vec<String>,
}

impl CardPlan {
    pub fn ready(&self) -> bool {
        self.checks.iter().all(|c| c.ok)
    }

    /// Bytes the plan writes.
    pub fn bytes(&self) -> u64 {
        self.files
            .iter()
            .map(|f| f.after.as_ref().map_or(0, |b| b.len() as u64))
            .sum()
    }
}

/// An EdgeTX card at a folder: a mount point, or a folder a test made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    root: PathBuf,
}

/// True for a path that looks like an EdgeTX model file name (`model01.yml`).
pub fn is_model_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower
        .strip_prefix("model")
        .and_then(|r| r.strip_suffix(".yml"))
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

/// `model01.yml` -> 1.
pub fn model_number(name: &str) -> Option<u32> {
    if !is_model_file(name) {
        return None;
    }
    name[5..name.len() - 4].parse().ok()
}

fn check(name: &str, r: Result<(), Refusal>) -> Check {
    match r {
        Ok(()) => Check {
            name: name.into(),
            ok: true,
            refusal: None,
        },
        Err(e) => Check {
            name: name.into(),
            ok: false,
            refusal: Some(e),
        },
    }
}

/// Characters per checklist line, for boards QuadCam has measured (128x64: 20).
pub fn checklist_width(board: Option<&str>) -> Option<usize> {
    match board.map(|b| b.to_ascii_lowercase()).as_deref() {
        Some("pocket") => Some(20),
        _ => None,
    }
}

/// `board` and `semver` from `radio.yml` text. Reads through the YAML engine; a file the
/// engine refuses falls back to plain top-level `key: value` lines.
pub fn identity_from_radio_yml(bytes: &[u8]) -> Identity {
    let mut id = Identity {
        firmware: Some("EdgeTX".into()),
        ..Default::default()
    };
    let clean = |v: &str| {
        let v = unquote(v.trim()).trim().to_string();
        (!v.is_empty()).then_some(v)
    };
    match Doc::parse("radio.yml", bytes) {
        Ok(d) => {
            id.board = d.top_value("board").and_then(|v| clean(&v));
            id.version = d.top_value("semver").and_then(|v| clean(&v));
        }
        Err(_) => {
            for line in latin1(bytes).lines() {
                if line.starts_with([' ', '\t', '-', '#']) {
                    continue;
                }
                if let Some((k, v)) = line.split_once(':') {
                    match k.trim() {
                        "board" => id.board = clean(v),
                        "semver" => id.version = clean(v),
                        _ => {}
                    }
                }
            }
        }
    }
    id
}

/// The marker's value, when the card has one.
pub fn read_marker(root: &Path) -> Option<String> {
    let s = std::fs::read_to_string(root.join(MARKER_FILE)).ok()?;
    let v = s.lines().next()?.trim().to_string();
    (!v.is_empty() && v.len() <= 200).then_some(v)
}

/// The radio clock from log names (`<model>-YYYY-MM-DD.csv`): a date before 2020 means
/// the clock reset (EdgeTX starts at 2000-01-01 when its clock battery is flat), and a
/// date after `today` means it runs ahead.
pub fn clock_check(logs: &[String], today: chrono::NaiveDate) -> ClockCheck {
    let dated: Vec<(chrono::NaiveDate, &String)> = logs
        .iter()
        .filter_map(|n| {
            let stem = n.strip_suffix(".csv").or_else(|| n.strip_suffix(".CSV"))?;
            let d = stem.get(stem.len().checked_sub(10)?..)?;
            Some((chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()?, n))
        })
        .collect();
    let Some((date, name)) = dated.iter().max_by_key(|(d, _)| *d) else {
        return ClockCheck {
            ok: true,
            newest_log: None,
            message: None,
        };
    };
    let message = if date.year_ce().1 < 2020 {
        Some(format!(
            "The radio clock looks reset: its newest log is dated {date}. Set the date on the radio; if it resets again, its clock battery may be dead."
        ))
    } else if *date > today + chrono::Days::new(1) {
        Some(format!(
            "The radio clock looks wrong: a log is dated {date}, after today."
        ))
    } else if let Some((d, _)) = dated.iter().find(|(d, _)| d.year_ce().1 < 2020) {
        Some(format!(
            "The radio clock was reset at some point (a log is dated {d}). If it happens again, its clock battery may be dead."
        ))
    } else {
        None
    };
    ClockCheck {
        ok: message.is_none(),
        newest_log: Some((*name).clone()),
        message,
    }
}

use chrono::Datelike;

impl Card {
    /// The card at `root`. Refuses a folder with neither `RADIO/radio.yml` nor `MODELS/`.
    pub fn open(root: &Path) -> Result<Card> {
        if !root.join(RADIO_FILE).is_file() && !root.join("MODELS").is_dir() {
            bail!(
                "{} is not an EdgeTX card: it has no RADIO/radio.yml and no MODELS folder.",
                root.display()
            );
        }
        Ok(Card {
            root: root.to_path_buf(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// A file's bytes; None when it does not exist.
    pub fn read(&self, rel: &str) -> Result<Option<Vec<u8>>> {
        let p = self.root.join(rel);
        match std::fs::read(&p) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("reading {}", p.display())),
        }
    }

    pub fn identity(&self) -> Identity {
        match self.read(RADIO_FILE) {
            Ok(Some(b)) => identity_from_radio_yml(&b),
            _ => Identity {
                firmware: Some("EdgeTX".into()),
                ..Default::default()
            },
        }
    }

    /// Model file names in `MODELS/`, sorted.
    pub fn model_files(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.root.join("MODELS"))
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .filter(|n| is_model_file(n) && !n.starts_with('.'))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    /// The selected model's file name, from `radio.yml`: `currModelFilename` when the
    /// file has it, else `currModel` as `modelNN.yml`.
    pub fn selected_model(&self) -> Option<String> {
        let b = self.read(RADIO_FILE).ok()??;
        let d = Doc::parse("radio.yml", &b).ok()?;
        selected_in(&d)
    }

    /// The card as Gear shows it; `model` adds that model's typed view.
    pub fn view(&self, model: Option<&str>, today: chrono::NaiveDate) -> Result<CardView> {
        let identity = self.identity();
        let read_only = compat::check_writable(
            Product::Edgetx,
            identity.board.as_deref(),
            identity.version.as_deref(),
        )
        .err();
        let selected = self.selected_model();
        let mut models = Vec::new();
        for f in self.model_files() {
            let (name, problem) = match self.read(&format!("MODELS/{f}")) {
                Ok(Some(b)) => match Doc::parse(&f, &b).and_then(|d| em::model_name(&d)) {
                    Ok(n) => (n.unwrap_or_default(), None),
                    Err(e) => (String::new(), Some(e.reason)),
                },
                Ok(None) => continue,
                Err(e) => (String::new(), Some(format!("{e:#}"))),
            };
            models.push(ModelSummary {
                selected: selected.as_deref() == Some(f.as_str()),
                file: f,
                name,
                problem,
            });
        }
        let selected_name = selected
            .as_deref()
            .and_then(|s| models.iter().find(|m| m.file == s).map(|m| m.name.clone()));
        let logs: Vec<String> = std::fs::read_dir(self.root.join("LOGS"))
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .filter(|n| !n.starts_with('.'))
                    .collect()
            })
            .unwrap_or_default();
        let model = match model {
            Some(f) => {
                let f = model_file_arg(f)?;
                let b = self
                    .read(&format!("MODELS/{f}"))?
                    .with_context(|| format!("The card has no MODELS/{f}."))?;
                let d = Doc::parse(&f, &b).map_err(anyhow::Error::new)?;
                Some(em::view(&f, &d).map_err(anyhow::Error::new)?)
            }
            None => None,
        };
        Ok(CardView {
            root: self.root.clone(),
            identity,
            read_only,
            models,
            selected_model: selected,
            selected_name,
            clock: clock_check(&logs, today),
            marker: read_marker(&self.root),
            model,
        })
    }

    /// What these edits would do. Writes nothing. `marker`, when given, is added as
    /// `.quadcam-id` if the card has none and the plan writes anything.
    pub fn plan(&self, edits: &[Edit], marker: Option<&str>) -> Result<CardPlan> {
        let identity = self.identity();
        let mut plan = CardPlan {
            identity: identity.clone(),
            checks: Vec::new(),
            files: Vec::new(),
            diff: Vec::new(),
            warnings: Vec::new(),
        };
        let known = compat::check_writable(
            Product::Edgetx,
            identity.board.as_deref(),
            identity.version.as_deref(),
        );
        let ok = known.is_ok();
        plan.checks.push(check("Known version", known));
        if !ok {
            return Ok(plan);
        }
        let mut work = Work {
            card: self,
            docs: BTreeMap::new(),
            raw: BTreeMap::new(),
            board: identity.board.clone(),
            warnings: Vec::new(),
            select_asked: false,
        };
        let selected_before = self.selected_model();
        let mut result = Ok(());
        for e in edits {
            result = work.edit(e);
            if result.is_err() {
                break;
            }
        }
        let (name, result) = match result {
            Ok(()) => ("Shape understood", Ok(())),
            Err(r) => (
                match r.code {
                    RefusalCode::RoundTrip => "Round trip",
                    RefusalCode::DeviceChanged => "Model identity",
                    RefusalCode::BadSetting => "Values in range",
                    _ => "Shape understood",
                },
                Err(r),
            ),
        };
        let failed = result.is_err();
        plan.checks.push(check(name, result));
        if failed {
            return Ok(plan);
        }
        // The selected model changes only when an edit asks for it.
        let selected_after = work
            .docs
            .get(RADIO_FILE)
            .and_then(selected_in)
            .or(selected_before.clone());
        let sel = if !work.select_asked && selected_after != selected_before {
            Err(Refusal::new(
                RefusalCode::BadSetting,
                "These edits would change the radio's selected model without asking; nothing was written.",
            ))
        } else {
            Ok(())
        };
        let sel_failed = sel.is_err();
        plan.checks.push(check("Selected model kept", sel));
        if sel_failed {
            return Ok(plan);
        }
        plan.warnings = std::mem::take(&mut work.warnings);
        for (path, after) in work.outputs()? {
            let before = self.read(&path)?;
            if before.as_deref() == after.as_deref() {
                continue;
            }
            plan.files.push(FileChange {
                path,
                before,
                after,
            });
        }
        if let (Some(m), false) = (marker, plan.files.is_empty()) {
            if read_marker(&self.root).is_none() {
                plan.files.push(FileChange {
                    path: MARKER_FILE.into(),
                    before: self.read(MARKER_FILE)?,
                    after: Some(format!("{m}\n").into_bytes()),
                });
            }
        }
        if !plan.files.is_empty() && !self.root.join(NEVER_INDEX).exists() {
            plan.files.push(FileChange {
                path: NEVER_INDEX.into(),
                before: None,
                after: Some(Vec::new()),
            });
        }
        plan.diff = diff_items(&plan.files);
        Ok(plan)
    }
}

fn selected_in(d: &Doc) -> Option<String> {
    if let Some(f) = d.top_value("currModelFilename") {
        let f = unquote(&f).to_string();
        if !f.is_empty() {
            return Some(f);
        }
    }
    let n: u32 = d.top_value("currModel")?.trim().parse().ok()?;
    Some(format!("model{n:02}.yml"))
}

/// A model file argument: `model01.yml` or `MODELS/model01.yml`.
fn model_file_arg(f: &str) -> Result<String> {
    let f = f.trim().trim_start_matches("MODELS/");
    if !is_model_file(f) {
        bail!("{f:?} is not a model file name (model01.yml).");
    }
    Ok(f.to_string())
}

/// The edits' working state: each touched file's doc, or raw bytes for non-YAML files
/// (None: deleted).
struct Work<'a> {
    card: &'a Card,
    docs: BTreeMap<String, Doc>,
    raw: BTreeMap<String, Option<Vec<u8>>>,
    board: Option<String>,
    warnings: Vec<String>,
    select_asked: bool,
}

impl Work<'_> {
    fn doc(&mut self, path: &str) -> Result<&mut Doc, Refusal> {
        if !self.docs.contains_key(path) {
            if matches!(self.raw.get(path), Some(None)) {
                return Err(bad(format!("{path} is deleted by an earlier edit.")));
            }
            let bytes = match self.raw.get(path) {
                Some(Some(b)) => Some(b.clone()),
                _ => self.card.read(path).map_err(|e| bad(format!("{e:#}")))?,
            };
            let bytes = bytes.ok_or_else(|| bad(format!("The card has no {path}.")))?;
            let name = path.rsplit('/').next().unwrap_or(path);
            let d = Doc::parse(name, &bytes)?;
            self.raw.remove(path);
            self.docs.insert(path.to_string(), d);
        }
        Ok(self.docs.get_mut(path).unwrap())
    }

    fn exists(&self, path: &str) -> bool {
        if self.docs.contains_key(path) {
            return true;
        }
        match self.raw.get(path) {
            Some(x) => x.is_some(),
            None => self.card.root.join(path).is_file(),
        }
    }

    fn edit(&mut self, e: &Edit) -> Result<(), Refusal> {
        match e {
            Edit::Model { file, name, ops } => {
                let f = model_file_arg(file).map_err(|e| bad(format!("{e:#}")))?;
                let path = format!("MODELS/{f}");
                let d = self.doc(&path)?;
                if let Some(want) = name {
                    let have = em::model_name(d)?.unwrap_or_default();
                    if &have != want {
                        return Err(Refusal::new(
                            RefusalCode::DeviceChanged,
                            format!("{f} is {have:?}, not {want:?}; wrong card?"),
                        ));
                    }
                }
                em::apply(d, ops)
            }
            Edit::Radio { ops } => {
                for op in ops {
                    self.radio_op(op)?;
                }
                Ok(())
            }
            Edit::Checklist { model, text } => self.checklist(model, text),
            Edit::ModelCopy { from, to, name } => self.copy(from, to, name),
            Edit::ModelDelete { file } => {
                let f = model_file_arg(file).map_err(|e| bad(format!("{e:#}")))?;
                let sel = match self.docs.get(RADIO_FILE) {
                    Some(d) => selected_in(d),
                    None => self.card.selected_model(),
                };
                if sel.as_deref() == Some(f.as_str()) {
                    return Err(bad(format!(
                        "{f} is the radio's selected model; select another model first."
                    )));
                }
                let path = format!("MODELS/{f}");
                if !self.exists(&path) {
                    return Err(bad(format!("The card has no {path}.")));
                }
                self.docs.remove(&path);
                self.raw.insert(path, None);
                Ok(())
            }
            other => Err(bad(format!(
                "{} is not an edit QuadCam's EdgeTX engine plans.",
                serde_json::to_value(other)
                    .ok()
                    .and_then(|v| v["kind"].as_str().map(str::to_string))
                    .unwrap_or_default()
            ))),
        }
    }

    fn radio_op(&mut self, op: &RadioOp) -> Result<(), Refusal> {
        match op {
            RadioOp::SetScalar { key, value } => {
                if RADIO_PROTECTED.contains(&key.as_str()) {
                    return Err(bad(format!(
                        "{key} is not set this way{}.",
                        if key.starts_with("currModel") {
                            "; use select_model"
                        } else {
                            ""
                        }
                    )));
                }
                if key == "hapticMode" {
                    if !HAPTIC_MODES.contains(&value.as_str()) {
                        return Err(bad(format!(
                            "hapticMode is one of {}.",
                            HAPTIC_MODES.join(", ")
                        )));
                    }
                    if value == "mode_alarms" {
                        self.warnings.push(
                            "hapticMode mode_alarms drops every special-function haptic.".into(),
                        );
                    }
                }
                self.doc(RADIO_FILE)?.set_top_scalar(key, value)?;
                Ok(())
            }
            RadioOp::SelectModel { file } => {
                let f = model_file_arg(file).map_err(|e| bad(format!("{e:#}")))?;
                if !self.exists(&format!("MODELS/{f}")) {
                    return Err(bad(format!("The card has no MODELS/{f}.")));
                }
                let n = model_number(&f).unwrap_or(0);
                self.select_asked = true;
                let d = self.doc(RADIO_FILE)?;
                d.set_top_scalar("currModel", &n.to_string())?;
                if d.top("currModelFilename")?.is_some() {
                    d.set_top_scalar("currModelFilename", &format!("\"{f}\""))?;
                }
                Ok(())
            }
        }
    }

    fn checklist(&mut self, model: &str, text: &str) -> Result<(), Refusal> {
        let f = model_file_arg(model).map_err(|e| bad(format!("{e:#}")))?;
        let d = self.doc(&format!("MODELS/{f}"))?;
        let name = em::model_name(d)?
            .filter(|n| !n.is_empty())
            .ok_or_else(|| bad(format!("{f} has no name; a checklist is named after it.")))?;
        let width = checklist_width(self.board.as_deref());
        let mut body = String::new();
        for line in text.lines() {
            let line = line.trim_end();
            if let Some(w) = width {
                if line.chars().count() > w {
                    return Err(bad(format!(
                        "Checklist line {line:?} is over {w} characters, the screen's width."
                    )));
                }
            }
            body.push_str(line);
            body.push('\n');
        }
        let bytes =
            to_latin1(&body).ok_or_else(|| bad("A checklist holds Latin-1 characters only."))?;
        // EdgeTX looks for MODELS/<name>.txt with spaces kept, then with `_`.
        let kept = format!("MODELS/{name}.txt");
        let under = format!("MODELS/{}.txt", name.replace(' ', "_"));
        let path = if self.exists(&kept) { kept } else { under };
        self.raw.insert(path, Some(bytes));
        Ok(())
    }

    fn copy(&mut self, from: &str, to: &str, name: &str) -> Result<(), Refusal> {
        let from = model_file_arg(from).map_err(|e| bad(format!("{e:#}")))?;
        let to = model_file_arg(to).map_err(|e| bad(format!("{e:#}")))?;
        let to_path = format!("MODELS/{to}");
        if self.exists(&to_path) {
            return Err(bad(format!("{to_path} exists; delete it first.")));
        }
        let src = self.doc(&format!("MODELS/{from}"))?.render();
        let mut d = Doc::parse(&to, &src)?;
        em::apply(
            &mut d,
            &[ModelOp::Rename {
                name: name.to_string(),
            }],
        )?;
        if let Some(t) = d.top("timers")? {
            for c in &t.children {
                if let Some(v) = c.child("value") {
                    d.set_line(v.line, format!("{}value: 0", " ".repeat(v.indent)));
                }
            }
        }
        // A copy keeps no ELRS model id of its source; set one with set_model_id.
        em::apply(&mut d, &[ModelOp::SetModelId { module: 0, id: 0 }])?;
        self.raw.remove(&to_path);
        self.docs.insert(to_path, d);
        Ok(())
    }

    /// Every touched file and its new bytes (None: delete). Edited YAML files get
    /// `checksum: 0` when they have a checksum line.
    fn outputs(mut self) -> Result<Vec<(String, Option<Vec<u8>>)>> {
        let mut out = Vec::new();
        for (p, d) in self.docs.iter_mut() {
            if d.changed() || d.original().is_empty() {
                d.zero_checksum();
            }
            out.push((p.clone(), Some(d.render())));
        }
        for (p, b) in self.raw {
            out.push((p, b));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }
}

fn bad(reason: impl Into<String>) -> Refusal {
    Refusal::new(RefusalCode::BadSetting, reason)
}

// ----- diff -----

/// Lines of context around each change.
const CONTEXT: usize = 3;

/// A line diff of two texts: the changes, with `CONTEXT` unchanged lines around each.
pub fn line_diff(a: &str, b: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = a.lines().collect();
    let b: Vec<&str> = b.lines().collect();
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..]
        .iter()
        .rev()
        .zip(b[pre..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (am, bm) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    // LCS over the changed middle.
    let (n, m) = (am.len(), bm.len());
    let mut t = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[i][j] = if am[i] == bm[j] {
                t[i + 1][j + 1] + 1
            } else {
                t[i + 1][j].max(t[i][j + 1])
            };
        }
    }
    let mut all: Vec<DiffLine> = a[..pre]
        .iter()
        .map(|l| DiffLine {
            op: LineOp::Same,
            text: l.to_string(),
        })
        .collect();
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && am[i] == bm[j] {
            all.push(DiffLine {
                op: LineOp::Same,
                text: am[i].into(),
            });
            i += 1;
            j += 1;
        } else if j < m && (i == n || t[i][j + 1] >= t[i + 1][j]) {
            all.push(DiffLine {
                op: LineOp::Add,
                text: bm[j].into(),
            });
            j += 1;
        } else {
            all.push(DiffLine {
                op: LineOp::Remove,
                text: am[i].into(),
            });
            i += 1;
        }
    }
    all.extend(a[a.len() - suf..].iter().map(|l| DiffLine {
        op: LineOp::Same,
        text: l.to_string(),
    }));
    let changed: Vec<usize> = all
        .iter()
        .enumerate()
        .filter(|(_, l)| l.op != LineOp::Same)
        .map(|(i, _)| i)
        .collect();
    all.into_iter()
        .enumerate()
        .filter(|(i, l)| l.op != LineOp::Same || changed.iter().any(|c| c.abs_diff(*i) <= CONTEXT))
        .map(|(_, l)| l)
        .collect()
}

fn is_text(path: &str) -> bool {
    [".yml", ".txt", ".lua", ".csv", MARKER_FILE]
        .iter()
        .any(|s| path.ends_with(s))
}

fn diff_items(files: &[FileChange]) -> Vec<DiffItem> {
    let mut out = Vec::new();
    let put: Vec<String> = files
        .iter()
        .filter(|f| f.after.is_some())
        .map(|f| f.path.clone())
        .collect();
    let delete: Vec<String> = files
        .iter()
        .filter(|f| f.after.is_none())
        .map(|f| f.path.clone())
        .collect();
    out.push(DiffItem::Files {
        label: "Card files".into(),
        put,
        delete,
    });
    for f in files {
        if !is_text(&f.path) {
            continue;
        }
        let a = f.before.as_deref().map(latin1).unwrap_or_default();
        let b = f.after.as_deref().map(latin1).unwrap_or_default();
        out.push(DiffItem::Lines {
            label: f.path.clone(),
            lines: line_diff(&a, &b),
        });
    }
    out
}

// ----- writing -----

/// How a write runs: timeouts for the link, and a stop flag read between files.
#[derive(Debug, Clone)]
pub struct WriteOptions {
    /// Every file write may take this long plus its bytes at `floor_bytes_per_s`.
    pub base_timeout: Duration,
    /// A speed well under the link's, for the timeout (not the ETA).
    pub floor_bytes_per_s: u64,
    /// The link's usual speed, for the ETA.
    pub bytes_per_s: u64,
    /// Set to stop after the current file.
    pub stop: Arc<AtomicBool>,
}

impl WriteOptions {
    /// A card in a reader or the built-in slot.
    pub fn reader() -> Self {
        Self {
            base_timeout: Duration::from_secs(20),
            floor_bytes_per_s: 1_000_000,
            bytes_per_s: 10_000_000,
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    /// A radio in USB Storage mode: about 0.3 MB/s (measured on a 2.12 radio).
    pub fn radio_usb() -> Self {
        Self {
            base_timeout: Duration::from_secs(30),
            floor_bytes_per_s: 100_000,
            bytes_per_s: 300_000,
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn timeout_for(&self, bytes: u64) -> Duration {
        self.base_timeout + Duration::from_secs(bytes / self.floor_bytes_per_s.max(1))
    }
}

/// Where a write stands, for a progress bar. A stop request shows `stopping` until the
/// current file is done ("Finishing the current file…").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct WriteProgress {
    /// 1-based.
    pub file: u32,
    pub files: u32,
    pub path: String,
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// Seconds left at the link's usual speed.
    pub eta_s: u64,
    pub stopping: bool,
}

/// What a write did.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct WriteReport {
    pub written: Vec<String>,
    pub deleted: Vec<String>,
    /// Stopped between files on request; `remaining` were not written.
    pub stopped: bool,
    pub remaining: Vec<String>,
}

/// "macOS did not answer…" for a write or an unmount that ran past its timeout.
pub fn stuck_message(what: &str, after: Duration) -> String {
    format!(
        "macOS did not finish {what} within {} s. Leave the card plugged in and do not pull it; a reboot may be needed before macOS sees it again.",
        after.as_secs()
    )
}

/// Runs `f` on its own thread and waits at most `timeout`. On a timeout the thread is
/// left to finish (a write is never stopped mid-file) and the error says so.
pub fn with_timeout<T: Send + 'static>(
    timeout: Duration,
    what: &str,
    f: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(timeout) {
        Ok(r) => r,
        Err(_) => Err(anyhow!(stuck_message(what, timeout))),
    }
}

/// True when this process may write card files at `root`. A process started by cargo
/// never writes under `/Volumes` (a real card) unless `QUADCAM_CARD_WRITE=real`.
pub fn writes_allowed(root: &Path, flag: Option<&str>, under_cargo: bool) -> bool {
    !(under_cargo && root.starts_with("/Volumes") && flag != Some("real"))
}

#[cfg(target_os = "macos")]
fn full_fsync(f: &std::fs::File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    extern "C" {
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    }
    const F_FULLFSYNC: i32 = 51;
    // SAFETY: fcntl on an open descriptor this File owns; F_FULLFSYNC takes no argument.
    let r = unsafe { fcntl(f.as_raw_fd(), F_FULLFSYNC) };
    if r == -1 {
        // Some file systems refuse F_FULLFSYNC; fsync is the next best.
        f.sync_all()
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
fn full_fsync(f: &std::fs::File) -> std::io::Result<()> {
    f.sync_all()
}

/// Writes one file: a temporary name beside it, the bytes, a full sync, a rename, a
/// sync of the folder, then a read-back. True when the bytes read back equal.
pub fn write_file_atomic(path: &Path, bytes: &[u8]) -> Result<bool> {
    use std::io::Write;
    let dir = path.parent().context("no parent folder")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let name = path
        .file_name()
        .context("no file name")?
        .to_string_lossy()
        .to_string();
    let tmp = dir.join(format!(".{name}.quadcam-tmp"));
    {
        let mut f =
            std::fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(bytes)?;
        full_fsync(&f)?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("renaming onto {}", path.display()))?;
    remove_apple_double(dir, &name);
    remove_apple_double(dir, &format!(".{name}.quadcam-tmp"));
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(std::fs::read(path)? == bytes)
}

/// AppleDouble files start with these bytes.
const APPLE_DOUBLE_MAGIC: [u8; 4] = [0x00, 0x05, 0x16, 0x07];

/// Removes `._<name>` beside a file QuadCam wrote. macOS keeps extended attributes on a
/// FAT card in these files; EdgeTX does not want them, and they clutter the card.
pub fn remove_apple_double(dir: &Path, name: &str) {
    let p = dir.join(format!("._{name}"));
    let is_ad = std::fs::File::open(&p)
        .and_then(|mut f| {
            let mut b = [0u8; 4];
            std::io::Read::read_exact(&mut f, &mut b).map(|_| b)
        })
        .is_ok_and(|b| b == APPLE_DOUBLE_MAGIC);
    if is_ad {
        let _ = std::fs::remove_file(p);
    }
}

/// Puts a plan on the card at `root`. Runs every guard again first: the plan is ready,
/// writes are allowed here, and each file still holds the bytes the plan read. Then
/// `backup` gets every touched file that exists (path, bytes); a failed backup writes
/// nothing. Then one file at a time, each under a timeout. A read-back mismatch puts the
/// backed-up bytes back at once and fails with both. `stop` takes effect between files.
pub fn write(
    root: &Path,
    plan: &CardPlan,
    opts: &WriteOptions,
    backup: &mut dyn FnMut(&str, &[u8]) -> Result<()>,
    progress: &mut dyn FnMut(&WriteProgress),
) -> Result<WriteReport> {
    if !plan.ready() {
        bail!(plan
            .checks
            .iter()
            .find_map(|c| c.refusal.clone())
            .map(|r| r.to_string())
            .unwrap_or_else(|| "Refused: the plan's checks did not pass.".into()));
    }
    if !writes_allowed(
        root,
        std::env::var("QUADCAM_CARD_WRITE").ok().as_deref(),
        std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
    ) {
        return Err(anyhow::Error::new(Refusal::new(
            RefusalCode::Disabled,
            "Card writes under /Volumes are off in tests.",
        )));
    }
    for f in &plan.files {
        let now = match std::fs::read(root.join(&f.path)) {
            Ok(b) => Some(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).with_context(|| format!("reading {}", f.path)),
        };
        if now != f.before {
            return Err(anyhow::Error::new(Refusal::new(
                RefusalCode::BeforeMismatch,
                format!("{} changed since the plan; plan again.", f.path),
            )));
        }
    }
    for f in &plan.files {
        if let Some(b) = &f.before {
            backup(&f.path, b).map_err(|e| {
                anyhow::Error::new(Refusal::new(
                    RefusalCode::NoBackup,
                    format!("The backup failed: {e:#}. Nothing was written."),
                ))
            })?;
        }
    }
    let total: u64 = plan.bytes();
    let n = plan.files.len() as u32;
    let mut done = 0u64;
    let mut report = WriteReport {
        written: vec![],
        deleted: vec![],
        stopped: false,
        remaining: vec![],
    };
    for (i, f) in plan.files.iter().enumerate() {
        if opts.stop.load(Ordering::SeqCst) {
            report.stopped = true;
            report.remaining = plan.files[i..].iter().map(|f| f.path.clone()).collect();
            break;
        }
        let size = f.after.as_ref().map_or(0, |b| b.len() as u64);
        progress(&WriteProgress {
            file: i as u32 + 1,
            files: n,
            path: f.path.clone(),
            bytes_done: done,
            bytes_total: total,
            eta_s: (total - done) / opts.bytes_per_s.max(1),
            stopping: false,
        });
        let path = root.join(&f.path);
        match &f.after {
            None => {
                let p = path.clone();
                with_timeout(
                    opts.base_timeout,
                    &format!("deleting {}", f.path),
                    move || Ok(std::fs::remove_file(&p)?),
                )?;
                report.deleted.push(f.path.clone());
            }
            Some(bytes) => {
                let (p, b) = (path.clone(), bytes.clone());
                let same = with_timeout(
                    opts.timeout_for(size),
                    &format!("writing {}", f.path),
                    move || write_file_atomic(&p, &b),
                )?;
                if !same {
                    let restore = match &f.before {
                        Some(old) => {
                            let (p, b) = (path.clone(), old.clone());
                            match with_timeout(
                                opts.timeout_for(old.len() as u64),
                                &format!("restoring {}", f.path),
                                move || write_file_atomic(&p, &b),
                            ) {
                                Ok(true) => "the backed-up bytes are back".to_string(),
                                Ok(false) => {
                                    "restoring the backed-up bytes also read back wrong".into()
                                }
                                Err(e) => format!("restoring failed: {e:#}"),
                            }
                        }
                        None => {
                            let _ = std::fs::remove_file(&path);
                            "the new file was removed".into()
                        }
                    };
                    bail!(
                        "{} read back different from what was written; {restore}.",
                        f.path
                    );
                }
                report.written.push(f.path.clone());
            }
        }
        done += size;
        if opts.stop.load(Ordering::SeqCst) && i + 1 < plan.files.len() {
            progress(&WriteProgress {
                file: i as u32 + 1,
                files: n,
                path: f.path.clone(),
                bytes_done: done,
                bytes_total: total,
                eta_s: 0,
                stopping: true,
            });
        }
    }
    Ok(report)
}

/// Runs a command and waits at most `timeout`; on a timeout the child is killed and the
/// error says a reboot may be needed.
pub fn run_with_timeout(
    cmd: &mut std::process::Command,
    what: &str,
    timeout: Duration,
) -> Result<std::process::Output> {
    use std::process::Stdio;
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {what}"))?;
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return Ok(child.wait_with_output()?);
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!(stuck_message(what, timeout));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// How long an unmount may take before QuadCam reports macOS stuck.
pub const UNMOUNT_TIMEOUT: Duration = Duration::from_secs(60);

/// Unmounts a card's whole disk (`disk4`) with `diskutil unmountDisk`, under a timeout.
/// Only after this succeeds may "safe to unplug" play. A process started by cargo never
/// unmounts a real disk unless `QUADCAM_SERIAL=real`.
pub fn release(whole_disk: &str, timeout: Duration) -> Result<()> {
    if !crate::gear::serial::serial_enabled(
        std::env::var("QUADCAM_SERIAL").ok().as_deref(),
        std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
    ) {
        return Err(anyhow::Error::new(Refusal::new(
            RefusalCode::Disabled,
            "Unmounting a real disk is off in tests.",
        )));
    }
    let disk = format!("/dev/{}", crate::disk::whole_disk_of(whole_disk));
    let out = run_with_timeout(
        std::process::Command::new("/usr/sbin/diskutil").args(["unmountDisk", &disk]),
        &format!("unmounting {disk}"),
        timeout,
    )?;
    if !out.status.success() {
        bail!(
            "diskutil unmountDisk failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}
