//! The voice studio on the core (design 7.4), with a fake ElevenLabs only: the key in a
//! fake Keychain (never in the settings file or an answer), the catalogue, the estimate,
//! A/B samples, and a batched render of line sets into a local pack. Every request goes to
//! canned HTTP; a cache hit makes none, and a render the credits do not cover makes none.

use quadcam_lib::core::{
    CatalogParams, Core, EstimateParams, KeyParams, NoHooks, SampleParams, VoiceRenderParams,
};
use quadcam_lib::gear::serial::FakePorts;
use quadcam_lib::gear::voice::eleven::fake::FakeHttp;
use quadcam_lib::gear::voice::eleven::Eleven;
use quadcam_lib::gear::voice::keychain::{KeyStore, MemKeys};
use quadcam_lib::gear::voice::tts::{ProviderConfig, Providers, Tts};
use quadcam_lib::gear::Env;
use quadcam_lib::mcp::{LocalBackend, Server};
use quadcam_lib::photos::Recorder;
use serde_json::json;
use std::sync::Arc;

const KEY: &str = "fake-key-0123456789abcdef";

struct Fake(Arc<FakeHttp>);

impl Providers for Fake {
    fn make(&self, cfg: &ProviderConfig) -> anyhow::Result<Box<dyn Tts>> {
        if cfg.provider != "elevenlabs" {
            anyhow::bail!("only ElevenLabs here");
        }
        Ok(Box::new(
            Eleven::new(self.0.clone(), cfg.key.clone().unwrap()).without_backoff(),
        ))
    }
}

struct Bench {
    core: Core,
    http: Arc<FakeHttp>,
    keys: Arc<MemKeys>,
    dir: tempfile::TempDir,
}

const MODELS: &str = r#"[
  {"model_id":"eleven_v4","name":"Eleven v4","can_do_text_to_speech":true},
  {"model_id":"eleven_v4_turbo","name":"Eleven v4 Turbo","can_do_text_to_speech":true},
  {"model_id":"eleven_v3","name":"Eleven v3","can_do_text_to_speech":true},
  {"model_id":"eleven_v3_conversational","name":"Eleven v3 Conversational","can_do_text_to_speech":true},
  {"model_id":"eleven_multilingual_v2","name":"Multilingual v2","can_do_text_to_speech":true},
  {"model_id":"eleven_flash_v2_5","name":"Flash v2.5","can_do_text_to_speech":true},
  {"model_id":"eleven_turbo_v2_5","name":"Turbo v2.5","can_do_text_to_speech":true,"model_rates":{"character_cost_multiplier":0.5}},
  {"model_id":"eleven_turbo_v2","name":"Turbo v2","can_do_text_to_speech":true,"model_rates":{"character_cost_multiplier":0.5}},
  {"model_id":"eleven_flash_v2","name":"Flash v2","can_do_text_to_speech":true,"model_rates":{"character_cost_multiplier":0.5}}]"#;
const VOICES: &str = r#"{"voices":[
  {"voice_id":"v-callum","name":"Callum","category":"premade","labels":{"accent":"american"}},
  {"voice_id":"v-matilda","name":"Matilda","category":"premade"}]}"#;

fn sub(left: u64) -> String {
    format!(
        r#"{{"tier":"creator","character_count":{},"character_limit":100000}}"#,
        100000 - left
    )
}

fn bench(credits_left: u64, key: bool) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let http = Arc::new(FakeHttp::default());
    http.on("/v1/models", 200, MODELS);
    http.on("/v1/voices", 200, VOICES);
    http.on("/v1/user/subscription", 200, sub(credits_left));
    http.synth(0.06);
    let keys = Arc::new(MemKeys::default());
    if key {
        keys.set("elevenlabs-api-key", KEY).unwrap();
    }
    let mut env = Env::fake(vec![], Arc::new(FakePorts::new(vec![])));
    env.tts = Arc::new(Fake(http.clone()));
    env.keys = keys.clone();
    std::fs::create_dir_all(dir.path().join("support")).unwrap();
    let core = Core::new(
        dir.path().join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(dir.path().join("support/settings.json"))
    .with_gear_env(env);
    Bench {
        core,
        http,
        keys,
        dir,
    }
}

