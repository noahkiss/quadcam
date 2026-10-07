//! Cut a CI fixture from a decoded blackbox CSV: a time window, the harness's columns, the
//! log's scales; the scrubber refuses anything identifying.
//!
//! cargo run -p quadcam-sim --example fixture -- <decoded.csv> <from s> <to s> <profile> <motor poles> <out.csv>

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() != 6 {
        eprintln!("usage: fixture <decoded.csv> <from s> <to s> <profile> <motor poles> <out.csv>");
        std::process::exit(2);
    }
    let text = std::fs::read_to_string(&a[0]).expect("read the decoded log");
    let (t0, t1): (f64, f64) = (a[1].parse().expect("from"), a[2].parse().expect("to"));
    let meta = [
        ("profile", a[3].as_str()),
        ("motor_poles", a[4].as_str()),
        ("acc_1g", "2048"),
    ];
    match quadcam_sim::log::excerpt(&text, t0, t1, &meta) {
        Ok(f) => std::fs::write(&a[5], f).expect("write the fixture"),
        Err(e) => {
            eprintln!("refused: {e}");
            std::process::exit(1);
        }
    }
}
