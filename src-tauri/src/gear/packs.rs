//! Packs (batteries) and pack types: the person's data in `gear.json`, each pack's history
//! from the flights assigned to it, and the charging sheet (`docs/gear-design.md` 7.6, 7.8).
//!
//! "Pack" means a physical battery only; a radio-log armed segment is a flight.
//!
//! `gear.json` keys: `packs`, `pack_types`, `pack_notes` (the charging sheet's free notes),
//! `flight_sets` (per flight id: the pack and place the person gave it) and
//! `flight_folders` (log folders the person added). Nothing in the repo names a real pack.

use super::flights::{median, Flight};
use super::store::{Store, Values};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const PACKS: &str = "packs";
pub const PACK_TYPES: &str = "pack_types";
pub const PACK_NOTES: &str = "pack_notes";
pub const FLIGHT_SETS: &str = "flight_sets";
pub const FLIGHT_FOLDERS: &str = "flight_folders";

/// A pack whose median resting voltage per cell sits this far under its type's is marked.
pub const WEAK_RESTING_PER_CELL_V: f64 = 0.05;
/// A pack whose median flight time is under this share of its type's is marked.
pub const WEAK_FLIGHT_SHARE: f64 = 0.8;
/// The resting voltage per cell the threshold suggestion aims for by default.
pub const DEFAULT_TARGET_V: f64 = 3.7;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum Chemistry {
    #[default]
    Lipo,
    Lihv,
    Liion,
}

/// A kind of pack: chemistry, cells, capacity, connector and charge settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Type)]
pub struct PackType {
    pub name: String,
    #[serde(default)]
    pub chemistry: Chemistry,
    /// Cells in series (1 for a 1S pack).
    #[serde(default = "one")]
    pub cells: u8,
    #[serde(default)]
    pub capacity_mah: Option<f64>,
    #[serde(default)]
    pub connector: Option<String>,
    /// Full and storage charge, volts per cell.
    #[serde(default)]
    pub full_v: Option<f64>,
    #[serde(default)]
    pub storage_v: Option<f64>,
    /// Charge current in amps.
    #[serde(default)]
    pub charge_a: Option<f64>,
    /// The mAh warning the radio gives for this type (the flight threshold).
    #[serde(default)]
    pub warn_mah: Option<f64>,
    #[serde(default)]
    pub note: String,
}

fn one() -> u8 {
    1
}

/// One physical battery.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Type)]
pub struct Pack {
    /// The label written on it ("A1"). Unique.
    pub label: String,
    #[serde(default)]
    pub pack_type: Option<String>,
    #[serde(default)]
    pub received: Option<NaiveDate>,
    #[serde(default)]
    pub retired: bool,
    /// When the person last marked it charged.
    #[serde(default)]
    pub charged_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub note: String,
}

/// What the person set on one flight.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Type)]
pub struct FlightSet {
    #[serde(default)]
    pub pack: Option<String>,
    #[serde(default)]
    pub place: Option<String>,
}

/// Whether a pack is ready, from its charge mark and its flights.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChargeState {
    /// Marked charged after its last flight.
    Charged,
    /// Flown since it was last marked charged (or never marked).
    Flown,
    /// No flight and no charge mark.
    Unknown,
}

/// One flight in a pack's history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct PackFlight {
    pub flight: String,
    pub start: NaiveDateTime,
    pub secs: f64,
    pub mah: Option<f64>,
    pub sag_min_v: Option<f64>,
    pub sag_p5_v: Option<f64>,
    pub resting_v: Option<f64>,
}

/// A pack with its history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct PackView {
    pub pack: Pack,
    /// Flights assigned to it (one per charge).
    pub cycles: usize,
    pub state: ChargeState,
    pub last_flown: Option<NaiveDateTime>,
    pub median_resting_v: Option<f64>,
    pub median_secs: Option<f64>,
    /// Why it stands out from its type, when it does.
    pub weak: Option<String>,
    /// Oldest first.
    pub history: Vec<PackFlight>,
}

