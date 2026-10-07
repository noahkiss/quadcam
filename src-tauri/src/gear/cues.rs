//! Cues: short spoken lines, sounds and macOS notifications for bench states, so the
//! person can work with their hands full:
//!
//! - `safe_to_unplug`: a bench step finished and QuadCam released the device (the FC's
//!   port closed, the card unmounted);
//! - `still_inserted`: a card unmounted but still in; after `still_inserted_every_s` and
//!   then every as long again until the card is pulled (`Reminders`; 0 turns it off);
//! - `step_failed`: a backup, an import or an apply did not finish.
//!
//! The `gearCues` setting (`CueSettings`) turns each cue and each channel (speech, sound,
//! notification) on or off. `Cues::fire` checks it and hands the cue to a `CueSink`.
//! `SystemCues` speaks with `/usr/bin/say`, plays a system sound with `/usr/bin/afplay` and
//! posts a notification with `/usr/bin/osascript`, each a child process with an argv list,
//! never a shell. A process started by cargo gets the silent `RecordedCues` from
//! `system()` unless `QUADCAM_CUES=real`, so tests make no sound.

use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Type)]
#[serde(rename_all = "snake_case")]
pub enum Cue {
    SafeToUnplug,
    StillInserted,
    StepFailed,
}

impl Cue {
    /// The spoken line and the notification text, for a device's name.
    pub fn line(self, device: &str) -> String {
        match self {
            Cue::SafeToUnplug => format!("{device} is safe to unplug."),
            Cue::StillInserted => format!("{device} is still inserted."),
            Cue::StepFailed => format!("{device}: the step failed."),
        }
    }

    /// The system sound (`/System/Library/Sounds/<name>.aiff`).
    pub fn sound(self) -> &'static str {
        match self {
            Cue::SafeToUnplug => "Glass",
            Cue::StillInserted => "Ping",
            Cue::StepFailed => "Basso",
        }
    }
}

/// The `gearCues` setting. Missing fields take their defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(default)]
pub struct CueSettings {
    /// Speak cues with the system voice.
    pub speech: bool,
    /// Play a system sound.
    pub sound: bool,
    /// Post a macOS notification.
    pub notification: bool,
    pub safe_to_unplug: bool,
    pub still_inserted: bool,
    /// Seconds between "still inserted" reminders; 0 for none.
    pub still_inserted_every_s: u32,
    pub step_failed: bool,
    /// A `say` voice name; None for the system voice.
    pub voice: Option<String>,
}

impl Default for CueSettings {
    fn default() -> Self {
        Self {
            speech: true,
            sound: false,
            notification: true,
            safe_to_unplug: true,
            still_inserted: true,
            still_inserted_every_s: 60,
            step_failed: true,
            voice: None,
        }
    }
}

impl CueSettings {
    pub fn wants(&self, cue: Cue) -> bool {
        match cue {
            Cue::SafeToUnplug => self.safe_to_unplug,
            Cue::StillInserted => self.still_inserted,
            Cue::StepFailed => self.step_failed,
        }
    }
}

/// Where cues go.
pub trait CueSink: Send + Sync {
    fn speak(&self, text: &str, voice: Option<&str>);
    fn sound(&self, name: &str);
    fn notify(&self, title: &str, body: &str);
}

/// One call a `RecordedCues` saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Played {
    Speak(String),
    Sound(String),
    Notify(String, String),
}

/// A silent sink that records every cue (tests, and any process started by cargo).
#[derive(Default)]
pub struct RecordedCues {
    pub played: Mutex<Vec<Played>>,
}

impl CueSink for RecordedCues {
    fn speak(&self, text: &str, _voice: Option<&str>) {
        self.played.lock().unwrap().push(Played::Speak(text.into()));
    }
    fn sound(&self, name: &str) {
        self.played.lock().unwrap().push(Played::Sound(name.into()));
    }
    fn notify(&self, title: &str, body: &str) {
        self.played
            .lock()
            .unwrap()
            .push(Played::Notify(title.into(), body.into()));
    }
}

/// The real sink: `say`, `afplay`, `osascript`, each started and left to finish.
pub struct SystemCues;

