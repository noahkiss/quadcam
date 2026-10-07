//! Flight analysis from EdgeTX radio logs (`docs/gear-design.md` 7.6).
//!
//! A flight is a run of armed log rows with no gap over `Tunables::segment_gap_s`: the
//! armed segments `logs::segments` finds, split again where the flight mode says disarmed
//! (`LogRow::armed`). A log with no `FM` column gives the same flights as `logs::segments`.
//!
//! Each flight gets the measures of 7.6: hover throttle, sag, resting voltage, mAh at
//! landing, the mAh steps (for any threshold), the worst link, and dropouts. `index` keeps a
//! cache of them per log file in `<gear>/flights.json`, which any run can rebuild.

pub mod synth;

use crate::logs::{self, LogRow, Tunables};
use anyhow::{Context, Result};
use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};

/// A hover window is at least this long, in seconds.
pub const HOVER_MIN_S: f64 = 2.0;
/// In a hover window the throttle varies by less than this, in percent.
pub const HOVER_THROTTLE_SPREAD: f64 = 3.0;
/// In a hover window roll and pitch sit within this share of full stick of centre.
pub const HOVER_STICK_CENTRE: f64 = 0.05;
/// A dropout lasts at least this long, in seconds.
pub const DROPOUT_MIN_S: f64 = 0.3;
/// After a dropout, a failsafe mode this many seconds later still counts against it.
pub const FAILSAFE_WINDOW_S: f64 = 2.0;

/// Median throttle in hover, in percent (0-100): over the whole flight, its first third and
/// its last third. None where the flight has no hover window.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Type)]
pub struct Hover {
    pub all: Option<f64>,
    pub early: Option<f64>,
    pub late: Option<f64>,
}

/// Where a flight's resting voltage came from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum RestingFrom {
    /// The median of the readings after disarm, while telemetry still streamed.
    AfterDisarm,
    /// The first reading of the next flight of the same model.
    NextArm,
}

/// The link at one edge of a dropout.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Type)]
pub struct LinkEdge {
    /// `RQly(%)`.
    pub lq: Option<f64>,
    /// `1RSS(dB)`.
    pub rssi_db: Option<f64>,
    /// `TPWR(mW)`.
    pub tx_power_mw: Option<f64>,
}

/// A span while armed where the flight mode is blank and the receiver battery reads 0: the
/// radio heard nothing from the quad.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct Dropout {
    /// Seconds from the flight's start.
    pub start_s: f64,
    pub secs: f64,
    /// The last row before it and the first row after it.
    pub before: LinkEdge,
    pub after: Option<LinkEdge>,
    /// The flight went on and no failsafe mode showed: only the downlink (telemetry) was
    /// lost, the quad still flew.
    pub downlink_only: bool,
}

/// The mAh used reached `mah` at `s` seconds into the flight.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Type)]
pub struct CapaMark {
    pub mah: f64,
    pub s: f64,
}

/// One flight's measures.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct Flight {
    /// `<YYYYMMDDTHHMMSS>-<model>`: the start and the EdgeTX model. Stable across rebuilds.
    pub id: String,
    /// The EdgeTX model name, from the log file's name.
    pub model: Option<String>,
    /// The log file.
    pub file: PathBuf,
    pub day: NaiveDate,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub secs: f64,
    pub hover: Hover,
    /// `RxBt` while armed: the 5th percentile (nearest rank) and the minimum. 0 V readings
    /// (no telemetry) do not count.
    pub sag_p5_v: Option<f64>,
    pub sag_min_v: Option<f64>,
    /// The first `RxBt` reading of the flight.
    pub arm_v: Option<f64>,
    pub resting_v: Option<f64>,
    pub resting_from: Option<RestingFrom>,
    /// `Capa` at disarm.
    pub mah: Option<f64>,
    /// Each time the mAh used went up: the threshold crossings come from these.
    #[serde(default)]
    pub capa: Vec<CapaMark>,
    pub max_current_a: Option<f64>,
    /// The worst `RQly` and `1RSS` while armed (0 and positive readings are no telemetry).
    pub worst_lq: Option<f64>,
    pub worst_rssi_db: Option<f64>,
    pub max_tx_power_mw: Option<f64>,
    /// Empty when the log has no `FM` column (dropouts cannot be told apart then).
    pub dropouts: Vec<Dropout>,
}

