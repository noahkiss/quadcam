//! The radio as a USB joystick: an EdgeTX radio in USB Joystick mode is a HID game
//! controller (`1209:4f54`, "<Radio> Joystick"). The web view's Gamepad API never sees it,
//! so QuadCam reads it natively (hidapi, shared, so other apps keep it too) and streams the
//! reports to the page. The live controls page uses it; the built-in sim will too.
//!
//! - A report is 19 bytes, no report id: 24 button bits (3 bytes, bit 0 first), then 8
//!   axes, u16 little-endian, 0..2048 (11 bits).
//! - The axes are the radio's mixer outputs: axis i is CH(i+1), so a model's mixes say
//!   which stick each channel follows (`switchmap::StickChannel`). 0 is −100 % (988 µs),
//!   1024 is centre, 2048 is +100 % (2012 µs).
//! - Fail-safe: a process started by cargo reaches no real HID device (`system` gives
//!   `NoHid`) unless `QUADCAM_HID=real`. Tests use `FakeHid`.
//!
//! Learned from the device's own report descriptor, read with hidapi.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// pid.codes' EdgeTX joystick id, the same on every EdgeTX radio.
pub const VID: u16 = 0x1209;
pub const PID: u16 = 0x4f54;
pub const REPORT_LEN: usize = 19;
pub const AXES: usize = 8;
pub const BUTTONS: usize = 24;
/// An axis' top value.
pub const AXIS_MAX: u16 = 2048;

/// The watch sends at most one frame per this interval (the page draws at 60 Hz).
pub const FRAME_INTERVAL: Duration = Duration::from_millis(16);
/// And one at least this often while connected, so the page knows the stream is alive.
pub const HEARTBEAT: Duration = Duration::from_millis(500);
/// How long the watch waits before it looks for the radio again.
pub const RETRY: Duration = Duration::from_secs(1);

/// One report, read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RadioFrame {
    /// Counts reports since the watch started.
    pub seq: u64,
    /// Bit i is button i+1.
    pub buttons: u32,
    /// Raw axes, 0..2048, axis 0 first.
    pub axes: Vec<u16>,
    /// The axes as channel values in µs, CH1 first.
    pub channels: Vec<u16>,
}

/// The radio as the page sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RadioEvent {
    pub connected: bool,
    /// `Radiomaster Pocket Joystick`.
    #[serde(default)]
    pub product: Option<String>,
    #[serde(default)]
    pub frame: Option<RadioFrame>,
}

/// `gear_radio`: one look at the radio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RadioSnapshot {
    pub connected: bool,
    #[serde(default)]
    pub product: Option<String>,
    #[serde(default)]
    pub frame: Option<RadioFrame>,
    /// Why there is no frame.
    #[serde(default)]
    pub message: Option<String>,
}

/// An axis value in µs: 0 is 988, 1024 is 1500, 2048 is 2012.
pub fn axis_us(raw: u16) -> u16 {
    988 + raw.min(AXIS_MAX) / 2
}

/// Reads a report: the button bits and the raw axes.
pub fn parse_report(r: &[u8]) -> Option<(u32, [u16; AXES])> {
    if r.len() < REPORT_LEN {
        return None;
    }
    let buttons = r[0] as u32 | (r[1] as u32) << 8 | (r[2] as u32) << 16;
    let mut axes = [0u16; AXES];
    for (i, a) in axes.iter_mut().enumerate() {
        *a = u16::from_le_bytes([r[3 + 2 * i], r[4 + 2 * i]]);
    }
    Some((buttons, axes))
}

/// A report as a frame.
pub fn frame(seq: u64, r: &[u8]) -> Option<RadioFrame> {
    let (buttons, axes) = parse_report(r)?;
    Some(RadioFrame {
        seq,
        buttons,
        axes: axes.to_vec(),
        channels: axes.iter().map(|a| axis_us(*a)).collect(),
    })
}

