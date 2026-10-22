//! The generator: a setup and the card it is for become card edits (`Edit::Model`,
//! `Edit::Checklist`, `Edit::ModelCopy`, `Edit::Radio`, `Edit::ModelDelete`,
//! `Edit::CardText`). It reads the card for what it owns there (special functions, logical
//! switches, scripts), and checks each model's ops by applying them to a copy. A second run
//! on a card the first one wrote gives edits that change no file.

use super::controls::{self, is_switch};
use super::lua::{self, FinderControl};
use super::{Control, LsSpec, LuaKind, RadioControlKind, RadioSetup, ScreenSpec, SfSpec};
use crate::gear::edgetx::card::{is_model_file, RadioOp, RADIO_FILE};
use crate::gear::edgetx::model::{
    self as em, LsDef, MixLine, ModelOp, SfDef, SwitchWarning, MAX_LOGICAL_SWITCHES,
};
use crate::gear::edgetx::yaml::{unquote, Doc};
use crate::gear::model::{Edit, Refusal, RefusalCode};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::{BTreeMap, BTreeSet};

/// The card files the generator reads.
pub trait Source {
    /// A file's bytes, from the card's root (`MODELS/model00.yml`).
    fn read(&self, path: &str) -> Option<Vec<u8>>;
    /// Every file's path from the card's root.
    fn files(&self) -> Vec<String>;
}

/// One position of a job's control and what it does.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct StateRow {
    /// What the state does, in the job's words.
    pub words: String,
    /// The control's position (`SC2`), for a switch.
    #[serde(default)]
    pub position: Option<String>,
    /// The channel's value in microseconds, as QuadCam shows it (1000, 1500, 2000).
    #[serde(default)]
    pub us: Option<u32>,
}

/// One job as the editor shows it: its channel, control and what each position does.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct JobRow {
    pub job: String,
    pub label: String,
    /// `CH5`.
    #[serde(default)]
    pub channel: Option<String>,
    /// None: no control does it.
    #[serde(default)]
    pub control: Option<String>,
    #[serde(default)]
    pub kind: Option<RadioControlKind>,
    #[serde(default)]
    pub high: Option<u8>,
    pub states: Vec<StateRow>,
    /// Controls of the board that fit the job and do no other job.
    pub free_controls: Vec<String>,
}

/// A sound a special function plays, and whether the card has it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct SoundCheck {
    pub track: String,
    /// `SOUNDS/en/<track>.wav`.
    pub path: String,
    pub present: bool,
}

/// What a setup gives on one card.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct Generated {
    pub edits: Vec<Edit>,
    pub jobs: Vec<JobRow>,
    pub sounds: Vec<SoundCheck>,
    /// Things to know that do not refuse: a missing sound, a script someone else wrote.
    pub warnings: Vec<String>,
}

fn bad(reason: impl Into<String>) -> Refusal {
    Refusal::new(RefusalCode::BadSetting, reason)
}

/// A job, the control doing it, and the assignment's direction.
struct Bound<'a> {
    control: &'a Control,
    channel: Option<u32>,
    high: u8,
}

struct Ctx<'a> {
    bound: BTreeMap<&'a str, Bound<'a>>,
    jobs: BTreeSet<&'a str>,
}

impl Ctx<'_> {
    fn assigned(&self, job: Option<&str>) -> bool {
        job.is_none_or(|j| self.bound.contains_key(j))
    }

    /// Replaces each `@job[.part]` token.
    fn resolve(&self, text: &str) -> Result<String, Refusal> {
        let mut out = String::new();
        let mut rest = text;
        while let Some(i) = rest.find('@') {
            out.push_str(&rest[..i]);
            let tail = &rest[i + 1..];
            let n = tail
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
                .unwrap_or(tail.len());
            let token = &tail[..n];
            let (job, part) = token.split_once('.').unwrap_or((token, ""));
            let b = self.bound.get(job).ok_or_else(|| {
                if self.jobs.contains(job) {
                    bad(format!(
                        "The template uses @{job}, but no control does that job; give it a control or mark the item with its job."
                    ))
                } else {
                    bad(format!("The template names @{job}, which is not a job."))
                }
            })?;
            let name = &b.control.name;
            let pos = |p: u8| -> Result<String, Refusal> {
                if !b.control.kind.has_positions() {
                    return Err(bad(format!(
                        "@{token}: {name} is not a switch, so it has no positions."
                    )));
                }
                Ok(format!("{name}{p}"))
            };
            let s = match part {
                "" => name.clone(),
                "hi" => pos(b.high)?,
                "lo" => pos(2 - b.high)?,
                "mid" => {
                    if b.control.kind != RadioControlKind::ThreePos {
                        return Err(bad(format!("@{token}: {name} has no middle position.")));
                    }
                    pos(1)?
                }
                "ch" => b
                    .channel
                    .ok_or_else(|| bad(format!("@{token}: the job {job} has no channel.")))?
                    .to_string(),
                other => {
                    return Err(bad(format!(
                        "@{job}.{other} is not a token (hi, lo, mid, ch)."
                    )))
                }
            };
            out.push_str(&s);
            rest = &tail[n..];
        }
        out.push_str(rest);
        Ok(out)
    }

    /// The direction a job's mix source takes: -1 when position 0 sends 2000 µs.
    fn sign(&self, source: &str) -> i32 {
        source
            .strip_prefix('@')
            .and_then(|j| self.bound.get(j))
            .map_or(1, |b| {
                if b.control.kind.has_positions() && b.high == 0 {
                    -1
                } else {
                    1
                }
            })
    }

    fn sf(&self, s: &SfDef) -> Result<SfDef, Refusal> {
        Ok(SfDef {
            swtch: self.resolve(&s.swtch)?,
            func: s.func.clone(),
            def: self.resolve(&s.def)?,
        })
    }
}

