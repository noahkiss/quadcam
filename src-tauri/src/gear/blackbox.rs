//! Pulled blackbox flash images (design 7.12): the raw image sits in the blob store
//! (`blobs`), and one small JSON record per pull sits in `<gear>/blackbox/<device>/`. A
//! record names the blob, the aircraft, the day, the pull method, the logs found in the
//! headers and whether the flash was erased afterwards.
//!
//! - Records are plain files named `<YYYY-MM-DDTHHMMSS>.json` (UTC). The record id is
//!   `<device>/<YYYY-MM-DDTHHMMSS>`, like a backup's.
//! - A blob that a record names is never collected (`keys`; `backup::Snapshots::prune`
//!   adds it to the blobs it keeps).
//! - The FC has no clock, so a log's own date is usually `0000-01-01`. The day of a pull is
//!   the day it ran. `link` pairs a pull's logs with the radio-log flights by order, and
//!   labels the pairing a guess.

use super::bf::blackbox::LogInfo;
use super::blobs::BlobRef;
use super::store::{safe, Store};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashSet;
use std::path::PathBuf;

/// A log shorter than this is a test arm, not a flight: it is not paired with a flight.
pub const MIN_FLIGHT_LOG_BYTES: u64 = 32 * 1024;

/// How the image was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// MSP over the serial port (about 84 KB/s).
    Msp,
    /// The FC in USB disk mode (`msc`). Not proven on a real FC.
    Msc,
}

impl Method {
    pub fn label(self) -> &'static str {
        match self {
            Method::Msp => "MSP",
            Method::Msc => "USB disk mode",
        }
    }
}

/// One pull.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Pull {
    /// `<device>/<YYYY-MM-DDTHHMMSS>`.
    pub id: String,
    /// The FC's device id.
    pub device: String,
    /// The aircraft profile the device is linked to, when it is.
    #[serde(default)]
    pub aircraft: Option<String>,
    pub pulled_at: DateTime<Utc>,
    /// The local day of the pull.
    pub day: NaiveDate,
    pub method: Method,
    pub blob: BlobRef,
    /// Bytes the flash reported used, and its size.
    pub used: u64,
    pub total: u64,
    pub logs: Vec<LogInfo>,
    /// The firmware of the first log, and the craft name of the first log that has one.
    #[serde(default)]
    pub firmware: Option<String>,
    #[serde(default)]
    pub craft: Option<String>,
    /// The flash was erased after the pull verified.
    #[serde(default)]
    pub erased: bool,
    /// Why the flash was not erased, or what the erase did.
    #[serde(default)]
    pub erase_note: Option<String>,
}

impl Pull {
    /// The id part after the device.
    pub fn stamp(&self) -> &str {
        self.id.rsplit('/').next().unwrap_or(&self.id)
    }
}

/// The pulls of a gear folder.
#[derive(Debug, Clone)]
pub struct Pulls {
    store: Store,
}

/// The stamp in an id: `2026-10-07T120000`.
pub fn stamp(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H%M%S").to_string()
}

impl Pulls {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    fn dir(&self, device: &str) -> PathBuf {
        self.store.blackbox_dir(device)
    }

    fn path(&self, id: &str) -> Result<PathBuf> {
        let (device, stamp) = id
            .rsplit_once('/')
            .with_context(|| format!("{id:?} is not a blackbox pull id (<device>/<time>)"))?;
        if stamp.is_empty() || stamp.contains(['/', '\\', '.']) {
            bail!("{id:?} is not a blackbox pull id");
        }
        Ok(self.dir(device).join(format!("{stamp}.json")))
    }

    /// Writes (or replaces) a pull's record: a temporary file, renamed into place.
    pub fn save(&self, p: &Pull) -> Result<()> {
        let path = self.path(&p.id)?;
        let dir = path.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(p)?)
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    /// The pull with this id.
    pub fn get(&self, id: &str) -> Result<Pull> {
        let path = self.path(id)?;
        let bytes = std::fs::read(&path).with_context(|| format!("No blackbox pull {id:?}."))?;
        serde_json::from_slice(&bytes).with_context(|| format!("reading {}", path.display()))
    }