/// Builds a report (tests and the mock): button bits and raw axes.
pub fn report(buttons: u32, axes: [u16; AXES]) -> Vec<u8> {
    let mut r = vec![buttons as u8, (buttons >> 8) as u8, (buttons >> 16) as u8];
    for a in axes {
        r.extend_from_slice(&a.to_le_bytes());
    }
    r
}

/// Where joystick radios come from.
pub trait HidSource: Send + Sync {
    /// Opens the radio's joystick; None when none is plugged in (or the radio is in
    /// another USB mode).
    fn open(&self) -> Result<Option<Box<dyn HidReader>>>;
}

/// One open joystick.
pub trait HidReader: Send {
    fn product(&self) -> String;
    /// One report, or None when none came within `timeout`. An error means the radio is
    /// gone.
    fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>>;
}

/// No HID devices: the source in tests unless `QUADCAM_HID=real`.
pub struct NoHid;

impl HidSource for NoHid {
    fn open(&self) -> Result<Option<Box<dyn HidReader>>> {
        Ok(None)
    }
}

/// A scripted radio. Clones share it.
#[derive(Clone, Default)]
pub struct FakeHid {
    st: Arc<Mutex<FakeState>>,
}

#[derive(Default)]
struct FakeState {
    plugged: bool,
    reports: VecDeque<Vec<u8>>,
    /// Bumped on unplug, so readers opened before it fail.
    generation: u64,
    opens: u32,
}

impl FakeHid {
    /// A radio plugged in, with no reports yet.
    pub fn plugged() -> Self {
        let f = Self::default();
        f.st.lock().unwrap().plugged = true;
        f
    }
    /// Queues a report.
    pub fn push(&self, report: Vec<u8>) {
        self.st.lock().unwrap().reports.push_back(report);
    }
    pub fn unplug(&self) {
        let mut s = self.st.lock().unwrap();
        s.plugged = false;
        s.generation += 1;
        s.reports.clear();
    }
    pub fn plug(&self) {
        self.st.lock().unwrap().plugged = true;
    }
    pub fn opens(&self) -> u32 {
        self.st.lock().unwrap().opens
    }
}

struct FakeReader {
    st: Arc<Mutex<FakeState>>,
    generation: u64,
}

impl HidSource for FakeHid {
    fn open(&self) -> Result<Option<Box<dyn HidReader>>> {
        let mut s = self.st.lock().unwrap();
        if !s.plugged {
            return Ok(None);
        }
        s.opens += 1;
        Ok(Some(Box::new(FakeReader {
            st: self.st.clone(),
            generation: s.generation,
        })))
    }
}

impl HidReader for FakeReader {
    fn product(&self) -> String {
        "Test Radio Joystick".into()
    }
    fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        {
            let mut s = self.st.lock().unwrap();
            if s.generation != self.generation {
                bail!("the radio is gone");
            }
            if let Some(r) = s.reports.pop_front() {
                return Ok(Some(r));
            }
        }
        std::thread::sleep(timeout.min(Duration::from_millis(5)));
        Ok(None)
    }
}

/// Whether to reach real HID devices: `QUADCAM_HID=real` forces them on and any other
/// value off; unset, they are on except under cargo.
pub fn hid_enabled(setting: Option<&str>, under_cargo: bool) -> bool {
    match setting {
        Some(v) => v == "real",
        None => !under_cargo,
    }
}

/// The system's joystick radios, or `NoHid` in a test process.
pub fn system() -> Arc<dyn HidSource> {
    if !hid_enabled(
        std::env::var("QUADCAM_HID").ok().as_deref(),
        std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
    ) {
        return Arc::new(NoHid);
    }
    real()
}

#[cfg(target_os = "macos")]
fn real() -> Arc<dyn HidSource> {
    Arc::new(RealHid::default())
}

#[cfg(not(target_os = "macos"))]
fn real() -> Arc<dyn HidSource> {
    Arc::new(NoHid)
}

/// hidapi, opened shared. One `HidApi` per process (the library allows one).
#[cfg(target_os = "macos")]
#[derive(Default)]
struct RealHid {
    api: Mutex<Option<hidapi::HidApi>>,
}