impl Flight {
    /// The second into the flight when the mAh used first reached `threshold`.
    pub fn crossed(&self, threshold: f64) -> Option<f64> {
        self.capa.iter().find(|m| m.mah >= threshold).map(|m| m.s)
    }
}

fn secs(a: NaiveDateTime, b: NaiveDateTime) -> f64 {
    (b - a).num_milliseconds() as f64 / 1000.0
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// The median; None for no values.
pub fn median(v: &[f64]) -> Option<f64> {
    let mut v: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

/// The nearest-rank percentile `p` (0-100); None for no values.
pub fn percentile(v: &[f64], p: f64) -> Option<f64> {
    let mut v: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let rank = ((p / 100.0) * v.len() as f64).ceil() as usize;
    Some(v[rank.clamp(1, v.len()) - 1])
}

fn min_of(v: impl Iterator<Item = f64>) -> Option<f64> {
    v.fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.min(x))))
}

fn max_of(v: impl Iterator<Item = f64>) -> Option<f64> {
    v.fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.max(x))))
}

/// A row's receiver battery when it is a reading (above 0 V).
fn volts(r: &LogRow) -> Option<f64> {
    r.rx_bat.filter(|v| *v > 0.0)
}

/// Throttle in percent, 0 at the bottom.
fn throttle_pct(r: &LogRow) -> Option<f64> {
    r.sticks
        .map(|s| ((s.thr + 1024.0) / 2048.0 * 100.0).clamp(0.0, 100.0))
}

fn centred(r: &LogRow) -> bool {
    let lim = 1024.0 * HOVER_STICK_CENTRE;
    r.sticks
        .is_some_and(|s| s.ail.abs() <= lim && s.ele.abs() <= lim)
}

/// The flight-mode column is there but blank and the receiver battery reads 0.
fn silent(r: &LogRow) -> bool {
    r.flight_mode.as_deref() == Some("") && r.rx_bat.unwrap_or(0.0) == 0.0
}

fn edge(r: &LogRow) -> LinkEdge {
    LinkEdge {
        lq: r.lq,
        rssi_db: r.rssi,
        tx_power_mw: r.tx_power_mw,
    }
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "log".into()
    } else {
        out
    }
}

/// The armed runs of `rows` (time order), as inclusive index ranges.
pub fn runs(rows: &[LogRow], tun: &Tunables) -> Vec<(usize, usize)> {
    let gap = tun.segment_gap_s;
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        if !r.armed() {
            continue;
        }
        match out.last_mut() {
            Some(run) if run.1 + 1 == i && secs(rows[run.1].time, r.time) <= gap => run.1 = i,
            _ => out.push((i, i)),
        }
    }
    out
}

/// Hover rows: the rows inside windows of `HOVER_MIN_S` or more where roll and pitch sit
/// centred and the throttle varies by less than `HOVER_THROTTLE_SPREAD`.
fn hover_rows<'a>(rows: &'a [LogRow]) -> Vec<&'a LogRow> {
    let mut out = Vec::new();
    let mut w: Vec<&LogRow> = Vec::new();
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    let close = |w: &mut Vec<&'a LogRow>, out: &mut Vec<&'a LogRow>| {
        if let (Some(a), Some(b)) = (w.first(), w.last()) {
            if secs(a.time, b.time) >= HOVER_MIN_S - 1e-9 {
                out.append(w);
            }
        }
        w.clear();
    };
    for r in rows {
        let (Some(t), true) = (throttle_pct(r), centred(r)) else {
            close(&mut w, &mut out);
            (lo, hi) = (f64::INFINITY, f64::NEG_INFINITY);
            continue;
        };
        if !w.is_empty() && hi.max(t) - lo.min(t) >= HOVER_THROTTLE_SPREAD {
            close(&mut w, &mut out);
            (lo, hi) = (f64::INFINITY, f64::NEG_INFINITY);
        }
        lo = lo.min(t);
        hi = hi.max(t);
        w.push(r);
    }
    close(&mut w, &mut out);
    out
}

