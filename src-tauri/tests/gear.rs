//! Gear foundation: `gear.json` follows the settings-file rules across processes (the CLI,
//! an agent through the app's socket, the app), detection lists a synthetic radio volume
//! and a fake serial port, the three MCP tools answer, and the Gear settings apply. No test
//! here opens a real port or touches a real volume: every core gets `Env::fake`.

use quadcam_lib::control;
use quadcam_lib::core::{Core, Hooks, NoHooks};
use quadcam_lib::core::{HookOutcome, OnConnectHook};
use quadcam_lib::disk::{DiskInfo, Volume};
use quadcam_lib::gear::cues::{CueService, RecordedCues};
use quadcam_lib::gear::events::{DeviceEventKind, Presence, Tracker};
use quadcam_lib::gear::model::{DeviceKind, Link};
use quadcam_lib::gear::serial::{FakePorts, PortInfo};
use quadcam_lib::gear::Automation;
use quadcam_lib::gear::Env;
use quadcam_lib::mcp::{AutoBackend, LocalBackend, Server};
use quadcam_lib::photos::Recorder;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Stands in for the GUI: counts Gear events.
#[derive(Default)]
struct MockGui {
    gear: AtomicUsize,
}

impl Hooks for MockGui {
    fn has_gui(&self) -> bool {
        true
    }
    fn gear_changed(&self) {
        self.gear.fetch_add(1, Ordering::SeqCst);
    }
}

const FC_PORT: &str = "/dev/cu.usbmodemFAKE1";