/// A pack type with its numbers across its packs: the charging sheet's row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct PackTypeView {
    pub pack_type: PackType,
    pub packs: usize,
    pub flights: usize,
    /// Full and storage charge for the whole pack (per cell times cells).
    pub full_total_v: Option<f64>,
    pub storage_total_v: Option<f64>,
    pub median_resting_v: Option<f64>,
    pub median_secs: Option<f64>,
    /// The mAh warning that lands at `target_v` per cell resting, from its flights' mAh and
    /// resting voltage (a straight-line fit). None with too few flights.
    pub suggested_warn_mah: Option<f64>,
}

/// `gear_packs`' answer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct PacksView {
    pub packs: Vec<PackView>,
    pub types: Vec<PackTypeView>,
    /// The charging sheet's free notes.
    pub notes: String,
    /// The resting voltage per cell `suggested_warn_mah` aims for.
    pub target_v: f64,
}

// ----- gear.json -----

/// Every pack.
pub fn packs(s: &Store) -> Result<Vec<Pack>> {
    s.list(PACKS)
}

pub fn pack_types(s: &Store) -> Result<Vec<PackType>> {
    s.list(PACK_TYPES)
}

pub fn notes(s: &Store) -> Result<String> {
    Ok(s.read()?
        .get(PACK_NOTES)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

pub fn flight_sets(s: &Store) -> Result<BTreeMap<String, FlightSet>> {
    Ok(s.read()?
        .get(FLIGHT_SETS)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default())
}

pub fn flight_folders(s: &Store) -> Result<Vec<PathBuf>> {
    s.list(FLIGHT_FOLDERS)
}

/// The list under `key`, made when missing.
pub(crate) fn list_mut<'a>(v: &'a mut Values, key: &str) -> Result<&'a mut Vec<Value>> {
    match v
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()))
    {
        Value::Array(a) => Ok(a),
        _ => bail!("gear.json: {key} is not a list; fix or remove it"),
    }
}

/// Adds `entry` to the list under `key`, or replaces the entry whose `field` equals `id`
/// (keeping fields this version does not know).
pub(crate) fn upsert(s: &Store, key: &str, field: &str, id: &str, entry: Value) -> Result<()> {
    s.update(|v| {
        let list = list_mut(v, key)?;
        match list
            .iter_mut()
            .find(|e| e.get(field).and_then(Value::as_str) == Some(id))
        {
            Some(Value::Object(old)) => {
                if let Value::Object(new) = entry {
                    old.extend(new);
                }
            }
            Some(other) => *other = entry,
            None => list.push(entry),
        }
        Ok(())
    })?;
    Ok(())
}

/// Removes and returns the entry whose `field` equals `id`.
pub(crate) fn remove(s: &Store, key: &str, field: &str, id: &str) -> Result<Value> {
    let (found, _) = s.update(|v| {
        let list = list_mut(v, key)?;
        let i = list
            .iter()
            .position(|e| e.get(field).and_then(Value::as_str) == Some(id));
        Ok(i.map(|i| list.remove(i)))
    })?;
    found.with_context(|| format!("No {} {id:?}.", key.trim_end_matches('s').replace('_', " ")))
}

/// Saves a pack. Its type must exist when it names one.
pub fn save_pack(s: &Store, p: &Pack) -> Result<Pack> {
    let label = p.label.trim();
    if label.is_empty() {
        bail!("A pack needs a label.");
    }
    if let Some(t) = p.pack_type.as_deref().filter(|t| !t.is_empty()) {
        if !pack_types(s)?.iter().any(|x| x.name == t) {
            bail!("No pack type {t:?}. Save the type first.");
        }
    }
    let p = Pack {
        label: label.to_string(),
        pack_type: p.pack_type.clone().filter(|t| !t.is_empty()),
        ..p.clone()
    };
    upsert(s, PACKS, "label", &p.label, serde_json::to_value(&p)?)?;
    Ok(p)
}

