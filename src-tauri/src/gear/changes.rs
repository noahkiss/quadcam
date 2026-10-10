//! Staged changes (design 4.1, 4.2): the set of edits a person or an agent queued for one
//! device, kept in the gear folder until they are applied or discarded.
//!
//! - One folder per change: `<gear>/changes/<YYYY-MM-DD>-<device>-<n>/`, holding
//!   `change.json` (the `StagedChange`) and, after an apply, `report.json`. The folder name
//!   is the change id. A discarded or applied change stays as bench history.
//! - `Changes` is the only writer. A write goes to a temp file and is renamed into place.
//! - `render_fc` turns an FC change's edits into the CLI lines the write engine sends, the
//!   diff the apply sheet shows and the before state the plan's digest covers.

use super::bf::cli;
use super::bf::dump::{self, parse_cmd, Cmd, Config};
use super::model::{
    ChangeEvent, ChangeStatus, DiffLine, Edit, LineOp, Refusal, RefusalCode, Section, StagedChange,
};
use super::osd::Pos;
use super::store::{safe, Store};
use crate::session::Editor;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::PathBuf;

/// `gear_changes`: which changes to list. Staged ones by default.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct ChangeFilter {
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub status: Option<ChangeStatus>,
    /// Also list applied, failed, reverted and discarded changes (the bench history).
    #[serde(default)]
    pub history: bool,
}

/// What `Changes::stage` needs.
#[derive(Debug, Clone)]
pub struct NewChange {
    pub device: String,
    pub title: String,
    pub edits: Vec<Edit>,
    pub base_backup: String,
    pub editor: Editor,
    pub note: String,
    pub status: ChangeStatus,
    /// The change a restore undoes.
    pub reverts: Option<String>,
}

/// `Changes::update`: the fields to change.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct ChangeUpdate {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub edits: Option<Vec<Edit>>,
    #[serde(default)]
    pub status: Option<ChangeStatus>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub order: Option<u32>,
}

/// The staged changes in a gear folder.
#[derive(Debug, Clone)]
pub struct Changes {
    store: Store,
}