/// A synthetic EdgeTX card in USB Storage mode, made up for the test.
fn radio_card(root: &Path) -> Volume {
    for d in ["RADIO", "MODELS", "LOGS", "SOUNDS/en"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    std::fs::write(
        root.join("RADIO/radio.yml"),
        "checksum: 0\r\nsemver: 2.12.4\r\nboard: pocket\r\ncurrModel: 0\r\n",
    )
    .unwrap();
    Volume {
        mount: root.to_path_buf(),
        info: DiskInfo {
            volume_uuid: Some("TEST-UUID-RADIO".into()),
            parent_whole_disk: "disk42".into(),
            bus_protocol: Some("USB".into()),
            removable: true,
            ..Default::default()
        },
        is_card: false,
        source: None,
        is_radio: quadcam_lib::disk::looks_like_radio(root),
        warnings: vec![],
    }
}

fn fake_ports() -> Arc<FakePorts> {
    Arc::new(FakePorts::new(vec![PortInfo {
        port: FC_PORT.into(),
        vid: 0x0483,
        pid: 0x5740,
        ..Default::default()
    }]))
}

/// Volumes a test can change while a core runs.
type Mounted = Arc<Mutex<Vec<Volume>>>;

/// What the system shows as still plugged in, which a test can change.
type Present = Arc<Mutex<Vec<Presence>>>;

fn env(mounted: &Mounted, present: &Present, cues: &Arc<RecordedCues>) -> Env {
    let (m, p) = (mounted.clone(), present.clone());
    Env {
        ports: fake_ports(),
        volumes: Arc::new(move || m.lock().unwrap().clone()),
        dfu: Arc::new(Vec::new),
        presence: Arc::new(move || p.lock().unwrap().clone()),
        card_reader: Arc::new(Vec::new),
        usb: Arc::new(Vec::new),
        cues: Arc::new(CueService::inline(cues.clone())),
        unmount: Arc::new(|_| Ok(())),
        mount: Arc::new(|_| Ok(())),
        fail_readback: None,
        card_write: None,
        disk: Arc::new(quadcam_lib::gear::health::FakeDisk::ok()),
        holders: Arc::new(|_| Vec::new()),
        tts: Arc::new(quadcam_lib::gear::voice::tts::NoProviders),
        keys: Arc::new(quadcam_lib::gear::voice::keychain::MemKeys::default()),
    }
}

fn core_with(
    dir: &Path,
    hooks: Arc<dyn Hooks>,
    mounted: &Mounted,
    present: &Present,
    cues: &Arc<RecordedCues>,
) -> Arc<Core> {
    Arc::new(
        Core::new(
            dir.join("cache"),
            None,
            hooks,
            Arc::new(Recorder::default()),
        )
        .with_settings(dir.join("support/settings.json"))
        .with_gear_env(env(mounted, present, cues)),
    )
}

fn core(dir: &Path, hooks: Arc<dyn Hooks>, mounted: &Mounted) -> Arc<Core> {
    core_with(dir, hooks, mounted, &Present::default(), &Arc::default())
}

fn radio_id(status: &Value) -> String {
    status["connected"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "radio")
        .and_then(|c| c["id"].as_str())
        .unwrap()
        .to_string()
}

#[test]
fn detect_lists_a_radio_volume_and_a_fake_port() {
    let dir = tempfile::tempdir().unwrap();
    let mounted: Mounted = Arc::new(Mutex::new(vec![radio_card(&dir.path().join("RADIO"))]));
    let c = core(dir.path(), Arc::new(NoHooks), &mounted);
    let found = c.gear_connected().unwrap();
    assert_eq!(found.len(), 2, "{found:#?}");
    let radio = &found[0];
    assert_eq!(radio.kind, DeviceKind::Radio);
    assert_eq!(radio.identity.board.as_deref(), Some("pocket"));
    assert_eq!(radio.identity.version.as_deref(), Some("2.12.4"));
    assert!(radio.id.as_deref().unwrap().starts_with("radio-"));
    assert!(
        !radio.id.as_deref().unwrap().contains("TEST-UUID"),
        "the id hashes the UUID"
    );
    assert_eq!(found[1].kind, DeviceKind::Fc);
    assert!(matches!(&found[1].link, Link::Serial { port, .. } if port == FC_PORT));
    // The same answer through dispatch, as the socket and the GUI get it.
    let s = c.dispatch("gear_status", Value::Null).unwrap();
    assert_eq!(s["connected"].as_array().unwrap().len(), 2);
    assert_eq!(
        PathBuf::from(s["gear_dir"].as_str().unwrap()),
        dir.path().join("support/gear"),
        "the default gear folder sits next to the settings file"
    );
}

#[test]
fn gear_json_writes_from_the_cli_and_an_agent_are_kept_and_seen() {
    let dir = tempfile::tempdir().unwrap();
    let mounted: Mounted = Arc::new(Mutex::new(vec![radio_card(&dir.path().join("RADIO"))]));
    let gui = Arc::new(MockGui::default());
    let app = core(dir.path(), gui.clone(), &mounted);
    let sock = dir.path().join("control.sock");
    control::serve(app.clone(), &sock).unwrap();
    let id = radio_id(&app.dispatch("gear_status", Value::Null).unwrap());

    // A key from a later version, which every write keeps.
    let store = app.gear_store();
    std::fs::create_dir_all(store.root()).unwrap();
    std::fs::write(store.gear_file(), r#"{"packs":[{"label":"1"}]}"#).unwrap();
    let stamp = store.stamp();

    // The CLI names the radio: a headless core on the same files.
    let cli = core(dir.path(), Arc::new(NoHooks), &mounted);
    cli.profile_save(
        "Whoop",
        &serde_json::from_value(json!({"aircraft": "65 mm whoop"})).unwrap(),
        None,
    )
    .unwrap();
    let d = cli
        .dispatch(
            "gear_device_save",
            json!({"id": id, "name": "Bench radio", "aircraft": "whoop"}),
        )
        .unwrap();
    assert_eq!(d["name"], "Bench radio");
    assert_eq!(d["aircraft"], "Whoop", "the profile's own spelling");
    assert_eq!(d["identity"]["board"], "pocket");
    assert_ne!(store.stamp(), stamp, "the app's poll sees the CLI's write");

    // The app reads it at once: nothing is cached.
    let devs = app.gear_devices().unwrap();
    assert_eq!(devs.len(), 1);
    assert_eq!(devs[0].name, "Bench radio");
    let s = app.gear_status().unwrap();
    assert_eq!(s.connected[0].device.as_ref().unwrap().name, "Bench radio");

    // An agent, through the app's socket, links nothing and renames.
    let mut mcp = Server::new(AutoBackend::new(sock.clone(), None));
    let r = mcp.call_tool(
        "quadcam_gear_edit",
        json!({"action": "device_save", "id": id, "name": "Radio", "aircraft": ""}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert!(gui.gear.load(Ordering::SeqCst) >= 1, "the app heard it");

    let v: Value = serde_json::from_slice(&std::fs::read(store.gear_file()).unwrap()).unwrap();
    assert_eq!(v["packs"][0]["label"], "1", "the unknown key survived");
    assert_eq!(v["devices"][0]["name"], "Radio");
    assert!(v["devices"][0]["aircraft"].is_null());

    // Refusals: an id nothing knows, an aircraft with no profile.
    let e = cli
        .dispatch("gear_device_save", json!({"id": "radio-0000"}))
        .unwrap_err();
    assert!(format!("{e:#}").starts_with("No device"), "{e:#}");
    let e = cli
        .dispatch("gear_device_save", json!({"id": id, "aircraft": "Nope"}))
        .unwrap_err();
    assert!(format!("{e:#}").contains("No aircraft profile"), "{e:#}");

    // Forget: the device goes, the other keys stay.
    let gone = cli
        .dispatch("gear_device_forget", json!({"id": id}))
        .unwrap();
    assert_eq!(gone["name"], "Radio");
    assert!(app.gear_devices().unwrap().is_empty());
    let v: Value = serde_json::from_slice(&std::fs::read(store.gear_file()).unwrap()).unwrap();
    assert_eq!(v["packs"][0]["label"], "1");
}

#[test]
fn the_poll_reports_plug_ins_and_marks_known_devices_seen() {
    let dir = tempfile::tempdir().unwrap();
    let card = radio_card(&dir.path().join("RADIO"));
    let mounted: Mounted = Arc::new(Mutex::new(vec![]));
    let c = core(dir.path(), Arc::new(NoHooks), &mounted);
    let mut t = Tracker::default();
    let kinds = |e: &[quadcam_lib::gear::events::DeviceEvent]| -> Vec<DeviceEventKind> {
        e.iter().map(|e| e.kind).collect()
    };

    assert_eq!(
        kinds(&c.gear_poll(&mut t).unwrap()),
        [DeviceEventKind::Connected],
        "the fake port"
    );
    assert!(c.gear_poll(&mut t).unwrap().is_empty(), "nothing changed");

    // The radio is plugged in, saved, unplugged, plugged in again.
    mounted.lock().unwrap().push(card.clone());
    let e = c.gear_poll(&mut t).unwrap();
    assert_eq!(kinds(&e), [DeviceEventKind::Connected]);
    let id = e[0].device.id.clone().unwrap();
    assert!(
        c.gear_devices().unwrap().is_empty(),
        "the poll saves no new device"
    );
    c.gear_device_save(&quadcam_lib::core::DeviceSaveParams {
        id: id.clone(),
        name: Some("Bench radio".into()),
        aircraft: None,
    })
    .unwrap();
    let saved_seen = c.gear_devices().unwrap()[0].last_seen;
    mounted.lock().unwrap().clear();
    assert_eq!(
        kinds(&c.gear_poll(&mut t).unwrap()),
        [DeviceEventKind::Removed]
    );
    std::thread::sleep(std::time::Duration::from_millis(10));
    mounted.lock().unwrap().push(card);
    let e = c.gear_poll(&mut t).unwrap();
    let dev = e[0].device.device.as_ref().unwrap();
    assert_eq!(dev.name, "Bench radio");
    assert!(dev.last_seen > saved_seen, "seen again on plug-in");
}

#[test]
fn cues_mark_the_end_of_a_job_and_never_quadcams_own_unmounts() {
    use std::time::{Duration, Instant};
    let dir = tempfile::tempdir().unwrap();
    let card = radio_card(&dir.path().join("RADIO"));
    let mounted: Mounted = Arc::new(Mutex::new(vec![card.clone()]));
    let present: Present = Arc::new(Mutex::new(vec![Presence::Disk {
        disk: "disk42".into(),
    }]));
    let cues = Arc::new(RecordedCues::default());
    let c = core_with(dir.path(), Arc::new(NoHooks), &mounted, &present, &cues);
    c.settings_set(
        &serde_json::from_value(json!({"gear_cues": {"reminder_grace_s": 60, "still_inserted_every_s": 300, "reminder_max": 2}}))
            .unwrap(),
    )
    .unwrap();
    let mut t = Tracker::default();
    let e = c.gear_poll(&mut t).unwrap();
    assert!(!e[0].app_initiated, "the person plugged the card in");
    let radio = e[0].device.clone();
    assert_eq!(quadcam_lib::core::link_handle(&radio.link), "disk42");

    // A job: QuadCam unmounts and mounts the card while it holds it. Those events are its
    // own and play nothing.
    {
        let _job = c.gear_hold("disk42");
        assert_eq!(
            c.gear_status().unwrap().working,
            ["disk42"],
            "the job shows as working"
        );
        mounted.lock().unwrap().clear();
        let e = c.gear_poll(&mut t).unwrap();
        assert_eq!(e[0].kind, DeviceEventKind::UnmountedPresent);
        assert!(e[0].app_initiated);
        mounted.lock().unwrap().push(card.clone());
        let e = c.gear_poll(&mut t).unwrap();
        assert!(e[0].app_initiated);
        mounted.lock().unwrap().clear();
        c.gear_poll(&mut t).unwrap();
    }
    assert!(
        cues.played.lock().unwrap().is_empty(),
        "no cue during a job"
    );
    assert!(c.gear_status().unwrap().working.is_empty(), "released");

    // The job's end: one cue. The same again within the debounce: none.
    assert!(c.gear_job_done(&radio, None));
    assert!(!c.gear_job_done(&radio, None));
    assert_eq!(cues.spoken(), ["The Radio done, safe to unplug."]);

    // The reminder: after the grace, then every interval, at most twice.
    let now = Instant::now();
    c.gear_play_reminders(&t, now + Duration::from_secs(30), chrono::NaiveTime::MIN);
    assert_eq!(cues.spoken().len(), 1, "the grace");
    c.gear_play_reminders(&t, now + Duration::from_secs(61), chrono::NaiveTime::MIN);
    assert_eq!(cues.spoken()[1], "The Radio is still inserted.");
    c.gear_play_reminders(&t, now + Duration::from_secs(400), chrono::NaiveTime::MIN);
    c.gear_play_reminders(&t, now + Duration::from_secs(5000), chrono::NaiveTime::MIN);
    assert_eq!(cues.spoken().len(), 3, "capped at two reminders");

    // Dismissed, or pulled: no more.
    assert!(c.gear_job_done(&renamed(&radio), None));
    assert_eq!(c.gear_status().unwrap().reminders, ["disk42"]);
    c.gear_dismiss_reminder(&radio);
    assert!(c.gear_status().unwrap().reminders.is_empty());
    assert!(!c.gear_dismiss("disk42"), "nothing left to dismiss");
    c.gear_play_reminders(&t, now + Duration::from_secs(9000), chrono::NaiveTime::MIN);
    assert_eq!(cues.spoken().len(), 4, "only the second done");
    present.lock().unwrap().clear();
    let e = c.gear_poll(&mut t).unwrap();
    assert_eq!(e[0].kind, DeviceEventKind::Removed);
    c.gear_play_reminders(&t, now + Duration::from_secs(20000), chrono::NaiveTime::MIN);
    assert_eq!(cues.spoken().len(), 4);

    // A batch: one cue for all of it; mute silences everything.
    c.gear_batch_done(&["A".into(), "B".into()], &[]);
    assert_eq!(cues.spoken()[4], "2 devices done, safe to unplug.");
    c.settings_set(&serde_json::from_value(json!({"gear_cues": {"mute": true}})).unwrap())
        .unwrap();
    assert!(!c.gear_batch_done(&[], &[("Backup".into(), "C".into())]));
}

/// The same device under another name, so the debounce (per cue and device) lets it play.
fn renamed(c: &quadcam_lib::gear::model::Connected) -> quadcam_lib::gear::model::Connected {
    let mut d = c.clone();
    d.device = Some(quadcam_lib::gear::model::Device {
        id: "radio-x".into(),
        kind: DeviceKind::Radio,
        name: "Bench radio".into(),
        aircraft: None,
        identity: Default::default(),
        last_seen: None,
        last_backup: None,
        last_space: None,
        aliases: Vec::new(),
        dfu_serial: None,
    });
    d
}

#[test]
fn on_connect_hooks_run_only_when_their_automation_is_on() {
    let dir = tempfile::tempdir().unwrap();
    let mounted: Mounted = Arc::new(Mutex::new(vec![radio_card(&dir.path().join("RADIO"))]));
    let cues = Arc::new(RecordedCues::default());
    let c = core_with(
        dir.path(),
        Arc::new(NoHooks),
        &mounted,
        &Present::default(),
        &cues,
    );
    let ran = Arc::new(AtomicUsize::new(0));
    let r2 = ran.clone();
    c.gear_add_hook(OnConnectHook {
        name: "test backup",
        automation: Automation::Backup,
        kinds: vec![DeviceKind::Radio],
        run: Arc::new(move |_, _| {
            r2.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }),
    });
    c.gear_add_hook(OnConnectHook {
        name: "test import",
        automation: Automation::Import,
        kinds: vec![DeviceKind::Radio],
        run: Arc::new(|_, _| anyhow::bail!("the card is busy")),
    });
    c.gear_add_hook(OnConnectHook {
        name: "fc only",
        automation: Automation::Backup,
        kinds: vec![DeviceKind::Fc],
        run: Arc::new(|_, _| Ok(())),
    });
    let radio = c.gear_connected().unwrap().remove(0);

    let runs = c.gear_on_connect(&radio);
    assert_eq!(runs.len(), 2, "only the radio's hooks");
    assert_eq!(runs[0].outcome, HookOutcome::Ran, "backup is on by default");
    assert_eq!(
        runs[1].outcome,
        HookOutcome::Off,
        "import is off by default"
    );
    assert_eq!(ran.load(Ordering::SeqCst), 1);
    assert_eq!(
        cues.spoken(),
        ["The Radio done, safe to unplug."],
        "one cue for the run"
    );

    c.settings_set(
        &serde_json::from_value(
            json!({"gear_auto_backup": false, "gear_on_connect": {"radio": ["backup", "import"]}}),
        )
        .unwrap(),
    )
    .unwrap();
    let runs = c.gear_on_connect(&radio);
    assert_eq!(
        runs[0].outcome,
        HookOutcome::Off,
        "the master switch is off"
    );
    assert!(
        matches!(&runs[1].outcome, HookOutcome::Failed { message } if message.contains("busy"))
    );
    assert_eq!(cues.spoken()[1], "test import failed on The Radio.");
    assert!(c
        .settings_set(
            &serde_json::from_value(json!({"gear_on_connect": {"toaster": ["backup"]}})).unwrap()
        )
        .is_err());
}

#[test]
fn the_three_mcp_tools() {
    let dir = tempfile::tempdir().unwrap();
    let mounted: Mounted = Arc::new(Mutex::new(vec![radio_card(&dir.path().join("RADIO"))]));
    let c = core(dir.path(), Arc::new(NoHooks), &mounted);
    let mut s = Server::new(LocalBackend(c));

    let names: Vec<String> = quadcam_lib::mcp::tools()
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    for t in ["quadcam_gear", "quadcam_gear_edit", "quadcam_gear_apply"] {
        assert!(names.contains(&t.to_string()), "{t}");
    }

    let r = s.call_tool("quadcam_gear", json!({"action": "status"}));
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("radio at"), "{text}");
    assert!(text.contains(FC_PORT), "{text}");
    let id = radio_id(&r["structuredContent"]);

    let r = s.call_tool(
        "quadcam_gear_edit",
        json!({"action": "device_save", "id": id, "name": "Bench radio"}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let r = s.call_tool("quadcam_gear", json!({"action": "devices"}));
    assert_eq!(r["structuredContent"]["devices"][0]["name"], "Bench radio");
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("EdgeTX 2.12.4 pocket"));

    let r = s.call_tool("quadcam_gear_edit", json!({"action": "device_save"}));
    assert_eq!(r["isError"], true, "device_save needs an id");
    let r = s.call_tool(
        "quadcam_gear_apply",
        json!({"action": "reflash_everything", "digest": "x", "confirm": true}),
    );
    assert_eq!(r["isError"], true);
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with("Refused"));
    // The firmware flash needs its digest and confirm before anything else.
    let r = s.call_tool(
        "quadcam_gear_apply",
        json!({"action": "flash", "device": "radio-1", "digest": "x"}),
    );
    assert_eq!(r["isError"], true);
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with("Refused: flash writes"));
}

#[test]
fn gear_settings_apply_and_the_voice_key_is_never_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let mounted: Mounted = Arc::new(Mutex::new(vec![]));
    let c = core(dir.path(), Arc::new(NoHooks), &mounted);
    let elsewhere = dir.path().join("elsewhere/gear");
    let v = c
        .settings_set(
            &serde_json::from_value(json!({
                "gear_dir": elsewhere, "gear_auto_backup": false, "gear_keep_recent": 3,
                "firmware_check": "daily", "tts_key": "not-a-real-key"
            }))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(v.values["ttsKey"], "(set)");
    let s = c.gear_status().unwrap();
    assert_eq!(s.gear_dir, elsewhere);
    assert!(!s.settings.auto_backup);
    assert_eq!(s.settings.keep_recent, 3);
    assert_eq!(s.settings.firmware_check, "daily");
    assert!(!serde_json::to_string(&s)
        .unwrap()
        .contains("not-a-real-key"));
    assert!(c
        .settings_set(&serde_json::from_value(json!({"firmware_check": "hourly"})).unwrap())
        .is_err());
    assert!(c
        .settings_set(&serde_json::from_value(json!({"gear_dir": "relative/path"})).unwrap())
        .is_err());
}

#[test]
fn profiles_from_before_gear_write_back_without_a_gear_key() {
    let dir = tempfile::tempdir().unwrap();
    let mounted: Mounted = Arc::new(Mutex::new(vec![]));
    let c = core(dir.path(), Arc::new(NoHooks), &mounted);
    let f = dir.path().join("support/settings.json");
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::write(
        &f,
        r#"{"profiles":[{"name":"Whoop A","aircraft":"65 mm whoop","camera_make":"","camera_model":"","video_system":"Analog","keywords":[],"author":"","place":null,"edgetx_models":[]}]}"#,
    )
    .unwrap();
    c.profile_save(
        "Whoop A",
        &serde_json::from_value(json!({"author": "me"})).unwrap(),
        None,
    )
    .unwrap();
    let v: Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
    assert!(v["profiles"][0].get("gear").is_none(), "{v}");
    // A profile with links keeps them.
    let p = c
        .profile_save(
            "Whoop A",
            &serde_json::from_value(
                json!({"gear": {"radio": "radio-1", "edgetx_model": "model01.yml"}}),
            )
            .unwrap(),
            None,
        )
        .unwrap();
    assert_eq!(p.gear.radio.as_deref(), Some("radio-1"));
    let v: Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
    assert_eq!(v["profiles"][0]["gear"]["edgetx_model"], "model01.yml");
}

/// The fail-safe: a core built the normal way in a cargo process sees no serial port.
#[test]
fn a_default_core_has_no_serial_ports_under_cargo() {
    if std::env::var("QUADCAM_SERIAL").as_deref() == Ok("real") {
        return;
    }
    assert!(quadcam_lib::gear::serial::real_ports().is_empty());
    let dir = tempfile::tempdir().unwrap();
    let c = Core::new(
        dir.path().join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(dir.path().join("settings.json"));
    let ports = c
        .gear_connected()
        .unwrap()
        .into_iter()
        .filter(|c| matches!(c.link, Link::Serial { .. }))
        .count();
    assert_eq!(ports, 0);
}
