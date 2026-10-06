//! Params and results of the import session methods.

use crate::core::LogChoice;
use crate::moments::Span;
use crate::session::{Editor, PlanPatch};
use crate::trim::RemovedCuts;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::PathBuf;

/// `stage` and `load`: a card mount point or a folder. None takes the first detected card.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct SourceParams {
    pub source: Option<PathBuf>,
    /// Join recordings the DVR split into files, this run. None follows the
    /// `join_split_recordings` setting.
    #[serde(default)]
    pub join: Option<bool>,
}

/// `dates`: where the radio logs come from, and the log day to match.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct DatesParams {
    #[serde(default)]
    pub logs: LogChoice,
    pub day: Option<NaiveDate>,
}

/// `suggest`: changes to clip plans. Without `editor`, an agent made them.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct SuggestParams {
    pub patches: Vec<PlanPatch>,
    #[serde(default)]
    pub editor: Option<Editor>,
}

/// `photos`: the clips to add (all verified clips when None), and the album.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct PhotosParams {
    pub ids: Option<Vec<usize>>,
    pub album: Option<String>,
}

/// `verify`: the clips to check again (all verified clips when None).
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct VerifyParams {
    pub ids: Option<Vec<usize>>,
}

/// `eject`: a mount point or `/dev/diskN`; None ejects the session's card.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct EjectParams {
    pub target: Option<String>,
}

/// `format_plan`: the FAT32 volume name; None uses the setting.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct LabelParams {
    pub label: Option<String>,
}

/// `session_cuts`: a session clip's new cut list, and what happens to exported cuts it drops.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct SessionCutsParams {
    pub id: usize,
    pub cuts: Vec<Span>,
    #[serde(default)]
    pub removed_cuts: Option<RemovedCuts>,
}

/// One session clip.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct SessionClipParams {
    pub id: usize,
}

/// `clear`'s answer.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct Cleared {
    pub cleared: bool,
}

/// `eject`'s answer.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct Ejected {
    pub ejected: bool,
}
