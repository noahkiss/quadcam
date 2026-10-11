//! On-device suggestions (`core/ondevice.rs`) through `Core`, the CLI's core calls and the
//! MCP tool, with `intel::fake::FakeHelper` in place of Apple's model. No real model runs.

use crate::common::*;
use quadcam_lib::api;
use quadcam_lib::core::{Core, NoHooks, OnDeviceParams};
use quadcam_lib::gear::flights::synth;
use quadcam_lib::gear::serial::FakePorts;
use quadcam_lib::gear::Env;
use quadcam_lib::intel::fake::FakeHelper;
use quadcam_lib::intel::Intel;
use quadcam_lib::mcp::{LocalBackend, Server};
use quadcam_lib::photos::Recorder;
use quadcam_lib::session::{Editor, PlanPatch};
use serde_json::json;
use std::path::Path;
use std::sync::Arc;

fn helper() -> Arc<FakeHelper> {
    Arc::new(FakeHelper::new(Box::new(|_| None)))
}

/// A core with two loaded clips and the fake helper.
fn loaded(dir: &Path, fake: Arc<FakeHelper>) -> Core {
    let c = Core::new(
        dir.join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_intel(Arc::new(Intel::with("/helper".into(), fake)));
    let mut d = c.defaults();
    d.output_dir = Some(dir.join("out"));
    std::fs::create_dir_all(dir.join("out")).unwrap();
    c.set_defaults(d);
    let src = dir.join("card");
    std::fs::create_dir_all(&src).unwrap();
    make_clip(&src.join("PICT0001.AVI"), 2, true);
    make_clip(&src.join("PICT0002.AVI"), 1, true);
    c.load(Some(&src)).unwrap();
    c
}

#[test]
fn an_outing_shares_one_name_with_pack_numbers() {
    let dir = tempfile::tempdir().unwrap();
    let fake = helper();
    let c = loaded(dir.path(), fake.clone());
    let r = c.suggest_on_device(&OnDeviceParams::default()).unwrap();
    assert!(r.available, "{r:?}");
    assert_eq!(r.clips.len(), 2);
    let s = c.session().unwrap();
    let names: Vec<&str> = s.plans.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["fake outing pack 1", "fake outing pack 2"]);
    for p in &s.plans {
        let od = p.ondevice.as_ref().unwrap();
        assert_eq!(od.name, p.name);
        assert_eq!(od.words, format!("fake {}", p.id));
        assert!(p.suggested.name && p.suggested.note && p.suggested.meta);
        assert_eq!(p.note, format!("Fake note for clip {}.", p.id));
        assert_eq!(p.meta.keywords, ["fake", "whoop"]);
    }
    assert_eq!(r.clips[0].filled, ["name", "note", "keywords"]);
    // One request for the outing: each clip's facts and its thumbnail.
    let reqs = fake.requests.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    let clips = reqs[0]["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 2);
    assert!(clips[0]["facts"].as_str().unwrap().contains("Length: 0:02"));
    assert_eq!(clips[0]["frames"].as_array().unwrap().len(), 1);
}

#[test]
fn a_typed_name_stays_and_a_new_run_replaces_only_its_own_values() {
    let dir = tempfile::tempdir().unwrap();
    let mut fake = FakeHelper::new(Box::new(|_| None));
    let round = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let r2 = round.clone();
    fake.answer = Some(Box::new(move |req| {
        let n = r2.load(std::sync::atomic::Ordering::SeqCst);
        let mut out = String::new();
        for c in req["clips"].as_array().unwrap() {
            out.push_str(
                &json!({"kind": "suggestion", "id": c["id"], "name": format!("idea {n}"), "keywords": [], "note": ""})
                    .to_string(),
            );
            out.push('\n');
        }
        out.push_str(&json!({"kind": "group", "name": format!("outing {n}")}).to_string());
        out
    }));
    let c = loaded(dir.path(), Arc::new(fake));
    c.patch(
        &[PlanPatch {
            id: 0,
            name: Some("my own name".into()),
            ..Default::default()
        }],
        Editor::User,
    )
    .unwrap();
    c.suggest_on_device(&OnDeviceParams::default()).unwrap();
    let s = c.session().unwrap();
    assert_eq!(s.plans[0].name, "my own name");
    assert!(!s.plans[0].suggested.name);
    assert_eq!(
        s.plans[0].ondevice.as_ref().unwrap().name,
        "outing 0 pack 1"
    );
    assert_eq!(s.plans[1].name, "outing 0 pack 2");

    // A second run: its own earlier value gives way, the typed name and an agent's stay.
    round.store(1, std::sync::atomic::Ordering::SeqCst);
    c.patch(
        &[PlanPatch {
            id: 1,
            note: Some("an agent's note".into()),
            ..Default::default()
        }],
        Editor::Agent,
    )
    .unwrap();
    let r = c.suggest_on_device(&OnDeviceParams::default()).unwrap();
    let s = c.session().unwrap();
    assert_eq!(s.plans[0].name, "my own name");
    assert_eq!(s.plans[1].name, "outing 1 pack 2");
    assert_eq!(s.plans[1].note, "an agent's note");
    assert_eq!(r.clips[0].filled, Vec::<String>::new());
    assert_eq!(r.clips[1].filled, ["name"]);

    // The person edits the suggested name: it is theirs now.
    c.patch(
        &[PlanPatch {
            id: 1,
            name: Some("outing 1 pack 2".into()),
            ..Default::default()
        }],
        Editor::User,
    )
    .unwrap();
    round.store(2, std::sync::atomic::Ordering::SeqCst);
    c.suggest_on_device(&OnDeviceParams { ids: Some(vec![1]) })
        .unwrap();
    assert_eq!(c.session().unwrap().plans[1].name, "outing 1 pack 2");
}

#[test]
fn the_setting_off_or_no_model_skips_silently() {
    let dir = tempfile::tempdir().unwrap();
    let fake = helper();
    let c = loaded(dir.path(), fake.clone());
    let mut d = c.defaults();
    d.on_device_suggestions = false;
    c.set_defaults(d);
    let r = c.suggest_on_device(&OnDeviceParams::default()).unwrap();
    assert!(!r.available);
    assert!(r.reason.unwrap().contains("on_device_suggestions"));
    assert!(fake.calls.lock().unwrap().is_empty());
    assert!(c.session().unwrap().plans.iter().all(|p| p.name.is_empty()));

    // A Mac without the model: caps says so, and nothing changes.
    let mut old = FakeHelper::new(Box::new(|_| None));
    old.caps = r#"{"kind":"caps","helper":1,"os":"15.6","features":[{"name":"suggest","available":false,"min_os":"26.0","reason":"On-device suggestions need macOS 26 or later."}]}"#.into();
    let dir = tempfile::tempdir().unwrap();
    let c = loaded(dir.path(), Arc::new(old));
    let r = c.suggest_on_device(&OnDeviceParams::default()).unwrap();
    assert!(!r.available);
    assert!(r.reason.unwrap().contains("macOS 26"));
    assert!(c
        .session()
        .unwrap()
        .plans
        .iter()
        .all(|p| p.ondevice.is_none()));

    // No helper at all (a process started by cargo): the same.
    let dir = tempfile::tempdir().unwrap();
    let c = loaded(dir.path(), helper()).with_intel(Arc::new(Intel::unavailable("no helper")));
    let r = c.suggest_on_device(&OnDeviceParams::default()).unwrap();
    assert_eq!(
        (r.available, r.reason.as_deref()),
        (false, Some("no helper"))
    );
}

#[test]
fn a_helper_that_answers_unavailable_mid_run_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut fake = FakeHelper::new(Box::new(|_| None));
    fake.answer = Some(Box::new(|_| {
        r#"{"kind":"error","code":"unavailable","message":"Apple Intelligence is off."}"#.into()
    }));
    let c = loaded(dir.path(), Arc::new(fake));
    let r = c.suggest_on_device(&OnDeviceParams::default()).unwrap();
    assert!(!r.available);
    assert_eq!(r.reason.as_deref(), Some("Apple Intelligence is off."));
    assert!(c.session().unwrap().plans.iter().all(|p| p.name.is_empty()));
}

