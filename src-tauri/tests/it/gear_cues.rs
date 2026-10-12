//! Cues in a voice pack's voice (the default; an explicit `macos` wins). One cue never
//! mixes voices: (a) the device's own name clip then the phrase clip, else (b) the kind's
//! whole sentence, else (c) the macOS voice for the whole cue. The pack is the one Settings
//! picks, else the pack chosen for the radio seen last. Naming a device adds its name to
//! the person's own lines. Mute, the channels and quiet hours still decide.
//! `RecordedCues` records every cue: nothing plays.

use quadcam_lib::core::{Core, NoHooks};
use quadcam_lib::gear::cues::{Cue, CueService, Played, RecordedCues};
use quadcam_lib::gear::model::Connected;
use quadcam_lib::gear::serial::FakePorts;
use quadcam_lib::gear::voice::packs::PackManifest;
use quadcam_lib::gear::Env;
use quadcam_lib::photos::Recorder;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct Bench {
    core: Core,
    cues: Arc<RecordedCues>,
    settings: PathBuf,
    voices: PathBuf,
    _dir: tempfile::TempDir,
}

fn bench(cues_setting: serde_json::Value) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let cues = Arc::new(RecordedCues::default());
    let mut env = Env::fake(vec![], Arc::new(FakePorts::new(vec![])));
    env.cues = Arc::new(CueService::inline(cues.clone()));
    std::fs::create_dir_all(dir.path().join("support")).unwrap();
    let settings = dir.path().join("support/settings.json");
    let gear = dir.path().join("gear");
    std::fs::write(
        &settings,
        json!({"gearDir": gear, "gearCues": cues_setting}).to_string(),
    )
    .unwrap();
    let core = Core::new(
        dir.path().join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(settings.clone())
    .with_gear_env(env);
    let voices = core.gear_store().voices_dir();
    Bench {
        core,
        cues,
        settings,
        voices,
        _dir: dir,
    }
}

