//! The validation harness (sim-design 6.4; S2 acceptance): every check runs on the CI
//! fixtures and the 75 mm and 65 mm example profiles pass their bands; the scrub test;
//! and the harness's consistency on a log the sim wrote itself.

use std::path::Path;

use quadcam_sim::log::{check_scrubbed, read_folder, Scales};
use quadcam_sim::preset;
use quadcam_sim::ring::Sticks;
use quadcam_sim::validate::{synth_log, validate, HoverPilot, Outcome, ValidationReport};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn checks_run(r: &ValidationReport) -> Vec<String> {
    let mut v: Vec<String> = r
        .checks
        .iter()
        .filter(|c| c.outcome != Outcome::NotInLog)
        .map(|c| c.check.clone())
        .collect();
    v.sort();
    v.dedup();
    v
}

#[test]
fn harness_is_consistent_on_a_sim_written_log() {
    for id in ["meteor75", "air65ii"] {
        let p = preset(id).unwrap();
        let mut pilot = HoverPilot::new(100.0, 0.3);
        let log = synth_log(&p, 13.0, |sim, t| {
            let hold = pilot.throttle(sim);
            let s = |throttle, roll, pitch, yaw| Sticks {
                throttle,
                roll,
                pitch,
                yaw,
            };
            match t {
                t if t < 3.0 => (s(hold, 0.0, 0.0, 0.0), true),
                // A straight punch.
                t if t < 3.4 => (s(0.95, 0.0, 0.0, 0.0), true),
                t if t < 5.0 => (s(hold, 0.0, 0.0, 0.0), true),
                // Chop, fall, punch out.
                t if t < 5.35 => (s(0.0, 0.0, 0.0, 0.0), true),
                t if t < 5.9 => (s(0.85, 0.0, 0.0, 0.0), true),
                t if t < 7.0 => (s(hold, 0.0, 0.0, 0.0), true),
                // A dash forward, then a level coast.
                t if t < 8.2 => (s(hold.max(0.45), 0.0, 0.6, 0.0), true),
                t if t < 9.2 => (s(hold, 0.0, 0.0, 0.0), true),
                // Acro: roll, pitch and yaw moves.
                t => {
                    let w = std::f64::consts::TAU * 1.5 * t;
                    (
                        s(
                            hold,
                            0.8 * w.sin(),
                            0.8 * (1.3 * w).sin(),
                            0.8 * (0.7 * w).sin(),
                        ),
                        false,
                    )
                }
            }
        });
        let r = validate(&p, &[log]);
        print!("{}", r.table());
        assert!(r.passed(), "{id}: {:?}", r.failures());
        let ran = checks_run(&r);
        for c in [
            "coast_down",
            "fall_recovery",
            "hover",
            "pitch",
            "punch",
            "roll",
            "sag",
            "yaw",
        ] {
            assert!(
                ran.contains(&c.to_string()),
                "{id}: {c} did not run; ran {ran:?}"
            );
        }
    }
}

fn fixtures(id: &str) -> Vec<quadcam_sim::log::LogData> {
    read_folder(&Path::new(FIXTURES).join(id), Scales::default()).unwrap()
}

#[test]
fn example_profiles_pass_on_the_ci_fixtures() {
    for id in ["meteor75", "air65ii"] {
        let logs = fixtures(id);
        assert!(logs.len() >= 5, "{id}: {} fixtures", logs.len());
        for l in &logs {
            assert_eq!(
                l.meta.get("profile").map(String::as_str),
                Some(id),
                "{}",
                l.name
            );
        }
        let r = validate(&preset(id).unwrap(), &logs);
        print!("{}", r.table());
        assert!(r.passed(), "{id}: {:?}", r.failures());
        let ran = checks_run(&r);
        // Coast-down and fall recovery need a speed column and a clean recovery, which the
        // example logs do not hold; the sim-written log above runs them.
        for c in ["hover", "pitch", "punch", "roll", "sag", "yaw"] {
            assert!(
                ran.contains(&c.to_string()),
                "{id}: {c} did not run; ran {ran:?}"
            );
        }
    }
}

#[test]
fn fixtures_are_scrubbed_and_small() {
    let mut total = 0;
    for dir in std::fs::read_dir(FIXTURES).unwrap() {
        let dir = dir.unwrap().path();
        for f in std::fs::read_dir(&dir).unwrap() {
            let f = f.unwrap().path();
            let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
            assert_eq!(
                ext,
                "csv",
                "{}: only decoded CSV excerpts, never raw logs",
                f.display()
            );
            let text = std::fs::read_to_string(&f).unwrap();
            total += text.len();
            if let Err(e) = check_scrubbed(&text) {
                panic!("{}: {e}", f.display());
            }
        }
    }
    assert!(total < 1_000_000, "fixtures total {total} bytes");
    // The scrub check fails on a planted UID.
    let one = std::fs::read_to_string(Path::new(FIXTURES).join("meteor75/hover.csv")).unwrap();
    let planted = one.replacen("\ntime", "\n# board_uid: 3A0047001851393436383537\ntime", 1);
    assert!(check_scrubbed(&planted).is_err());
    let planted = format!("{one}0,1,2,3,0036003A3133510D37363435\n");
    assert!(check_scrubbed(&planted).is_err());
}
