//! Scratch: grid the sim PID gains against the rate checks.
use quadcam_sim::log::{read, Scales};
use quadcam_sim::validate::validate;
use std::path::Path;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let base = quadcam_sim::preset(&a[0]).unwrap();
    let axes: Vec<usize> = a[1].split(',').map(|x| x.parse().unwrap()).collect();
    let logs: Vec<_> = a[5..]
        .iter()
        .map(|f| {
            read(
                Path::new(f),
                Scales {
                    motor_poles: 12.0,
                    ..Scales::default()
                },
            )
            .unwrap()
        })
        .filter(|l| l.len() > 800)
        .collect();
    let names = ["roll", "pitch", "yaw"];
    let mut res = Vec::new();
    let g = |i: usize| -> Vec<f64> { a[i].split(',').map(|x| x.parse().unwrap()).collect() };
    let (pg, fg, dg) = (g(2), g(3), g(4));
    let verbose = pg.len() * fg.len() * dg.len() == 1;
    for &ps in &pg {
        for &fs in &fg {
            for &ds in &dg {
                let mut p = base.clone();
                for &ax in &axes {
                    p.fc.gains[ax].p.value *= ps;
                    p.fc.gains[ax].ff.value *= fs;
                    p.fc.gains[ax].d.value *= ds;
                }
                let r = validate(&p, &logs);
                let (mut lag, mut os, mut n, mut fails) = (0.0, 0.0, 0.0, 0);
                for c in &r.checks {
                    if !axes.iter().any(|&ax| c.check == names[ax]) || c.window.is_none() {
                        continue;
                    }
                    if c.measure == "lag" {
                        lag += (c.sim - c.logged).powi(2);
                        n += 1.0;
                    }
                    if c.measure == "overshoot" {
                        os += (c.sim - c.logged).powi(2);
                    }
                    if c.outcome == quadcam_sim::validate::Outcome::Fail {
                        fails += 1;
                    }
                    if verbose {
                        println!(
                            "{} {} {} {:?} log {:.1} sim {:.1} {:?}",
                            c.check, c.measure, c.log, c.window, c.logged, c.sim, c.outcome
                        );
                    }
                }
                res.push(((lag / n).sqrt(), (os / n).sqrt(), fails, ps, fs, ds));
            }
        }
    }
    res.sort_by(|a, b| a.2.cmp(&b.2).then(a.0.total_cmp(&b.0)));
    for r in res.iter().take(10) {
        println!(
            "lag RMS {:5.1} ms  overshoot RMS {:5.1}  fails {:2}  P x{} FF x{} D x{}",
            r.0, r.1, r.2, r.3, r.4, r.5
        );
    }
}
