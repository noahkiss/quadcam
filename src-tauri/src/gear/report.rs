//! The session report: one flying day (or the days of the last import) in a few numbers,
//! as data and as Markdown to share: flights, air time, the longest flight, the worst
//! link, dropouts, pack use and crashes.

use super::crashes::Crash;
use super::flights::Flight;
use super::packs::FlightSet;
use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// One flight, in short.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct FlightLine {
    pub flight: String,
    pub model: Option<String>,
    pub start: NaiveDateTime,
    pub secs: f64,
}

/// The worst link of the session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct WorstLink {
    pub flight: String,
    pub start: NaiveDateTime,
    pub lq: Option<f64>,
    pub rssi_db: Option<f64>,
}

/// One pack's use in the session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct PackUse {
    pub pack: String,
    pub flights: usize,
    /// The mAh used, summed over its flights.
    pub mah: f64,
}

/// `gear_session_report`'s answer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct SessionReport {
    pub days: Vec<NaiveDate>,
    /// Library clips from the import the report is for (0 for a day picked by hand).
    pub clips: usize,
    pub flights: usize,
    pub air_s: f64,
    pub longest: Option<FlightLine>,
    pub worst_link: Option<WorstLink>,
    pub dropouts: usize,
    /// Of the dropouts, those where the quad flew on (telemetry only).
    pub downlink_only: usize,
    pub packs: Vec<PackUse>,
    /// Flights with no pack assigned.
    pub no_pack: usize,
    pub crashes: Vec<Crash>,
    /// The same, as Markdown.
    pub markdown: String,
}

