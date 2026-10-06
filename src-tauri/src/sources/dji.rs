//! DJI digital video: an O4 air unit over USB, or a goggles card. Clips are MP4s under
//! `DCIM/DJI_*/`, named `DJI_<YYYYMMDDHHMMSS>_<NNNN>_<x>.MP4` by the unit's clock in local
//! time (older goggles write `DJIG####.MP4`). Next to the video track sit DJI's own data
//! streams (`djmd`, `dbgi`) and a cover picture; `moov` is the last box. `MISC/` holds DJI's
//! housekeeping and is ignored. QuadCam only reads these volumes: it stages copies and never
//! writes to or formats them.

use super::{CardPolicy, EncodePlan, Inspect, Source, SourceKind};
use crate::media::{Format, Probe, Tools};
use crate::moments::SignalScan;
use crate::scan::FoundClip;
use anyhow::{bail, Result};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use std::path::{Path, PathBuf};

pub struct Dji;

/// The clock time and the counter in a DJI file name, wherever `DJI_<14 digits>_<4 digits>`
/// appears in it (a staged copy may carry a folder prefix). Case is ignored.
pub fn parse_name(name: &str) -> Option<(NaiveDateTime, u32)> {
    let upper = name.to_ascii_uppercase();
    let b = upper.as_bytes();
    let digits = |r: std::ops::Range<usize>| {
        b.get(r.clone())
            .is_some_and(|s| s.iter().all(u8::is_ascii_digit))
    };
    upper.match_indices("DJI_").find_map(|(i, _)| {
        let ts = i + 4;
        let n = ts + 15;
        if !(digits(ts..ts + 14) && b.get(ts + 14) == Some(&b'_') && digits(n..n + 4)) {
            return None;
        }
        let time = NaiveDateTime::parse_from_str(&upper[ts..ts + 14], "%Y%m%d%H%M%S").ok()?;
        Some((time, upper[n..n + 4].parse().ok()?))
    })
}

/// `DJIG0001.MP4`: older goggles, numbered, no clock in the name.
fn goggles_number(name: &str) -> Option<u32> {
    let upper = name.to_ascii_uppercase();
    let stem = upper.strip_suffix(".MP4")?.strip_prefix("DJIG")?;
    (stem.len() == 4 && stem.bytes().all(|c| c.is_ascii_digit()))
        .then(|| stem.parse().ok())
        .flatten()
}

/// A DJI clip name: `DJI_<14 digits>_<4 digits>_*.MP4` or `DJIG####.MP4`.
pub fn is_clip_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.ends_with(".MP4")
        && ((upper.starts_with("DJI_") && parse_name(name).is_some())
            || goggles_number(name).is_some())
}

/// Recording order: the clock time in the name, then the counter, then the name.
fn order_key(c: &FoundClip) -> (Option<NaiveDateTime>, u32, String) {
    match parse_name(&c.name) {
        Some((t, n)) => (Some(t), n, c.name.to_lowercase()),
        None => (
            None,
            goggles_number(&c.name).unwrap_or(u32::MAX),
            c.name.to_lowercase(),
        ),
    }
}

/// The folders that may hold clips: `DCIM/DJI_*/` of a volume, `DJI_*/` when `root` is
/// the `DCIM` folder, and `root` itself (a `DJI_*` folder, or a folder of copied clips).
fn clip_dirs(root: &Path) -> Vec<PathBuf> {
    let dji_dirs = |dir: &Path| -> Vec<PathBuf> {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut v: Vec<PathBuf> = rd
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .to_ascii_uppercase()
                    .starts_with("DJI_")
            })
            .map(|e| e.path())
            .collect();
        v.sort();
        v
    };
    let dcim = std::fs::read_dir(root).ok().and_then(|rd| {
        rd.flatten()
            .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case("DCIM"))
            .map(|e| e.path())
    });
    let mut out = vec![root.to_path_buf()];
    if let Some(d) = dcim {
        out.extend(dji_dirs(&d));
    }
    out.extend(dji_dirs(root));
    out
}

fn found(path: &Path, root: Option<&Path>) -> Option<FoundClip> {
    let name = path.file_name()?.to_string_lossy().to_string();
    if name.starts_with('.') || !is_clip_name(&name) || !path.is_file() {
        return None;
    }
    let rel = root
        .and_then(|r| path.strip_prefix(r).ok())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| name.clone());
    Some(FoundClip {
        path: path.to_path_buf(),
        rel,
        size: path.metadata().map(|m| m.len()).unwrap_or(0),
        name,
    })
}

impl Source for Dji {
    fn kind(&self) -> SourceKind {
        SourceKind::Dji
    }