#[test]
fn mcp_suggest_runs_the_model_and_marks_it() {
    let dir = tempfile::tempdir().unwrap();
    let c = Arc::new(loaded(dir.path(), helper()));
    let mut srv = Server::new(LocalBackend(c));
    let out = srv.call_tool("quadcam_suggest", json!({"on_device": true}));
    assert_eq!(out["isError"], false, "{out}");
    let text = out["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("On-device model suggested names for 2 of 2 clips"),
        "{text}"
    );
    assert!(
        text.contains("on-device suggestion \"fake outing pack 1\""),
        "{text}"
    );
    assert_eq!(
        out["structuredContent"]["clips"][1]["short_name"],
        "fake outing pack 2"
    );
    let none = srv.call_tool("quadcam_suggest", json!({}));
    assert_eq!(none["isError"], true);
}

#[test]
fn the_session_report_summarizes_the_day_on_request() {
    let dir = tempfile::tempdir().unwrap();
    let logs = dir.path().join("logs");
    synth::write_known(&logs).unwrap();
    let fake = helper();
    // A settings file in the temp folder: the gear folder sits beside it, never the real one.
    let support = dir.path().join("support");
    std::fs::create_dir_all(&support).unwrap();
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(
        support.join("settings.json"),
        json!({"outputDir": out}).to_string(),
    )
    .unwrap();
    let c = Core::new(
        dir.path().join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(support.join("settings.json"))
    .with_gear_env(Env::fake(vec![], Arc::new(FakePorts::new(vec![]))))
    .with_intel(Arc::new(Intel::with("/helper".into(), fake.clone())));
    c.gear_flight_folders(&api::FlightFoldersParams {
        add: Some(logs),
        remove: None,
    })
    .unwrap();
    let plain = c.gear_session_report(&Default::default()).unwrap();
    assert_eq!(plain.summary, None);
    assert!(fake.requests.lock().unwrap().is_empty());

    let r = c
        .gear_session_report(&api::ReportParams {
            day: None,
            summarize: Some(true),
        })
        .unwrap();
    let summary = r.summary.clone().unwrap();
    assert!(summary.starts_with("A fake summary. Day: "), "{summary}");
    assert!(r.markdown.contains(&summary));
    let facts = fake.requests.lock().unwrap()[0]["facts"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(facts.contains("Radio-log flights: 3"), "{facts}");

    // Off: no summary, and why.
    let mut d = c.defaults();
    d.on_device_suggestions = false;
    c.set_defaults(d);
    let r = c
        .gear_session_report(&api::ReportParams {
            day: None,
            summarize: Some(true),
        })
        .unwrap();
    assert_eq!(r.summary, None);
    assert!(r.summary_unavailable.unwrap().contains("off"));
}