    /// One device's pulls, oldest first. A record that does not parse is skipped.
    pub fn list(&self, device: &str) -> Vec<Pull> {
        let mut out: Vec<Pull> = std::fs::read_dir(self.dir(device))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
            .collect();
        out.sort_by_key(|p: &Pull| p.pulled_at);
        out
    }

    /// Every device's pulls, oldest first.
    pub fn all(&self) -> Vec<Pull> {
        let mut out: Vec<Pull> = std::fs::read_dir(self.store.root().join("blackbox"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .flat_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                self.list(&name)
            })
            .collect();
        out.sort_by_key(|p| p.pulled_at);
        out
    }

    /// The newest pull of a device.
    pub fn latest(&self, device: &str) -> Option<Pull> {
        self.list(device).pop()
    }
}

/// The blobs the records name.
pub fn keys(store: &Store) -> HashSet<String> {
    Pulls::new(store.clone())
        .all()
        .iter()
        .map(|p| p.blob.key())
        .collect()
}

/// A file name for an export: `<craft-or-device>_<stamp>`.
pub fn export_stem(p: &Pull) -> String {
    let who = p
        .craft
        .clone()
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| safe(&p.device));
    format!("{}_{}", safe(&who), p.stamp())
}

/// A flight a pull's log may belong to.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub secs: f64,
}

/// One log paired with one flight.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct FlightLink {
    /// The log's number in the image (1-based).
    pub log: u32,
    pub log_bytes: u64,
    pub flight: String,
    pub flight_secs: f64,
    /// Bytes per second of flight: a log and its flight agree when these are alike across
    /// pairs. None with fewer than three pairs.
    pub fits: Option<bool>,
}

/// The pairing of a pull's logs with flights, and what it left over.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Linked {
    pub links: Vec<FlightLink>,
    /// Flight logs (not test arms) with no flight to pair with.
    pub unpaired_logs: u32,
    /// Candidate flights with no log.
    pub unpaired_flights: u32,
    /// Logs shorter than `MIN_FLIGHT_LOG_BYTES`.
    pub short_logs: u32,
    /// Always says this is a guess and how it was made.
    pub note: String,
}

