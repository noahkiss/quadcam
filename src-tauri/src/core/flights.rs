//! `Core`'s flights, packs, crashes, session report and "Pack up" check (WP12). Each is a
//! thin join of the `gear::flights`, `gear::packs`, `gear::crashes`, `gear::report` and
//! `gear::preflight` logic with the profiles, the library and what is plugged in.

use super::Core;
use crate::gear::crashes::{self, Crash, CrashFilter};
use crate::gear::flights::{self, Flight};
use crate::gear::model::{DeviceKind, Link};
use crate::gear::packs::{self, FlightSet, Pack, PackType, PacksView};
use crate::gear::preflight::{self, Preflight};
use crate::gear::report::{self, SessionReport};
use crate::library::LibClip;
use crate::metadata::Profile;
use anyhow::{bail, Result};
use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A clip claims a flight that starts within this many seconds of the clip's span.
const CLIP_SLACK_S: f64 = 90.0;

/// `gear_flights`: narrow the flights. Every field is optional.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct FlightFilter {
    #[serde(default)]
    pub day: Option<NaiveDate>,
    /// An aircraft profile name.
    #[serde(default)]
    pub aircraft: Option<String>,
    /// A pack label.
    #[serde(default)]
    pub pack: Option<String>,
    #[serde(default)]
    pub place: Option<String>,
    /// One more log folder to read, for this call only.
    #[serde(default)]
    pub logs: Option<PathBuf>,
}

/// A flight with what QuadCam joins to it.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct FlightReport {
    pub flight: Flight,
    /// The aircraft profile whose EdgeTX models name the flight's model.
    pub aircraft: Option<String>,
    pub pack: Option<String>,
    /// The place: set by hand, else the matched clip's, else the profile's.
    pub place: Option<String>,
    /// The library clip that holds the flight, by time.
    pub clip: Option<String>,
    pub clip_name: Option<String>,
    /// The pack QuadCam suggests when none is set.
    pub suggested_pack: Option<String>,
    /// The mAh warning of the pack's type (else the aircraft's pack type), and the second
    /// the flight crossed it.
    pub threshold_mah: Option<f64>,
    pub crossed_at_s: Option<f64>,
}

/// One flight's worst link, for the range trend.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct TrendPoint {
    pub flight: String,
    pub start: NaiveDateTime,
    pub worst_lq: Option<f64>,
    pub worst_rssi_db: Option<f64>,
}

/// The worst link per flight at one place, oldest first.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct PlaceTrend {
    pub place: String,
    pub points: Vec<TrendPoint>,
}

/// `gear_flights`' answer.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct FlightsView {
    /// Newest first.
    pub flights: Vec<FlightReport>,
    /// Every day with flights (before the filter), newest first.
    pub days: Vec<NaiveDate>,
    /// The log folders read.
    pub sources: Vec<PathBuf>,
    pub places: Vec<PlaceTrend>,
}

/// `gear_flight_set`: `Some("")` clears a value; leaving it out keeps it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct FlightSetParams {
    pub flight: String,
    #[serde(default)]
    pub pack: Option<String>,
    #[serde(default)]
    pub place: Option<String>,
}

/// `gear_flight_folders`: add or remove a log folder; neither lists them.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct FlightFoldersParams {
    #[serde(default)]
    pub add: Option<PathBuf>,
    #[serde(default)]
    pub remove: Option<PathBuf>,
}

/// `gear_packs`: the resting voltage per cell the threshold suggestion aims for.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct PacksParams {
    #[serde(default)]
    pub target_v: Option<f64>,
}

/// `gear_pack_save`: a pack, and `charged` to mark it charged now (true) or clear the mark
/// (false).
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct PackSaveParams {
    pub pack: Pack,
    #[serde(default)]
    pub charged: Option<bool>,
}

/// `gear_pack_notes`: the charging sheet's notes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct NotesParams {
    pub text: String,
}

/// `gear_session_report`: a day, else the days of the last import, else the newest day
/// with flights.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct ReportParams {
    #[serde(default)]
    pub day: Option<NaiveDate>,
}

