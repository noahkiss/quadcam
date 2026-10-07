//! Footage sources: the video systems QuadCam reads clips from. Each `Source` knows how its
//! card or folder is laid out, how to tell a whole clip from a half-written one, how to
//! repair it, how its video goes into the output container, whether its frames show dead
//! air, and what may happen to its card after an import (format, delete the imported clips). The pipeline asks the clip's source and never
//! assumes one. Two today: `dji` (DJI O4 MP4s) and `analog` (DVR MJPEG in AVI).

pub mod analog;
pub mod dji;

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
    /// A DJI air unit or goggles: H.264 or H.265 in MP4, named by the unit's clock.
    Dji,
}

impl SourceKind {
    /// The name people know the video system by: `analog`, `DJI`. A profile's
    /// `video_system` matches it, ignoring case.
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::Analog => "analog",
            SourceKind::Dji => "DJI",
        }
    }

    /// How a file's description names where it came from: `DVR PICT0001.AVI`,
    /// `DJI DJI_..._D.MP4`.
    pub fn file_label(self) -> &'static str {
        match self {
            SourceKind::Analog => "DVR",
            SourceKind::Dji => "DJI",
        }
    }
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
    /// Copy the streams as they are into a new container (analog MJPEG into MOV, DJI MP4
    /// into MOV).
    Remux,
    /// Copy the file byte for byte, no ffmpeg (DJI MP4 into MP4). Every stream stays,
    /// the ones ffmpeg's MP4 muxer refuses included.
    Copy,
}

/// What may happen to a source's card: whether it may be formatted after an import or by
/// card prep, and whether its imported clip files may be deleted. Formatting runs every guard
/// in `disk::format_card` and deleting every guard in `Core::delete_imported_clips` either
/// way; this only says whether each is offered at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardPolicy {
    /// The app offers "Format card" for this source after an import.
    pub format_offered: bool,
    /// Card prep may erase this source's card once every clip on it is in the library.
    pub prep_offered: bool,
    /// "Delete clips after import" may delete this source's clip files (only the files
    /// `Source::pick` takes as clips; sidecars and every other file stay).
    pub delete_clips_offered: bool,
    /// The file system the card should have, and gets when formatted (`disk::erase_personality`).
    pub filesystem: &'static str,
    /// Cards larger than this get a warning when they are inserted, and in the format
    /// confirm. Never a refusal. `u64::MAX`: never.
    pub warn_above_bytes: u64,
    /// Said after "Card is X, not <filesystem>." when the file system differs.
    pub filesystem_advice: &'static str,
    /// The warning for a card over `warn_above_bytes`.
    pub size_warning: &'static str,
}

pub trait Source: Send + Sync {
    fn kind(&self) -> SourceKind;
    /// True when the card or folder at `root` holds clips of this source.
    fn detect(&self, root: &Path) -> bool;
    /// Every clip under `root`, in recording order.
    fn list(&self, root: &Path) -> Vec<FoundClip>;
    /// The clips among files a person dropped.
    fn pick(&self, files: &[PathBuf]) -> Vec<FoundClip>;
    /// Files next to a clip that belong to it (telemetry, OSD). None for analog.
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

/// Every source, in detection order: strict checks (a name pattern) before loose ones
/// (analog takes any `.avi`).
pub fn all() -> [&'static dyn Source; 2] {
    [&dji::Dji, &analog::Analog]
}

/// The source of a kind.
pub fn get(kind: SourceKind) -> &'static dyn Source {
    match kind {
        SourceKind::Analog => &analog::Analog,
        SourceKind::Dji => &dji::Dji,
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
