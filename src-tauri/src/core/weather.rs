//! `Core`'s weather: the wind, temperature and sky of library clips' places and hours, from
//! Open-Meteo (`crate::weather`), written into each clip file once. Refused while the
//! `weather` setting is off, before any request.

use super::Core;
use crate::api::{Event, LibraryTask, Task};
use crate::library::{self as lib, LibClip};
use crate::metadata::Place;
use crate::weather::{self, Weather};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

/// One weather run at a time.
static ONE: Mutex<()> = Mutex::new(());

/// `library_weather`: which clips.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct WeatherParams {
    /// Library clip ids; empty means every clip without weather, or with a record saved
    /// before the sky values (cuts excluded).
    pub ids: Vec<String>,
}

/// What a weather run did with one clip.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct WeatherClip {
    pub id: String,
    pub path: PathBuf,
    pub weather: Option<Weather>,
    /// The clip had its weather already; nothing was asked. A record without the sky values
    /// is not kept: it is looked up again.
    pub kept: bool,
    /// Why nothing was looked up or found: no place, no time, no data for that hour yet.
    pub skipped: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, specta::Type)]
pub struct WeatherReport {
    pub clips: Vec<WeatherClip>,
}

impl WeatherReport {
    /// Clips that got their weather in this run.
    pub fn found(&self) -> usize {
        self.clips
            .iter()
            .filter(|c| c.weather.is_some() && !c.kept && c.skipped.is_none() && c.error.is_none())
            .count()
    }
}

impl Core {
    fn weather_progress(&self, done: usize, total: usize) {
        self.hooks.event(Event::LibraryTask(LibraryTask {
            task: Task::Weather,
            done,
            total,
        }));
    }

    /// Looks up the time zone of each saved place that `wanted` picks and has none yet, and
    /// stores it on the place. Only while the `weather` setting is on; a failed lookup leaves
    /// the place without one, to be tried again on the next use. Returns the saved places.
    pub(super) fn place_zones(&self, wanted: &dyn Fn(&Place) -> bool) -> Vec<Place> {
        let d = self.defaults();
        if !d.weather {
            return d.places;
        }
        let tmp = self.cache.join("weather");
        let found: Vec<(Place, String)> = d
            .places
            .iter()
            .filter(|p| p.tz.is_none() && wanted(p))
            .filter_map(|p| {
                let z = weather::zone(self.weather_fetch.as_ref(), &tmp, p.lat, p.lon).ok()??;
                Some((p.clone(), z))
            })
            .collect();
        if found.is_empty() {
            return d.places;
        }
        let saved = self.settings_update(|v| {
            let mut places: Vec<Place> = crate::settings::list(v, "places");
            for (old, z) in &found {
                // Only the place as it was looked up: not one renamed or moved meanwhile.
                if let Some(p) = places
                    .iter_mut()
                    .find(|p| p.name == old.name && p.lat == old.lat && p.lon == old.lon)
                {
                    p.tz = Some(z.clone());
                }
            }
            v.insert("places".into(), serde_json::to_value(&places)?);
            Ok(places)
        });
        saved.unwrap_or_else(|e| {
            eprintln!("quadcam: place time zones not saved: {e:#}");
            self.defaults().places
        })
    }

    /// Looks up the wind, temperature and sky of library clips' places and hours and writes
    /// them into the clip files (`app.quadcam.weather`). `ids` empty means every clip without
    /// weather. A clip that has weather keeps it: it is never asked for again, except once
    /// when its record was saved before the sky values (`Weather::lacks_sky`); until that
    /// lookup finds the hour, the old record stays. A clip needs a location and a time of
    /// day. Refused while the `weather` setting is off.
    pub fn library_weather(&self, p: &WeatherParams) -> Result<WeatherReport> {
        if !self.defaults().weather {
            bail!("Weather is off. Turn on the weather setting first (Settings > Import, or settings set weather=true).");
        }
        let (root, mut clips) =
            self.with_index(|root, ix| Ok(((root.to_path_buf(), ix.clips.clone()), false)))?;
        clips.retain(|c| c.cut_of.is_none());
        if p.ids.is_empty() {
            clips.retain(|c| c.weather.as_ref().is_none_or(Weather::lacks_sky));
        } else {
            for id in &p.ids {
                if !clips.iter().any(|c| &c.id == id || c.aliases.contains(id)) {
                    bail!("No clip {id} in the library.");
                }
            }
            clips.retain(|c| p.ids.iter().any(|id| &c.id == id || c.aliases.contains(id)));
        }
        let mut report = WeatherReport::default();
        let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
        // A saved place first used here gets its time zone now.
        let places = self.place_zones(&|p| {
            clips.iter().any(|c| {
                crate::metadata::place_for(std::slice::from_ref(p), c.location.as_ref()).is_some()
            })
        });
        let tmp = self.cache.join("weather");
        let mut memo = weather::Memo::new();
        let total = clips.len();
        for (i, c) in clips.iter().enumerate() {
            self.weather_progress(i, total);
            let mut row = WeatherClip {
                id: c.id.clone(),
                path: c.path.clone(),
                weather: c.weather.clone(),
                kept: c.weather.as_ref().is_some_and(|w| !w.lacks_sky()),
                skipped: None,
                error: None,
            };
            if !row.kept {
                match self.clip_weather(&root, c, &places, &tmp, &mut memo) {
                    Ok(Ok(w)) => row.weather = Some(w),
                    Ok(Err(why)) => row.skipped = Some(why),
                    Err(e) => row.error = Some(format!("{e:#}")),
                }
            }
            report.clips.push(row);
        }
        self.weather_progress(total, total);
        if report.found() > 0 {
            self.hooks.library_changed();
        }
        Ok(report)
    }

    /// One clip's weather, written into its file and index entry; or why there is none.
    fn clip_weather(
        &self,
        root: &std::path::Path,
        c: &LibClip,
        places: &[Place],
        tmp: &std::path::Path,
        memo: &mut weather::Memo,
    ) -> Result<std::result::Result<Weather, String>> {
        let Some(l) = &c.location else {
            return Ok(Err("no place".into()));
        };
        let tz = crate::metadata::zone_for(places, Some(l));
        let Some(at) = crate::sun::local_instant(c.date, c.time.as_deref(), tz) else {
            return Ok(Err("no time of day".into()));
        };
        let Some(w) = weather::lookup(self.weather_fetch.as_ref(), tmp, memo, l.lat, l.lon, at)?
        else {
            return Ok(Err("Open-Meteo has no data for that hour yet".into()));
        };
        lib::write_keys(
            &root.join(&c.path),
            &[(lib::KEY_WEATHER, serde_json::to_string(&w)?)],
        )?;
        self.reread_clips(std::slice::from_ref(&c.id))?;
        Ok(Ok(w))
    }
}
