//! Decoded blackbox logs: the CSV `blackbox_decode` writes (run as a separate program; its
//! output is data), and the test fixtures cut from it (sim-design 6.2, 6.4).
//!
//! A fixture is the same CSV with only the columns the harness reads, a short time window,
//! and `# key: value` lines in front (the profile and the log's scales). The scrubber
//! writes fixtures and refuses text holding a board UID, serial, date or name.

use std::collections::BTreeMap;
use std::path::Path;

use crate::bbl::BblLog;

/// The columns a fixture keeps, by `blackbox_decode`'s header names.
pub const FIXTURE_COLUMNS: &[&str] = &[
    "time (us)",
    "setpoint[0]",
    "setpoint[1]",
    "setpoint[2]",
    "setpoint[3]",
    "gyroADC[0]",
    "gyroADC[1]",
    "gyroADC[2]",
    "accSmooth[0]",
    "accSmooth[1]",
    "accSmooth[2]",
    "motor[0]",
    "motor[1]",
    "motor[2]",
    "motor[3]",
    "eRPM[0]",
    "eRPM[1]",
    "eRPM[2]",
    "eRPM[3]",
    "vbatLatest (V)",
    "amperageLatest (A)",
    "imuQuaternion[0]",
    "imuQuaternion[1]",
    "imuQuaternion[2]",
];

/// The optional ground-speed column a fixture may carry (coast-down needs it).
pub const SPEED_COLUMN: &str = "speed (m/s)";

/// The quaternion's fixed-point scale in the log.
const Q15: f64 = 32767.0;

/// DShot's motor value range in the log: 48 is zero, 2047 full.
const DSHOT_MIN: f64 = 48.0;
const DSHOT_SPAN: f64 = 1999.0;

/// One decoded log, in physical units. Sample `i` of every vector is one log frame.
#[derive(Debug, Clone, Default)]
pub struct LogData {
    pub name: String,
    pub meta: BTreeMap<String, String>,
    /// Seconds from the first frame.
    pub t: Vec<f64>,
    /// Rate setpoint (deg/s): roll, pitch, yaw, Betaflight axes.
    pub setpoint: Vec<[f64; 3]>,
    /// Throttle 0..1 (`setpoint[3]` / 1000).
    pub throttle: Vec<f64>,
    /// Gyro (deg/s), Betaflight axes.
    pub gyro: Vec<[f64; 3]>,
    /// Accelerometer (g), body axes: z is thrust over mass.
    pub acc: Vec<[f64; 3]>,
    /// ESC duty 0..1 per motor.
    pub duty: Vec<[f64; 4]>,
    pub rpm: Vec<[f64; 4]>,
    pub vbat: Vec<f64>,
    pub amps: Vec<f64>,
    /// Attitude (w, x, y, z), from the FC's estimate.
    pub quat: Vec<[f64; 4]>,
    /// Horizontal ground speed (m/s), when the log has it (a `speed (m/s)` column).
    pub speed: Option<Vec<f64>>,
}

impl LogData {
    pub fn len(&self) -> usize {
        self.t.len()
    }

    pub fn is_empty(&self) -> bool {
        self.t.is_empty()
    }

    /// Median sample interval (s).
    pub fn dt(&self) -> f64 {
        let mut d: Vec<f64> = self.t.windows(2).map(|w| w[1] - w[0]).collect();
        if d.is_empty() {
            return 0.0;
        }
        d.sort_by(f64::total_cmp);
        d[d.len() / 2]
    }

    pub fn mean_duty(&self, i: usize) -> f64 {
        self.duty[i].iter().sum::<f64>() / 4.0
    }

    pub fn mean_rpm(&self, i: usize) -> f64 {
        self.rpm[i].iter().sum::<f64>() / 4.0
    }

    /// The first index at or after time `t`.
    pub fn index_at(&self, t: f64) -> usize {
        self.t.partition_point(|x| *x < t)
    }
}

/// Scales a decoded CSV does not carry: the motor pole count (eRPM to rpm) and the
/// accelerometer's 1 g. A fixture's `# motor_poles:` and `# acc_1g:` lines override them.
#[derive(Debug, Clone, Copy)]
pub struct Scales {
    pub motor_poles: f64,
    pub acc_1g: f64,
}

impl Default for Scales {
    fn default() -> Scales {
        Scales {
            motor_poles: 14.0,
            acc_1g: 2048.0,
        }
    }
}

