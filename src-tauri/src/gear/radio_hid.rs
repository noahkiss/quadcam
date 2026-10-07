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
    /// The firmware version the radio reports over USB.
    #[serde(default)]
    pub version: Option<String>,
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
    /// The firmware version from the USB device (`bcdDevice`, `2.12`), when it says.
    fn version(&self) -> Option<String> {
        None
    }
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
    fn version(&self) -> Option<String> {
        Some("2.12".into())
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
        let Some(release) = api
            .device_list()
            .find(|d| d.vendor_id() == VID && d.product_id() == PID)
            .map(|d| d.release_number())
        else {
            return Ok(None);
        };
        let dev = api.open(VID, PID)?;
        let product = dev
            .get_product_string()
            .ok()
            .flatten()
            .unwrap_or_else(|| "USB radio".into());
        // bcdDevice is BCD: 0x0212 is 2.12.
        let version = Some(format!("{:x}.{:02x}", release >> 8, release & 0xff));
        Ok(Some(Box::new(RealReader {
            dev,
            product,
            version,
        })))
    }
}

#[cfg(target_os = "macos")]
struct RealReader {
    dev: hidapi::HidDevice,
    product: String,
    version: Option<String>,
}

#[cfg(target_os = "macos")]
impl HidReader for RealReader {
    fn product(&self) -> String {
        self.product.clone()
    }
    fn version(&self) -> Option<String> {
        self.version.clone()
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
            version: None,
            frame: None,
            message: Some(
                "No radio in USB Joystick mode. Plug it in and choose USB Joystick on the radio."
                    .into(),
            ),
        });
    };
    let product = r.product();
    let version = r.version();
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
        version,
        frame: last,
        message,
    })
}