/// Every flight in one log's rows (one file, time order). `file` and `model` tag them.
/// Resting voltage from the next arm is filled by `fill_resting`, which sees every file.
pub fn analyse(rows: &[LogRow], model: Option<&str>, file: &Path, tun: &Tunables) -> Vec<Flight> {
    let has_fm = rows.iter().any(|r| r.flight_mode.is_some());
    let mut out = Vec::new();
    for (a, b) in runs(rows, tun) {
        let run = &rows[a..=b];
        let (start, end) = (run[0].time, run[run.len() - 1].time);
        let len = secs(start, end);
        let at = |t: NaiveDateTime| round3(secs(start, t));

        let hov = hover_rows(run);
        let thr = |f: &dyn Fn(&LogRow) -> bool| -> Option<f64> {
            let v: Vec<f64> = hov
                .iter()
                .filter(|r| f(r))
                .filter_map(|r| throttle_pct(r))
                .collect();
            median(&v)
        };
        let third = len / 3.0;
        let hover = Hover {
            all: thr(&|_| true),
            early: thr(&|r| secs(start, r.time) <= third),
            late: thr(&|r| secs(r.time, end) <= third),
        };

        let v: Vec<f64> = run.iter().filter_map(volts).collect();

        // The readings after disarm, while rows keep coming.
        let mut tail = Vec::new();
        let mut prev = end;
        for r in &rows[b + 1..] {
            if r.armed() || secs(prev, r.time) > tun.segment_gap_s {
                break;
            }
            prev = r.time;
            tail.extend(volts(r));
        }
        let resting_v = median(&tail);

        let mut capa: Vec<CapaMark> = Vec::new();
        for r in run {
            if let Some(c) = r.capacity_mah {
                if capa.last().is_none_or(|m| c > m.mah) {
                    capa.push(CapaMark {
                        mah: c,
                        s: at(r.time),
                    });
                }
            }
        }

        let mut dropouts = Vec::new();
        if has_fm {
            let mut i = 0;
            while i < run.len() {
                if !silent(&run[i]) {
                    i += 1;
                    continue;
                }
                let s = i;
                while i < run.len() && silent(&run[i]) {
                    i += 1;
                }
                let after = run.get(i);
                let t_end = after.map_or(run[i - 1].time, |r| r.time);
                let length = round3(secs(run[s].time, t_end));
                if length + 1e-9 < DROPOUT_MIN_S {
                    continue;
                }
                let before = if s > 0 {
                    Some(&run[s - 1])
                } else {
                    rows[..a].last()
                };
                let failsafe = run[s..].iter().any(|r| {
                    secs(t_end, r.time) <= FAILSAFE_WINDOW_S
                        && r.flight_mode.as_deref().is_some_and(|m| m.contains("FS"))
                });
                dropouts.push(Dropout {
                    start_s: at(run[s].time),
                    secs: length,
                    before: before.map(edge).unwrap_or_default(),
                    after: after.map(edge),
                    downlink_only: after.is_some() && !failsafe,
                });
            }
        }

        out.push(Flight {
            id: format!(
                "{}-{}",
                start.format("%Y%m%dT%H%M%S"),
                slug(model.unwrap_or(""))
            ),
            model: model.map(str::to_string),
            file: file.to_path_buf(),
            day: start.date(),
            start,
            end,
            secs: round3(len),
            hover,
            sag_p5_v: percentile(&v, 5.0),
            sag_min_v: min_of(v.iter().copied()),
            arm_v: v.first().copied(),
            resting_from: resting_v.map(|_| RestingFrom::AfterDisarm),
            resting_v,
            mah: run.iter().rev().find_map(|r| r.capacity_mah),
            capa,
            max_current_a: max_of(run.iter().filter_map(|r| r.current_a)),
            worst_lq: min_of(run.iter().filter_map(|r| r.lq.filter(|v| *v > 0.0))),
            worst_rssi_db: min_of(run.iter().filter_map(|r| r.rssi.filter(|v| *v < 0.0))),
            max_tx_power_mw: max_of(run.iter().filter_map(|r| r.tx_power_mw)),
            dropouts,
        });
    }
    out
}

/// Gives a flight with no readings after disarm the first reading of the next flight of
/// the same model, when that flight starts the same day. `flights` is in time order.
pub fn fill_resting(flights: &mut [Flight]) {
    for i in 0..flights.len() {
        if flights[i].resting_v.is_some() {
            continue;
        }
        let next = flights[i + 1..]
            .iter()
            .find(|f| f.model == flights[i].model && f.day == flights[i].day)
            .and_then(|f| f.arm_v);
        if let Some(v) = next {
            flights[i].resting_v = Some(v);
            flights[i].resting_from = Some(RestingFrom::NextArm);
        }
    }
}

