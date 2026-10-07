//! Cues: short spoken lines, sounds and macOS notifications, quiet by design. QuadCam mounts,
//! unmounts and opens ports all the time (each job mounts, works and releases), so:
//!
//! 1. Cues mark outcomes a person cares about, never internal transitions. QuadCam's own
//!    mounts, unmounts and port opens are app-initiated (`Core::gear_hold`) and never cue.
//! 2. One cue per job, at its end: "<device> done, safe to unplug." or "<step> failed on
//!    <device>." A batch or an automation run gets one cue for the whole run (`batch_cue`).
//! 3. The same cue for the same device within `debounce_s` is dropped (`Gate`). Cues play
//!    one at a time from a queue (`CueService`), never over each other.
//! 4. The "still inserted" reminder starts only after a job's "done", waits
//!    `reminder_grace_s`, repeats every `still_inserted_every_s` at most `reminder_max`
//!    times, and stops when the device is removed or the person dismisses it (`Reminders`).
//! 5. Each cue has a toggle, `mute` silences all, and `quiet_hours` silences speech and
//!    sound. Notifications follow macOS Focus on their own: macOS holds them back. An app
//!    cannot read the Focus state without Full Disk Access, so speech and sound use quiet
//!    hours instead.
//!
//! `SystemCues` speaks with `/usr/bin/say`, plays a system sound with `/usr/bin/afplay` and
//! posts a notification with `/usr/bin/osascript`, each a child process with an argv list,
//! never a shell, run to the end before the next cue. A process started by cargo gets the
//! silent `RecordedCues` from `system()` unless `QUADCAM_CUES=real`, so tests make no sound.

use chrono::NaiveTime;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Type)]
#[serde(rename_all = "snake_case")]
pub enum Cue {
    /// A job finished and the device is released.
    SafeToUnplug,
    /// The reminder: the device is still in after "done".
    StillInserted,
    /// A job's step failed.
    StepFailed,
}

/// One cue to play: what, for which device (or "3 devices"), and the failed step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CueEvent {
    pub cue: Cue,
    pub device: String,
    pub step: Option<String>,
}

impl CueEvent {
    pub fn new(cue: Cue, device: impl Into<String>) -> Self {
        Self {
            cue,
            device: device.into(),
            step: None,
        }
    }

    pub fn failed(step: impl Into<String>, device: impl Into<String>) -> Self {
        Self {
            cue: Cue::StepFailed,
            device: device.into(),
            step: Some(step.into()),
        }
    }

    /// The spoken line and the notification text.
    pub fn line(&self) -> String {
        match self.cue {
            Cue::SafeToUnplug => format!("{} done, safe to unplug.", self.device),
            Cue::StillInserted => format!("{} is still inserted.", self.device),
            Cue::StepFailed => format!(
                "{} failed on {}.",
                self.step.as_deref().unwrap_or("A step"),
                self.device
            ),
        }
    }
}

impl Cue {
    /// The system sound (`/System/Library/Sounds/<name>.aiff`).
    pub fn sound(self) -> &'static str {
        match self {
            Cue::SafeToUnplug => "Glass",
            Cue::StillInserted => "Ping",
            Cue::StepFailed => "Basso",
        }
    }
}

/// Speech and sound stay silent from `start` to `end` (local time, `HH:MM`; may cross
/// midnight).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct QuietHours {
    pub start: String,
    pub end: String,
}

impl QuietHours {
    pub fn contains(&self, t: NaiveTime) -> bool {
        let p = |s: &str| NaiveTime::parse_from_str(s.trim(), "%H:%M").ok();
        let (Some(a), Some(b)) = (p(&self.start), p(&self.end)) else {
            return false;
        };
        if a <= b {
            a <= t && t < b
        } else {
            t >= a || t < b
        }
    }
}