/// What the reader sees.
pub enum Signal<'a> {
    /// No radio to open; the reader tries again after `RETRY`.
    Absent,
    /// Opened: the product name.
    Connected(&'a str),
    /// One report, stamped with `quadcam_sim::input::now_ns` as it arrived.
    Report { t_ns: u64, report: &'a [u8] },
    /// A read timed out with no report.
    Idle,
    /// The radio went away.
    Gone,
}

/// Reads the radio until `stop`: every report, stamped on arrival, and a reopen after the
/// radio goes. Every consumer (the throttled page stream, the sim's unthrottled sinks) sees
/// the same reports through `on`.
pub fn read_loop(src: &dyn HidSource, stop: &AtomicBool, mut on: impl FnMut(Signal)) {
    while !stop.load(Ordering::SeqCst) {
        let Some(mut r) = src.open().ok().flatten() else {
            on(Signal::Absent);
            sleep_unless(stop, RETRY);
            continue;
        };
        on(Signal::Connected(&r.product()));
        while !stop.load(Ordering::SeqCst) {
            match r.read(Duration::from_millis(20)) {
                Ok(Some(rep)) => on(Signal::Report {
                    t_ns: quadcam_sim::input::now_ns(),
                    report: &rep,
                }),
                Ok(None) => on(Signal::Idle),
                Err(_) => break,
            }
        }
        if !stop.load(Ordering::SeqCst) {
            on(Signal::Gone);
            sleep_unless(stop, Duration::from_millis(200));
        }
    }
}

/// A report as the sim's sample. The parsing is `parse_report`'s, as for a frame.
pub fn sample(t_ns: u64, report: &[u8]) -> Option<quadcam_sim::input::InputSample> {
    let (buttons, axes) = parse_report(report)?;
    Some(quadcam_sim::input::InputSample {
        t_ns,
        axes,
        buttons,
    })
}

/// The page's stream from the reader: a `connected: false` event while there is none,
/// each changed frame (at most one per `FRAME_INTERVAL`), a heartbeat every `HEARTBEAT`.
#[derive(Default)]
pub struct Throttle {
    seq: u64,
    said_absent: bool,
    product: Option<String>,
    sent: Option<(Instant, Vec<u8>)>,
    pending: Option<Vec<u8>>,
}

impl Throttle {
    pub fn on(&mut self, sig: &Signal, emit: &mut impl FnMut(RadioEvent)) {
        match sig {
            Signal::Absent | Signal::Gone => {
                if !self.said_absent || matches!(sig, Signal::Gone) {
                    emit(RadioEvent {
                        connected: false,
                        product: None,
                        frame: None,
                    });
                    self.said_absent = true;
                }
                self.sent = None;
                self.pending = None;
                return;
            }
            Signal::Connected(p) => {
                self.said_absent = false;
                self.product = Some(p.to_string());
                emit(RadioEvent {
                    connected: true,
                    product: self.product.clone(),
                    frame: None,
                });
                return;
            }
            Signal::Report { report, .. } => {
                self.seq += 1;
                if self.sent.as_ref().is_none_or(|(_, last)| last != report) {
                    self.pending = Some(report.to_vec());
                }
            }
            Signal::Idle => {}
        }
        let due = match &self.sent {
            None => self.pending.is_some(),
            Some((t, _)) => {
                (self.pending.is_some() && t.elapsed() >= FRAME_INTERVAL)
                    || t.elapsed() >= HEARTBEAT
            }
        };
        if due {
            let rep = self
                .pending
                .take()
                .or_else(|| self.sent.as_ref().map(|(_, l)| l.clone()));
            if let Some(rep) = rep {
                if let Some(f) = frame(self.seq, &rep) {
                    emit(RadioEvent {
                        connected: true,
                        product: self.product.clone(),
                        frame: Some(f),
                    });
                }
                self.sent = Some((Instant::now(), rep));
            }
        }
    }
}

/// Streams the radio until `stop`, throttled for the page (`Throttle`).
pub fn watch(src: &dyn HidSource, stop: &AtomicBool, mut emit: impl FnMut(RadioEvent)) {
    let mut th = Throttle::default();
    read_loop(src, stop, |sig| th.on(&sig, &mut emit));
}

/// What an unthrottled sink receives.
#[derive(Debug, Clone, PartialEq)]
pub enum Feed {
    Connected(String),
    /// Every report, with its arrival time.
    Sample(quadcam_sim::input::InputSample),
    Gone,
}

pub type Sink = Arc<dyn Fn(&Feed) + Send + Sync>;
pub type Emit = Arc<dyn Fn(RadioEvent) + Send + Sync>;

/// One reader thread for every consumer of the radio: the page's throttled stream and any
/// number of unthrottled, timestamped sinks (the sim's input ring, the calibration). It runs
/// while anyone listens.
pub struct Hub {
    src: Mutex<Arc<dyn HidSource>>,
    st: Mutex<HubState>,
}

#[derive(Default)]
struct HubState {
    thread: Option<(Arc<AtomicBool>, std::thread::JoinHandle<()>)>,
    watch: Option<Emit>,
    sinks: Vec<(u64, Sink)>,
    next: u64,
}

/// Drop it to stop receiving.
pub struct Subscription {
    hub: Arc<Hub>,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.hub
            .st
            .lock()
            .unwrap()
            .sinks
            .retain(|(i, _)| *i != self.id);
        self.hub.stop_if_idle();
    }
}

impl Hub {
    pub fn new(src: Arc<dyn HidSource>) -> Arc<Self> {
        Arc::new(Self {
            src: Mutex::new(src),
            st: Mutex::new(HubState::default()),
        })
    }

    pub fn source(&self) -> Arc<dyn HidSource> {
        self.src.lock().unwrap().clone()
    }

    pub fn set_source(&self, src: Arc<dyn HidSource>) {
        *self.src.lock().unwrap() = src;
    }

    /// Starts or stops the page's stream. True while it runs.
    pub fn watch(self: &Arc<Self>, on: bool, emit: Emit) -> Result<bool> {
        self.st.lock().unwrap().watch = on.then_some(emit);
        if on {
            self.ensure_running()?;
        } else {
            self.stop_if_idle();
        }
        Ok(on)
    }

    /// Every report from now on, unthrottled, until the subscription drops.
    pub fn subscribe(self: &Arc<Self>, sink: Sink) -> Result<Subscription> {
        let id = {
            let mut st = self.st.lock().unwrap();
            st.next += 1;
            let id = st.next;
            st.sinks.push((id, sink));
            id
        };
        let sub = Subscription {
            hub: self.clone(),
            id,
        };
        self.ensure_running()?;
        Ok(sub)
    }

