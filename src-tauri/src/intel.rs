//! The Swift helper `quadcam-intel` (`intel/` in the repo, `docs/development.md`): Apple's
//! on-device frameworks behind a child process that prints one JSON object per line. The app
//! bundles it beside its own binaries (`Contents/MacOS`); a cargo build puts it in
//! `target/<profile>/`. It runs only through `modules::run::Runner`, by absolute path, with
//! a timeout. A missing helper, a macOS older than a feature needs, or a locale without its
//! speech assets makes that feature unavailable, never an error.
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

mod names;
pub use names::{ClipAsk, ClipIdea, Ideas, NamesAsk};

/// The helper's file name, beside the app's binaries.
pub const HELPER: &str = "quadcam-intel";

/// One feature as the helper's `caps` reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct Feature {
    /// `asr`: the speech check of voice lines; `suggest`: on-device clip names and notes.
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
    /// `suggest`: whether the model also looks at frames (macOS 27).
    #[serde(default)]
    pub images: Option<bool>,
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
            images: None,
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
    /// `caps`' on-device suggestions, asked once per process.
    names: Mutex<Option<Feature>>,
}

impl Intel {
    /// The helper beside this process's binary (or one folder up, for a test binary in
    /// `deps/`). None in a process started by cargo unless `QUADCAM_INTEL=real`.
    pub fn system() -> Self {
        let under_cargo = std::env::var_os("CARGO_MANIFEST_DIR").is_some();
        if under_cargo && std::env::var("QUADCAM_INTEL").as_deref() != Ok("real") {
            return Self::unavailable("The quadcam-intel helper is off in a process started by cargo (QUADCAM_INTEL=real turns it on).");
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
            names: Mutex::new(None),
        }
    }

    /// No helper, for this reason.
    pub fn unavailable(reason: &str) -> Self {
        Intel {
            program: Err(reason.into()),
            runner: Arc::new(Process),
            caps: Mutex::new(Vec::new()),
            names: Mutex::new(None),
        }
    }

    /// The helper's stdout and exit code.
    fn run_raw(&self, args: Vec<OsString>, timeout: Duration) -> Result<(String, i32)> {
        let program = match &self.program {
            Ok(p) => p,
            Err(why) => bail!("{why}"),
        };
        let out = self
            .runner
            .run(program, &args, &std::env::temp_dir(), timeout)?;
        Ok((
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.code().unwrap_or(-1),
        ))
    }

    fn run(&self, args: Vec<OsString>, timeout: Duration) -> Result<(Vec<Out>, i32)> {
        let (stdout, code) = self.run_raw(args, timeout)?;
        let mut lines = Vec::new();
        for l in stdout.lines() {
            if l.trim().is_empty() {
                continue;
            }
            // A line of a kind this build does not know is skipped: a newer helper may add some.
            if let Ok(o) = serde_json::from_str::<Out>(l) {
                lines.push(o);
            }
        }
        Ok((lines, code))
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

/// A helper for tests: it answers `caps`, `asr`, `install` and `suggest` without running
/// anything.
pub mod fake {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{ExitStatus, Output};

    /// What the fake hears in a file, by its path; None makes the file unreadable.
    pub type Hear = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

    /// The `suggest` answer (JSON lines) to a request.
    pub type Answer = Box<dyn Fn(&serde_json::Value) -> String + Send + Sync>;

    pub struct FakeHelper {
        /// The `caps` line.
        pub caps: String,
        pub hear: Hear,
        /// The exit code of every run.
        pub code: i32,
        /// Every run's arguments.
        pub calls: Mutex<Vec<Vec<String>>>,
        /// Answers `suggest`; None gives `fake_ideas`.
        pub answer: Option<Answer>,
        /// Every `suggest` request.
        pub requests: Mutex<Vec<serde_json::Value>>,
    }

    /// The default `suggest` answer: `fake <id>` per clip, keywords, a note, `fake outing`
    /// for more than one clip, and a summary that repeats the first fact.
    pub fn fake_ideas(req: &serde_json::Value) -> String {
        let mut out = String::new();
        if req["task"] == "summary" {
            let first = req["facts"]
                .as_str()
                .unwrap_or("")
                .lines()
                .next()
                .unwrap_or("");
            out.push_str(&serde_json::json!({"kind": "summary", "text": format!("A fake summary. {first}"), "ms": 5}).to_string());
            return out;
        }
        let clips = req["clips"].as_array().cloned().unwrap_or_default();
        for c in &clips {
            let id = c["id"].as_u64().unwrap_or(0);
            let images = c["frames"].as_array().map_or(0, |f| f.len().min(2));
            let line = serde_json::json!({"kind": "suggestion", "id": id, "name": format!("fake {id}"), "keywords": ["fake", "whoop"], "note": format!("Fake note for clip {id}."), "images": images, "ms": 5});
            out.push_str(&line.to_string());
            out.push('\n');
        }
        if clips.len() > 1 {
            out.push_str(r#"{"kind":"group","name":"fake outing","ms":5}"#);
        }
        out
    }

    impl FakeHelper {
        /// A helper whose speech check is available and hears what `hear` says.
        pub fn new(hear: Hear) -> Self {
            FakeHelper {
                caps: r#"{"kind":"caps","helper":1,"os":"27.0","features":[{"name":"asr","available":true,"min_os":"26.0","locale":"en-US","assets":"installed"},{"name":"suggest","available":true,"min_os":"26.0","images":true}]}"#.into(),
                hear,
                code: 0,
                calls: Mutex::new(Vec::new()),
                answer: None,
                requests: Mutex::new(Vec::new()),
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
                "suggest" => {
                    let req: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[1])?)?;
                    out = match &self.answer {
                        Some(a) => a(&req),
                        None => fake_ideas(&req),
                    };
                    self.requests.lock().unwrap().push(req);
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
    fn languages_map_to_speech_locales() {
        assert_eq!(locale_of("en"), "en-US");
        assert_eq!(locale_of(""), "en-US");
        assert_eq!(locale_of("de"), "de");
    }
}