    fn detect(&self, root: &Path) -> bool {
        clip_dirs(root).iter().any(|d| {
            std::fs::read_dir(d).is_ok_and(|rd| {
                rd.flatten().any(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    !name.starts_with('.') && is_clip_name(&name)
                })
            })
        })
    }

    fn list(&self, root: &Path) -> Vec<FoundClip> {
        let mut out: Vec<FoundClip> = clip_dirs(root)
            .iter()
            .filter_map(|d| std::fs::read_dir(d).ok())
            .flat_map(|rd| rd.flatten().map(|e| e.path()).collect::<Vec<_>>())
            .filter_map(|p| found(&p, Some(root)))
            .collect();
        out.sort_by_key(order_key);
        out.dedup_by(|a, b| a.path == b.path);
        out
    }

    fn pick(&self, files: &[PathBuf]) -> Vec<FoundClip> {
        let mut out: Vec<FoundClip> = files.iter().filter_map(|p| found(p, None)).collect();
        out.sort_by_key(order_key);
        out.dedup_by(|a, b| a.path == b.path);
        out
    }

    /// The goggles' subtitle track: a same-stem `.SRT`.
    fn sidecars(&self, clip: &Path) -> Vec<PathBuf> {
        let (Some(dir), Some(stem)) = (clip.parent(), clip.file_stem()) else {
            return Vec::new();
        };
        let stem = stem.to_string_lossy();
        let Ok(rd) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut v: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_stem().is_some_and(|s| s.to_string_lossy() == stem)
                    && p.extension()
                        .is_some_and(|x| x.to_string_lossy().eq_ignore_ascii_case("srt"))
                    && p.is_file()
            })
            .collect();
        v.sort();
        v
    }

    /// Whole when it has a `moov` box. Without one (power lost while recording) there is
    /// no index to play the frames from.
    fn inspect(&self, staged: &Path) -> Result<Inspect> {
        Ok(Inspect {
            complete: crate::qtmeta::has_whole_moov(staged),
        })
    }

    fn repair_path(&self, staged: &Path) -> PathBuf {
        staged.with_file_name(format!(
            "{}.recovered.mp4",
            staged.file_stem().unwrap_or_default().to_string_lossy()
        ))
    }

    /// A DJI file without `moov` cannot be rebuilt here: the frame index is gone.
    fn repair(&self, _tools: &Tools, staged: &Path, _dst: &Path) -> Result<()> {
        bail!(
            "{} has no moov box (recording cut off); it cannot be repaired",
            staged.display()
        )
    }

    /// The local time in the file name, else the container's `creation_time` (UTC).
    fn intrinsic_time(&self, staged: &Path) -> Option<DateTime<Utc>> {
        let name = staged.file_name()?.to_string_lossy().to_string();
        parse_name(&name)
            .and_then(|(t, _)| Local.from_local_datetime(&t).earliest())
            .map(|t| t.with_timezone(&Utc))
            .or_else(|| crate::qtmeta::movie_time(staged).ok().flatten())
    }

    fn encode_plan(&self, _probe: &Probe, want: Format) -> EncodePlan {
        match want {
            // ffmpeg's MP4 muxer refuses DJI's data streams; the file already is an MP4.
            Format::Mp4 => EncodePlan::Copy,
            Format::Mov => EncodePlan::Remux,
        }
    }

    /// Blue screen and static are analog signals, and sampling 4K frames is slow.
    fn signal(
        &self,
        _tools: &Tools,
        _src: &Path,
        _fps: Option<f64>,
        _duration: f64,
    ) -> Option<SignalScan> {
        None
    }

    fn original_ext(&self, staged: &Path) -> String {
        staged
            .extension()
            .map(|x| x.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| "mp4".into())
    }

    /// Goggles format their own cards, and an air unit is not a card.
    fn card_policy(&self) -> CardPolicy {
        CardPolicy {
            format_offered: false,
            filesystem: "exFAT",
            warn_above_bytes: u64::MAX,
            filesystem_advice: "Import works; QuadCam only reads DJI cards.",
            size_warning: "",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{self, analog::Analog};

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }

    #[test]
    fn parses_names() {
        let (t, n) = parse_name("DJI_20261005183636_0001_D.MP4").unwrap();
        assert_eq!(t.to_string(), "2026-10-05 18:36:36");
        assert_eq!(n, 1);
        assert!(parse_name("dji_20261005183636_0012_d.mp4").is_some_and(|x| x.1 == 12));
        assert!(
            parse_name("DCIM_DJI_001_DJI_20261005183636_0002_D.MP4").is_some_and(|x| x.1 == 2),
            "a staged copy with a folder prefix"
        );
        assert!(parse_name("DJI_2026100518363_0001_D.MP4").is_none());
        assert!(
            parse_name("DJI_20261305183636_0001_D.MP4").is_none(),
            "month 13"
        );
        assert!(is_clip_name("DJI_20261005183636_0001_D.MP4"));
        assert!(is_clip_name("DJIG0007.MP4"));
        assert!(!is_clip_name("DJI_20261005183636_0001_D.SRT"));
        assert!(!is_clip_name("PICT0001.AVI"));
        assert!(!is_clip_name("DJI_001"));
    }

    #[test]
    fn detects_dji_before_analog_and_ignores_misc() {
        let empty = tempfile::tempdir().unwrap();
        assert!(sources::detect(empty.path()).is_none());

        let analog = tempfile::tempdir().unwrap();
        touch(&analog.path().join("DCIM/PICT0001.AVI"));
        assert!(!Dji.detect(analog.path()));
        assert_eq!(
            sources::detect(analog.path()).unwrap().kind(),
            SourceKind::Analog
        );

        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("DCIM/DJI_001");
        touch(&dir.join("DJI_20261005183636_0002_D.MP4"));
        touch(&dir.join("DJI_20261005183636_0001_D.MP4"));
        touch(&dir.join("DJI_20261005170000_0009_D.MP4"));
        touch(&dir.join("DJI_20261005183636_0001_D.SRT"));
        touch(&dir.join("._DJI_20261005183636_0001_D.MP4"));
        touch(&d.path().join("MISC/THM/DJI_20261005183636_0001_D.MP4"));
        touch(&d.path().join("MISC/FC8770.db"));
        // An AVI on the same volume would make analog's loose check say yes too.
        touch(&d.path().join("MISC/old.avi"));
        assert!(Analog.detect(d.path()));
        assert_eq!(sources::all()[0].kind(), SourceKind::Dji);
        assert_eq!(
            sources::detect(d.path()).unwrap().kind(),
            SourceKind::Dji,
            "DJI is tried first"
        );
        let names: Vec<String> = Dji.list(d.path()).into_iter().map(|c| c.rel).collect();
        assert_eq!(
            names,
            [
                "DCIM/DJI_001/DJI_20261005170000_0009_D.MP4",
                "DCIM/DJI_001/DJI_20261005183636_0001_D.MP4",
                "DCIM/DJI_001/DJI_20261005183636_0002_D.MP4",
            ],
            "by clock time, then counter; MISC and dot files left out"
        );
        assert_eq!(
            Dji.sidecars(&dir.join("DJI_20261005183636_0001_D.MP4")),
            [dir.join("DJI_20261005183636_0001_D.SRT")]
        );
        assert!(Dji
            .sidecars(&dir.join("DJI_20261005183636_0002_D.MP4"))
            .is_empty());
        // The DJI_001 folder itself, and dropped files.
        assert_eq!(Dji.list(&dir).len(), 3);
        let picked = Dji.pick(&[
            dir.join("DJI_20261005183636_0002_D.MP4"),
            dir.join("DJI_20261005183636_0001_D.SRT"),
            d.path().join("MISC/FC8770.db"),
        ]);
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].rel, "DJI_20261005183636_0002_D.MP4");
    }

    #[test]
    fn intrinsic_time_from_the_name_in_local_time() {
        let t = Dji
            .intrinsic_time(Path::new("/s/DJI_20261005183636_0001_D.MP4"))
            .unwrap();
        let want = Local
            .from_local_datetime(
                &NaiveDateTime::parse_from_str("2026-10-05 18:36:36", "%Y-%m-%d %H:%M:%S").unwrap(),
            )
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(t, want);
        assert_eq!(Dji.intrinsic_time(Path::new("/s/missing.MP4")), None);
    }

    #[test]
    fn plans_policy_and_repair() {
        let probe = Probe::default();
        assert_eq!(Dji.encode_plan(&probe, Format::Mp4), EncodePlan::Copy);
        assert_eq!(Dji.encode_plan(&probe, Format::Mov), EncodePlan::Remux);
        assert_eq!(Dji.original_ext(Path::new("/x/DJI_1.MP4")), "mp4");
        let p = Dji.card_policy();
        assert!(!p.format_offered);
        assert_eq!(p.filesystem, "exFAT");
        assert_eq!(p.warn_above_bytes, u64::MAX);
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("DJI_20261005183636_0001_D.MP4");
        // ftyp then a bare mdat: cut off before moov.
        let mut bytes = Vec::new();
        bytes.extend(16u32.to_be_bytes());
        bytes.extend(b"ftypisom\0\0\0\0");
        bytes.extend(16u32.to_be_bytes());
        bytes.extend(b"mdat01234567");
        std::fs::write(&f, &bytes).unwrap();
        assert!(!Dji.inspect(&f).unwrap().complete);
        let tools = Tools {
            ffmpeg: PathBuf::from("ffmpeg"),
            ffprobe: PathBuf::from("ffprobe"),
        };
        assert!(Dji.repair(&tools, &f, &Dji.repair_path(&f)).is_err());
        // A moov at the end: whole.
        bytes.extend(8u32.to_be_bytes());
        bytes.extend(b"moov");
        std::fs::write(&f, &bytes).unwrap();
        assert!(Dji.inspect(&f).unwrap().complete);
    }
}
