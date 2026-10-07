//! The headless bench (not CI): step cost in free flight and in contact, and a real-time
//! run on the physics thread that should drop no steps after its first second.
//!
//! cargo run -p quadcam-sim --example bench [-- <real-time seconds, default 600>]

use std::time::Duration;

use quadcam_sim::bench;
use quadcam_sim::rapier3d_f64::glamx::DQuat;
use quadcam_sim::ring::{FakeRc, RcFrame, Sticks};
use quadcam_sim::runner::{spawn, HostClock, RunnerConfig};
use quadcam_sim::snapshot::snapshot_buffer;
use quadcam_sim::{preset, Sim, SimSettings, WorldSpec};

fn main() {
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(600);
    for id in ["meteor75", "air65ii", "five_inch", "seven_inch"] {
        let f = bench::free_flight(id, 200_000);
        let (c, frac) = bench::contact(id, 200_000);
        println!(
            "{id:10} free p50 {:4.0} p99 {:4.0} max {:6.1} us | contact ({:.0} % touching) p50 {:4.0} p99 {:4.0} max {:6.1} us",
            f.quantile_us(0.5),
            f.quantile_us(0.99),
            f.max_ns as f64 / 1e3,
            frac * 100.0,
            c.quantile_us(0.5),
            c.quantile_us(0.99),
            c.max_ns as f64 / 1e3
        );
    }

    let mut sim = Sim::new(
        &preset("meteor75").unwrap(),
        WorldSpec::plain_room(6.0, 6.0, 3.0),
        SimSettings::default(),
        1,
    );
    sim.set_body([0.0, 0.0, 1.0], DQuat::IDENTITY, [0.0; 3], [0.0; 3]);
    let input = FakeRc::constant(RcFrame::from_sticks(Sticks::default(), &[1000]));
    let (w, _r) = snapshot_buffer();
    let runner = spawn(
        sim,
        input,
        w,
        HostClock::new(),
        RunnerConfig::default(),
        None,
        Some(Box::new(|e| {
            eprintln!(
                "dropped {} steps ({:.1} ms behind) at step {}",
                e.steps,
                e.behind_ns as f64 / 1e6,
                e.step
            )
        })),
    );
    std::thread::sleep(Duration::from_secs(1));
    let first = runner.stats();
    std::thread::sleep(Duration::from_secs(secs.saturating_sub(1)));
    let stats = runner.stats();
    drop(runner);
    println!(
        "real time {secs} s: {} steps ({:.1} per s), dropped {} in the first second, {} after; step p99 {:.0} us, max {:.1} us",
        stats.steps,
        stats.steps as f64 / secs as f64,
        first.dropped_steps,
        stats.dropped_steps - first.dropped_steps,
        stats.cost.quantile_us(0.99),
        stats.cost.max_ns as f64 / 1e3
    );
}
