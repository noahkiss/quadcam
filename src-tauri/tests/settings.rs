//! Settings, places and profiles: the 0.3.0 file carries over, and writes from the CLI and
//! an agent while the app runs are neither lost nor undone by the app.

use quadcam_lib::control::{self, Client};
use quadcam_lib::core::{Core, Hooks, NoHooks};
use quadcam_lib::mcp::{AutoBackend, Server};
use quadcam_lib::photos::Recorder;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Stands in for the GUI: counts settings events.
#[derive(Default)]
struct MockGui {
    settings: AtomicUsize,
}

impl Hooks for MockGui {
    fn has_gui(&self) -> bool {
        true
    }
    fn settings_changed(&self) {
        self.settings.fetch_add(1, Ordering::SeqCst);
    }
}

const V030: &str = r#"{"profiles":[{"name":"Whoop A","aircraft":"65 mm whoop","camera_make":"Maker","camera_model":"Goggle","video_system":"Analog","keywords":["fpv"],"author":"","place":null,"edgetx_models":["WHOOP A"]}],"defaultProfile":"Whoop A","places":[],"outputDir":"/tmp/x","encoder":"videotoolbox","photosAlbum":"Drone","defaultName":"flight","tunables":{"max_log_age_days":60,"segment_gap_s":5,"session_gap_min":20,"tolerance_s":30},"formatLabel":"ECHO"}"#;

fn read(f: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(f).unwrap()).unwrap()
}

fn core(dir: &Path, hooks: Arc<dyn Hooks>, settings: &Path) -> Arc<Core> {
    Arc::new(
        Core::new(
            dir.join("cache"),
            None,
            hooks,
            Arc::new(Recorder::default()),
        )
        .with_settings(settings.to_path_buf()),
    )
}

#[test]
fn places_and_profiles_crud() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("settings.json");
    std::fs::write(&f, V030).unwrap();
    let c = core(dir.path(), Arc::new(NoHooks), &f);

    let p = c
        .place_save("Statue of Liberty", Some(40.6892), Some(-74.0445), None)
        .unwrap();
    assert_eq!(p.name, "Statue of Liberty");
    assert!(
        c.place_save("statue of liberty", None, None, None).is_ok(),
        "an update may change nothing"
    );
    let e = c.place_save("Nowhere", None, None, None).unwrap_err();
    assert!(format!("{e:#}").contains("needs lat and lon"), "{e:#}");
    let e = c
        .place_save("Bad", Some(91.0), Some(0.0), None)
        .unwrap_err();
    assert!(format!("{e:#}").contains("latitude"), "{e:#}");

    // A profile that uses the place follows a rename and loses it on delete.
    let fields = serde_json::from_value(
        json!({"place": "statue of liberty", "keywords": ["a", "A", " b "]}),
    )
    .unwrap();
    let prof = c.profile_save("Whoop A", &fields, None).unwrap();
    assert_eq!(prof.place.as_deref(), Some("Statue of Liberty"));
    assert_eq!(prof.keywords, vec!["a", "b"]);
    assert_eq!(prof.camera_make, "Maker", "fields not given stay");
    c.place_save("Statue of Liberty", None, None, Some("Liberty Island"))
        .unwrap();
    assert_eq!(
        c.profiles().unwrap().0[0].place.as_deref(),
        Some("Liberty Island")
    );
    assert_eq!(
        c.defaults().places[0].name,
        "Liberty Island",
        "defaults reload after a write"
    );
    let gone = c.place_delete("liberty island").unwrap();
    assert_eq!(gone.profiles_cleared, vec!["Whoop A"]);
    assert_eq!(c.profiles().unwrap().0[0].place, None);

    let e = c
        .profile_save(
            "Whoop A",
            &serde_json::from_value(json!({"place": "Mars"})).unwrap(),
            None,
        )
        .unwrap_err();
    assert!(
        format!("{e:#}").contains("No saved place \"Mars\""),
        "{e:#}"
    );
    let e = c
        .profile_save(
            "Whoop A",
            &serde_json::from_value(json!({"colour": "red"})).unwrap(),
            None,
        )
        .unwrap_err();
    assert!(format!("{e:#}").contains("unknown profile field"), "{e:#}");

    // Rename the default profile: the default follows. Delete it: the next one takes over.
    c.profile_save(
        "Five",
        &serde_json::from_value(json!({"aircraft": "5-inch"})).unwrap(),
        None,
    )
    .unwrap();
    c.profile_save("Whoop A", &Default::default(), Some("Whoop B"))
        .unwrap();
    assert_eq!(c.profiles().unwrap().1.as_deref(), Some("Whoop B"));
    assert!(
        c.profile_save("Five", &Default::default(), Some("whoop b"))
            .is_err(),
        "names stay unique"
    );
    c.profile_delete("Whoop B").unwrap();
    assert_eq!(c.profiles().unwrap().1.as_deref(), Some("Five"));
    assert_eq!(c.profile_default("").unwrap(), None);
    assert!(c.profile_default("Ghost").is_err());

    // Every 0.3.0 key is still in the file.
    let v = read(&f);
    for k in [
        "outputDir",
        "encoder",
        "photosAlbum",
        "defaultName",
        "tunables",
        "formatLabel",
    ] {
        assert_eq!(v[k], serde_json::from_str::<Value>(V030).unwrap()[k], "{k}");
    }
}