pub fn delete_pack(s: &Store, label: &str) -> Result<Pack> {
    Ok(serde_json::from_value(remove(s, PACKS, "label", label)?)?)
}

pub fn save_type(s: &Store, t: &PackType) -> Result<PackType> {
    let name = t.name.trim();
    if name.is_empty() {
        bail!("A pack type needs a name.");
    }
    if t.cells == 0 {
        bail!("A pack type has at least one cell.");
    }
    let t = PackType {
        name: name.to_string(),
        ..t.clone()
    };
    upsert(s, PACK_TYPES, "name", &t.name, serde_json::to_value(&t)?)?;
    Ok(t)
}

/// Deletes a pack type no pack uses.
pub fn delete_type(s: &Store, name: &str) -> Result<PackType> {
    let users: Vec<String> = packs(s)?
        .into_iter()
        .filter(|p| p.pack_type.as_deref() == Some(name))
        .map(|p| p.label)
        .collect();
    if !users.is_empty() {
        bail!("Packs {} use type {name:?}.", users.join(", "));
    }
    Ok(serde_json::from_value(remove(
        s, PACK_TYPES, "name", name,
    )?)?)
}

pub fn save_notes(s: &Store, text: &str) -> Result<String> {
    s.update(|v| {
        v.insert(PACK_NOTES.into(), Value::String(text.to_string()));
        Ok(())
    })?;
    Ok(text.to_string())
}

/// Sets a flight's pack and place. `Some("")` clears one; None leaves it.
pub fn set_flight(
    s: &Store,
    flight: &str,
    pack: Option<&str>,
    place: Option<&str>,
) -> Result<FlightSet> {
    if let Some(p) = pack.filter(|p| !p.is_empty()) {
        if !packs(s)?.iter().any(|x| x.label == p) {
            bail!("No pack {p:?}. Save it first.");
        }
    }
    let (set, _) = s.update(|v| {
        let map = match v
            .entry(FLIGHT_SETS.to_string())
            .or_insert_with(|| Value::Object(Default::default()))
        {
            Value::Object(m) => m,
            _ => bail!("gear.json: {FLIGHT_SETS} is not a map; fix or remove it"),
        };
        let mut set: FlightSet = map
            .get(flight)
            .and_then(|e| serde_json::from_value(e.clone()).ok())
            .unwrap_or_default();
        let opt = |x: &str| (!x.is_empty()).then(|| x.to_string());
        if let Some(p) = pack {
            set.pack = opt(p);
        }
        if let Some(p) = place {
            set.place = opt(p);
        }
        if set == FlightSet::default() {
            map.remove(flight);
        } else {
            map.insert(flight.to_string(), serde_json::to_value(&set)?);
        }
        Ok(set)
    })?;
    Ok(set)
}

/// Adds or removes a log folder. Returns the list.
pub fn edit_folders(
    s: &Store,
    add: Option<PathBuf>,
    remove: Option<PathBuf>,
) -> Result<Vec<PathBuf>> {
    let (list, _) = s.update(|v| {
        let list = list_mut(v, FLIGHT_FOLDERS)?;
        if let Some(r) = &remove {
            list.retain(|e| e.as_str().map(PathBuf::from).as_ref() != Some(r));
        }
        if let Some(a) = add {
            if !a.is_absolute() {
                bail!("A log folder must be an absolute path.");
            }
            if !list
                .iter()
                .any(|e| e.as_str().map(PathBuf::from).as_ref() == Some(&a))
            {
                list.push(Value::String(a.to_string_lossy().into()));
            }
        }
        Ok(list
            .iter()
            .filter_map(|e| e.as_str().map(PathBuf::from))
            .collect())
    })?;
    Ok(list)
}

// ----- history and suggestions -----

