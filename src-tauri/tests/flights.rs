//! Flights and packs (WP12) through `Core`, the CLI and the MCP tools, on the synthetic log
//! (`gear::flights::synth`), whose measures are known. No real log, device or folder.

use quadcam_lib::api;
use quadcam_lib::core::{Core, NoHooks};
use quadcam_lib::gear::flights::synth::{self, KNOWN};
use quadcam_lib::gear::packs::{Pack, PackType};
use quadcam_lib::gear::serial::FakePorts;
use quadcam_lib::gear::Env;
use quadcam_lib::mcp::{LocalBackend, Server};
use quadcam_lib::photos::Recorder;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

const SETTINGS: &str = r#"{"profiles":[{"name":"Whoop","edgetx_models":["Whoop"],"place":"Field","gear":{"pack_type":"1S 300"}}]}"#;

/// A core whose settings file holds one profile, with the synthetic log in a folder it reads.
fn setup(dir: &Path) -> Arc<Core> {
    let support = dir.join("support");
    std::fs::create_dir_all(&support).unwrap();
    std::fs::write(support.join("settings.json"), SETTINGS).unwrap();
    let logs = dir.join("logs");
    synth::write_known(&logs).unwrap();
    let c = Core::new(
        dir.join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(support.join("settings.json"))
    .with_gear_env(Env::fake(vec![], Arc::new(FakePorts::new(vec![]))));
    c.gear_flight_folders(&api::FlightFoldersParams {
        add: Some(logs),
        remove: None,
    })
    .unwrap();
    Arc::new(c)
}

fn one_s() -> PackType {
    PackType {
        name: "1S 300".into(),
        cells: 1,
        warn_mah: Some(KNOWN.threshold),
        ..Default::default()
    }
}

#[test]
fn flights_carry_the_known_measures_and_joins() {
    let dir = tempfile::tempdir().unwrap();
    let c = setup(dir.path());
    c.gear_pack_type_save(&one_s()).unwrap();
    let v = c.gear_flights(&Default::default()).unwrap();
    assert_eq!(v.flights.len(), KNOWN.flights);
    assert_eq!(v.days.len(), 1);
    // The range trend: one place, its flights oldest first.
    assert_eq!(v.places.len(), 1);
    let starts: Vec<_> = v.places[0].points.iter().map(|p| p.start).collect();
    assert_eq!(starts.len(), KNOWN.flights);
    assert!(starts.windows(2).all(|w| w[0] < w[1]), "{starts:?}");
    // Newest first: flight 1 is last.
    let f1 = v.flights.last().unwrap();
    assert_eq!(f1.aircraft.as_deref(), Some("Whoop"));
    assert_eq!(f1.place.as_deref(), Some("Field"));
    let m = &f1.flight;
    assert!((m.hover.all.unwrap() - KNOWN.hover_all).abs() < 1e-6);
    assert!((m.sag_p5_v.unwrap() - KNOWN.sag_p5).abs() < 1e-6);
    assert!((m.resting_v.unwrap() - KNOWN.f1_resting).abs() < 1e-6);
    assert_eq!(m.mah, Some(KNOWN.f1_mah));
    assert_eq!(m.dropouts.len(), 1);
    // The threshold comes from the profile's pack type.
    assert_eq!(f1.threshold_mah, Some(KNOWN.threshold));
    assert_eq!(f1.crossed_at_s, Some(KNOWN.threshold_at_s));
    // The place trend holds every flight there.
    assert_eq!(v.places.len(), 1);
    assert_eq!(v.places[0].points.len(), KNOWN.flights);
    // Filters.
    let none = c
        .gear_flights(&api::FlightFilter {
            aircraft: Some("Other".into()),
            ..Default::default()
        })
        .unwrap();
    assert!(none.flights.is_empty());
}

#[test]
fn pack_history_and_next_label() {
    let dir = tempfile::tempdir().unwrap();
    let c = setup(dir.path());
    c.gear_pack_type_save(&one_s()).unwrap();
    for l in ["A1", "A2"] {
        c.gear_pack_save(&api::PackSaveParams {
            pack: Pack {
                label: l.into(),
                pack_type: Some("1S 300".into()),
                ..Default::default()
            },
            charged: Some(true),
        })
        .unwrap();
    }
    let v = c.gear_flights(&Default::default()).unwrap();
    let ids: Vec<String> = v
        .flights
        .iter()
        .rev()
        .map(|f| f.flight.id.clone())
        .collect();
    // With no pack set yet, the aircraft's type gives the first label.
    assert_eq!(
        v.flights.last().unwrap().suggested_pack.as_deref(),
        Some("A1")
    );
    let r = c
        .gear_flight_set(&api::FlightSetParams {
            flight: ids[0].clone(),
            pack: Some("A1".into()),
            place: None,
        })
        .unwrap();
    assert_eq!(r.pack.as_deref(), Some("A1"));
    assert!(c
        .gear_flight_set(&api::FlightSetParams {
            flight: "nope".into(),
            pack: Some("A1".into()),
            place: None,
        })
        .is_err());
    let v = c.gear_flights(&Default::default()).unwrap();
    let f2 = v.flights.iter().find(|f| f.flight.id == ids[1]).unwrap();
    assert_eq!(f2.suggested_pack.as_deref(), Some("A2"));

    let p = c.gear_packs(&Default::default()).unwrap();
    let a1 = p.packs.iter().find(|p| p.pack.label == "A1").unwrap();
    assert_eq!(a1.cycles, 1);
    assert_eq!(a1.history[0].mah, Some(KNOWN.f1_mah));
    assert_eq!(a1.history[0].resting_v, Some(KNOWN.f1_resting));
    // A1 flew after it was marked charged (the synthetic day is in the past, so it reads
    // as charged); A2 never flew.
    assert_eq!(p.types[0].packs, 2);
    assert_eq!(p.types[0].flights, 1);
    // The pack filter.
    let only = c
        .gear_flights(&api::FlightFilter {
            pack: Some("A1".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(only.flights.len(), 1);
}

#[test]
fn session_report_preflight_and_crashes() {
    let dir = tempfile::tempdir().unwrap();
    let c = setup(dir.path());
    let crash = c
        .gear_crash_save(&api::CrashSaveParams {
            aircraft: Some("Whoop".into()),
            day: Some("2026-10-04".parse().unwrap()),
            broke: Some("prop".into()),
            parts: Some(vec!["prop".into()]),
            ..Default::default()
        })
        .unwrap();
    assert!(c
        .gear_crash_save(&api::CrashSaveParams {
            clip: Some("no-such-clip".into()),
            ..Default::default()
        })
        .is_err());
    // No library and no import: the newest day with flights.
    let r = c.gear_session_report(&Default::default()).unwrap();
    assert_eq!(r.days, vec!["2026-10-04".parse().unwrap()]);
    assert_eq!(r.flights, KNOWN.flights);
    assert_eq!(r.crashes, vec![crash.clone()]);
    assert!(r.markdown.contains("- Flights: 3, air time 1:50"));

    let list = c
        .gear_crashes(&quadcam_lib::gear::crashes::CrashFilter {
            aircraft: Some("Whoop".into()),
            clip: None,
        })
        .unwrap();
    assert_eq!(list, vec![crash.clone()]);
    c.gear_crash_delete(&crash.id).unwrap();
    assert!(c.gear_crashes(&Default::default()).unwrap().is_empty());

    // Save the report: a new file, then a refusal until overwrite, then a folder and a gap.
    let out = dir.path().join("report.md");
    let save = |path: &Path, overwrite: bool| {
        c.gear_session_report_save(&api::ReportSaveParams {
            day: Some("2026-10-04".parse().unwrap()),
            path: path.to_path_buf(),
            overwrite,
        })
    };
    let saved = save(&out, false).unwrap();
    let md = std::fs::read_to_string(&out).unwrap();
    assert_eq!(saved.bytes, md.len() as u64);
    assert!(md.starts_with("# Session report: 2026-10-04"), "{md}");
    assert!(save(&out, false).is_err());
    assert!(save(&out, true).is_ok());
    assert!(save(dir.path(), true).is_err());
    assert!(save(&dir.path().join("nope/r.md"), true).is_err());

    let p = c.gear_preflight().unwrap();
    let ids: Vec<&str> = p.rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        ["packs", "radio", "cards_space", "backups", "cards_in"]
    );
}

#[test]
fn mcp_actions() {
    let dir = tempfile::tempdir().unwrap();
    let c = setup(dir.path());
    let mut s = Server::new(LocalBackend(c));
    let call = |s: &mut Server<LocalBackend>, name: &str, args: Value| -> Value {
        let r = s.call_tool(name, args.clone());
        assert_eq!(r["isError"], false, "{name} {args}: {r}");
        r
    };
    let text = |r: &Value| r["content"][0]["text"].as_str().unwrap_or("").to_string();

    let r = call(
        &mut s,
        "quadcam_gear_edit",
        json!({"action":"pack_type_save","name":"1S 300","cells":1,"warn_mah":300}),
    );
    assert!(text(&r).contains("1S 300"), "{r}");
    call(
        &mut s,
        "quadcam_gear_edit",
        json!({"action":"pack_save","name":"A1","pack_type":"1S 300","charged":true}),
    );
    let r = call(&mut s, "quadcam_gear", json!({"action":"flights"}));
    let flights = r["structuredContent"]["flights"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(flights.len(), KNOWN.flights);
    assert!(text(&r).contains("hover 43"), "{}", text(&r));
    let id = flights.last().unwrap()["flight"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    call(
        &mut s,
        "quadcam_gear_edit",
        json!({"action":"flight_set","flight":id,"pack":"A1"}),
    );
    let r = call(&mut s, "quadcam_gear", json!({"action":"packs"}));
    assert!(text(&r).contains("A1"), "{}", text(&r));
    let r = call(
        &mut s,
        "quadcam_gear",
        json!({"action":"session_report","day":"2026-10-04"}),
    );
    assert!(
        text(&r).starts_with("# Session report: 2026-10-04"),
        "{}",
        text(&r)
    );
    let file = dir.path().join("mcp-report.md");
    let r = call(
        &mut s,
        "quadcam_gear_edit",
        json!({"action":"report_save","day":"2026-10-04","to":file}),
    );
    assert!(
        text(&r).starts_with("Wrote the session report"),
        "{}",
        text(&r)
    );
    assert!(std::fs::read_to_string(&file)
        .unwrap()
        .starts_with("# Session report: 2026-10-04"));
    let again = s.call_tool(
        "quadcam_gear_edit",
        json!({"action":"report_save","day":"2026-10-04","to":file}),
    );
    assert_eq!(again["isError"], true, "{again}");
    let r = call(&mut s, "quadcam_gear", json!({"action":"preflight"}));
    assert!(text(&r).contains("Packs charged"), "{}", text(&r));
    let r = call(
        &mut s,
        "quadcam_gear_edit",
        json!({"action":"crash_save","aircraft":"Whoop","day":"2026-10-04","broke":"arm","parts":["arm"]}),
    );
    let cid = r["structuredContent"]["id"].as_str().unwrap().to_string();
    let r = call(
        &mut s,
        "quadcam_gear",
        json!({"action":"crashes","aircraft":"Whoop"}),
    );
    assert!(text(&r).contains("arm"), "{}", text(&r));
    call(
        &mut s,
        "quadcam_gear_edit",
        json!({"action":"crash_delete","id":cid}),
    );
}

#[test]
fn cli_report_markdown() {
    let home = tempfile::tempdir().unwrap();
    let logs = home.path().join("logs");
    synth::write_known(&logs).unwrap();
    let cli = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
            .env("HOME", home.path())
            .env("QUADCAM_PHOTOS", "dry-run")
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    cli(&[
        "--json",
        "gear",
        "flights",
        "folders",
        "--add",
        logs.to_str().unwrap(),
    ]);
    let v: Value = serde_json::from_str(cli(&["--json", "gear", "flights"]).trim()).unwrap();
    assert_eq!(
        v["result"]["flights"].as_array().unwrap().len(),
        KNOWN.flights
    );
    let md = cli(&["gear", "report", "--day", "2026-10-04", "--markdown"]);
    assert!(md.starts_with("# Session report: 2026-10-04"), "{md}");
    let file = home.path().join("r.md");
    let v: Value = serde_json::from_str(
        cli(&[
            "--json",
            "gear",
            "report",
            "--day",
            "2026-10-04",
            "--out",
            file.to_str().unwrap(),
        ])
        .trim(),
    )
    .unwrap();
    assert_eq!(v["result"]["bytes"], md.len() as u64);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap().trim_end(),
        md.trim_end()
    );
    cli(&[
        "--json",
        "gear",
        "packs",
        "type",
        "save",
        "1S 300",
        "--cells",
        "1",
        "--chemistry",
        "lihv",
    ]);
    cli(&[
        "--json",
        "gear",
        "packs",
        "save",
        "A1",
        "--type",
        "1S 300",
        "--charged",
    ]);
    let v: Value = serde_json::from_str(cli(&["--json", "gear", "preflight"]).trim()).unwrap();
    assert_eq!(v["result"]["rows"][0]["state"], "pass", "{v}");
}

/// Writes the mock core's Flights seed (`app/e2e/fixtures/flights.json`) from the real core
/// on the synthetic log, when `QUADCAM_UPDATE_FIXTURES=1`. Paths are made generic.
#[test]
fn record_mock_fixture() {
    if std::env::var("QUADCAM_UPDATE_FIXTURES").as_deref() != Ok("1") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let c = setup(dir.path());
    c.gear_pack_type_save(&PackType {
        capacity_mah: Some(300.0),
        full_v: Some(4.35),
        storage_v: Some(3.85),
        charge_a: Some(0.3),
        connector: Some("BT2.0".into()),
        chemistry: quadcam_lib::gear::packs::Chemistry::Lihv,
        ..one_s()
    })
    .unwrap();
    for l in ["A1", "A2", "A3"] {
        c.gear_pack_save(&api::PackSaveParams {
            pack: Pack {
                label: l.into(),
                pack_type: Some("1S 300".into()),
                ..Default::default()
            },
            charged: None,
        })
        .unwrap();
    }
    let v = c.gear_flights(&Default::default()).unwrap();
    let first = v.flights.last().unwrap().flight.id.clone();
    c.gear_flight_set(&api::FlightSetParams {
        flight: first,
        pack: Some("A1".into()),
        place: None,
    })
    .unwrap();
    let mut out = json!({
        "flights": c.gear_flights(&Default::default()).unwrap(),
        "packs": c.gear_packs(&Default::default()).unwrap(),
        "report": c.gear_session_report(&Default::default()).unwrap(),
        "preflight": c.gear_preflight().unwrap(),
    });
    // The mAh steps are long and the UI does not read them.
    fn strip(v: &mut Value) {
        match v {
            Value::Object(m) => {
                m.remove("capa");
                m.values_mut().for_each(strip);
            }
            Value::Array(a) => a.iter_mut().for_each(strip),
            _ => {}
        }
    }
    strip(&mut out);
    let text = serde_json::to_string_pretty(&out).unwrap().replace(
        &dir.path().to_string_lossy().to_string(),
        "/Users/pilot/fpv",
    );
    let dest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../app/e2e/fixtures/flights.json");
    std::fs::write(dest, text + "\n").unwrap();
}