/// Parse a decoded CSV or a fixture.
pub fn parse(name: &str, text: &str, scales: Scales) -> Result<LogData, String> {
    let mut meta = BTreeMap::new();
    let mut lines = text.lines().peekable();
    while let Some(l) = lines.peek() {
        if let Some(rest) = l.strip_prefix('#') {
            if let Some((k, v)) = rest.split_once(':') {
                meta.insert(k.trim().to_string(), v.trim().to_string());
            }
            lines.next();
        } else {
            break;
        }
    }
    let header = lines.next().ok_or("empty log")?;
    let cols: Vec<&str> = header.split(',').map(|c| c.trim()).collect();
    let idx = |name: &str| cols.iter().position(|c| *c == name);
    let need = |name: &str| idx(name).ok_or_else(|| format!("{name}: column missing"));
    let it = need("time (us)")?;
    let isp: Vec<usize> = (0..4)
        .map(|i| need(&format!("setpoint[{i}]")))
        .collect::<Result<_, _>>()?;
    let ig: Vec<usize> = (0..3)
        .map(|i| need(&format!("gyroADC[{i}]")))
        .collect::<Result<_, _>>()?;
    let ia: Vec<usize> = (0..3)
        .map(|i| need(&format!("accSmooth[{i}]")))
        .collect::<Result<_, _>>()?;
    let im: Vec<usize> = (0..4)
        .map(|i| need(&format!("motor[{i}]")))
        .collect::<Result<_, _>>()?;
    let ie: Vec<usize> = (0..4)
        .map(|i| need(&format!("eRPM[{i}]")))
        .collect::<Result<_, _>>()?;
    let iv = need("vbatLatest (V)")?;
    let ic = idx("amperageLatest (A)");
    let is = idx(SPEED_COLUMN);
    let mut speed = Vec::new();
    let iq: Vec<Option<usize>> = (0..3)
        .map(|i| idx(&format!("imuQuaternion[{i}]")))
        .collect();

    let num = |k: &str, d: f64| {
        meta.get(k)
            .and_then(|v: &String| v.parse().ok())
            .unwrap_or(d)
    };
    let poles = num("motor_poles", scales.motor_poles);
    let one_g = num("acc_1g", scales.acc_1g);
    let mut d = LogData {
        name: name.to_string(),
        ..LogData::default()
    };
    let mut t0 = None;
    for l in lines {
        let f: Vec<&str> = l.split(',').map(|x| x.trim()).collect();
        let g = |i: usize| f.get(i).and_then(|x| x.parse::<f64>().ok());
        // Skip rows the decoder could not fill (event rows, short rows).
        let Some(t_us) = g(it) else { continue };
        let row = (|| -> Option<()> {
            let t = (t_us - *t0.get_or_insert(t_us)) / 1e6;
            let sp = [g(isp[0])?, g(isp[1])?, g(isp[2])?];
            let thr = g(isp[3])? / 1000.0;
            let gy = [g(ig[0])?, g(ig[1])?, g(ig[2])?];
            let ac = [g(ia[0])? / one_g, g(ia[1])? / one_g, g(ia[2])? / one_g];
            let mut duty = [0.0; 4];
            let mut rpm = [0.0; 4];
            for k in 0..4 {
                duty[k] = ((g(im[k])? - DSHOT_MIN) / DSHOT_SPAN).clamp(0.0, 1.0);
                rpm[k] = g(ie[k])? * 100.0 / (poles / 2.0);
            }
            let v = g(iv)?;
            let a = ic.and_then(g).unwrap_or(0.0);
            let q = match (iq[0], iq[1], iq[2]) {
                (Some(a), Some(b), Some(c)) => {
                    // Logged as Q15 (×32767), w left out.
                    let (x, y, z) = (g(a)? / Q15, g(b)? / Q15, g(c)? / Q15);
                    [(1.0 - x * x - y * y - z * z).max(0.0).sqrt(), x, y, z]
                }
                _ => [1.0, 0.0, 0.0, 0.0],
            };
            d.t.push(t);
            d.setpoint.push(sp);
            d.throttle.push(thr);
            d.gyro.push(gy);
            d.acc.push(ac);
            d.duty.push(duty);
            d.rpm.push(rpm);
            d.vbat.push(v);
            d.amps.push(a);
            d.quat.push(q);
            if let Some(k) = is {
                speed.push(g(k)?);
            }
            Some(())
        })();
        let _ = row;
    }
    d.meta = meta;
    if is.is_some() {
        d.speed = Some(speed);
    }
    if d.is_empty() {
        return Err(format!("{name}: no rows"));
    }
    Ok(d)
}

