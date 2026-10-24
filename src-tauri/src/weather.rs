//! Weather at a clip's place and time: the wind, temperature and sky of that hour, from
//! Open-Meteo (open-meteo.com, free and keyless). Only when the `weather` setting is on.
//!
//! Open-Meteo's archive (Best Match: ERA5 reanalysis and its newer models) holds decades of
//! hours and answers up to the previous day; an hour it has no value for yet comes from its
//! historical forecast instead. The archive has no visibility, so that value comes only from
//! the historical forecast. A request names a day, the location rounded to two decimals
//! (about a kilometre), and the hourly values; nothing else leaves the Mac. One answer is
//! kept per clip, in the file (`app.quadcam.weather`), and never asked for again, except
//! once for a record saved before the sky values existed (`Weather::lacks_sky`).
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
const HOURLY: &str = "temperature_2m,wind_speed_10m,wind_gusts_10m,wind_direction_10m,\
cloud_cover,precipitation,rain,snowfall,weather_code,relative_humidity_2m,is_day,visibility";

/// The forecast API, asked only for a place's time zone (`zone`).
pub const FORECAST: &str = "https://api.open-meteo.com/v1/forecast";

/// The request for a place's time zone: the location rounded as for weather, no values.
pub fn zone_url(lat: f64, lon: f64) -> String {
    format!("{FORECAST}?latitude={lat:.2}&longitude={lon:.2}&timezone=auto&forecast_days=1")
}

/// The IANA time zone in an Open-Meteo answer asked with `timezone=auto`; None when it
/// names none that QuadCam knows.
pub fn parse_zone(body: &[u8]) -> Result<Option<String>> {
    #[derive(Deserialize)]
    struct Answer {
        #[serde(default)]
        timezone: Option<String>,
    }
    let a: Answer =
        serde_json::from_slice(body).context("Open-Meteo sent an answer QuadCam cannot read")?;
    Ok(a.timezone.filter(|z| z.parse::<chrono_tz::Tz>().is_ok()))
}

/// A place's IANA time zone, from Open-Meteo (`timezone=auto`). `tmp` is a folder for the
/// download.
pub fn zone(fetch: &dyn Fetch, tmp: &Path, lat: f64, lon: f64) -> Result<Option<String>> {
    std::fs::create_dir_all(tmp)?;
    let dest = tmp.join(format!("open-meteo-zone-{}.json", std::process::id()));
    let got = fetch
        .download(&zone_url(lat, lon), &dest)
        .and_then(|()| Ok(std::fs::read(&dest)?));
    let _ = std::fs::remove_file(&dest);
    parse_zone(&got?)
}

/// The sky of an hour, from its WMO weather code (`sky`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Sky {
    Clear,
    PartlyCloudy,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    HeavyRain,
    Sleet,
    Snow,
    Thunderstorm,
    Hail,
}

impl Sky {
    /// The sky as a word (`partly cloudy`).
    pub fn word(self) -> &'static str {
        match self {
            Sky::Clear => "clear",
            Sky::PartlyCloudy => "partly cloudy",
            Sky::Cloudy => "cloudy",
            Sky::Fog => "fog",
            Sky::Drizzle => "drizzle",
            Sky::Rain => "rain",
            Sky::HeavyRain => "heavy rain",
            Sky::Sleet => "sleet",
            Sky::Snow => "snow",
            Sky::Thunderstorm => "thunderstorm",
            Sky::Hail => "hail",
        }
    }
}

/// The sky for a WMO weather code (WMO 4677 as Open-Meteo uses it); None for a code outside
/// that table. Mainly clear reads as clear; freezing drizzle and freezing rain as sleet;
/// showers as rain or snow, violent showers as heavy rain; a storm with hail as hail. The
/// words have no night form: `is_day` picks only the icon (sun or moon) in the app.
pub fn sky(weather_code: u8) -> Option<Sky> {
    let s = match weather_code {
        0 | 1 => Sky::Clear,
        2 => Sky::PartlyCloudy,
        3 => Sky::Cloudy,
        45 | 48 => Sky::Fog,
        51 | 53 | 55 => Sky::Drizzle,
        56 | 57 | 66 | 67 => Sky::Sleet,
        61 | 63 | 80 | 81 => Sky::Rain,
        65 | 82 => Sky::HeavyRain,
        71 | 73 | 75 | 77 | 85 | 86 => Sky::Snow,
        95 => Sky::Thunderstorm,
        96 | 99 => Sky::Hail,
        _ => return None,
    };
    Some(s)
}

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
    /// Cloud cover, percent of the sky.
    #[serde(default)]
    pub cloud_cover_pct: Option<f64>,
    /// Rain, showers and snow of the hour, in mm of water.
    #[serde(default)]
    pub precipitation_mm: Option<f64>,
    #[serde(default)]
    pub rain_mm: Option<f64>,
    #[serde(default)]
    pub snowfall_cm: Option<f64>,
    /// The WMO weather code of the hour.
    #[serde(default)]
    pub weather_code: Option<u8>,
    /// The sky that code names (`sky`).
    #[serde(default)]
    pub sky: Option<Sky>,
    /// Relative humidity 2 m above ground, percent.
    #[serde(default)]
    pub humidity_pct: Option<f64>,
    /// Whether the sun was up.
    #[serde(default)]
    pub is_day: Option<bool>,
    /// Visibility in metres; only the historical forecast has it.
    #[serde(default)]
    pub visibility_m: Option<f64>,
}

