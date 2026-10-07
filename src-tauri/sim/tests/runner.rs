//! The physics thread: real time, never slow motion; a stall drops its backlog and logs it;
//! a recording made on the thread replays exactly (sim-design 2.2, 2.3; S1 acceptance).

mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use quadcam_sim::rapier3d_f64::glamx::DQuat;
use quadcam_sim::record::Recorder;
use quadcam_sim::ring::{rc_ring, FakeRc};
use quadcam_sim::runner::{spawn, Clock, Control, DropEvent, HostClock, RunnerConfig};
use quadcam_sim::snapshot::snapshot_buffer;
use quadcam_sim::{preset, Sim, SimSettings, WorldSpec};

fn hovering_sim() -> Sim {
    let mut s = Sim::new(
        &preset("meteor75").unwrap(),
        WorldSpec::plain_room(6.0, 6.0, 3.0),
        SimSettings::default(),
        1,
    );
    s.set_body([0.0, 0.0, 1.0], DQuat::IDENTITY, [0.0; 3], [0.0; 3]);
    s
}

#[test]
fn a_forced_stall_drops_sim_time_and_logs_it() {
    let (w, mut r) = snapshot_buffer();
    let clock = HostClock::new();
    let log: Arc<Mutex<Vec<DropEvent>>> = Arc::default();
    let l2 = log.clone();
    let input = FakeRc::constant(rc(sticks(0.0, 0.0, 0.0, 0.0), Switches::default()));
    let runner = spawn(
        hovering_sim(),
        input,
        w,
        clock,
        RunnerConfig::default(),
        None,
        Some(Box::new(move |e| l2.lock().unwrap().push(e))),
    );
    let t0 = clock.now_ns();
    std::thread::sleep(Duration::from_millis(300));
    runner.send(Control::Stall(Duration::from_millis(100)));
    std::thread::sleep(Duration::from_millis(700));
    let now = clock.now_ns();
    let snap = r.read().cur;
    let stats = runner.stats();
    let (sim, _) = runner.stop();
    let wall = (now - t0) as f64 / 1e9;
    println!(
        "stall: {:.3} s wall, {} steps, {} dropped, drops {:?}",
        wall, stats.steps, stats.dropped_steps, stats.drops
    );
    // One drop of at least the 100 ms stall (a busy machine oversleeps), its steps the gap
    // at 2 kHz, logged through the callback.
    let drops: Vec<_> = stats
        .drops
        .iter()
        .filter(|d| d.behind_ns > 50_000_000)
        .collect();
    assert_eq!(drops.len(), 1, "{:?}", stats.drops);
    let behind = drops[0].behind_ns;
    assert!(behind >= 99_000_000, "{behind}");
    assert!((drops[0].steps as f64 - behind as f64 / 500_000.0).abs() <= 1.0);
    assert_eq!(log.lock().unwrap().len(), stats.drops.len());
    // No slow motion: sim time skipped the gap (steps ≈ wall − gap), and the newest
    // snapshot stands for "now", not for a moment the stall ago.
    let expect = (wall - behind as f64 / 1e9) * 2000.0;
    assert!(
        ((stats.steps as f64) - expect).abs() < 0.05 * expect,
        "{} steps, expected {expect:.0}",
        stats.steps
    );
    assert!(
        now.saturating_sub(snap.host_ns) < 5_000_000,
        "{} ns behind",
        now - snap.host_ns
    );
    assert_eq!(snap.dropped_steps, stats.dropped_steps);
    assert!(sim.step >= stats.steps);
}

/// A clock the test drives: `sleep_until` jumps to the deadline; a stall jumps further.
#[derive(Clone)]
struct ScriptClock {
    now: Arc<AtomicU64>,
    stall_at: u64,
    stall_ns: u64,
    stop_at: u64,
    stop: Arc<AtomicU64>,
}

impl Clock for ScriptClock {
    fn now_ns(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
    fn sleep_until_ns(&self, t: u64) {
        let mut t = t.max(self.now_ns());
        if t >= self.stall_at && self.now_ns() < self.stall_at {
            t += self.stall_ns;
        }
        self.now.store(t, Ordering::SeqCst);
        if t >= self.stop_at {
            self.stop.store(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(1000));
        }
    }
}

#[test]
fn drop_logic_on_a_scripted_clock() {
    // 1 s of sim with a 100 ms stall at 0.5 s: exactly 200 steps dropped, 1800 run.
    let stop = Arc::new(AtomicU64::new(0));
    let clock = ScriptClock {
        now: Arc::new(AtomicU64::new(1_000_000_000)),
        stall_at: 1_500_000_000,
        stall_ns: 100_000_000,
        stop_at: 2_000_000_000,
        stop: stop.clone(),
    };
    let (w, _r) = snapshot_buffer();
    let input = FakeRc::constant(rc(sticks(0.0, 0.0, 0.0, 0.0), Switches::default()));
    let cfg = RunnerConfig {
        realtime_qos: false,
        ..RunnerConfig::default()
    };
    let runner = spawn(hovering_sim(), input, w, clock.clone(), cfg, None, None);
    while stop.load(Ordering::SeqCst) == 0 {
        std::thread::sleep(Duration::from_millis(5));
    }
    let stats = runner.stats();
    drop(runner);
    assert_eq!(stats.drops.len(), 1, "{:?}", stats.drops);
    assert_eq!(stats.drops[0].behind_ns, 100_000_000);
    assert_eq!(stats.drops[0].steps, 200);
    assert!((1800..=1802).contains(&stats.steps), "{}", stats.steps);
}

#[test]
fn a_recording_from_the_thread_replays_exactly() {
    let (mut tx, rx) = rc_ring(4096);
    let (w, _r) = snapshot_buffer();
    let clock = HostClock::new();
    let profile = preset("air65ii").unwrap();
    let world = WorldSpec::plain_room(6.0, 6.0, 3.0);
    let settings = SimSettings::default();
    let mut sim = Sim::new(&profile, world.clone(), settings.clone(), 9);
    sim.set_body([0.0, 0.0, 1.5], DQuat::IDENTITY, [0.0; 3], [0.0; 3]);
    // The replay starts from the same placed body.
    let rec = Recorder::new(&profile, &world, &settings, 9);
    let runner = spawn(sim, rx, w, clock, RunnerConfig::default(), Some(rec), None);
    // An input thread at 500 Hz: arm, then wiggle the sticks.
    let producer = std::thread::spawn(move || {
        for k in 0..300u64 {
            let t = k as f64 * 0.002;
            let sw = Switches {
                arm: t > 0.05,
                angle: true,
                turtle: false,
            };
            let mut f = rc(
                sticks(0.35, (t * 7.0).sin() * 0.3, (t * 5.0).cos() * 0.2, 0.0),
                sw,
            );
            f.t_ns = clock.now_ns();
            tx.push(f);
            std::thread::sleep(Duration::from_millis(2));
        }
    });
    producer.join().unwrap();
    let (sim, rec) = runner.stop();
    let rec = rec.unwrap();
    assert!(rec.frames.len() > 50, "{} input changes", rec.frames.len());
    let mut replay = rec.sim();
    replay.set_body([0.0, 0.0, 1.5], DQuat::IDENTITY, [0.0; 3], [0.0; 3]);
    for f in rec.frames_by_step() {
        replay.step(&f);
    }
    assert_eq!(replay.step, sim.step);
    assert_eq!(replay.state_hash(), sim.state_hash());
}