#[cfg(target_os = "macos")]
impl HidSource for RealHid {
    fn open(&self) -> Result<Option<Box<dyn HidReader>>> {
        let mut guard = self.api.lock().unwrap();
        let api = match guard.as_mut() {
            Some(a) => {
                a.refresh_devices()?;
                a
            }
            None => guard.insert(hidapi::HidApi::new()?),
        };
        if !api
            .device_list()
            .any(|d| d.vendor_id() == VID && d.product_id() == PID)
        {
            return Ok(None);
        }
        let dev = api.open(VID, PID)?;
        let product = dev
            .get_product_string()
            .ok()
            .flatten()
            .unwrap_or_else(|| "USB radio".into());
        Ok(Some(Box::new(RealReader { dev, product })))
    }
}

#[cfg(target_os = "macos")]
struct RealReader {
    dev: hidapi::HidDevice,
    product: String,
}

#[cfg(target_os = "macos")]
impl HidReader for RealReader {
    fn product(&self) -> String {
        self.product.clone()
    }
    fn read(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        let mut buf = [0u8; 64];
        let n = self
            .dev
            .read_timeout(&mut buf, timeout.as_millis() as i32)?;
        Ok((n > 0).then(|| buf[..n].to_vec()))
    }
}

/// One look: opens the radio and waits up to `wait` for a report.
pub fn snapshot(src: &dyn HidSource, wait: Duration) -> Result<RadioSnapshot> {
    let Some(mut r) = src.open()? else {
        return Ok(RadioSnapshot {
            connected: false,
            product: None,
            frame: None,
            message: Some(
                "No radio in USB Joystick mode. Plug it in and choose USB Joystick on the radio."
                    .into(),
            ),
        });
    };
    let product = r.product();
    let start = Instant::now();
    let mut last = None;
    while start.elapsed() < wait {
        match r.read(Duration::from_millis(50))? {
            Some(rep) => {
                if let Some(f) = frame(1, &rep) {
                    last = Some(f);
                    break;
                }
            }
            None => continue,
        }
    }
    let message = last
        .is_none()
        .then(|| "The radio sent no report: is USB Joystick mode on?".to_string());
    Ok(RadioSnapshot {
        connected: true,
        product: Some(product),
        frame: last,
        message,
    })
}

/// Streams the radio until `stop`: a `connected: false` event while there is none, each
/// changed frame (at most one per `FRAME_INTERVAL`), a heartbeat every `HEARTBEAT`, and
/// a reopen after the radio goes.
pub fn watch(src: &dyn HidSource, stop: &AtomicBool, mut emit: impl FnMut(RadioEvent)) {
    let mut seq = 0u64;
    let mut said_absent = false;
    while !stop.load(Ordering::SeqCst) {
        let reader = src.open().ok().flatten();
        let Some(mut r) = reader else {
            if !said_absent {
                emit(RadioEvent {
                    connected: false,
                    product: None,
                    frame: None,
                });
                said_absent = true;
            }
            sleep_unless(stop, RETRY);
            continue;
        };
        said_absent = false;
        let product = Some(r.product());
        emit(RadioEvent {
            connected: true,
            product: product.clone(),
            frame: None,
        });
        let mut sent: Option<(Instant, Vec<u8>)> = None;
        let mut pending: Option<Vec<u8>> = None;
        while !stop.load(Ordering::SeqCst) {
            match r.read(Duration::from_millis(20)) {
                Ok(Some(rep)) => {
                    seq += 1;
                    if sent.as_ref().is_none_or(|(_, last)| *last != rep) {
                        pending = Some(rep);
                    }
                }
                Ok(None) => {}
                Err(_) => break,
            }
            let due = match &sent {
                None => pending.is_some(),
                Some((t, _)) => {
                    (pending.is_some() && t.elapsed() >= FRAME_INTERVAL) || t.elapsed() >= HEARTBEAT
                }
            };
            if due {
                let rep = pending
                    .take()
                    .or_else(|| sent.as_ref().map(|(_, l)| l.clone()));
                if let Some(rep) = rep {
                    if let Some(f) = frame(seq, &rep) {
                        emit(RadioEvent {
                            connected: true,
                            product: product.clone(),
                            frame: Some(f),
                        });
                    }
                    sent = Some((Instant::now(), rep));
                }
            }
        }
        if !stop.load(Ordering::SeqCst) {
            emit(RadioEvent {
                connected: false,
                product: None,
                frame: None,
            });
            said_absent = true;
            sleep_unless(stop, Duration::from_millis(200));
        }
    }
}

