//! Weather at a clip's place and time: the wind and temperature of that hour, from
//! Open-Meteo (open-meteo.com, free and keyless). Only when the `weather` setting is on.
//!
//! Open-Meteo's archive (ERA5 reanalysis) holds decades of hours but runs about five days
//! behind; a newer hour comes from its historical forecast instead. A request names a day,
//! the location rounded to two decimals (about a kilometre), and the hourly values; nothing
//! else leaves the Mac. One answer is kept per clip, in the file (`app.quadcam.weather`),
//! and never asked for again.
//!
//! Downloads go through `modules::fetch::Fetch`, so a process started by cargo never reaches
//! the internet and tests answer with a fake.

use crate::modules::fetch::Fetch;
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, DurationRound, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

pub const ARCHIVE: &str = "https://archive-api.open-meteo.com/v1/archive";
pub const HISTORICAL_FORECAST: &str = "https://historical-forecast-api.open-meteo.com/v1/forecast";
const HOURLY: &str = "temperature_2m,wind_speed_10m,wind_gusts_10m,wind_direction_10m";

/// The weather of one hour at one place. Metric: degrees Celsius and km/h.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct Weather {
    /// The hour it describes, in UTC (`2026-09-28T18:00Z`).
    pub hour: String,
    pub temperature_c: f64,
    /// Mean wind 10 m above ground.
    pub wind_kmh: f64,
    #[serde(default)]
    pub gusts_kmh: Option<f64>,
    /// Where the wind came from, in degrees (0 north, 90 east).
    #[serde(default)]
    pub wind_from_deg: Option<f64>,
    /// `archive` or `historical_forecast`.
    pub source: String,
}

/// The hour nearest `t`.
pub fn nearest_hour(t: DateTime<Utc>) -> DateTime<Utc> {
    (t + Duration::minutes(30))
        .duration_trunc(Duration::hours(1))
        .unwrap_or(t)
}

/// The request for one day at one place.
pub fn url(base: &str, lat: f64, lon: f64, day: chrono::NaiveDate) -> String {
    let d = day.format("%Y-%m-%d");
    format!(
        "{base}?latitude={lat:.2}&longitude={lon:.2}&start_date={d}&end_date={d}&hourly={HOURLY}&wind_speed_unit=kmh&temperature_unit=celsius&timezone=GMT"
    )
}

/// The values for `hour` in an Open-Meteo answer; None when the answer has no value for it
/// (an hour the source does not cover yet).
pub fn parse(body: &[u8], hour: DateTime<Utc>, source: &str) -> Result<Option<Weather>> {
    #[derive(Deserialize)]
    struct Hourly {
        time: Vec<String>,
        temperature_2m: Vec<Option<f64>>,
        wind_speed_10m: Vec<Option<f64>>,
        #[serde(default)]
        wind_gusts_10m: Vec<Option<f64>>,
        #[serde(default)]
        wind_direction_10m: Vec<Option<f64>>,
    }
    #[derive(Deserialize)]
    struct Answer {
        hourly: Option<Hourly>,
        #[serde(default)]
        reason: Option<String>,
    }
    let a: Answer =
        serde_json::from_slice(body).context("Open-Meteo sent an answer QuadCam cannot read")?;
    let Some(h) = a.hourly else {
        anyhow::bail!(
            "Open-Meteo refused: {}",
            a.reason.unwrap_or_else(|| "no hourly values".into())
        );
    };
    let want = hour.format("%Y-%m-%dT%H:%M").to_string();
    let Some(i) = h.time.iter().position(|t| *t == want) else {
        return Ok(None);
    };
    let at = |v: &[Option<f64>]| v.get(i).copied().flatten();
    let (Some(temperature_c), Some(wind_kmh)) = (at(&h.temperature_2m), at(&h.wind_speed_10m))
    else {
        return Ok(None);
    };
    Ok(Some(Weather {
        hour: format!("{want}Z"),
        temperature_c,
        wind_kmh,
        gusts_kmh: at(&h.wind_gusts_10m),
        wind_from_deg: at(&h.wind_direction_10m),
        source: source.into(),
    }))
}

/// Answers already downloaded in one run, by URL, so clips of one day and place share one
/// request.
pub type Memo = HashMap<String, Vec<u8>>;