/// The flights of one log file.
pub fn analyse_file(path: &Path, tun: &Tunables) -> Result<Vec<Flight>> {
    let rows = logs::read_rows(path)?;
    let model = rows.first().and_then(|r| r.model.clone());
    Ok(analyse(&rows, model.as_deref(), path, tun))
}

/// `<gear>/flights.json`: each log file's flights, by path, size and mtime.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cache {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub files: Vec<CacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub path: PathBuf,
    pub len: u64,
    pub mtime_ms: i64,
    pub flights: Vec<Flight>,
}

/// Bumped when a measure changes, so an old cache is rebuilt.
pub const CACHE_VERSION: u32 = 1;

fn stamp(p: &Path) -> Option<(u64, i64)> {
    let m = std::fs::metadata(p).ok()?;
    let t = m
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;
    Some((m.len(), t))
}

/// Every log file under the sources: a folder of CSVs, a radio's root (its `LOGS/`), or a
/// folder of such folders (the log store, one folder per radio).
pub fn log_files(sources: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for s in sources {
        let direct = logs::csv_files(s);
        if direct.is_empty() {
            if let Ok(rd) = std::fs::read_dir(s) {
                let mut subs: Vec<PathBuf> = rd
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect();
                subs.sort();
                for d in subs {
                    out.extend(logs::csv_files(&d));
                }
            }
        } else {
            out.extend(direct);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Every flight in the logs under `sources`, in time order, one per id (the same log in two
/// places counts once). Files the cache knows by size and mtime are not read again; the
/// cache is rewritten when anything changed (a write failure only costs the next read).
pub fn index(sources: &[PathBuf], cache_file: &Path, tun: &Tunables) -> Result<Vec<Flight>> {
    let old: Cache = std::fs::read(cache_file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .filter(|c: &Cache| c.version == CACHE_VERSION)
        .unwrap_or_default();
    let mut cache = Cache {
        version: CACHE_VERSION,
        files: Vec::new(),
    };
    let mut changed = false;
    for f in log_files(sources) {
        let Some((len, mtime_ms)) = stamp(&f) else {
            continue;
        };
        if let Some(e) = old
            .files
            .iter()
            .find(|e| e.path == f && e.len == len && e.mtime_ms == mtime_ms)
        {
            cache.files.push(e.clone());
            continue;
        }
        changed = true;
        let flights = analyse_file(&f, tun).unwrap_or_default();
        cache.files.push(CacheEntry {
            path: f,
            len,
            mtime_ms,
            flights,
        });
    }
    changed |= cache.files.len() != old.files.len();
    if changed {
        let _ = write_cache(cache_file, &cache);
    }
    let mut all: Vec<Flight> = Vec::new();
    for e in cache.files {
        for fl in e.flights {
            if !all.iter().any(|x| x.id == fl.id) {
                all.push(fl);
            }
        }
    }
    all.sort_by(|a, b| a.start.cmp(&b.start).then(a.id.cmp(&b.id)));
    fill_resting(&mut all);
    Ok(all)
}

fn write_cache(path: &Path, cache: &Cache) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(cache)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::synth::{self, KNOWN};
    use super::*;

    fn known() -> Vec<Flight> {
        let d = tempfile::tempdir().unwrap();
        let p = synth::write_known(d.path()).unwrap();
        let mut f = analyse_file(&p, &Tunables::default()).unwrap();
        fill_resting(&mut f);
        f
    }

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-6)
    }

    #[test]
    fn finds_the_known_flights() {
        let f = known();
        assert_eq!(f.len(), KNOWN.flights);
        assert_eq!(f[0].secs, KNOWN.f1_secs);
        assert_eq!(f[0].id, "20261004T100002-whoop");
        assert_eq!(f[0].model.as_deref(), Some("Whoop"));
        // The disarmed rows before the first arm are not part of it.
        assert_eq!(f[0].start.format("%H:%M:%S").to_string(), "10:00:02");
    }

    #[test]
    fn hover_throttle_all_early_late() {
        let f = known();
        let h = f[0].hover;
        assert!(close(h.all, KNOWN.hover_all), "{h:?}");
        assert!(close(h.early, KNOWN.hover_early), "{h:?}");
        assert!(close(h.late, KNOWN.hover_late), "{h:?}");
        // Flight 2 never sits centred: no hover.
        assert_eq!(f[1].hover, Hover::default());
    }

    #[test]
    fn sag_p5_and_min() {
        let f = known();
        assert!(close(f[0].sag_p5_v, KNOWN.sag_p5), "{:?}", f[0].sag_p5_v);
        assert!(close(f[0].sag_min_v, KNOWN.sag_min));
    }

    #[test]
    fn resting_after_disarm_else_next_arm() {
        let f = known();
        assert!(close(f[0].resting_v, KNOWN.f1_resting));
        assert_eq!(f[0].resting_from, Some(RestingFrom::AfterDisarm));
        // Flight 2's log stops at disarm: the next flight's first reading.
        assert!(close(f[1].resting_v, KNOWN.f2_resting));
        assert_eq!(f[1].resting_from, Some(RestingFrom::NextArm));
        // The last flight has neither.
        assert_eq!(f[2].resting_v, None);
    }

    #[test]
    fn mah_at_landing_and_threshold() {
        let f = known();
        assert!(close(f[0].mah, KNOWN.f1_mah));
        assert!(close(f[0].crossed(KNOWN.threshold), KNOWN.threshold_at_s));
        assert_eq!(f[0].crossed(10_000.0), None);
    }

    #[test]
    fn dropouts_with_edges() {
        let f = known();
        assert_eq!(f[0].dropouts.len(), 1, "{:?}", f[0].dropouts);
        let d = &f[0].dropouts[0];
        assert!((d.start_s - KNOWN.drop_start_s).abs() < 1e-6);
        assert!((d.secs - KNOWN.drop_secs).abs() < 1e-6);
        assert_eq!(d.before.lq, Some(KNOWN.drop_before_lq));
        assert_eq!(d.before.rssi_db, Some(KNOWN.drop_before_rssi));
        assert_eq!(d.before.tx_power_mw, Some(KNOWN.drop_before_tpwr));
        let a = d.after.unwrap();
        assert_eq!(a.lq, Some(KNOWN.drop_after_lq));
        assert_eq!(a.tx_power_mw, Some(KNOWN.drop_after_tpwr));
        assert!(d.downlink_only);
        // Flight 3 ends in a failsafe: not downlink only.
        let d3 = f[2].dropouts.last().unwrap();
        assert!(!d3.downlink_only);
        assert!(d3.after.is_none());
    }

    #[test]
    fn worst_link_and_current() {
        let f = known();
        assert_eq!(f[0].worst_lq, Some(KNOWN.drop_before_lq));
        assert_eq!(f[0].worst_rssi_db, Some(KNOWN.drop_before_rssi));
        assert_eq!(f[0].max_tx_power_mw, Some(KNOWN.drop_after_tpwr));
        assert_eq!(f[0].max_current_a, Some(KNOWN.max_current));
    }

    #[test]
    fn no_fm_column_gives_the_old_segments() {
        let d = tempfile::tempdir().unwrap();
        let p = synth::write_known(d.path()).unwrap();
        let mut rows = logs::read_rows(&p).unwrap();
        for r in &mut rows {
            r.flight_mode = None;
        }
        let tun = Tunables::default();
        let f = analyse(&rows, Some("Whoop"), &p, &tun);
        let times: Vec<NaiveDateTime> = rows.iter().map(|r| r.time).collect();
        let segs = logs::segments(&times, &tun);
        assert_eq!(f.len(), segs.len());
        for (a, b) in f.iter().zip(&segs) {
            assert_eq!((a.start, a.end), (b.start, b.end));
            assert!(a.dropouts.is_empty());
        }
    }

    #[test]
    fn index_caches_and_dedupes() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        let b = d.path().join("store/radio1");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        synth::write_known(&a).unwrap();
        synth::write_known(&b).unwrap();
        let cache = d.path().join("flights.json");
        let tun = Tunables::default();
        let srcs = vec![a.clone(), d.path().join("store")];
        let f = index(&srcs, &cache, &tun).unwrap();
        assert_eq!(f.len(), KNOWN.flights);
        assert!(cache.is_file());
        let c: Cache = serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
        assert_eq!(c.files.len(), 2);
        // A second run reads the cache, with the same answer.
        assert_eq!(index(&srcs, &cache, &tun).unwrap(), f);
    }

    #[test]
    fn percentile_and_median() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[1.0, 2.0, 3.0, 4.0]), Some(2.5));
        assert_eq!(median(&[]), None);
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&v, 5.0), Some(5.0));
        assert_eq!(percentile(&[7.0], 5.0), Some(7.0));
    }
}
