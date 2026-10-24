//! Decode a blackbox file or flash image with QuadCam's own decoder and write one CSV per
//! log, in `blackbox_decode`'s column names for the fields the harness reads.
//!
//! `cargo run -p quadcam-sim --example decode -- <file.bbl> <out-dir>`

use quadcam_sim::bbl::decode_image;
use quadcam_sim::log::FIXTURE_COLUMNS;
use std::fmt::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(src), Some(out)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: decode <file.bbl> <out-dir>");
        std::process::exit(2);
    };
    let bytes = std::fs::read(src).expect("read the file");
    std::fs::create_dir_all(out).expect("make the output folder");
    for log in decode_image(&bytes) {
        let log = match log {
            Ok(l) => l,
            Err(e) => {
                eprintln!("{e}");
                continue;
            }
        };
        let s = &log.stats;
        println!(
            "log {:>2}: {:>7.1} s, {} I + {} P frames, {} corrupt, ended {}",
            log.index,
            log.duration(),
            s.i_frames,
            s.p_frames,
            s.corrupt,
            s.ended
        );
        // `blackbox_decode` names the scaled columns with their unit.
        let name = |c: &str| match c {
            "time (us)" => "time".to_string(),
            "vbatLatest (V)" => "vbatLatest".to_string(),
            "amperageLatest (A)" => "amperageLatest".to_string(),
            c => c.to_string(),
        };
        let cols: Vec<Option<usize>> = FIXTURE_COLUMNS
            .iter()
            .map(|c| log.field(&name(c)))
            .collect();
        let mut text = FIXTURE_COLUMNS.join(",");
        text.push('\n');
        for f in &log.frames {
            for (k, (c, ix)) in FIXTURE_COLUMNS.iter().zip(&cols).enumerate() {
                if k > 0 {
                    text.push(',');
                }
                let v = ix.map_or(0, |i| f[i]);
                if c.ends_with("(V)") || c.ends_with("(A)") {
                    write!(text, "{:.2}", v as f64 / 100.0).unwrap();
                } else {
                    write!(text, "{v}").unwrap();
                }
            }
            text.push('\n');
        }
        let path = format!("{out}/log{:02}.csv", log.index);
        std::fs::write(&path, text).expect("write the CSV");
    }
}
