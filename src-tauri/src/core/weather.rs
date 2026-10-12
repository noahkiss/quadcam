//! `Core`'s weather: the wind and temperature of library clips' places and hours, from
//! Open-Meteo (`crate::weather`), written into each clip file once. Refused while the
//! `weather` setting is off, before any request.

use super::Core;
use crate::api::{Event, LibraryTask, Task};
use crate::library::{self as lib, LibClip};
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
    /// Library clip ids; empty means every clip without weather (cuts excluded).
    pub ids: Vec<String>,
}

/// What a weather run did with one clip.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct WeatherClip {
    pub id: String,
    pub path: PathBuf,
    pub weather: Option<Weather>,
    /// The clip had its weather already; nothing was asked.
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
            .filter(|c| c.weather.is_some() && !c.kept)
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

    /// Looks up the wind and temperature of library clips' places and hours and writes them
    /// into the clip files (`app.quadcam.weather`). `ids` empty means every clip without
    /// weather. A clip that has weather keeps it: it is never asked for again. A clip needs a
    /// location and a time of day. Refused while the `weather` setting is off.
    pub fn library_weather(&self, p: &WeatherParams) -> Result<WeatherReport> {
        if !self.defaults().weather {
            bail!("Weather is off. Turn on the weather setting first (Settings > Import, or settings set weather=true).");
        }
        let (root, mut clips) =
            self.with_index(|root, ix| Ok(((root.to_path_buf(), ix.clips.clone()), false)))?;
        clips.retain(|c| c.cut_of.is_none());
        if p.ids.is_empty() {
            clips.retain(|c| c.weather.is_none());
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
        let tmp = self.cache.join("weather");
        let mut memo = weather::Memo::new();
        let total = clips.len();
        for (i, c) in clips.iter().enumerate() {
            self.weather_progress(i, total);
            let mut row = WeatherClip {
                id: c.id.clone(),
                path: c.path.clone(),
                weather: c.weather.clone(),
                kept: c.weather.is_some(),
                skipped: None,
                error: None,
            };
            if !row.kept {
                match self.clip_weather(&root, c, &tmp, &mut memo) {
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
        tmp: &std::path::Path,
        memo: &mut weather::Memo,
    ) -> Result<std::result::Result<Weather, String>> {
        let Some(l) = &c.location else {
            return Ok(Err("no place".into()));
        };
        let Some(at) = crate::sun::local_instant(c.date, c.time.as_deref()) else {
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
