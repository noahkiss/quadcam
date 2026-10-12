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
//! 6. With `voice_source` `voice_pack` (the default), speech plays clips from an installed
//!    voice pack, in the same voice as the radio: the pack picked in `voice_pack`, else the
//!    pack chosen for the radio seen last. One cue never mixes voices (`pack_clips`): (a) the
//!    device's own name (the person's own line for it) then the bare phrase
//!    (`Cue::pack_line`); else (b) the kind's whole sentence (`KindWord::line`); else (c) the
//!    macOS voice says the whole cue. The lines are in `resources/voice/lines.csv`, group
//!    `cues`.
//!
//! `SystemCues` speaks with `/usr/bin/say` and plays a system sound with `/usr/bin/afplay`,
//! each a child process with an argv list, never a shell, run to the end before the next
//! cue. It posts a notification through the UserNotifications framework when the process
//! runs from an app bundle (`native_notify`), else with `/usr/bin/osascript`: outside a
//! bundle `UNUserNotificationCenter` has no app to post for and raises. A process started by cargo gets the
//! silent `RecordedCues` from `system()` unless `QUADCAM_CUES=real`, so tests make no sound.

use chrono::NaiveTime;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    /// An FC ran on USB with its battery in past its limit: it may overheat.
    UnplugNow,
}

/// The kind of device a voice pack names when it has no clip of the device's own name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KindWord {
    Radio,
    Quad,
    Goggles,
    /// A DJI air unit (an O4) over USB.
    AirUnit,
    DvrCard,
    /// An ExpressLRS transmitter or receiver.
    ExpressLrs,
}

impl KindWord {
    pub const ALL: [KindWord; 6] = [
        KindWord::Radio,
        KindWord::Quad,
        KindWord::Goggles,
        KindWord::AirUnit,
        KindWord::DvrCard,
        KindWord::ExpressLrs,
    ];

    /// The cue as one whole sentence for this kind, a line of the `quadcam` set (group
    /// `cues`): the macOS line with the kind in place of the name, said in one breath.
    /// None where the pair does not apply: "still inserted" is for cards and the air unit,
    /// "unplug now" for a quad only.
    pub fn line(self, cue: Cue) -> Option<(&'static str, &'static str)> {
        use KindWord::*;
        Some(match (cue, self) {
            (Cue::SafeToUnplug, Radio) => ("SOUNDS/en/qdradio.wav", "Radio done, safe to unplug."),
            (Cue::SafeToUnplug, Quad) => ("SOUNDS/en/qdquad.wav", "Quad done, safe to unplug."),
            (Cue::SafeToUnplug, Goggles) => {
                ("SOUNDS/en/qdgoggle.wav", "Goggles done, safe to unplug.")
            }
            (Cue::SafeToUnplug, AirUnit) => {
                ("SOUNDS/en/qdair.wav", "Air unit done, safe to unplug.")
            }
            (Cue::SafeToUnplug, DvrCard) => {
                ("SOUNDS/en/qddvr.wav", "DVR card done, safe to unplug.")
            }
            (Cue::SafeToUnplug, ExpressLrs) => {
                ("SOUNDS/en/qdelrs.wav", "ExpressLRS done, safe to unplug.")
            }
            (Cue::StillInserted, Radio) => {
                ("SOUNDS/en/qsradio.wav", "The radio card is still inserted.")
            }
            (Cue::StillInserted, Goggles) => (
                "SOUNDS/en/qsgoggle.wav",
                "The goggles card is still inserted.",
            ),
            (Cue::StillInserted, AirUnit) => {
                ("SOUNDS/en/qsair.wav", "The air unit is still plugged in.")
            }
            (Cue::StillInserted, DvrCard) => {
                ("SOUNDS/en/qsdvr.wav", "The DVR card is still inserted.")
            }
            (Cue::StepFailed, Radio) => ("SOUNDS/en/qfradio.wav", "A step failed on the radio."),
            (Cue::StepFailed, Quad) => ("SOUNDS/en/qfquad.wav", "A step failed on the quad."),
            (Cue::StepFailed, Goggles) => {
                ("SOUNDS/en/qfgoggle.wav", "A step failed on the goggles.")
            }
            (Cue::StepFailed, AirUnit) => ("SOUNDS/en/qfair.wav", "A step failed on the air unit."),
            (Cue::StepFailed, DvrCard) => ("SOUNDS/en/qfdvr.wav", "A step failed on the DVR card."),
            (Cue::StepFailed, ExpressLrs) => (
                "SOUNDS/en/qfelrs.wav",
                "A step failed on the ExpressLRS device.",
            ),
            (Cue::UnplugNow, Quad) => Cue::UnplugNow.pack_line(),
            _ => return None,
        })
    }
}