impl Changes {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.store.changes_dir().join(safe(id))
    }

    fn file(&self, id: &str) -> PathBuf {
        self.dir(id).join("change.json")
    }

    /// One change by id.
    pub fn get(&self, id: &str) -> Result<StagedChange> {
        let bytes = std::fs::read(self.file(id))
            .with_context(|| format!("No staged change {id:?}. See `gear changes`."))?;
        serde_json::from_slice(&bytes).with_context(|| format!("change {id} is not valid JSON"))
    }

    /// Every change in the folder that parses, sorted by device, order, then id.
    pub fn all(&self) -> Vec<StagedChange> {
        let mut out: Vec<StagedChange> = std::fs::read_dir(self.store.changes_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| std::fs::read(e.path().join("change.json")).ok())
            .filter_map(|b| serde_json::from_slice(&b).ok())
            .collect();
        out.sort_by(|a, b| (&a.device, a.order, &a.id).cmp(&(&b.device, b.order, &b.id)));
        out
    }

    pub fn list(&self, f: &ChangeFilter) -> Vec<StagedChange> {
        self.all()
            .into_iter()
            .filter(|c| f.device.as_deref().is_none_or(|d| c.device == d))
            .filter(|c| match f.status {
                Some(s) => c.status == s,
                None => f.history || c.status.staged(),
            })
            .collect()
    }

    /// The staged changes of each device, for the plug-in bar and `GearStatus`.
    pub fn staged_counts(&self) -> std::collections::BTreeMap<String, usize> {
        let mut m = std::collections::BTreeMap::new();
        for c in self.all().into_iter().filter(|c| c.status.staged()) {
            *m.entry(c.device).or_insert(0) += 1;
        }
        m
    }

    fn write(&self, c: &StagedChange) -> Result<()> {
        let dir = self.dir(&c.id);
        std::fs::create_dir_all(&dir)?;
        write_atomic(&dir.join("change.json"), &serde_json::to_vec_pretty(c)?)
    }

    /// Stages a new change. Needs at least one edit.
    pub fn stage(&self, n: NewChange) -> Result<StagedChange> {
        if n.edits.is_empty() {
            bail!("A change needs at least one edit.");
        }
        let day = Utc::now().format("%Y-%m-%d");
        let slug = safe(&n.device);
        std::fs::create_dir_all(self.store.changes_dir())?;
        // create_dir is atomic: a CLI and the app staging at once get different numbers.
        let mut i = 1;
        let id = loop {
            let id = format!("{day}-{slug}-{i}");
            match std::fs::create_dir(self.dir(&id)) {
                Ok(()) => break id,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => i += 1,
                Err(e) => return Err(e.into()),
            }
        };
        let order = self
            .all()
            .iter()
            .filter(|c| c.device == n.device && c.status.staged())
            .map(|c| c.order + 1)
            .max()
            .unwrap_or(0);
        let c = StagedChange {
            id,
            device: n.device,
            title: n.title,
            status: n.status,
            edits: n.edits,
            base_backup: n.base_backup,
            editor: n.editor,
            note: n.note,
            order,
            history: vec![ChangeEvent {
                at: Utc::now(),
                status: n.status,
                note: "Staged".into(),
            }],
            reverts: n.reverts,
        };
        self.write(&c)?;
        Ok(c)
    }

    /// Changes a staged change. Only Draft and Ready are set by hand; the apply engine
    /// sets the rest. An applied change cannot be edited.
    pub fn update(&self, id: &str, u: ChangeUpdate) -> Result<StagedChange> {
        let mut c = self.get(id)?;
        if !c.status.staged() {
            bail!(
                "Change {id} is {:?}; only a staged change can be edited.",
                c.status
            );
        }
        if let Some(t) = u.title {
            c.title = t;
        }
        if let Some(e) = u.edits {
            if e.is_empty() {
                bail!("A change needs at least one edit; discard it instead.");
            }
            c.edits = e;
        }
        if let Some(n) = u.note {
            c.note = n;
        }
        if let Some(o) = u.order {
            c.order = o;
        }
        if let Some(s) = u.status {
            if !matches!(
                s,
                ChangeStatus::Draft
                    | ChangeStatus::Ready
                    | ChangeStatus::Try
                    | ChangeStatus::ReadFirst
            ) {
                bail!(
                    "Status {s:?} is set by the apply engine; set draft, ready, try or read_first, or discard."
                );
            }
            if s != c.status {
                c.status = s;
                c.history.push(ChangeEvent {
                    at: Utc::now(),
                    status: s,
                    note: String::new(),
                });
            }
        }
        self.write(&c)?;
        Ok(c)
    }

    /// Moves a change to a status and records why in its history.
    pub fn set_status(&self, id: &str, status: ChangeStatus, note: &str) -> Result<StagedChange> {
        let mut c = self.get(id)?;
        c.status = status;
        c.history.push(ChangeEvent {
            at: Utc::now(),
            status,
            note: note.to_string(),
        });
        self.write(&c)?;
        Ok(c)
    }

    /// Discards a staged change. It stays as history.
    pub fn discard(&self, id: &str) -> Result<StagedChange> {
        let c = self.get(id)?;
        if !c.status.staged() {
            bail!(
                "Change {id} is {:?}; only a staged change can be discarded.",
                c.status
            );
        }
        self.set_status(id, ChangeStatus::Discarded, "Discarded")
    }

    /// Writes `report.json` next to the change.
    pub fn write_report(&self, id: &str, json: &[u8]) -> Result<()> {
        write_atomic(&self.dir(id).join("report.json"), json)
    }

    pub fn read_report(&self, id: &str) -> Option<Vec<u8>> {
        std::fs::read(self.dir(id).join("report.json")).ok()
    }
}

fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

// ----- FC edits to CLI lines -----

/// A `set` the plan range-checks on the FC right before the write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetRef {
    pub section: Section,
    pub name: String,
    pub value: String,
}

/// An FC change as the write engine sends it.
#[derive(Debug, Clone, Default)]
pub struct FcRender {
    /// Every line sent: selections, the changes, then the restoring selections.
    pub lines: Vec<String>,
    /// The before/after view of `lines`.
    pub diff: Vec<DiffLine>,
    /// What the touched settings hold in the base dump; the digest covers it.
    pub before: Vec<String>,
    /// Each `set`, for the range check.
    pub sets: Vec<SetRef>,
    /// Lines that cannot be sent, as refusals.
    pub problems: Vec<Refusal>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Profile,
    Rate,
    Battery,
}

fn family(s: Section) -> Option<Family> {
    match s {
        Section::Master => None,
        Section::Profile(_) => Some(Family::Profile),
        Section::RateProfile(_) => Some(Family::Rate),
        Section::BatteryProfile(_) => Some(Family::Battery),
    }
}

fn select_of(f: Family, n: u8) -> Section {
    match f {
        Family::Profile => Section::Profile(n),
        Family::Rate => Section::RateProfile(n),
        Family::Battery => Section::BatteryProfile(n),
    }
}

fn index(s: Section) -> u8 {
    match s {
        Section::Master => 0,
        Section::Profile(n) | Section::RateProfile(n) | Section::BatteryProfile(n) => n,
    }
}

fn section_key(s: Section) -> String {
    s.select_line().unwrap_or_else(|| "master".into())
}

/// The profile selection the FC had when the base dump was read: the last selection of
/// each kind (a dump ends with "restore original profile selection").
fn original(base: &Config, f: Family) -> Option<u8> {
    base.lines.iter().rev().find_map(|l| match &l.cmd {
        Some(Cmd::Select(s)) if family(*s) == Some(f) => Some(index(*s)),
        _ => None,
    })
}

fn bad(code: RefusalCode, reason: String) -> Refusal {
    Refusal::new(code, reason)
}

/// One item before selections are added: the section a line belongs to and its text.
struct Item {
    section: Section,
    line: String,
}