/// `gear_crash_save`: a crash; an empty id makes a new one. With a clip and no aircraft or
/// day, they come from the clip.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct CrashSaveParams {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub clip: Option<String>,
    #[serde(default)]
    pub time_s: Option<f64>,
    #[serde(default)]
    pub aircraft: Option<String>,
    #[serde(default)]
    pub day: Option<NaiveDate>,
    #[serde(default)]
    pub broke: Option<String>,
    #[serde(default)]
    pub parts: Option<Vec<String>>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub repaired: Option<bool>,
}

/// The profile whose EdgeTX model names include `model`.
fn aircraft_of<'a>(profiles: &'a [Profile], model: Option<&str>) -> Option<&'a Profile> {
    let m = model?;
    profiles
        .iter()
        .find(|p| p.edgetx_models.iter().any(|x| x.eq_ignore_ascii_case(m)))
}

/// A library clip's start in local time, when it has a time of day.
fn clip_start(c: &LibClip) -> Option<NaiveDateTime> {
    let t = chrono::NaiveTime::parse_from_str(c.time.as_deref()?, "%H:%M").ok()?;
    Some(c.date.and_time(t))
}

/// The clip whose span (start to start plus length, with slack) holds the flight's start.
fn clip_of<'a>(clips: &'a [LibClip], f: &Flight) -> Option<&'a LibClip> {
    clips
        .iter()
        .filter(|c| c.date == f.day && c.cut_of.is_none())
        .filter_map(|c| {
            let s = clip_start(c)?;
            let off = (f.start - s).num_milliseconds() as f64 / 1000.0;
            (off >= -CLIP_SLACK_S && off <= c.duration + CLIP_SLACK_S).then_some((c, off.abs()))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|x| x.0)
}

/// Bytes free and in all on a mounted volume (`df -k`).
fn space(mount: &Path) -> (Option<u64>, Option<u64>) {
    let Ok(out) = std::process::Command::new("/bin/df")
        .arg("-k")
        .arg(mount)
        .output()
    else {
        return (None, None);
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let Some(line) = text.lines().nth(1) else {
        return (None, None);
    };
    let cols: Vec<&str> = line.split_whitespace().collect();
    let kb = |i: usize| {
        cols.get(i)
            .and_then(|v| v.parse::<u64>().ok())
            .map(|k| k * 1024)
    };
    (kb(3), kb(1))
}

/// The date at the start of a backup id or time (`2026-10-04T101500-connect`, an RFC 3339
/// time). TODO(WP4): read the backup's `taken_at` instead.
fn backup_time(s: &str) -> Option<NaiveDateTime> {
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&chrono::Local).naive_local());
    }
    let d = NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok()?;
    d.and_hms_opt(0, 0, 0)
}

impl Core {
    /// The log folders to read: the log store (`<gear>/logs/`), the folders the person
    /// added, and the `LOGS/` of each radio plugged in.
    pub fn gear_flight_sources(&self) -> Result<Vec<PathBuf>> {
        let store = self.gear_store();
        let mut v = vec![store.root().join("logs")];
        v.extend(packs::flight_folders(&store)?);
        for c in self.gear_connected().unwrap_or_default() {
            if let (DeviceKind::Radio, Link::Volume { mount, .. }) = (c.kind, &c.link) {
                v.push(mount.clone());
            }
        }
        v.dedup();
        Ok(v)
    }

    /// Every flight in the sources (and `extra`), from the cache where it is current.
    fn gear_all_flights(&self, extra: Option<&Path>) -> Result<(Vec<Flight>, Vec<PathBuf>)> {
        let mut sources = self.gear_flight_sources()?;
        if let Some(e) = extra {
            sources.push(e.to_path_buf());
        }
        let cache = self.gear_store().flights_file();
        let tun = self.defaults().tunables;
        Ok((flights::index(&sources, &cache, &tun)?, sources))
    }

    fn library_clips(&self) -> Vec<LibClip> {
        self.library(&Default::default())
            .map(|v| v.clips.into_iter().map(|i| i.clip).collect())
            .unwrap_or_default()
    }