/// The `gearCues` setting. Missing fields take their defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(default)]
pub struct CueSettings {
    /// Silences every cue.
    pub mute: bool,
    /// Speak cues with the system voice.
    pub speech: bool,
    /// Play a system sound.
    pub sound: bool,
    /// Post a macOS notification.
    pub notification: bool,
    /// "<device> done, safe to unplug."
    pub safe_to_unplug: bool,
    /// The reminder after "done" while the device is still in.
    pub still_inserted: bool,
    /// "<step> failed on <device>."
    pub step_failed: bool,
    /// The same cue for the same device within this many seconds is dropped.
    pub debounce_s: u32,
    /// Seconds after "done" before the first reminder.
    pub reminder_grace_s: u32,
    /// Seconds between reminders.
    pub still_inserted_every_s: u32,
    /// Reminders at most, per "done"; 0 for none.
    pub reminder_max: u32,
    /// Speech and sound stay silent in these hours.
    pub quiet_hours: Option<QuietHours>,
    /// A `say` voice name; None for the system voice.
    pub voice: Option<String>,
}

impl Default for CueSettings {
    fn default() -> Self {
        Self {
            mute: false,
            speech: true,
            sound: false,
            notification: true,
            safe_to_unplug: true,
            still_inserted: true,
            step_failed: true,
            debounce_s: 30,
            reminder_grace_s: 60,
            still_inserted_every_s: 300,
            reminder_max: 3,
            quiet_hours: None,
            voice: None,
        }
    }
}

impl CueSettings {
    pub fn wants(&self, cue: Cue) -> bool {
        !self.mute
            && match cue {
                Cue::SafeToUnplug => self.safe_to_unplug,
                Cue::StillInserted => self.still_inserted,
                Cue::StepFailed => self.step_failed,
            }
    }
}

/// The channels a cue plays on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Channels {
    pub speech: bool,
    pub sound: bool,
    pub notification: bool,
}

/// Decides whether a cue plays, and on which channels: the toggles, mute, quiet hours and
/// the debounce. Pure, on a given clock.
#[derive(Debug, Default)]
pub struct Gate {
    last: HashMap<(Cue, String), Instant>,
}

impl Gate {
    pub fn admit(
        &mut self,
        s: &CueSettings,
        e: &CueEvent,
        now: Instant,
        local: NaiveTime,
    ) -> Option<Channels> {
        if !s.wants(e.cue) {
            return None;
        }
        let key = (e.cue, e.device.clone());
        let debounce = Duration::from_secs(u64::from(s.debounce_s));
        if let Some(t) = self.last.get(&key) {
            if now.duration_since(*t) < debounce {
                return None;
            }
        }
        let quiet = s.quiet_hours.as_ref().is_some_and(|q| q.contains(local));
        let ch = Channels {
            speech: s.speech && !quiet,
            sound: s.sound && !quiet,
            notification: s.notification,
        };
        if ch == Channels::default() {
            return None;
        }
        self.last.insert(key, now);
        Some(ch)
    }
}

/// The "still inserted" reminders, per device key. A reminder is armed by a job's "done";
/// it is due `grace` later, then every `every`, `max` times at most; removal or a dismiss
/// ends it.
#[derive(Debug, Default)]
pub struct Reminders {
    armed: HashMap<String, Armed>,
}

#[derive(Debug, Clone)]
struct Armed {
    device: String,
    next: Instant,
    left: u32,
}

impl Reminders {
    /// Starts (or restarts) the reminder for `key`, after a job's "done".
    pub fn arm(&mut self, key: &str, device: &str, s: &CueSettings, now: Instant) {
        if s.reminder_max == 0 {
            self.armed.remove(key);
            return;
        }
        self.armed.insert(
            key.to_string(),
            Armed {
                device: device.to_string(),
                next: now + Duration::from_secs(u64::from(s.reminder_grace_s)),
                left: s.reminder_max,
            },
        );
    }

    /// Stops the reminder for `key`.
    pub fn dismiss(&mut self, key: &str) {
        self.armed.remove(key);
    }

    pub fn is_armed(&self, key: &str) -> bool {
        self.armed.contains_key(key)
    }