/// Labels in natural order: "A2" before "A10".
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn key(s: &str) -> Vec<(String, u64)> {
        let mut out = Vec::new();
        let mut text = String::new();
        let mut num = String::new();
        for c in s.chars() {
            if c.is_ascii_digit() {
                num.push(c);
            } else {
                if !num.is_empty() {
                    out.push((std::mem::take(&mut text), num.parse().unwrap_or(0)));
                    num.clear();
                }
                text.push(c.to_ascii_lowercase());
            }
        }
        out.push((text, num.parse().unwrap_or(0)));
        out
    }
    key(a).cmp(&key(b))
}

/// The pack QuadCam suggests for a flight: the next label in order after the pack of the
/// previous flight that day of the same model, among that pack's type, skipping retired
/// packs; else the first pack of `default_type` (the aircraft's), else None.
pub fn suggest_pack(
    flight: &Flight,
    flights: &[Flight],
    sets: &BTreeMap<String, FlightSet>,
    packs: &[Pack],
    default_type: Option<&str>,
) -> Option<String> {
    let in_order = |ty: Option<&str>| {
        let mut v: Vec<&Pack> = packs
            .iter()
            .filter(|p| !p.retired && (ty.is_none() || p.pack_type.as_deref() == ty))
            .collect();
        v.sort_by(|a, b| natural_cmp(&a.label, &b.label));
        v
    };
    let prev = flights
        .iter()
        .filter(|f| f.start < flight.start && f.day == flight.day && f.model == flight.model)
        .rev()
        .find_map(|f| sets.get(&f.id).and_then(|s| s.pack.clone()));
    if let Some(prev) = prev {
        let ty = packs
            .iter()
            .find(|p| p.label == prev)
            .and_then(|p| p.pack_type.clone());
        let order = in_order(ty.as_deref());
        if let Some(i) = order.iter().position(|p| p.label == prev) {
            return order.get(i + 1).or(order.first()).map(|p| p.label.clone());
        }
        return order.first().map(|p| p.label.clone());
    }
    default_type.and_then(|t| in_order(Some(t)).first().map(|p| p.label.clone()))
}

/// The mAh at which the straight line through `(mah, resting volts per cell)` reaches
/// `target_v`. None with fewer than three points, or when resting voltage does not fall as
/// mAh rises.
pub fn suggest_threshold(points: &[(f64, f64)], target_v: f64) -> Option<f64> {
    if points.len() < 3 {
        return None;
    }
    let n = points.len() as f64;
    let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let my = points.iter().map(|p| p.1).sum::<f64>() / n;
    let sxx: f64 = points.iter().map(|p| (p.0 - mx).powi(2)).sum();
    let sxy: f64 = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    if sxx <= 0.0 {
        return None;
    }
    let slope = sxy / sxx;
    if slope >= 0.0 {
        return None;
    }
    let x = mx + (target_v - my) / slope;
    (x.is_finite() && x > 0.0).then(|| (x / 10.0).round() * 10.0)
}