    fn reports(
        &self,
        all: &[Flight],
        sets: &BTreeMap<String, FlightSet>,
        packs: &[Pack],
        types: &[PackType],
    ) -> Result<Vec<FlightReport>> {
        let (profiles, _) = self.profiles()?;
        let clips = self.library_clips();
        Ok(all
            .iter()
            .map(|f| {
                let prof = aircraft_of(&profiles, f.model.as_deref());
                let set = sets.get(&f.id).cloned().unwrap_or_default();
                let clip = clip_of(&clips, f);
                let place = set
                    .place
                    .clone()
                    .or_else(|| clip.and_then(|c| c.place.clone()))
                    .or_else(|| prof.and_then(|p| p.place.clone()));
                let pack_type = set
                    .pack
                    .as_deref()
                    .and_then(|l| packs.iter().find(|p| p.label == l))
                    .and_then(|p| p.pack_type.clone())
                    .or_else(|| prof.and_then(|p| p.gear.pack_type.clone()));
                let threshold = pack_type
                    .as_deref()
                    .and_then(|t| types.iter().find(|x| x.name == t))
                    .and_then(|t| t.warn_mah);
                let suggested = if set.pack.is_none() {
                    packs::suggest_pack(
                        f,
                        all,
                        sets,
                        packs,
                        prof.and_then(|p| p.gear.pack_type.as_deref()),
                    )
                } else {
                    None
                };
                FlightReport {
                    aircraft: prof.map(|p| p.name.clone()),
                    pack: set.pack.clone(),
                    place,
                    clip: clip.map(|c| c.id.clone()),
                    clip_name: clip.map(|c| c.display_name()),
                    suggested_pack: suggested,
                    crossed_at_s: threshold.and_then(|t| f.crossed(t)),
                    threshold_mah: threshold,
                    flight: f.clone(),
                }
            })
            .collect())
    }

    /// Flights from the radio logs, with their measures, pack, place and clip.
    pub fn gear_flights(&self, filter: &FlightFilter) -> Result<FlightsView> {
        let store = self.gear_store();
        let (all, sources) = self.gear_all_flights(filter.logs.as_deref())?;
        let sets = packs::flight_sets(&store)?;
        let (pk, types) = (packs::packs(&store)?, packs::pack_types(&store)?);
        let mut days: Vec<NaiveDate> = all.iter().map(|f| f.day).collect();
        days.sort();
        days.dedup();
        days.reverse();
        let reports = self.reports(&all, &sets, &pk, &types)?;

        let mut places: Vec<PlaceTrend> = Vec::new();
        for r in &reports {
            let Some(p) = &r.place else { continue };
            let point = TrendPoint {
                flight: r.flight.id.clone(),
                start: r.flight.start,
                worst_lq: r.flight.worst_lq,
                worst_rssi_db: r.flight.worst_rssi_db,
            };
            match places.iter_mut().find(|t| &t.place == p) {
                Some(t) => t.points.push(point),
                None => places.push(PlaceTrend {
                    place: p.clone(),
                    points: vec![point],
                }),
            }
        }
        places.sort_by(|a, b| a.place.cmp(&b.place));

        let mut flights: Vec<FlightReport> = reports
            .into_iter()
            .filter(|r| {
                filter.day.is_none_or(|d| r.flight.day == d)
                    && filter
                        .aircraft
                        .as_ref()
                        .is_none_or(|a| r.aircraft.as_ref() == Some(a))
                    && filter
                        .pack
                        .as_ref()
                        .is_none_or(|p| r.pack.as_ref() == Some(p))
                    && filter
                        .place
                        .as_ref()
                        .is_none_or(|p| r.place.as_ref() == Some(p))
            })
            .collect();
        flights.reverse();
        Ok(FlightsView {
            flights,
            days,
            sources,
            places,
        })
    }

    /// Sets a flight's pack or place. The flight must be one the logs hold.
    pub fn gear_flight_set(&self, p: &FlightSetParams) -> Result<FlightReport> {
        let store = self.gear_store();
        let (all, _) = self.gear_all_flights(None)?;
        if !all.iter().any(|f| f.id == p.flight) {
            bail!("No flight {:?} in the radio logs.", p.flight);
        }
        packs::set_flight(&store, &p.flight, p.pack.as_deref(), p.place.as_deref())?;
        self.hooks.gear_changed();
        let sets = packs::flight_sets(&store)?;
        let (pk, types) = (packs::packs(&store)?, packs::pack_types(&store)?);
        let r = self.reports(&all, &sets, &pk, &types)?;
        Ok(r.into_iter().find(|r| r.flight.id == p.flight).unwrap())
    }

    pub fn gear_flight_folders(&self, p: &FlightFoldersParams) -> Result<Vec<PathBuf>> {
        let changes = p.add.is_some() || p.remove.is_some();
        let v = packs::edit_folders(&self.gear_store(), p.add.clone(), p.remove.clone())?;
        if changes {
            self.hooks.gear_changed();
        }
        Ok(v)
    }

