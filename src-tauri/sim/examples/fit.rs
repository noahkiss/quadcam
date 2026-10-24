//! Fit a sim profile from blackbox logs and print the report and its validation.
//!
//! `cargo run --release -p quadcam-sim --example fit -- <base> <logs> [--diff F] [--mass-g N]
//! [--radio CSV]... [--out profile.json]`

use quadcam_sim::fit::{fit, FitInputs, RadioFlight};
use quadcam_sim::log::{read_path, Scales};
use quadcam_sim::validate::validate;

fn radio(path: &str) -> Vec<RadioFlight> {
    // EdgeTX CSV: `RxBt(V)` and `Curr(A)`, one flight per stretch of non-zero RxBt.
    let text = std::fs::read_to_string(path).expect("read the radio log");
    let mut lines = text.lines();
    let head: Vec<&str> = lines.next().unwrap_or("").split(',').collect();
    let col = |n: &str| head.iter().position(|c| *c == n);
    let (Some(iv), Some(ia)) = (col("RxBt(V)"), col("Curr(A)")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cur = RadioFlight {
        name: path.to_string(),
        ..Default::default()
    };
    for (k, l) in lines.enumerate() {
        let f: Vec<&str> = l.split(',').collect();
        let v: f64 = f.get(iv).and_then(|x| x.parse().ok()).unwrap_or(0.0);
        let a: f64 = f.get(ia).and_then(|x| x.parse().ok()).unwrap_or(0.0);
        if v > 0.5 && a > 0.2 {
            cur.t.push(k as f64 / 10.0);
            cur.volts.push(v);
            cur.amps.push(a);
        } else if !cur.t.is_empty() {
            out.push(std::mem::take(&mut cur));
            cur.name = path.to_string();
        }
    }
    out.push(cur);
    out
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let base = quadcam_sim::preset(&a[1]).expect("a preset id");
    let mut inputs = FitInputs {
        id: format!("{}-fitted", base.id),
        label: format!("{} (fitted)", base.label),
        ..Default::default()
    };
    let mut out = None;
    let mut i = 3;
    while i < a.len() {
        match a[i].as_str() {
            "--diff" => inputs.diff = Some(std::fs::read_to_string(&a[i + 1]).unwrap()),
            "--mass-g" => inputs.mass = Some(a[i + 1].parse::<f64>().unwrap() / 1000.0),
            "--radio" => inputs.radio.extend(radio(&a[i + 1])),
            "--out" => out = Some(a[i + 1].clone()),
            x => panic!("unknown {x}"),
        }
        i += 2;
    }
    let poles = inputs
        .diff
        .as_deref()
        .map(|d| quadcam_sim::diff::FcConfig::from_diff(d).motor_poles as f64)
        .unwrap_or(base.motor.poles.value);
    let scales = Scales {
        motor_poles: poles,
        ..Default::default()
    };
    let logs = read_path(std::path::Path::new(&a[2]), scales).expect("read the logs");
    let t0 = std::time::Instant::now();
    let r = fit(&base, &inputs, &logs).expect("fit");
    print!("{}", r.table());
    eprintln!("fit took {:.1} s", t0.elapsed().as_secs_f64());
    let p = r.profile.unwrap();
    println!("\nBase {}:", base.id);
    print!("{}", validate(&base, &logs).table());
    println!("\nFitted:");
    print!("{}", validate(&p, &logs).table());
    if let Some(o) = out {
        std::fs::write(o, serde_json::to_string_pretty(&p).unwrap()).unwrap();
    }
}
