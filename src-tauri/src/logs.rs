//! EdgeTX radio logs: read armed rows, split into segments and sessions, and match them to
//! clips.

use anyhow::{Context, Result};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct Tunables {
    /// A gap over this many seconds between rows starts a new armed segment.
    pub segment_gap_s: f64,
    /// A gap over this many minutes between segments starts a new session.
    pub session_gap_min: f64,
    /// Slack, in seconds, allowed when a clip claims segments.
    pub tolerance_s: f64,
    /// A log day further than this from the import date means the radio clock reset.
    pub max_log_age_days: i64,
    /// A clip with its own clock (DJI) takes a matching log's time only when the log starts
    /// within this many seconds of the clip clock.
    #[serde(default = "default_clock_skew_s")]
    pub clock_skew_s: f64,
}

fn default_clock_skew_s() -> f64 {
    300.0
}

impl Default for Tunables {
    fn default() -> Self {
        Self {
            segment_gap_s: 5.0,
            session_gap_min: 20.0,
            tolerance_s: 30.0,
            max_log_age_days: 60,
            clock_skew_s: default_clock_skew_s(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, specta::Type)]
pub struct Segment {
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub session: usize,
}

impl Segment {
    pub fn secs(&self) -> f64 {
        (self.end - self.start).num_milliseconds() as f64 / 1000.0
    }
}

/// Every CSV under `dir` (and `dir/LOGS` when `dir` is a radio's root).
pub fn csv_files(dir: &Path) -> Vec<PathBuf> {
    let base = if dir.join("LOGS").is_dir() {
        dir.join("LOGS")
    } else {
        dir.to_path_buf()
    };
    let Ok(rd) = std::fs::read_dir(&base) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            !n.starts_with('.') && n.to_lowercase().ends_with(".csv")
        })
        .collect();
    v.sort();
    v
}

/// Stick positions from one log row, as EdgeTX writes them: -1024..1024. Throttle is -1024
/// at the bottom.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default, specta::Type)]
pub struct Sticks {
    pub ail: f64,
    pub ele: f64,
    pub thr: f64,
    pub rud: f64,
}

/// One EdgeTX log row: its time, the sticks when the model logs them, and the flight
/// controller's attitude telemetry (radians) when the receiver sends it. A column the log
/// does not have reads as None (`channels` as empty).
#[derive(Debug, Clone, PartialEq)]
pub struct LogRow {
    pub time: NaiveDateTime,
    pub sticks: Option<Sticks>,
    pub roll: Option<f64>,
    pub pitch: Option<f64>,
    /// Receiver battery (`RxBt(V)`), link quality (`RQly(%)`) and signal (`1RSS(dB)`).
    pub rx_bat: Option<f64>,
    pub lq: Option<f64>,
    pub rssi: Option<f64>,
    /// The EdgeTX model name, from the log file's name.
    pub model: Option<std::sync::Arc<str>>,
    /// Transmit power (`TPWR(mW)`) and signal to noise (`RSNR(dB)`).
    pub tx_power_mw: Option<f64>,
    pub snr_db: Option<f64>,
    /// Current (`Curr(A)`), capacity used (`Capa(mAh)`) and battery left (`Bat%(%)`).
    pub current_a: Option<f64>,
    pub capacity_mah: Option<f64>,
    pub bat_pct: Option<f64>,
    /// The flight controller's flight mode (`FM`): Some("") when the column is there but
    /// empty (no telemetry), None when the log has no `FM` column.
    pub flight_mode: Option<std::sync::Arc<str>>,
    /// Channel outputs in microseconds (`CH1(us)`, `CH2(us)`, ...), CH1 first. Empty when
    /// the log has none; a channel the row lacks is NaN.
    pub channels: Vec<f64>,
    /// The radio's own battery (`TxBat(V)`).
    pub tx_bat: Option<f64>,
}

impl LogRow {
    /// A row at `time` with no values.
    pub fn at(time: NaiveDateTime) -> LogRow {
        LogRow {
            time,
            sticks: None,
            roll: None,
            pitch: None,
            rx_bat: None,
            lq: None,
            rssi: None,
            model: None,
            tx_power_mw: None,
            snr_db: None,
            current_a: None,
            capacity_mah: None,
            bat_pct: None,
            flight_mode: None,
            channels: Vec::new(),
            tx_bat: None,
        }
    }