/// One cue to play: what, for which device (or "3 devices"), and the failed step. `name`
/// is the name the person gave the device, `kind` its kind: a voice pack speaks them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CueEvent {
    pub cue: Cue,
    pub device: String,
    pub step: Option<String>,
    pub name: Option<String>,
    pub kind: Option<KindWord>,
}

impl CueEvent {
    pub fn new(cue: Cue, device: impl Into<String>) -> Self {
        Self {
            cue,
            device: device.into(),
            step: None,
            name: None,
            kind: None,
        }
    }

    pub fn failed(step: impl Into<String>, device: impl Into<String>) -> Self {
        Self {
            cue: Cue::StepFailed,
            device: device.into(),
            step: Some(step.into()),
            name: None,
            kind: None,
        }
    }

    /// The device's own name (empty: none) and kind, for a voice pack.
    pub fn about(mut self, name: Option<String>, kind: Option<KindWord>) -> Self {
        self.name = name.filter(|n| !n.trim().is_empty());
        self.kind = kind;
        self
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
            Cue::UnplugNow => match &self.step {
                Some(m) => format!(
                    "Unplug {} now. It has run on USB with a battery for {m}.",
                    self.device
                ),
                None => format!("Unplug {} now.", self.device),
            },
        }
    }
}

impl Cue {
    pub const ALL: [Cue; 4] = [
        Cue::SafeToUnplug,
        Cue::StillInserted,
        Cue::StepFailed,
        Cue::UnplugNow,
    ];

    /// The cue's line in a voice pack: its card path under `SOUNDS/en/` and its text, a line
    /// of `resources/voice/lines.csv` (group `cues`). The text is fixed: a pack cannot say a
    /// device's own name.
    pub fn pack_line(self) -> (&'static str, &'static str) {
        match self {
            Cue::SafeToUnplug => ("SOUNDS/en/qcdone.wav", "Done, safe to unplug."),
            Cue::StillInserted => ("SOUNDS/en/qcstill.wav", "Still inserted."),
            Cue::StepFailed => ("SOUNDS/en/qcfail.wav", "A step failed."),
            Cue::UnplugNow => ("SOUNDS/en/qcunplug.wav", "Unplug the quad now."),
        }
    }

    /// The system sound (`/System/Library/Sounds/<name>.aiff`).
    pub fn sound(self) -> &'static str {
        match self {
            Cue::SafeToUnplug => "Glass",
            Cue::StillInserted => "Ping",
            Cue::StepFailed => "Basso",
            Cue::UnplugNow => "Sosumi",
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
    /// "Unplug <device> now.": an FC on USB with its battery in past its time limit.
    pub unplug_now: bool,
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
    /// What speaks the lines: a voice pack when one is installed (the default), or macOS
    /// (`say`) when the person picks it. A cue the pack has no clip for speaks with macOS.
    pub voice_source: VoiceSource,
    /// The installed pack that speaks cues with `voice_pack`; None for the pack chosen for
    /// the radio seen last.
    pub voice_pack: Option<String>,
}

/// What speaks the cue lines.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum VoiceSource {
    Macos,
    /// The default: a voice pack when one is installed (see `CueSettings::voice_pack`),
    /// else macOS.
    #[default]
    VoicePack,
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
            unplug_now: true,
            debounce_s: 30,
            reminder_grace_s: 60,
            still_inserted_every_s: 300,
            reminder_max: 3,
            quiet_hours: None,
            voice: None,
            voice_source: VoiceSource::VoicePack,
            voice_pack: None,
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
                Cue::UnplugNow => self.unplug_now,
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
    name: Option<String>,
    kind: Option<KindWord>,
    next: Instant,
    left: u32,
}

