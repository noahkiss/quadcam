//! The helper's `suggest`: clip names, keywords and notes, and a flying day's summary, from
//! Apple's on-device language model (Foundation Models: text on macOS 26, frames too on 27).
//! QuadCam owns the facts it sends; the helper owns the instructions and the answers' shapes.
//! A Mac without the model (an older macOS, Apple Intelligence off) gives `unavailable`,
//! never an error.

use super::{Feature, Intel, UNAVAILABLE};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// One clip as the model sees it.
#[derive(Debug, Clone, Serialize)]
pub struct ClipAsk {
    pub id: usize,
    /// One fact per line: date and time of day, place, aircraft, length, moments, flight.
    pub facts: String,
    /// Still frames of the clip; the helper uses them where the model takes images.
    pub frames: Vec<PathBuf>,
}

/// A request for names: the clips of one outing, and names the person gave before.
#[derive(Debug, Clone, Default, Serialize)]
pub struct NamesAsk {
    pub examples: Vec<String>,
    pub clips: Vec<ClipAsk>,
}

/// What the model suggested for one clip.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClipIdea {
    pub id: usize,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// The frames the prompt held.
    #[serde(default)]
    pub images: usize,
    #[serde(default)]
    pub ms: u64,
    /// Why this clip has no suggestion (the model declined, a timeout).
    #[serde(default)]
    pub error: Option<String>,
}

/// The answer to a `NamesAsk`.
#[derive(Debug, Clone, Default)]
pub struct Ideas {
    pub clips: Vec<ClipIdea>,
    /// One name for the whole outing, when it had more than one clip.
    pub group: Option<String>,
    pub unavailable: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Line {
    Suggestion(ClipIdea),
    Group { name: String },
    Summary { text: String },
    Error { code: String, message: String },
}

/// Seconds a names run may take: the model loads once, then a clip with frames takes about
/// 7 s on an M1 Pro.
fn names_timeout(clips: usize) -> Duration {
    Duration::from_secs(60 + 30 * clips as u64)
}

const SUMMARY_TIMEOUT: Duration = Duration::from_secs(90);

/// Clips per helper run: a run's timeout grows with its clips.
const CHUNK: usize = 12;

impl Intel {
    /// On-device suggestions' state: available, whether frames are used, or why not.
    /// Never an error; asked once per process.
    pub fn suggestions(&self) -> Feature {
        if let Some(f) = self.names.lock().unwrap().clone() {
            return f;
        }
        let f = match self.run(vec!["caps".into()], Duration::from_secs(20)) {
            Ok((lines, _)) => {
                let mut found = None;
                for o in lines {
                    match o {
                        super::Out::Caps(c) => {
                            found = c.features.into_iter().find(|f| f.name == "suggest")
                        }
                        super::Out::Error { message, .. } => {
                            found = Some(Feature::unavailable("suggest", &message))
                        }
                        _ => {}
                    }
                }
                found.unwrap_or_else(|| {
                    Feature::unavailable(
                        "suggest",
                        "This quadcam-intel helper has no on-device suggestions.",
                    )
                })
            }
            Err(e) => Feature::unavailable("suggest", &format!("{e:#}")),
        };
        *self.names.lock().unwrap() = Some(f.clone());
        f
    }

    /// Names, keywords and a note per clip, and one name for the outing. A helper that is
    /// missing or a Mac without the model gives `unavailable`. Errors only when the helper
    /// failed to run.
    pub fn suggest_names(&self, ask: &NamesAsk) -> Result<Ideas> {
        let mut out = Ideas::default();
        if let Err(why) = &self.program {
            out.unavailable = Some(why.clone());
            return Ok(out);
        }
        for chunk in ask.clips.chunks(CHUNK) {
            let part = NamesAsk {
                examples: ask.examples.clone(),
                clips: chunk.to_vec(),
            };
            let body = serde_json::json!({"task": "names", "examples": part.examples, "clips": part.clips});
            let (lines, code) = self.run_request(&body, names_timeout(chunk.len()))?;
            for l in lines {
                match l {
                    Line::Suggestion(i) => out.clips.push(i),
                    Line::Group { name } => out.group = Some(name),
                    Line::Error { code: c, message } => {
                        if code == UNAVAILABLE || c == "unavailable" {
                            out.unavailable = Some(message);
                            return Ok(out);
                        }
                        bail!("quadcam-intel: {message}");
                    }
                    Line::Summary { .. } => {}
                }
            }
            if code == UNAVAILABLE {
                out.unavailable = Some(
                    "The quadcam-intel helper says on-device suggestions are unavailable.".into(),
                );
                return Ok(out);
            }
        }
        // A chunked run names each part; a group name needs the whole outing in one run.
        if ask.clips.len() > CHUNK {
            out.group = None;
        }
        Ok(out)
    }

    /// A short paragraph about a flying day from its facts. Ok(Err(why)) when the model is
    /// unavailable here.
    pub fn summarize(&self, facts: &str) -> Result<std::result::Result<String, String>> {
        if let Err(why) = &self.program {
            return Ok(Err(why.clone()));
        }
        let body = serde_json::json!({"task": "summary", "facts": facts});
        let (lines, code) = self.run_request(&body, SUMMARY_TIMEOUT)?;
        for l in lines {
            match l {
                Line::Summary { text } => return Ok(Ok(text)),
                Line::Error { code: c, message } => {
                    if code == UNAVAILABLE || c == "unavailable" {
                        return Ok(Err(message));
                    }
                    bail!("quadcam-intel: {message}");
                }
                _ => {}
            }
        }
        if code == UNAVAILABLE {
            return Ok(Err(
                "The quadcam-intel helper says on-device suggestions are unavailable.".into(),
            ));
        }
        bail!("The quadcam-intel helper wrote no summary.")
    }