/// The app runs (a core with GUI hooks on a control socket). The CLI writes the file
/// directly, an agent writes through the socket, and the app writes its own change. Every
/// change survives, and the app's defaults follow each one.
#[test]
fn writes_while_the_app_runs_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("settings.json");
    std::fs::write(&f, V030).unwrap();
    let gui = Arc::new(MockGui::default());
    let app = core(dir.path(), gui.clone(), &f);
    let sock = dir.path().join("control.sock");
    control::serve(app.clone(), &sock).unwrap();

    // The CLI: a headless core on the same file, no socket.
    let cli = core(dir.path(), Arc::new(NoHooks), &f);
    cli.place_save("Golden Gate Bridge", Some(37.8197), Some(-122.4786), None)
        .unwrap();

    // An agent through the MCP server, which finds the app on the socket.
    let mut mcp = Server::new(AutoBackend::new(sock.clone(), None));
    assert_eq!(quadcam_lib::mcp::Backend::mode(&mut mcp.backend), "app");
    let r = mcp.call_tool(
        "quadcam_profiles",
        json!({"action": "save", "name": "Five", "fields": {"aircraft": "5-inch", "place": "Golden Gate Bridge"}}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let r = mcp.call_tool(
        "quadcam_settings",
        json!({"action": "write", "values": {"format": "mov"}}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert!(
        gui.settings.load(Ordering::SeqCst) >= 2,
        "the app heard about the agent's writes"
    );

    // The app (the person in Settings) changes one key. It writes only that key.
    app.settings_set(&serde_json::from_value(json!({"placeFolders": true})).unwrap())
        .unwrap();

    let v = read(&f);
    assert_eq!(
        v["places"][0]["name"], "Golden Gate Bridge",
        "the CLI's place survived"
    );
    assert_eq!(
        v["profiles"][1]["name"], "Five",
        "the agent's profile survived"
    );
    assert_eq!(v["profiles"][0]["name"], "Whoop A");
    assert_eq!(v["format"], "mov");
    assert_eq!(v["placeFolders"], true);
    assert_eq!(v["formatLabel"], "ECHO");

    // The app's defaults include the CLI's write after any reload (its file watcher calls
    // reload_settings; here the app's own write did).
    let d = app.defaults();
    assert_eq!(d.places.len(), 1);
    assert_eq!(d.profiles.len(), 2);
    assert_eq!(d.format, quadcam_lib::media::Format::Mov);

    // The socket answers the raw settings call the GUI uses.
    let mut client = Client::connect(&sock).unwrap();
    let s = client.call("settings", Value::Null).unwrap();
    assert_eq!(s["values"]["placeFolders"], true);
    assert_eq!(s["effective"]["format"], "mov");
}

#[test]
fn the_google_key_is_written_but_never_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("settings.json");
    let c = core(dir.path(), Arc::new(NoHooks), &f);
    let v = c
        .settings_set(
            &serde_json::from_value(
                json!({"google_places_key": "test-key-123", "geocoder": "google"}),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(v.values["googlePlacesKey"], "(set)");
    assert_eq!(read(&f)["googlePlacesKey"], "test-key-123");
    assert_eq!(
        c.defaults().google_places_key.as_deref(),
        Some("test-key-123")
    );
    let shown = serde_json::to_string(&c.status()).unwrap()
        + &serde_json::to_string(&c.settings().unwrap()).unwrap();
    assert!(!shown.contains("test-key-123"), "{shown}");
    let r = Server::new(quadcam_lib::mcp::LocalBackend(c.clone()))
        .call_tool("quadcam_settings", json!({"action": "read"}));
    assert!(!r.to_string().contains("test-key-123"));
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("google_places_key set"));
}