    /// The reminders due now, as cues. Keys not in `present` (removed devices) end.
    pub fn due(&mut self, present: &[String], s: &CueSettings, now: Instant) -> Vec<CueEvent> {
        self.armed.retain(|k, _| present.contains(k));
        let every = Duration::from_secs(u64::from(s.still_inserted_every_s).max(1));
        let mut out = Vec::new();
        self.armed.retain(|_, a| {
            if now < a.next {
                return true;
            }
            out.push(CueEvent::new(Cue::StillInserted, a.device.clone()));
            a.left -= 1;
            a.next = now + every;
            a.left > 0
        });
        out
    }
}

/// One cue for a whole batch or automation run: `done` devices finished, `failed` holds
/// (step, device) pairs. None when nothing ran.
pub fn batch_cue(done: &[String], failed: &[(String, String)]) -> Option<CueEvent> {
    let total = done.len() + failed.len();
    match (total, failed.first()) {
        (0, _) => None,
        (1, Some((step, device))) => Some(CueEvent::failed(step.clone(), device.clone())),
        (_, Some((step, _))) => {
            let same_step = failed.iter().all(|(s, _)| s == step);
            Some(CueEvent::failed(
                if same_step {
                    step.clone()
                } else {
                    "A step".into()
                },
                format!("{} of {total} devices", failed.len()),
            ))
        }
        (1, None) => Some(CueEvent::new(Cue::SafeToUnplug, done[0].clone())),
        (n, None) => Some(CueEvent::new(Cue::SafeToUnplug, format!("{n} devices"))),
    }
}

/// Where cues go. Each call returns when the cue has played.
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