pub fn read(path: &Path, scales: Scales) -> Result<LogData, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    parse(&name, &text, scales)
}

/// A log from QuadCam's own blackbox decoder (`bbl`), in the same units as a decoded CSV.
/// The log's header gives the accelerometer's 1 g and the motor output range; `scales`
/// gives the pole count. Logs without the fields the harness reads are refused.
pub fn from_bbl(name: &str, log: &BblLog, scales: Scales) -> Result<LogData, String> {
    let need = |f: &str| log.field(f).ok_or_else(|| format!("{name}: no {f} field"));
    let it = need("time")?;
    let isp = [0, 1, 2, 3].map(|i| log.field(&format!("setpoint[{i}]")));
    let ig = [0, 1, 2].map(|i| log.field(&format!("gyroADC[{i}]")));
    let ia = [0, 1, 2].map(|i| log.field(&format!("accSmooth[{i}]")));
    let im = [0, 1, 2, 3].map(|i| log.field(&format!("motor[{i}]")));
    let ie = [0, 1, 2, 3].map(|i| log.field(&format!("eRPM[{i}]")));
    let iq = [0, 1, 2].map(|i| log.field(&format!("imuQuaternion[{i}]")));
    for (what, ok) in [
        ("setpoint", isp.iter().all(Option::is_some)),
        ("gyroADC", ig.iter().all(Option::is_some)),
        ("accSmooth", ia.iter().all(Option::is_some)),
        ("motor", im.iter().all(Option::is_some)),
        ("eRPM (bidirectional DShot)", ie.iter().all(Option::is_some)),
    ] {
        if !ok {
            return Err(format!("{name}: the log has no {what} fields"));
        }
    }
    let iv = need("vbatLatest")?;
    let ic = log.field("amperageLatest");
    let one_g = log
        .header_ints("acc_1G")
        .first()
        .map(|g| *g as f64)
        .unwrap_or(scales.acc_1g);
    let out = log.header_ints("motorOutput");
    let (m_min, m_max) = match out.as_slice() {
        [a, b, ..] if b > a => (*a as f64, *b as f64),
        _ => (DSHOT_MIN, DSHOT_MIN + DSHOT_SPAN),
    };
    let poles = scales.motor_poles;
    let mut d = LogData {
        name: name.to_string(),
        ..LogData::default()
    };
    d.meta.insert("motor_poles".into(), format!("{poles}"));
    d.meta.insert("acc_1g".into(), format!("{one_g}"));
    let Some(t0) = log.frames.first().map(|f| f[it]) else {
        return Err(format!("{name}: no frames"));
    };
    let v = |f: &[i64], i: Option<usize>| i.map_or(0.0, |i| f[i] as f64);
    for f in &log.frames {
        d.t.push((f[it] - t0) as f64 / 1e6);
        d.setpoint.push([v(f, isp[0]), v(f, isp[1]), v(f, isp[2])]);
        d.throttle.push(v(f, isp[3]) / 1000.0);
        d.gyro.push([v(f, ig[0]), v(f, ig[1]), v(f, ig[2])]);
        d.acc.push([
            v(f, ia[0]) / one_g,
            v(f, ia[1]) / one_g,
            v(f, ia[2]) / one_g,
        ]);
        d.duty
            .push(im.map(|i| ((v(f, i) - m_min) / (m_max - m_min)).clamp(0.0, 1.0)));
        d.rpm.push(ie.map(|i| v(f, i) * 100.0 / (poles / 2.0)));
        // Hundredths of a volt and of an amp.
        d.vbat.push(v(f, Some(iv)) / 100.0);
        d.amps.push(v(f, ic) / 100.0);
        d.quat.push(if iq.iter().all(Option::is_some) {
            let (x, y, z) = (v(f, iq[0]) / Q15, v(f, iq[1]) / Q15, v(f, iq[2]) / Q15);
            [(1.0 - x * x - y * y - z * z).max(0.0).sqrt(), x, y, z]
        } else {
            [1.0, 0.0, 0.0, 0.0]
        });
    }
    Ok(d)
}

/// Every log in a blackbox file or flash image, named `<file>#<n>`. Logs shorter than
/// `min_seconds` (a test arm) are left out; a log the harness cannot read is an error.
pub fn read_bbl(path: &Path, scales: Scales, min_seconds: f64) -> Result<Vec<LogData>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    from_image(&file, &bytes, scales, min_seconds)
}

