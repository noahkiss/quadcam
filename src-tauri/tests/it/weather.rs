//! Weather (`weather`, `core/weather.rs`) and sun tags (`sun`) on library clips, through the
//! core, MCP and the CLI. Open-Meteo is a fake: no test reaches the network.

use crate::common::*;
use quadcam_lib::core::{Core, LibEdit, NoHooks, WeatherParams};
use quadcam_lib::library as lib;
use quadcam_lib::metadata::Location;
use quadcam_lib::modules::fetch::Fetch;
use quadcam_lib::photos::Recorder;
use serde_json::json;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Open-Meteo stand-in: an hourly day for the asked date, with no values from the archive
/// when `archive_behind`. Records every URL.
#[derive(Default)]
struct FakeMeteo {
    archive_behind: bool,
    asked: Mutex<Vec<String>>,
}

impl FakeMeteo {
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl Fetch for FakeMeteo {
    fn download(&self, url: &str, dest: &Path) -> anyhow::Result<()> {
        self.asked.lock().unwrap().push(url.to_string());
        let day = url
            .split("start_date=")
            .nth(1)
            .and_then(|r| r.get(..10))
            .unwrap()
            .to_string();
        let empty = self.archive_behind && url.starts_with(quadcam_lib::weather::ARCHIVE);
        let v = |x: f64| if empty { None } else { Some(x) };
        let body = json!({
            "hourly": {
                "time": (0..24).map(|h| format!("{day}T{h:02}:00")).collect::<Vec<_>>(),
                "temperature_2m": (0..24).map(|h| v(10.0 + h as f64)).collect::<Vec<_>>(),
                "wind_speed_10m": (0..24).map(|_| v(14.0)).collect::<Vec<_>>(),
                "wind_gusts_10m": (0..24).map(|_| v(25.0)).collect::<Vec<_>>(),
                "wind_direction_10m": (0..24).map(|_| v(200.0)).collect::<Vec<_>>(),
            }
        });
        std::fs::write(dest, body.to_string())?;
        Ok(())
    }
}

fn core(dir: &Path, fake: &Arc<FakeMeteo>, weather: bool) -> Core {
    let c = Core::new(
        dir.join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_weather_fetch(fake.clone());
    let mut d = c.defaults();
    d.output_dir = Some(dir.join("out"));
    d.weather = weather;
    d.video_analysis = false;
    std::fs::create_dir_all(dir.join("out")).unwrap();
    c.set_defaults(d);
    c
}

/// One imported 5 s clip; its id.
fn imported(core: &Core) -> (tempfile::TempDir, String) {
    let src = tempfile::tempdir().unwrap();
    make_clip(&src.path().join("PICT0001.AVI"), 5, false);
    core.load(Some(src.path())).unwrap();
    core.import(&Default::default()).unwrap();
    let id = core.library(&Default::default()).unwrap().clips[0]
        .clip
        .id
        .clone();
    (src, id)
}

fn place(core: &Core, id: &str, time: &str) {
    core.library_edit(
        id,
        &LibEdit {
            location: Some(Location {
                lat: 40.6892,
                lon: -74.0445,
                name: None,
            }),
            date: Some("2026-06-21".parse().unwrap()),
            time: Some(time.into()),
            ..Default::default()
        },
    )
    .unwrap();
}

fn item(file: &Path, key: &str) -> Option<String> {
    let items = quadcam_lib::qtmeta::read(file).unwrap();
    quadcam_lib::qtmeta::get(&items, key).map(str::to_string)
}

#[test]
fn weather_off_asks_nothing() {
    let work = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeMeteo::default());
    let core = core(work.path(), &fake, false);
    let (_src, id) = imported(&core);
    place(&core, &id, "18:30");
    let e = core.library_weather(&WeatherParams::default()).unwrap_err();
    assert!(format!("{e:#}").contains("Weather is off"), "{e:#}");
    let e = core
        .library_weather(&WeatherParams { ids: vec![id] })
        .unwrap_err();
    assert!(format!("{e:#}").contains("Weather is off"), "{e:#}");
    assert!(fake.asked().is_empty());
}

#[test]
fn weather_is_looked_up_once_and_a_new_time_makes_it_stale() {
    let work = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeMeteo::default());
    let core = core(work.path(), &fake, true);
    let (_src, id) = imported(&core);

    // No place and no time: nothing to ask.
    let r = core.library_weather(&WeatherParams::default()).unwrap();
    assert_eq!(r.clips[0].skipped.as_deref(), Some("no place"), "{r:?}");
    assert!(fake.asked().is_empty());