fn key(action: &str, k: Option<&str>) -> KeyParams {
    KeyParams {
        action: action.into(),
        key: k.map(String::from),
    }
}

#[test]
fn the_key_lives_in_the_keychain_and_never_comes_back() {
    let b = bench(50000, false);
    assert!(!b.core.gear_voice_key(&key("status", None)).unwrap().set);
    let s = b.core.gear_voice_key(&key("set", Some(KEY))).unwrap();
    assert!(s.set);
    assert_eq!(s.source, "keychain");
    assert_eq!(s.hint, "ends in cdef");
    let shown = serde_json::to_string(&s).unwrap();
    assert!(
        !shown.contains(KEY) && !shown.contains("0123456789"),
        "{shown}"
    );
    assert_eq!(
        b.keys.get("elevenlabs-api-key").unwrap().as_deref(),
        Some(KEY)
    );
    // Not in the settings file, not in the gear folder, not in the voice view.
    for f in ["support/settings.json", "support/gear/gear.json"] {
        if let Ok(t) = std::fs::read_to_string(b.dir.path().join(f)) {
            assert!(!t.contains(KEY), "{f}");
        }
    }
    let view = serde_json::to_string(&b.core.gear_voice(&Default::default()).unwrap()).unwrap();
    assert!(!view.contains(KEY));
    assert!(b
        .core
        .gear_voice_key(&key("set", Some("two words")))
        .is_err());
    assert!(b.core.gear_voice_key(&key("set", Some("  "))).is_err());
    assert!(b.core.gear_voice_key(&key("bogus", None)).is_err());
    assert!(!b.core.gear_voice_key(&key("delete", None)).unwrap().set);
    assert_eq!(b.keys.get("elevenlabs-api-key").unwrap(), None);
}

#[test]
fn the_catalogue_needs_a_key_and_lists_voices_models_and_credits() {
    let b = bench(50000, false);
    let e = b
        .core
        .gear_voice_catalog(&CatalogParams::default())
        .unwrap_err();
    assert!(e.to_string().contains("key set"), "{e}");
    assert!(b.http.seen.lock().unwrap().is_empty());
    b.core.gear_voice_key(&key("set", Some(KEY))).unwrap();
    let c = b
        .core
        .gear_voice_catalog(&CatalogParams::default())
        .unwrap();
    assert_eq!(c.voices.len(), 2);
    // The account lists v4 and v4 turbo by these ids; the rate table prices them.
    let by = |id: &str| c.models.iter().find(|m| m.id == id).expect(id);
    assert_eq!(by("eleven_v4").cost_per_char, 1.0);
    assert_eq!(by("eleven_v4_turbo").cost_per_char, 0.5);
    assert_eq!(by("eleven_turbo_v2_5").cost_per_char, 0.5);
    assert_eq!(by("eleven_v3").max_chars, 5000);
    assert_eq!(c.credits.unwrap().remaining, 50000);
    let only = b
        .core
        .gear_voice_catalog(&CatalogParams {
            credits: true,
            ..Default::default()
        })
        .unwrap();
    assert!(only.voices.is_empty() && only.credits.is_some());
}

fn estimate(
    b: &Bench,
    sets: &[&str],
    model: &str,
) -> anyhow::Result<quadcam_lib::core::StudioEstimate> {
    b.core.gear_voice_estimate(&EstimateParams {
        sets: sets.iter().map(|s| s.to_string()).collect(),
        voice: "callum".into(),
        model: model.into(),
        ..Default::default()
    })
}

