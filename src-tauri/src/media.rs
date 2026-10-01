//! ffmpeg and ffprobe: probe, recover, convert, verify, thumbnails and preview proxies.

use crate::sources::EncodePlan;
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Homebrew first. A GUI app does not inherit the shell PATH, so look there explicitly.
const TOOL_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin"];

pub const INSTALL_HINT: &str = "brew install ffmpeg";

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

fn find_tool(name: &str) -> Option<PathBuf> {
    let from_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    TOOL_DIRS
        .iter()
        .map(PathBuf::from)
        .chain(from_path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

pub fn find_tools() -> Result<Tools> {
    match (find_tool("ffmpeg"), find_tool("ffprobe")) {
        (Some(ffmpeg), Some(ffprobe)) => Ok(Tools { ffmpeg, ffprobe }),
        _ => bail!("ffmpeg and ffprobe not found. Install them with: {INSTALL_HINT}"),
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct Probe {
    pub duration: f64,
    pub video_packets: u64,
    pub video_streams: usize,
    pub audio_streams: usize,
    pub fps: Option<f64>,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub tags: BTreeMap<String, String>,
    /// Anything ffprobe printed at `-v error`.
    pub errors: String,
}

fn parse_rate(r: &str) -> Option<f64> {
    let (n, d) = r.split_once('/')?;
    let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
    (d > 0.0 && n > 0.0).then_some(n / d)
}

/// Counts packets and reads streams, duration and format tags.
pub fn probe(tools: &Tools, path: &Path) -> Result<Probe> {
    let out = Command::new(&tools.ffprobe)
        .args(["-v", "error", "-count_packets", "-show_entries"])
        .arg("stream=codec_type,nb_read_packets,r_frame_rate,width,height:format=duration:format_tags")
        .args(["-of", "json"])
        .arg(path)
        .output()
        .context("running ffprobe")?;
    let errors = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        bail!("ffprobe could not read {}: {errors}", path.display());
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).context("parsing ffprobe json")?;
    let mut p = Probe {
        errors,
        ..Default::default()
    };
    for s in v["streams"].as_array().into_iter().flatten() {
        match s["codec_type"].as_str() {
            Some("video") => {
                p.video_streams += 1;
                if p.video_streams == 1 {
                    p.video_packets = s["nb_read_packets"]
                        .as_str()
                        .and_then(|x| x.parse().ok())
                        .unwrap_or(0);
                    p.fps = s["r_frame_rate"].as_str().and_then(parse_rate);
                    p.width = s["width"].as_u64();
                    p.height = s["height"].as_u64();
                }
            }
            Some("audio") => p.audio_streams += 1,
            _ => {}
        }
    }
    p.duration = v["format"]["duration"]
        .as_str()
        .and_then(|x| x.parse().ok())
        .unwrap_or(0.0);
    if p.duration <= 0.0 {
        if let Some(fps) = p.fps {
            p.duration = p.video_packets as f64 / fps;
        }
    }
    if let Some(tags) = v["format"]["tags"].as_object() {
        for (k, val) in tags {
            if let Some(s) = val.as_str() {
                p.tags.insert(k.to_lowercase(), s.to_string());
            }
        }
    }
    Ok(p)
}

/// Rewrites a half-written AVI into a complete one: `-fflags +genpts`, stream copy.
pub fn recover(tools: &Tools, src: &Path, dst: &Path) -> Result<()> {
    let out = Command::new(&tools.ffmpeg)
        .args(["-v", "error", "-y", "-fflags", "+genpts", "-i"])
        .arg(src)
        .args(["-map", "0", "-c", "copy", "-f", "avi"])
        .arg(dst)
        .output()
        .context("running ffmpeg")?;
    if !out.status.success() || !dst.is_file() {
        let _ = std::fs::remove_file(dst);
        bail!(
            "recovery failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Mp4,
    Mov,
}

impl Format {
    pub fn ext(self) -> &'static str {
        match self {
            Format::Mp4 => "mp4",
            Format::Mov => "mov",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum Encoder {
    /// `h264_videotoolbox -q:v 65`, falling back to x264 on failure.
    Videotoolbox,
    /// `libx264 -preset veryfast -crf 20` (software fallback).
    X264,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct Meta {
    pub title: String,
    pub comment: String,
    pub creation_time: DateTime<Utc>,
    pub date: String,
    pub description: String,
}

impl Meta {
    fn args(&self) -> Vec<String> {
        let mut a = Vec::new();
        let mut put = |k: &str, v: &str| {
            if !v.is_empty() {
                a.push("-metadata".to_string());
                a.push(format!("{k}={v}"));
            }
        };
        put("title", &self.title);
        put("comment", &self.comment);
        put(
            "creation_time",
            &self.creation_time.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        );
        put("date", &self.date);
        put("description", &self.description);
        a
    }
}

/// The ffmpeg argument list for one conversion (after the global flags).
pub fn convert_args(
    src: &Path,
    dst: &Path,
    format: Format,
    plan: EncodePlan,
    encoder: Encoder,
    meta: &Meta,
) -> Vec<String> {
    let s = |p: &Path| p.to_string_lossy().to_string();
    let mut a: Vec<String> = Vec::new();
    match plan {
        EncodePlan::Transcode => {
            a.extend(["-i".into(), s(src), "-map".into(), "0".into()]);
            match encoder {
                Encoder::Videotoolbox => {
                    a.extend(["-c:v", "h264_videotoolbox", "-q:v", "65"].map(String::from))
                }
                Encoder::X264 => a.extend(
                    ["-c:v", "libx264", "-preset", "veryfast", "-crf", "20"].map(String::from),
                ),
            }
            a.extend(["-c:a", "aac", "-b:a", "128k", "-fps_mode", "passthrough"].map(String::from));
            // No `+faststart`: `qtmeta` adds metadata by rewriting the `moov` box in place,
            // which needs it at the end of the file. Local playback and Photos do not care.
            a.extend(meta.args());
            a.extend(["-f".into(), format.ext().into(), s(dst)]);
        }
        EncodePlan::Remux => {
            a.extend(["-fflags".into(), "+genpts".into(), "-i".into(), s(src)]);
            a.extend(["-map", "0", "-c", "copy"].map(String::from));
            // No `use_metadata_tags`: its keys land where Apple's frameworks cannot read
            // them, and they confuse ffprobe next to ours. `qtmeta` writes the QuickTime
            // keys (description included) after ffmpeg.
            a.extend(meta.args());
            a.extend(["-f".into(), format.ext().into(), s(dst)]);
        }
    }
    a
}

/// Runs ffmpeg with `-progress`, calling `on_progress` with seconds of output written.
pub fn run_ffmpeg(tools: &Tools, args: &[String], on_progress: &mut dyn FnMut(f64)) -> Result<()> {
    let mut child = Command::new(&tools.ffmpeg)
        .args([
            "-v",
            "error",
            "-nostdin",
            "-y",
            "-progress",
            "pipe:1",
            "-nostats",
        ])
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting ffmpeg")?;
    let stderr = child.stderr.take().expect("piped stderr");
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut BufReader::new(stderr), &mut s);
        s
    });
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(us) = line
                .strip_prefix("out_time_us=")
                .and_then(|x| x.parse::<i64>().ok())
            {
                on_progress(us.max(0) as f64 / 1e6);
            }
        }
    }
    let status = child.wait()?;
    let err = err_thread.join().unwrap_or_default();
    if !status.success() {
        bail!("ffmpeg failed: {}", err.trim());
    }
    Ok(())
}

/// Converts, falling back from VideoToolbox to x264 if the hardware encoder fails.
/// Returns the encoder that produced the file.
#[allow(clippy::too_many_arguments)]
pub fn convert(
    tools: &Tools,
    src: &Path,
    dst: &Path,
    format: Format,
    plan: EncodePlan,
    encoder: Encoder,
    meta: &Meta,
    on_progress: &mut dyn FnMut(f64),
) -> Result<Encoder> {
    let r = run_ffmpeg(
        tools,
        &convert_args(src, dst, format, plan, encoder, meta),
        on_progress,
    );
    match r {
        Ok(()) => Ok(encoder),
        Err(e) if plan == EncodePlan::Transcode && encoder == Encoder::Videotoolbox => {
            let _ = std::fs::remove_file(dst);
            run_ffmpeg(
                tools,
                &convert_args(src, dst, format, plan, Encoder::X264, meta),
                on_progress,
            )
            .map_err(|e2| anyhow!("{e}; x264 fallback also failed: {e2}"))?;
            Ok(Encoder::X264)
        }
        Err(e) => Err(e),
    }
}

/// The ffmpeg argument list for one cut of `span` from `src` (after the global flags).
///
/// Cuts come from the DVR's MJPEG source, not from the H.264 output. MJPEG is all-intra:
/// every frame is a keyframe, so a seek lands on the exact frame.
/// - MP4 re-encodes the range with the same encoder as the export. Input `-ss` with
///   re-encoding is frame-accurate, and the cut plays everywhere the export does. Stream-copying
///   the H.264 export instead would snap the start to its GOP keyframes (seconds apart).
/// - MOV stream-copies the MJPEG range, which is lossless and still frame-exact for the
///   same reason.
pub fn cut_args(
    src: &Path,
    dst: &Path,
    span: crate::moments::Span,
    format: Format,
    encoder: Encoder,
    meta: &Meta,
) -> Vec<String> {
    let s = |p: &Path| p.to_string_lossy().to_string();
    let mut a: Vec<String> = Vec::new();
    if format == Format::Mov {
        a.extend(["-fflags".into(), "+genpts".into()]);
    }
    a.extend([
        "-ss".into(),
        format!("{:.3}", span.start),
        "-i".into(),
        s(src),
        "-t".into(),
        format!("{:.3}", span.secs()),
        "-map".into(),
        "0".into(),
    ]);
    match format {
        Format::Mp4 => {
            match encoder {
                Encoder::Videotoolbox => {
                    a.extend(["-c:v", "h264_videotoolbox", "-q:v", "65"].map(String::from))
                }
                Encoder::X264 => a.extend(
                    ["-c:v", "libx264", "-preset", "veryfast", "-crf", "20"].map(String::from),
                ),
            }
            a.extend(["-c:a", "aac", "-b:a", "128k"].map(String::from));
            a.extend(meta.args());
            a.extend(["-f".into(), "mp4".into(), s(dst)]);
        }
        Format::Mov => {
            a.extend(["-c", "copy"].map(String::from));
            a.extend(meta.args());
            a.extend(["-f".into(), "mov".into(), s(dst)]);
        }
    }
    a
}

/// Writes one cut, falling back from VideoToolbox to x264 like `convert`.
pub fn cut(
    tools: &Tools,
    src: &Path,
    dst: &Path,
    span: crate::moments::Span,
    format: Format,
    encoder: Encoder,
    meta: &Meta,
) -> Result<Encoder> {
    match run_ffmpeg(
        tools,
        &cut_args(src, dst, span, format, encoder, meta),
        &mut |_| {},
    ) {
        Ok(()) => Ok(encoder),
        Err(e) if format == Format::Mp4 && encoder == Encoder::Videotoolbox => {
            let _ = std::fs::remove_file(dst);
            run_ffmpeg(
                tools,
                &cut_args(src, dst, span, format, Encoder::X264, meta),
                &mut |_| {},
            )
            .map_err(|e2| anyhow!("{e}; x264 fallback also failed: {e2}"))?;
            Ok(Encoder::X264)
        }
        Err(e) => Err(e),
    }
}

/// Checks a cut: it opens, has one video stream, audio when the source had it, and its frame
/// count and duration match the range within a frame (plus the usual duration tolerance).
pub fn verify_cut(
    tools: &Tools,
    src: &Probe,
    out: &Path,
    span: crate::moments::Span,
) -> Result<Probe> {
    let p = probe(tools, out).context("cut does not open")?;
    if p.video_streams != 1 {
        bail!("cut has {} video streams, expected 1", p.video_streams);
    }
    let want_audio = usize::from(src.audio_streams > 0);
    if p.audio_streams != want_audio {
        bail!(
            "cut has {} audio streams, expected {want_audio}",
            p.audio_streams
        );
    }
    let fps = src.fps.unwrap_or(30.0);
    let want = span.secs() * fps;
    if (p.video_packets as f64 - want).abs() > 1.5 {
        bail!(
            "cut has {} frames, expected about {:.0}",
            p.video_packets,
            want
        );
    }
    if (p.duration - span.secs()).abs() > DURATION_TOLERANCE + 1.0 / fps {
        bail!(
            "cut is {:.3}s long, expected {:.3}s",
            p.duration,
            span.secs()
        );
    }
    Ok(p)
}

/// Duration tolerance between source and output.
pub const DURATION_TOLERANCE: f64 = 0.1;

/// Output checks. `src` is the probe of the file that was converted
/// (the recovered copy for a half-written clip).
pub fn verify(tools: &Tools, src: &Probe, out: &Path, meta: &Meta) -> Result<Probe> {
    let p = probe(tools, out).context("output does not open")?;
    if p.video_streams != 1 {
        bail!("output has {} video streams, expected 1", p.video_streams);
    }
    let want_audio = usize::from(src.audio_streams > 0);
    if p.audio_streams != want_audio {
        bail!(
            "output has {} audio streams, expected {want_audio}",
            p.audio_streams
        );
    }
    if p.video_packets != src.video_packets {
        bail!(
            "frame count {} does not match source {}",
            p.video_packets,
            src.video_packets
        );
    }
    if (p.duration - src.duration).abs() > DURATION_TOLERANCE {
        bail!(
            "duration {:.3}s differs from source {:.3}s",
            p.duration,
            src.duration
        );
    }
    // A MOV keeps some tags only as QuickTime keys (written by `qtmeta`).
    let tag = |k: &str| {
        p.tags
            .get(k)
            .or_else(|| p.tags.get(&format!("com.apple.quicktime.{k}")))
            .map(String::as_str)
            .unwrap_or("")
    };
    for (k, want) in [
        ("title", &meta.title),
        ("comment", &meta.comment),
        ("date", &meta.date),
        ("description", &meta.description),
    ] {
        if tag(k) != want.as_str() {
            bail!("metadata {k} reads back as {:?}, expected {want:?}", tag(k));
        }
    }
    let ct = tag("creation_time");
    // Some muxer setups report the mvhd time and a key, joined by ';'.
    let ok = ct.split(';').any(|c| {
        DateTime::parse_from_rfc3339(c.trim()).is_ok_and(|t| {
            (t.with_timezone(&Utc) - meta.creation_time)
                .num_seconds()
                .abs()
                <= 1
        })
    });
    if !ok {
        bail!(
            "creation_time reads back as {ct:?}, expected {}",
            meta.creation_time.to_rfc3339()
        );
    }
    Ok(p)
}

/// Sets the file's modified time so Finder sorts by flight date.
pub fn set_mtime(path: &Path, t: DateTime<Utc>) -> Result<()> {
    let f = std::fs::File::options().write(true).open(path)?;
    f.set_modified(t.into())?;
    Ok(())
}

/// `ffmpeg -ss 1 -i in.avi -frames:v 1 -vf scale=240:-1 thumb.jpg`; retries at 0 s for short clips.
pub fn thumbnail(tools: &Tools, src: &Path, dst: &Path) -> Result<()> {
    for ss in ["1", "0"] {
        let out = Command::new(&tools.ffmpeg)
            .args(["-v", "error", "-y", "-ss", ss, "-i"])
            .arg(src)
            .args(["-frames:v", "1", "-vf", "scale=240:-1"])
            .arg(dst)
            .output()?;
        if out.status.success() && dst.metadata().is_ok_and(|m| m.len() > 0) {
            return Ok(());
        }
    }
    bail!("could not make a thumbnail")
}

/// A small H.264 proxy the webview can play, since it cannot play MJPEG AVI.
pub fn proxy(tools: &Tools, src: &Path, dst: &Path) -> Result<()> {
    let tmp = dst.with_extension("part.mp4");
    let out = Command::new(&tools.ffmpeg)
        .args(["-v", "error", "-y", "-i"])
        .arg(src)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-vf",
            "scale=480:-2",
            "-c:v",
            "h264_videotoolbox",
            "-q:v",
            "50",
        ])
        .args([
            "-c:a",
            "aac",
            "-b:a",
            "96k",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ])
        .arg(&tmp)
        .output()?;
    if !out.status.success() {
        let out = Command::new(&tools.ffmpeg)
            .args(["-v", "error", "-y", "-i"])
            .arg(src)
            .args([
                "-map",
                "0:v:0",
                "-map",
                "0:a:0?",
                "-vf",
                "scale=480:-2",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
            ])
            .args([
                "-c:a",
                "aac",
                "-b:a",
                "96k",
                "-movflags",
                "+faststart",
                "-f",
                "mp4",
            ])
            .arg(&tmp)
            .output()?;
        if !out.status.success() {
            bail!(
                "proxy failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
    }
    std::fs::rename(&tmp, dst)?;
    Ok(())
}

/// Samples about `sample_fps` frames per second of `src`'s first video stream, each scaled to
/// `w` x `h` planar YUV 4:4:4, for dead-air detection. Returns each frame with its time in
/// seconds, sorted, at most one per `1/sample_fps` bucket.
///
/// Times come from each frame's own timestamp (ffmpeg's `showinfo`), never from its position:
/// DVR files drop frames (one test clip lacks over 60 of every 1200 sample points), and
/// counting frames would drift by seconds over a long clip.
///
/// Speed: a DVR clip is MJPEG, every frame a keyframe, and decoding is most of the cost. The
/// `noise` bitstream filter with `drop` discards every packet except the first two of each
/// bucket (by timestamp) before the decoder, so only those frames are decoded: about 3 s for
/// a 10-minute 1.4 GB clip, against about 5 s or more for decode-everything plus the `fps`
/// filter. Two per bucket, so one dropped or broken frame does not leave a hole. If this
/// ffmpeg refuses the bitstream filter, it falls back to the `fps` filter.
pub fn sample_frames(
    tools: &Tools,
    src: &Path,
    fps: Option<f64>,
    sample_fps: f64,
    w: usize,
    h: usize,
) -> Result<Vec<(f64, Vec<u8>)>> {
    let step = 1.0 / sample_fps;
    let vf = |pre: &str| format!("{pre}showinfo,scale={w}:{h}:flags=neighbor");
    let run = |pre: &[String], vf: &str| -> Result<Vec<(f64, Vec<u8>)>> {
        let out = Command::new(&tools.ffmpeg)
            .args(["-hide_banner", "-nostats", "-loglevel", "info", "-nostdin"])
            .args(pre)
            .arg("-i")
            .arg(src)
            .args([
                "-map",
                "0:v:0",
                "-an",
                "-fps_mode",
                "passthrough",
                "-vf",
                vf,
            ])
            .args(["-pix_fmt", "yuv444p", "-f", "rawvideo", "-"])
            .output()
            .context("running ffmpeg")?;
        let log = String::from_utf8_lossy(&out.stderr);
        if !out.status.success() {
            bail!("frame sampling failed: {}", log.trim());
        }
        let times: Vec<f64> = log
            .lines()
            .filter(|l| l.contains("Parsed_showinfo"))
            .filter_map(|l| l.split("pts_time:").nth(1))
            .filter_map(|x| x.split_whitespace().next()?.parse().ok())
            .collect();
        let frames: Vec<&[u8]> = out.stdout.chunks_exact(w * h * 3).collect();
        if times.len() != frames.len() {
            bail!(
                "frame sampling: {} frames but {} timestamps",
                frames.len(),
                times.len()
            );
        }
        let mut pairs: Vec<(f64, Vec<u8>)> = Vec::new();
        for (t, f) in times.into_iter().zip(frames) {
            let bucket = |x: f64| (x / step + 1e-6).floor() as i64;
            if pairs.last().is_some_and(|(p, _)| bucket(*p) >= bucket(t)) {
                continue;
            }
            pairs.push((t, f.to_vec()));
        }
        Ok(pairs)
    };
    if let Some(fps) = fps.filter(|f| *f > 0.0) {
        let bsf = format!("noise=drop=gte(mod(pts*tb\\,{step})\\,{:.6})", 2.5 / fps);
        if let Ok(frames) = run(&["-bsf:v".into(), bsf], &vf("")) {
            if !frames.is_empty() {
                return Ok(frames);
            }
        }
    }
    run(&[], &vf(&format!("fps={sample_fps},")))
}

/// exiftool, when installed. Optional: verify uses it as a second reader.
pub fn find_exiftool() -> Option<PathBuf> {
    find_tool("exiftool")
}

/// Checks that every QuickTime item and the location read back: through ffprobe always, and
/// through exiftool's `Keys` and `UserData` groups when exiftool is installed.
pub fn verify_qt(tools: &Tools, out: &Path, items: &[(String, String)]) -> Result<()> {
    let p = probe(tools, out).context("output does not open")?;
    for (k, want) in items {
        let got = p
            .tags
            .get(&k.to_lowercase())
            .map(String::as_str)
            .unwrap_or("");
        if got != want && !got.split(';').any(|x| x == want) {
            bail!("metadata {k} reads back as {got:?}, expected {want:?}");
        }
    }
    let loc = items
        .iter()
        .find(|(k, _)| k == "com.apple.quicktime.location.ISO6709")
        .map(|(_, v)| v.as_str());
    let (Some(loc), Some(exif)) = (loc, find_exiftool()) else {
        return Ok(());
    };
    let out = Command::new(exif)
        .args([
            "-j",
            "-n",
            "-Keys:GPSCoordinates",
            "-UserData:GPSCoordinates",
            "-G1",
        ])
        .arg(out)
        .output()
        .context("running exiftool")?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    let want = parse_iso6709(loc).context("bad ISO 6709 value")?;
    for g in ["Keys:GPSCoordinates", "UserData:GPSCoordinates"] {
        let got = v[0][g]
            .as_str()
            .map(str::to_string)
            .or_else(|| v[0][g].as_f64().map(|x| x.to_string()));
        let ok = got.as_deref().and_then(|s| {
            let mut it = s.split_whitespace().filter_map(|x| x.parse::<f64>().ok());
            Some((it.next()?, it.next()?))
        });
        match ok {
            Some((la, lo)) if (la - want.0).abs() < 1e-3 && (lo - want.1).abs() < 1e-3 => {}
            _ => bail!("exiftool reads {g} as {got:?}, expected {loc}"),
        }
    }
    Ok(())
}

/// `+40.6892-074.0445/` to (lat, lon).
pub fn parse_iso6709(s: &str) -> Option<(f64, f64)> {
    let s = s.trim().trim_end_matches('/');
    let split = s[1..].find(['+', '-'])? + 1;
    Some((s[..split].parse().ok()?, s[split..].parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(format: Format, plan: EncodePlan) -> String {
        let meta = Meta {
            title: String::new(),
            comment: String::new(),
            creation_time: DateTime::<Utc>::default(),
            date: String::new(),
            description: String::new(),
        };
        convert_args(
            Path::new("in"),
            Path::new("out"),
            format,
            plan,
            Encoder::Videotoolbox,
            &meta,
        )
        .join(" ")
    }

    /// The analog plans give the arguments they always had; the other two pairs follow.
    #[test]
    fn the_encode_plan_picks_the_codec_and_the_format_picks_the_container() {
        let mp4 = args(Format::Mp4, EncodePlan::Transcode);
        assert!(mp4.starts_with("-i in -map 0 -c:v h264_videotoolbox -q:v 65 -c:a aac"));
        assert!(mp4.ends_with("-f mp4 out"));
        let mov = args(Format::Mov, EncodePlan::Remux);
        assert!(mov.starts_with("-fflags +genpts -i in -map 0 -c copy"));
        assert!(mov.ends_with("-f mov out"));
        assert!(args(Format::Mp4, EncodePlan::Remux).ends_with("-f mp4 out"));
        assert!(args(Format::Mov, EncodePlan::Transcode).contains("h264_videotoolbox"));
    }
}