fn sleep_unless(stop: &AtomicBool, d: Duration) {
    let end = Instant::now() + d;
    while !stop.load(Ordering::SeqCst) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_cargo_there_is_no_real_hid() {
        assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some());
        assert!(!hid_enabled(None, true));
        assert!(hid_enabled(Some("real"), true));
        assert!(!hid_enabled(Some("off"), false));
        assert!(hid_enabled(None, false));
        if std::env::var("QUADCAM_HID").ok().as_deref() != Some("real") {
            assert!(system().open().unwrap().is_none());
        }
    }

    #[test]
    fn a_report_reads_as_buttons_and_channels() {
        // The idle report the spike recorded: button 8 on, sticks centred, throttle low.
        let r = report(0x80, [1024, 1024, 7, 1024, 0, 0, 0, 0]);
        assert_eq!(r.len(), REPORT_LEN);
        let f = frame(5, &r).unwrap();
        assert_eq!(f.buttons, 0x80);
        assert_eq!(f.channels, vec![1500, 1500, 991, 1500, 988, 988, 988, 988]);
        assert_eq!(axis_us(AXIS_MAX), 2012);
        assert_eq!(axis_us(4000), 2012, "out of range is clamped");
        assert!(
            parse_report(&r[..18]).is_none(),
            "a short report is not read"
        );
    }

    #[test]
    fn snapshot_without_a_radio_says_why() {
        let s = snapshot(&NoHid, Duration::from_millis(10)).unwrap();
        assert!(!s.connected && s.frame.is_none());
        assert!(s.message.unwrap().contains("USB Joystick"));
        let f = FakeHid::plugged();
        f.push(report(0, [2048, 0, 0, 0, 0, 0, 0, 0]));
        let s = snapshot(&f, Duration::from_millis(200)).unwrap();
        assert_eq!(s.frame.unwrap().channels[0], 2012);
    }

    #[test]
    fn watch_streams_changes_and_survives_a_replug() {
        let f = FakeHid::plugged();
        let stop = Arc::new(AtomicBool::new(false));
        let events = Arc::new(Mutex::new(Vec::<RadioEvent>::new()));
        let (f2, stop2, ev2) = (f.clone(), stop.clone(), events.clone());
        let t = std::thread::spawn(move || watch(&f2, &stop2, |e| ev2.lock().unwrap().push(e)));
        let wait_for = |pred: &dyn Fn(&[RadioEvent]) -> bool| {
            let end = Instant::now() + Duration::from_secs(5);
            while Instant::now() < end {
                if pred(&events.lock().unwrap()) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            false
        };
        assert!(wait_for(&|e| e.iter().any(|x| x.connected)));
        for _ in 0..5 {
            f.push(report(1, [1024; 8]));
        }
        f.push(report(1, [2048, 1024, 0, 1024, 0, 0, 0, 0]));
        assert!(wait_for(&|e| e
            .iter()
            .filter_map(|x| x.frame.as_ref())
            .any(|fr| fr.channels[0] == 2012)));
        let frames = events
            .lock()
            .unwrap()
            .iter()
            .filter(|x| x.frame.is_some())
            .count();
        assert!(frames <= 3, "same reports are not sent again ({frames})");
        f.unplug();
        assert!(wait_for(&|e| e.last().is_some_and(|x| !x.connected)));
        f.plug();
        assert!(wait_for(&|e| e.last().is_some_and(|x| x.connected)));
        assert!(f.opens() >= 2);
        stop.store(true, Ordering::SeqCst);
        t.join().unwrap();
    }
}