#[test]
fn the_estimate_prices_carriers_at_the_models_rate_and_makes_no_paid_call() {
    let b = bench(50000, true);
    let half = estimate(&b, &["sample"], "eleven_turbo_v2_5").unwrap();
    let full = estimate(&b, &["sample"], "eleven_v4").unwrap();
    assert_eq!(half.voice, "v-callum");
    assert_eq!(half.voice_name, "Callum");
    assert_eq!(half.lines, 12);
    // Batches by tone: calm, alert and number lines never share a take.
    assert_eq!(half.estimate.batches, 3);
    assert!(half.estimate.chars > 12 * "The word is .".len() as u64);
    assert_eq!(full.estimate.credits, full.estimate.chars);
    assert_eq!(
        half.estimate.credits,
        (half.estimate.chars as f64 / 2.0).ceil() as u64
    );
    // USD from the table: flash and turbo v2.5 are $0.04 per 1K characters, with no promo.
    assert_eq!(half.estimate.usd_per_1k, 0.04);
    assert!((half.estimate.usd - half.estimate.chars as f64 * 0.04 / 1000.0).abs() < 1e-9);
    assert_eq!(half.estimate.credits_basis, "estimated");
    assert_eq!(half.estimate.promo_until, None);
    let text = quadcam_lib::core::voice_estimate_text(&half.estimate);
    assert!(
        text.contains("$0.04 per 1K") && text.contains("credits (estimated)"),
        "{text}"
    );
    // v4 is $0.08, or $0.022 while its promo lasts.
    let u = full.estimate.usd_per_1k;
    assert!(u == 0.08 || u == 0.022, "{u}");
    assert_eq!(full.estimate.promo_until.is_some(), u == 0.022);
    assert_eq!(b.http.posts(), 0);
    assert!(estimate(&b, &["nope"], "eleven_v4").is_err());
    assert!(estimate(&b, &["sample"], "eleven_nothing")
        .unwrap_err()
        .to_string()
        .contains("lists no model"));
    // Two sets combine and a line in both counts once.
    let one = estimate(&b, &["quad"], "eleven_v4").unwrap();
    let two = estimate(&b, &["quad", "sample"], "eleven_v4").unwrap();
    assert!(two.lines < one.lines + 12);
}

fn sample(voices: &[&str], models: &[&str], dry: bool, confirm: bool) -> SampleParams {
    SampleParams {
        voices: voices.iter().map(|s| s.to_string()).collect(),
        models: models.iter().map(|s| s.to_string()).collect(),
        dry_run: dry,
        confirm,
        ..Default::default()
    }
}

#[test]
fn a_sample_waits_for_confirm_then_writes_one_wav_per_voice_model_and_line() {
    let b = bench(50000, true);
    let both = ["eleven_turbo_v2_5", "eleven_v4"];
    let dry = b
        .core
        .gear_voice_sample(&sample(&["Callum", "Matilda"], &both, true, false))
        .unwrap();
    assert!(dry.dry_run && dry.items.is_empty());
    assert_eq!(dry.combos, 4);
    assert_eq!(dry.estimate.batches, 12);
    assert_eq!(b.http.posts(), 0);

    let wait = b
        .core
        .gear_voice_sample(&sample(&["Callum", "Matilda"], &both, false, false))
        .unwrap();
    assert!(wait.needs_confirm && wait.items.is_empty());
    assert_eq!(b.http.posts(), 0);

    let done = b
        .core
        .gear_voice_sample(&sample(&["Callum", "Matilda"], &both, false, true))
        .unwrap();
    assert_eq!(b.http.posts(), 12);
    assert_eq!(done.items.len(), 4 * 12);
    for i in &done.items {
        let bytes = std::fs::read(&i.file).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert!(i.ms > 100, "{} {}", i.text, i.ms);
        assert!(i
            .file
            .contains(&format!("{}-", i.voice_name.to_lowercase())));
    }
    assert!(done.items.iter().any(|i| i.text == "Six"));
    assert!(done.warnings.is_empty(), "{:?}", done.warnings);

    // The same sample again: every batch is cached, so nothing is billed or asked.
    let again = b
        .core
        .gear_voice_sample(&sample(&["Callum", "Matilda"], &both, false, false))
        .unwrap();
    assert!(!again.needs_confirm);
    assert_eq!(again.estimate.credits, 0);
    assert_eq!(again.items.len(), 48);
    assert_eq!(b.http.posts(), 12);
}

