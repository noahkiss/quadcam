//! `Core::library_overlay`: a library clip's telemetry as a subtitle file, and with `burn` a
//! copy of the clip with the numbers drawn in (`crate::telemetry`, `docs/telemetry-overlay.md`).
//! The numbers come from a read-only radio-log match or from a goggles `.srt` sidecar. The
//! files go to a folder outside the library; the clip itself is never changed.

use super::rematch::LibMatchParams;
use super::Core;
use crate::library::LibClip;
use crate::logs::Badge;
use crate::media;
use crate::telemetry::{self, Sample, TelemetryField};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where the numbers come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum OverlaySource {
    /// The EdgeTX radio log the clip matches.
    Log,
    /// A goggles `.srt` sidecar: `srt`, else the one kept beside the clip's original.
    Srt,
}

/// `library_overlay`: one clip, the source, the fields, and where to write.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct OverlayParams {
    /// A library clip id.
    pub id: String,
    /// None: the radio log when the clip matches one, else the sidecar.
    pub source: Option<OverlaySource>,
    /// A goggles `.srt` to read instead of the kept sidecar.
    pub srt: Option<PathBuf>,
    /// The log folder; the `logDir` setting when missing.
    pub logs: Option<PathBuf>,
    /// The numbers to show, in order; empty for every number the source has.
    pub fields: Vec<TelemetryField>,
    /// Also write a copy of the clip with the numbers burned in.
    pub burn: bool,
    /// The folder to write to; the `overlayDir` setting, else `~/Movies/quadcam-overlays`,
    /// when missing.
    pub out: Option<PathBuf>,
    /// Replace `<clip>.telemetry.srt` and `.mp4` when they exist. Without it a run adds a
    /// number to the name and never replaces a file.
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct OverlayReport {
    pub id: String,
    pub source: OverlaySource,
    /// The radio log's match (`matched`, `likely`) or the sidecar file read.
    pub from: String,
    /// The fields the source has, and the ones shown.
    pub available: Vec<TelemetryField>,
    pub fields: Vec<TelemetryField>,
    pub cues: usize,
    /// The span the cues cover, in clip seconds.
    pub first_s: Option<f64>,
    pub last_s: Option<f64>,
    pub srt: PathBuf,
    /// The burned-in copy, when `burn` made one.
    pub video: Option<PathBuf>,
    /// Sidecar keys the overlay does not show.
    pub unknown_keys: Vec<String>,
    pub warnings: Vec<String>,
}

/// `dir/<stem>.telemetry.<ext>`; without `overwrite`, ` 2`, ` 3`, ... before `.telemetry`
/// when it exists.
fn out_name(dir: &Path, stem: &str, ext: &str, overwrite: bool) -> PathBuf {
    let mut n = 1;
    loop {
        let name = match n {
            1 => format!("{stem}.telemetry.{ext}"),
            _ => format!("{stem} {n}.telemetry.{ext}"),
        };
        let p = dir.join(name);
        if overwrite || std::fs::symlink_metadata(&p).is_err() {
            return p;
        }
        n += 1;
    }
}

/// Moves the finished `part` to `to`. Without `overwrite` it never replaces a file: a hard
/// link fails when `to` exists, where a rename would replace it.
fn publish(part: &Path, to: &Path, overwrite: bool) -> Result<()> {
    let done = if overwrite {
        std::fs::rename(part, to)
    } else {
        std::fs::hard_link(part, to).and_then(|_| std::fs::remove_file(part))
    };
    if let Err(e) = done {
        let _ = std::fs::remove_file(part);
        return Err(e).with_context(|| format!("writing {}", to.display()));
    }
    Ok(())
}

/// Writes `text` to `to` through a hidden `.part` file.
fn write_new(to: &Path, text: &str, overwrite: bool) -> Result<()> {
    let part = part_of(to);
    std::fs::write(&part, text).with_context(|| format!("writing {}", part.display()))?;
    publish(&part, to, overwrite)
}

