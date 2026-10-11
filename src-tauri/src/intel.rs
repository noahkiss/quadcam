//! The Swift helper `quadcam-intel` (`intel/` in the repo, `docs/development.md`): Apple's
//! on-device frameworks behind a child process that prints one JSON object per line. The app
//! bundles it beside its own binaries (`Contents/MacOS`); a cargo build puts it in
//! `target/<profile>/`. It runs only through `modules::run::Runner`, by absolute path, with
//! a timeout. A missing helper, a macOS older than a feature needs, or a locale without its
//! speech assets makes that feature unavailable, never an error.
//!
//! Features: `asr` (the speech check of voice lines, macOS 26+) and `video` (highlights and the
//! key frame of a clip, MediaIntelligence's VideoAnalyzer, macOS 27+).
//!
//! A process started by cargo gets no helper unless `QUADCAM_INTEL=real`, so tests never
//! depend on the Mac they run on; they pass a fake `Runner` through `Intel::with`.

use crate::modules::run::{Process, Runner};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The helper's file name, beside the app's binaries.
pub const HELPER: &str = "quadcam-intel";

/// One feature as the helper's `caps` reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct Feature {
    /// `asr`: the speech check of voice lines; `video`: highlights and the key frame.
    pub name: String,
    pub available: bool,
    /// The oldest macOS the feature runs on.
    #[serde(default)]
    pub min_os: String,
    /// Why it is unavailable, when it is.
    #[serde(default)]
    pub reason: Option<String>,
    /// The locale asked about, and the state of its assets: `installed`, `supported` (can be
    /// installed), `downloading` or `unsupported`.
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub assets: Option<String>,
}

impl Feature {
    fn unavailable(name: &str, reason: &str) -> Self {
        Feature {
            name: name.into(),
            available: false,
            min_os: String::new(),
            reason: Some(reason.into()),
            locale: None,
            assets: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Caps {
    #[serde(default)]
    features: Vec<Feature>,
}

/// A word and where it is in the file, in seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

/// What the recognizer heard in one file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Heard {
    pub file: String,
    /// The text; absent when the file failed (`error`).
    #[serde(default)]
    pub text: Option<String>,
    /// The words' mean confidence, 0-1.
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub words: Vec<Word>,
    #[serde(default)]
    pub error: Option<String>,
}

/// A time range in a video file, in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Range {
    pub start: f64,
    pub end: f64,
}

/// One segment of the analyzer's interest curve: 0 dull, higher more interesting.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Level {
    pub start: f64,
    pub end: f64,
    pub level: f64,
}

/// What the video analyzer found in one file. Each request reports its own error; `error`
/// means the file could not be analysed at all.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Watched {
    pub file: String,
    /// None when highlights were not asked for, or failed (`highlights_error`).
    #[serde(default)]
    pub highlights: Option<Vec<Range>>,
    #[serde(default)]
    pub levels: Vec<Level>,
    /// Seconds into the file: the frame that best stands for the clip.
    #[serde(default)]
    pub keyframe: Option<f64>,
    #[serde(default)]
    pub highlights_error: Option<String>,
    #[serde(default)]
    pub keyframe_error: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// The answer of a video analysis: what the analyzer found, or why the feature is
/// unavailable.
#[derive(Debug, Clone, Default)]
pub struct VideoAnswer {
    pub watched: Option<Watched>,
    pub unavailable: Option<String>,
}

/// The answer of a transcription: what each file said, or why the feature is unavailable.
#[derive(Debug, Clone, Default)]
pub struct Transcripts {
    pub heard: Vec<Heard>,
    pub unavailable: Option<String>,
}

/// One line of the helper's output, by its `kind`.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Out {
    Caps(Caps),
    Asr(Heard),
    Video(Watched),
    Install { status: String },
    Error { code: String, message: String },
}

/// The exit code the helper uses for "unavailable here".
const UNAVAILABLE: i32 = 3;

/// Files per helper run: a run's timeout grows with its files, so one stuck file cannot hold
/// a long list.
const CHUNK: usize = 40;