/// The weather at a place and moment: the archive first, then the historical forecast.
/// None when neither has that hour yet. `tmp` is a folder for the download.
pub fn lookup(
    fetch: &dyn Fetch,
    tmp: &Path,
    memo: &mut Memo,
    lat: f64,
    lon: f64,
    at: DateTime<Utc>,
) -> Result<Option<Weather>> {
    let hour = nearest_hour(at);
    for (base, source) in [
        (ARCHIVE, "archive"),
        (HISTORICAL_FORECAST, "historical_forecast"),
    ] {
        let u = url(base, lat, lon, hour.date_naive());
        let body = match memo.get(&u) {
            Some(b) => b.clone(),
            None => {
                std::fs::create_dir_all(tmp)?;
                let dest = tmp.join("open-meteo.json");
                fetch.download(&u, &dest)?;
                let b = std::fs::read(&dest)?;
                let _ = std::fs::remove_file(&dest);
                memo.insert(u.clone(), b.clone());
                b
            }
        };
        if let Some(w) = parse(&body, hour, source)? {
            return Ok(Some(w));
        }
    }
    Ok(None)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Answers every request from a table of bodies by URL prefix, and records the URLs.
    #[derive(Default)]
    pub struct FakeMeteo {
        pub bodies: Vec<(String, String)>,
        pub asked: Mutex<Vec<String>>,
    }

    impl Fetch for FakeMeteo {
        fn download(&self, url: &str, dest: &Path) -> Result<()> {
            self.asked.lock().unwrap().push(url.to_string());
            let body = self
                .bodies
                .iter()
                .find(|(p, _)| url.starts_with(p.as_str()))
                .map(|(_, b)| b.clone())
                .context("no fake answer")?;
            std::fs::write(dest, body)?;
            Ok(())
        }
    }

    pub fn day(date: &str, temp: Option<f64>) -> String {
        let times: Vec<String> = (0..24).map(|h| format!("{date}T{h:02}:00")).collect();
        let t: Vec<Option<f64>> = (0..24).map(|h| temp.map(|x| x + h as f64 / 10.0)).collect();
        let w: Vec<Option<f64>> = (0..24).map(|h| temp.map(|_| 10.0 + h as f64)).collect();
        serde_json::json!({
            "latitude": 40.7, "longitude": -74.0,
            "hourly": {"time": times, "temperature_2m": t, "wind_speed_10m": w,
                "wind_gusts_10m": w.iter().map(|x| x.map(|v| v + 8.0)).collect::<Vec<_>>(),
                "wind_direction_10m": vec![Some(270.0); 24]}
        })
        .to_string()
    }

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn the_nearest_hour_rounds_half_up() {
        assert_eq!(
            nearest_hour(utc("2026-09-28T18:29:00Z")),
            utc("2026-09-28T18:00:00Z")
        );
        assert_eq!(
            nearest_hour(utc("2026-09-28T18:30:00Z")),
            utc("2026-09-28T19:00:00Z")
        );
        assert_eq!(
            nearest_hour(utc("2026-09-28T23:45:00Z")),
            utc("2026-09-29T00:00:00Z")
        );
    }

    #[test]
    fn the_request_names_a_day_and_a_rounded_place() {
        let u = url(
            ARCHIVE,
            40.689247,
            -74.044502,
            "2026-09-28".parse().unwrap(),
        );
        assert!(u.starts_with("https://archive-api.open-meteo.com/v1/archive?"));
        assert!(u.contains("latitude=40.69&longitude=-74.04"), "{u}");
        assert!(u.contains("start_date=2026-09-28&end_date=2026-09-28"));
        assert!(u.contains("timezone=GMT"));
    }

    #[test]
    fn reads_the_hour_and_takes_the_forecast_when_the_archive_is_behind() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeMeteo {
            bodies: vec![
                (ARCHIVE.into(), day("2026-09-28", None)),
                (HISTORICAL_FORECAST.into(), day("2026-09-28", Some(18.0))),
            ],
            ..Default::default()
        };
        let mut memo = Memo::new();
        let at = utc("2026-09-28T18:10:00Z");
        let w = lookup(&fake, d.path(), &mut memo, 40.7, -74.0, at)
            .unwrap()
            .unwrap();
        assert_eq!(w.hour, "2026-09-28T18:00Z");
        assert!((w.temperature_c - 19.8).abs() < 1e-9);
        assert_eq!(w.wind_kmh, 28.0);
        assert_eq!(w.gusts_kmh, Some(36.0));
        assert_eq!(w.wind_from_deg, Some(270.0));
        assert_eq!(w.source, "historical_forecast");
        // A second clip that day asks nothing new.
        lookup(&fake, d.path(), &mut memo, 40.7, -74.0, at).unwrap();
        assert_eq!(fake.asked.lock().unwrap().len(), 2);
    }

    #[test]
    fn no_value_is_no_weather_and_a_refusal_is_an_error() {
        let at = utc("2026-09-28T18:00:00Z");
        assert_eq!(
            parse(day("2026-09-28", None).as_bytes(), at, "archive").unwrap(),
            None
        );
        assert_eq!(
            parse(day("2026-09-27", Some(1.0)).as_bytes(), at, "archive").unwrap(),
            None
        );
        let e = parse(
            br#"{"error":true,"reason":"Parameter is out of range"}"#,
            at,
            "archive",
        )
        .unwrap_err();
        assert!(format!("{e:#}").contains("out of range"), "{e:#}");
    }
}