/// Renders an FC change. `base` is the device's latest `dump all`; without it the names
/// cannot be checked and the before state is empty.
pub fn render_fc(edits: &[Edit], base: Option<&Config>) -> FcRender {
    let mut out = FcRender::default();
    let mut items: Vec<Item> = Vec::new();
    let push_set =
        |items: &mut Vec<Item>, out: &mut FcRender, section: Section, name: &str, value: &str| {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if name.is_empty() || value.is_empty() {
                out.problems.push(bad(
                    RefusalCode::ShapeUnknown,
                    format!("`set {name} = {value}` has no name or no value; nothing was written."),
                ));
                return;
            }
            if let Some(b) = base {
                let known = b.sections_of(&name);
                if known.is_empty() {
                    out.problems.push(bad(
                        RefusalCode::BadSetting,
                        format!("`{name}` is not a setting on this FC."),
                    ));
                    return;
                }
                let fam = family(section);
                if section == Section::Master && !known.contains(&Section::Master) {
                    out.problems.push(bad(
                        RefusalCode::BadSetting,
                        format!("`{name}` is a profile setting; select a profile first."),
                    ));
                    return;
                }
                if fam.is_some() && !known.contains(&Section::Master) && !known.contains(&section) {
                    out.problems.push(bad(
                        RefusalCode::BadSetting,
                        format!("`{name}` has no {} on this FC.", section_key(section)),
                    ));
                    return;
                }
            }
            out.sets.push(SetRef {
                section,
                name: name.clone(),
                value: value.clone(),
            });
            items.push(Item {
                section,
                line: dump::render_set(&name, &value),
            });
        };
    for e in edits {
        match e {
            Edit::FcSet {
                section,
                name,
                value,
            } => push_set(&mut items, &mut out, *section, name, value),
            Edit::FcLines { lines } => {
                let mut cur = Section::Master;
                for l in dump::cli_lines(&lines.join("\n")) {
                    if let Some(f) = cli::forbidden(&l) {
                        out.problems.push(bad(
                            RefusalCode::ShapeUnknown,
                            format!("`{l}` ({f}) is not sent; QuadCam saves and exits itself."),
                        ));
                        continue;
                    }
                    match parse_cmd(&l) {
                        Cmd::Select(s) => cur = s,
                        Cmd::Control => out.problems.push(bad(
                            RefusalCode::ShapeUnknown,
                            format!("`{l}` is not sent; QuadCam saves and exits itself."),
                        )),
                        Cmd::Set { name, value } => {
                            push_set(&mut items, &mut out, cur, &name, &value)
                        }
                        Cmd::Other { .. } => items.push(Item {
                            section: Section::Master,
                            line: l,
                        }),
                    }
                }
            }
            Edit::OsdElement {
                element,
                x,
                y,
                profiles,
            } => {
                let el = element
                    .trim()
                    .to_ascii_lowercase()
                    .trim_start_matches("osd_")
                    .trim_end_matches("_pos")
                    .to_string();
                let name = format!("osd_{el}_pos");
                if *x > 63 || *y > 31 || profiles.iter().any(|p| !(1..=3).contains(p)) {
                    out.problems.push(bad(
                        RefusalCode::ShapeUnknown,
                        format!(
                            "`{name}` takes x 0-63, y 0-31 and OSD profiles 1-3; nothing was written."
                        ),
                    ));
                } else {
                    // Keep the variant bits the FC holds; move the element and pick its profiles.
                    let held = base
                        .and_then(|b| b.get(Section::Master, &name))
                        .and_then(|v| v.trim().parse::<u16>().ok())
                        .map(Pos::decode);
                    let pos = Pos {
                        x: *x,
                        y: *y,
                        profiles: profiles.iter().fold(0u8, |m, p| m | (1 << (p - 1))),
                        variant: held.map_or(0, |h| h.variant),
                    };
                    push_set(
                        &mut items,
                        &mut out,
                        Section::Master,
                        &name,
                        &pos.encode().to_string(),
                    );
                }
            }
            Edit::FcAux {
                slot,
                mode,
                aux,
                start,
                end,
            } => {
                let tail = tail_words(base, "aux", *slot, 6, "0 0");
                items.push(Item {
                    section: Section::Master,
                    line: format!("aux {slot} {mode} {aux} {start} {end} {tail}"),
                });
            }
            Edit::FcAdjrange {
                slot,
                range_aux,
                start,
                end,
                function,
                select_aux,
            } => {
                let tail = tail_words(base, "adjrange", *slot, 7, "0 0");
                items.push(Item {
                    section: Section::Master,
                    line: format!(
                        "adjrange {slot} 0 {range_aux} {start} {end} {function} {select_aux} {tail}"
                    ),
                });
            }
            other => out.problems.push(bad(
                RefusalCode::ShapeUnknown,
                format!(
                    "{} edits have no FC writer yet; nothing was written.",
                    edit_name(other)
                ),
            )),
        }
    }
    // Selections: switch only when the next item lives elsewhere, and put the FC's own
    // selection back at the end (a saved `profile N` would change the active profile).
    let mut selected: [Option<u8>; 3] = [None; 3];
    let slot = |f: Family| match f {
        Family::Profile => 0,
        Family::Rate => 1,
        Family::Battery => 2,
    };
    let mut touched: Vec<Family> = Vec::new();
    let mut seq: Vec<(Section, String)> = Vec::new();
    for it in &items {
        if let Some(f) = family(it.section) {
            if selected[slot(f)] != Some(index(it.section)) {
                selected[slot(f)] = Some(index(it.section));
                seq.push((it.section, it.section.select_line().unwrap_or_default()));
            }
            if !touched.contains(&f) {
                touched.push(f);
            }
        }
        seq.push((it.section, it.line.clone()));
    }
    if let Some(b) = base {
        for f in touched {
            if let Some(o) = original(b, f) {
                if selected[slot(f)] != Some(o) {
                    let s = select_of(f, o);
                    seq.push((s, s.select_line().unwrap_or_default()));
                }
            }
        }
    }
    for (section, line) in &seq {
        out.lines.push(line.clone());
        let is_select = matches!(parse_cmd(line), Cmd::Select(_));
        if is_select {
            out.diff.push(DiffLine {
                op: LineOp::Same,
                text: line.clone(),
            });
            continue;
        }
        match parse_cmd(line) {
            Cmd::Set { name, value } => {
                let before = base
                    .and_then(|b| b.get(*section, &name))
                    .map(str::to_string);
                out.before.push(format!(
                    "{}:{name}={}",
                    section_key(*section),
                    before.as_deref().unwrap_or("-")
                ));
                diff_pair(
                    &mut out.diff,
                    before.map(|v| dump::render_set(&name, &v)),
                    line,
                    |a, b| {
                        let (x, y) = (parse_cmd(a), parse_cmd(b));
                        matches!((x, y), (Cmd::Set { value: v1, .. }, Cmd::Set { .. })
                            if v1.eq_ignore_ascii_case(&value))
                    },
                );
            }
            Cmd::Other { key, .. } => {
                let before = base
                    .and_then(|b| b.other(Section::Master, &key))
                    .map(str::to_string);
                out.before
                    .push(format!("{key}={}", before.as_deref().unwrap_or("-")));
                let same = before.as_deref() == Some(line.as_str());
                diff_pair(&mut out.diff, before, line, move |_, _| same);
            }
            _ => {}
        }
    }
    out
}