/// The helper, or why there is none.
pub struct Intel {
    program: std::result::Result<PathBuf, String>,
    runner: Arc<dyn Runner>,
    /// `caps` per locale, asked once per process (an install clears it).
    caps: Mutex<Vec<(String, Feature)>>,
    /// The `video` feature from `caps`, asked once per process.
    video: Mutex<Option<Feature>>,
}

impl Intel {
    /// The helper beside this process's binary (or one folder up, for a test binary in
    /// `deps/`). None in a process started by cargo unless `QUADCAM_INTEL=real`.
    pub fn system() -> Self {
        let under_cargo = std::env::var_os("CARGO_MANIFEST_DIR").is_some();
        if under_cargo && std::env::var("QUADCAM_INTEL").as_deref() != Ok("real") {
            return Self::unavailable("The speech check is off in a process started by cargo (QUADCAM_INTEL=real turns it on).");
        }
        match locate() {
            Some(p) => Self::with(p, Arc::new(Process)),
            None => Self::unavailable("This copy of QuadCam has no quadcam-intel helper."),
        }
    }

    /// A helper at `program`, run by `runner` (tests pass a fake).
    pub fn with(program: PathBuf, runner: Arc<dyn Runner>) -> Self {
        Intel {
            program: Ok(program),
            runner,
            caps: Mutex::new(Vec::new()),
            video: Mutex::new(None),
        }
    }

    /// No helper, for this reason.
    pub fn unavailable(reason: &str) -> Self {
        Intel {
            program: Err(reason.into()),
            runner: Arc::new(Process),
            caps: Mutex::new(Vec::new()),
            video: Mutex::new(None),
        }
    }

    fn run(&self, args: Vec<OsString>, timeout: Duration) -> Result<(Vec<Out>, i32)> {
        let program = match &self.program {
            Ok(p) => p,
            Err(why) => bail!("{why}"),
        };
        let out = self
            .runner
            .run(program, &args, &std::env::temp_dir(), timeout)?;
        let mut lines = Vec::new();
        for l in String::from_utf8_lossy(&out.stdout).lines() {
            if l.trim().is_empty() {
                continue;
            }
            // A line of a kind this build does not know is skipped: a newer helper may add some.
            if let Ok(o) = serde_json::from_str::<Out>(l) {
                lines.push(o);
            }
        }
        Ok((lines, out.status.code().unwrap_or(-1)))
    }

    /// The speech check's state for `locale`: available, or why not. Never an error.
    pub fn speech(&self, locale: &str) -> Feature {
        if let Some((_, f)) = self.caps.lock().unwrap().iter().find(|(l, _)| l == locale) {
            return f.clone();
        }
        let f = match self.run(
            vec!["caps".into(), "--locale".into(), locale.into()],
            Duration::from_secs(20),
        ) {
            Ok((lines, _)) => {
                let mut found = None;
                for o in lines {
                    match o {
                        Out::Caps(c) => found = c.features.into_iter().find(|f| f.name == "asr"),
                        Out::Error { message, .. } => {
                            found = Some(Feature::unavailable("asr", &message))
                        }
                        _ => {}
                    }
                }
                found.unwrap_or_else(|| {
                    Feature::unavailable(
                        "asr",
                        "The quadcam-intel helper reported no speech check.",
                    )
                })
            }
            Err(e) => Feature::unavailable("asr", &format!("{e:#}")),
        };
        self.caps
            .lock()
            .unwrap()
            .push((locale.to_string(), f.clone()));
        f
    }

    /// Video analysis's state: available (macOS 27 or later with a helper built on its SDK),
    /// or why not. Never an error.
    pub fn video_feature(&self) -> Feature {
        if let Some(f) = self.video.lock().unwrap().clone() {
            return f;
        }
        let f = match &self.program {
            Err(why) => Feature::unavailable("video", why),
            Ok(_) => match self.run(vec!["caps".into()], Duration::from_secs(20)) {
                Ok((lines, _)) => lines
                    .into_iter()
                    .find_map(|o| match o {
                        Out::Caps(c) => c.features.into_iter().find(|f| f.name == "video"),
                        Out::Error { message, .. } => Some(Feature::unavailable("video", &message)),
                        _ => None,
                    })
                    .unwrap_or_else(|| {
                        Feature::unavailable(
                            "video",
                            "This quadcam-intel helper has no video analysis.",
                        )
                    }),
                Err(e) => Feature::unavailable("video", &format!("{e:#}")),
            },
        };
        *self.video.lock().unwrap() = Some(f.clone());
        f
    }

