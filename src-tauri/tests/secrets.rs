//! Write-only secrets: the ElevenLabs key (a fake Keychain here) and the settings in
//! `settings::SECRET_KEYS`. Each surface can set one; no answer, error or file but the
//! secret's own store ever holds it, and every read shows only `secret::hint`.
//! Each test secret carries a marker in its middle that no hint may show.

use quadcam_lib::core::{Core, KeyParams, NoHooks};
use quadcam_lib::gear::serial::FakePorts;
use quadcam_lib::gear::voice::keychain::{KeyStore, MemKeys};
use quadcam_lib::gear::Env;
use quadcam_lib::mcp::{LocalBackend, Server};
use quadcam_lib::photos::Recorder;
use serde_json::{json, Value};
use std::sync::Arc;

const ELEVEN: &str = "sk_9e1f-ELEVENMIDDLE-4a2b8d4d";
const GOOGLE: &str = "AIza-GOOGLEMIDDLE-q7Rt";
const TTS: &str = "tts-TTSMIDDLE-0042";
const PHRASE: &str = "my PHRASEMIDDLE words";
const MARKERS: &[&str] = &["ELEVENMIDDLE", "GOOGLEMIDDLE", "TTSMIDDLE", "PHRASEMIDDLE"];

/// Fails when `shown` holds any test secret's middle.
fn no_secret(what: &str, shown: &str) {
    for m in MARKERS {
        assert!(!shown.contains(m), "{what} shows a secret: {shown}");
    }
}

fn core(dir: &std::path::Path, keys: Arc<MemKeys>) -> Arc<Core> {
    let mut env = Env::fake(vec![], Arc::new(FakePorts::new(vec![])));
    env.keys = keys;
    std::fs::create_dir_all(dir.join("support")).unwrap();
    Arc::new(
        Core::new(
            dir.join("cache"),
            None,
            Arc::new(NoHooks),
            Arc::new(Recorder::default()),
        )
        .with_settings(dir.join("support/settings.json"))
        .with_gear_env(env),
    )
}

/// Every file under `dir` but the settings file, which is the settings' own store.
fn files_but_settings(dir: &std::path::Path) -> String {
    let mut all = String::new();
    for e in walk(dir) {
        if e.file_name().is_some_and(|n| n == "settings.json") {
            continue;
        }
        all += &String::from_utf8_lossy(&std::fs::read(&e).unwrap_or_default());
    }
    all
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

#[test]
fn mcp_sets_every_secret_and_reads_back_only_hints() {
    let dir = tempfile::tempdir().unwrap();
    let keys = Arc::new(MemKeys::default());
    let c = core(dir.path(), keys.clone());
    let mut s = Server::new(LocalBackend(c.clone()));
    let mut seen = Vec::new();
    let mut call = |tool: &str, a: Value| {
        let r = s.call_tool(tool, a);
        seen.push(r.to_string());
        r
    };

    let r = call(
        "quadcam_gear_edit",
        json!({"action": "voice_key_set", "key": format!("{ELEVEN}\n")}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["hint"], "sk_9…8d4d (29 chars)");
    assert_eq!(
        keys.get("elevenlabs-api-key").unwrap().as_deref(),
        Some(ELEVEN)
    );

    let r = call(
        "quadcam_settings",
        json!({"action": "write", "values": {"google_places_key": GOOGLE, "tts_key": TTS, "elrs_binding_phrase": PHRASE}}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let r = call("quadcam_settings", json!({"action": "read"}));
    let sec = &r["structuredContent"]["secrets"];
    assert_eq!(sec["google_places_key"]["hint"], "AIza…q7Rt (22 chars)");
    assert_eq!(sec["tts_key"]["hint"], "tts-…0042 (18 chars)");
    assert_eq!(sec["elrs_binding_phrase"]["hint"], "my P…ords (21 chars)");
    assert_eq!(sec["elrs_binding_phrase"]["length"], 21);
    let t = r["content"][0]["text"].as_str().unwrap();
    assert!(t.contains("google_places_key AIza…q7Rt (22 chars)"), "{t}");

    // Refused values: the error names the rule, never the value.
    for bad in [
        json!({"action": "voice_key_set", "key": "sk_9e1f ELEVENMIDDLE 4a2b"}),
        json!({"action": "voice_key_set", "key": "sk_9e1f\u{7}ELEVENMIDDLE"}),
    ] {
        let r = call("quadcam_gear_edit", bad);
        assert_eq!(r["isError"], true, "{r}");
    }
    let r = call(
        "quadcam_settings",
        json!({"action": "write", "values": {"elrs_binding_phrase": "PHRASEMIDDLE\"quoted"}}),
    );
    assert_eq!(r["isError"], true, "{r}");

    // Reads anywhere else show none of them either.
    call("quadcam_status", json!({}));
    call("quadcam_gear", json!({"action": "status"}));
    call("quadcam_gear", json!({"action": "voice"}));

    for (i, out) in seen.iter().enumerate() {
        no_secret(&format!("answer {i}"), out);
    }
    no_secret("files", &files_but_settings(dir.path()));
}

#[test]
fn the_core_reports_hints_only() {
    let dir = tempfile::tempdir().unwrap();
    let keys = Arc::new(MemKeys::default());
    let c = core(dir.path(), keys);
    let k = c
        .gear_voice_key(&KeyParams {
            action: "set".into(),
            key: Some(ELEVEN.into()),
        })
        .unwrap();
    assert_eq!(
        (k.prefix.as_str(), k.suffix.as_str(), k.length),
        ("sk_9", "8d4d", 29)
    );
    let v = c
        .settings_set(
            &serde_json::from_value(
                json!({"google_places_key": GOOGLE, "tts_key": TTS, "elrs_binding_phrase": PHRASE}),
            )
            .unwrap(),
        )
        .unwrap();
    // Schema 1 keeps the "(set)" marker in `values`; `secrets` holds the hints.
    assert_eq!(v.values["ttsKey"], "(set)");
    assert_eq!(v.secrets["ttsKey"].hint, "tts-…0042 (18 chars)");
    let e = c
        .gear_voice_key(&KeyParams {
            action: "set".into(),
            key: Some("two ELEVENMIDDLE words".into()),
        })
        .unwrap_err();
    let shown = [
        serde_json::to_string(&k).unwrap(),
        serde_json::to_string(&v).unwrap(),
        serde_json::to_string(&c.settings().unwrap()).unwrap(),
        serde_json::to_string(&c.status()).unwrap(),
        serde_json::to_string(&c.gear_status().unwrap()).unwrap(),
        serde_json::to_string(&c.gear_voice_sets().unwrap()).unwrap(),
        format!("{e:#}"),
    ];
    for (i, s) in shown.iter().enumerate() {
        no_secret(&format!("core answer {i}"), s);
    }
}