fn spawn(program: &str, args: &[&str]) {
    if let Ok(mut child) = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

impl CueSink for SystemCues {
    fn speak(&self, text: &str, voice: Option<&str>) {
        match voice.filter(|v| !v.trim().is_empty()) {
            Some(v) => spawn("/usr/bin/say", &["-v", v, "--", text]),
            None => spawn("/usr/bin/say", &["--", text]),
        }
    }
    fn sound(&self, name: &str) {
        let path = format!("/System/Library/Sounds/{name}.aiff");
        spawn("/usr/bin/afplay", &[&path]);
    }
    fn notify(&self, title: &str, body: &str) {
        // The texts are arguments of the script, never part of it.
        spawn(
            "/usr/bin/osascript",
            &[
                "-e",
                "on run argv",
                "-e",
                "display notification (item 2 of argv) with title (item 1 of argv)",
                "-e",
                "end run",
                title,
                body,
            ],
        );
    }
}

/// Whether cues reach the speakers: `QUADCAM_CUES=real` forces them on and any other value
/// off; unset, a process started by cargo is silent.
pub fn cues_enabled(setting: Option<&str>, under_cargo: bool) -> bool {
    match setting {
        Some("real") => true,
        Some(_) => false,
        None => !under_cargo,
    }
}

/// The sink this process uses.
pub fn system() -> Arc<dyn CueSink> {
    if cues_enabled(
        std::env::var("QUADCAM_CUES").ok().as_deref(),
        std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
    ) {
        Arc::new(SystemCues)
    } else {
        Arc::new(RecordedCues::default())
    }
}

/// Plays a cue on each channel the settings turn on. Returns false when the cue is off.
pub fn fire(sink: &dyn CueSink, settings: &CueSettings, cue: Cue, device: &str) -> bool {
    if !settings.wants(cue) {
        return false;
    }
    let line = cue.line(device);
    if settings.sound {
        sink.sound(cue.sound());
    }
    if settings.speech {
        sink.speak(&line, settings.voice.as_deref());
    }
    if settings.notification {
        sink.notify("QuadCam", &line);
    }
    true
}

/// The "still inserted" reminder per device: due `every` after the unmount, then every
/// `every` again, until the device is gone. A zero `every` is never due.
#[derive(Debug, Default)]
pub struct Reminders {
    last: HashMap<String, Instant>,
}

impl Reminders {
    /// The keys (device links) due now, among the ones still unmounted and present.
    /// Forgets keys no longer present.
    pub fn due(&mut self, present: &[String], every: Duration, now: Instant) -> Vec<String> {
        self.last.retain(|k, _| present.contains(k));
        let mut out = Vec::new();
        for k in present {
            let since = *self.last.entry(k.clone()).or_insert(now);
            if !every.is_zero() && now.duration_since(since) >= every {
                self.last.insert(k.clone(), now);
                out.push(k.clone());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fire_follows_the_settings() {
        let sink = RecordedCues::default();
        let s = CueSettings::default();
        assert!(fire(&sink, &s, Cue::SafeToUnplug, "Bench radio"));
        assert_eq!(
            *sink.played.lock().unwrap(),
            [
                Played::Speak("Bench radio is safe to unplug.".into()),
                Played::Notify("QuadCam".into(), "Bench radio is safe to unplug.".into())
            ]
        );
        let off = CueSettings {
            step_failed: false,
            ..Default::default()
        };
        assert!(!fire(&sink, &off, Cue::StepFailed, "x"));
        let sound_only = CueSettings {
            speech: false,
            notification: false,
            sound: true,
            ..Default::default()
        };
        sink.played.lock().unwrap().clear();
        fire(&sink, &sound_only, Cue::StepFailed, "x");
        assert_eq!(
            *sink.played.lock().unwrap(),
            [Played::Sound("Basso".into())]
        );
    }

    #[test]
    fn settings_fill_missing_fields() {
        let s: CueSettings = serde_json::from_str(r#"{"speech": false}"#).unwrap();
        assert!(!s.speech);
        assert!(s.notification);
        assert_eq!(s.still_inserted_every_s, 60);
    }

    #[test]
    fn reminders_repeat_until_removed() {
        let mut r = Reminders::default();
        let t0 = Instant::now();
        let every = Duration::from_secs(60);
        let k = vec!["card".to_string()];
        let s = |n| t0 + Duration::from_secs(n);
        assert!(
            r.due(&k, every, t0).is_empty(),
            "the unmount itself says safe to unplug"
        );
        assert!(r.due(&k, every, s(30)).is_empty());
        assert_eq!(r.due(&k, every, s(61)), k);
        assert!(r.due(&k, every, s(90)).is_empty());
        assert_eq!(r.due(&k, every, s(122)), k);
        assert!(r.due(&[], every, s(200)).is_empty(), "pulled");
        assert!(
            r.due(&k, every, s(201)).is_empty(),
            "a new unmount starts again"
        );
        assert_eq!(r.due(&k, every, s(262)), k);
        let mut off = Reminders::default();
        assert!(off.due(&k, Duration::ZERO, t0).is_empty());
        assert!(off.due(&k, Duration::ZERO, s(500)).is_empty());
    }

    #[test]
    fn silent_under_cargo() {
        assert!(!cues_enabled(None, true));
        assert!(cues_enabled(None, false));
        assert!(!cues_enabled(Some("off"), false));
        assert!(cues_enabled(Some("real"), true));
    }
}