#[test]
fn a_recorded_character_count_replaces_the_estimate() {
    let b = bench(50000, true);
    // The header says each character billed a quarter credit on v4, not one.
    b.http.bill(0.25);
    let before = estimate(&b, &["sample"], "eleven_v4").unwrap();
    assert_eq!(before.estimate.credits_basis, "estimated");
    b.core
        .gear_voice_sample(&sample(&["Callum"], &["eleven_v4"], false, true))
        .unwrap();
    // Other batches (another set) are priced at the recorded rate.
    let after = estimate(&b, &["quad"], "eleven_v4").unwrap();
    assert_eq!(after.estimate.credits_basis, "recorded");
    assert!((after.estimate.cost_per_char - 0.25).abs() < 0.01);
    assert!(after.estimate.credits < after.estimate.chars / 3);
    let text = quadcam_lib::core::voice_estimate_text(&after.estimate);
    assert!(text.contains("recorded rate"), "{text}");
    // Another model keeps its estimate.
    let other = estimate(&b, &["quad"], "eleven_v4_turbo").unwrap();
    assert_eq!(other.estimate.credits_basis, "estimated");
}

#[test]
fn batches_stay_below_the_models_request_limit() {
    let b = bench(100000, true);
    // A long carrier makes a full batch of lines run past 5,000 characters.
    let est = |model: &str| {
        b.core
            .gear_voice_estimate(&EstimateParams {
                sets: vec!["edgetx".into()],
                voice: "callum".into(),
                model: model.into(),
                batch: Some(quadcam_lib::gear::voice::batch::BatchSettings {
                    carrier: format!(
                        "{} The word is {{line}}.",
                        "A long run of words that carries on.".repeat(3)
                    ),
                    max_lines: 100,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .unwrap()
            .estimate
    };
    let long = est("eleven_flash_v2_5");
    let short = est("eleven_v3");
    assert!(
        short.batches > long.batches,
        "{} {}",
        short.batches,
        long.batches
    );
    // No batch is longer than nine tenths of the model's limit.
    assert!(short.chars / short.batches as u64 <= 4500);
    assert!(long.chars / long.batches as u64 <= 36000);
}

#[test]
fn a_render_the_credits_do_not_cover_makes_no_call() {
    let b = bench(30, true);
    let e = b
        .core
        .gear_voice_sample(&sample(&["Callum"], &["eleven_v4"], false, true))
        .unwrap_err();
    assert!(e.to_string().contains("credits"), "{e}");
    assert_eq!(b.http.posts(), 0);
    let e = b
        .core
        .gear_voice_render(&VoiceRenderParams {
            voice: "Callum".into(),
            model: "eleven_v4".into(),
            sets: vec!["quad".into()],
            confirm: true,
            ..Default::default()
        })
        .unwrap_err();
    assert!(e.to_string().contains("credits"), "{e}");
    assert_eq!(b.http.posts(), 0);
}

fn render(voice: &str, model: &str, sets: &[&str], dry: bool, confirm: bool) -> VoiceRenderParams {
    VoiceRenderParams {
        voice: voice.into(),
        model: model.into(),
        sets: sets.iter().map(|s| s.to_string()).collect(),
        dry_run: dry,
        confirm,
        ..Default::default()
    }
}

#[test]
fn a_set_renders_into_a_local_pack_and_recutting_costs_nothing() {
    let b = bench(50000, true);
    let dry = b
        .core
        .gear_voice_render(&render(
            "Callum",
            "eleven_turbo_v2_5",
            &["quad"],
            true,
            false,
        ))
        .unwrap();
    assert!(dry.dry_run && dry.estimate.is_some());
    assert_eq!(b.http.posts(), 0);
    let wait = b
        .core
        .gear_voice_render(&render(
            "Callum",
            "eleven_turbo_v2_5",
            &["quad"],
            false,
            false,
        ))
        .unwrap();
    assert!(wait.needs_confirm);
    assert_eq!(b.http.posts(), 0);

    let r = b
        .core
        .gear_voice_render(&render(
            "Callum",
            "eleven_turbo_v2_5",
            &["quad"],
            false,
            true,
        ))
        .unwrap();
    assert_eq!(r.pack, "local-elevenlabs-callum-eleven-turbo-v2-5");
    assert!(r.rendered > 0 && !r.needs_confirm, "{r:?}");
    let posts = b.http.posts();
    assert_eq!(posts as u32, r.rendered);
    let pack = b.dir.path().join("support/gear/voices").join(&r.pack);
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(pack.join("pack.json")).unwrap()).unwrap();
    assert_eq!(manifest["provider"], "elevenlabs");
    assert_eq!(manifest["model"], "eleven_turbo_v2_5");
    let files = manifest["files"].as_array().unwrap();
    assert!(files.iter().any(|f| f == "SOUNDS/en/armed.wav"));
    assert!(files.iter().any(|f| f == "SOUNDS/en/SYSTEM/0006.wav"));
    for f in files {
        assert!(pack.join(f.as_str().unwrap()).is_file(), "{f}");
    }
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);

    // The same render with other trim settings re-cuts from the cache: no request.
    let mut again = render("Callum", "eleven_turbo_v2_5", &["quad"], false, false);
    again.settings = Some(quadcam_lib::gear::voice::render::RenderSettings {
        tail_ms: 200,
        ..Default::default()
    });
    let r2 = b.core.gear_voice_render(&again).unwrap();
    assert!(!r2.needs_confirm);
    assert_eq!(r2.rendered, 0);
    assert_eq!(b.http.posts(), posts);

    // A second set adds to the same pack.
    let r3 = b
        .core
        .gear_voice_render(&render(
            "Callum",
            "eleven_turbo_v2_5",
            &["heli"],
            false,
            true,
        ))
        .unwrap();
    assert_eq!(r3.pack, r.pack);
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(pack.join("pack.json")).unwrap()).unwrap();
    let files = m["files"].as_array().unwrap();
    assert!(files.iter().any(|f| f == "SOUNDS/en/idleu1.wav"));
    assert!(files.iter().any(|f| f == "SOUNDS/en/turtle.wav"));
}

#[test]
fn a_render_without_a_voice_or_model_says_what_to_name() {
    let b = bench(50000, true);
    let e = b
        .core
        .gear_voice_render(&render("", "eleven_v4", &["quad"], true, false))
        .unwrap_err();
    assert!(e.to_string().contains("Name a voice"), "{e}");
    let e = b
        .core
        .gear_voice_render(&render("Callum", "", &["quad"], true, false))
        .unwrap_err();
    assert!(e.to_string().contains("Name a model"), "{e}");
    let e = b
        .core
        .gear_voice_render(&render("Nobody", "eleven_v4", &["quad"], true, false))
        .unwrap_err();
    assert!(e.to_string().contains("no voice"), "{e}");
}

#[test]
fn the_mcp_tools_price_first_and_a_paid_sample_needs_confirm() {
    let b = bench(50000, true);
    let http = b.http.clone();
    let mut s = Server::new(LocalBackend(Arc::new(b.core)));

    let r = s.call_tool("quadcam_gear", json!({"action": "voice_sets"}));
    assert_eq!(r["isError"], false, "{r}");
    let t = r["content"][0]["text"].as_str().unwrap();
    assert!(
        t.contains("key is stored") && t.contains("edgetx") && t.contains("sample"),
        "{t}"
    );
    assert!(!t.contains(KEY));

    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "voice_catalog", "what": "credits"}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("50000 left"));

    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "voice_estimate", "sets": ["sample"], "voice": "Callum", "voice_model": "eleven_turbo_v2_5"}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("credits"));
    assert_eq!(http.posts(), 0);

    let ask = json!({"action": "voice_sample", "voices": ["Callum"], "voice_models": ["eleven_turbo_v2_5"]});
    let r = s.call_tool("quadcam_gear_edit", ask);
    assert_eq!(r["isError"], false, "{r}");
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Not rendered"));
    assert_eq!(http.posts(), 0);

    let go = json!({"action": "voice_sample", "voices": ["Callum"], "voice_models": ["eleven_turbo_v2_5"], "confirm": true});
    let r = s.call_tool("quadcam_gear_edit", go);
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(http.posts(), 3);

    // An agent can delete the key but has no way to set one.
    let r = s.call_tool("quadcam_gear_edit", json!({"action": "voice_key_delete"}));
    assert_eq!(r["isError"], false, "{r}");
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("No ElevenLabs key"));
    let r = s.call_tool(
        "quadcam_gear_edit",
        json!({"action": "voice_key_set", "key": KEY}),
    );
    assert_eq!(r["isError"], true);
}