    /// Analyses one video file: its highlights and their levels, its key frame, or both. A
    /// missing helper or a macOS older than 27 gives `unavailable`; a file the analyzer could
    /// not read has its own `error`. Errors only when the helper failed to run. `seconds` is
    /// the clip's length; the timeout grows with it (the analyzer runs about 5x real time).
    pub fn video(
        &self,
        file: &Path,
        highlights: bool,
        keyframe: bool,
        seconds: f64,
    ) -> Result<VideoAnswer> {
        let mut a = VideoAnswer::default();
        if let Err(why) = &self.program {
            a.unavailable = Some(why.clone());
            return Ok(a);
        }
        let f = self.video_feature();
        if !f.available {
            a.unavailable = f.reason.or(Some("Video analysis is unavailable.".into()));
            return Ok(a);
        }
        let mut args: Vec<OsString> = vec!["video".into(), file.as_os_str().to_owned()];
        if highlights {
            args.push("--highlights".into());
        }
        if keyframe {
            args.push("--keyframe".into());
        }
        let timeout = Duration::from_secs(120 + seconds.max(0.0) as u64);
        let (lines, code) = self.run(args, timeout)?;
        for o in lines {
            match o {
                Out::Video(w) => a.watched = Some(w),
                Out::Error { code: c, message } => {
                    if code == UNAVAILABLE || c == "unavailable" {
                        a.unavailable = Some(message);
                        return Ok(a);
                    }
                    bail!("quadcam-intel: {message}");
                }
                _ => {}
            }
        }
        if a.watched.is_none() {
            if code == UNAVAILABLE {
                a.unavailable =
                    Some("The quadcam-intel helper says video analysis is unavailable.".into());
            } else {
                bail!(
                    "quadcam-intel reported nothing for {} (exit {code})",
                    file.display()
                );
            }
        }
        Ok(a)
    }

    /// Transcribes each file in `locale`. A helper that is missing, a macOS without the
    /// feature, or a locale without its assets gives `unavailable`; a file the recognizer
    /// could not read has its own `error`. Errors only when the helper failed to run.
    pub fn transcribe(&self, files: &[PathBuf], locale: &str) -> Result<Transcripts> {
        let mut t = Transcripts::default();
        if let Err(why) = &self.program {
            t.unavailable = Some(why.clone());
            return Ok(t);
        }
        for chunk in files.chunks(CHUNK) {
            let mut args: Vec<OsString> = vec!["asr".into(), "--locale".into(), locale.into()];
            args.extend(chunk.iter().map(|f| f.as_os_str().to_owned()));
            let timeout = Duration::from_secs(30 + 5 * chunk.len() as u64);
            let (lines, code) = self.run(args, timeout)?;
            for o in lines {
                match o {
                    Out::Asr(h) => t.heard.push(h),
                    Out::Error { code: c, message } => {
                        if code == UNAVAILABLE || c == "unavailable" {
                            t.unavailable = Some(message);
                            return Ok(t);
                        }
                        bail!("quadcam-intel: {message}");
                    }
                    _ => {}
                }
            }
            if code == UNAVAILABLE {
                t.unavailable =
                    Some("The quadcam-intel helper says the speech check is unavailable.".into());
                return Ok(t);
            }
        }
        Ok(t)
    }

    /// Installs the speech assets for `locale` through macOS (Apple's download). Returns
    /// `installed` or `already`.
    pub fn install_speech(&self, locale: &str) -> Result<String> {
        let (lines, _) = self
            .run(
                vec!["install".into(), "--locale".into(), locale.into()],
                Duration::from_secs(600),
            )
            .context("installing the speech assets")?;
        self.caps.lock().unwrap().clear();
        for o in lines {
            match o {
                Out::Install { status } => return Ok(status),
                Out::Error { message, .. } => bail!("{message}"),
                _ => {}
            }
        }
        bail!("The quadcam-intel helper did not report the install.")
    }
}

