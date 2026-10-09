//! Events the app sends to the web view. Each is its own type, and its event name is the
//! type name in kebab case (`ImportProgress` is `import-progress`), so the front end gets
//! a typed listener for each.

use crate::core::FormatPlan;
use crate::pipeline::ClipResult;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event as _;

/// What the core reports while it works. `Hooks::event` takes one; the GUI emits it.
#[derive(Debug, Clone)]
pub enum Event {
    Progress(Progress),
    ImportProgress(ImportProgress),
    ImportResult(Box<ImportResult>),
    LibraryTask(LibraryTask),
    RadioInput(RadioInput),
    SimCalibration(SimCalibrationEvent),
}

impl Event {
    /// The event name and its JSON payload.
    pub fn name_and_payload(&self) -> (&'static str, serde_json::Value) {
        fn v<T: Serialize>(x: &T) -> serde_json::Value {
            serde_json::to_value(x).unwrap_or(serde_json::Value::Null)
        }
        match self {
            Event::Progress(x) => (Progress::NAME, v(x)),
            Event::ImportProgress(x) => (ImportProgress::NAME, v(x)),
            Event::ImportResult(x) => (ImportResult::NAME, v(&**x)),
            Event::LibraryTask(x) => (LibraryTask::NAME, v(x)),
            Event::RadioInput(x) => (RadioInput::NAME, v(x)),
            Event::SimCalibration(x) => (SimCalibrationEvent::NAME, v(x)),
        }
    }
}

/// The step `Progress` reports on.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// Copying clips off the card.
    Stage,
    /// Probing, recovering and thumbnailing them.
    Analyse,
}

/// Loading a card: clip `index` of `total`, `done` of `size` bytes copied.
#[derive(Debug, Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct Progress {
    pub phase: Phase,
    pub index: usize,
    pub total: usize,
    pub done: u64,
    pub size: u64,
}

/// Converting one clip: `seconds` of `duration` done.
#[derive(Debug, Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct ImportProgress {
    pub id: usize,
    pub seconds: f64,
    pub duration: f64,
}

/// One clip finished converting (verified, failed or skipped).
#[derive(Debug, Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct ImportResult(pub ClipResult);

/// The radio in USB Joystick mode: connected or not, and its latest report
/// (`gear_radio_watch` starts the stream).
#[derive(Debug, Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct RadioInput(pub crate::gear::radio_hid::RadioEvent);

/// The sim's calibration session: its step, coverage, draft and live output
/// (`gear_sim_calibrate` starts it).
#[derive(Debug, Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct SimCalibrationEvent(pub crate::core::CalibrateView);

/// The long library jobs.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Task {
    Rebuild,
    Cuts,
    Thumbnails,
    Moments,
}

/// A library job's progress: `done` of `total`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct LibraryTask {
    pub task: Task,
    pub done: usize,
    pub total: usize,
}

/// The session changed. Read it again with `session`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct SessionChanged;

/// The library index changed. Read it again with `library`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct LibraryChanged;

/// The settings file changed. Read it again with `settings`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct SettingsChanged;

/// Something mounted or unmounted under /Volumes.
#[derive(Debug, Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct VolumesChanged;

/// `gear.json` changed (devices, links). Read Gear again with `gear_status`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct GearChanged;

/// What is plugged in changed: `events` says what happened (connected, identified,
/// unmounted but still in, removed), `connected` is what is plugged in now, and
/// `unmounted` what is unmounted but still in.
#[derive(Debug, Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct DeviceChanged {
    pub events: Vec<crate::gear::events::DeviceEvent>,
    pub connected: Vec<crate::gear::model::Connected>,
    pub unmounted: Vec<crate::gear::model::Connected>,
}

/// An agent asked to erase the card. Answer with `answer_format_request`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct AgentFormatRequest {
    pub id: u64,
    pub plan: FormatPlan,
}

/// The agent's format request with this id is closed (answered or timed out).
#[derive(Debug, Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct AgentFormatClosed(pub u64);

/// An agent asked to apply a staged change. Answer with `answer_apply_request`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct AgentApplyRequest {
    pub id: u64,
    pub change: crate::gear::model::StagedChange,
    pub plan: crate::gear::model::ApplyPlan,
}

/// The agent's apply request with this id is closed (answered or timed out).
#[derive(Debug, Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct AgentApplyClosed(pub u64);

/// A menu item was chosen: its id.
#[derive(Debug, Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct Menu(pub String);

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The payloads keep the shapes the UI reads.
    #[test]
    fn event_payloads() {
        let (n, p) = Event::Progress(Progress {
            phase: Phase::Stage,
            index: 1,
            total: 3,
            done: 10,
            size: 20,
        })
        .name_and_payload();
        assert_eq!(n, "progress");
        assert_eq!(
            p,
            json!({"phase": "stage", "index": 1, "total": 3, "done": 10, "size": 20})
        );
        let (n, p) = Event::LibraryTask(LibraryTask {
            task: Task::Thumbnails,
            done: 0,
            total: 2,
        })
        .name_and_payload();
        assert_eq!(n, "library-task");
        assert_eq!(p, json!({"task": "thumbnails", "done": 0, "total": 2}));
        let (n, p) = Event::ImportProgress(ImportProgress {
            id: 2,
            seconds: 1.5,
            duration: 3.0,
        })
        .name_and_payload();
        assert_eq!(n, "import-progress");
        assert_eq!(p, json!({"id": 2, "seconds": 1.5, "duration": 3.0}));
        assert_eq!(ImportResult::NAME, "import-result");
        assert_eq!(SessionChanged::NAME, "session-changed");
        assert_eq!(AgentFormatRequest::NAME, "agent-format-request");
        assert_eq!(AgentFormatClosed::NAME, "agent-format-closed");
        assert_eq!(AgentApplyRequest::NAME, "agent-apply-request");
        assert_eq!(AgentApplyClosed::NAME, "agent-apply-closed");
        assert_eq!(VolumesChanged::NAME, "volumes-changed");
        assert_eq!(Menu::NAME, "menu");
        assert_eq!(GearChanged::NAME, "gear-changed");
        assert_eq!(DeviceChanged::NAME, "device-changed");
    }
}
