//! The physics thread (sim-design 2.2): fixed steps that follow the wall clock.
//!
//! A dedicated thread at the highest user QoS sleeps until the next step deadline, then runs
//! every step that is due. When it falls more than 25 ms behind (a stall), it drops the
//! backlog: sim time jumps forward to now, the state does not advance through the gap, and
//! the drop is logged with its size. Wall time is never stretched, so there is no slow
//! motion. The clock is only read here, never inside a step.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::record::{Recorder, Recording};
use crate::ring::RcSource;
use crate::sim::Sim;
use crate::snapshot::{SnapshotPair, SnapshotWriter};

/// A monotonic host clock in nanoseconds.
pub trait Clock: Send + 'static {
    fn now_ns(&self) -> u64;
    fn sleep_until_ns(&self, t_ns: u64);
}

/// The host's clock: [`crate::input::now_ns`] (shared with the radio input) and
/// `mach_wait_until` on macOS.
#[derive(Debug, Clone, Copy)]
pub struct HostClock {
    #[cfg(target_os = "macos")]
    numer: u64,
    #[cfg(target_os = "macos")]
    denom: u64,
}

#[cfg(target_os = "macos")]
mod mach {
    #[repr(C)]
    pub struct TimebaseInfo {
        pub numer: u32,
        pub denom: u32,
    }
    extern "C" {
        pub fn mach_absolute_time() -> u64;
        pub fn mach_wait_until(deadline: u64) -> i32;
        pub fn mach_timebase_info(info: *mut TimebaseInfo) -> i32;
        pub fn pthread_set_qos_class_self_np(qos: u32, relative_priority: i32) -> i32;
        pub fn mach_thread_self() -> u32;
        pub fn thread_policy_set(thread: u32, flavor: u32, info: *const u32, count: u32) -> i32;
    }
    /// `THREAD_TIME_CONSTRAINT_POLICY` and its word count.
    pub const TIME_CONSTRAINT_POLICY: u32 = 2;
    pub const TIME_CONSTRAINT_COUNT: u32 = 4;
    /// `QOS_CLASS_USER_INTERACTIVE`.
    pub const QOS_USER_INTERACTIVE: u32 = 0x21;
}

impl Default for HostClock {
    fn default() -> HostClock {
        HostClock::new()
    }
}

impl HostClock {
    pub fn new() -> HostClock {
        #[cfg(target_os = "macos")]
        {
            let mut tb = mach::TimebaseInfo { numer: 0, denom: 0 };
            // SAFETY: mach_timebase_info writes the struct it is given.
            unsafe { mach::mach_timebase_info(&mut tb) };
            HostClock {
                numer: tb.numer.max(1) as u64,
                denom: tb.denom.max(1) as u64,
            }
        }
        #[cfg(not(target_os = "macos"))]
        HostClock {}
    }
}

impl Clock for HostClock {
    /// The sim's one monotonic clock, the one the HID thread stamps samples with.
    fn now_ns(&self) -> u64 {
        crate::input::now_ns()
    }