    /// Packs with their history, pack types with their numbers, and the charging notes.
    pub fn gear_packs(&self, p: &PacksParams) -> Result<PacksView> {
        let store = self.gear_store();
        let (all, _) = self.gear_all_flights(None)?;
        Ok(packs::view(
            &packs::packs(&store)?,
            &packs::pack_types(&store)?,
            &packs::notes(&store)?,
            &all,
            &packs::flight_sets(&store)?,
            p.target_v.unwrap_or(packs::DEFAULT_TARGET_V),
        ))
    }

    pub fn gear_pack_save(&self, p: &PackSaveParams) -> Result<Pack> {
        let mut pack = p.pack.clone();
        match p.charged {
            Some(true) => pack.charged_at = Some(chrono::Utc::now()),
            Some(false) => pack.charged_at = None,
            None => {}
        }
        let out = packs::save_pack(&self.gear_store(), &pack)?;
        self.hooks.gear_changed();
        Ok(out)
    }

    pub fn gear_pack_delete(&self, label: &str) -> Result<Pack> {
        let out = packs::delete_pack(&self.gear_store(), label)?;
        self.hooks.gear_changed();
        Ok(out)
    }

    pub fn gear_pack_type_save(&self, t: &PackType) -> Result<PackType> {
        let out = packs::save_type(&self.gear_store(), t)?;
        self.hooks.gear_changed();
        Ok(out)
    }

    pub fn gear_pack_type_delete(&self, name: &str) -> Result<PackType> {
        let out = packs::delete_type(&self.gear_store(), name)?;
        self.hooks.gear_changed();
        Ok(out)
    }

    pub fn gear_pack_notes(&self, p: &NotesParams) -> Result<String> {
        let out = packs::save_notes(&self.gear_store(), &p.text)?;
        self.hooks.gear_changed();
        Ok(out)
    }

    /// The session report: a day, else the days of the last import, else the newest day.
    pub fn gear_session_report(&self, p: &ReportParams) -> Result<SessionReport> {
        let store = self.gear_store();
        let (all, _) = self.gear_all_flights(None)?;
        let (days, clips) = match p.day {
            Some(d) => (vec![d], 0),
            None => {
                let last: Vec<LibClip> = self
                    .library(&crate::library::Filter {
                        group: Some("last_import".into()),
                        ..Default::default()
                    })
                    .map(|v| v.clips.into_iter().map(|i| i.clip).collect())
                    .unwrap_or_default();
                let mut d: Vec<NaiveDate> = last.iter().map(|c| c.date).collect();
                d.sort();
                d.dedup();
                if d.is_empty() {
                    (all.iter().map(|f| f.day).max().into_iter().collect(), 0)
                } else {
                    (d, last.len())
                }
            }
        };
        Ok(report::build(
            &days,
            &all,
            &packs::flight_sets(&store)?,
            &crashes::list(&store, &CrashFilter::default())?,
            clips,
        ))
    }

    /// The "Pack up" check: packs charged, the radio's model, card space, backups, and
    /// cards still in the Mac. Reads only.
    pub fn gear_preflight(&self) -> Result<Preflight> {
        let store = self.gear_store();
        let packs_view = self.gear_packs(&PacksParams::default())?;
        let devices = store.devices()?;
        let connected = self.gear_connected().unwrap_or_default();
        let reminders = self.gear.cues.reminders.lock().unwrap().armed();
        let name = |id: Option<&str>, kind: DeviceKind| {
            id.and_then(|id| devices.iter().find(|d| d.id == id))
                .map(|d| d.name.clone())
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| kind.label().to_string())
        };

        let mut radios: Vec<preflight::RadioSeen> = Vec::new();
        for c in connected.iter().filter(|c| c.kind == DeviceKind::Radio) {
            let card = c.id.as_ref().and_then(|id| {
                self.gear_card(&super::CardParams {
                    device: Some(id.clone()),
                    ..Default::default()
                })
                .ok()
            });
            radios.push(preflight::RadioSeen {
                name: name(c.id.as_deref(), c.kind),
                connected: true,
                selected_model: card.as_ref().and_then(|g| {
                    g.card
                        .selected_name
                        .clone()
                        .or_else(|| g.card.selected_model.clone())
                }),
                aircraft: card.and_then(|g| g.selected_aircraft),
                last_seen: None,
            });
        }
        for d in devices.iter().filter(|d| d.kind == DeviceKind::Radio) {
            radios.push(preflight::RadioSeen {
                name: name(Some(&d.id), d.kind),
                connected: false,
                selected_model: None,
                aircraft: None,
                last_seen: d
                    .last_seen
                    .map(|t| t.with_timezone(&chrono::Local).naive_local()),
            });
        }

