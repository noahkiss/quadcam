//! A synthetic EdgeTX log with known measures, for the flight-analysis tests and the
//! session report. Made-up model name and values; no row comes from a real log.
//!
//! Rows every 0.1 s. Three flights of the model `Whoop` on 2026-10-04:
//!
//! 1. 10:00:02, 60 s, after 2 s of disarmed rows (`ACRO*`). Hover at 40 % for its first
//!    10 s and at 46 % from 45 s to 55 s; a centred stretch of 1.5 s at 70 % and a centred
//!    stretch whose throttle steps 5 % every 0.5 s, neither long or steady enough. `RxBt`
//!    3.80 V, 3.50 V from 5.0 s to 8.9 s, 3.30 V once at 40 s. `Capa` rises 6 mAh a second
//!    to 360. A dropout from 25.0 s to 25.5 s (the flight goes on), a 0.2 s blip at 35 s.
//!    Then 5 s of disarmed rows at 3.95 V.
//! 2. 10:03:00, 30 s, never centred, `RxBt` 3.90 V, no rows after disarm.
//! 3. 10:06:00, 20 s, first reading 4.05 V, silent from 18 s to its end (a failsafe).

use anyhow::Result;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The measures the synthetic log was built to have.
pub struct Known {
    pub flights: usize,
    pub f1_secs: f64,
    pub hover_all: f64,
    pub hover_early: f64,
    pub hover_late: f64,
    pub sag_p5: f64,
    pub sag_min: f64,
    pub f1_resting: f64,
    pub f2_resting: f64,
    pub f1_mah: f64,
    pub threshold: f64,
    pub threshold_at_s: f64,
    pub drop_start_s: f64,
    pub drop_secs: f64,
    pub drop_before_lq: f64,
    pub drop_before_rssi: f64,
    pub drop_before_tpwr: f64,
    pub drop_after_lq: f64,
    pub drop_after_tpwr: f64,
    pub max_current: f64,
}

pub const KNOWN: Known = Known {
    flights: 3,
    f1_secs: 60.0,
    hover_all: 43.0,
    hover_early: 40.0,
    hover_late: 46.0,
    sag_p5: 3.50,
    sag_min: 3.30,
    f1_resting: 3.95,
    f2_resting: 4.05,
    f1_mah: 360.0,
    threshold: 300.0,
    threshold_at_s: 50.0,
    drop_start_s: 25.0,
    drop_secs: 0.5,
    drop_before_lq: 60.0,
    drop_before_rssi: -95.0,
    drop_before_tpwr: 100.0,
    drop_after_lq: 70.0,
    drop_after_tpwr: 250.0,
    max_current: 12.0,
};

/// The log's file name.
pub const FILE: &str = "Whoop-2026-10-04-100000.csv";

const HEADER: &str = "Date,Time,1RSS(dB),RQly(%),RSNR(dB),TPWR(mW),RxBt(V),Curr(A),Capa(mAh),Bat%(%),FM,Rud,Ele,Thr,Ail,TxBat(V)";

/// One row's values.
struct Row {
    rssi: f64,
    lq: f64,
    tpwr: f64,
    rxbt: f64,
    curr: f64,
    capa: i64,
    fm: &'static str,
    ail: f64,
    thr_pct: f64,
}

impl Default for Row {
    fn default() -> Self {
        Row {
            rssi: -60.0,
            lq: 100.0,
            tpwr: 25.0,
            rxbt: 3.80,
            curr: 5.0,
            capa: 0,
            fm: "ACRO",
            ail: 0.0,
            thr_pct: 60.0,
        }
    }
}

fn push(out: &mut String, start_s: i64, tenths: i64, r: &Row) {
    let ms = start_s * 1000 + tenths * 100;
    let (h, m, s, f) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    let thr = r.thr_pct / 100.0 * 2048.0 - 1024.0;
    let _ = writeln!(
        out,
        "2026-10-04,{h:02}:{m:02}:{s:02}.{f:03},{},{},9,{},{:.2},{:.1},{},{},{},0,0,{:.2},{},7.9",
        r.rssi,
        r.lq,
        r.tpwr,
        r.rxbt,
        r.curr,
        r.capa,
        (100 - r.capa / 4).max(0),
        r.fm,
        thr,
        r.ail
    );
}

/// The synthetic log's text.
pub fn known_text() -> String {
    let mut out = String::from(HEADER);
    out.push('\n');
    let t0 = 10 * 3600;
    // Disarmed before the first arm.
    for k in 0..20 {
        let r = Row {
            fm: "ACRO*",
            rxbt: 4.15,
            thr_pct: 0.0,
            curr: 0.1,
            ..Row::default()
        };
        push(&mut out, t0, k, &r);
    }
    // Flight 1.
    for m in 0..=600i64 {
        let mut r = Row {
            capa: 6 * m / 10,
            ail: 500.0,
            ..Row::default()
        };
        if m <= 100 {
            (r.ail, r.thr_pct) = (0.0, 40.0);
        } else if (200..230).contains(&m) {
            (r.ail, r.thr_pct) = (0.0, 50.0 + 5.0 * ((m - 200) / 5) as f64);
        } else if (300..=315).contains(&m) {
            (r.ail, r.thr_pct) = (0.0, 70.0);
        } else if (450..=550).contains(&m) {
            (r.ail, r.thr_pct) = (0.0, 46.0);
        }
        if (50..=89).contains(&m) {
            r.rxbt = 3.50;
        }
        if m == 400 {
            r.rxbt = 3.30;
        }
        if m == 300 {
            r.curr = 12.0;
        }
        if m == 249 {
            (r.lq, r.rssi, r.tpwr) = (60.0, -95.0, 100.0);
        }
        if (250..=254).contains(&m) || (350..=351).contains(&m) {
            (r.fm, r.rxbt, r.lq, r.rssi) = ("", 0.0, 0.0, 0.0);
            if m < 300 {
                r.tpwr = 250.0;
            }
        }
        if m == 255 {
            (r.lq, r.rssi, r.tpwr) = (70.0, -93.0, 250.0);
        }
        push(&mut out, t0, 20 + m, &r);
    }
    // After disarm.
    for k in 621..=670 {
        let r = Row {
            fm: "ACRO*",
            rxbt: 3.95,
            capa: 360,
            curr: 0.2,
            thr_pct: 0.0,
            ..Row::default()
        };
        push(&mut out, t0, k, &r);
    }
    // Flight 2: never centred, ends at disarm.
    for m in 0..=300i64 {
        let r = Row {
            fm: "ANGL",
            rxbt: 3.90,
            capa: 5 * m / 10,
            ail: 500.0,
            ..Row::default()
        };
        push(&mut out, t0 + 180, m, &r);
    }
    // Flight 3: a failsafe at the end.
    for m in 0..=200i64 {
        let mut r = Row {
            rxbt: if m == 0 { 4.05 } else { 3.85 },
            capa: 5 * m / 10,
            ail: 500.0,
            ..Row::default()
        };
        if m >= 180 {
            (r.fm, r.rxbt, r.lq, r.rssi) = ("", 0.0, 0.0, 0.0);
        }
        push(&mut out, t0 + 360, m, &r);
    }
    out
}

/// Writes the synthetic log into `dir` as `FILE`; returns its path.
pub fn write_known(dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let p = dir.join(FILE);
    std::fs::write(&p, known_text())?;
    Ok(p)
}