/// The helper beside the running binary, or one folder up (a test binary in `deps/`).
fn locate() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe = exe.canonicalize().unwrap_or(exe);
    let dir = exe.parent()?;
    let found = [Some(dir), dir.parent()]
        .into_iter()
        .flatten()
        .map(|d| d.join(HELPER))
        .find(|p| is_executable(p));
    found
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// A speech locale for a pack language: `en` reads as `en-US`; any other is passed on, and
/// the helper finds its nearest supported locale.
pub fn locale_of(lang: &str) -> String {
    match lang.trim() {
        "" | "en" => "en-US".into(),
        l => l.into(),
    }
}

/// A helper for tests: it answers `caps`, `asr`, `video` and `install` without running
/// anything.
pub mod fake {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{ExitStatus, Output};

    /// What the fake hears in a file, by its path; None makes the file unreadable.
    pub type Hear = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;
    /// What the fake's video analysis finds in a file, by its path; None makes the file
    /// unreadable.
    pub type See = Box<dyn Fn(&str) -> Option<Watched> + Send + Sync>;

    /// The fake's video answer for every file: highlights at 2-6 s and 20-30 s on a level
    /// curve that peaks in the first, and the key frame at 3 s.
    pub fn seen(file: &str) -> Watched {
        Watched {
            file: file.into(),
            highlights: Some(vec![
                Range {
                    start: 2.0,
                    end: 6.0,
                },
                Range {
                    start: 20.0,
                    end: 30.0,
                },
            ]),
            levels: (0..80)
                .map(|i| Level {
                    start: i as f64 * 0.5,
                    end: i as f64 * 0.5 + 0.5,
                    level: if (4..12).contains(&i) { 6.0 } else { 2.0 },
                })
                .collect(),
            keyframe: Some(3.0),
            ..Default::default()
        }
    }

    pub struct FakeHelper {
        /// The `caps` line.
        pub caps: String,
        pub hear: Hear,
        pub see: See,
        /// The exit code of every run.
        pub code: i32,
        /// Every run's arguments.
        pub calls: Mutex<Vec<Vec<String>>>,
    }

    impl FakeHelper {
        /// A helper whose speech check is available and hears what `hear` says.
        pub fn new(hear: Hear) -> Self {
            FakeHelper {
                caps: r#"{"kind":"caps","helper":1,"os":"27.0","features":[{"name":"asr","available":true,"min_os":"26.0","locale":"en-US","assets":"installed"},{"name":"video","available":true,"min_os":"27.0"}]}"#.into(),
                hear,
                see: Box::new(|f| Some(seen(f))),
                code: 0,
                calls: Mutex::new(Vec::new()),
            }
        }

        /// The files `asr` runs were asked about.
        pub fn heard_files(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c[0] == "asr")
                .flat_map(|c| c[3..].to_vec())
                .collect()
        }