/// `m:ss`.
pub fn mmss(s: f64) -> String {
    let s = s.round().max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The report for `days` from their flights (any order), the person's flight sets and
/// crashes, and the number of clips imported.
pub fn build(
    days: &[NaiveDate],
    flights: &[Flight],
    sets: &BTreeMap<String, FlightSet>,
    crashes: &[Crash],
    clips: usize,
) -> SessionReport {
    let mut fl: Vec<&Flight> = flights.iter().filter(|f| days.contains(&f.day)).collect();
    fl.sort_by_key(|f| f.start);
    let line = |f: &Flight| FlightLine {
        flight: f.id.clone(),
        model: f.model.clone(),
        start: f.start,
        secs: f.secs,
    };
    let longest = fl
        .iter()
        .max_by(|a, b| a.secs.total_cmp(&b.secs))
        .map(|f| line(f));
    let worst_link = fl
        .iter()
        .filter(|f| f.worst_lq.is_some() || f.worst_rssi_db.is_some())
        .min_by(|a, b| {
            let k = |f: &Flight| {
                (
                    f.worst_lq.unwrap_or(f64::INFINITY),
                    f.worst_rssi_db.unwrap_or(f64::INFINITY),
                )
            };
            let (x, y) = (k(a), k(b));
            x.0.total_cmp(&y.0).then(x.1.total_cmp(&y.1))
        })
        .map(|f| WorstLink {
            flight: f.id.clone(),
            start: f.start,
            lq: f.worst_lq,
            rssi_db: f.worst_rssi_db,
        });
    let mut packs: Vec<PackUse> = Vec::new();
    let mut no_pack = 0;
    for f in &fl {
        match sets.get(&f.id).and_then(|s| s.pack.clone()) {
            Some(p) => match packs.iter_mut().find(|u| u.pack == p) {
                Some(u) => {
                    u.flights += 1;
                    u.mah += f.mah.unwrap_or(0.0);
                }
                None => packs.push(PackUse {
                    pack: p,
                    flights: 1,
                    mah: f.mah.unwrap_or(0.0),
                }),
            },
            None => no_pack += 1,
        }
    }
    packs.sort_by(|a, b| super::packs::natural_cmp(&a.pack, &b.pack));
    let crashes: Vec<Crash> = crashes
        .iter()
        .filter(|c| days.contains(&c.day))
        .cloned()
        .collect();
    let mut r = SessionReport {
        days: days.to_vec(),
        clips,
        flights: fl.len(),
        air_s: fl.iter().map(|f| f.secs).sum(),
        longest,
        worst_link,
        dropouts: fl.iter().map(|f| f.dropouts.len()).sum(),
        downlink_only: fl
            .iter()
            .flat_map(|f| &f.dropouts)
            .filter(|d| d.downlink_only)
            .count(),
        packs,
        no_pack,
        crashes,
        markdown: String::new(),
    };
    r.markdown = markdown(&r);
    r
}

/// The report as Markdown.
pub fn markdown(r: &SessionReport) -> String {
    let days: Vec<String> = r.days.iter().map(|d| d.to_string()).collect();
    let mut s = format!(
        "# Session report: {}\n\n",
        if days.is_empty() {
            "no flights".to_string()
        } else {
            days.join(", ")
        }
    );
    if r.clips > 0 {
        let _ = writeln!(s, "- Clips imported: {}", r.clips);
    }
    let _ = writeln!(s, "- Flights: {}, air time {}", r.flights, mmss(r.air_s));
    if let Some(l) = &r.longest {
        let _ = writeln!(
            s,
            "- Longest flight: {} at {}{}",
            mmss(l.secs),
            l.start.format("%H:%M"),
            l.model
                .as_deref()
                .map(|m| format!(" ({m})"))
                .unwrap_or_default()
        );
    }
    if let Some(w) = &r.worst_link {
        let mut parts = Vec::new();
        if let Some(lq) = w.lq {
            parts.push(format!("LQ {lq:.0} %"));
        }
        if let Some(rssi) = w.rssi_db {
            parts.push(format!("RSSI {rssi:.0} dB"));
        }
        let _ = writeln!(
            s,
            "- Worst link: {} at {}",
            parts.join(", "),
            w.start.format("%H:%M")
        );
    }
    let _ = writeln!(
        s,
        "- Dropouts: {}{}",
        r.dropouts,
        if r.downlink_only > 0 {
            format!(" ({} telemetry only)", r.downlink_only)
        } else {
            String::new()
        }
    );
    if !r.packs.is_empty() || r.no_pack > 0 {
        s.push_str("\n## Packs\n\n| Pack | Flights | mAh |\n|---|---|---|\n");
        for p in &r.packs {
            let _ = writeln!(s, "| {} | {} | {:.0} |", p.pack, p.flights, p.mah);
        }
        if r.no_pack > 0 {
            let _ = writeln!(s, "| (none) | {} | |", r.no_pack);
        }
    }
    if !r.crashes.is_empty() {
        s.push_str("\n## Crashes\n\n");
        for c in &r.crashes {
            let mut line = format!("- {}", c.aircraft.as_deref().unwrap_or("Unknown aircraft"));
            if !c.broke.is_empty() {
                let _ = write!(line, ": {}", c.broke);
            }
            if !c.parts.is_empty() {
                let _ = write!(line, "; parts: {}", c.parts.join(", "));
            }
            if c.repaired {
                line.push_str(" (repaired)");
            }
            let _ = writeln!(s, "{line}");
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gear::flights::{self, synth};
    use crate::logs::Tunables;

    #[test]
    fn report_on_the_synthetic_day() {
        let d = tempfile::tempdir().unwrap();
        let p = synth::write_known(d.path()).unwrap();
        let mut f = flights::analyse_file(&p, &Tunables::default()).unwrap();
        flights::fill_resting(&mut f);
        let day = f[0].day;
        let mut sets = BTreeMap::new();
        sets.insert(
            f[0].id.clone(),
            FlightSet {
                pack: Some("A1".into()),
                place: None,
            },
        );
        let crash = Crash {
            id: "c".into(),
            aircraft: Some("Whoop".into()),
            day,
            broke: "prop".into(),
            parts: vec!["prop".into()],
            ..Default::default()
        };
        let r = build(&[day], &f, &sets, &[crash], 2);
        assert_eq!(r.flights, 3);
        assert_eq!(r.air_s, 110.0);
        assert_eq!(r.longest.as_ref().unwrap().secs, 60.0);
        let w = r.worst_link.as_ref().unwrap();
        assert_eq!(w.lq, Some(60.0));
        assert_eq!(w.rssi_db, Some(-95.0));
        // Flight 1's dropout and flight 3's failsafe.
        assert_eq!((r.dropouts, r.downlink_only), (2, 1));
        assert_eq!(r.packs.len(), 1);
        assert_eq!(r.packs[0].mah, 360.0);
        assert_eq!(r.no_pack, 2);
        assert_eq!(r.crashes.len(), 1);
        let md = &r.markdown;
        assert!(md.starts_with("# Session report: 2026-10-04\n"), "{md}");
        assert!(md.contains("- Flights: 3, air time 1:50"));
        assert!(md.contains("- Longest flight: 1:00 at 10:00 (Whoop)"));
        assert!(md.contains("- Worst link: LQ 60 %, RSSI -95 dB at 10:00"));
        assert!(md.contains("- Dropouts: 2 (1 telemetry only)"));
        assert!(md.contains("| A1 | 1 | 360 |"));
        assert!(md.contains("- Whoop: prop; parts: prop"));
        // Another day has none of it.
        let other = build(&[day.succ_opt().unwrap()], &f, &sets, &[], 0);
        assert_eq!(other.flights, 0);
        assert!(other.longest.is_none());
    }
}