impl RecordedCues {
    /// The spoken lines so far.
    pub fn spoken(&self) -> Vec<String> {
        self.played
            .lock()
            .unwrap()
            .iter()
            .filter_map(|p| match p {
                Played::Speak(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }
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

/// The real sink: `say`, `afplay`, `osascript`, each run to its end.
pub struct SystemCues;

fn run(program: &str, args: &[&str]) {
    let _ = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

impl CueSink for SystemCues {
    fn speak(&self, text: &str, voice: Option<&str>) {
        match voice.filter(|v| !v.trim().is_empty()) {
            Some(v) => run("/usr/bin/say", &["-v", v, "--", text]),
            None => run("/usr/bin/say", &["--", text]),
        }
    }
    fn sound(&self, name: &str) {
        let path = format!("/System/Library/Sounds/{name}.aiff");
        run("/usr/bin/afplay", &[&path]);
    }
    fn notify(&self, title: &str, body: &str) {
        // The texts are arguments of the script, never part of it.
        run(
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

/// Plays one admitted cue on its channels.
fn play(sink: &dyn CueSink, e: &CueEvent, ch: Channels, voice: Option<&str>) {
    let line = e.line();
    if ch.sound {
        sink.sound(e.cue.sound());
    }
    if ch.speech {
        sink.speak(&line, voice);
    }
    if ch.notification {
        sink.notify("QuadCam", &line);
    }
}

type Item = (CueEvent, Channels, Option<String>);

/// The cue queue: the gate, the reminders, and one player, so cues never overlap.
pub struct CueService {
    gate: Mutex<Gate>,
    pub reminders: Mutex<Reminders>,
    sink: Arc<dyn CueSink>,
    queue: Option<Mutex<mpsc::Sender<Item>>>,
}

impl CueService {
    /// Plays each cue on the caller's thread (tests: deterministic order, no thread).
    pub fn inline(sink: Arc<dyn CueSink>) -> Self {
        Self {
            gate: Mutex::default(),
            reminders: Mutex::default(),
            sink,
            queue: None,
        }
    }

    /// Plays cues one after another on a worker thread.
    pub fn queued(sink: Arc<dyn CueSink>) -> Self {
        let (tx, rx) = mpsc::channel::<Item>();
        let player = sink.clone();
        std::thread::spawn(move || {
            for (e, ch, voice) in rx {
                play(player.as_ref(), &e, ch, voice.as_deref());
            }
        });
        Self {
            gate: Mutex::default(),
            reminders: Mutex::default(),
            sink,
            queue: Some(Mutex::new(tx)),
        }
    }

    /// The system sink, queued (silent under cargo).
    pub fn system() -> Self {
        Self::queued(system())
    }

    /// Plays a cue if the settings and the debounce let it. True when it was queued.
    pub fn fire(&self, s: &CueSettings, e: CueEvent, now: Instant, local: NaiveTime) -> bool {
        let Some(ch) = self.gate.lock().unwrap().admit(s, &e, now, local) else {
            return false;
        };
        match &self.queue {
            Some(q) => q.lock().unwrap().send((e, ch, s.voice.clone())).is_ok(),
            None => {
                play(self.sink.as_ref(), &e, ch, s.voice.as_deref());
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn service() -> (Arc<RecordedCues>, CueService) {
        let sink = Arc::new(RecordedCues::default());
        (sink.clone(), CueService::inline(sink))
    }

    #[test]
    fn fire_follows_the_settings() {
        let (sink, cues) = service();
        let now = Instant::now();
        let s = CueSettings::default();
        assert!(cues.fire(
            &s,
            CueEvent::new(Cue::SafeToUnplug, "Bench radio"),
            now,
            t(12, 0)
        ));
        assert_eq!(
            *sink.played.lock().unwrap(),
            [
                Played::Speak("Bench radio done, safe to unplug.".into()),
                Played::Notify("QuadCam".into(), "Bench radio done, safe to unplug.".into())
            ]
        );
        let off = CueSettings {
            step_failed: false,
            ..Default::default()
        };
        assert!(!cues.fire(&off, CueEvent::failed("Backup", "x"), now, t(12, 0)));
        let muted = CueSettings {
            mute: true,
            ..Default::default()
        };
        assert!(!cues.fire(&muted, CueEvent::failed("Backup", "y"), now, t(12, 0)));
        let sound_only = CueSettings {
            speech: false,
            notification: false,
            sound: true,
            ..Default::default()
        };
        sink.played.lock().unwrap().clear();
        cues.fire(&sound_only, CueEvent::failed("Backup", "z"), now, t(12, 0));
        assert_eq!(
            *sink.played.lock().unwrap(),
            [Played::Sound("Basso".into())]
        );
    }

    #[test]
    fn the_same_cue_for_the_same_device_is_debounced() {
        let (sink, cues) = service();
        let s = CueSettings::default();
        let t0 = Instant::now();
        let e = || CueEvent::new(Cue::SafeToUnplug, "Card");
        assert!(cues.fire(&s, e(), t0, t(12, 0)));
        assert!(!cues.fire(&s, e(), t0 + Duration::from_secs(29), t(12, 0)));
        assert!(
            cues.fire(&s, CueEvent::new(Cue::SafeToUnplug, "Other"), t0, t(12, 0)),
            "another device"
        );
        assert!(
            cues.fire(&s, CueEvent::failed("Backup", "Card"), t0, t(12, 0)),
            "another cue"
        );
        assert!(cues.fire(&s, e(), t0 + Duration::from_secs(31), t(12, 0)));
        assert_eq!(sink.spoken().len(), 4);
    }

    #[test]
    fn quiet_hours_silence_speech_and_sound_only() {
        let q = QuietHours {
            start: "22:00".into(),
            end: "07:00".into(),
        };
        assert!(q.contains(t(23, 0)) && q.contains(t(6, 59)));
        assert!(!q.contains(t(7, 0)) && !q.contains(t(21, 59)));
        let day = QuietHours {
            start: "12:00".into(),
            end: "13:00".into(),
        };
        assert!(day.contains(t(12, 30)) && !day.contains(t(13, 0)));
        let (sink, cues) = service();
        let s = CueSettings {
            sound: true,
            quiet_hours: Some(q),
            ..Default::default()
        };
        cues.fire(
            &s,
            CueEvent::new(Cue::SafeToUnplug, "Card"),
            Instant::now(),
            t(23, 30),
        );
        assert_eq!(
            *sink.played.lock().unwrap(),
            [Played::Notify(
                "QuadCam".into(),
                "Card done, safe to unplug.".into()
            )],
            "macOS Focus decides about the notification"
        );
        let no_notes = CueSettings {
            notification: false,
            ..s
        };
        assert!(!cues.fire(
            &no_notes,
            CueEvent::new(Cue::SafeToUnplug, "Other"),
            Instant::now(),
            t(23, 30)
        ));
    }

    #[test]
    fn reminders_wait_repeat_cap_and_stop() {
        let s = CueSettings {
            reminder_grace_s: 60,
            still_inserted_every_s: 300,
            reminder_max: 2,
            ..Default::default()
        };
        let t0 = Instant::now();
        let at = |n| t0 + Duration::from_secs(n);
        let k = vec!["disk4".to_string()];
        let mut r = Reminders::default();
        assert!(r.due(&k, &s, at(1000)).is_empty(), "nothing before a done");
        r.arm("disk4", "The card", &s, t0);
        assert!(r.due(&k, &s, at(59)).is_empty(), "the grace");
        let due = r.due(&k, &s, at(60));
        assert_eq!(due[0].line(), "The card is still inserted.");
        assert!(r.due(&k, &s, at(300)).is_empty());
        assert_eq!(r.due(&k, &s, at(360)).len(), 1);
        assert!(r.due(&k, &s, at(5000)).is_empty(), "the cap");
        assert!(!r.is_armed("disk4"));
        // Removal and dismiss end it.
        r.arm("disk4", "The card", &s, t0);
        assert!(r.due(&[], &s, at(30)).is_empty());
        assert!(r.due(&k, &s, at(100)).is_empty(), "removed");
        r.arm("disk4", "The card", &s, t0);
        r.dismiss("disk4");
        assert!(r.due(&k, &s, at(100)).is_empty(), "dismissed");
        let none = CueSettings {
            reminder_max: 0,
            ..Default::default()
        };
        r.arm("disk4", "The card", &none, t0);
        assert!(!r.is_armed("disk4"));
    }

    #[test]
    fn one_cue_per_batch() {
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(batch_cue(&[], &[]), None);
        assert_eq!(
            batch_cue(&names(&["Radio"]), &[]).unwrap().line(),
            "Radio done, safe to unplug."
        );
        assert_eq!(
            batch_cue(&names(&["A", "B", "C"]), &[]).unwrap().line(),
            "3 devices done, safe to unplug."
        );
        let failed = vec![("Backup".to_string(), "B".to_string())];
        assert_eq!(
            batch_cue(&[], &failed).unwrap().line(),
            "Backup failed on B."
        );
        assert_eq!(
            batch_cue(&names(&["A", "C"]), &failed).unwrap().line(),
            "Backup failed on 1 of 3 devices."
        );
        let mixed = vec![
            ("Backup".to_string(), "B".to_string()),
            ("Import".to_string(), "C".to_string()),
        ];
        assert_eq!(
            batch_cue(&names(&["A"]), &mixed).unwrap().line(),
            "A step failed on 2 of 3 devices."
        );
    }

    #[test]
    fn the_queue_plays_in_order_on_one_thread() {
        let sink = Arc::new(RecordedCues::default());
        let cues = CueService::queued(sink.clone());
        let s = CueSettings {
            notification: false,
            ..Default::default()
        };
        for d in ["A", "B", "C"] {
            assert!(cues.fire(
                &s,
                CueEvent::new(Cue::SafeToUnplug, d),
                Instant::now(),
                t(9, 0)
            ));
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while sink.spoken().len() < 3 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            sink.spoken(),
            [
                "A done, safe to unplug.",
                "B done, safe to unplug.",
                "C done, safe to unplug."
            ]
        );
    }

    #[test]
    fn settings_fill_missing_fields() {
        let s: CueSettings = serde_json::from_str(
            r#"{"speech": false, "quiet_hours": {"start": "22:00", "end": "07:00"}}"#,
        )
        .unwrap();
        assert!(!s.speech);
        assert!(s.notification);
        assert_eq!(s.reminder_max, 3);
        assert!(s.quiet_hours.is_some());
    }

    #[test]
    fn silent_under_cargo() {
        assert!(!cues_enabled(None, true));
        assert!(cues_enabled(None, false));
        assert!(!cues_enabled(Some("off"), false));
        assert!(cues_enabled(Some("real"), true));
    }
}