    fn sleep_until_ns(&self, t_ns: u64) {
        #[cfg(target_os = "macos")]
        {
            let wait = t_ns.saturating_sub(self.now_ns());
            if wait == 0 {
                return;
            }
            // SAFETY: reads the tick counter, then waits until an absolute tick count.
            unsafe {
                let ticks = (wait as u128 * self.denom as u128 / self.numer as u128) as u64;
                mach::mach_wait_until(mach::mach_absolute_time() + ticks);
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let now = self.now_ns();
            if t_ns > now {
                std::thread::sleep(Duration::from_nanos(t_ns - now));
            }
        }
    }
}

/// Raise the calling thread to the highest user QoS, then ask for the time-constraint
/// (real-time) policy audio threads use: woken every `period_ns`, needing about a fifth of
/// it. A busy machine then cannot starve the physics for tens of milliseconds.
fn set_realtime_qos(clock: &HostClock, period_ns: u64) {
    #[cfg(target_os = "macos")]
    // SAFETY: both calls change only the calling thread's own scheduling.
    unsafe {
        mach::pthread_set_qos_class_self_np(mach::QOS_USER_INTERACTIVE, 0);
        let ticks = |ns: u64| (ns as u128 * clock.denom as u128 / clock.numer as u128) as u32;
        let policy = [ticks(period_ns), ticks(period_ns / 5), ticks(period_ns), 1];
        mach::thread_policy_set(
            mach::mach_thread_self(),
            mach::TIME_CONSTRAINT_POLICY,
            policy.as_ptr(),
            mach::TIME_CONSTRAINT_COUNT,
        );
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (clock, period_ns);
}

/// A dropped backlog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DropEvent {
    /// The step the sim was about to run.
    pub step: u64,
    /// Host time the drop was found (ns).
    pub host_ns: u64,
    /// How far behind the thread was (ns), and the steps that skipped.
    pub behind_ns: u64,
    pub steps: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunnerConfig {
    /// Behind by more than this (ns), drop the backlog. 25 ms by default.
    pub max_backlog_ns: u64,
    /// Run at the highest user QoS.
    pub realtime_qos: bool,
}

impl Default for RunnerConfig {
    fn default() -> RunnerConfig {
        RunnerConfig {
            max_backlog_ns: 25_000_000,
            realtime_qos: true,
        }
    }
}

/// Step cost histogram: 1 µs buckets up to 1 ms, then one overflow bucket.
#[derive(Debug, Clone)]
pub struct CostHistogram {
    pub buckets: Vec<u64>,
    pub max_ns: u64,
}

impl Default for CostHistogram {
    fn default() -> CostHistogram {
        CostHistogram {
            buckets: vec![0; 1001],
            max_ns: 0,
        }
    }
}

impl CostHistogram {
    pub fn add(&mut self, ns: u64) {
        let i = ((ns / 1000) as usize).min(1000);
        self.buckets[i] += 1;
        self.max_ns = self.max_ns.max(ns);
    }

    pub fn count(&self) -> u64 {
        self.buckets.iter().sum()
    }

    /// The `q` quantile (0..1), in µs (bucket upper edge).
    pub fn quantile_us(&self, q: f64) -> f64 {
        let n = self.count();
        if n == 0 {
            return 0.0;
        }
        let want = (q * n as f64).ceil() as u64;
        let mut acc = 0;
        for (i, c) in self.buckets.iter().enumerate() {
            acc += c;
            if acc >= want {
                return (i + 1) as f64;
            }
        }
        1001.0
    }

    pub fn merge(&mut self, o: &CostHistogram) {
        for (a, b) in self.buckets.iter_mut().zip(&o.buckets) {
            *a += b;
        }
        self.max_ns = self.max_ns.max(o.max_ns);
    }
}

#[derive(Debug, Clone, Default)]
pub struct RunnerStats {
    pub steps: u64,
    pub dropped_steps: u64,
    pub drops: Vec<DropEvent>,
    pub cost: CostHistogram,
}

struct Shared {
    steps: AtomicU64,
    dropped: AtomicU64,
    drops: Mutex<Vec<DropEvent>>,
    cost: Mutex<CostHistogram>,
    stop: AtomicBool,
}

/// Messages to the physics thread.
pub enum Control {
    /// Back to the start pad.
    Reset,
    /// Test hook: block the thread this long, as a stall would.
    Stall(Duration),
}

pub struct Runner {
    handle: Option<JoinHandle<(Sim, Option<Recording>)>>,
    tx: Sender<Control>,
    shared: Arc<Shared>,
}

/// Start the physics thread. `on_drop` is called (from the physics thread) for each dropped
/// backlog, for the host's log.
pub fn spawn<C: Clock>(
    sim: Sim,
    input: impl RcSource + 'static,
    out: SnapshotWriter,
    clock: C,
    cfg: RunnerConfig,
    recorder: Option<Recorder>,
    on_drop: Option<Box<dyn Fn(DropEvent) + Send>>,
) -> Runner {
    let (tx, rx) = channel();
    let shared = Arc::new(Shared {
        steps: AtomicU64::new(0),
        dropped: AtomicU64::new(0),
        drops: Mutex::new(Vec::new()),
        cost: Mutex::new(CostHistogram::default()),
        stop: AtomicBool::new(false),
    });
    let sh = shared.clone();
    let handle = std::thread::Builder::new()
        .name("quadcam-sim physics".into())
        .spawn(move || run(sim, input, out, clock, cfg, recorder, on_drop, rx, sh))
        .expect("spawn the physics thread");
    Runner {
        handle: Some(handle),
        tx,
        shared,
    }
}

#[allow(clippy::too_many_arguments)]
fn run<C: Clock>(
    mut sim: Sim,
    mut input: impl RcSource,
    mut out: SnapshotWriter,
    clock: C,
    cfg: RunnerConfig,
    mut recorder: Option<Recorder>,
    on_drop: Option<Box<dyn Fn(DropEvent) + Send>>,
    rx: Receiver<Control>,
    shared: Arc<Shared>,
) -> (Sim, Option<Recording>) {
    let dt_ns = sim.dt * 1e9;
    if cfg.realtime_qos {
        set_realtime_qos(&HostClock::new(), dt_ns as u64);
    }
    let mut anchor = clock.now_ns();
    let mut k: u64 = 0;
    let deadline = |anchor: u64, k: u64| anchor + (k as f64 * dt_ns).round() as u64;
    let mut prev = sim.snapshot();
    let mut dropped_total = 0u64;
    let mut local_cost = CostHistogram::default();
    let mut last_publish = anchor;
    while !shared.stop.load(Ordering::Relaxed) {
        while let Ok(c) = rx.try_recv() {
            match c {
                Control::Reset => sim.reset(),
                Control::Stall(d) => std::thread::sleep(d),
            }
        }
        let now = clock.now_ns();
        let next = deadline(anchor, k);
        if now > next + cfg.max_backlog_ns {
            let behind = now - next;
            let steps = (behind as f64 / dt_ns) as u64;
            let ev = DropEvent {
                step: sim.step,
                host_ns: now,
                behind_ns: behind,
                steps,
            };
            dropped_total += steps;
            shared.dropped.store(dropped_total, Ordering::Relaxed);
            if let Ok(mut d) = shared.drops.lock() {
                d.push(ev);
            }
            if let Some(f) = &on_drop {
                f(ev);
            }
            anchor = now;
            k = 0;
        }
        loop {
            let t = deadline(anchor, k);
            if t > now {
                break;
            }
            let c0 = clock.now_ns();
            let f = input.frame_at(t);
            if let Some(r) = recorder.as_mut() {
                r.push(sim.step, &f);
            }
            sim.step(&f);
            let mut snap = sim.snapshot();
            snap.host_ns = t;
            snap.dropped_steps = dropped_total;
            out.write(SnapshotPair { prev, cur: snap });
            prev = snap;
            local_cost.add(clock.now_ns().saturating_sub(c0));
            k += 1;
            shared.steps.fetch_add(1, Ordering::Relaxed);
        }
        if now.saturating_sub(last_publish) > 100_000_000 {
            if let Ok(mut c) = shared.cost.try_lock() {
                c.merge(&local_cost);
                local_cost = CostHistogram::default();
                last_publish = now;
            }
        }
        clock.sleep_until_ns(deadline(anchor, k));
    }
    if let Ok(mut c) = shared.cost.lock() {
        c.merge(&local_cost);
    }
    let rec = recorder.map(|r| r.finish(&sim));
    (sim, rec)
}

impl Runner {
    pub fn send(&self, c: Control) {
        let _ = self.tx.send(c);
    }

    pub fn stats(&self) -> RunnerStats {
        RunnerStats {
            steps: self.shared.steps.load(Ordering::Relaxed),
            dropped_steps: self.shared.dropped.load(Ordering::Relaxed),
            drops: self
                .shared
                .drops
                .lock()
                .map(|d| d.clone())
                .unwrap_or_default(),
            cost: self
                .shared
                .cost
                .lock()
                .map(|c| c.clone())
                .unwrap_or_default(),
        }
    }

    /// Stop the thread; returns the sim and the recording, if one was kept.
    pub fn stop(mut self) -> (Sim, Option<Recording>) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.handle
            .take()
            .expect("runner joined once")
            .join()
            .expect("physics thread panicked")
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_quantiles() {
        let mut h = CostHistogram::default();
        for i in 0..100 {
            h.add(i * 1000 + 500);
        }
        assert_eq!(h.quantile_us(0.5), 50.0);
        assert_eq!(h.quantile_us(0.99), 99.0);
        h.add(5_000_000);
        assert_eq!(h.quantile_us(1.0), 1001.0);
    }

    #[test]
    fn host_clock_is_monotonic_and_sleeps() {
        let c = HostClock::new();
        let a = c.now_ns();
        c.sleep_until_ns(a + 2_000_000);
        let b = c.now_ns();
        assert!(b >= a + 2_000_000, "{}", b - a);
        assert!(b < a + 50_000_000);
    }
}