/// `out` or the folder it would be made in, with symlinks resolved, is inside `root`.
fn in_library(out: &Path, root: &Path) -> bool {
    if out.starts_with(root) {
        return true;
    }
    let Ok(root) = root.canonicalize() else {
        return false;
    };
    out.ancestors()
        .find_map(|a| a.canonicalize().ok())
        .is_some_and(|a| a.starts_with(&root))
}

fn part_of(p: &Path) -> PathBuf {
    let name = p.file_name().unwrap_or_default().to_string_lossy();
    p.with_file_name(format!(".{name}.part"))
}

/// The kept `.srt` sidecar of a clip's original.
fn kept_srt(root: &Path, c: &LibClip) -> Option<PathBuf> {
    c.sidecars(root).into_iter().find(|p| {
        p.extension()
            .is_some_and(|x| x.to_string_lossy().eq_ignore_ascii_case("srt"))
    })
}

impl Core {
    /// The clip's log samples from a read-only match of its day's clips, with the badge, or
    /// why there are none.
    fn overlay_log(
        &self,
        clip: &LibClip,
        all: &[LibClip],
        logs: Option<PathBuf>,
    ) -> Result<(Vec<Sample>, Badge), String> {
        let ids: Vec<String> = all
            .iter()
            .filter(|c| c.date == clip.date && c.cut_of.is_none())
            .map(|c| c.id.clone())
            .collect();
        let r = self
            .library_match_logs(&LibMatchParams {
                ids,
                logs,
                day: None,
                apply: false,
            })
            .map_err(|e| e.to_string())?;
        let m = r
            .clips
            .into_iter()
            .find(|m| m.id == clip.id)
            .ok_or_else(|| "no radio log match for the clip".to_string())?;
        match m.badge {
            Badge::Unmatched => Err(format!(
                "no radio log matches the clip{}",
                m.reason.map(|r| format!(" ({r})")).unwrap_or_default()
            )),
            b if m.telemetry.is_empty() => {
                Err(format!("the {b:?} radio log has no rows for the clip").to_lowercase())
            }
            b => Ok((m.telemetry, b)),
        }
    }