/// Remove the old line and add the new, or one unchanged line when they agree.
fn diff_pair(
    diff: &mut Vec<DiffLine>,
    before: Option<String>,
    line: &str,
    same: impl Fn(&str, &str) -> bool,
) {
    match before {
        Some(old) if same(&old, line) => diff.push(DiffLine {
            op: LineOp::Same,
            text: line.to_string(),
        }),
        Some(old) => {
            diff.push(DiffLine {
                op: LineOp::Remove,
                text: old,
            });
            diff.push(DiffLine {
                op: LineOp::Add,
                text: line.to_string(),
            });
        }
        None => diff.push(DiffLine {
            op: LineOp::Add,
            text: line.to_string(),
        }),
    }
}

/// The trailing words of the base's `verb slot ...` line from word `from`, else `dflt`.
fn tail_words(base: Option<&Config>, verb: &str, slot: u8, from: usize, dflt: &str) -> String {
    base.and_then(|b| b.other(Section::Master, &format!("{verb} {slot}")))
        .map(|l| {
            l.split_whitespace()
                .skip(from)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| dflt.to_string())
}

fn edit_name(e: &Edit) -> &'static str {
    match e {
        Edit::OsdElement { .. } => "OSD element",
        Edit::CardFiles { .. } => "Card file",
        Edit::Model { .. } | Edit::Radio { .. } | Edit::Checklist { .. } => "Radio",
        Edit::ModelCopy { .. } | Edit::ModelDelete { .. } => "Radio model",
        Edit::Restore { .. } => "Restore",
        Edit::ElrsOptions { .. } => "ELRS",
        _ => "These",
    }
}