impl Reminders {
    /// Starts (or restarts) the reminder for `key`, after a job's "done".
    pub fn arm(&mut self, key: &str, device: &str, s: &CueSettings, now: Instant) {
        self.arm_for(key, device, None, None, s, now)
    }

    /// `arm`, with the device's own name and kind for a voice pack's reminder.
    pub fn arm_for(
        &mut self,
        key: &str,
        device: &str,
        name: Option<String>,
        kind: Option<KindWord>,
        s: &CueSettings,
        now: Instant,
    ) {
        if s.reminder_max == 0 {
            self.armed.remove(key);
            return;
        }
        self.armed.insert(
            key.to_string(),
            Armed {
                device: device.to_string(),
                name,
                kind,
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

    /// The keys with a reminder armed, sorted.
    pub fn armed(&self) -> Vec<String> {
        let mut v: Vec<String> = self.armed.keys().cloned().collect();
        v.sort();
        v
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
            out.push(
                CueEvent::new(Cue::StillInserted, a.device.clone()).about(a.name.clone(), a.kind),
            );
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

/// The clip that speaks `cue` from a voice pack, or None for the macOS voice. `pack` is the
/// pack's folder, the card paths its manifest lists and its language (see
/// `CueSettings::voice_pack`). The line must be listed and on disk; a pack for another
/// language holds it under `SOUNDS/<lang>/`.
pub fn pack_clip(pack: Option<(&Path, &[String], &str)>, cue: Cue) -> Option<PathBuf> {
    pack_file(pack, cue.pack_line().0)
}

/// A voice pack's file for a card path under `SOUNDS/en/` (or the pack's own language),
/// when the manifest lists it and it is on disk.
pub fn pack_file(pack: Option<(&Path, &[String], &str)>, path: &str) -> Option<PathBuf> {
    let (dir, files, lang) = pack?;
    let own = path.replacen("SOUNDS/en/", &format!("SOUNDS/{lang}/"), 1);
    let found = [own.as_str(), path]
        .into_iter()
        .filter(|p| files.iter().any(|f| f == p))
        .map(|p| dir.join(p))
        .find(|f| f.is_file());
    found
}

/// The clips that speak a cue from a voice pack, in order. Never two voices in one cue:
/// (a) the person's name for the device (`name_path`, its line in the person's own lines)
/// and then the bare phrase, when the pack holds both; else (b) the kind's whole sentence
/// (`KindWord::line`); else None, and the macOS voice says the whole cue.
pub fn pack_clips(
    pack: Option<(&Path, &[String], &str)>,
    e: &CueEvent,
    name_path: Option<&str>,
) -> Option<Vec<PathBuf>> {
    let named = name_path
        .and_then(|p| pack_file(pack, p))
        .zip(pack_clip(pack, e.cue));
    if let Some((name, phrase)) = named {
        return Some(vec![name, phrase]);
    }
    let (line, _) = e.kind?.line(e.cue)?;
    Some(vec![pack_file(pack, line)?])
}

/// The pause between a name clip and the phrase clip.
pub const CLIP_GAP: Duration = Duration::from_millis(250);

/// Where cues go. Each call returns when the cue has played.
pub trait CueSink: Send + Sync {
    fn speak(&self, text: &str, voice: Option<&str>);
    /// Plays a voice pack's clips (WAVs) in order, `CLIP_GAP` apart.
    fn clips(&self, paths: &[PathBuf]);
    fn sound(&self, name: &str);
    fn notify(&self, title: &str, body: &str);
}

/// One call a `RecordedCues` saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Played {
    Speak(String),
    Clips(Vec<PathBuf>),
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
    fn clips(&self, paths: &[PathBuf]) {
        self.played
            .lock()
            .unwrap()
            .push(Played::Clips(paths.to_vec()));
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
    fn clips(&self, paths: &[PathBuf]) {
        for (i, p) in paths.iter().enumerate() {
            if i > 0 {
                std::thread::sleep(CLIP_GAP);
            }
            run("/usr/bin/afplay", &[&p.to_string_lossy()]);
        }
    }
    fn sound(&self, name: &str) {
        let path = format!("/System/Library/Sounds/{name}.aiff");
        run("/usr/bin/afplay", &[&path]);
    }
    fn notify(&self, title: &str, body: &str) {
        #[cfg(target_os = "macos")]
        if native_notify(title, body) {
            return;
        }
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

/// Posts a notification through `UNUserNotificationCenter`, as the app. False (nothing
/// posted) when this process is not inside an app bundle, where the framework has no app to
/// post for. The first post asks the person to allow notifications; a refusal drops it.
#[cfg(target_os = "macos")]
fn native_notify(title: &str, body: &str) -> bool {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSBundle, NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
        UNUserNotificationCenter,
    };
    let bundle = NSBundle::mainBundle();
    let in_app =
        bundle.bundleIdentifier().is_some() && bundle.bundlePath().to_string().ends_with(".app");
    if !in_app {
        return false;
    }
    let (title, body) = (title.to_string(), body.to_string());
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let post = RcBlock::new(move |granted: Bool, _err: *mut NSError| {
        if !granted.as_bool() {
            return;
        }
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&title));
        content.setBody(&NSString::from_str(&body));
        let id = NSString::from_str(&format!("cue-{}", now_nanos()));
        let req = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
        UNUserNotificationCenter::currentNotificationCenter()
            .addNotificationRequest_withCompletionHandler(&req, None);
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &post,
    );
    true
}

#[cfg(target_os = "macos")]
fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
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

/// Plays one admitted cue on its channels. Speech plays `clip` (a voice pack's) when there
/// is one, else the macOS voice speaks the line.
fn play(sink: &dyn CueSink, e: &CueEvent, ch: Channels, voice: Option<&str>, clips: &[PathBuf]) {
    let line = e.line();
    if ch.sound {
        sink.sound(e.cue.sound());
    }
    if ch.speech {
        if clips.is_empty() {
            sink.speak(&line, voice);
        } else {
            sink.clips(clips);
        }
    }
    if ch.notification {
        sink.notify("QuadCam", &line);
    }
}

type Item = (CueEvent, Channels, Option<String>, Vec<PathBuf>);

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
            for (e, ch, voice, clip) in rx {
                play(player.as_ref(), &e, ch, voice.as_deref(), &clip);
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

    /// Plays a cue if the settings and the debounce let it, with the macOS voice. True when
    /// it was queued.
    pub fn fire(&self, s: &CueSettings, e: CueEvent, now: Instant, local: NaiveTime) -> bool {
        self.fire_with(s, e, |_| None, now, local)
    }

    /// `fire`, with `clip` giving the voice pack's clip for a cue (`pack_clip`); it is asked
    /// only with `voice_source` `voice_pack`, and None speaks with macOS.
    pub fn fire_with(
        &self,
        s: &CueSettings,
        e: CueEvent,
        clips: impl FnOnce(&CueEvent) -> Option<Vec<PathBuf>>,
        now: Instant,
        local: NaiveTime,
    ) -> bool {
        let Some(ch) = self.gate.lock().unwrap().admit(s, &e, now, local) else {
            return false;
        };
        let clip = match s.voice_source {
            VoiceSource::VoicePack if ch.speech => clips(&e).unwrap_or_default(),
            _ => Vec::new(),
        };
        match &self.queue {
            Some(q) => q
                .lock()
                .unwrap()
                .send((e, ch, s.voice.clone(), clip))
                .is_ok(),
            None => {
                play(self.sink.as_ref(), &e, ch, s.voice.as_deref(), &clip);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An unset `voice_source` means a voice pack when one is installed; an explicit
    /// `macos` stays macOS.
    #[test]
    fn voice_source_defaults_to_the_voice_pack() {
        let s: CueSettings = serde_json::from_value(serde_json::json!({"mute": true})).unwrap();
        assert_eq!(s.voice_source, VoiceSource::VoicePack);
        let s: CueSettings =
            serde_json::from_value(serde_json::json!({"voice_source": "macos"})).unwrap();
        assert_eq!(s.voice_source, VoiceSource::Macos);
    }

    #[test]
    fn a_pack_clip_needs_the_line_listed_and_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let done = "SOUNDS/en/qcdone.wav";
        std::fs::create_dir_all(dir.path().join("SOUNDS/en")).unwrap();
        std::fs::write(dir.path().join(done), b"RIFF").unwrap();
        let files = vec![done.to_string(), "SOUNDS/en/qcfail.wav".to_string()];
        let p = Some((dir.path(), files.as_slice(), "en"));
        assert_eq!(pack_clip(p, Cue::SafeToUnplug), Some(dir.path().join(done)));
        // Listed but lost on disk, or not in the pack: the macOS voice.
        assert_eq!(pack_clip(p, Cue::StepFailed), None);
        assert_eq!(pack_clip(p, Cue::StillInserted), None);
        assert_eq!(pack_clip(None, Cue::SafeToUnplug), None);
        // A German pack holds its lines under SOUNDS/de/.
        std::fs::create_dir_all(dir.path().join("SOUNDS/de")).unwrap();
        std::fs::write(dir.path().join("SOUNDS/de/qcdone.wav"), b"RIFF").unwrap();
        let de = vec!["SOUNDS/de/qcdone.wav".to_string()];
        assert_eq!(
            pack_clip(Some((dir.path(), de.as_slice(), "de")), Cue::SafeToUnplug),
            Some(dir.path().join("SOUNDS/de/qcdone.wav"))
        );
    }

    #[test]
    fn a_voice_pack_clip_replaces_speech_only() {
        let (sink, cues) = service();
        let s = CueSettings {
            voice_source: VoiceSource::VoicePack,
            ..Default::default()
        };
        let clip = PathBuf::from("/packs/x/SOUNDS/en/qcdone.wav");
        let c = clip.clone();
        cues.fire_with(
            &s,
            CueEvent::new(Cue::SafeToUnplug, "Bench radio"),
            move |_| Some(vec![c]),
            Instant::now(),
            t(12, 0),
        );
        assert_eq!(
            *sink.played.lock().unwrap(),
            [
                Played::Clips(vec![clip.clone()]),
                Played::Notify("QuadCam".into(), "Bench radio done, safe to unplug.".into())
            ]
        );
        // macOS as the source never asks for a clip.
        sink.played.lock().unwrap().clear();
        cues.fire_with(
            &CueSettings {
                voice_source: VoiceSource::Macos,
                ..Default::default()
            },
            CueEvent::new(Cue::SafeToUnplug, "Other radio"),
            |_| panic!("asked for a clip"),
            Instant::now(),
            t(12, 0),
        );
        assert!(matches!(sink.played.lock().unwrap()[0], Played::Speak(_)));
    }

    /// Name then phrase, else the kind's sentence, else nothing (macOS says it all).
    #[test]
    fn pack_clips_never_mix_voices() {
        let dir = tempfile::tempdir().unwrap();
        let put = |p: &str| {
            let f = dir.path().join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, b"RIFF").unwrap();
        };
        let (name, phrase, kind) = (
            "SOUNDS/en/nm000001.wav",
            "SOUNDS/en/qcdone.wav",
            "SOUNDS/en/qdradio.wav",
        );
        for p in [name, phrase, kind] {
            put(p);
        }
        let e = CueEvent::new(Cue::SafeToUnplug, "Pocket")
            .about(Some("Pocket".into()), Some(KindWord::Radio));
        let all: Vec<String> = [name, phrase, kind].map(String::from).to_vec();
        let pk = |files: &[String]| -> Option<Vec<PathBuf>> {
            pack_clips(Some((dir.path(), files, "en")), &e, Some(name))
        };
        let j = |p: &str| dir.path().join(p);
        assert_eq!(pk(&all), Some(vec![j(name), j(phrase)]));
        // No name clip: the kind's whole sentence.
        let no_name: Vec<String> = [phrase, kind].map(String::from).to_vec();
        assert_eq!(pk(&no_name), Some(vec![j(kind)]));
        // A name clip without the phrase: the kind's sentence, never the name alone.
        let no_phrase: Vec<String> = [name, kind].map(String::from).to_vec();
        assert_eq!(pk(&no_phrase), Some(vec![j(kind)]));
        // Neither the name nor the kind: the macOS voice for the whole cue.
        assert_eq!(pk(&[phrase.to_string()]), None);
        // A pair that does not apply ("still inserted" for a quad) has no sentence.
        assert_eq!(KindWord::Quad.line(Cue::StillInserted), None);
        assert!(KindWord::ALL
            .iter()
            .all(|k| k.line(Cue::StepFailed).is_some()));
    }

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
