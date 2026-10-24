//! Validate a preset against blackbox logs: decoded CSVs, `.bbl` files or flash images.
//!
//! cargo run -p quadcam-sim --example validate -- <preset> <motor poles> <log>...

use std::path::Path;

use quadcam_sim::log::{read_path, Scales};
use quadcam_sim::preset;
use quadcam_sim::validate::validate;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let p = preset(&args[0]).expect("unknown preset");
    let scales = Scales {
        motor_poles: args[1].parse().expect("motor poles"),
        ..Scales::default()
    };
    let logs: Vec<_> = args[2..]
        .iter()
        .flat_map(|f| read_path(Path::new(f), scales).expect("read log"))
        .filter(|l| l.len() > 100)
        .collect();
    print!("{}", validate(&p, &logs).table());
}