impl Weather {
    /// A record saved before QuadCam asked for the sky: no cloud cover and no weather code.
    /// Such a clip is looked up once more.
    pub fn lacks_sky(&self) -> bool {
        self.cloud_cover_pct.is_none() && self.weather_code.is_none()
    }
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
        #[serde(default)]
        cloud_cover: Vec<Option<f64>>,
        #[serde(default)]
        precipitation: Vec<Option<f64>>,
        #[serde(default)]
        rain: Vec<Option<f64>>,
        #[serde(default)]
        snowfall: Vec<Option<f64>>,
        #[serde(default)]
        weather_code: Vec<Option<f64>>,
        #[serde(default)]
        relative_humidity_2m: Vec<Option<f64>>,
        #[serde(default)]
        is_day: Vec<Option<f64>>,
        #[serde(default)]
        visibility: Vec<Option<f64>>,
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
    let weather_code = at(&h.weather_code)
        .filter(|c| (0.0..=255.0).contains(c))
        .map(|c| c.round() as u8);
    Ok(Some(Weather {
        hour: format!("{want}Z"),
        temperature_c,
        wind_kmh,
        gusts_kmh: at(&h.wind_gusts_10m),
        wind_from_deg: at(&h.wind_direction_10m),
        source: source.into(),
        cloud_cover_pct: at(&h.cloud_cover),
        precipitation_mm: at(&h.precipitation),
        rain_mm: at(&h.rain),
        snowfall_cm: at(&h.snowfall),
        weather_code,
        sky: weather_code.and_then(sky),
        humidity_pct: at(&h.relative_humidity_2m),
        is_day: at(&h.is_day).map(|d| d >= 0.5),
        visibility_m: at(&h.visibility),
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
                // One name per process: the app and a CLI run may ask at once.
                let dest = tmp.join(format!("open-meteo-{}.json", std::process::id()));
                let got = fetch
                    .download(&u, &dest)
                    .and_then(|()| Ok(std::fs::read(&dest)?));
                let _ = std::fs::remove_file(&dest);
                let b = got?;
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

    /// One hourly day as Open-Meteo answers it. `visibility` false leaves it null, as the
    /// archive does.
    pub fn day_with(date: &str, temp: Option<f64>, visibility: bool) -> String {
        let times: Vec<String> = (0..24).map(|h| format!("{date}T{h:02}:00")).collect();
        let t: Vec<Option<f64>> = (0..24).map(|h| temp.map(|x| x + h as f64 / 10.0)).collect();
        let w: Vec<Option<f64>> = (0..24).map(|h| temp.map(|_| 10.0 + h as f64)).collect();
        let all = |x: f64| vec![temp.map(|_| x); 24];
        serde_json::json!({
            "latitude": 40.7, "longitude": -74.0,
            "hourly": {"time": times, "temperature_2m": t, "wind_speed_10m": w,
                "wind_gusts_10m": w.iter().map(|x| x.map(|v| v + 8.0)).collect::<Vec<_>>(),
                "wind_direction_10m": vec![Some(270.0); 24],
                "cloud_cover": all(75.0), "precipitation": all(1.2), "rain": all(1.2),
                "snowfall": all(0.0), "weather_code": all(61.0),
                "relative_humidity_2m": all(88.0),
                "is_day": (0..24).map(|h| temp.map(|_| if (6..20).contains(&h) { 1.0 } else { 0.0 })).collect::<Vec<_>>(),
                "visibility": vec![if visibility { temp.map(|_| 9000.0) } else { None }; 24]}
        })
        .to_string()
    }

    pub fn day(date: &str, temp: Option<f64>) -> String {
        day_with(date, temp, true)
    }

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn a_zone_comes_from_the_answer_and_must_be_known() {
        let z = |b: &str| parse_zone(b.as_bytes()).unwrap();
        assert_eq!(
            z(r#"{"latitude":51.5,"timezone":"Europe/London","utc_offset_seconds":3600}"#),
            Some("Europe/London".into())
        );
        assert_eq!(z(r#"{"timezone":"Mars/Olympus"}"#), None);
        assert_eq!(z(r#"{"latitude":1}"#), None);
        assert!(parse_zone(b"not json").is_err());
        let u = zone_url(51.5074, -0.1278);
        assert!(
            u.starts_with(FORECAST) && u.contains("latitude=51.51&longitude=-0.13&timezone=auto"),
            "{u}"
        );
        let fake = FakeMeteo {
            bodies: vec![(FORECAST.into(), r#"{"timezone":"Asia/Tokyo"}"#.into())],
            ..Default::default()
        };
        let d = tempfile::tempdir().unwrap();
        assert_eq!(
            zone(&fake, d.path(), 35.7, 139.7).unwrap(),
            Some("Asia/Tokyo".into())
        );
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
        assert_eq!(w.cloud_cover_pct, Some(75.0));
        assert_eq!(w.precipitation_mm, Some(1.2));
        assert_eq!(w.weather_code, Some(61));
        assert_eq!(w.sky, Some(Sky::Rain));
        assert_eq!(w.humidity_pct, Some(88.0));
        assert_eq!(w.is_day, Some(true));
        assert_eq!(w.visibility_m, Some(9000.0));
        assert!(!w.lacks_sky());
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

    #[test]
    fn the_archive_has_no_visibility_and_null_is_none() {
        let at = utc("2026-09-28T03:00:00Z");
        let w = parse(
            day_with("2026-09-28", Some(5.0), false).as_bytes(),
            at,
            "archive",
        )
        .unwrap()
        .unwrap();
        assert_eq!(w.visibility_m, None);
        assert_eq!(w.is_day, Some(false));
        assert_eq!(w.sky, Some(Sky::Rain));
        // An answer without the sky values at all still reads.
        let old = br#"{"hourly":{"time":["2026-09-28T03:00"],"temperature_2m":[5.0],"wind_speed_10m":[9.0],"cloud_cover":[null],"weather_code":[null]}}"#;
        let w = parse(old, at, "archive").unwrap().unwrap();
        assert_eq!(
            (w.cloud_cover_pct, w.weather_code, w.sky),
            (None, None, None)
        );
        assert!(w.lacks_sky());
    }

    #[test]
    fn a_record_saved_before_the_sky_values_still_reads() {
        let w: Weather = serde_json::from_str(
            r#"{"hour":"2026-09-28T18:00Z","temperature_c":18.2,"wind_kmh":12.0,"gusts_kmh":20.0,"wind_from_deg":270.0,"source":"archive"}"#,
        )
        .unwrap();
        assert_eq!(w.wind_kmh, 12.0);
        assert_eq!((w.sky, w.visibility_m, w.is_day), (None, None, None));
        assert!(w.lacks_sky());
    }

    #[test]
    fn every_wmo_code_group_names_a_sky() {
        let groups: &[(&[u8], Sky)] = &[
            (&[0, 1], Sky::Clear),
            (&[2], Sky::PartlyCloudy),
            (&[3], Sky::Cloudy),
            (&[45, 48], Sky::Fog),
            (&[51, 53, 55], Sky::Drizzle),
            (&[56, 57], Sky::Sleet),
            (&[61, 63], Sky::Rain),
            (&[65], Sky::HeavyRain),
            (&[66, 67], Sky::Sleet),
            (&[71, 73, 75, 77], Sky::Snow),
            (&[80, 81], Sky::Rain),
            (&[82], Sky::HeavyRain),
            (&[85, 86], Sky::Snow),
            (&[95], Sky::Thunderstorm),
            (&[96, 99], Sky::Hail),
        ];
        let mut known = 0;
        for (codes, want) in groups {
            for &c in *codes {
                assert_eq!(sky(c), Some(*want), "code {c}");
                known += 1;
            }
        }
        // Every other code is outside the table.
        let unknown = (0..=255u8).filter(|&c| sky(c).is_none()).count();
        assert_eq!(known + unknown, 256);
        for c in [4, 10, 44, 50, 60, 70, 90, 98, 100, 255] {
            assert_eq!(sky(c), None, "code {c}");
        }
        assert_eq!(Sky::PartlyCloudy.word(), "partly cloudy");
        assert_eq!(
            serde_json::to_value(Sky::HeavyRain).unwrap(),
            serde_json::json!("heavy_rain")
        );
    }
}