/// Installs a pack folder holding `paths` (card paths) as tiny WAV stand-ins.
fn pack(voices: &Path, id: &str, paths: &[&str]) -> PathBuf {
    let dir = voices.join(id);
    for p in paths {
        let f = dir.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, b"RIFF").unwrap();
    }
    let m = PackManifest {
        id: id.into(),
        voice: id.into(),
        lang: "en".into(),
        provider: "say".into(),
        model: String::new(),
        settings: Default::default(),
        lines: paths.len() as u32,
        license: "CC0-1.0".into(),
        attribution: String::new(),
        lines_csv_sha: String::new(),
        version: "1".into(),
        files: paths.iter().map(|p| p.to_string()).collect(),
        retakes: vec![],
        kept_batches: vec![],
        firmware: "edgetx".into(),
        voice_id: String::new(),
        heard: vec![],
    };
    std::fs::write(dir.join("pack.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    dir
}

/// Two saved radios, each with its chosen pack; `seen` gives each one's last-seen time.
fn radios(b: &Bench, chosen: &[(&str, &str, &str)]) {
    b.core
        .gear_store()
        .update(|v| {
            let devices: Vec<_> = chosen
                .iter()
                .map(|(id, _, seen)| json!({"id": id, "kind": "radio", "last_seen": seen}))
                .collect();
            v.insert("devices".into(), json!(devices));
            let picks: serde_json::Map<_, _> = chosen
                .iter()
                .map(|(id, pack, _)| (id.to_string(), json!(pack)))
                .collect();
            v.insert("voice".into(), json!({ "chosen": picks }));
            Ok(())
        })
        .unwrap();
}

fn card() -> Connected {
    named_card(None)
}

/// A DVR card, saved under `name` when one is given.
fn named_card(name: Option<&str>) -> Connected {
    let device = name.map(|n| json!({"id": "card-1", "kind": "dvr_card", "name": n}));
    serde_json::from_value(json!({
        "id": "card-1", "kind": "dvr_card",
        "link": {"kind": "volume", "mount": "/tmp/none", "volume_uuid": null, "bus_protocol": "USB", "whole_disk": "disk9"},
        "identity": {}, "device": device, "usb": null, "also": []
    }))
    .unwrap()
}

fn custom_lines(b: &Bench) -> Vec<serde_json::Value> {
    let v = b.core.gear_store().read().unwrap();
    v.get("voice")
        .and_then(|v| v.get("custom_lines"))
        .and_then(|l| l.as_array().cloned())
        .unwrap_or_default()
}

fn spoken(b: &Bench) -> Vec<Played> {
    b.cues
        .played
        .lock()
        .unwrap()
        .iter()
        .filter(|p| matches!(p, Played::Speak(_) | Played::Clips(_)))
        .cloned()
        .collect()
}

const DONE: &str = "SOUNDS/en/qcdone.wav";
const FAIL: &str = "SOUNDS/en/qcfail.wav";
/// "DVR card done, safe to unplug.", the kind's whole sentence.
const DVR_DONE: &str = "SOUNDS/en/qddvr.wav";

#[test]
fn the_radios_pack_speaks_the_cue_and_a_missing_line_falls_back_to_macos() {
    let b = bench(json!({"voice_source": "voice_pack", "debounce_s": 0, "notification": false}));
    // The pack holds the DVR card's "done" sentence, and no "a step failed" for it.
    let older = pack(&b.voices, "en-old-v1", &[DONE, FAIL, DVR_DONE]);
    let newer = pack(&b.voices, "en-new-v1", &[DONE, FAIL, DVR_DONE]);
    std::fs::remove_file(newer.join(FAIL)).unwrap();
    radios(
        &b,
        &[
            ("radio-a", "en-old-v1", "2026-10-01T10:00:00Z"),
            ("radio-b", "en-new-v1", "2026-10-11T10:00:00Z"),
        ],
    );
    // The radio seen last picks the voice.
    b.core.gear_job_done(&card(), None);
    assert_eq!(spoken(&b), [Played::Clips(vec![newer.join(DVR_DONE)])]);

    // No kind sentence for the failure: macOS says the whole cue, never a mix.
    b.cues.played.lock().unwrap().clear();
    b.core.gear_job_done(&card(), Some("Backup"));
    let got = spoken(&b);
    assert!(
        matches!(got.as_slice(), [Played::Speak(s)] if s.starts_with("Backup failed on")),
        "{got:?}"
    );
    let _ = older;
}

#[test]
fn the_pack_is_the_default_a_pick_wins_and_an_explicit_macos_wins() {
    let b = bench(json!({"debounce_s": 0, "notification": false}));
    let picked = pack(&b.voices, "en-picked-v1", &[DONE, DVR_DONE]);
    let radio = pack(&b.voices, "en-radio-v1", &[DONE, DVR_DONE]);

    // No radio has a pack and none is picked: macOS.
    b.core.gear_job_done(&card(), None);
    assert!(matches!(spoken(&b).as_slice(), [Played::Speak(_)]));

    // voice_source unset: the radio's pack.
    radios(&b, &[("radio-a", "en-radio-v1", "2026-10-11T10:00:00Z")]);
    b.cues.played.lock().unwrap().clear();
    b.core.gear_job_done(&card(), None);
    assert_eq!(spoken(&b), [Played::Clips(vec![radio.join(DVR_DONE)])]);

    // An explicit macos wins over every pack.
    let mut s: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&b.settings).unwrap()).unwrap();
    s["gearCues"]["voice_source"] = json!("macos");
    std::fs::write(&b.settings, s.to_string()).unwrap();
    b.cues.played.lock().unwrap().clear();
    b.core.gear_job_done(&card(), None);
    assert!(matches!(spoken(&b).as_slice(), [Played::Speak(_)]));

    // A pick in Settings beats the radio's pack.
    s["gearCues"]["voice_source"] = json!("voice_pack");
    s["gearCues"]["voice_pack"] = json!("en-picked-v1");
    std::fs::write(&b.settings, s.to_string()).unwrap();
    b.cues.played.lock().unwrap().clear();
    b.core.gear_job_done(&card(), None);
    assert_eq!(spoken(&b), [Played::Clips(vec![picked.join(DVR_DONE)])]);

    // A picked pack that is gone speaks with macOS, not another pack.
    s["gearCues"]["voice_pack"] = json!("en-deleted-v1");
    std::fs::write(&b.settings, s.to_string()).unwrap();
    b.cues.played.lock().unwrap().clear();
    b.core.gear_job_done(&card(), None);
    assert!(matches!(spoken(&b).as_slice(), [Played::Speak(_)]));
}

#[test]
fn mute_channels_and_quiet_hours_still_decide() {
    let b = bench(json!({"voice_source": "voice_pack", "debounce_s": 0, "mute": true}));
    pack(&b.voices, "en-a-v1", &[DONE, DVR_DONE]);
    radios(&b, &[("radio-a", "en-a-v1", "2026-10-11T10:00:00Z")]);
    b.core.gear_job_done(&card(), None);
    assert!(b.cues.played.lock().unwrap().is_empty(), "muted");

    // Speech off: no clip, the notification still posts.
    let mut s: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&b.settings).unwrap()).unwrap();
    s["gearCues"]["mute"] = json!(false);
    s["gearCues"]["speech"] = json!(false);
    std::fs::write(&b.settings, s.to_string()).unwrap();
    b.core.gear_job_done(&card(), None);
    let played = b.cues.played.lock().unwrap().clone();
    assert!(spoken(&b).is_empty(), "{played:?}");
    assert!(played.iter().any(|p| matches!(p, Played::Notify(..))));

    // Quiet all day: no speech or clip.
    s["gearCues"]["speech"] = json!(true);
    s["gearCues"]["quiet_hours"] = json!({"start": "00:00", "end": "23:59"});
    std::fs::write(&b.settings, s.to_string()).unwrap();
    b.cues.played.lock().unwrap().clear();
    b.core.gear_job_done(&card(), None);
    let late = chrono::Local::now().format("%H:%M").to_string() == "23:59";
    if !late {
        assert!(spoken(&b).is_empty());
    }
}