    place(&core, &id, "18:30");
    let r = core.library_weather(&WeatherParams::default()).unwrap();
    let w = r.clips[0].weather.clone().expect("weather found");
    assert_eq!(r.found(), 1);
    assert_eq!(
        (w.wind_kmh, w.gusts_kmh, w.wind_from_deg),
        (14.0, Some(25.0), Some(200.0))
    );
    assert_eq!(w.source, "archive");
    let asked = fake.asked();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(
        asked[0].contains("latitude=40.69&longitude=-74.04"),
        "{asked:?}"
    );

    // In the file and in the library view, with the sun tag beside it.
    let it = core.library(&Default::default()).unwrap().clips[0].clone();
    assert_eq!(it.clip.weather.as_ref(), Some(&w));
    let saved: serde_json::Value =
        serde_json::from_str(&item(&it.file, lib::KEY_WEATHER).unwrap()).unwrap();
    assert_eq!(saved["wind_kmh"], 14.0);
    let view = serde_json::to_value(&it).unwrap();
    assert!(view.get("sun").is_some(), "{view}");

    // Never asked again: a run over every clip skips it, a run by id keeps it.
    let r = core.library_weather(&WeatherParams::default()).unwrap();
    assert!(r.clips.is_empty(), "{r:?}");
    let r = core
        .library_weather(&WeatherParams {
            ids: vec![id.clone()],
        })
        .unwrap();
    assert!(r.clips[0].kept && r.found() == 0, "{r:?}");
    assert_eq!(fake.asked().len(), 1);

    // A new time of day removes it; the next run asks for the new hour.
    place(&core, &id, "07:00");
    let it = core.library(&Default::default()).unwrap().clips[0].clone();
    assert!(it.clip.weather.is_none());
    assert_eq!(item(&it.file, lib::KEY_WEATHER), None);
    assert_eq!(
        core.library_weather(&WeatherParams::default())
            .unwrap()
            .found(),
        1
    );
    // The same place and hour again changes nothing.
    place(&core, &id, "07:00");
    assert!(core.library(&Default::default()).unwrap().clips[0]
        .clip
        .weather
        .is_some());
}

#[test]
fn a_recent_hour_comes_from_the_historical_forecast() {
    let work = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeMeteo {
        archive_behind: true,
        ..Default::default()
    });
    let core = core(work.path(), &fake, true);
    let (_src, id) = imported(&core);
    place(&core, &id, "12:00");
    let r = core.library_weather(&WeatherParams::default()).unwrap();
    assert_eq!(
        r.clips[0].weather.as_ref().unwrap().source,
        "historical_forecast"
    );
    let asked = fake.asked();
    assert_eq!(asked.len(), 2);
    assert!(asked[0].starts_with(quadcam_lib::weather::ARCHIVE));
    assert!(asked[1].starts_with(quadcam_lib::weather::HISTORICAL_FORECAST));
}

#[test]
fn mcp_shows_weather_and_sun_on_library_clips() {
    use quadcam_lib::mcp::{LocalBackend, Server};
    let work = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeMeteo::default());
    let c = core(work.path(), &fake, true);
    let (_src, id) = imported(&c);
    place(&c, &id, "18:30");
    let mut s = Server::new(LocalBackend(Arc::new(c)));
    let r = s.call_tool("quadcam_library_files", json!({"action": "weather"}));
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("wind 14 km/h from 200° (gusts 25)"), "{text}");
    let r = s.call_tool("quadcam_library", json!({}));
    let clip = &r["structuredContent"]["clips"][0];
    assert_eq!(clip["weather"]["wind_kmh"], 14.0, "{r}");
    assert!(clip.get("sun").is_some(), "{r}");
    let r = s.call_tool("quadcam_library_files", json!({"action": "weather"}));
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Every clip has its weather already"));
    // Location lookups stay off under cargo.
    let r = s.call_tool("quadcam_places", json!({"action": "here"}));
    assert_eq!(r["isError"], true, "{r}");
    let r = s.call_tool("quadcam_places", json!({"action": "reverse", "lat": 40.0}));
    assert!(r.to_string().contains("lat and lon are required"), "{r}");
}

#[test]
fn the_cli_refuses_weather_while_the_setting_is_off() {
    let home = tempfile::tempdir().unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
        .env("HOME", home.path())
        .env("QUADCAM_PHOTOS", "dry-run")
        .args(["--json", "library", "weather"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], false, "{v}");
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Weather is off"));
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
        .env("HOME", home.path())
        .args(["--json", "places", "here"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("off in tests"),
        "{v}"
    );
}