/// Every pack with its history, every type with its numbers, and the notes.
pub fn view(
    packs: &[Pack],
    types: &[PackType],
    notes: &str,
    flights: &[Flight],
    sets: &BTreeMap<String, FlightSet>,
    target_v: f64,
) -> PacksView {
    let history = |label: &str| -> Vec<&Flight> {
        flights
            .iter()
            .filter(|f| sets.get(&f.id).and_then(|s| s.pack.as_deref()) == Some(label))
            .collect()
    };
    let cells = |p: &Pack| {
        p.pack_type
            .as_deref()
            .and_then(|t| types.iter().find(|x| x.name == t))
            .map_or(1, |t| t.cells.max(1)) as f64
    };
    let mut views: Vec<PackView> = packs
        .iter()
        .map(|p| {
            let fl = history(&p.label);
            let rest: Vec<f64> = fl.iter().filter_map(|f| f.resting_v).collect();
            let secs: Vec<f64> = fl.iter().map(|f| f.secs).collect();
            let last = fl.iter().map(|f| f.end).max();
            let state = match (p.charged_at, last) {
                (Some(c), Some(l)) if c.with_timezone(&chrono::Local).naive_local() > l => {
                    ChargeState::Charged
                }
                (Some(_), None) => ChargeState::Charged,
                (_, Some(_)) => ChargeState::Flown,
                (None, None) => ChargeState::Unknown,
            };
            PackView {
                pack: p.clone(),
                cycles: fl.len(),
                state,
                last_flown: last,
                median_resting_v: median(&rest),
                median_secs: median(&secs),
                weak: None,
                history: fl
                    .iter()
                    .map(|f| PackFlight {
                        flight: f.id.clone(),
                        start: f.start,
                        secs: f.secs,
                        mah: f.mah,
                        sag_min_v: f.sag_min_v,
                        sag_p5_v: f.sag_p5_v,
                        resting_v: f.resting_v,
                    })
                    .collect(),
            }
        })
        .collect();
    views.sort_by(|a, b| natural_cmp(&a.pack.label, &b.pack.label));

    let type_views: Vec<PackTypeView> = types
        .iter()
        .map(|t| {
            let mine: Vec<&PackView> = views
                .iter()
                .filter(|v| v.pack.pack_type.as_deref() == Some(t.name.as_str()))
                .collect();
            let fl: Vec<&Flight> = mine.iter().flat_map(|v| history(&v.pack.label)).collect();
            let c = t.cells.max(1) as f64;
            let points: Vec<(f64, f64)> = fl
                .iter()
                .filter_map(|f| Some((f.mah?, f.resting_v? / c)))
                .collect();
            let pack_rest: Vec<f64> = mine.iter().filter_map(|v| v.median_resting_v).collect();
            let pack_secs: Vec<f64> = mine.iter().filter_map(|v| v.median_secs).collect();
            PackTypeView {
                pack_type: t.clone(),
                packs: mine.len(),
                flights: fl.len(),
                full_total_v: t.full_v.map(|v| v * c),
                storage_total_v: t.storage_v.map(|v| v * c),
                median_resting_v: median(&pack_rest),
                median_secs: median(&pack_secs),
                suggested_warn_mah: suggest_threshold(&points, target_v),
            }
        })
        .collect();

    // A pack well below its type's median, among types with two packs or more flown.
    for v in &mut views {
        let Some(t) = type_views
            .iter()
            .find(|t| Some(t.pack_type.name.as_str()) == v.pack.pack_type.as_deref())
        else {
            continue;
        };
        let flown = views_flown(packs, flights, sets, &t.pack_type.name);
        if flown < 2 {
            continue;
        }
        let c = cells(&v.pack);
        let mut why = Vec::new();
        if let (Some(m), Some(tm)) = (v.median_resting_v, t.median_resting_v) {
            if (tm - m) / c >= WEAK_RESTING_PER_CELL_V - 1e-9 {
                why.push(format!("rests at {m:.2} V, its type at {tm:.2} V"));
            }
        }
        if let (Some(m), Some(tm)) = (v.median_secs, t.median_secs) {
            if m < tm * WEAK_FLIGHT_SHARE {
                why.push(format!("flies {m:.0} s, its type {tm:.0} s"));
            }
        }
        if !why.is_empty() {
            v.weak = Some(why.join("; "));
        }
    }

    PacksView {
        packs: views,
        types: type_views,
        notes: notes.to_string(),
        target_v,
    }
}