    /// Runs `suggest` on a request written to a temporary file (the runner gives the
    /// helper no stdin), and reads its lines.
    fn run_request(&self, body: &serde_json::Value, timeout: Duration) -> Result<(Vec<Line>, i32)> {
        static N: AtomicU64 = AtomicU64::new(0);
        let file = std::env::temp_dir().join(format!(
            "quadcam-suggest-{}-{}.json",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::write(&file, serde_json::to_vec(body)?)?;
        let args: Vec<OsString> = vec!["suggest".into(), file.clone().into_os_string()];
        let ran = self.run_raw(args, timeout);
        let _ = std::fs::remove_file(&file);
        let (stdout, code) = ran?;
        let lines = stdout
            .lines()
            .filter(|l| !l.trim().is_empty())
            // A line of a kind this build does not know is skipped: a newer helper may add some.
            .filter_map(|l| serde_json::from_str::<Line>(l).ok())
            .collect();
        Ok((lines, code))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intel::fake::FakeHelper;
    use crate::modules::run::Process;
    use std::path::Path;
    use std::sync::Arc;

    #[test]
    fn suggestions_read_their_feature_and_ideas() {
        let fake = Arc::new(FakeHelper::new(Box::new(|_| None)));
        let i = Intel::with("/helper".into(), fake.clone());
        let f = i.suggestions();
        assert!(f.available);
        assert_eq!(f.images, Some(true));
        let ideas = i
            .suggest_names(&NamesAsk {
                examples: vec!["farm-fun".into()],
                clips: vec![
                    ClipAsk {
                        id: 3,
                        facts: "Place: Field".into(),
                        frames: vec!["/t/3.jpg".into()],
                    },
                    ClipAsk {
                        id: 4,
                        facts: "Place: Field".into(),
                        frames: vec![],
                    },
                ],
            })
            .unwrap();
        assert_eq!(ideas.unavailable, None);
        assert_eq!(ideas.group.as_deref(), Some("fake outing"));
        assert_eq!(ideas.clips[0].name.as_deref(), Some("fake 3"));
        assert_eq!(ideas.clips[0].images, 1);
        let req = &fake.requests.lock().unwrap()[0];
        assert_eq!(req["task"], "names");
        assert_eq!(req["examples"][0], "farm-fun");
        // The request file is gone after the run.
        let calls = fake.calls.lock().unwrap();
        let file = &calls.iter().find(|c| c[0] == "suggest").unwrap()[1];
        assert!(!Path::new(file).exists());
        // caps asked once.
        drop(calls);
        i.suggestions();
        assert_eq!(
            fake.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c[0] == "caps")
                .count(),
            1
        );
    }

    #[test]
    fn a_missing_helper_has_no_suggestions_and_no_summary() {
        let i = Intel::unavailable("no helper");
        assert!(!i.suggestions().available);
        let ideas = i.suggest_names(&NamesAsk::default()).unwrap();
        assert_eq!(ideas.unavailable.as_deref(), Some("no helper"));
        assert_eq!(i.summarize("Day: x").unwrap(), Err("no helper".into()));
    }

    /// Apple's model through the real helper. Runs only with `QUADCAM_INTEL=real`, a built
    /// helper and the model available (macOS 26 or later, Apple Intelligence on); a frame
    /// joins the prompt on macOS 27.
    #[test]
    fn the_real_helper_names_a_clip() {
        if std::env::var("QUADCAM_INTEL").as_deref() != Ok("real") {
            eprintln!("skipped: set QUADCAM_INTEL=real");
            return;
        }
        let Some(helper) = super::super::locate() else {
            eprintln!("skipped: no quadcam-intel beside the test binary");
            return;
        };
        let i = Intel::with(helper, Arc::new(Process));
        let f = i.suggestions();
        if !f.available {
            eprintln!("skipped: {}", f.reason.unwrap_or_default());
            return;
        }
        let ideas = i
            .suggest_names(&NamesAsk {
                examples: vec!["sunset field rips pack 1".into()],
                clips: vec![
                    ClipAsk {
                        id: 0,
                        facts: "Date: 2026-07-04, 19:40 (evening)\nPlace: Field\nAircraft: Whoop\nLength: 2:10\nMoments: flip at 0:40".into(),
                        frames: vec![],
                    },
                    ClipAsk {
                        id: 1,
                        facts: "Date: 2026-07-04, 19:46 (evening)\nPlace: Field\nAircraft: Whoop\nLength: 1:55".into(),
                        frames: vec![],
                    },
                ],
            })
            .unwrap();
        assert_eq!(ideas.unavailable, None);
        assert_eq!(ideas.clips.len(), 2);
        for c in &ideas.clips {
            let n = c.name.clone().unwrap_or_default();
            assert!(!n.is_empty() && n.len() < 80, "{c:?}");
        }
        let text = i
            .summarize("Day: Saturday 2026-07-04\nPlaces: Field\nClips: 2")
            .unwrap();
        assert!(text.is_ok_and(|t| !t.is_empty()));
    }
}
