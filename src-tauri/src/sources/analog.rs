//! Analog DVRs: MJPEG and PCM in AVI, numbered `PICT0001.AVI` and up, on a FAT32 card of
//! 32 GB or less. A DVR has no clock. Power loss leaves a half-written AVI (wrong RIFF size,
//! no `idx1`), which a stream copy with fresh timestamps makes playable again.

use super::{CardPolicy, EncodePlan, Inspect, Source, SourceKind};
use crate::media::{self, Format, Probe, Tools};
use crate::moments::SignalScan;
use crate::scan::{self, FoundClip};
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

/// Cards above this size get a warning: most analog DVRs take 32 GB at most.
pub const WARN_ABOVE_BYTES: u64 = crate::disk::FAT32_CARD_BYTES;

pub struct Analog;

impl Source for Analog {
    fn kind(&self) -> SourceKind {
        SourceKind::Analog
    }

    fn detect(&self, root: &Path) -> bool {
        scan::has_clips(root)
    }

    fn list(&self, root: &Path) -> Vec<FoundClip> {
        scan::find_clips(root)
    }

    fn pick(&self, files: &[PathBuf]) -> Vec<FoundClip> {
        scan::clips_in(files)
    }

    fn sidecars(&self, _clip: &Path) -> Vec<PathBuf> {
        Vec::new()
    }

    fn inspect(&self, staged: &Path) -> Result<Inspect> {
        Ok(Inspect {
            complete: scan::check_avi(staged)?.complete(),
        })
    }

    fn repair_path(&self, staged: &Path) -> PathBuf {
        staged.with_file_name(format!(
            "{}.recovered.avi",
            staged.file_stem().unwrap_or_default().to_string_lossy()
        ))
    }

    fn repair(&self, tools: &Tools, staged: &Path, dst: &Path) -> Result<()> {
        media::recover(tools, staged, dst)
    }

    fn intrinsic_time(&self, _staged: &Path) -> Option<DateTime<Utc>> {
        None
    }

    fn encode_plan(&self, _probe: &Probe, want: Format) -> EncodePlan {
        match want {
            // The webview and Photos want H.264.
            Format::Mp4 => EncodePlan::Transcode,
            // MOV keeps the MJPEG frames, lossless.
            Format::Mov => EncodePlan::Remux,
        }
    }

    fn signal(
        &self,
        tools: &Tools,
        src: &Path,
        fps: Option<f64>,
        duration: f64,
    ) -> Option<SignalScan> {
        crate::moments::scan_signal(tools, src, fps, duration).ok()
    }

    fn original_ext(&self, _staged: &Path) -> String {
        "avi".into()
    }

    fn card_policy(&self) -> CardPolicy {
        CardPolicy {
            format_offered: true,
            filesystem: "FAT32",
            warn_above_bytes: WARN_ABOVE_BYTES,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe() -> Probe {
        serde_json::from_value(serde_json::json!({
            "duration": 1.0, "video_packets": 30, "video_streams": 1, "audio_streams": 0,
            "width": 720, "height": 480, "fps": 30.0, "errors": "", "tags": {}
        }))
        .unwrap()
    }

    #[test]
    fn analog_answers_as_before() {
        let a = Analog;
        assert_eq!(a.kind(), SourceKind::Analog);
        assert_eq!(a.encode_plan(&probe(), Format::Mp4), EncodePlan::Transcode);
        assert_eq!(a.encode_plan(&probe(), Format::Mov), EncodePlan::Remux);
        assert_eq!(a.original_ext(Path::new("/x/PICT0001.AVI")), "avi");
        assert_eq!(a.intrinsic_time(Path::new("/x/PICT0001.AVI")), None);
        assert!(a.sidecars(Path::new("/x/PICT0001.AVI")).is_empty());
        assert_eq!(
            a.repair_path(Path::new("/s/PICT0004.AVI")),
            PathBuf::from("/s/PICT0004.recovered.avi")
        );
        assert_eq!(
            a.card_policy(),
            CardPolicy {
                format_offered: true,
                filesystem: "FAT32",
                warn_above_bytes: 34_000_000_000,
            }
        );
    }

    #[test]
    fn detects_lists_and_inspects() {
        let d = tempfile::tempdir().unwrap();
        assert!(!Analog.detect(d.path()));
        assert!(super::super::detect(d.path()).is_none());
        std::fs::create_dir(d.path().join("DCIM")).unwrap();
        let clip = d.path().join("DCIM/PICT0001.AVI");
        // A RIFF header whose size is wrong, and no idx1: half-written.
        let mut bytes = b"RIFF".to_vec();
        bytes.extend(1000u32.to_le_bytes());
        bytes.extend(b"AVI LIST");
        bytes.extend(4u32.to_le_bytes());
        bytes.extend(b"hdrl");
        std::fs::write(&clip, &bytes).unwrap();
        assert!(Analog.detect(d.path()));
        assert_eq!(super::super::for_root(d.path()).kind(), SourceKind::Analog);
        assert_eq!(Analog.list(d.path()).len(), 1);
        assert_eq!(Analog.pick(std::slice::from_ref(&clip)).len(), 1);
        assert!(!Analog.inspect(&clip).unwrap().complete);
        // The same file with a right RIFF size and an idx1 chunk: whole.
        let mut whole = b"RIFF".to_vec();
        let body: Vec<u8> = [
            b"AVI ".to_vec(),
            b"idx1".to_vec(),
            0u32.to_le_bytes().to_vec(),
        ]
        .concat();
        whole.extend((body.len() as u32).to_le_bytes());
        whole.extend(body);
        std::fs::write(&clip, &whole).unwrap();
        assert!(Analog.inspect(&clip).unwrap().complete);
    }
}