/// Every log in a blackbox image already in memory, named `<file>#<n>`, as `read_bbl`.
pub fn from_image(
    file: &str,
    bytes: &[u8],
    scales: Scales,
    min_seconds: f64,
) -> Result<Vec<LogData>, String> {
    let mut out = Vec::new();
    for l in crate::bbl::decode_image(bytes) {
        let l = l.map_err(|e| format!("{file}: {e}"))?;
        if l.duration() < min_seconds {
            continue;
        }
        out.push(from_bbl(&format!("{file}#{}", l.index), &l, scales)?);
    }
    Ok(out)
}

/// Whether a path is a blackbox file QuadCam decodes itself.
pub fn is_bbl(p: &Path) -> bool {
    p.extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| ["bbl", "bfl", "bbs"].contains(&x.to_ascii_lowercase().as_str()))
}

/// Logs shorter than this are test arms, not flights.
pub const MIN_FLIGHT_SECONDS: f64 = 5.0;

/// Logs from a path: a decoded CSV, a blackbox file or flash image, or a folder of either
/// (sorted by name).
pub fn read_path(path: &Path, scales: Scales) -> Result<Vec<LogData>, String> {
    if path.is_file() {
        return if is_bbl(path) {
            read_bbl(path, scales, MIN_FLIGHT_SECONDS)
        } else {
            Ok(vec![read(path, scales)?])
        };
    }
    let mut paths: Vec<_> = std::fs::read_dir(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| is_bbl(p) || p.extension().is_some_and(|x| x == "csv"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for p in paths {
        if is_bbl(&p) {
            out.extend(read_bbl(&p, scales, MIN_FLIGHT_SECONDS)?);
        } else {
            out.push(read(&p, scales)?);
        }
    }
    Ok(out)
}

/// Every decoded CSV in a folder (`*.csv`), sorted by name.
pub fn read_folder(dir: &Path, scales: Scales) -> Result<Vec<LogData>, String> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "csv"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for p in paths {
        out.push(read(&p, scales)?);
    }
    Ok(out)
}

/// Cut a fixture from a decoded CSV: rows with time in `[t0, t1)` seconds from the log's
/// first frame, only `FIXTURE_COLUMNS`, time re-based to zero. `meta` becomes `# k: v`
/// lines. Nothing else of the source (headers, events, names) is carried over.
pub fn excerpt(text: &str, t0: f64, t1: f64, meta: &[(&str, &str)]) -> Result<String, String> {
    let mut lines = text.lines().skip_while(|l| l.starts_with('#'));
    let header = lines.next().ok_or("empty log")?;
    let cols: Vec<&str> = header.split(',').map(|c| c.trim()).collect();
    let mut names: Vec<&str> = FIXTURE_COLUMNS.to_vec();
    if cols.contains(&SPEED_COLUMN) {
        names.push(SPEED_COLUMN);
    }
    let pick: Vec<usize> = names
        .iter()
        .map(|c| {
            cols.iter()
                .position(|x| x == c)
                .ok_or_else(|| format!("{c}: column missing"))
        })
        .collect::<Result<_, _>>()?;
    let mut out = String::new();
    for (k, v) in meta {
        out.push_str(&format!("# {k}: {v}\n"));
    }
    out.push_str(&names.join(","));
    out.push('\n');
    let mut first: Option<f64> = None;
    let mut base: Option<f64> = None;
    for l in lines {
        let f: Vec<&str> = l.split(',').map(|x| x.trim()).collect();
        let Some(t_us) = f.get(pick[0]).and_then(|x| x.parse::<f64>().ok()) else {
            continue;
        };
        let s = (t_us - *first.get_or_insert(t_us)) / 1e6;
        if s < t0 || s >= t1 {
            continue;
        }
        if pick
            .iter()
            .any(|i| f.get(*i).is_none_or(|x| x.parse::<f64>().is_err()))
        {
            continue;
        }
        let b = *base.get_or_insert(t_us);
        let mut row = vec![format!("{}", (t_us - b).round() as i64)];
        row.extend(pick[1..].iter().map(|i| f[*i].to_string()));
        out.push_str(&row.join(","));
        out.push('\n');
    }
    check_scrubbed(&out)?;
    Ok(out)
}

/// Refuse text that holds what a publishable fixture must not: a board UID or serial (a
/// long hex or digit run), a calendar date or clock time, or a craft or pilot name line.
pub fn check_scrubbed(text: &str) -> Result<(), String> {
    for (n, line) in text.lines().enumerate() {
        let lower = line.to_ascii_lowercase();
        for word in ["uid", "serial", "craft", "pilot", "name", "board"] {
            if lower.contains(word) {
                return Err(format!("line {}: names a {word}", n + 1));
            }
        }
        for tok in line.split(|c: char| !c.is_ascii_alphanumeric()) {
            let hexish = tok.len() >= 12 && tok.chars().all(|c| c.is_ascii_hexdigit());
            let has_alpha = tok.chars().any(|c| c.is_ascii_alphabetic());
            if hexish && (has_alpha || tok.len() >= 16) {
                return Err(format!("line {}: UID-shaped value {tok}", n + 1));
            }
        }
        let b = line.as_bytes();
        for i in 0..b.len().saturating_sub(9) {
            let w = &b[i..i + 10];
            let d = |k: usize| w[k].is_ascii_digit();
            if d(0)
                && d(1)
                && d(2)
                && d(3)
                && (w[4] == b'-' || w[4] == b'/')
                && d(5)
                && d(6)
                && w[7] == w[4]
                && d(8)
                && d(9)
            {
                return Err(format!("line {}: a date", n + 1));
            }
        }
        for i in 0..b.len().saturating_sub(4) {
            let w = &b[i..i + 5];
            if w[0].is_ascii_digit()
                && w[1].is_ascii_digit()
                && w[2] == b':'
                && w[3].is_ascii_digit()
                && w[4].is_ascii_digit()
                && !line.starts_with('#')
            {
                return Err(format!("line {}: a clock time", n + 1));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded() -> String {
        let mut s = String::from(
            "loopIteration, time (us), setpoint[0], setpoint[1], setpoint[2], setpoint[3], \
             gyroADC[0], gyroADC[1], gyroADC[2], accSmooth[0], accSmooth[1], accSmooth[2], \
             motor[0], motor[1], motor[2], motor[3], eRPM[0], eRPM[1], eRPM[2], eRPM[3], \
             vbatLatest (V), amperageLatest (A), imuQuaternion[0], imuQuaternion[1], \
             imuQuaternion[2], flightModeFlags (flags)\n",
        );
        for i in 0..100 {
            let t = 5_000_000 + i * 1250;
            s.push_str(&format!(
                "{i}, {t}, 10, -5, 0, 350, 9, -4, 1, 0, 0, 2048, 742, 742, 742, 742, \
                 210, 210, 210, 210, 3.80, 6.2, 0, 0, 0, ANGLE_MODE\n"
            ));
        }
        s
    }

    #[test]
    fn parses_units() {
        let d = parse(
            "x",
            &decoded(),
            Scales {
                motor_poles: 12.0,
                acc_1g: 2048.0,
            },
        )
        .unwrap();
        assert_eq!(d.len(), 100);
        assert!((d.dt() - 0.00125).abs() < 1e-9);
        assert!((d.duty[0][0] - (742.0 - 48.0) / 1999.0).abs() < 1e-12);
        assert_eq!(d.rpm[0][0], 21000.0 / 6.0 * 1.0);
        assert_eq!(d.acc[0][2], 1.0);
        assert_eq!(d.throttle[0], 0.35);
    }

    #[test]
    fn excerpt_keeps_the_window_and_round_trips() {
        let f = excerpt(
            &decoded(),
            0.025,
            0.05,
            &[("profile", "meteor75"), ("motor_poles", "12")],
        )
        .unwrap();
        let d = parse("f", &f, Scales::default()).unwrap();
        assert_eq!(d.len(), 20);
        assert_eq!(d.t[0], 0.0);
        assert_eq!(d.meta["profile"], "meteor75");
        // motor_poles from the fixture overrides the default 14.
        assert_eq!(d.rpm[0][0], 3500.0);
        assert!(!f.contains("flightModeFlags"));
    }

    #[test]
    fn scrub_check_catches_a_planted_uid() {
        let clean = excerpt(&decoded(), 0.0, 1.0, &[("profile", "meteor75")]).unwrap();
        assert!(check_scrubbed(&clean).is_ok());
        for planted in [
            "# board_uid: 3A0047001851393436383537\n",
            "# id: 0036003A3133510D37363435\n",
            "1,2,3,deadbeefcafe12\n",
            "# date: 2026-10-07\n",
            "# craft_name: SOMEQUAD\n",
            "1,2,12:34,4\n",
        ] {
            let bad = format!("{planted}{clean}");
            assert!(check_scrubbed(&bad).is_err(), "missed {planted:?}");
        }
    }
}
