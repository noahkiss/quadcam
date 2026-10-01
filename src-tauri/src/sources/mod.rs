//! Footage sources: the video systems QuadCam reads clips from. Each `Source` knows how its
//! card or folder is laid out, how to tell a whole clip from a half-written one, how to
//! repair it, how its video goes into the output container, whether its frames show dead
//! air, and what its card may be formatted to. The pipeline asks the clip's source and never
//! assumes one. Today there is one: `analog` (DVR MJPEG in AVI).

pub mod analog;

use crate::media::{Format, Probe, Tools};
use crate::moments::SignalScan;
use crate::scan::FoundClip;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Which video system a clip came from.
#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash, specta::Type,
)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    /// An analog DVR: MJPEG in AVI.
    #[default]
    Analog,
}

/// What a source's own structure check says about a staged clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inspect {
    /// The file is whole; false means it was cut off (power loss, card pulled).
    pub complete: bool,
}

/// How a clip's video goes into the output container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodePlan {
    /// Re-encode the video as H.264 (analog MJPEG into MP4).
    Transcode,
    /// Copy the streams as they are (analog MJPEG into MOV).
    Remux,
}

/// What a source's card may be formatted to. Formatting runs every guard in
/// `disk::format_card` either way; this only says whether it is offered at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardPolicy {
    /// The app offers "Format card" for this source.
    pub format_offered: bool,
    /// The file system the card should have, and gets when formatted.
    pub filesystem: &'static str,
    /// Cards larger than this get a warning when they are inserted.
    pub warn_above_bytes: u64,
}

pub trait Source: Send + Sync {
    fn kind(&self) -> SourceKind;
    /// True when the card or folder at `root` holds clips of this source.
    fn detect(&self, root: &Path) -> bool;
    /// Every clip under `root`, in recording order.
    fn list(&self, root: &Path) -> Vec<FoundClip>;
    /// The clips among files a person dropped.
    fn pick(&self, files: &[PathBuf]) -> Vec<FoundClip>;
    /// Files that belong to a clip (telemetry, OSD). None for analog.
    fn sidecars(&self, clip: &Path) -> Vec<PathBuf>;
    /// Whether a staged clip is whole.
    fn inspect(&self, staged: &Path) -> Result<Inspect>;
    /// Where the repaired copy of a half-written clip goes.
    fn repair_path(&self, staged: &Path) -> PathBuf;
    /// Writes a playable copy of a half-written clip to `dst`.
    fn repair(&self, tools: &Tools, staged: &Path, dst: &Path) -> Result<()>;
    /// The time the clip itself records, if the source has a clock. None for analog: a DVR
    /// has none, so dates come from radio logs or the import day.
    fn intrinsic_time(&self, staged: &Path) -> Option<DateTime<Utc>>;
    /// How the clip's video goes into a `want` file.
    fn encode_plan(&self, probe: &Probe, want: Format) -> EncodePlan;
    /// Dead air in the clip's frames, when the source has such a thing (analog: blue
    /// screen, static, test pattern, black). None when it cannot be sampled.
    fn signal(
        &self,
        tools: &Tools,
        src: &Path,
        fps: Option<f64>,
        duration: f64,
    ) -> Option<SignalScan>;
    /// The file extension a kept original gets.
    fn original_ext(&self, staged: &Path) -> String;
    fn card_policy(&self) -> CardPolicy;
}

/// Every source, in detection order.
pub fn all() -> [&'static dyn Source; 1] {
    [&analog::Analog]
}

/// The source of a kind.
pub fn get(kind: SourceKind) -> &'static dyn Source {
    match kind {
        SourceKind::Analog => &analog::Analog,
    }
}

/// The source whose clips are at `root`, if any.
pub fn detect(root: &Path) -> Option<&'static dyn Source> {
    all().into_iter().find(|s| s.detect(root))
}

/// The source to read `root` with: the one detected there, else analog.
pub fn for_root(root: &Path) -> &'static dyn Source {
    detect(root).unwrap_or(&analog::Analog)
}
