//! A radio setup as data (`docs/radio-design.md` 4.2 and 5): what the pilot wants (jobs),
//! which control does each job on this radio and in which direction (assignments), the
//! model parts that do not depend on a control (the template), and the per-aircraft
//! differences (overrides). `generate` turns a setup and the card it is for into the same
//! card edits the model and radio editors stage, so one change carries the whole setup with
//! the usual diff, plan, digest and apply.
//!
//! - `controls`: the controls a job may take on a board: the board table's (`edgetx::board`)
//!   switches, pots and sliders, and the trims.
//! - `defaults`: the jobs of design 5.1, the default assignment per board and a starting
//!   template.
//! - `generate`: the generator, and the check of a setup against its board.
//! - `lua`: QuadCam's own telemetry screens (MIT, its own code), rendered per setup.
//! - `postflash`: the steps after a firmware flash: the storage pack, `elrs.lua` and the
//!   volume label.
//!
//! Template text names a job's control with `@job` tokens, resolved per assignment:
//!
//! | Token | Gives |
//! |---|---|
//! | `@arm` | the control (`SA`, `P1`, `TrimRudRight`) |
//! | `@arm.hi` | its position that sends 2000 µs (`SA2`) |
//! | `@arm.lo` | its position that sends 1000 µs (`SA0`) |
//! | `@rates.mid` | the middle position of a 3-position switch (`SC1`) |
//! | `@arm.ch` | the job's channel, 0 for CH1 (`ch(@arm.ch)` is `ch(4)`) |
//!
//! A mix line whose source is `@job` takes the assignment's direction: its weight is negated
//! when position 0 sends 2000 µs. Sensors stay `{RxBt}`: the model ops look up their slots.
//! An item that belongs to a job (`job`) is left out while no control does that job.

pub mod controls;
pub mod defaults;
pub mod generate;
pub mod lua;
pub mod postflash;

use super::model::{Field, LsDef, MixLine, SfDef};
use serde::{Deserialize, Serialize};
use specta::Type;

/// What kind of control a radio has (`docs/radio-design.md` 5.2), and what a job accepts.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Type)]
#[serde(rename_all = "snake_case")]
pub enum RadioControlKind {
    /// A 2-position switch or a button that latches in (`2POS`).
    TwoPos,
    /// A 3-position switch.
    ThreePos,
    /// A spring-back switch or button: on while held (`TOGGLE`).
    Momentary,
    /// A customizable switch (GX12 SW1-SW6): set per model.
    Custom,
    /// A pot or wheel, with or without a centre detent.
    Pot,
    Slider,
    /// One direction of a trim hat (`TrimRudRight`).
    Trim,
}

impl RadioControlKind {
    /// Switches have positions (`SA0`..`SA2`); pots, sliders and trims do not.
    pub fn has_positions(self) -> bool {
        matches!(
            self,
            RadioControlKind::TwoPos
                | RadioControlKind::ThreePos
                | RadioControlKind::Momentary
                | RadioControlKind::Custom
        )
    }
}

/// One control of a board.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct Control {
    /// The EdgeTX name (`SA`, `P1`, `SL1`, `SW1`, `TrimRudRight`).
    pub name: String,
    pub kind: RadioControlKind,
    /// The type `radio.yml` gives it (`2POS`, `TOGGLE`, `without_detent`, `slider`); None
    /// for a custom switch or a trim.
    #[serde(default)]
    pub radio_type: Option<String>,
}

/// What the pilot wants one control to do.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct Job {
    /// A short id the template names (`arm`, `rates`).
    pub id: String,
    pub label: String,
    /// The channel it drives, 0 for CH1. None for a job that stays on the radio.
    #[serde(default)]
    pub channel: Option<u32>,
    /// The control kinds that may do it.
    pub fits: Vec<RadioControlKind>,
    /// Each state in plain words, from 1000 µs to 2000 µs.
    #[serde(default)]
    pub states: Vec<String>,
}