    /// Every report into a sim input ring.
    pub fn subscribe_ring(
        self: &Arc<Self>,
        ring: Arc<quadcam_sim::input::InputRing>,
    ) -> Result<Subscription> {
        self.subscribe(Arc::new(move |f| {
            if let Feed::Sample(s) = f {
                ring.push(*s);
            }
        }))
    }

    fn ensure_running(self: &Arc<Self>) -> Result<()> {
        let mut st = self.st.lock().unwrap();
        if st.thread.as_ref().is_some_and(|(_, h)| !h.is_finished()) {
            return Ok(());
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (hub, s2) = (Arc::downgrade(self), stop.clone());
        let src = self.source();
        let h = std::thread::Builder::new()
            .name("radio-hid".into())
            .spawn(move || {
                let mut th = Throttle::default();
                read_loop(src.as_ref(), &s2, |sig| {
                    let Some(hub) = hub.upgrade() else {
                        s2.store(true, Ordering::SeqCst);
                        return;
                    };
                    let st = hub.st.lock().unwrap();
                    if let Some(w) = &st.watch {
                        th.on(&sig, &mut |e| w(e));
                    }
                    let feed = match sig {
                        Signal::Connected(p) => Feed::Connected(p.to_string()),
                        Signal::Report { t_ns, report } => match sample(t_ns, report) {
                            Some(s) => Feed::Sample(s),
                            None => return,
                        },
                        Signal::Gone => Feed::Gone,
                        Signal::Absent | Signal::Idle => return,
                    };
                    for (_, k) in &st.sinks {
                        k(&feed);
                    }
                })
            })?;
        st.thread = Some((stop, h));
        Ok(())
    }

    fn stop_if_idle(&self) {
        let t = {
            let mut st = self.st.lock().unwrap();
            if st.watch.is_some() || !st.sinks.is_empty() {
                return;
            }
            st.thread.take()
        };
        if let Some((stop, h)) = t {
            stop.store(true, Ordering::SeqCst);
            let _ = h.join();
        }
    }

    /// Whether the reader thread runs.
    pub fn running(&self) -> bool {
        self.st
            .lock()
            .unwrap()
            .thread
            .as_ref()
            .is_some_and(|(_, h)| !h.is_finished())
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
    fn every_report_reaches_the_ring_with_its_time() {
        let f = FakeHid::plugged();
        let hub = Hub::new(Arc::new(f.clone()));
        let ring = Arc::new(quadcam_sim::input::InputRing::new(1024));
        let frames = Arc::new(Mutex::new(0usize));
        let fr = frames.clone();
        hub.watch(
            true,
            Arc::new(move |e: RadioEvent| {
                if e.frame.is_some() {
                    *fr.lock().unwrap() += 1;
                }
            }),
        )
        .unwrap();
        let sub = hub.subscribe_ring(ring.clone()).unwrap();
        let before = quadcam_sim::input::now_ns();
        for i in 0..500u16 {
            f.push(report(i as u32, [i, 2048 - i, 0, 1024, 0, 0, 0, 0]));
        }
        let end = Instant::now() + Duration::from_secs(5);
        while ring.pushed() < 500 && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(5));
        }
        let (got, _) = ring.since(0);
        assert_eq!(got.len(), 500, "every report, none dropped or merged");
        for (i, s) in got.iter().enumerate() {
            assert_eq!(s.axes[0], i as u16);
            assert_eq!(s.buttons, i as u32);
            assert!(s.t_ns >= before);
        }
        assert!(got.windows(2).all(|w| w[0].t_ns <= w[1].t_ns));
        let n = *frames.lock().unwrap();
        assert!(n < 100, "the page stream stays throttled ({n})");
        // The page stream stops; the sink keeps the reader running until it drops.
        hub.watch(false, Arc::new(|_| {})).unwrap();
        assert!(hub.running());
        drop(sub);
        assert!(!hub.running());
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