#[test]
fn a_named_device_plays_its_name_then_the_phrase_else_its_kind_else_macos() {
    let b = bench(json!({"debounce_s": 0, "notification": false}));
    radios(&b, &[("radio-a", "en-mine-v1", "2026-10-11T10:00:00Z")]);
    // Naming the device adds its name to the person's own lines, once.
    b.core
        .gear_store()
        .save_device(&serde_json::from_value(json!({"id": "card-1", "kind": "dvr_card"})).unwrap())
        .unwrap();
    let save = |n: &str| {
        b.core
            .gear_device_save(&quadcam_lib::core::DeviceSaveParams {
                id: "card-1".into(),
                name: Some(n.into()),
                ..Default::default()
            })
            .unwrap()
    };
    save("Whoop DVR");
    save("Whoop DVR");
    let lines = custom_lines(&b);
    let mine: Vec<_> = lines.iter().filter(|l| l["text"] == "Whoop DVR").collect();
    assert_eq!(mine.len(), 1, "{lines:?}");
    let name_path = mine[0]["path"].as_str().unwrap().to_string();
    assert!(name_path.starts_with("SOUNDS/en/nm"), "{name_path}");

    // (a) The pack holds the name and the phrase: the name, then the phrase.
    let dir = pack(&b.voices, "en-mine-v1", &[&name_path, DONE, DVR_DONE]);
    let c = named_card(Some("Whoop DVR"));
    b.core.gear_job_done(&c, None);
    assert_eq!(
        spoken(&b),
        [Played::Clips(vec![dir.join(&name_path), dir.join(DONE)])]
    );

    // (b) No name clip: the kind's whole sentence.
    std::fs::remove_file(dir.join(&name_path)).unwrap();
    b.cues.played.lock().unwrap().clear();
    b.core.gear_job_done(&c, None);
    assert_eq!(spoken(&b), [Played::Clips(vec![dir.join(DVR_DONE)])]);

    // (c) Neither: macOS says the whole cue, with the name.
    std::fs::remove_file(dir.join(DVR_DONE)).unwrap();
    b.cues.played.lock().unwrap().clear();
    b.core.gear_job_done(&c, None);
    assert_eq!(
        spoken(&b),
        [Played::Speak("Whoop DVR done, safe to unplug.".into())]
    );

    // A rename adds the new name and leaves the old line.
    save("Race DVR");
    let texts: Vec<_> = custom_lines(&b).iter().map(|l| l["text"].clone()).collect();
    assert!(texts.contains(&json!("Whoop DVR")) && texts.contains(&json!("Race DVR")));
}

#[test]
fn every_kind_sentence_has_a_line_in_the_quadcam_set() {
    use quadcam_lib::gear::cues::KindWord;
    let lines = quadcam_lib::gear::voice::sets::lines_of("quadcam").unwrap();
    let mut n = 0;
    for kind in KindWord::ALL {
        for cue in Cue::ALL {
            let Some((path, text)) = kind.line(cue) else {
                continue;
            };
            n += 1;
            let l = lines
                .iter()
                .find(|l| l.line.path == path)
                .unwrap_or_else(|| panic!("{kind:?} {cue:?}: no line {path}"));
            assert_eq!(l.line.text, text, "{kind:?} {cue:?}");
            assert_eq!(l.line.group, "cues");
        }
    }
    // 6 done, 4 still inserted (cards and the air unit), 6 failed, 1 unplug (quad).
    assert_eq!(n, 17);
}

#[test]
fn every_cue_has_a_line_in_the_quadcam_set() {
    let lines = quadcam_lib::gear::voice::sets::lines_of("quadcam").unwrap();
    for cue in Cue::ALL {
        let (path, text) = cue.pack_line();
        let l = lines
            .iter()
            .find(|l| l.line.path == path)
            .unwrap_or_else(|| panic!("{cue:?}: no line {path} in the quadcam set"));
        assert_eq!(l.line.text, text, "{cue:?}");
        assert_eq!(l.line.group, "cues", "{cue:?}");
    }
}