/// Which control does a job on one radio.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct Assignment {
    pub job: String,
    /// The control's EdgeTX name.
    pub control: String,
    /// For a switch: its position (0 or 2) that sends 2000 µs, so its other end sends
    /// 1000 µs. Unused for pots, sliders, wheels and trims.
    #[serde(default)]
    pub high: Option<u8>,
}

/// A mix line of the template; `job` leaves it out while the job has no control.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct MixSpec {
    /// 0 for CH1.
    pub channel: u32,
    #[serde(flatten)]
    pub line: MixLine,
    #[serde(default)]
    pub job: Option<String>,
}

/// A logical switch of the template (0 is L1); `ls` None leaves it empty.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct LsSpec {
    pub index: u32,
    #[serde(default)]
    pub ls: Option<LsDef>,
    #[serde(default)]
    pub job: Option<String>,
}

/// A special function of the template.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct SfSpec {
    #[serde(flatten)]
    pub sf: SfDef,
    #[serde(default)]
    pub job: Option<String>,
}

/// A timer of the template (0 is Timer 1). Its stored value is never set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct TimerSpec {
    pub index: u32,
    /// Unquoted values (`swtch` `L1`, `name` `TOT`).
    pub fields: Vec<Field>,
}

/// A telemetry screen of the template (0 is screen 1).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ScreenSpec {
    /// A value screen: up to 4 lines of up to 3 sources (`{RxBt}`, `Tmr2`).
    Values { index: u32, lines: Vec<Vec<String>> },
    /// A Lua script screen (`SCRIPTS/TELEMETRY/<script>.lua`).
    Script { index: u32, script: String },
}

/// What QuadCam's Lua screen of one name shows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LuaKind {
    /// The quad's battery, link quality, TX power and a timer (0 is Timer 1).
    Volt { timer: u32 },
    /// The finder: signal strength in large digits, a beep that speeds up as the signal
    /// rises, only while the `job`'s control is on.
    Finder { job: String },
}

/// One of QuadCam's Lua telemetry screens to put on the card.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct LuaScreen {
    /// The script name (`volt`: `SCRIPTS/TELEMETRY/volt.lua`), 6 characters at most.
    pub name: String,
    #[serde(flatten)]
    pub kind: LuaKind,
}

/// Which special functions on the card the setup owns: they are removed unless the setup
/// still wants them, and the setup's own come after every other function. A function the
/// setup lists is always its own.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct Owns {
    /// Every function of these kinds (`SET_SCREEN`, `LOGS`).
    #[serde(default)]
    pub funcs: Vec<String>,
    /// `PLAY_TRACK` of these tracks, including ones an older setup played.
    #[serde(default)]
    pub tracks: Vec<String>,
    /// This function on this switch (`L2` `HAPTIC`).
    #[serde(default)]
    pub switch_funcs: Vec<SwitchFunc>,
    /// This function whose definition starts with this text (`RESET` `Tmr3,`).
    #[serde(default)]
    pub def_prefixes: Vec<FuncPrefix>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct SwitchFunc {
    pub swtch: String,
    pub func: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct FuncPrefix {
    pub func: String,
    pub def: String,
}

/// The model parts every aircraft shares.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct ModelTemplate {
    #[serde(default)]
    pub timers: Vec<TimerSpec>,
    /// The power-on checklist screen on or off.
    #[serde(default)]
    pub checklist_on: Option<bool>,
    /// Other top-level values the model file has (`disableTelemetryWarning`).
    #[serde(default)]
    pub flags: Vec<Field>,
    /// The exact mix lines of each channel the template names; other channels stay.
    #[serde(default)]
    pub mixes: Vec<MixSpec>,
    /// The setup owns every logical switch: one it does not list is emptied.
    #[serde(default)]
    pub logical_switches: Vec<LsSpec>,
    /// The setup's own special functions, in order.
    #[serde(default)]
    pub functions: Vec<SfSpec>,
    /// Functions added when the model lacks them and otherwise left where they are.
    #[serde(default)]
    pub keep_functions: Vec<SfSpec>,
    #[serde(default)]
    pub owns: Owns,
    /// Jobs whose control must be at its 1000 µs end at power-on (the arm switch).
    #[serde(default)]
    pub switch_warnings: Vec<String>,
    #[serde(default)]
    pub screens: Vec<ScreenSpec>,
    /// QuadCam's Lua screens the template's script screens show.
    #[serde(default)]
    pub lua: Vec<LuaScreen>,
}