/// Lines that make `base` read as `target`: every `set` whose value differs, and the
/// list-like commands (modes, adjustments, features) whose text differs. Restoring a
/// backup stages these; resources, serial ports and timers stay as they are.
pub fn restore_lines(target: &Config, base: &Config) -> Vec<String> {
    const VERBS: &[&str] = &[
        "aux",
        "adjrange",
        "rxrange",
        "feature",
        "beeper",
        "beacon",
        "led",
        "color",
        "mode_color",
        "vtx",
        "mmix",
        "smix",
        "servo",
        "rxfail",
    ];
    let mut out = Vec::new();
    let mut section = Section::Master;
    for l in &target.lines {
        let Some(cmd) = &l.cmd else { continue };
        match cmd {
            Cmd::Set { name, value } => {
                let now = base.get(l.section, name);
                if now.is_some_and(|n| !n.eq_ignore_ascii_case(value)) {
                    if l.section != section {
                        if let Some(s) = l.section.select_line() {
                            out.push(s);
                        }
                        section = l.section;
                    }
                    out.push(dump::render_set(name, value));
                }
            }
            Cmd::Other { verb, key } if VERBS.contains(&verb.as_str()) => {
                let t = l.text.trim();
                // A dump lists every feature off, then the ones on: only the last line
                // of a key says what the FC holds.
                if target.other(l.section, key) != Some(t) {
                    continue;
                }
                if base.other(l.section, key).is_some_and(|b| b != t) {
                    out.push(t.to_string());
                }
            }
            _ => {}
        }
    }
    out
}

/// One setting or list-like line a change touches, as the revert and its overlap check
/// name it: `("rateprofile 1", "set roll_expo")`.
pub type Touch = (Section, String);

/// What the rendered CLI lines touch: each `set` by name, each list-like command by its key.
pub fn touched(lines: &[String]) -> Vec<Touch> {
    let mut section = Section::Master;
    let mut out: Vec<Touch> = Vec::new();
    for l in lines {
        match dump::parse_cmd(l) {
            Cmd::Select(s) => section = s,
            Cmd::Set { name, .. } => out.push((section, format!("set {name}"))),
            Cmd::Other { verb, key } => out.push((section, format!("{verb} {key}"))),
            Cmd::Control => {}
        }
    }
    out.dedup();
    out
}