        /// The files `video` runs were asked about.
        pub fn seen_files(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c[0] == "video")
                .map(|c| c[1].clone())
                .collect()
        }
    }

    impl Runner for FakeHelper {
        fn run(
            &self,
            _program: &Path,
            args: &[OsString],
            _cwd: &Path,
            _timeout: Duration,
        ) -> Result<Output> {
            let args: Vec<String> = args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            self.calls.lock().unwrap().push(args.clone());
            let mut out = String::new();
            match args[0].as_str() {
                "caps" => out = self.caps.clone(),
                "asr" => {
                    for f in &args[3..] {
                        let line = match (self.hear)(f) {
                            Some(t) => {
                                serde_json::json!({"kind": "asr", "file": f, "text": t, "confidence": 0.9, "words": []})
                            }
                            None => {
                                serde_json::json!({"kind": "asr", "file": f, "error": "unreadable"})
                            }
                        };
                        out.push_str(&line.to_string());
                        out.push('\n');
                    }
                }
                "video" => {
                    let w = (self.see)(&args[1]).unwrap_or_else(|| Watched {
                        file: args[1].clone(),
                        error: Some("unreadable".into()),
                        ..Default::default()
                    });
                    let mut v = serde_json::to_value(&w).unwrap();
                    v["kind"] = "video".into();
                    out = v.to_string();
                }
                _ => out = r#"{"kind":"install","locale":"en-US","status":"already"}"#.into(),
            }
            Ok(Output {
                status: ExitStatus::from_raw(self.code << 8),
                stdout: out.into_bytes(),
                stderr: Vec::new(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeHelper;
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{ExitStatus, Output};

    fn fake(heard: &[(&str, &str)]) -> FakeHelper {
        let heard: Vec<(String, String)> = heard
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        FakeHelper::new(Box::new(move |f| {
            let name = Path::new(f).file_name()?.to_string_lossy().into_owned();
            heard
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, t)| t.clone())
        }))
    }

    #[test]
    fn a_missing_helper_is_unavailable_not_an_error() {
        let i = Intel::unavailable("no helper");
        let f = i.speech("en-US");
        assert!(!f.available);
        assert_eq!(f.reason.as_deref(), Some("no helper"));
        let t = i.transcribe(&[PathBuf::from("/x.wav")], "en-US").unwrap();
        assert!(t.heard.is_empty());
        assert_eq!(t.unavailable.as_deref(), Some("no helper"));
    }

    #[test]
    fn a_helper_runs_and_its_lines_are_read() {
        let fake = Arc::new(fake(&[("armed.wav", "Armed.")]));
        let i = Intel::with("/helper".into(), fake.clone());
        assert!(i.speech("en-US").available);
        i.speech("en-US");
        let t = i
            .transcribe(&["/p/armed.wav".into(), "/p/gone.wav".into()], "en-US")
            .unwrap();
        assert_eq!(t.unavailable, None);
        assert_eq!(t.heard[0].text.as_deref(), Some("Armed."));
        assert_eq!(t.heard[1].error.as_deref(), Some("unreadable"));
        let calls = fake.calls.lock().unwrap();
        // caps asked once, then cached.
        assert_eq!(calls.iter().filter(|c| c[0] == "caps").count(), 1);
        assert_eq!(
            calls[1],
            ["asr", "--locale", "en-US", "/p/armed.wav", "/p/gone.wav"]
        );
    }

    #[test]
    fn an_old_macos_or_missing_assets_read_as_unavailable() {
        let mut f = fake(&[]);
        f.code = 3;
        f.caps = r#"{"kind":"caps","helper":1,"os":"15.6","features":[{"name":"asr","available":false,"min_os":"26.0","reason":"Speech recognition on this Mac needs macOS 26 or later."}]}"#.into();
        struct Old(FakeHelper);
        impl Runner for Old {
            fn run(
                &self,
                p: &Path,
                a: &[OsString],
                c: &Path,
                t: Duration,
            ) -> Result<std::process::Output> {
                if a[0] == "asr" {
                    return Ok(Output {
                        status: ExitStatus::from_raw(3 << 8),
                        stdout: br#"{"kind":"error","code":"unavailable","message":"Speech recognition on this Mac needs macOS 26 or later."}"#.to_vec(),
                        stderr: Vec::new(),
                    });
                }
                self.0.run(p, a, c, t)
            }
        }
        let i = Intel::with("/helper".into(), Arc::new(Old(f)));
        let s = i.speech("en-US");
        assert!(!s.available);
        assert!(s.reason.unwrap().contains("macOS 26"));
        let t = i.transcribe(&["/a.wav".into()], "en-US").unwrap();
        assert!(t.unavailable.unwrap().contains("macOS 26"));
    }

    #[test]
    fn a_cargo_process_gets_no_helper_unless_asked() {
        if std::env::var("QUADCAM_INTEL").as_deref() == Ok("real") {
            return;
        }
        let f = Intel::system().speech("en-US");
        assert!(!f.available);
        assert!(f.reason.as_deref().unwrap_or("").contains("cargo"), "{f:?}");
    }

    /// The real helper on a real spoken file. Runs only where the helper was built and the
    /// speech check is available (macOS 26 or later with the en-US assets); skips elsewhere.
    #[test]
    fn the_real_helper_hears_a_spoken_line() {
        let Some(helper) = locate() else {
            eprintln!("skipped: no quadcam-intel beside the test binary");
            return;
        };
        let i = Intel::with(helper, Arc::new(Process));
        let f = i.speech("en-US");
        if !f.available {
            eprintln!("skipped: {}", f.reason.unwrap_or_default());
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("line.aiff");
        let said = std::process::Command::new("/usr/bin/say")
            .arg("-o")
            .arg(&file)
            .arg("Battery low")
            .status()
            .unwrap();
        assert!(said.success());
        let t = i.transcribe(&[file], "en-US").unwrap();
        assert_eq!(t.unavailable, None);
        let text = t.heard[0].text.clone().unwrap_or_default();
        assert!(text.to_lowercase().contains("battery low"), "{text:?}");
    }

    #[test]
    fn video_analysis_runs_the_helper_once_per_file() {
        let fake = Arc::new(fake(&[]));
        let i = Intel::with("/helper".into(), fake.clone());
        assert!(i.video_feature().available);
        let a = i.video(Path::new("/c/a.mp4"), true, true, 60.0).unwrap();
        assert_eq!(a.unavailable, None);
        let w = a.watched.unwrap();
        assert_eq!(w.keyframe, Some(3.0));
        assert_eq!(w.highlights.unwrap().len(), 2);
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.iter().filter(|c| c[0] == "caps").count(), 1);
        assert_eq!(
            calls[1],
            ["video", "/c/a.mp4", "--highlights", "--keyframe"]
        );
    }

    #[test]
    fn video_analysis_is_unavailable_before_macos_27() {
        let mut f = fake(&[]);
        f.caps = r#"{"kind":"caps","helper":1,"os":"26.1","features":[{"name":"video","available":false,"min_os":"27.0","reason":"Video analysis on this Mac needs macOS 27 or later."}]}"#.into();
        let f = Arc::new(f);
        let i = Intel::with("/helper".into(), f.clone());
        let a = i.video(Path::new("/a.mp4"), true, true, 10.0).unwrap();
        assert!(a.unavailable.unwrap().contains("macOS 27"));
        assert!(f.seen_files().is_empty(), "no video run on an older macOS");
        // An older helper without the feature at all.
        let mut f = fake(&[]);
        f.caps = r#"{"kind":"caps","helper":1,"os":"27.0","features":[]}"#.into();
        let i = Intel::with("/helper".into(), Arc::new(f));
        assert!(!i.video_feature().available);
        let none = Intel::unavailable("no helper");
        assert_eq!(
            none.video(Path::new("/a.mp4"), true, true, 1.0)
                .unwrap()
                .unavailable
                .as_deref(),
            Some("no helper")
        );
    }

    /// The real helper on a synthetic clip. Runs only with `QUADCAM_INTEL=real` where the
    /// helper was built and video analysis is available (macOS 27 or later); skips elsewhere.
    #[test]
    fn the_real_helper_analyses_a_clip() {
        if std::env::var("QUADCAM_INTEL").as_deref() != Ok("real") {
            eprintln!("skipped: QUADCAM_INTEL=real runs the real helper");
            return;
        }
        let Some(helper) = locate() else {
            eprintln!("skipped: no quadcam-intel beside the test binary");
            return;
        };
        let i = Intel::with(helper, Arc::new(Process));
        let f = i.video_feature();
        if !f.available {
            eprintln!("skipped: {}", f.reason.unwrap_or_default());
            return;
        }
        let Ok(tools) = crate::media::find_tools() else {
            eprintln!("skipped: no ffmpeg");
            return;
        };
        let d = tempfile::tempdir().unwrap();
        let clip = d.path().join("clip.mp4");
        let made = std::process::Command::new(&tools.ffmpeg)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=6:size=320x240:rate=25",
            ])
            .args(["-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status()
            .unwrap();
        assert!(made.success());
        let a = i.video(&clip, true, true, 6.0).unwrap();
        assert_eq!(a.unavailable, None);
        let w = a.watched.unwrap();
        assert_eq!(w.error, None, "{w:?}");
        let k = w.keyframe.expect("a key frame");
        assert!((0.0..=6.0).contains(&k), "{k}");
        assert!(w.highlights.is_some(), "{w:?}");
        assert!(!w.levels.is_empty(), "{w:?}");
    }

    #[test]
    fn languages_map_to_speech_locales() {
        assert_eq!(locale_of("en"), "en-US");
        assert_eq!(locale_of(""), "en-US");
        assert_eq!(locale_of("de"), "de");
    }
}