        let mut cards = Vec::new();
        let mut cards_in = Vec::new();
        for c in &connected {
            let Link::Volume { mount, .. } = &c.link else {
                continue;
            };
            let n = name(c.id.as_deref(), c.kind);
            if matches!(c.kind, DeviceKind::Goggles | DeviceKind::DvrCard) {
                let (free, total) = space(mount);
                cards.push(preflight::CardSpace {
                    name: n.clone(),
                    free,
                    total,
                });
            }
            cards_in.push(n);
        }
        // A card unmounted but still inserted keeps its "still inserted" reminder.
        if !reminders.is_empty() && cards_in.is_empty() {
            cards_in.push("a card (unmounted)".into());
        }

        let backups = devices
            .iter()
            .filter(|d| matches!(d.kind, DeviceKind::Radio | DeviceKind::Fc))
            .map(|d| preflight::BackupSeen {
                name: name(Some(&d.id), d.kind),
                last_backup: d.last_backup.as_deref().and_then(backup_time),
            })
            .collect();

        Ok(preflight::check(&preflight::Input {
            packs: packs_view.packs,
            radios,
            cards,
            backups,
            cards_in,
            now: chrono::Local::now().naive_local(),
        }))
    }

    pub fn gear_crashes(&self, f: &CrashFilter) -> Result<Vec<Crash>> {
        crashes::list(&self.gear_store(), f)
    }

    /// Saves a crash. A new one on a clip takes the clip's aircraft and day.
    pub fn gear_crash_save(&self, p: &CrashSaveParams) -> Result<Crash> {
        let store = self.gear_store();
        let id = p.id.clone().unwrap_or_default();
        let mut c = if id.is_empty() {
            Crash::default()
        } else {
            crashes::list(&store, &CrashFilter::default())?
                .into_iter()
                .find(|c| c.id == id)
                .ok_or_else(|| anyhow::anyhow!("No crash {id:?}."))?
        };
        if let Some(clip) = &p.clip {
            c.clip = (!clip.is_empty()).then(|| clip.clone());
        }
        let lib = c.clip.as_ref().and_then(|id| {
            self.library_clips()
                .into_iter()
                .find(|x| &x.id == id || x.aliases.contains(id))
        });
        if c.clip.is_some() && lib.is_none() && id.is_empty() {
            bail!("No library clip {:?}.", c.clip.as_deref().unwrap_or(""));
        }
        if let Some(t) = p.time_s {
            c.time_s = Some(t);
        }
        if let (Some(t), Some(l)) = (c.time_s, &lib) {
            if t > l.duration + 0.5 {
                bail!("time_s {t} is past the clip's end ({:.1} s).", l.duration);
            }
        }
        if let Some(a) = &p.aircraft {
            c.aircraft = (!a.is_empty()).then(|| a.clone());
        } else if c.aircraft.is_none() {
            c.aircraft = lib.as_ref().and_then(|l| l.aircraft.clone());
        }
        c.day = p
            .day
            .or_else(|| (!id.is_empty()).then_some(c.day))
            .or_else(|| lib.as_ref().map(|l| l.date))
            .unwrap_or_else(|| chrono::Local::now().date_naive());
        if let Some(b) = &p.broke {
            c.broke = b.clone();
        }
        if let Some(parts) = &p.parts {
            c.parts = parts.clone();
        }
        if let Some(n) = &p.note {
            c.note = n.clone();
        }
        if let Some(r) = p.repaired {
            c.repaired = r;
        }
        let out = crashes::save(&store, &c)?;
        self.hooks.gear_changed();
        Ok(out)
    }

    pub fn gear_crash_delete(&self, id: &str) -> Result<Crash> {
        let out = crashes::delete(&self.gear_store(), id)?;
        self.hooks.gear_changed();
        Ok(out)
    }
}