/// Pairs the flight-sized logs of a pull with `flights` (oldest first) by order, the newest
/// log with the newest flight. Where the counts differ the older ones stay unpaired: a flash
/// that was not erased holds older logs, and a radio may have missed a flight. `fits`
/// compares each pair's bytes per second with the median pair's (within half to double).
pub fn link(logs: &[LogInfo], flights: &[Candidate]) -> Linked {
    let flight_logs: Vec<&LogInfo> = logs
        .iter()
        .filter(|l| l.size >= MIN_FLIGHT_LOG_BYTES)
        .collect();
    let short_logs = (logs.len() - flight_logs.len()) as u32;
    let k = flight_logs.len().min(flights.len());
    let (lo, fo) = (flight_logs.len() - k, flights.len() - k);
    let mut links: Vec<FlightLink> = (0..k)
        .map(|i| FlightLink {
            log: flight_logs[lo + i].index,
            log_bytes: flight_logs[lo + i].size,
            flight: flights[fo + i].id.clone(),
            flight_secs: flights[fo + i].secs,
            fits: None,
        })
        .collect();
    if k >= 3 {
        let rate = |l: &FlightLink| l.log_bytes as f64 / l.flight_secs.max(1.0);
        let mut rates: Vec<f64> = links.iter().map(rate).collect();
        rates.sort_by(f64::total_cmp);
        let median = rates[rates.len() / 2];
        for l in &mut links {
            let r = rate(l);
            l.fits = Some(r >= median * 0.5 && r <= median * 2.0);
        }
    }
    let note = "Guess: the FC has no clock, so logs are paired with the radio's flights by order, newest with newest. Test arms under 32 KB are skipped. Check the sizes before you trust a pair.".to_string();
    Linked {
        links,
        unpaired_logs: (lo) as u32,
        unpaired_flights: fo as u32,
        short_logs,
        note,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn pull(device: &str, t: DateTime<Utc>) -> Pull {
        Pull {
            id: format!("{device}/{}", stamp(t)),
            device: device.into(),
            aircraft: Some("Meteor75".into()),
            pulled_at: t,
            day: t.date_naive(),
            method: Method::Msp,
            blob: BlobRef {
                xxh64: "0123456789abcdef".into(),
                size: 10,
            },
            used: 10,
            total: 100,
            logs: Vec::new(),
            firmware: None,
            craft: Some("Meteor 75".into()),
            erased: false,
            erase_note: None,
        }
    }

    fn log(index: u32, size: u64) -> LogInfo {
        LogInfo {
            index,
            offset: 0,
            size,
            firmware: Some("Betaflight".into()),
            craft: None,
            start: None,
            dated: false,
            looptime_us: Some(125),
            headers: 8,
        }
    }

    fn cand(id: &str, secs: f64) -> Candidate {
        Candidate {
            id: id.into(),
            secs,
        }
    }

    #[test]
    fn records_round_trip_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let p = Pulls::new(Store::new(dir.path()));
        let a = pull("fc-1", Utc.with_ymd_and_hms(2026, 10, 7, 12, 0, 0).unwrap());
        let b = pull("fc-1", Utc.with_ymd_and_hms(2026, 10, 8, 9, 30, 0).unwrap());
        p.save(&b).unwrap();
        p.save(&a).unwrap();
        assert_eq!(p.list("fc-1"), vec![a.clone(), b.clone()]);
        assert_eq!(p.latest("fc-1"), Some(b.clone()));
        assert_eq!(p.get(&a.id).unwrap(), a);
        assert_eq!(a.id, "fc-1/2026-10-07T120000");
        assert!(p.get("fc-1/../x").is_err());
        assert!(p.get("nothing").is_err());
        assert_eq!(p.all().len(), 2);
        p.save(&pull(
            "fc-2",
            Utc.with_ymd_and_hms(2026, 10, 7, 13, 0, 0).unwrap(),
        ))
        .unwrap();
        assert_eq!(p.all().len(), 3);
        assert_eq!(keys(&Store::new(dir.path())).len(), 1, "all name one blob");
        assert_eq!(export_stem(&a), "Meteor_75_2026-10-07T120000");
    }

    #[test]
    fn links_pair_the_newest_by_order_and_say_guess() {
        let logs = [
            log(1, 200_000),
            log(2, 4_000),
            log(3, 300_000),
            log(4, 100_000),
        ];
        let flights = [cand("f1", 40.0), cand("f2", 60.0), cand("f3", 20.0)];
        let l = link(&logs, &flights);
        assert_eq!(l.short_logs, 1, "log 2 is a test arm");
        // Three flight logs, three flights: log 1-f1, 3-f2, 4-f3.
        let pairs: Vec<(u32, &str)> = l.links.iter().map(|x| (x.log, x.flight.as_str())).collect();
        assert_eq!(pairs, vec![(1, "f1"), (3, "f2"), (4, "f3")]);
        assert_eq!((l.unpaired_logs, l.unpaired_flights), (0, 0));
        assert!(l.note.starts_with("Guess"));
        // 5000, 5000 and 5000 bytes per second: all fit.
        assert!(l.links.iter().all(|x| x.fits == Some(true)));
    }

    #[test]
    fn extra_logs_stay_unpaired_and_a_bad_fit_shows() {
        let logs = [
            log(1, 100_000),
            log(2, 100_000),
            log(3, 100_000),
            log(4, 100_000),
        ];
        let flights = [cand("f1", 20.0), cand("f2", 20.0)];
        let l = link(&logs, &flights);
        assert_eq!((l.unpaired_logs, l.unpaired_flights), (2, 0));
        assert_eq!(l.links[0].log, 3, "the newest two logs");
        assert_eq!(l.links[0].fits, None, "two pairs are too few to judge");
        let flights = [cand("a", 20.0), cand("b", 20.0), cand("c", 400.0)];
        let l = link(&logs[..3], &flights);
        let fits: Vec<_> = l.links.iter().map(|x| x.fits).collect();
        assert_eq!(fits, vec![Some(true), Some(true), Some(false)]);
        // No flights: nothing pairs.
        let l = link(&logs, &[]);
        assert!(l.links.is_empty() && l.unpaired_logs == 4);
    }
}