fn views_flown(
    packs: &[Pack],
    flights: &[Flight],
    sets: &BTreeMap<String, FlightSet>,
    ty: &str,
) -> usize {
    packs
        .iter()
        .filter(|p| p.pack_type.as_deref() == Some(ty))
        .filter(|p| {
            flights
                .iter()
                .any(|f| sets.get(&f.id).and_then(|s| s.pack.as_deref()) == Some(p.label.as_str()))
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gear::flights::Hover;

    fn flight(id: &str, hm: &str, secs: f64, mah: f64, resting: f64) -> Flight {
        let start =
            NaiveDateTime::parse_from_str(&format!("2026-10-04 {hm}:00"), "%Y-%m-%d %H:%M:%S")
                .unwrap();
        Flight {
            id: id.into(),
            model: Some("Whoop".into()),
            file: "x.csv".into(),
            day: start.date(),
            start,
            end: start + chrono::Duration::milliseconds((secs * 1000.0) as i64),
            secs,
            hover: Hover::default(),
            sag_p5_v: Some(3.5),
            sag_min_v: Some(3.3),
            arm_v: Some(4.2),
            resting_v: Some(resting),
            resting_from: None,
            mah: Some(mah),
            capa: vec![],
            max_current_a: None,
            worst_lq: None,
            worst_rssi_db: None,
            max_tx_power_mw: None,
            dropouts: vec![],
        }
    }

    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::new(d.path());
        (d, s)
    }

    fn one_s() -> PackType {
        PackType {
            name: "1S 300".into(),
            chemistry: Chemistry::Lihv,
            cells: 1,
            capacity_mah: Some(300.0),
            full_v: Some(4.35),
            storage_v: Some(3.85),
            warn_mah: Some(250.0),
            ..Default::default()
        }
    }

    #[test]
    fn packs_and_types_round_trip_and_keep_unknown_keys() {
        let (_d, s) = store();
        s.update(|v| {
            v.insert("future".into(), Value::Bool(true));
            Ok(())
        })
        .unwrap();
        assert!(save_pack(
            &s,
            &Pack {
                label: "A1".into(),
                pack_type: Some("1S 300".into()),
                ..Default::default()
            }
        )
        .is_err());
        save_type(&s, &one_s()).unwrap();
        let p = save_pack(
            &s,
            &Pack {
                label: " A1 ".into(),
                pack_type: Some("1S 300".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(p.label, "A1");
        assert_eq!(packs(&s).unwrap(), vec![p]);
        assert!(delete_type(&s, "1S 300").is_err());
        delete_pack(&s, "A1").unwrap();
        delete_type(&s, "1S 300").unwrap();
        assert!(delete_pack(&s, "A1").is_err());
        assert_eq!(s.read().unwrap()["future"], Value::Bool(true));
    }

    #[test]
    fn flight_sets_and_folders() {
        let (_d, s) = store();
        assert!(set_flight(&s, "f1", Some("A1"), None).is_err());
        save_pack(
            &s,
            &Pack {
                label: "A1".into(),
                ..Default::default()
            },
        )
        .unwrap();
        set_flight(&s, "f1", Some("A1"), Some("Field")).unwrap();
        let set = set_flight(&s, "f1", None, Some("")).unwrap();
        assert_eq!(
            set,
            FlightSet {
                pack: Some("A1".into()),
                place: None
            }
        );
        assert_eq!(flight_sets(&s).unwrap()["f1"].pack.as_deref(), Some("A1"));
        set_flight(&s, "f1", Some(""), None).unwrap();
        assert!(flight_sets(&s).unwrap().is_empty());
        assert!(edit_folders(&s, Some("rel".into()), None).is_err());
        let l = edit_folders(&s, Some("/logs/a".into()), None).unwrap();
        assert_eq!(edit_folders(&s, Some("/logs/a".into()), None).unwrap(), l);
        assert!(edit_folders(&s, None, Some("/logs/a".into()))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn pack_history_cycles_state_and_weak_mark() {
        let types = vec![one_s()];
        let mk = |l: &str| Pack {
            label: l.into(),
            pack_type: Some("1S 300".into()),
            ..Default::default()
        };
        let mut packs = vec![mk("A1"), mk("A2"), mk("A3"), mk("A4")];
        let flights = vec![
            flight("f1", "10:00", 120.0, 200.0, 3.80),
            flight("f2", "10:05", 120.0, 220.0, 3.78),
            flight("f3", "10:10", 125.0, 240.0, 3.76),
            flight("f4", "10:15", 70.0, 250.0, 3.60),
            flight("f5", "10:20", 120.0, 230.0, 3.79),
        ];
        let mut sets = BTreeMap::new();
        for (f, p) in [
            ("f1", "A1"),
            ("f2", "A2"),
            ("f3", "A3"),
            ("f4", "A4"),
            ("f5", "A1"),
        ] {
            sets.insert(
                f.to_string(),
                FlightSet {
                    pack: Some(p.into()),
                    place: None,
                },
            );
        }
        // A2 charged after its flight.
        packs[1].charged_at = Some(
            chrono::Local
                .from_local_datetime(&flights[1].end)
                .unwrap()
                .with_timezone(&Utc)
                + chrono::Duration::hours(1),
        );
        let v = view(&packs, &types, "notes", &flights, &sets, DEFAULT_TARGET_V);
        let a1 = &v.packs[0];
        assert_eq!(a1.pack.label, "A1");
        assert_eq!(a1.cycles, 2);
        assert_eq!(a1.history[1].flight, "f5");
        assert_eq!(a1.state, ChargeState::Flown);
        assert_eq!(v.packs[1].state, ChargeState::Charged);
        let a4 = &v.packs[3];
        assert!(
            a4.weak.as_deref().unwrap().contains("rests at 3.60"),
            "{:?}",
            a4.weak
        );
        assert!(a4.weak.as_deref().unwrap().contains("flies 70 s"));
        assert_eq!(a1.weak, None);
        let t = &v.types[0];
        assert_eq!((t.packs, t.flights), (4, 5));
        assert_eq!(t.full_total_v, Some(4.35));
        assert!(t.suggested_warn_mah.is_some());
        assert_eq!(v.notes, "notes");
    }

    use chrono::TimeZone;

    #[test]
    fn next_label_in_order() {
        let mk = |l: &str, ty: &str| Pack {
            label: l.into(),
            pack_type: Some(ty.into()),
            ..Default::default()
        };
        let mut packs = vec![mk("A10", "t"), mk("A2", "t"), mk("A1", "t"), mk("B1", "u")];
        let flights = vec![
            flight("f1", "10:00", 60.0, 200.0, 3.8),
            flight("f2", "10:05", 60.0, 200.0, 3.8),
        ];
        let mut sets = BTreeMap::new();
        assert_eq!(
            suggest_pack(&flights[1], &flights, &sets, &packs, Some("u")).as_deref(),
            Some("B1")
        );
        assert_eq!(
            suggest_pack(&flights[1], &flights, &sets, &packs, None),
            None
        );
        sets.insert(
            "f1".into(),
            FlightSet {
                pack: Some("A2".into()),
                place: None,
            },
        );
        assert_eq!(
            suggest_pack(&flights[1], &flights, &sets, &packs, None).as_deref(),
            Some("A10")
        );
        sets.insert(
            "f1".into(),
            FlightSet {
                pack: Some("A10".into()),
                place: None,
            },
        );
        assert_eq!(
            suggest_pack(&flights[1], &flights, &sets, &packs, None).as_deref(),
            Some("A1")
        );
        packs[2].retired = true;
        assert_eq!(
            suggest_pack(&flights[1], &flights, &sets, &packs, None).as_deref(),
            Some("A2")
        );
    }

    #[test]
    fn threshold_from_a_line() {
        // Resting falls 0.1 V per 100 mAh from 4.0 V at 0 mAh: 3.7 V at 300 mAh.
        let pts: Vec<(f64, f64)> = [100.0, 200.0, 250.0, 350.0]
            .iter()
            .map(|m| (*m, 4.0 - m / 1000.0))
            .collect();
        assert_eq!(suggest_threshold(&pts, 3.7), Some(300.0));
        assert_eq!(suggest_threshold(&pts[..2], 3.7), None);
        assert_eq!(
            suggest_threshold(&[(1.0, 3.0), (2.0, 3.1), (3.0, 3.2)], 3.7),
            None
        );
    }
}