    pub fn library_overlay(&self, p: &OverlayParams) -> Result<OverlayReport> {
        let (root, clips) =
            self.with_index(|root, ix| Ok(((root.to_path_buf(), ix.clips.clone()), false)))?;
        let clip = clips
            .iter()
            .find(|c| c.id == p.id || c.aliases.contains(&p.id))
            .with_context(|| format!("No clip {} in the library.", p.id))?
            .clone();
        if clip.cut_of.is_some() {
            bail!("{} is a kept cut; make the overlay from its clip.", p.id);
        }
        let mut warnings = Vec::new();
        let sidecar_path = p.srt.clone().or_else(|| kept_srt(&root, &clip));
        let logs = p.logs.clone().or(self.defaults().log_dir.clone());

        let read_sidecar = |path: &Path| -> Result<(telemetry::Sidecar, String)> {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            Ok((telemetry::from_sidecar(&text), path.display().to_string()))
        };
        let (source, samples, from, unknown_keys) = match p.source {
            Some(OverlaySource::Log) => {
                if logs.is_none() {
                    bail!("No log folder: give logs, or set logDir.");
                }
                let (s, b) = self
                    .overlay_log(&clip, &clips, logs)
                    .map_err(anyhow::Error::msg)?;
                (
                    OverlaySource::Log,
                    s,
                    format!("{b:?}").to_lowercase(),
                    Vec::new(),
                )
            }
            Some(OverlaySource::Srt) => {
                let path = sidecar_path.context("The clip has no kept .srt sidecar; give srt.")?;
                let (s, from) = read_sidecar(&path)?;
                (OverlaySource::Srt, s.samples, from, s.unknown_keys)
            }
            None => {
                let log = match &logs {
                    Some(_) => self.overlay_log(&clip, &clips, logs.clone()),
                    None => Err("no log folder set".into()),
                };
                match (log, &sidecar_path) {
                    (Ok((s, b)), _) => (
                        OverlaySource::Log,
                        s,
                        format!("{b:?}").to_lowercase(),
                        Vec::new(),
                    ),
                    (Err(why), Some(path)) => {
                        warnings.push(format!("Radio log: {why}; used the .srt sidecar."));
                        let (s, from) = read_sidecar(path)?;
                        (OverlaySource::Srt, s.samples, from, s.unknown_keys)
                    }
                    (Err(why), None) => bail!(
                        "Nothing to show: {why}, and the clip has no .srt sidecar (give srt)."
                    ),
                }
            }
        };
        if samples.is_empty() {
            bail!(
                "The {} has no numbers QuadCam shows{}.",
                match source {
                    OverlaySource::Log => "radio log",
                    OverlaySource::Srt => "sidecar",
                },
                if unknown_keys.is_empty() {
                    String::new()
                } else {
                    format!(" (keys: {})", unknown_keys.join(", "))
                }
            );
        }
        let available = telemetry::available(&samples);
        let fields: Vec<TelemetryField> = if p.fields.is_empty() {
            available.clone()
        } else {
            let missing: Vec<&str> = p
                .fields
                .iter()
                .filter(|f| !available.contains(f))
                .map(|f| f.name())
                .collect();
            if !missing.is_empty() {
                warnings.push(format!("The source has no {}.", missing.join(", ")));
            }
            p.fields.clone()
        };
        let cells = telemetry::cells(&samples);
        let (text, cues) = telemetry::to_srt(&samples, &fields, cells, clip.duration);
        if cues == 0 {
            bail!("No numbers fall inside the clip's {:.0} s.", clip.duration);
        }
        let inside = |s: &&Sample| s.end > 0.0 && s.start < clip.duration;
        let first_s = samples
            .iter()
            .filter(inside)
            .map(|s| s.start.max(0.0))
            .reduce(f64::min);
        let last_s = samples
            .iter()
            .filter(inside)
            .map(|s| s.end.min(clip.duration))
            .reduce(f64::max);

        let out = p
            .out
            .clone()
            .or_else(|| {
                let values = self
                    .settings_file
                    .as_deref()
                    .map(|f| crate::settings::read(f).unwrap_or_default())
                    .unwrap_or_default();
                crate::settings::overlay_dir_from(&values)
            })
            .context("No out folder and no home folder.")?;
        if !out.is_absolute() {
            bail!("{} is not an absolute folder path.", out.display());
        }
        if in_library(&out, &root) {
            bail!(
                "{} is inside the library folder; pick a folder outside it.",
                out.display()
            );
        }
        std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
        let stem = clip
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let srt = out_name(&out, &stem, "srt", p.overwrite);
        write_new(&srt, &text, p.overwrite)?;

        let mut video = None;
        if p.burn {
            let tools = media::find_tools()?;
            if !telemetry::can_burn(&tools)? {
                warnings.push(
                    "This ffmpeg has no subtitles filter (libass), so no burned-in copy was made. Homebrew's ffmpeg has it: set ffmpeg_source to homebrew.".into(),
                );
            } else {
                let dst = out_name(&out, &stem, "mp4", p.overwrite);
                let part = part_of(&dst);
                let src = root.join(&clip.path);
                let encoder = self.defaults().encoder;
                let run = |e: media::Encoder| {
                    media::run_ffmpeg(
                        &tools,
                        &telemetry::burn_args(&src, &srt, &part, e),
                        &mut |_| {},
                    )
                };
                let ran = match run(encoder) {
                    Err(_) if encoder == media::Encoder::Videotoolbox => run(media::Encoder::X264),
                    r => r,
                };
                let checked = ran.and_then(|_| {
                    let got = media::probe(&tools, &part)?;
                    if got.video_streams == 0 || (got.duration - clip.duration).abs() > 1.0 {
                        bail!(
                            "the burned-in copy is {:.1} s, the clip {:.1} s",
                            got.duration,
                            clip.duration
                        );
                    }
                    Ok(())
                });
                if let Err(e) = checked {
                    let _ = std::fs::remove_file(&part);
                    return Err(e.context("burning the overlay in"));
                }
                publish(&part, &dst, p.overwrite)?;
                video = Some(dst);
            }
        }
        Ok(OverlayReport {
            id: clip.id,
            source,
            from,
            available,
            fields,
            cues,
            first_s,
            last_s,
            srt,
            video,
            unknown_keys,
            warnings,
        })
    }
}