/// One thing wrong with a setup, at the job (or slot) it is about.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct SetupProblem {
    /// The job it is about, when it is about one.
    #[serde(default)]
    pub job: Option<String>,
    /// The control it is about, when it is about one.
    #[serde(default)]
    pub control: Option<String>,
    pub message: String,
}

fn problem(job: Option<&str>, control: Option<&str>, message: String) -> SetupProblem {
    SetupProblem {
        job: job.map(str::to_string),
        control: control.map(str::to_string),
        message,
    }
}

/// Checks a setup against its board: every assignment names a job and a control of the
/// board whose kind the job takes, a switch's direction is 0 or 2, and no control or
/// channel does two jobs. Returns the board's controls with the fitted parts in place.
pub fn check(setup: &RadioSetup) -> Result<Vec<Control>, Refusal> {
    let (controls, problems) = problems(setup);
    if let Some(p) = problems.into_iter().next() {
        let code = if controls::controls(&setup.board).is_none() {
            RefusalCode::UnknownBoard
        } else {
            RefusalCode::BadSetting
        };
        return Err(Refusal::new(code, p.message));
    }
    Ok(controls.unwrap_or_default())
}

/// Every problem `check` would refuse, each at its job, and the board's controls with the
/// fitted parts in place (None when the board or a fitted part is unknown).
pub fn problems(setup: &RadioSetup) -> (Option<Vec<Control>>, Vec<SetupProblem>) {
    let mut out = Vec::new();
    let controls = match controls::fitted_controls(&setup.board, &setup.fitted) {
        Ok(Some(c)) => c,
        Ok(None) => {
            out.push(problem(
                None,
                None,
                format!(
                    "QuadCam has no control list for the board {:?}; a setup needs one.",
                    setup.board
                ),
            ));
            return (None, out);
        }
        Err(r) => {
            out.push(problem(None, None, r.reason));
            return (None, out);
        }
    };
    let mut ids = BTreeSet::new();
    let mut channels = BTreeMap::new();
    for j in &setup.jobs {
        if j.id.is_empty()
            || j.id
                .contains(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        {
            out.push(problem(
                None,
                None,
                format!("Job id {:?} may hold letters, digits and _ only.", j.id),
            ));
            continue;
        }
        if !ids.insert(j.id.as_str()) {
            out.push(problem(
                None,
                None,
                format!("Two jobs are named {:?}.", j.id),
            ));
        }
        if let Some(ch) = j.channel {
            if ch >= 32 {
                out.push(problem(
                    Some(&j.id),
                    None,
                    format!("{}: a channel is CH1-CH32.", j.label),
                ));
            } else if let Some(other) = channels.insert(ch, &j.label) {
                out.push(problem(
                    Some(&j.id),
                    None,
                    format!("{} and {} both drive CH{}.", other, j.label, ch + 1),
                ));
            }
        }
    }
    let mut used: BTreeMap<&str, &str> = BTreeMap::new();
    let mut seen_jobs = BTreeSet::new();
    for a in &setup.assignments {
        let Some(job) = setup.job(&a.job) else {
            out.push(problem(
                None,
                Some(&a.control),
                format!("The assignment names {:?}, which is not a job.", a.job),
            ));
            continue;
        };
        let at = |m: String| problem(Some(&a.job), Some(&a.control), m);
        if !seen_jobs.insert(a.job.as_str()) {
            out.push(at(format!("{} has two controls.", job.label)));
            continue;
        }
        let Some(c) = controls.iter().find(|c| c.name == a.control) else {
            let blanked = setup
                .fitted
                .iter()
                .any(|f| f.control == a.control && f.radio_type.eq_ignore_ascii_case("none"));
            out.push(at(if blanked {
                format!(
                    "{} is set as not fitted, so it cannot do {}.",
                    a.control, job.label
                )
            } else {
                format!(
                    "{} is not a control of the {} ({}).",
                    a.control,
                    setup.board,
                    controls
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }));
            continue;
        };
        if !job.fits.contains(&c.kind) {
            out.push(at(format!(
                "{} cannot do {}: it is a {}, and {} takes {}.",
                c.name,
                job.label,
                kind_words(c.kind),
                job.label,
                job.fits
                    .iter()
                    .map(|k| kind_words(*k))
                    .collect::<Vec<_>>()
                    .join(" or ")
            )));
            continue;
        }
        if c.kind == RadioControlKind::Custom {
            out.push(at(format!(
                "{} is a customizable switch; QuadCam does not set those up yet.",
                c.name
            )));
            continue;
        }
        if c.kind.has_positions() {
            if !matches!(a.high, Some(0) | Some(2)) {
                out.push(at(format!(
                    "{} on {}: say which position (0 or 2) sends 2000 µs.",
                    job.label, c.name
                )));
            }
        } else if a.high.is_some() {
            out.push(at(format!(
                "{} on {}: a {} has no positions; leave the direction out.",
                job.label,
                c.name,
                kind_words(c.kind)
            )));
        }
        if let Some(other) = used.insert(c.name.as_str(), job.label.as_str()) {
            out.push(at(format!(
                "{} does two jobs: {} and {}.",
                c.name, other, job.label
            )));
        }
    }
    let mut files = BTreeSet::new();
    for a in &setup.aircraft {
        if !is_model_file(&a.file) {
            out.push(problem(
                None,
                None,
                format!("{:?} is not a model file name (model01.yml).", a.file),
            ));
            continue;
        }
        if !files.insert(a.file.as_str()) {
            out.push(problem(None, None, format!("Two aircraft use {}.", a.file)));
        }
        if let Some(n) = &a.name {
            if let Err(r) = em::check_model_name(n) {
                out.push(problem(None, None, r.reason));
            }
        }
    }
    if setup.aircraft.is_empty() {
        out.push(problem(
            None,
            None,
            "A setup names at least one model.".into(),
        ));
    }
    (Some(controls), out)
}

fn kind_words(k: RadioControlKind) -> &'static str {
    match k {
        RadioControlKind::TwoPos => "2-position switch",
        RadioControlKind::ThreePos => "3-position switch",
        RadioControlKind::Momentary => "momentary switch",
        RadioControlKind::Custom => "customizable switch",
        RadioControlKind::Pot => "pot",
        RadioControlKind::Slider => "slider",
        RadioControlKind::Trim => "trim",
    }
}

/// The job table: each job, its control and what each position does.
pub fn job_rows(setup: &RadioSetup, controls: &[Control]) -> Vec<JobRow> {
    let used: BTreeSet<&str> = setup
        .assignments
        .iter()
        .map(|a| a.control.as_str())
        .collect();
    setup
        .jobs
        .iter()
        .map(|j| {
            let a = setup.assignment(&j.id);
            let c = a.and_then(|a| controls.iter().find(|c| c.name == a.control));
            let n = j.states.len();
            let states = j
                .states
                .iter()
                .enumerate()
                .map(|(i, w)| {
                    // 1000 µs to 2000 µs across the states.
                    let us = (n > 1).then(|| 1000 + (1000 * i / (n - 1)) as u32);
                    let position = match (c, a.and_then(|a| a.high)) {
                        (Some(c), Some(high)) if c.kind.has_positions() => {
                            let at_high = if n > 1 { i * 2 / (n - 1) } else { 2 };
                            // `at_high` runs 0 (1000 µs), 1, 2 (2000 µs).
                            let p = if high == 2 { at_high } else { 2 - at_high };
                            (c.kind == RadioControlKind::ThreePos || p != 1)
                                .then(|| format!("{}{p}", c.name))
                        }
                        _ => None,
                    };
                    StateRow {
                        words: w.clone(),
                        position,
                        us,
                    }
                })
                .collect();
            JobRow {
                job: j.id.clone(),
                label: j.label.clone(),
                channel: j.channel.map(|c| format!("CH{}", c + 1)),
                control: c.map(|c| c.name.clone()),
                kind: c.map(|c| c.kind),
                high: a.and_then(|a| a.high),
                states,
                free_controls: controls
                    .iter()
                    .filter(|c| j.fits.contains(&c.kind) && c.kind != RadioControlKind::Custom)
                    .filter(|c| !used.contains(c.name.as_str()))
                    .map(|c| c.name.clone())
                    .collect(),
            }
        })
        .collect()
}

/// The setup's special functions on the card: the template's own and owned kinds.
struct Ownership {
    funcs: BTreeSet<String>,
    tracks: BTreeSet<String>,
    switch_funcs: BTreeSet<(String, String)>,
    prefixes: Vec<(String, String)>,
}

impl Ownership {
    fn owns(&self, sf: &SfDef) -> bool {
        let track = sf.def.split(',').next().unwrap_or("");
        self.funcs.contains(&sf.func)
            || (sf.func == "PLAY_TRACK" && self.tracks.contains(track))
            || self
                .switch_funcs
                .contains(&(sf.swtch.clone(), sf.func.clone()))
            || self
                .prefixes
                .iter()
                .any(|(f, p)| *f == sf.func && sf.def.starts_with(p.as_str()))
    }
}

/// Each model's merged parts: the template with the aircraft's overrides.
struct ModelParts {
    lss: BTreeMap<u32, Option<LsDef>>,
    functions: Vec<SfDef>,
    keep: Vec<SfDef>,
}

fn parts(setup: &RadioSetup, ctx: &Ctx<'_>, a: &super::Aircraft) -> Result<ModelParts, Refusal> {
    let t = &setup.template;
    let mut lss: BTreeMap<u32, Option<LsDef>> = BTreeMap::new();
    let mut put = |l: &LsSpec| -> Result<(), Refusal> {
        if l.index >= MAX_LOGICAL_SWITCHES {
            return Err(bad(format!(
                "Logical switches are L1-L{MAX_LOGICAL_SWITCHES}."
            )));
        }
        let v = match (&l.ls, ctx.assigned(l.job.as_deref())) {
            (Some(d), true) => Some(LsDef {
                func: d.func.clone(),
                def: ctx.resolve(&d.def)?,
                andsw: ctx.resolve(&d.andsw)?,
                delay: d.delay,
                duration: d.duration,
            }),
            _ => None,
        };
        lss.insert(l.index, v);
        Ok(())
    };
    for l in &t.logical_switches {
        put(l)?;
    }
    for l in &a.logical_switches {
        put(l)?;
    }
    let pick = |list: &[SfSpec], drop: &[String]| -> Result<Vec<SfDef>, Refusal> {
        let mut out = Vec::new();
        for s in list {
            if !ctx.assigned(s.job.as_deref()) || drop.contains(&s.sf.swtch) {
                continue;
            }
            out.push(ctx.sf(&s.sf)?);
        }
        Ok(out)
    };
    let mut functions = pick(&t.functions, &a.drop_functions)?;
    functions.extend(pick(&a.functions, &[])?);
    let keep = pick(&t.keep_functions, &[])?;
    Ok(ModelParts {
        lss,
        functions,
        keep,
    })
}

fn ownership(setup: &RadioSetup, ctx: &Ctx<'_>, functions: &[SfDef]) -> Result<Ownership, Refusal> {
    let o = &setup.template.owns;
    let mut own = Ownership {
        funcs: o.funcs.iter().cloned().collect(),
        tracks: o.tracks.iter().cloned().collect(),
        switch_funcs: BTreeSet::new(),
        prefixes: o
            .def_prefixes
            .iter()
            .map(|p| (p.func.clone(), p.def.clone()))
            .collect(),
    };
    for sw in &o.switch_funcs {
        own.switch_funcs
            .insert((ctx.resolve(&sw.swtch)?, sw.func.clone()));
    }
    // A function the setup lists is its own: a track it plays, a function on a switch.
    let every = setup
        .template
        .functions
        .iter()
        .chain(setup.aircraft.iter().flat_map(|a| a.functions.iter()))
        .filter_map(|s| ctx.assigned(s.job.as_deref()).then_some(&s.sf));
    for sf in every {
        if sf.func == "PLAY_TRACK" {
            own.tracks
                .insert(sf.def.split(',').next().unwrap_or("").to_string());
        } else {
            own.switch_funcs
                .insert((ctx.resolve(&sf.swtch)?, sf.func.clone()));
        }
    }
    for sf in functions {
        if sf.func != "PLAY_TRACK" {
            own.switch_funcs.insert((sf.swtch.clone(), sf.func.clone()));
        }
    }
    Ok(own)
}

/// The model ops of one aircraft on the model as `doc` holds it.
fn model_ops(
    setup: &RadioSetup,
    ctx: &Ctx<'_>,
    a: &super::Aircraft,
    doc: &Doc,
) -> Result<Vec<ModelOp>, Refusal> {
    let t = &setup.template;
    let view = em::view(&doc.name, doc)?;
    let mut ops = Vec::new();
    if let Some(n) = &a.name {
        if &view.name != n {
            ops.push(ModelOp::Rename { name: n.clone() });
        }
    }
    if let Some(id) = a.model_id {
        ops.push(ModelOp::SetModelId { module: 0, id });
    }
    for tm in &t.timers {
        if a.drop_timers.contains(&tm.index) {
            continue;
        }
        let mut fields = Vec::new();
        for f in &tm.fields {
            fields.push(em::Field::new(&f.key, &ctx.resolve(&f.value)?));
        }
        ops.push(ModelOp::SetTimer {
            index: tm.index,
            fields,
        });
    }
    for i in &a.drop_timers {
        if view.timers.iter().any(|x| x.index == *i) {
            ops.push(ModelOp::RemoveTimer { index: *i });
        }
    }
    if let Some(on) = t.checklist_on {
        ops.push(ModelOp::SetChecklist { enabled: on });
    }
    if !t.flags.is_empty() {
        ops.push(ModelOp::SetFlags {
            flags: t.flags.clone(),
        });
    }
    // Mixes, channel by channel; a channel's lines in template order.
    let mut channels: BTreeMap<u32, Vec<MixLine>> = BTreeMap::new();
    for m in &t.mixes {
        let lines = channels.entry(m.channel).or_default();
        if !ctx.assigned(m.job.as_deref()) {
            continue;
        }
        lines.push(MixLine {
            source: ctx.resolve(&m.line.source)?,
            weight: m.line.weight * ctx.sign(&m.line.source),
            swtch: ctx.resolve(&m.line.swtch)?,
            mltpx: m.line.mltpx.clone(),
        });
    }
    for (channel, lines) in channels {
        ops.push(ModelOp::SetMixes { channel, lines });
    }
    // Logical switches: the setup owns them all.
    let p = parts(setup, ctx, a)?;
    let mut indexes: BTreeSet<u32> = p.lss.keys().copied().collect();
    indexes.extend(view.logical_switches.iter().map(|l| l.index));
    for index in indexes {
        ops.push(ModelOp::SetLogicalSwitch {
            index,
            ls: p.lss.get(&index).cloned().flatten(),
        });
    }
    // Special functions: those the setup owns go, then its own come back in order after the
    // others. Kept ones are added only when missing.
    let own = ownership(setup, ctx, &p.functions)?;
    let existing: Vec<SfDef> = view
        .special_functions
        .iter()
        .map(|s| SfDef {
            swtch: s.swtch.clone(),
            func: s.func.clone(),
            def: s.def.clone(),
        })
        .collect();
    let remove: Vec<SfDef> = existing.iter().filter(|s| own.owns(s)).cloned().collect();
    let mut add: Vec<SfDef> = p.keep.clone();
    add.extend(p.functions.iter().cloned());
    ops.push(ModelOp::SpecialFunctions { remove, add });
    let mut warnings = Vec::new();
    for j in &t.switch_warnings {
        let Some(b) = ctx.bound.get(j.as_str()) else {
            continue;
        };
        if !is_switch(b.control) {
            return Err(bad(format!(
                "A power-on warning needs a switch; {} is not one.",
                b.control.name
            )));
        }
        warnings.push(SwitchWarning {
            switch: b.control.name.clone(),
            pos: ["up", "mid", "down"][(2 - b.high) as usize].into(),
        });
    }
    ops.push(ModelOp::SetSwitchWarnings { warnings });
    for s in &t.screens {
        match s {
            ScreenSpec::Values { index, lines } => ops.push(ModelOp::SetScreenValues {
                index: *index,
                lines: lines.clone(),
            }),
            ScreenSpec::Script { index, script } => {
                // A finder screen without a finder control is left as the model has it.
                let lua = t.lua.iter().find(|l| &l.name == script);
                if let Some(LuaKind::Finder { job }) = lua.map(|l| &l.kind) {
                    if !ctx.bound.contains_key(job.as_str()) {
                        continue;
                    }
                }
                ops.push(ModelOp::SetScreen {
                    index: *index,
                    script: Some(script.clone()),
                });
            }
        }
    }
    Ok(ops)
}

fn parse(path: &str, bytes: &[u8]) -> Result<Doc, Refusal> {
    Doc::parse(path.rsplit('/').next().unwrap_or(path), bytes)
}

/// The edits that put `setup` on the card `src` holds. Refuses a setup that does not fit
/// its board or the card (a model with another aircraft's name, a sensor the model lacks).
pub fn generate(setup: &RadioSetup, src: &dyn Source) -> Result<Generated, Refusal> {
    let controls = check(setup)?;
    let mut ctx = Ctx {
        bound: BTreeMap::new(),
        jobs: setup.jobs.iter().map(|j| j.id.as_str()).collect(),
    };
    for asg in &setup.assignments {
        let control = controls.iter().find(|c| c.name == asg.control).unwrap();
        let job = setup.job(&asg.job).unwrap();
        ctx.bound.insert(
            asg.job.as_str(),
            Bound {
                control,
                channel: job.channel,
                high: asg.high.unwrap_or(2),
            },
        );
    }
    let radio_bytes = src.read(RADIO_FILE);
    if let Some(b) = &radio_bytes {
        let board = crate::gear::edgetx::card::identity_from_radio_yml(b).board;
        if let Some(board) = board.filter(|b| !b.eq_ignore_ascii_case(&setup.board)) {
            return Err(Refusal::new(
                RefusalCode::Incompatible,
                format!(
                    "This setup is for the {}, and the card is a {board}'s.",
                    setup.board
                ),
            ));
        }
    }
    let mut edits = Vec::new();
    let mut warnings = Vec::new();
    // Each model as it will be after its edits, for copies made from it.
    let mut after: BTreeMap<String, Doc> = BTreeMap::new();
    let mut tracks: Vec<String> = Vec::new();
    for a in &setup.aircraft {
        let path = format!("MODELS/{}", a.file);
        let doc = match src.read(&path) {
            Some(b) => {
                let d = parse(&path, &b)?;
                let have = em::model_name(&d)?.unwrap_or_default();
                let fits = match &a.name {
                    None => true,
                    Some(n) => &have == n || a.was_named.contains(&have),
                };
                if !fits {
                    return Err(Refusal::new(
                        RefusalCode::DeviceChanged,
                        format!(
                            "{} is {have:?}, not {:?}; wrong card?",
                            a.file,
                            a.name.as_deref().unwrap_or_default()
                        ),
                    ));
                }
                d
            }
            None => {
                let from = a.copy_from.as_deref().ok_or_else(|| {
                    bad(format!(
                        "The card has no {path}; name a model to copy it from (copy_from)."
                    ))
                })?;
                let base = match after.get(from) {
                    Some(d) => d.clone(),
                    None => {
                        let p = format!("MODELS/{from}");
                        let b = src.read(&p).ok_or_else(|| {
                            bad(format!("The card has no {p} to copy {} from.", a.file))
                        })?;
                        parse(&p, &b)?
                    }
                };
                let name = match &a.name {
                    Some(n) => n.clone(),
                    None => em::model_name(&base)?.unwrap_or_default(),
                };
                edits.push(Edit::ModelCopy {
                    from: from.to_string(),
                    to: a.file.clone(),
                    name: name.clone(),
                });
                let mut d = Doc::parse(&a.file, &base.render())?;
                em::apply(&mut d, &[ModelOp::Rename { name }])?;
                d
            }
        };
        let ops = model_ops(setup, &ctx, a, &doc)?;
        let mut probe = doc.clone();
        em::apply(&mut probe, &ops)
            .map_err(|r| Refusal::new(r.code, format!("{}: {}", a.file, r.reason)))?;
        for s in em::view(&a.file, &probe)?.special_functions {
            if s.func == "PLAY_TRACK" {
                if let Some(t) = s.def.split(',').next().filter(|t| !t.is_empty()) {
                    tracks.push(t.to_string());
                }
            }
        }
        after.insert(a.file.clone(), probe);
        edits.push(Edit::Model {
            file: a.file.clone(),
            name: None,
            ops,
        });
        if let Some(text) = &a.checklist {
            edits.push(Edit::Checklist {
                model: a.file.clone(),
                text: text.clone(),
            });
        }
    }
    // radio.yml.
    let r = &setup.radio;
    let mut ops = Vec::new();
    for f in &r.scalars {
        ops.push(RadioOp::SetScalar {
            key: f.key.clone(),
            value: f.value.clone(),
        });
    }
    if let Some(m) = &r.select_model {
        ops.push(RadioOp::SelectModel { file: m.clone() });
    }
    for f in &r.remove_functions {
        ops.push(RadioOp::RemoveFunctions { func: f.clone() });
    }
    let radio_doc = radio_bytes
        .as_deref()
        .map(|b| Doc::parse("radio.yml", b))
        .transpose()?;
    // Every slot's type when the setup says so; else the fitted parts only. A blanked slot
    // is not among `controls`, so it is written from the fitted list.
    let typed: Vec<(String, String, bool)> = {
        let board_slots = crate::gear::edgetx::board::spec(Some(&setup.board))
            .map(|s| s.controls())
            .unwrap_or_default();
        board_slots
            .iter()
            .filter(|b| b.kind != crate::gear::edgetx::board::BoardControlKind::Custom)
            .filter_map(|b| {
                let fit = setup.fitted.iter().find(|f| f.control == b.name);
                if fit.is_none() && !r.control_types {
                    return None;
                }
                let t = match fit {
                    Some(f) => controls::slot_type(b.kind, &f.radio_type).ok()?,
                    None => controls::factory_type(b),
                };
                let switch = b.kind == crate::gear::edgetx::board::BoardControlKind::Switch;
                Some((b.name.clone(), t, switch))
            })
            .collect()
    };
    {
        for (name, value, switch) in typed {
            let block = if switch { "switchConfig" } else { "potsConfig" };
            let present = radio_doc
                .as_ref()
                .and_then(|d| d.find(&[block, &name]).ok().flatten())
                .is_some();
            if !present {
                warnings.push(format!(
                    "radio.yml has no {name} in {block}, so its type stays as the radio has it."
                ));
                continue;
            }
            ops.push(RadioOp::SetControlType {
                control: name,
                value,
            });
        }
    }
    if !ops.is_empty() {
        if radio_doc.is_none() {
            return Err(bad(format!("The card has no {RADIO_FILE}.")));
        }
        edits.push(Edit::Radio { ops });
    }
    if r.delete_other_models {
        let keep: BTreeSet<&str> = setup.aircraft.iter().map(|a| a.file.as_str()).collect();
        for f in src.files() {
            if let Some(m) = f.strip_prefix("MODELS/").filter(|m| is_model_file(m)) {
                if !keep.contains(m) {
                    edits.push(Edit::ModelDelete {
                        file: m.to_string(),
                    });
                }
            }
        }
    }
    // QuadCam's Lua screens: put when missing or when QuadCam's own differs.
    let shown: BTreeSet<&str> = setup
        .template
        .screens
        .iter()
        .filter_map(|s| match s {
            ScreenSpec::Script { script, .. } => Some(script.as_str()),
            _ => None,
        })
        .collect();
    for l in &setup.template.lua {
        if !shown.contains(l.name.as_str()) {
            continue;
        }
        let finder = match &l.kind {
            LuaKind::Finder { job } => match ctx.bound.get(job.as_str()) {
                Some(b) if b.control.kind.has_positions() => Some(FinderControl {
                    switch: b.control.name.clone(),
                    on_position: b.high,
                }),
                Some(b) => {
                    return Err(bad(format!(
                        "The finder screen needs a switch; {} is not one.",
                        b.control.name
                    )))
                }
                None => continue,
            },
            LuaKind::Volt { .. } => None,
        };
        if l.name.is_empty() || l.name.len() > em::MAX_SCRIPT_NAME {
            return Err(bad(format!(
                "Script {:?}: a telemetry script name is 1-{} characters.",
                l.name,
                em::MAX_SCRIPT_NAME
            )));
        }
        let text = lua::render(l, finder.as_ref()).unwrap_or_default();
        let path = format!("SCRIPTS/TELEMETRY/{}.lua", l.name);
        match src.read(&path) {
            Some(b) if b == text.as_bytes() => {}
            Some(b) if !lua::is_quadcam(&b) => warnings.push(format!(
                "{path} is a script QuadCam did not write; it stays as it is. Delete it to get QuadCam's."
            )),
            _ => {
                edits.push(Edit::CardText {
                    path: path.clone(),
                    text: Some(text),
                });
                let luac = format!("{path}c");
                if src.read(&luac).is_some() {
                    edits.push(Edit::CardText {
                        path: luac,
                        text: None,
                    });
                }
            }
        }
    }
    // Sounds the functions play: a missing one is a warning; voice packs put sounds on.
    let lang = radio_doc
        .as_ref()
        .and_then(|d| d.top_value("ttsLanguage"))
        .map(|v| unquote(v.trim()).to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "en".into());
    let files: BTreeSet<String> = src.files().into_iter().collect();
    let mut sounds = Vec::new();
    let mut seen = BTreeSet::new();
    for t in tracks {
        if !seen.insert(t.clone()) {
            continue;
        }
        let path = format!("SOUNDS/{lang}/{t}.wav");
        let present = files.contains(&path) || files.iter().any(|f| f.eq_ignore_ascii_case(&path));
        if !present {
            warnings.push(format!(
                "The card has no {path}: that callout stays silent until a voice pack puts it on."
            ));
        }
        sounds.push(SoundCheck {
            track: t,
            path,
            present,
        });
    }
    Ok(Generated {
        edits,
        jobs: job_rows(setup, &controls),
        sounds,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::super::defaults;
    use super::*;
    use crate::gear::edgetx::synth;

    struct Mem(BTreeMap<String, Vec<u8>>);

    impl Source for Mem {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            self.0.get(path).cloned()
        }
        fn files(&self) -> Vec<String> {
            self.0.keys().cloned().collect()
        }
    }

    fn token_ctx(setup: &RadioSetup, controls: &[Control]) -> String {
        let mut ctx = Ctx {
            bound: BTreeMap::new(),
            jobs: setup.jobs.iter().map(|j| j.id.as_str()).collect(),
        };
        for a in &setup.assignments {
            ctx.bound.insert(
                a.job.as_str(),
                Bound {
                    control: controls.iter().find(|c| c.name == a.control).unwrap(),
                    channel: setup.job(&a.job).unwrap().channel,
                    high: a.high.unwrap_or(2),
                },
            );
        }
        format!(
            "{} {} {} {} {} {}",
            ctx.resolve("@arm.hi,@finder.lo").unwrap(),
            ctx.resolve("@rates.lo/@rates.mid/@rates.hi").unwrap(),
            ctx.resolve("ch(@arm.ch),0").unwrap(),
            ctx.resolve("L10,@blackbox").unwrap(),
            ctx.sign("@osd"),
            ctx.resolve("@vtx.hi").unwrap_err().reason
        )
    }

    #[test]
    fn tokens_follow_the_assignment() {
        let s = defaults::setup("pocket", "model00.yml").unwrap();
        let c = check(&s).unwrap();
        assert_eq!(
            token_ctx(&s, &c),
            "SA2,SD0 SC2/SC1/SC0 ch(4),0 L10,TrimRudRight -1 @vtx.hi: P1 is not a switch, so it has no positions."
        );
        let s = defaults::setup("gx12", "model00.yml").unwrap();
        let c = check(&s).unwrap();
        assert!(token_ctx(&s, &c).starts_with("SA2,SD0 SC0/SC1/SC2 ch(4),0 L10,TrimRudRight 1 "));
    }

    #[test]
    fn check_refuses_what_does_not_fit() {
        let base = defaults::setup("pocket", "model00.yml").unwrap();
        let mut s = base.clone();
        s.assignments
            .iter_mut()
            .find(|a| a.job == "arm")
            .unwrap()
            .control = "SE".into();
        let e = check(&s).unwrap_err();
        assert!(e.reason.contains("SE cannot do Arm"), "{e}");
        let mut s = base.clone();
        s.assignments
            .iter_mut()
            .find(|a| a.job == "osd")
            .unwrap()
            .control = "SC".into();
        assert!(check(&s).unwrap_err().reason.contains("SC does two jobs"));
        let mut s = base.clone();
        s.assignments
            .iter_mut()
            .find(|a| a.job == "arm")
            .unwrap()
            .high = Some(1);
        assert!(check(&s).unwrap_err().reason.contains("which position"));
        let mut s = base.clone();
        s.assignments
            .iter_mut()
            .find(|a| a.job == "vtx")
            .unwrap()
            .control = "SW1".into();
        assert!(check(&s)
            .unwrap_err()
            .reason
            .contains("not a control of the pocket"));
        let mut s = defaults::setup("gx12", "model00.yml").unwrap();
        s.jobs
            .iter_mut()
            .find(|j| j.id == "finder")
            .unwrap()
            .fits
            .push(RadioControlKind::Custom);
        s.assignments
            .iter_mut()
            .find(|a| a.job == "finder")
            .unwrap()
            .control = "SW1".into();
        assert!(check(&s).unwrap_err().reason.contains("customizable"));
        let mut s = base.clone();
        s.board = "tx16s".into();
        assert_eq!(check(&s).unwrap_err().code, RefusalCode::UnknownBoard);
        let mut s = base;
        s.jobs[1].channel = Some(4);
        assert!(check(&s).unwrap_err().reason.contains("both drive CH5"));
    }

    #[test]
    fn job_rows_name_positions_and_values() {
        let s = defaults::setup("pocket", "model00.yml").unwrap();
        let c = check(&s).unwrap();
        let rows = job_rows(&s, &c);
        let rates = rows.iter().find(|r| r.job == "rates").unwrap();
        assert_eq!(rates.channel.as_deref(), Some("CH7"));
        let pos: Vec<_> = rates
            .states
            .iter()
            .map(|s| s.position.clone().unwrap())
            .collect();
        assert_eq!(pos, ["SC2", "SC1", "SC0"]);
        let us: Vec<_> = rates.states.iter().map(|s| s.us.unwrap()).collect();
        assert_eq!(us, [1000, 1500, 2000]);
        let arm = rows.iter().find(|r| r.job == "arm").unwrap();
        let pos: Vec<_> = arm
            .states
            .iter()
            .map(|s| s.position.clone().unwrap())
            .collect();
        assert_eq!(pos, ["SA0", "SA2"]);
        let vtx = rows.iter().find(|r| r.job == "vtx").unwrap();
        assert!(vtx.states.iter().all(|s| s.position.is_none()));
        assert!(rows
            .iter()
            .find(|r| r.job == "turtle")
            .unwrap()
            .free_controls
            .is_empty());
    }

    fn synth_card(board: &str) -> Mem {
        let dir = tempfile::tempdir().unwrap();
        let card = synth::SynthCard {
            board: board.into(),
            ..Default::default()
        };
        synth::write_card(dir.path(), &card).unwrap();
        let mut m = BTreeMap::new();
        for e in walkdir(dir.path()) {
            let rel = e
                .strip_prefix(dir.path())
                .unwrap()
                .to_string_lossy()
                .to_string();
            m.insert(rel, std::fs::read(&e).unwrap());
        }
        Mem(m)
    }

    fn walkdir(p: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(p).unwrap().flatten() {
            if e.file_type().unwrap().is_dir() {
                out.extend(walkdir(&e.path()));
            } else {
                out.push(e.path());
            }
        }
        out
    }

    #[test]
    fn the_default_setup_generates_on_a_synthetic_card_and_again_changes_nothing() {
        let src = synth_card("pocket");
        let model = src
            .files()
            .into_iter()
            .find(|f| f.starts_with("MODELS/model"))
            .unwrap();
        let file = model.trim_start_matches("MODELS/").to_string();
        let s = defaults::setup("pocket", &file).unwrap();
        let g = generate(&s, &src).unwrap();
        assert!(g.edits.iter().any(|e| matches!(e, Edit::Model { .. })));
        assert!(g.edits.iter().any(
            |e| matches!(e, Edit::CardText { path, .. } if path == "SCRIPTS/TELEMETRY/finder.lua")
        ));
        assert!(g.sounds.iter().any(|x| x.track == "ratefree"));
    }
}