    /// False only when the flight mode says disarmed: Betaflight sends it with a trailing
    /// `*` (`ACRO*`). A log without `FM`, or a row with it blank (no telemetry), counts as
    /// armed, so a log recorded only while armed reads as before.
    pub fn armed(&self) -> bool {
        !self
            .flight_mode
            .as_deref()
            .is_some_and(|m| m.ends_with('*'))
    }
}

/// The model name in an EdgeTX log file name: `<model>-YYYY-MM-DD[-HHMMSS].csv`. A trailing
/// ` (n)` is the log store's second copy of a changed log (`radiologs`), not part of the name.
pub fn model_from_file_name(name: &str) -> Option<String> {
    let stem = name
        .strip_suffix(".csv")
        .or_else(|| name.strip_suffix(".CSV"))?;
    let stem = strip_copy_suffix(stem);
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    let mut parts: Vec<&str> = stem.split('-').collect();
    if parts.len() > 4 && parts.last().is_some_and(|p| digits(p) && p.len() == 6) {
        parts.pop();
    }
    if parts.len() < 4 {
        return None;
    }
    let n = parts.len();
    let (y, m, d) = (parts[n - 3], parts[n - 2], parts[n - 1]);
    if !(digits(y) && y.len() == 4 && digits(m) && m.len() == 2 && digits(d) && d.len() == 2) {
        return None;
    }
    let model = parts[..n - 3].join("-");
    (!model.trim().is_empty()).then_some(model)
}

/// `stem (2)` -> `stem`: drops the log store's copy number.
fn strip_copy_suffix(stem: &str) -> &str {
    stem.strip_suffix(')')
        .and_then(|s| s.rsplit_once(" ("))
        .filter(|(_, n)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        .map_or(stem, |(base, _)| base)
}

/// Splits one CSV line. EdgeTX quotes text columns such as the flight mode.
fn split_csv(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                out.push(line[start..i].trim().trim_matches('"'));
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(line[start..].trim().trim_matches('"'));
    out
}

/// Every row of one EdgeTX CSV text. Date and Time are the first two columns. The stick
/// columns (`Ail`, `Ele`, `Thr`, `Rud`), attitude (`Roll(rad)`, `Ptch(rad)`) and the other
/// telemetry columns `LogRow` holds are found by header name, so any column order and any
/// extra or missing columns work.
pub fn parse_rows(text: &str) -> Vec<LogRow> {
    let mut lines = text.lines();
    let header: Vec<String> = lines
        .next()
        .map(|h| split_csv(h).into_iter().map(str::to_string).collect())
        .unwrap_or_default();
    let col = |name: &str| header.iter().position(|h| h == name);
    let (ail, ele, thr, rud) = (col("Ail"), col("Ele"), col("Thr"), col("Rud"));
    let (roll, pitch) = (col("Roll(rad)"), col("Ptch(rad)"));
    let (rx_bat, lq, rssi) = (col("RxBt(V)"), col("RQly(%)"), col("1RSS(dB)"));
    let (tpwr, rsnr, curr) = (col("TPWR(mW)"), col("RSNR(dB)"), col("Curr(A)"));
    let (capa, bat_pct, fm, tx_bat) =
        (col("Capa(mAh)"), col("Bat%(%)"), col("FM"), col("TxBat(V)"));
    // `CHn(us)` columns, by channel number.
    let mut ch: Vec<(usize, usize)> = header
        .iter()
        .enumerate()
        .filter_map(|(i, h)| {
            let n = h
                .strip_prefix("CH")?
                .strip_suffix("(us)")?
                .parse::<usize>()
                .ok()?;
            (n >= 1).then_some((n, i))
        })
        .collect();
    ch.sort();
    let n_ch = ch.last().map_or(0, |(n, _)| *n);
    let mut modes: Vec<std::sync::Arc<str>> = Vec::new();
    let mut out = Vec::new();
    for line in lines {
        let cols = split_csv(line);
        let (Some(d), Some(t)) = (cols.first(), cols.get(1)) else {
            continue;
        };
        let Ok(time) = NaiveDateTime::parse_from_str(&format!("{d} {t}"), "%Y-%m-%d %H:%M:%S%.f")
        else {
            continue;
        };
        let num = |i: Option<usize>| {
            i.and_then(|i| cols.get(i))
                .and_then(|v| v.parse::<f64>().ok())
        };
        let sticks = match (num(ail), num(ele), num(thr), num(rud)) {
            (Some(ail), Some(ele), Some(thr), Some(rud)) => Some(Sticks { ail, ele, thr, rud }),
            _ => None,
        };
        let flight_mode = fm.map(|i| {
            let m = cols.get(i).copied().unwrap_or("");
            match modes.iter().find(|x| &***x == m) {
                Some(x) => x.clone(),
                None => {
                    let x: std::sync::Arc<str> = m.into();
                    modes.push(x.clone());
                    x
                }
            }
        });
        let mut channels = vec![f64::NAN; n_ch];
        for &(n, i) in &ch {
            if let Some(v) = num(Some(i)) {
                channels[n - 1] = v;
            }
        }
        out.push(LogRow {
            sticks,
            roll: num(roll),
            pitch: num(pitch),
            rx_bat: num(rx_bat),
            lq: num(lq),
            rssi: num(rssi),
            tx_power_mw: num(tpwr),
            snr_db: num(rsnr),
            current_a: num(curr),
            capacity_mah: num(capa),
            bat_pct: num(bat_pct),
            flight_mode,
            channels,
            tx_bat: num(tx_bat),
            ..LogRow::at(time)
        });
    }
    out
}

/// Every row of one EdgeTX CSV file, tagged with the model name from the file name.
pub fn read_rows(path: &Path) -> Result<Vec<LogRow>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let model: Option<std::sync::Arc<str>> = path
        .file_name()
        .and_then(|n| model_from_file_name(&n.to_string_lossy()))
        .map(Into::into);
    let mut rows = parse_rows(&text);
    for r in &mut rows {
        r.model = model.clone();
    }
    Ok(rows)
}

