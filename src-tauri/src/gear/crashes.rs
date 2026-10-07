//! The crash and repair log: a crash marked on a library clip (a time in the clip), what
//! broke and the parts used, tied to an aircraft profile. The person's data, in `gear.json`
//! under `crashes`.

use super::packs::{remove, upsert};
use super::store::Store;
use anyhow::{bail, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use specta::Type;

pub const CRASHES: &str = "crashes";

/// One crash and its repair.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Type)]
pub struct Crash {
    pub id: String,
    /// The library clip it happened in.
    #[serde(default)]
    pub clip: Option<String>,
    /// Seconds into the clip.
    #[serde(default)]
    pub time_s: Option<f64>,
    /// The aircraft profile.
    #[serde(default)]
    pub aircraft: Option<String>,
    pub day: NaiveDate,
    /// What broke.
    #[serde(default)]
    pub broke: String,
    /// Parts used for the repair.
    #[serde(default)]
    pub parts: Vec<String>,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub repaired: bool,
}

/// `gear_crashes`: narrow by aircraft or clip; both empty for every crash.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct CrashFilter {
    #[serde(default)]
    pub aircraft: Option<String>,
    #[serde(default)]
    pub clip: Option<String>,
}

/// Every crash, newest day first, then by clip time.
pub fn list(s: &Store, f: &CrashFilter) -> Result<Vec<Crash>> {
    let mut v: Vec<Crash> = s.list(CRASHES)?;
    v.retain(|c| {
        f.aircraft
            .as_ref()
            .is_none_or(|a| c.aircraft.as_ref() == Some(a))
            && f.clip.as_ref().is_none_or(|x| c.clip.as_ref() == Some(x))
    });
    v.sort_by(|a, b| {
        b.day
            .cmp(&a.day)
            .then(a.time_s.unwrap_or(0.0).total_cmp(&b.time_s.unwrap_or(0.0)))
    });
    Ok(v)
}

/// A new id: the time now, to the millisecond.
pub fn new_id() -> String {
    format!("crash-{}", chrono::Utc::now().format("%Y%m%dT%H%M%S%3f"))
}

/// Saves a crash; an empty id makes a new one.
pub fn save(s: &Store, c: &Crash) -> Result<Crash> {
    if let Some(t) = c.time_s {
        if !t.is_finite() || t < 0.0 {
            bail!("time_s is seconds into the clip, 0 or more.");
        }
    }
    let mut c = c.clone();
    if c.id.trim().is_empty() {
        c.id = new_id();
    }
    c.parts = c
        .parts
        .iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    upsert(s, CRASHES, "id", &c.id, serde_json::to_value(&c)?)?;
    Ok(c)
}

pub fn delete(s: &Store, id: &str) -> Result<Crash> {
    Ok(serde_json::from_value(remove(s, CRASHES, "id", id)?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_list_filter_delete() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::new(d.path());
        let day = NaiveDate::from_ymd_opt(2026, 10, 4).unwrap();
        let a = save(
            &s,
            &Crash {
                clip: Some("clip1".into()),
                time_s: Some(42.5),
                aircraft: Some("Whoop".into()),
                day,
                broke: "prop".into(),
                parts: vec![" prop ".into(), "".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(a.id.starts_with("crash-"));
        assert_eq!(a.parts, vec!["prop"]);
        let b = save(
            &s,
            &Crash {
                id: "x".into(),
                aircraft: Some("Five".into()),
                day: day.succ_opt().unwrap(),
                ..Default::default()
            },
        )
        .unwrap();
        let all = list(&s, &CrashFilter::default()).unwrap();
        assert_eq!(all, vec![b.clone(), a.clone()]);
        let mine = list(
            &s,
            &CrashFilter {
                aircraft: Some("Whoop".into()),
                clip: None,
            },
        )
        .unwrap();
        assert_eq!(mine, vec![a.clone()]);
        // Saving again with the id edits in place.
        let a2 = save(
            &s,
            &Crash {
                repaired: true,
                ..a.clone()
            },
        )
        .unwrap();
        assert_eq!(list(&s, &CrashFilter::default()).unwrap().len(), 2);
        assert!(a2.repaired);
        delete(&s, "x").unwrap();
        assert!(delete(&s, "x").is_err());
        assert!(save(
            &s,
            &Crash {
                time_s: Some(-1.0),
                day,
                ..Default::default()
            }
        )
        .is_err());
    }
}