/// One model on the radio, for one aircraft, with what differs from the template.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct Aircraft {
    /// The model file (`model00.yml`).
    pub file: String,
    /// The model name; None keeps the name the model has.
    #[serde(default)]
    pub name: Option<String>,
    /// Names the model may have before the setup renames it (a factory name). Any other
    /// name refuses: "wrong card?".
    #[serde(default)]
    pub was_named: Vec<String>,
    /// The ELRS model id of the internal module.
    #[serde(default)]
    pub model_id: Option<u32>,
    /// When the file is missing: copy it from this model file, after that model's edits.
    #[serde(default)]
    pub copy_from: Option<String>,
    /// The power-on checklist text (`MODELS/<name>.txt`).
    #[serde(default)]
    pub checklist: Option<String>,
    /// Template timers this model does not have (0 is Timer 1); they are removed.
    #[serde(default)]
    pub drop_timers: Vec<u32>,
    /// Logical switches that replace the template's at the same index, or add to it.
    #[serde(default)]
    pub logical_switches: Vec<LsSpec>,
    /// The template's own functions on these switches are left out (`L20`).
    #[serde(default)]
    pub drop_functions: Vec<String>,
    /// Functions after the template's own.
    #[serde(default)]
    pub functions: Vec<SfSpec>,
}

/// `radio.yml` parts of the setup.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct RadioPart {
    /// Top-level values (`hapticMode`), checked as the Settings editor checks them.
    #[serde(default)]
    pub scalars: Vec<Field>,
    /// The model the radio selects.
    #[serde(default)]
    pub select_model: Option<String>,
    /// Radio special functions removed by function (`VOLUME`).
    #[serde(default)]
    pub remove_functions: Vec<String>,
    /// Write each switch's and pot's type from the board's controls (`switchConfig`,
    /// `potsConfig`), fixing a factory file that has them wrong.
    #[serde(default)]
    pub control_types: bool,
    /// Delete every model file that is not one of the setup's aircraft.
    #[serde(default)]
    pub delete_other_models: bool,
}

/// What is fitted in one switch or pot slot when it differs from the board's factory part:
/// a swapped switch module, or a blanking plug (`none`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct FittedSlot {
    /// The slot's EdgeTX name (`SA`, `P2`).
    pub control: String,
    /// The type `radio.yml` takes: `2POS`, `3POS`, `TOGGLE` or `NONE` for a switch;
    /// `with_detent`, `without_detent` or `none` for a pot; `slider` or `none` for a slider.
    pub radio_type: String,
}

/// A radio setup: jobs, the controls that do them, the template and the aircraft.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct RadioSetup {
    /// The board whose controls the assignments name (`pocket`, `gx12`).
    pub board: String,
    /// Slots whose fitted part differs from the board's factory part. The generator writes
    /// each one's type to `radio.yml`.
    #[serde(default)]
    pub fitted: Vec<FittedSlot>,
    pub jobs: Vec<Job>,
    pub assignments: Vec<Assignment>,
    #[serde(default)]
    pub template: ModelTemplate,
    pub aircraft: Vec<Aircraft>,
    #[serde(default)]
    pub radio: RadioPart,
}

impl RadioSetup {
    pub fn job(&self, id: &str) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }

    pub fn assignment(&self, job: &str) -> Option<&Assignment> {
        self.assignments.iter().find(|a| a.job == job)
    }
}