/// Row timestamps from one EdgeTX CSV. Date and Time are the first two columns.
pub fn read_csv(path: &Path) -> Result<Vec<NaiveDateTime>> {
    Ok(read_rows(path)?.into_iter().map(|r| r.time).collect())
}

/// All rows under `dir`, grouped by day, sorted by time.
pub fn read_log_rows(dir: &Path) -> BTreeMap<NaiveDate, Vec<LogRow>> {
    let mut days: BTreeMap<NaiveDate, Vec<LogRow>> = BTreeMap::new();
    for f in csv_files(dir) {
        for r in read_rows(&f).unwrap_or_default() {
            days.entry(r.time.date()).or_default().push(r);
        }
    }
    for v in days.values_mut() {
        v.sort_by_key(|r| r.time);
    }
    days
}

/// All row times under `dir`, grouped by day, sorted.
pub fn read_log_dir(dir: &Path) -> BTreeMap<NaiveDate, Vec<NaiveDateTime>> {
    read_log_rows(dir)
        .into_iter()
        .map(|(d, rows)| (d, rows.into_iter().map(|r| r.time).collect()))
        .collect()
}

/// Splits rows at gaps over `segment_gap_s`, then numbers sessions at gaps over `session_gap_min`.
pub fn segments(rows: &[NaiveDateTime], tun: &Tunables) -> Vec<Segment> {
    let gap = Duration::milliseconds((tun.segment_gap_s * 1000.0) as i64);
    let sgap = Duration::milliseconds((tun.session_gap_min * 60_000.0) as i64);
    let mut out: Vec<Segment> = Vec::new();
    for &t in rows {
        match out.last_mut() {
            Some(s) if t - s.end <= gap => s.end = t,
            last => {
                let session = match last {
                    Some(prev) if t - prev.end > sgap => prev.session + 1,
                    Some(prev) => prev.session,
                    None => 0,
                };
                out.push(Segment {
                    start: t,
                    end: t,
                    session,
                });
            }
        }
    }
    out
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum Badge {
    Matched,
    Likely,
    Unmatched,
}

#[derive(Debug, Clone, Serialize, PartialEq, specta::Type)]
pub struct ClipMatch {
    pub badge: Badge,
    /// Start of the first claimed segment.
    pub start: Option<NaiveDateTime>,
    pub segments: usize,
    /// First start to last end of the claimed segments, in seconds.
    pub span_s: f64,
    /// Index of the first claimed segment in the list given to `match_clips`.
    #[serde(skip)]
    pub first: usize,
}

impl ClipMatch {
    fn unmatched() -> Self {
        Self {
            badge: Badge::Unmatched,
            start: None,
            segments: 0,
            span_s: 0.0,
            first: 0,
        }
    }
}

/// Walks clips (PICT order) and segments (time order) together. A clip claims consecutive
/// segments of one session while their span fits inside its duration plus the tolerance.
/// **matched**: the unarmed time left over is within twice the tolerance.
/// **likely**: the order fits but the clip holds much more unarmed time than the span.
/// **unmatched**: the next segment does not fit; the segment stays for the next clip.
pub fn match_clips(durations: &[f64], segs: &[Segment], tun: &Tunables) -> Vec<ClipMatch> {
    let mut out = Vec::with_capacity(durations.len());
    let mut j = 0;
    for &dur in durations {
        let limit = dur + tun.tolerance_s;
        if j >= segs.len() || dur <= 0.0 || segs[j].secs() > limit {
            out.push(ClipMatch::unmatched());
            continue;
        }
        let first = segs[j];
        let mut k = j;
        while k + 1 < segs.len()
            && segs[k + 1].session == first.session
            && (segs[k + 1].end - first.start).num_milliseconds() as f64 / 1000.0 <= limit
        {
            k += 1;
        }
        let span = (segs[k].end - first.start).num_milliseconds() as f64 / 1000.0;
        let badge = if dur - span <= 2.0 * tun.tolerance_s {
            Badge::Matched
        } else {
            Badge::Likely
        };
        out.push(ClipMatch {
            badge,
            start: Some(first.start),
            segments: k - j + 1,
            span_s: span,
            first: j,
        });
        j = k + 1;
    }
    out
}

/// A log dated before 2020 (2000-01-01 after an RTC reset) or far from the import date.
pub fn day_is_plausible(day: NaiveDate, import_day: NaiveDate, tun: &Tunables) -> bool {
    day.year() >= 2020 && (import_day - day).num_days().abs() <= tun.max_log_age_days
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(&format!("2026-09-30 {s}"), "%Y-%m-%d %H:%M:%S").unwrap()
    }

    /// Rows every 0.5 s from `a` to `b`.
    fn rows(a: &str, b: &str) -> Vec<NaiveDateTime> {
        let (a, b) = (t(a), t(b));
        let mut v = Vec::new();
        let mut x = a;
        while x <= b {
            v.push(x);
            x += Duration::milliseconds(500);
        }
        v
    }

    #[test]
    fn segments_and_sessions() {
        let mut r = rows("10:00:00", "10:03:00");
        r.extend(rows("10:05:00", "10:08:00")); // 2 min gap: new segment, same session
        r.extend(rows("11:00:00", "11:02:00")); // 52 min gap: new session
        let s = segments(&r, &Tunables::default());
        assert_eq!(s.len(), 3);
        assert_eq!((s[0].session, s[1].session, s[2].session), (0, 0, 1));
        assert_eq!(s[0].secs(), 180.0);
    }

    #[test]
    fn matching_badges() {
        let tun = Tunables::default();
        let mut r = rows("10:00:00", "10:03:00");
        r.extend(rows("10:05:00", "10:08:00"));
        r.extend(rows("11:00:00", "11:02:00"));
        let s = segments(&r, &tun);
        // Clip 1: two flights in one file, 8 min 20 s long (span 8:00) -> matched, 2 segments.
        // Clip 2: a 10 s bench clip -> unmatched, next segment stays.
        // Clip 3: 6 min for a 2 min flight -> likely.
        // Clip 4: nothing left -> unmatched.
        let m = match_clips(&[500.0, 10.0, 360.0, 60.0], &s, &tun);
        assert_eq!(m[0].badge, Badge::Matched);
        assert_eq!(m[0].segments, 2);
        assert_eq!(m[0].start, Some(t("10:00:00")));
        assert_eq!(m[1].badge, Badge::Unmatched);
        assert_eq!(m[2].badge, Badge::Likely);
        assert_eq!(m[2].start, Some(t("11:00:00")));
        assert_eq!(m[3].badge, Badge::Unmatched);
    }

    #[test]
    fn rtc_reset_is_implausible() {
        let tun = Tunables::default();
        let today = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        assert!(!day_is_plausible(
            NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(),
            today,
            &tun
        ));
        assert!(!day_is_plausible(
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            today,
            &tun
        ));
        assert!(day_is_plausible(
            NaiveDate::from_ymd_opt(2026, 9, 28).unwrap(),
            today,
            &tun
        ));
    }

    #[test]
    fn parses_sticks_attitude_and_quoted_columns() {
        let text = "Date,Time,RQly(%),FM,Ptch(rad),Roll(rad),Rud,Ele,Thr,Ail,TxBat(V)\n\
            2026-09-30,10:00:00.000,100,\"AIR*\",-0.02,2.71,-21,0,-1024,1000,7.8\n\
            2026-09-30,10:00:00.100,100,\"A,B\",0.00,0.00,1,2,3,4,7.8\n\
            junk\n";
        let rows = parse_rows(text);
        assert_eq!(rows.len(), 2);
        let s = rows[0].sticks.unwrap();
        assert_eq!((s.ail, s.ele, s.thr, s.rud), (1000.0, 0.0, -1024.0, -21.0));
        assert_eq!(rows[0].roll, Some(2.71));
        // A comma inside a quoted column does not shift the columns after it.
        assert_eq!(rows[1].sticks.unwrap().ail, 4.0);
        // A log without stick columns still gives times.
        let rows = parse_rows("Date,Time,1RSS(dB)\n2026-09-30,10:00:00.000,-50\n");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].sticks.is_none());
    }

    #[test]
    fn parses_telemetry_and_channel_columns() {
        let text =
            "Date,Time,TPWR(mW),RSNR(dB),Curr(A),Capa(mAh),Bat%(%),FM,CH1(us),CH3(us),TxBat(V)\n\
            2026-09-30,10:00:00.000,100,9,12.5,250,80,ACRO,1500,1000,7.9\n\
            2026-09-30,10:00:00.100,,,,,,,,,\n\
            2026-09-30,10:00:00.200,25,9,0.1,250,80,ACRO*,1500,988,7.9\n";
        let rows = parse_rows(text);
        assert_eq!(rows.len(), 3);
        let r = &rows[0];
        assert_eq!(r.tx_power_mw, Some(100.0));
        assert_eq!(r.snr_db, Some(9.0));
        assert_eq!(r.current_a, Some(12.5));
        assert_eq!(r.capacity_mah, Some(250.0));
        assert_eq!(r.bat_pct, Some(80.0));
        assert_eq!(r.tx_bat, Some(7.9));
        assert_eq!(r.flight_mode.as_deref(), Some("ACRO"));
        assert_eq!(r.channels.len(), 3);
        assert_eq!((r.channels[0], r.channels[2]), (1500.0, 1000.0));
        assert!(r.channels[1].is_nan());
        assert!(r.armed());
        // A blank flight mode is there but empty, and still counts as armed.
        assert_eq!(rows[1].flight_mode.as_deref(), Some(""));
        assert!(rows[1].armed());
        assert_eq!(rows[1].capacity_mah, None);
        assert!(!rows[2].armed());
        // No FM column: every row is armed, as before.
        let rows = parse_rows("Date,Time,RxBt(V)\n2026-09-30,10:00:00.000,4.1\n");
        assert_eq!(rows[0].flight_mode, None);
        assert!(rows[0].armed());
        assert!(rows[0].channels.is_empty());
    }

    #[test]
    fn model_names_from_log_files() {
        assert_eq!(
            model_from_file_name("Model01-2026-09-30-100000.csv").as_deref(),
            Some("Model01")
        );
        assert_eq!(
            model_from_file_name("AIR65 II-2000-01-01.csv").as_deref(),
            Some("AIR65 II")
        );
        assert_eq!(
            model_from_file_name("My-Quad-2026-09-30-100000.csv").as_deref(),
            Some("My-Quad")
        );
        assert_eq!(
            model_from_file_name("Whoop-2026-09-30 (2).csv").as_deref(),
            Some("Whoop"),
            "the store's second copy"
        );
        assert_eq!(
            model_from_file_name("Whoop-2026-09-30-100000 (12).csv").as_deref(),
            Some("Whoop")
        );
        assert_eq!(model_from_file_name("notes.csv"), None);
        assert_eq!(model_from_file_name("2026-09-30.csv"), None);
    }

    #[test]
    fn reads_edgetx_csv() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("LOGS")).unwrap();
        std::fs::write(
            d.path().join("LOGS/Model01-2026-09-30-100000.csv"),
            "Date,Time,1RSS(dB),RQly(%)\n2026-09-30,10:00:00.000,-50,100\n2026-09-30,10:00:00.500,-50,100\n",
        )
        .unwrap();
        let days = read_log_dir(d.path());
        assert_eq!(days.len(), 1);
        assert_eq!(days.values().next().unwrap().len(), 2);
    }
}