/// The lines that put back what `lines` changed: each touched `set` takes its value from
/// `before` (the dump taken just before the apply), each list-like command its old line.
/// The second list names what `before` cannot give back (a line that did not exist).
pub fn inverse_lines(lines: &[String], before: &Config) -> (Vec<String>, Vec<String>) {
    let mut section = Section::Master;
    let mut shown = Section::Master;
    let mut out = Vec::new();
    let mut lost = Vec::new();
    for l in lines {
        let (key, old): (String, Option<String>) = match dump::parse_cmd(l) {
            Cmd::Select(s) => {
                section = s;
                continue;
            }
            Cmd::Set { name, .. } => (
                format!("set {name}"),
                before
                    .get(section, &name)
                    .map(|v| dump::render_set(&name, v)),
            ),
            Cmd::Other { verb, key } => (
                format!("{verb} {key}"),
                before.other(section, &key).map(str::to_string),
            ),
            Cmd::Control => continue,
        };
        match old {
            Some(line) => {
                if section != shown {
                    if let Some(s) = section.select_line() {
                        out.push(s);
                    }
                    shown = section;
                }
                if !out.contains(&line) {
                    out.push(line);
                }
            }
            None => lost.push(key),
        }
    }
    (out, lost)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Changes) {
        let d = tempfile::tempdir().unwrap();
        let c = Changes::new(Store::new(d.path().join("gear")));
        (d, c)
    }

    fn new(device: &str) -> NewChange {
        NewChange {
            device: device.into(),
            title: "t".into(),
            edits: vec![Edit::FcLines {
                lines: vec!["set osd_cap_alarm = 100".into()],
            }],
            base_backup: String::new(),
            editor: Editor::User,
            note: String::new(),
            status: ChangeStatus::Ready,
            reverts: None,
        }
    }

    #[test]
    fn stage_list_update_discard() {
        let (_d, c) = store();
        let a = c.stage(new("fc-1")).unwrap();
        let b = c.stage(new("fc-1")).unwrap();
        assert!(
            a.id.ends_with("-fc-1-1") && b.id.ends_with("-fc-1-2"),
            "{}",
            a.id
        );
        assert_eq!((a.order, b.order), (0, 1));
        assert_eq!(c.list(&ChangeFilter::default()).len(), 2);
        assert_eq!(c.staged_counts()["fc-1"], 2);
        let u = c
            .update(
                &a.id,
                ChangeUpdate {
                    status: Some(ChangeStatus::Draft),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(u.status, ChangeStatus::Draft);
        assert_eq!(u.history.len(), 2);
        assert!(c
            .update(
                &a.id,
                ChangeUpdate {
                    status: Some(ChangeStatus::Verified),
                    ..Default::default()
                }
            )
            .is_err());
        c.discard(&a.id).unwrap();
        assert_eq!(c.list(&ChangeFilter::default()).len(), 1);
        let h = ChangeFilter {
            history: true,
            ..Default::default()
        };
        assert_eq!(c.list(&h).len(), 2);
        assert!(c.discard(&a.id).is_err(), "already discarded");
        assert!(c.update(&a.id, ChangeUpdate::default()).is_err());
        let mut empty = new("fc-1");
        empty.edits.clear();
        assert!(c.stage(empty).is_err());
    }

    const DUMP: &str = "# Betaflight / STM32G47X (G473) 2025.12.5-alpha Jun 25 2026 / 03:24:50 (eb2bb5a33) MSP API: 1.47\nset osd_cap_alarm = 2200\naux 0 0 0 1700 2100 0 0\nprofile 1\nset p_roll = 40\nprofile 0\nset p_roll = 45\nprofile 1\nrateprofile 0\n";

    #[test]
    fn render_selects_and_restores_the_selection() {
        let base = Config::parse(DUMP);
        let r = render_fc(
            &[
                Edit::FcSet {
                    section: Section::Profile(0),
                    name: "p_roll".into(),
                    value: "50".into(),
                },
                Edit::FcSet {
                    section: Section::Master,
                    name: "osd_cap_alarm".into(),
                    value: "100".into(),
                },
            ],
            Some(&base),
        );
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        // The dump ends on profile 1: select 0 to edit, then back to 1.
        assert_eq!(
            r.lines,
            [
                "profile 0",
                "set p_roll = 50",
                "set osd_cap_alarm = 100",
                "profile 1"
            ]
        );
        assert!(r.before.contains(&"profile 0:p_roll=45".to_string()));
        let ops: Vec<_> = r.diff.iter().map(|l| (l.op, l.text.as_str())).collect();
        assert!(ops.contains(&(LineOp::Remove, "set p_roll = 45")));
        assert!(ops.contains(&(LineOp::Add, "set p_roll = 50")));
    }

    #[test]
    fn render_refuses_what_cannot_be_sent() {
        let base = Config::parse(DUMP);
        let r = render_fc(
            &[Edit::FcLines {
                lines: vec![
                    "set nope = 1".into(),
                    "save".into(),
                    "set p_roll = 3".into(),
                    "set osd_cap_alarm".into(),
                ],
            }],
            Some(&base),
        );
        let codes: Vec<_> = r.problems.iter().map(|p| p.code).collect();
        assert_eq!(
            codes,
            [
                RefusalCode::BadSetting,
                RefusalCode::ShapeUnknown,
                RefusalCode::BadSetting,
                RefusalCode::ShapeUnknown
            ]
        );
        assert!(r.problems[2].reason.contains("select a profile"));
        let osd = render_fc(
            &[Edit::OsdElement {
                element: "x".into(),
                x: 1,
                y: 1,
                profiles: vec![],
            }],
            Some(&base),
        );
        assert_eq!(osd.problems[0].code, RefusalCode::BadSetting);
    }

    #[test]
    fn osd_element_move_is_one_set_line() {
        // vbat at x 12, y 3 on OSD profiles 1 and 2, variant 1: 12 | 3 << 5 | 3 << 11 | 1 << 14.
        let held = Pos {
            x: 12,
            y: 3,
            profiles: 3,
            variant: 1,
        }
        .encode();
        let base = Config::parse(&DUMP.replacen(
            "set osd_cap_alarm = 2200\n",
            &format!("set osd_cap_alarm = 2200\nset osd_vbat_pos = {held}\n"),
            1,
        ));
        let r = render_fc(
            &[Edit::OsdElement {
                element: "vbat".into(),
                x: 20,
                y: 9,
                profiles: vec![1, 3],
            }],
            Some(&base),
        );
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        let want = Pos {
            x: 20,
            y: 9,
            profiles: 0b101,
            variant: 1,
        }
        .encode();
        assert_eq!(r.lines, [format!("set osd_vbat_pos = {want}")]);
        assert_eq!(r.before, [format!("master:osd_vbat_pos={held}")]);
        let bad = render_fc(
            &[Edit::OsdElement {
                element: "osd_vbat_pos".into(),
                x: 64,
                y: 1,
                profiles: vec![],
            }],
            Some(&base),
        );
        assert_eq!(bad.problems[0].code, RefusalCode::ShapeUnknown);
        let off = render_fc(
            &[Edit::OsdElement {
                element: "vbat".into(),
                x: 1,
                y: 1,
                profiles: vec![],
            }],
            Some(&base),
        );
        assert_eq!(off.lines.len(), 1, "an empty profile list turns it off");
    }

    #[test]
    fn aux_keeps_the_trailing_words() {
        let base = Config::parse(DUMP);
        let r = render_fc(
            &[Edit::FcAux {
                slot: 0,
                mode: 0,
                aux: 1,
                start: 1300,
                end: 2100,
            }],
            Some(&base),
        );
        assert_eq!(r.lines, ["aux 0 0 1 1300 2100 0 0"]);
        assert!(r.diff.iter().any(|l| l.op == LineOp::Remove));
    }

    #[test]
    fn touched_names_each_set_and_list_command_with_its_section() {
        let lines: Vec<String> = [
            "rateprofile 1",
            "set roll_expo = 40",
            "set roll_expo = 41",
            "aux 0 0 1 900 2100 0 0",
            "set osd_cap_alarm = 9",
        ]
        .map(String::from)
        .to_vec();
        let t = touched(&lines);
        assert!(t.contains(&(Section::RateProfile(1), "set roll_expo".to_string())));
        assert!(t
            .iter()
            .any(|(s, k)| *s == Section::RateProfile(1) && k.starts_with("aux")));
        assert!(t.contains(&(Section::RateProfile(1), "set osd_cap_alarm".to_string())));
    }

    #[test]
    fn inverse_lines_take_old_values_and_report_what_was_missing() {
        let before = Config::parse(
            "set a = 1
rateprofile 1
set roll_expo = 10
",
        );
        let lines: Vec<String> = [
            "set a = 5",
            "rateprofile 1",
            "set roll_expo = 40",
            "set new = 1",
        ]
        .map(String::from)
        .to_vec();
        let (inv, lost) = inverse_lines(&lines, &before);
        assert_eq!(inv, ["set a = 1", "rateprofile 1", "set roll_expo = 10"]);
        assert_eq!(lost, ["set new"]);
    }

    #[test]
    fn restore_lines_make_the_base_read_as_the_target() {
        let base = Config::parse("set a = 1\nset b = 2\naux 0 0 0 900 2100 0 0\n");
        let target = Config::parse("set a = 1\nset b = 9\naux 0 0 1 900 2100 0 0\n");
        assert_eq!(
            restore_lines(&target, &base),
            ["set b = 9", "aux 0 0 1 900 2100 0 0"]
        );
    }
}
