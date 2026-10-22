//! The voice studio on the core (design 7.4), with a fake ElevenLabs only: the key in a
//! fake Keychain (never in the settings file or an answer), the catalogue, the estimate,
//! A/B samples, and a batched render of line sets into a local pack. Every request goes to
//! canned HTTP; a cache hit makes none, and a render the credits do not cover makes none.

use quadcam_lib::api::{Event, VoiceProgress};
use quadcam_lib::core::{
    CatalogParams, Core, EstimateParams, Hooks, KeyParams, NoHooks, RenderPacksParams,
    SampleParams, VoiceRenderParams,
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
  {"voice_id":"v-callum","name":"Callum","category":"premade","labels":{"accent":"american"},"description":"A steady voice."},
  {"voice_id":"v-matilda","name":"Matilda","category":"premade"}]}"#;

fn sub(left: u64) -> String {
    format!(
        r#"{{"tier":"creator","character_count":{},"character_limit":100000}}"#,
        100000 - left
    )
}

fn bench(credits_left: u64, key: bool) -> Bench {
    bench_with(credits_left, key, Arc::new(NoHooks))
}

fn bench_with(credits_left: u64, key: bool, hooks: Arc<dyn Hooks>) -> Bench {
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
        hooks,
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
    assert_eq!(s.hint, "fake…cdef (25 chars)");
    assert_eq!(
        (s.prefix.as_str(), s.suffix.as_str(), s.length),
        ("fake", "cdef", 25)
    );
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
    let v4 = by("eleven_v4");
    assert_eq!(
        v4.cost_per_char,
        quadcam_lib::gear::voice::rates::credits_per_char(v4.usd_per_1k)
    );
    assert_eq!(by("eleven_v3").cost_per_char, 0.22);
    assert_eq!(by("eleven_turbo_v2_5").cost_per_char, 0.11);
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
    // Credits from the dollar rate and the credit price: $0.04 per 1K is 0.11 a character.
    assert_eq!(
        half.estimate.credits,
        (half.estimate.chars as f64 * 0.11).ceil() as u64
    );
    assert_eq!(
        full.estimate.cost_per_char,
        quadcam_lib::gear::voice::rates::credits_per_char(full.estimate.usd_per_1k)
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

/// A paid sample the way the studio runs it: ask, then confirm with the digest.
fn paid_sample(b: &Bench, p: SampleParams) -> quadcam_lib::core::SampleReport {
    let ask = b
        .core
        .gear_voice_sample(&SampleParams {
            confirm: false,
            ..p.clone()
        })
        .unwrap();
    b.core
        .gear_voice_sample(&SampleParams {
            confirm: true,
            digest: Some(ask.digest),
            ..p
        })
        .unwrap()
}

/// A paid render the way the studio runs it: ask, then confirm with the digest.
fn paid_render(b: &Bench, p: VoiceRenderParams) -> quadcam_lib::core::RenderReport {
    let ask = b
        .core
        .gear_voice_render(&VoiceRenderParams {
            confirm: false,
            ..p.clone()
        })
        .unwrap();
    b.core
        .gear_voice_render(&VoiceRenderParams {
            confirm: true,
            digest: Some(ask.digest),
            ..p
        })
        .unwrap()
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

    let mut go = sample(&["Callum", "Matilda"], &both, false, true);
    go.digest = Some(wait.digest.clone());
    let done = b.core.gear_voice_sample(&go).unwrap();
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
    // The header says each character billed a quarter credit on v4, far from its one credit:
    // logged, not kept.
    b.http.bill(0.25);
    let before = estimate(&b, &["sample"], "eleven_v4").unwrap();
    assert_eq!(before.estimate.credits_basis, "estimated");
    paid_sample(&b, sample(&["Callum"], &["eleven_v4"], false, true));
    let after = estimate(&b, &["quad"], "eleven_v4").unwrap();
    assert_eq!(after.estimate.credits_basis, "estimated");
    let log = std::fs::read_to_string(b.dir.path().join("cache/voice/charcost.log")).unwrap();
    assert!(
        log.contains("eleven_v4: billed") && log.contains("not kept"),
        "{log}"
    );
    // Close to its multiplier: kept, and other batches (another set) are priced at it.
    b.http.bill(0.215);
    paid_sample(&b, sample(&["Matilda"], &["eleven_v3"], false, true));
    let after = estimate(&b, &["quad"], "eleven_v3").unwrap();
    assert_eq!(after.estimate.credits_basis, "recorded");
    assert!((after.estimate.cost_per_char - 0.215).abs() < 0.01);
    assert!(after.estimate.credits < after.estimate.chars);
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
    // No batch is longer than nine tenths of the model's limit: the planner the estimate
    // uses, on the same lines and carrier, batch by batch.
    use quadcam_lib::gear::voice::{batch, rates, sets};
    let bs = batch::BatchSettings {
        carrier: format!(
            "{} The word is {{line}}.",
            "A long run of words that carries on.".repeat(3)
        ),
        max_lines: 100,
        ..Default::default()
    };
    let items: Vec<batch::Item> = sets::lines_of("edgetx")
        .unwrap()
        .into_iter()
        .map(|l| batch::Item {
            spoken: l.line.text,
            tone: l.tone,
        })
        .collect();
    for (model, limit) in [("eleven_v3", 4500), ("eleven_flash_v2_5", 36000)] {
        assert_eq!(rates::batch_limit(model, 5000), limit, "{model}");
        let longest = batch::plan_within(&items, &bs, limit)
            .iter()
            .map(|b| b.text.chars().count())
            .max()
            .unwrap();
        assert!(longest <= limit, "{model}: a batch of {longest} characters");
    }
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

    let mut go = render("Callum", "eleven_turbo_v2_5", &["quad"], false, true);
    go.digest = Some(wait.digest.clone());
    let r = b.core.gear_voice_render(&go).unwrap();
    // The pack is named by the voice id, not the voice's name.
    assert_eq!(
        r.pack,
        format!(
            "local-elevenlabs-{}-eleven-turbo-v2-5",
            &quadcam_lib::gear::voice::sha256_hex(b"v-callum")[..12]
        )
    );
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
    let r3 = paid_render(
        &b,
        render("Callum", "eleven_turbo_v2_5", &["heli"], false, true),
    );
    assert_eq!(r3.pack, r.pack);
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(pack.join("pack.json")).unwrap()).unwrap();
    let files = m["files"].as_array().unwrap();
    assert!(files.iter().any(|f| f == "SOUNDS/en/idleu1.wav"));
    assert!(files.iter().any(|f| f == "SOUNDS/en/turtle.wav"));
}

#[test]
fn a_pack_cut_by_an_older_quadcam_is_cut_again_from_the_cache() {
    let b = bench(50000, true);
    let r = paid_render(
        &b,
        render("Callum", "eleven_turbo_v2_5", &["quad"], false, true),
    );
    let posts = b.http.posts();
    let pack = b.dir.path().join("support/gear/voices").join(&r.pack);
    let read = || -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(pack.join("pack.json")).unwrap()).unwrap()
    };
    let kept = read()["kept_batches"].clone();
    assert!(!kept.as_array().unwrap().is_empty());

    // Rendered again as it is: every batch is kept, none is cut again.
    let same = b
        .core
        .gear_voice_render(&render(
            "Callum",
            "eleven_turbo_v2_5",
            &["quad"],
            false,
            false,
        ))
        .unwrap();
    assert_eq!((same.rendered, same.from_cache), (0, 0), "{same:?}");

    // A pack whose kept batches were cut by an older cut (another cut version in their keys):
    // the render cuts every batch again from the batch cache, with no confirm and no request.
    let mut m = read();
    m["kept_batches"] = serde_json::json!(["cut-by-an-older-quadcam"]);
    std::fs::write(pack.join("pack.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    let again = b
        .core
        .gear_voice_render(&render(
            "Callum",
            "eleven_turbo_v2_5",
            &["quad"],
            false,
            false,
        ))
        .unwrap();
    assert!(!again.needs_confirm, "{again:?}");
    assert_eq!(again.plan.chars, 0);
    assert_eq!(again.rendered, 0);
    assert_eq!(again.from_cache, r.rendered);
    assert_eq!(b.http.posts(), posts);
    let now = read()["kept_batches"].clone();
    for k in kept.as_array().unwrap() {
        assert!(now.as_array().unwrap().contains(k), "{k}");
    }
}

#[test]
fn deleting_a_studio_pack_keeps_its_batches_unless_asked() {
    use quadcam_lib::core::{PackDeleteParams, VoiceParams};
    let b = bench(90_000, true);
    let r = paid_render(
        &b,
        render("Callum", "eleven_turbo_v2_5", &["quad"], false, true),
    );
    let posts = b.http.posts();
    let v = b.core.gear_voice(&VoiceParams::default()).unwrap();
    let k = v.packs.iter().find(|k| k.id == r.pack).unwrap();
    assert_eq!(
        (k.firmware.as_str(), k.model.as_str()),
        ("edgetx", "eleven_turbo_v2_5")
    );
    assert!(k.sets.contains(&"quad".to_string()), "{:?}", k.sets);
    let gone = b
        .core
        .gear_voice_pack_delete(&PackDeleteParams {
            pack: r.pack.clone(),
            takes: false,
        })
        .unwrap();
    assert_eq!(gone.takes, 0);
    // The same render again comes from the batch cache: no request.
    paid_render(
        &b,
        render("Callum", "eleven_turbo_v2_5", &["quad"], false, true),
    );
    assert_eq!(b.http.posts(), posts);
    let gone = b
        .core
        .gear_voice_pack_delete(&PackDeleteParams {
            pack: r.pack.clone(),
            takes: true,
        })
        .unwrap();
    assert_eq!(gone.takes as usize, posts);
    // Now it pays again: the render waits for confirm.
    let again = b
        .core
        .gear_voice_render(&render(
            "Callum",
            "eleven_turbo_v2_5",
            &["quad"],
            false,
            false,
        ))
        .unwrap();
    assert!(again.needs_confirm);
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
    let t = r["content"][0]["text"].as_str().unwrap();
    let digest = r["structuredContent"]["digest"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(t.contains("Not rendered") && t.contains(&digest), "{t}");
    assert_eq!(http.posts(), 0);

    // A confirm without the digest, or with a digest of another plan, pays nothing.
    let bare = json!({"action": "voice_sample", "voices": ["Callum"], "voice_models": ["eleven_turbo_v2_5"], "confirm": true});
    let r = s.call_tool("quadcam_gear_edit", bare);
    assert_eq!(r["isError"], true, "{r}");
    let more = json!({"action": "voice_sample", "voices": ["Callum", "Matilda"], "voice_models": ["eleven_turbo_v2_5"], "confirm": true, "digest": digest});
    let r = s.call_tool("quadcam_gear_edit", more);
    assert_eq!(r["isError"], true, "{r}");
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("plan changed"));
    assert_eq!(http.posts(), 0);

    let go = json!({"action": "voice_sample", "voices": ["Callum"], "voice_models": ["eleven_turbo_v2_5"], "confirm": true, "digest": digest});
    let r = s.call_tool("quadcam_gear_edit", go);
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(http.posts(), 3);

    // An agent can delete the key and set one; the answer shows only the hint.
    let r = s.call_tool("quadcam_gear_edit", json!({"action": "voice_key_delete"}));
    assert_eq!(r["isError"], false, "{r}");
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("No ElevenLabs key"));
    let r = s.call_tool(
        "quadcam_gear_edit",
        json!({"action": "voice_key_set", "key": format!("{KEY}\n")}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert!(!r.to_string().contains("0123456789"), "{r}");
    assert_eq!(r["structuredContent"]["hint"], "fake…cdef (25 chars)");
    assert_eq!(r["structuredContent"]["length"], 25);
}

#[test]
fn a_confirm_pays_only_for_the_plan_its_estimate_priced() {
    let b = bench(50000, true);
    // Sample: priced with one voice and one model, confirmed after more were ticked.
    let one = b
        .core
        .gear_voice_sample(&sample(&["Callum"], &["eleven_turbo_v2_5"], false, false))
        .unwrap();
    assert!(one.needs_confirm && !one.digest.is_empty());
    let mut more = sample(
        &["Callum", "Matilda"],
        &["eleven_turbo_v2_5", "eleven_v4"],
        false,
        true,
    );
    more.digest = Some(one.digest.clone());
    let e = b.core.gear_voice_sample(&more).unwrap_err();
    assert!(e.to_string().contains("plan changed"), "{e}");
    let e = b
        .core
        .gear_voice_sample(&sample(&["Callum"], &["eleven_turbo_v2_5"], false, true))
        .unwrap_err();
    assert!(e.to_string().contains("digest"), "{e}");
    assert_eq!(b.http.posts(), 0);

    // Render: priced for one set, confirmed for another, or with another seed.
    let ask = b
        .core
        .gear_voice_render(&render(
            "Callum",
            "eleven_turbo_v2_5",
            &["quad"],
            false,
            false,
        ))
        .unwrap();
    assert!(ask.needs_confirm && !ask.digest.is_empty());
    let mut other = render("Callum", "eleven_turbo_v2_5", &["heli"], false, true);
    other.digest = Some(ask.digest.clone());
    assert!(b
        .core
        .gear_voice_render(&other)
        .unwrap_err()
        .to_string()
        .contains("plan changed"));
    let mut seeded = render("Callum", "eleven_turbo_v2_5", &["quad"], false, true);
    seeded.digest = Some(ask.digest.clone());
    seeded.settings = Some(quadcam_lib::gear::voice::render::RenderSettings {
        seed: 5,
        ..Default::default()
    });
    assert!(b
        .core
        .gear_voice_render(&seeded)
        .unwrap_err()
        .to_string()
        .contains("plan changed"));
    assert_eq!(b.http.posts(), 0);
    // The plan it priced goes ahead.
    let mut same = render("Callum", "eleven_turbo_v2_5", &["quad"], false, true);
    same.digest = Some(ask.digest);
    assert!(b.core.gear_voice_render(&same).unwrap().rendered > 0);
}

#[test]
fn a_flagged_line_stays_out_of_the_pack_until_a_re_take_redoes_its_batch() {
    let b = bench(50000, true);
    // The first take of "Signal low" is silent; a take with a seed is not.
    b.http.mute("Signal low");
    let r = paid_render(
        &b,
        render("Callum", "eleven_turbo_v2_5", &["quad"], false, true),
    );
    let batches = r.rendered;
    assert!(batches > 1, "{r:?}");
    // Two card paths speak "Signal low": both need the re-take.
    let mut paths: Vec<&str> = r.retakes.iter().map(|t| t.path.as_str()).collect();
    paths.sort();
    assert_eq!(
        paths,
        ["SOUNDS/en/SYSTEM/rssi_org.wav", "SOUNDS/en/lowrssi.wav"],
        "{r:?}"
    );
    assert!(r.retakes.iter().all(|t| t.reason == "the cut is silent"));
    let text = quadcam_lib::core::voice_report_text(&r);
    assert!(
        text.contains("re-take") && text.contains("lowrssi"),
        "{text}"
    );
    let pack = b.dir.path().join("support/gear/voices").join(&r.pack);
    assert!(!pack.join("SOUNDS/en/lowrssi.wav").exists());
    assert!(pack.join("SOUNDS/en/critbat.wav").is_file());
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(pack.join("pack.json")).unwrap()).unwrap();
    assert!(!m["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f == "SOUNDS/en/lowrssi.wav"));
    assert_eq!(m["retakes"].as_array().unwrap().len(), 2);
    // The pack library shows the lines that need a re-take.
    let library = |b: &Bench| {
        let v = b
            .core
            .gear_voice(&quadcam_lib::core::VoiceParams::default())
            .unwrap();
        v.packs.into_iter().find(|k| k.id == r.pack).unwrap()
    };
    let k = library(&b);
    assert_eq!(k.retakes.len(), 2, "{k:?}");
    // The lines waiting for a re-take do not drop the set from the pack's sets.
    assert!(k.sets.contains(&"quad".to_string()), "{:?}", k.sets);
    assert!(quadcam_lib::core::voice_view_text(
        &b.core
            .gear_voice(&quadcam_lib::core::VoiceParams::default())
            .unwrap()
    )
    .contains("2 lines need a re-take"));
    let posts = b.http.posts();
    assert_eq!(posts as u32, batches);

    // The paid batch stays cached: the same render again pays nothing and still leaves it out.
    let again = b
        .core
        .gear_voice_render(&render(
            "Callum",
            "eleven_turbo_v2_5",
            &["quad"],
            false,
            false,
        ))
        .unwrap();
    assert!(!again.needs_confirm);
    assert_eq!(again.retakes.len(), 2);
    assert_eq!(b.http.posts(), posts);

    // A re-take with a new seed redoes only the batch that holds the flagged line.
    let mut retake = render("Callum", "eleven_turbo_v2_5", &["quad"], false, true);
    retake.settings = Some(quadcam_lib::gear::voice::render::RenderSettings {
        seed: 7,
        ..Default::default()
    });
    let est = b
        .core
        .gear_voice_estimate(&EstimateParams {
            sets: vec!["quad".into()],
            voice: "Callum".into(),
            model: "eleven_turbo_v2_5".into(),
            settings: retake.settings.clone(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(est.estimate.batches, 1);
    let r2 = paid_render(&b, retake);
    assert_eq!(r2.rendered, 1, "{r2:?}");
    assert_eq!(b.http.posts(), posts + 1);
    assert!(r2.retakes.is_empty(), "{:?}", r2.retakes);
    assert!(pack.join("SOUNDS/en/lowrssi.wav").is_file());
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(pack.join("pack.json")).unwrap()).unwrap();
    assert!(m["retakes"].as_array().unwrap().is_empty());
    assert!(library(&b).retakes.is_empty());
    assert!(m["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f == "SOUNDS/en/lowrssi.wav"));
    assert!(m["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f == "SOUNDS/en/critbat.wav"));
}

#[test]
fn a_pack_named_for_its_voice_moves_to_the_voice_id_and_keeps_its_radios() {
    let b = bench(50000, true);
    let r = paid_render(
        &b,
        render("Callum", "eleven_turbo_v2_5", &["sample"], false, true),
    );
    let voices = b.dir.path().join("support/gear/voices");
    // Make it a pack from 0.12.0 or earlier: named for the voice's name, no voice id recorded.
    let legacy = "local-elevenlabs-callum-eleven-turbo-v2-5";
    std::fs::rename(voices.join(&r.pack), voices.join(legacy)).unwrap();
    let mf = voices.join(legacy).join("pack.json");
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(&mf).unwrap()).unwrap();
    m["id"] = legacy.into();
    m["voice_id"] = "".into();
    std::fs::write(&mf, m.to_string()).unwrap();
    let gear = b.dir.path().join("support/gear/gear.json");
    let mut g: serde_json::Value = std::fs::read(&gear)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| json!({}));
    g["voice"] = json!({
        "chosen": {"radio-1": legacy, "radio-2": "en-demo-v1"},
        "overrides": {"radio-1": {"SOUNDS/en/armed.wav": {"kind": "pack", "pack": legacy}}},
    });
    std::fs::write(&gear, g.to_string()).unwrap();
    // Another voice of the same name does not take it.
    let e = b
        .core
        .gear_voice_estimate(&EstimateParams {
            sets: vec!["sample".into()],
            voice: "Matilda".into(),
            model: "eleven_turbo_v2_5".into(),
            ..Default::default()
        })
        .unwrap();
    assert!(e.estimate.chars > 0);
    // The same voice finds it: every batch is kept, nothing to pay.
    let posts = b.http.posts();
    let again = b
        .core
        .gear_voice_render(&render(
            "Callum",
            "eleven_turbo_v2_5",
            &["sample"],
            false,
            false,
        ))
        .unwrap();
    assert!(!again.needs_confirm, "{again:?}");
    assert_eq!(b.http.posts(), posts);
    assert_eq!(again.pack, r.pack);
    assert!(voices.join(&r.pack).join("pack.json").is_file());
    assert!(!voices.join(legacy).exists());
    let g: serde_json::Value = serde_json::from_slice(&std::fs::read(&gear).unwrap()).unwrap();
    assert_eq!(g["voice"]["chosen"]["radio-1"], r.pack.as_str());
    assert_eq!(g["voice"]["chosen"]["radio-2"], "en-demo-v1");
    assert_eq!(
        g["voice"]["overrides"]["radio-1"]["SOUNDS/en/armed.wav"]["pack"],
        r.pack.as_str()
    );
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(voices.join(&r.pack).join("pack.json")).unwrap())
            .unwrap();
    assert_eq!(m["id"], r.pack.as_str());
    assert_eq!(m["voice_id"], "v-callum");
}

#[test]
fn samples_of_two_lines_with_one_file_name_keep_both() {
    let b = bench(50000, true);
    let mut p = sample(&["Callum"], &["eleven_turbo_v2_5"], false, false);
    p.sets = vec!["sample".into(), "scripts".into()];
    p.lines = vec![
        "SOUNDS/en/armed.wav".into(),
        "SOUNDS/en/SCRIPTS/YAAPU/armed.wav".into(),
    ];
    let done = paid_sample(&b, p);
    assert_eq!(done.items.len(), 2, "{:?}", done.items);
    let (a, y) = (&done.items[0].file, &done.items[1].file);
    assert_ne!(a, y);
    assert!(y.ends_with("SOUNDS/en/SCRIPTS/YAAPU/armed.wav"), "{y}");
    assert_ne!(std::fs::read(a).unwrap(), std::fs::read(y).unwrap());
}

fn sets_view(b: &Bench, voice: &str, model: &str) -> quadcam_lib::core::StudioView {
    b.core
        .gear_voice_sets(&quadcam_lib::core::SetsParams {
            voice: voice.into(),
            model: model.into(),
            ..Default::default()
        })
        .unwrap()
}

fn rendered_of(v: &quadcam_lib::core::StudioView, id: &str) -> (u32, u32) {
    let s = v.sets.iter().find(|s| s.id == id).unwrap();
    (s.rendered.unwrap(), s.lines)
}

/// A line one render paid for is cut again from its take in every later render, whatever set
/// it comes in: the estimate prices only lines no take holds, and counts the rest as rendered.
#[test]
fn a_line_rendered_once_is_never_billed_again() {
    use quadcam_lib::gear::voice::{batch, lines::Spelling, sets};
    let b = bench(99_000, true);
    let m = "eleven_turbo_v2_5";
    let before = sets_view(&b, "v-callum", m);
    assert_eq!(rendered_of(&before, "quad").0, 0);
    let r = paid_render(&b, render("Callum", m, &["quad"], false, true));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let posts = b.http.posts();

    // Every quad line is rendered now; the heli set shares the radio's prompts, numbers and
    // units, and the extras share the EdgeTX prompts the quad set holds.
    let after = sets_view(&b, "v-callum", m);
    // The voice list counts the lines each voice and model have.
    let counts = &after.rendered;
    assert_eq!(counts.len(), 1, "{counts:?}");
    assert_eq!(
        (counts[0].voice.as_str(), counts[0].model.as_str()),
        ("v-callum", m)
    );
    assert!(counts[0].lines > 300 && counts[0].lines < 396, "{counts:?}");
    let (done, all) = rendered_of(&after, "quad");
    assert_eq!(done, all);
    let (heli_done, heli_all) = rendered_of(&after, "heli");
    assert!(
        heli_done > 200 && heli_done < heli_all,
        "{heli_done}/{heli_all}"
    );
    let (extras_done, _) = rendered_of(&after, "extras");
    assert!(extras_done >= 28, "{extras_done}");
    // Another voice has none.
    assert_eq!(rendered_of(&sets_view(&b, "v-matilda", m), "quad").0, 0);

    // Adding the extras prices only their own lines: an extras line that says what a quad line
    // says (at another path) is rendered too.
    let both = estimate(&b, &["quad", "extras"], m).unwrap();
    assert_eq!(both.rendered + both.to_render + both.retakes, both.lines);
    let sp = Spelling::builtin().unwrap();
    let quad_said: std::collections::HashSet<String> = sets::lines_of("quad")
        .unwrap()
        .iter()
        .map(|l| sp.apply(&l.line.text))
        .collect();
    let new: std::collections::HashSet<String> = sets::lines_of("extras")
        .unwrap()
        .iter()
        .map(|l| sp.apply(&l.line.text))
        .filter(|t| !quad_said.contains(t))
        .collect();
    assert_eq!(both.estimate.lines as usize, new.len());
    let same_text = sets::combine(&["quad".into(), "extras".into()], &[])
        .unwrap()
        .iter()
        .filter(|l| quad_said.contains(&sp.apply(&l.line.text)))
        .count();
    assert_eq!(both.rendered as usize, same_text);
    assert!(both.rendered > all, "{} {all}", both.rendered);
    // Planned the old way, as whole batches keyed by their text, the last part-full batch of
    // each tone takes extras lines and every quad line in it would be billed again.
    let items: Vec<batch::Item> = sets::combine(&["quad".into(), "extras".into()], &[])
        .unwrap()
        .into_iter()
        .map(|l| batch::Item {
            spoken: sp.apply(&l.line.text),
            tone: l.tone,
        })
        .collect();
    let whole = batch::plan_within(&items, &Default::default(), 0);
    let quad_in_new_batches: usize = whole
        .iter()
        .filter(|w| w.items.iter().any(|(i, _)| new.contains(&i.spoken)))
        .flat_map(|w| w.items.iter())
        .filter(|(i, _)| quad_said.contains(&i.spoken))
        .count();
    assert!(quad_in_new_batches > 0, "{quad_in_new_batches}");

    // Rendering the helicopter set sends only the helicopter's own lines.
    let heli = estimate(&b, &["heli"], m).unwrap();
    assert_eq!(heli.rendered, heli_done);
    assert_eq!(heli.to_render, heli_all - heli_done);
    let r = paid_render(&b, render("Callum", m, &["heli"], false, true));
    assert_eq!(b.http.posts() - posts, heli.estimate.batches as usize);
    assert!(
        r.notes.iter().any(|n| n.contains("from earlier takes")),
        "{:?}",
        r.notes
    );
    let pack = b.dir.path().join("support/gear/voices").join(&r.pack);
    for f in [
        "SOUNDS/en/idleu1.wav",
        "SOUNDS/en/turtle.wav",
        "SOUNDS/en/SYSTEM/hello.wav",
    ] {
        assert!(pack.join(f).is_file(), "{f}");
    }
    let posts = b.http.posts();

    // The sample's lines are all quad lines: sampling this voice and model is free now.
    let s = b
        .core
        .gear_voice_sample(&sample(&["Callum"], &[m], false, false))
        .unwrap();
    assert!(!s.needs_confirm, "{s:?}");
    assert_eq!(s.estimate.credits, 0);
    assert_eq!(s.items.len(), 12);
    assert_eq!(b.http.posts(), posts);
    let quad_again = estimate(&b, &["quad", "heli"], m).unwrap();
    assert_eq!(quad_again.to_render, 0);
    assert_eq!(quad_again.estimate.chars, 0);
}

/// A sample's takes count too: a render after it does not send the sampled lines again.
#[test]
fn a_render_after_a_sample_reuses_the_sampled_lines() {
    let b = bench(99_000, true);
    let m = "eleven_turbo_v2_5";
    let cold = estimate(&b, &["quad"], m).unwrap();
    paid_sample(&b, sample(&["Callum"], &[m], false, true));
    let warm = estimate(&b, &["quad"], m).unwrap();
    // Every sample line is a quad line, some at more than one path.
    assert!(warm.rendered >= 12, "{warm:?}");
    assert!(warm.estimate.chars < cold.estimate.chars);
    let r = paid_render(&b, render("Callum", m, &["quad"], false, true));
    assert!(
        r.notes
            .iter()
            .any(|n| n.contains("come from earlier takes")),
        "{:?}",
        r.notes
    );
    let pack = b.dir.path().join("support/gear/voices").join(&r.pack);
    assert!(pack.join("SOUNDS/en/SYSTEM/0006.wav").is_file());
}

/// The sets the studio shows, in order: every set but `sample`, and `custom` only with lines.
fn shown_sets(v: &quadcam_lib::core::StudioView) -> Vec<String> {
    v.sets
        .iter()
        .filter(|x| x.id != "sample" && !(x.id == "custom" && x.lines == 0))
        .map(|x| x.id.clone())
        .collect()
}

/// The profiles name the aircraft flown: quads by default, a helicopter adds its own. With
/// nothing saved, the studio ticks every set it shows.
#[test]
fn the_sets_follow_the_aircraft_flown_and_all_are_ticked_at_first() {
    let b = bench(50_000, true);
    let v = b.core.gear_voice_sets(&Default::default()).unwrap();
    assert_eq!(v.aircraft, ["quad"]);
    assert_eq!(v.picks, shown_sets(&v));
    assert!(v.picks.iter().any(|p| p == "quadcam"), "{:?}", v.picks);
    assert!(!v.picks.iter().any(|p| p == "sample"), "{:?}", v.picks);
    assert!(v.sets.iter().all(|s| s.rendered.is_none()));
    let kinds = |id: &str| v.sets.iter().find(|s| s.id == id).unwrap().kinds.clone();
    assert_eq!(kinds("quad"), ["quad"]);
    assert_eq!(kinds("heli"), ["heli"]);
    assert!(kinds("easter").is_empty());
    assert_eq!(kinds("edgetx").len(), 5);
    for (name, aircraft) in [("Whoop", "65 mm whoop"), ("Goblin", "700 heli")] {
        b.core
            .profile_save(
                name,
                &serde_json::from_value(json!({"aircraft": aircraft})).unwrap(),
                None,
            )
            .unwrap();
    }
    let v = b.core.gear_voice_sets(&Default::default()).unwrap();
    assert_eq!(v.aircraft, ["quad", "heli"]);
    assert_eq!(v.picks, shown_sets(&v));
    let t = quadcam_lib::core::voice_sets_text(&v);
    assert!(
        t.contains("Aircraft flown (from the profiles): quad, heli"),
        "{t}"
    );
}

/// The studio's ticked sets live in the gear folder and come back; `quadcam` comes back
/// ticked even when the saved sets leave it out.
#[test]
fn the_ticked_sets_are_saved_and_quadcam_is_always_ticked() {
    let b = bench(50_000, true);
    let save = |sets: &[&str]| {
        b.core
            .gear_voice_save_sets(&quadcam_lib::core::SaveSetsParams {
                sets: sets.iter().map(|s| s.to_string()).collect(),
            })
            .unwrap()
    };
    save(&["easter", "quad", "quadcam"]);
    assert!(b
        .dir
        .path()
        .join("support/gear/voice-studio.json")
        .is_file());
    let v = b.core.gear_voice_sets(&Default::default()).unwrap();
    assert_eq!(v.picks, ["quad", "easter", "quadcam"]);
    save(&["extras"]);
    let v = b.core.gear_voice_sets(&Default::default()).unwrap();
    assert_eq!(v.picks, ["extras", "quadcam"]);
    // Nothing ticked is a choice too: only `quadcam` comes back.
    save(&[]);
    let v = b.core.gear_voice_sets(&Default::default()).unwrap();
    assert_eq!(v.picks, ["quadcam"]);
}

/// A flag from an older cut version does not count: the take is cut and checked again when it
/// is used, at no cost, and the new checks decide.
#[test]
fn a_cut_version_change_checks_reused_lines_again() {
    let b = bench(50_000, true);
    let m = "eleven_turbo_v2_5";
    b.http.mute("Signal low");
    paid_render(&b, render("Callum", m, &["quad"], false, true));
    assert_eq!(estimate(&b, &["quad"], m).unwrap().retakes, 2);
    let posts = b.http.posts();
    // Mark every take as cut by an older version.
    let dir = b.dir.path().join("cache/voice/lines/elevenlabs");
    for f in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = f.path();
        if p.extension().is_some_and(|x| x == "json") {
            let mut v: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            for e in v["batches"].as_array_mut().unwrap() {
                e["cut"] = 1.into();
            }
            std::fs::write(&p, v.to_string()).unwrap();
        }
    }
    let e = estimate(&b, &["quad"], m).unwrap();
    assert_eq!((e.retakes, e.to_render), (0, 0), "{e:?}");
    // The render cuts again from the cache: nothing is sent, and the silent cut is flagged anew.
    let r = b
        .core
        .gear_voice_render(&render("Callum", m, &["quad"], false, false))
        .unwrap();
    assert!(!r.needs_confirm);
    assert_eq!(r.retakes.len(), 2);
    assert_eq!(b.http.posts(), posts);
    assert_eq!(estimate(&b, &["quad"], m).unwrap().retakes, 2);
}

/// Writes one take as an older line-by-line render left it: a JSON beside raw PCM.
fn old_take(dir: &std::path::Path, name: &str, text: &str, voice: &str, speed: f64) {
    let d = dir.join(name);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("ab12_seed1.json"),
        json!({"path": format!("{name}.wav"), "text": text, "voice": voice, "model": "eleven_turbo_v2_5", "speed": speed, "seed": 1, "cost": 4}).to_string(),
    )
    .unwrap();
    // Half a second of a 220 Hz tone at 32 kHz, 16-bit mono.
    let pcm: Vec<u8> = (0..16_000)
        .flat_map(|i| {
            let v = (8000.0 * (i as f64 * 220.0 * std::f64::consts::TAU / 32_000.0).sin()) as i16;
            v.to_le_bytes()
        })
        .collect();
    std::fs::write(d.join("ab12_seed1.pcm"), pcm).unwrap();
}

/// Takes an older line-by-line render left count as rendered after an import, and a render at
/// their speed cuts them from the cache instead of sending their lines.
#[test]
fn imported_takes_count_as_rendered_and_are_never_sent_again() {
    let b = bench(50_000, true);
    let m = "eleven_turbo_v2_5";
    let old = tempfile::tempdir().unwrap();
    old_take(old.path(), "armed", "Armed", "v-callum", 1.2);
    old_take(old.path(), "disarm", "Disarmed", "v-callum", 1.2);
    std::fs::create_dir_all(old.path().join("bad")).unwrap();
    std::fs::write(old.path().join("bad/x.json"), "{}").unwrap();
    std::fs::write(old.path().join("bad/x.pcm"), [0u8; 4]).unwrap();
    let before: Vec<_> = std::fs::read_dir(old.path())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();

    let dry = b
        .core
        .gear_voice_import_takes(&quadcam_lib::core::ImportTakesParams {
            folder: old.path().display().to_string(),
            dry_run: true,
        })
        .unwrap();
    assert_eq!((dry.found, dry.imported, dry.already), (3, 2, 0), "{dry:?}");
    assert_eq!(dry.skipped.len(), 1);
    assert!(dry.counts.is_empty());
    let go = |b: &Bench| {
        b.core
            .gear_voice_import_takes(&quadcam_lib::core::ImportTakesParams {
                folder: old.path().display().to_string(),
                dry_run: false,
            })
            .unwrap()
    };
    let r = go(&b);
    assert_eq!(r.imported, 2);
    assert_eq!(r.counts.len(), 1);
    assert_eq!(
        (
            r.counts[0].voice.as_str(),
            r.counts[0].lines,
            r.counts[0].speed
        ),
        ("v-callum", 2, 1.2)
    );
    assert!(quadcam_lib::core::voice_import_text(&r).contains("2 imported"));
    // Again: nothing new. The folder is unchanged.
    assert_eq!(go(&b).already, 2);
    let after: Vec<_> = std::fs::read_dir(old.path())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    assert_eq!(before, after);

    // The voice list shows them; at the studio's speed 1 they are another take.
    let v = b.core.gear_voice_sets(&Default::default()).unwrap();
    assert_eq!(v.rendered[0].lines, 2);
    assert_eq!(estimate(&b, &["sample"], m).unwrap().rendered, 0);
    // At their speed, the sample set's "Armed" and "Disarmed" come from the import.
    let fast = Some(quadcam_lib::gear::voice::render::RenderSettings {
        speed: 1.2,
        ..Default::default()
    });
    let e = b
        .core
        .gear_voice_estimate(&EstimateParams {
            sets: vec!["sample".into()],
            voice: "Callum".into(),
            model: m.into(),
            settings: fast.clone(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(e.rendered, 2, "{e:?}");
    let mut p = render("Callum", m, &["sample"], false, false);
    p.settings = fast;
    let r = paid_render(&b, VoiceRenderParams { confirm: true, ..p });
    assert!(
        r.notes
            .iter()
            .any(|n| n.starts_with("2 lines come from earlier takes")),
        "{:?}",
        r.notes
    );
    let pack = b.dir.path().join("support/gear/voices").join(&r.pack);
    assert!(pack.join("SOUNDS/en/armed.wav").is_file());
    assert!(
        !r.retakes.iter().any(|t| t.text == "Armed"),
        "{:?}",
        r.retakes
    );
}

/// The files of every cached take of a test bench, by extension.
fn cached_takes(b: &Bench, ext: &str) -> Vec<std::path::PathBuf> {
    let dir = b.dir.path().join("cache/voice/batch/elevenlabs");
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == ext))
        .collect();
    v.sort();
    v
}

/// Every take the line index files of a test bench name.
fn indexed_keys(b: &Bench) -> Vec<String> {
    let dir = b.dir.path().join("cache/voice/lines/elevenlabs");
    let mut keys = Vec::new();
    for f in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = f.path();
        if p.extension().is_some_and(|x| x == "json") {
            let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            for e in v["batches"].as_array().unwrap() {
                keys.push(e["key"].as_str().unwrap().to_string());
            }
        }
    }
    keys
}

/// Rewrites every entry of every line index file of a test bench.
fn edit_index(b: &Bench, f: impl Fn(&mut serde_json::Value)) {
    let dir = b.dir.path().join("cache/voice/lines/elevenlabs");
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "json") {
            let mut v: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            for e in v["batches"].as_array_mut().unwrap() {
                f(e);
            }
            std::fs::write(&p, v.to_string()).unwrap();
        }
    }
}

/// A reused take that can no longer be read or cut is removed, from the cache and the line
/// index, before anything is sent; the next estimate prices its lines, and nothing loops.
#[test]
fn a_reused_take_that_cannot_be_read_or_cut_is_removed_and_priced_again() {
    let m = "eleven_turbo_v2_5";
    let unreadable = |b: &Bench| {
        for w in cached_takes(b, "wav") {
            std::fs::write(w, b"not a wav").unwrap();
        }
    };
    let uncuttable = |b: &Bench| {
        // A hand-edited index: the text no longer matches the take's timestamps.
        edit_index(b, |e| {
            let t = format!("{} Again.", e["text"].as_str().unwrap());
            e["text"] = t.into();
        })
    };
    for (what, spoil) in [
        ("unreadable", &unreadable as &dyn Fn(&Bench)),
        ("uncuttable", &uncuttable),
    ] {
        let b = bench(99_000, true);
        paid_sample(&b, sample(&["Callum"], &[m], false, true));
        assert!(!indexed_keys(&b).is_empty());
        spoil(&b);
        let posts = b.http.posts();
        // The estimate still counts every line as rendered: nothing to confirm.
        let e = estimate(&b, &["sample"], m).unwrap();
        assert_eq!((e.to_render, e.estimate.chars), (0, 0), "{what}: {e:?}");
        // The render (of another set, with lines of its own) fails before it sends anything.
        let ask = b
            .core
            .gear_voice_render(&render("Callum", m, &["quad"], false, false))
            .unwrap();
        assert!(ask.needs_confirm);
        let err = b
            .core
            .gear_voice_render(&VoiceRenderParams {
                digest: Some(ask.digest),
                ..render("Callum", m, &["quad"], false, true)
            })
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("so it was removed; a new estimate prices it again"),
            "{what}: {err:#}"
        );
        assert_eq!(b.http.posts(), posts, "{what}");
        // The takes are gone from the cache and the index; the estimate prices their lines.
        assert!(cached_takes(&b, "wav").is_empty(), "{what}");
        assert!(indexed_keys(&b).is_empty(), "{what}");
        let e = estimate(&b, &["sample"], m).unwrap();
        assert_eq!(e.to_render, e.lines, "{what}: {e:?}");
        // A render of that estimate works.
        let r = paid_render(&b, render("Callum", m, &["sample"], false, true));
        assert!(r.retakes.is_empty(), "{what}: {:?}", r.retakes);
        assert!(b.http.posts() > posts, "{what}");
    }
}

/// Batches a render made before a later batch failed are paid for: the line index holds
/// them, so a later plan that groups their lines otherwise does not send them again.
#[test]
fn batches_made_before_a_failed_render_count_as_rendered() {
    let b = bench(99_000, true);
    let m = "eleven_turbo_v2_5";
    b.http.fail_after(1);
    let ask = b
        .core
        .gear_voice_render(&render("Callum", m, &["quad"], false, false))
        .unwrap();
    assert!(ask.estimate.as_ref().unwrap().batches > 2);
    let err = b
        .core
        .gear_voice_render(&VoiceRenderParams {
            digest: Some(ask.digest),
            ..render("Callum", m, &["quad"], false, true)
        })
        .unwrap_err();
    assert_eq!(b.http.posts(), 2, "{err:#}");
    assert_eq!(cached_takes(&b, "wav").len(), 1);
    // Smaller batches group the quad lines otherwise: the batch made counts all the same.
    let e = b
        .core
        .gear_voice_estimate(&EstimateParams {
            sets: vec!["quad".into()],
            voice: "Callum".into(),
            model: m.into(),
            batch: Some(quadcam_lib::gear::voice::batch::BatchSettings {
                max_lines: 7,
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();
    assert!(e.rendered >= 7, "{e:?}");
}

/// One take file: a JSON of `meta` and a PCM of `pcm_len` bytes (a sparse file, so a long one
/// costs no disk).
fn take_file(dir: &std::path::Path, name: &str, meta: serde_json::Value, pcm_len: u64) {
    std::fs::write(dir.join(format!("{name}.json")), meta.to_string()).unwrap();
    let f = std::fs::File::create(dir.join(format!("{name}.pcm"))).unwrap();
    f.set_len(pcm_len).unwrap();
}

/// An import keeps a take only at a speed of 0.5 to 2, a rate of 8000 to 48000, and a PCM
/// of whole 16-bit samples, at most 10 minutes at 48 kHz.
#[test]
fn an_import_keeps_only_takes_within_its_bounds() {
    let b = bench(50_000, true);
    let old = tempfile::tempdir().unwrap();
    let max: u64 = 48_000 * 2 * 600;
    let take = |name: &str, speed: f64, rate: u32, len: u64| {
        take_file(
            old.path(),
            name,
            json!({"text": name, "voice": "v-callum", "model": "eleven_turbo_v2_5",
                   "speed": speed, "seed": 1, "rate": rate}),
            len,
        )
    };
    // At the bounds: kept.
    take("slow", 0.5, 8_000, 2);
    take("fast", 2.0, 48_000, max);
    // Past them: skipped, each with its reason.
    take("slower", 0.49, 32_000, 2);
    take("faster", 2.01, 32_000, 2);
    take("low", 1.0, 7_999, 2);
    take("high", 1.0, 48_001, 2);
    take("odd", 1.0, 32_000, 3);
    take("empty", 1.0, 32_000, 0);
    take("long", 1.0, 32_000, max + 2);
    let r = b
        .core
        .gear_voice_import_takes(&quadcam_lib::core::ImportTakesParams {
            folder: old.path().display().to_string(),
            dry_run: true,
        })
        .unwrap();
    assert_eq!((r.found, r.imported), (9, 2), "{r:?}");
    let why = |name: &str| {
        r.skipped
            .iter()
            .find(|s| s.starts_with(&format!("{name}.json")))
            .unwrap_or_else(|| panic!("{name}: {:?}", r.skipped))
            .clone()
    };
    for n in ["slower", "faster"] {
        assert!(why(n).contains("outside 0.5 to 2"), "{}", why(n));
    }
    for n in ["low", "high"] {
        assert!(why(n).contains("outside 8000 to 48000"), "{}", why(n));
    }
    for n in ["odd", "empty", "long"] {
        assert!(
            why(n).contains("empty, odd-sized or too long"),
            "{}",
            why(n)
        );
    }
    assert_eq!(r.skipped.len(), 7);
}

/// A take is reused only in the carrier sentence it was spoken in: another carrier sends the
/// line again, and the same carrier reuses it.
#[test]
fn a_take_is_reused_only_with_its_own_carrier() {
    let b = bench(99_000, true);
    let m = "eleven_turbo_v2_5";
    let say = quadcam_lib::gear::voice::batch::BatchSettings {
        carrier: "Say {line}.".into(),
        ..Default::default()
    };
    paid_render(
        &b,
        VoiceRenderParams {
            batch: Some(say.clone()),
            ..render("Callum", m, &["sample"], false, true)
        },
    );
    let with = |bs: Option<quadcam_lib::gear::voice::batch::BatchSettings>| {
        b.core
            .gear_voice_estimate(&EstimateParams {
                sets: vec!["sample".into()],
                voice: "Callum".into(),
                model: m.into(),
                batch: bs,
                ..Default::default()
            })
            .unwrap()
    };
    let same = with(Some(say));
    assert_eq!((same.rendered, same.to_render), (same.lines, 0), "{same:?}");
    let other = with(None);
    assert_eq!(
        (other.rendered, other.to_render),
        (0, other.lines),
        "{other:?}"
    );
    // The heli set reuses them only under the same carrier, too.
    let tone = quadcam_lib::gear::voice::batch::BatchSettings {
        tone_carriers: [("calm".to_string(), "Say {line}.".to_string())].into(),
        ..Default::default()
    };
    let part = with(Some(tone));
    assert!(part.rendered > 0 && part.rendered < part.lines, "{part:?}");
}

/// An imported take and a batch of one line under the carrier `{line}` never share a key: a
/// lost line index makes the batch priced and sent, and the imported take stays.
#[test]
fn an_imported_take_never_stands_in_for_a_batch() {
    let b = bench(50_000, true);
    let m = "eleven_turbo_v2_5";
    let old = tempfile::tempdir().unwrap();
    old_take(old.path(), "armed", "Armed", "v-callum", 1.0);
    b.core
        .gear_voice_import_takes(&quadcam_lib::core::ImportTakesParams {
            folder: old.path().display().to_string(),
            dry_run: false,
        })
        .unwrap();
    let imported = cached_takes(&b, "wav");
    assert_eq!(imported.len(), 1);
    std::fs::remove_dir_all(b.dir.path().join("cache/voice/lines")).unwrap();
    let bare = quadcam_lib::gear::voice::batch::BatchSettings {
        carrier: "{line}".into(),
        ..Default::default()
    };
    let mut p = render("Callum", m, &["sample"], false, false);
    p.lines = vec!["SOUNDS/en/armed.wav".into()];
    p.batch = Some(bare);
    p.settings = Some(quadcam_lib::gear::voice::render::RenderSettings {
        seed: 1,
        ..Default::default()
    });
    let ask = b.core.gear_voice_render(&p).unwrap();
    assert!(ask.needs_confirm, "{ask:?}");
    assert!(ask.plan.chars > 0);
    paid_render(&b, VoiceRenderParams { confirm: true, ..p });
    assert_eq!(b.http.posts(), 1);
    let now = cached_takes(&b, "wav");
    assert_eq!(now.len(), 2);
    assert!(now.contains(&imported[0]));
}

/// A take imported by 0.14.0 sits at the key a batch of its text has. Importing it again
/// moves it to its own key.
#[test]
fn an_import_moves_a_take_from_its_old_key() {
    use quadcam_lib::gear::voice::{batch::BatchCache, tts::Aligned, wav::Pcm};
    let b = bench(50_000, true);
    let old = tempfile::tempdir().unwrap();
    old_take(old.path(), "armed", "Armed", "v-callum", 1.2);
    let cache = BatchCache::new(&b.dir.path().join("cache"));
    let legacy = BatchCache::key(
        "elevenlabs",
        "v-callum",
        "eleven_turbo_v2_5",
        1.2,
        "Armed",
        1,
    );
    let a = Aligned {
        pcm: Pcm {
            rate: 32_000,
            samples: vec![1000; 16_000],
        },
        alignment: Default::default(),
        billed_chars: None,
    };
    cache
        .put(
            "elevenlabs",
            &legacy,
            "v-callum",
            "eleven_turbo_v2_5",
            1.2,
            1,
            "Armed",
            &a,
        )
        .unwrap();
    // A real one-line batch at that key (non-empty alignment) is never touched.
    let real = BatchCache::key(
        "elevenlabs",
        "v-callum",
        "eleven_turbo_v2_5",
        1.2,
        "Disarmed",
        1,
    );
    let mut aligned = a.clone();
    aligned.alignment.characters = vec!["D".into()];
    aligned.alignment.starts = vec![0.0];
    aligned.alignment.ends = vec![0.1];
    cache
        .put(
            "elevenlabs",
            &real,
            "v-callum",
            "eleven_turbo_v2_5",
            1.2,
            1,
            "Disarmed",
            &aligned,
        )
        .unwrap();
    old_take(old.path(), "disarm", "Disarmed", "v-callum", 1.2);
    let r = b
        .core
        .gear_voice_import_takes(&quadcam_lib::core::ImportTakesParams {
            folder: old.path().display().to_string(),
            dry_run: false,
        })
        .unwrap();
    assert_eq!((r.imported, r.already), (1, 1), "{r:?}");
    assert!(!cache.has("elevenlabs", &legacy));
    assert!(cache.has("elevenlabs", &real));
    let alone = BatchCache::alone_key(
        "elevenlabs",
        "v-callum",
        "eleven_turbo_v2_5",
        1.2,
        "Armed",
        1,
    );
    assert_eq!(
        cache.get("elevenlabs", &alone).unwrap().pcm.samples,
        a.pcm.samples
    );
    assert!(indexed_keys(&b).contains(&alone));
}

/// An imported take was spoken alone: a breath or hiss before its first word is not a
/// carrier's last sound, and the take is not flagged for one.
#[test]
fn an_imported_take_is_not_judged_for_a_carrier() {
    let b = bench(50_000, true);
    let m = "eleven_turbo_v2_5";
    let old = tempfile::tempdir().unwrap();
    old_take(old.path(), "armed", "Armed", "v-callum", 1.2);
    // 60 ms of loud hiss, 40 ms of quiet, then the word.
    let mut seed: u32 = 7;
    let mut v: Vec<i16> = (0..32 * 60)
        .map(|_| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            ((seed >> 16) as i32 % 12_000 - 6_000) as i16
        })
        .collect();
    v.extend(std::iter::repeat_n(0i16, 32 * 40));
    v.extend(
        (0..16_000)
            .map(|i| (8000.0 * (i as f64 * 220.0 * std::f64::consts::TAU / 32_000.0).sin()) as i16),
    );
    let pcm: Vec<u8> = v.iter().flat_map(|s| s.to_le_bytes()).collect();
    std::fs::write(old.path().join("armed/ab12_seed1.pcm"), pcm).unwrap();
    b.core
        .gear_voice_import_takes(&quadcam_lib::core::ImportTakesParams {
            folder: old.path().display().to_string(),
            dry_run: false,
        })
        .unwrap();
    let mut p = render("Callum", m, &["sample"], false, false);
    p.lines = vec!["SOUNDS/en/armed.wav".into()];
    p.settings = Some(quadcam_lib::gear::voice::render::RenderSettings {
        speed: 1.2,
        ..Default::default()
    });
    let r = b.core.gear_voice_render(&p).unwrap();
    assert!(!r.needs_confirm, "{r:?}");
    assert!(r.retakes.is_empty(), "{:?}", r.retakes);
    assert_eq!(b.http.posts(), 0);
}

/// A bench whose speech check hears every line right except the card paths in `wrong`,
/// where it hears "Battery.".
fn bench_hearing(
    wrong: Arc<std::sync::Mutex<Vec<String>>>,
) -> (Bench, Arc<quadcam_lib::intel::fake::FakeHelper>) {
    let mut texts: Vec<(String, String)> = Vec::new();
    for id in quadcam_lib::gear::voice::sets::ids() {
        if id == "custom" {
            continue;
        }
        for l in quadcam_lib::gear::voice::sets::lines_of(id).unwrap() {
            texts.push((l.line.path, l.line.text));
        }
    }
    let helper = Arc::new(quadcam_lib::intel::fake::FakeHelper::new(Box::new(
        move |f: &str| {
            if wrong
                .lock()
                .unwrap()
                .iter()
                .any(|w| f.ends_with(w.as_str()))
            {
                return Some("Battery.".into());
            }
            texts
                .iter()
                .find(|(p, _)| f.ends_with(p.as_str()))
                .map(|(_, t)| format!("{t}."))
        },
    )));
    let mut b = bench(99_000, true);
    let intel = quadcam_lib::intel::Intel::with("/helper/quadcam-intel".into(), helper.clone());
    b.core = b.core.with_intel(Arc::new(intel));
    (b, helper)
}

fn check(b: &Bench, pack: &str, accept: &[&str]) -> quadcam_lib::core::VoiceCheckReport {
    b.core
        .gear_voice_check(&quadcam_lib::core::VoiceCheckParams {
            pack: pack.into(),
            accept: accept.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        })
        .unwrap()
}

#[test]
fn a_line_the_speech_check_hears_wrong_is_a_re_take_until_accepted() {
    const LOW: &str = "SOUNDS/en/lowbat.wav";
    let wrong = Arc::new(std::sync::Mutex::new(vec![LOW.to_string()]));
    let (b, helper) = bench_hearing(wrong.clone());
    let m = "eleven_turbo_v2_5";
    let r = paid_render(&b, render("Callum", m, &["quad"], false, true));
    // The check ran after the render, on every line it put in the pack.
    assert!(r.heard.len() > 10, "{r:?}");
    let low = r.heard.iter().find(|h| h.path == LOW).unwrap();
    assert!(!low.ok && !low.accepted);
    assert_eq!(low.heard, "Battery.");
    assert!(r.heard.iter().filter(|h| h.path != LOW).all(|h| h.ok));
    let retake = r.retakes.iter().find(|t| t.path == LOW).expect("a re-take");
    assert_eq!(retake.reason, "The speech check heard \"Battery.\".");
    let pack = b.dir.path().join("support/gear/voices").join(&r.pack);
    assert!(!pack.join(LOW).exists());
    let held = b
        .dir
        .path()
        .join("cache/voice/held")
        .join(&r.pack)
        .join(LOW);
    assert!(held.is_file());
    // The library shows what was heard per line.
    let k = b
        .core
        .gear_voice(&Default::default())
        .unwrap()
        .packs
        .into_iter()
        .find(|k| k.id == r.pack)
        .unwrap();
    assert!(k.heard.iter().any(|h| h.path == LOW && !h.ok));
    assert!(k.retakes.iter().any(|t| t.path == LOW));
    let posts = b.http.posts();

    // The same render again costs nothing and keeps the line out, without hearing it again.
    helper.calls.lock().unwrap().clear();
    let again = b
        .core
        .gear_voice_render(&render("Callum", m, &["quad"], false, false))
        .unwrap();
    assert!(!again.needs_confirm);
    assert_eq!(b.http.posts(), posts);
    assert!(again.retakes.iter().any(|t| t.path == LOW), "{again:?}");
    assert!(!helper.heard_files().iter().any(|f| f.ends_with(LOW)));
    // An estimate with another seed prices only the batch that holds the line.
    let est = b
        .core
        .gear_voice_estimate(&EstimateParams {
            sets: vec!["quad".into()],
            voice: "Callum".into(),
            model: m.into(),
            settings: Some(quadcam_lib::gear::voice::render::RenderSettings {
                seed: 7,
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(est.estimate.batches, 1, "{est:?}");
    assert!(est.retakes == 0 && est.to_render >= 1, "{est:?}");

    // Accepted, the line goes back into the pack and a render keeps it there for free.
    let a = check(&b, &r.pack, &[LOW]);
    assert!(a.lines[0].accepted);
    assert!(pack.join(LOW).is_file());
    assert!(!held.exists());
    let again = b
        .core
        .gear_voice_render(&render("Callum", m, &["quad"], false, false))
        .unwrap();
    assert!(!again.retakes.iter().any(|t| t.path == LOW), "{again:?}");
    assert!(pack.join(LOW).is_file());
    assert_eq!(b.http.posts(), posts);
    let k = b
        .core
        .gear_voice(&Default::default())
        .unwrap()
        .packs
        .into_iter()
        .find(|k| k.id == r.pack)
        .unwrap();
    assert!(
        k.heard.iter().any(|h| h.path == LOW && h.accepted),
        "{:?}",
        k.heard
    );

    // A check on request hears the pack again; a line heard right needs nothing.
    wrong.lock().unwrap().clear();
    let c = check(&b, &r.pack, &[]);
    assert!(c.available && c.mismatches == 0, "{c:?}");
    assert!(c.lines.iter().all(|h| h.ok));
    let text = quadcam_lib::core::voice_check_text(&c);
    assert!(text.contains("0 heard wrong"), "{text}");
}

#[test]
fn a_sample_heard_wrong_warns_and_its_line_is_a_re_take() {
    const SIX: &str = "SOUNDS/en/SYSTEM/0006.wav";
    let wrong = Arc::new(std::sync::Mutex::new(vec![SIX.to_string()]));
    let (b, _) = bench_hearing(wrong);
    let m = "eleven_turbo_v2_5";
    let s = paid_sample(&b, sample(&["Callum"], &[m], false, true));
    let six = s.items.iter().find(|i| i.line == SIX).unwrap();
    assert_eq!(six.heard.as_deref(), Some("Battery."));
    assert_eq!(six.heard_ok, Some(false));
    assert!(s
        .items
        .iter()
        .filter(|i| i.line != SIX)
        .all(|i| i.heard_ok == Some(true)));
    assert!(
        s.warnings
            .iter()
            .any(|w| w.contains(SIX) && w.contains("heard")),
        "{:?}",
        s.warnings
    );
    // A render of the set with the same seed keeps the sampled line out at no cost.
    let est = estimate(&b, &["sample"], m).unwrap();
    assert!(est.retakes >= 1, "{est:?}");
}

#[test]
fn without_the_helper_the_check_is_unavailable_and_renders_go_on() {
    let b = bench(99_000, true);
    let r = paid_render(
        &b,
        render("Callum", "eleven_turbo_v2_5", &["sample"], false, true),
    );
    assert!(r.heard.is_empty());
    let c = check(&b, &r.pack, &[]);
    assert!(!c.available);
    assert!(c.reason.unwrap().contains("helper"));
    let v = b.core.gear_voice(&Default::default()).unwrap();
    assert!(!v.speech.available);
    assert!(quadcam_lib::core::voice_view_text(&v).contains("Speech check: unavailable"));
    // Accepting a line the check never heard is refused.
    let e = b
        .core
        .gear_voice_check(&quadcam_lib::core::VoiceCheckParams {
            pack: r.pack.clone(),
            accept: vec!["SOUNDS/en/armed.wav".into()],
            ..Default::default()
        })
        .unwrap_err();
    assert!(e.to_string().contains("has not heard"), "{e}");
    assert!(b
        .core
        .gear_voice_check(&quadcam_lib::core::VoiceCheckParams {
            pack: "no-such-pack".into(),
            ..Default::default()
        })
        .is_err());
}

fn packs(voices: &[&str], models: &[&str], sets: &[&str]) -> RenderPacksParams {
    RenderPacksParams {
        voices: voices.iter().map(|s| s.to_string()).collect(),
        models: models.iter().map(|s| s.to_string()).collect(),
        sets: sets.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

#[test]
fn several_voices_price_as_one_plan_and_render_one_pack_each() {
    let b = bench(99_000, true);
    let m = "eleven_turbo_v2_5";
    let p = packs(&["Callum", "Matilda"], &[m], &["sample"]);
    let dry = b
        .core
        .gear_voice_render_packs(&RenderPacksParams {
            dry_run: true,
            ..p.clone()
        })
        .unwrap();
    assert_eq!(dry.pairs.len(), 2);
    assert_eq!(dry.pairs[0].voice_name, "Callum");
    assert_eq!(dry.pairs[1].voice_name, "Matilda");
    assert_eq!(dry.lines, 12);
    let chars: u64 = dry.pairs.iter().map(|x| x.estimate.chars).sum();
    let credits: u64 = dry.pairs.iter().map(|x| x.estimate.credits).sum();
    assert!(chars > 0);
    assert_eq!((dry.estimate.chars, dry.estimate.credits), (chars, credits));
    assert_eq!(dry.estimate.usd_per_1k, dry.pairs[0].estimate.usd_per_1k);
    assert_eq!(b.http.posts(), 0);
    // The question: nothing sent until the confirm repeats the digest.
    let ask = b.core.gear_voice_render_packs(&p).unwrap();
    assert!(ask.needs_confirm && ask.packs.is_empty());
    assert_eq!(ask.digest, dry.digest);
    let e = b
        .core
        .gear_voice_render_packs(&RenderPacksParams {
            confirm: true,
            ..p.clone()
        })
        .unwrap_err();
    assert!(e.to_string().contains("digest"), "{e}");
    let e = b
        .core
        .gear_voice_render_packs(&RenderPacksParams {
            confirm: true,
            digest: Some("0".repeat(64)),
            ..p.clone()
        })
        .unwrap_err();
    assert!(e.to_string().contains("estimate again"), "{e}");
    assert_eq!(b.http.posts(), 0);
    let done = b
        .core
        .gear_voice_render_packs(&RenderPacksParams {
            confirm: true,
            digest: Some(ask.digest),
            ..p.clone()
        })
        .unwrap();
    assert!(!done.stopped && done.error.is_none(), "{done:?}");
    assert_eq!(done.packs.len(), 2);
    assert_ne!(done.packs[0].pack, done.packs[1].pack);
    for r in &done.packs {
        assert!(b
            .dir
            .path()
            .join("support/gear/voices")
            .join(&r.pack)
            .join("pack.json")
            .is_file());
    }
    // Every line has a take now: the same plan costs nothing.
    let again = b
        .core
        .gear_voice_render_packs(&RenderPacksParams { dry_run: true, ..p })
        .unwrap();
    assert_eq!(again.estimate.chars, 0);
    assert!(again.pairs.iter().all(|x| x.to_render == 0));
}

#[test]
fn a_failed_pack_keeps_the_packs_made_before_it() {
    let b = bench(99_000, true);
    let m = "eleven_turbo_v2_5";
    let p = packs(&["Callum", "Matilda"], &[m], &["sample"]);
    let ask = b.core.gear_voice_render_packs(&p).unwrap();
    b.http.fail_after(ask.pairs[0].estimate.batches as usize);
    let r = b
        .core
        .gear_voice_render_packs(&RenderPacksParams {
            confirm: true,
            digest: Some(ask.digest),
            ..p
        })
        .unwrap();
    assert_eq!(r.packs.len(), 1);
    assert_eq!(r.packs[0].voice, "Callum");
    let e = r.error.unwrap();
    assert!(e.starts_with("Matilda"), "{e}");
}

/// Stops the run on the first batch it hears of.
#[derive(Default)]
struct StopOnFirst {
    core: std::sync::OnceLock<std::sync::Weak<Core>>,
    seen: std::sync::Mutex<Vec<VoiceProgress>>,
}

impl Hooks for StopOnFirst {
    fn event(&self, e: Event) {
        if let Event::VoiceProgress(p) = e {
            if p.done == 1 {
                if let Some(c) = self.core.get().and_then(|w| w.upgrade()) {
                    assert!(c.gear_stop(quadcam_lib::core::STUDIO_JOB));
                }
            }
            self.seen.lock().unwrap().push(p);
        }
    }
}

#[test]
fn a_stop_ends_a_render_after_the_current_batch_and_reports_progress() {
    let hooks = Arc::new(StopOnFirst::default());
    let b = bench_with(99_000, true, hooks.clone());
    let core = Arc::new(b.core);
    hooks.core.set(Arc::downgrade(&core)).unwrap();
    let m = "eleven_turbo_v2_5";
    let p = packs(&["Callum", "Matilda"], &[m], &["sample"]);
    let ask = core.gear_voice_render_packs(&p).unwrap();
    let total = ask.estimate.batches;
    assert!(total > 2);
    let r = core
        .gear_voice_render_packs(&RenderPacksParams {
            confirm: true,
            digest: Some(ask.digest),
            ..p.clone()
        })
        .unwrap();
    assert!(r.stopped, "{r:?}");
    assert!(r.packs.is_empty());
    assert_eq!(b.http.posts(), 1);
    let seen = hooks.seen.lock().unwrap().clone();
    assert!(seen[0].running && seen[0].total == total && seen[0].kind == "render");
    assert!(seen
        .iter()
        .any(|e| e.voice_name == "Callum" && e.model == m));
    assert!(seen.iter().any(|e| e.stopping));
    assert!(!seen.last().unwrap().running);
    // The job is gone, and the batch made before the stop is not billed again.
    assert!(core.gear_jobs().is_empty());
    let dry = core
        .gear_voice_render_packs(&RenderPacksParams { dry_run: true, ..p })
        .unwrap();
    assert_eq!(dry.estimate.batches, total - 1);
    assert!(dry.estimate.chars < ask.estimate.chars);
    assert!(dry.pairs[0].rendered > 0, "{:?}", dry.pairs[0]);
}

#[test]
fn a_sample_reports_progress_and_stops_after_the_current_batch() {
    let hooks = Arc::new(StopOnFirst::default());
    let b = bench_with(99_000, true, hooks.clone());
    let core = Arc::new(b.core);
    hooks.core.set(Arc::downgrade(&core)).unwrap();
    let p = sample(&["Callum", "Matilda"], &["eleven_turbo_v2_5"], false, false);
    let ask = core.gear_voice_sample(&p).unwrap();
    let r = core
        .gear_voice_sample(&SampleParams {
            confirm: true,
            digest: Some(ask.digest),
            ..p
        })
        .unwrap();
    assert!(r.stopped);
    assert_eq!(b.http.posts(), 1);
    let seen = hooks.seen.lock().unwrap().clone();
    assert_eq!(seen[0].kind, "sample");
    assert_eq!(seen[0].total, ask.estimate.batches);
    assert!(!seen.last().unwrap().running);
}

const LIBRARY: &str = r#"{"voices":[
  {"public_owner_id":"own1","voice_id":"lib-ranger","name":"Ranger","accent":"british","gender":"male","age":"middle_aged",
   "use_case":"narrative_story","category":"professional","language":"en","rate":2.0,"preview_url":"https://example.invalid/p.mp3",
   "verified_languages":[{"language":"en","model_id":"eleven_v4","accent":"british"}]},
  {"public_owner_id":"own2","voice_id":"v-callum","name":"Callum","accent":"american","language":"en"}],
  "has_more":false,"total_count":2}"#;

#[test]
fn the_library_marks_the_voices_the_account_has_and_an_add_waits_for_confirm() {
    let b = bench(50_000, true);
    b.http.on("/v1/shared-voices", 200, LIBRARY);
    let p = b
        .core
        .gear_voice_library(&quadcam_lib::core::LibraryParams {
            query: quadcam_lib::gear::voice::tts::LibraryQuery {
                language: "en".into(),
                ..Default::default()
            },
            id: String::new(),
        })
        .unwrap();
    assert_eq!(p.voices.len(), 2);
    assert!(!p.voices[0].added);
    assert!(p.voices[1].added, "Callum is in the account");
    assert_eq!(p.voices[0].rate, Some(2.0));
    // Add by id: the account does not answer for a library voice, so the library does.
    b.http.on_first("/v1/voices/lib-ranger", 404, "{}");
    let add = |confirm| {
        b.core.gear_voice_add(&quadcam_lib::core::VoiceAddParams {
            voice: "lib-ranger".into(),
            name: "Ranger UK".into(),
            confirm,
            ..Default::default()
        })
    };
    let ask = add(false).unwrap();
    assert!(ask.needs_confirm && ask.added_id.is_none());
    assert_eq!(ask.voice.public_owner_id, "own1");
    assert_eq!(b.http.posts(), 0);
    b.http.on_first(
        "/v1/voices/add/own1/lib-ranger",
        200,
        r#"{"voice_id":"lib-ranger"}"#,
    );
    let done = add(true).unwrap();
    assert_eq!(done.added_id.as_deref(), Some("lib-ranger"));
    let seen = b.http.seen.lock().unwrap().clone();
    let post = seen.iter().find(|r| r.method == "POST").unwrap();
    assert!(post.url.ends_with("/v1/voices/add/own1/lib-ranger"));
    assert!(post.body.as_deref().unwrap().contains("Ranger UK"));
    // A voice the account has already is not added again.
    let again = b
        .core
        .gear_voice_add(&quadcam_lib::core::VoiceAddParams {
            voice: "v-callum".into(),
            confirm: true,
            ..Default::default()
        })
        .unwrap();
    assert!(again.already && !again.needs_confirm);
    assert_eq!(b.http.posts(), 1);
}

#[test]
fn a_voice_with_a_rate_of_its_own_costs_that_many_times_the_credits() {
    let b = bench(99_000, true);
    let m = "eleven_turbo_v2_5";
    let plain = estimate(&b, &["sample"], m).unwrap();
    assert_eq!(plain.rate, None);
    // Callum now bills twice the credits (a library voice with a rate of its own).
    b.http.on_first(
        "/v1/voices",
        200,
        r#"{"voices":[{"voice_id":"v-callum","name":"Callum","category":"professional","sharing":{"rate":2.0}}]}"#,
    );
    let twice = estimate(&b, &["sample"], m).unwrap();
    assert_eq!(twice.rate, Some(2.0));
    assert_eq!(twice.estimate.chars, plain.estimate.chars);
    assert_eq!(
        twice.estimate.credits,
        (plain.estimate.chars as f64 * 0.11 * 2.0).ceil() as u64
    );
    assert!((twice.estimate.usd - 2.0 * plain.estimate.usd).abs() < 1e-9);
}

#[test]
fn a_preview_plays_from_the_cache_and_comes_only_over_https() {
    let b = bench(50_000, true);
    let p = |url: &str| {
        b.core
            .gear_voice_library_preview(&quadcam_lib::core::LibraryPreviewParams {
                url: url.into(),
            })
    };
    assert!(p("http://example.invalid/a.mp3").is_err());
    assert!(p("file:///etc/passwd").is_err());
    // A preview fetched once plays from the cache; under cargo nothing reaches the internet.
    let url = "https://example.invalid/voices/abc/preview.mp3?x=1";
    assert!(p(url).is_err());
    let dir = b.dir.path().join("cache/voice/library-previews");
    let name = format!(
        "{}.mp3",
        &quadcam_lib::gear::voice::sha256_hex(url.as_bytes())[..16]
    );
    std::fs::write(dir.join(&name), b"ID3").unwrap();
    assert_eq!(p(url).unwrap(), dir.join(&name).display().to_string());
}

// ----- the sample gallery and flags -----

fn gallery(b: &Bench) -> quadcam_lib::core::SampleGallery {
    b.core.gear_voice_samples().unwrap()
}

fn flag(
    b: &Bench,
    sample: &str,
    pack: &str,
    line: &str,
    reason: &str,
) -> quadcam_lib::core::SampleVoice {
    b.core
        .gear_voice_flag(&quadcam_lib::core::FlagParams {
            sample: sample.into(),
            pack: pack.into(),
            line: line.into(),
            reason: reason.into(),
            note: "sounds like a sneeze".into(),
            clear: false,
        })
        .unwrap()
}

#[test]
fn samples_stay_in_the_gear_folder_by_voice_until_deleted() {
    let b = bench(50000, true);
    let m = "eleven_turbo_v2_5";
    let done = paid_sample(&b, sample(&["Callum", "Matilda"], &[m], false, true));
    let root = b.dir.path().join("support/gear/voice-samples");
    assert!(done
        .items
        .iter()
        .all(|i| i.file.starts_with(root.to_str().unwrap())));
    assert!(!b.dir.path().join("cache/voice/samples").exists());
    let g = gallery(&b);
    let cards: Vec<_> = g.voices.iter().filter(|v| v.kind == "sample").collect();
    assert_eq!(cards.len(), 2, "{g:?}");
    let callum = cards.iter().find(|v| v.voice_name == "Callum").unwrap();
    assert_eq!(callum.voice, "v-callum");
    assert_eq!(callum.model, m);
    assert_eq!(callum.speed, 1.0);
    assert_eq!(callum.description, "A steady voice.");
    assert_eq!(callum.accent, "american");
    assert_eq!(callum.sets, ["sample"]);
    assert_eq!(callum.takes.len(), 12);
    assert!(callum
        .takes
        .iter()
        .all(|t| t.seed == Some(0) && std::path::Path::new(&t.file).is_file()));
    // A new core (a relaunch) reads the same gallery from disk.
    let again = bench_at(&b);
    assert_eq!(gallery(&again).voices, g.voices);
    // Another speed is its own card.
    let mut fast = sample(&["Callum"], &[m], false, true);
    fast.settings = Some(quadcam_lib::gear::voice::render::RenderSettings {
        speed: 1.1,
        ..Default::default()
    });
    paid_sample(&b, fast);
    assert_eq!(
        gallery(&b)
            .voices
            .iter()
            .filter(|v| v.kind == "sample")
            .count(),
        3
    );
    // Delete one, then all; the takes stay in the cache, so sampling again is free.
    let one = b
        .core
        .gear_voice_samples_delete(&quadcam_lib::core::SamplesDeleteParams {
            ids: vec![callum.id.clone()],
            all: false,
        })
        .unwrap();
    assert!(!one.voices.iter().any(|v| v.id == callum.id));
    assert!(b
        .core
        .gear_voice_samples_delete(&quadcam_lib::core::SamplesDeleteParams {
            ids: vec!["../voices".into()],
            all: false,
        })
        .is_err());
    let none = b
        .core
        .gear_voice_samples_delete(&quadcam_lib::core::SamplesDeleteParams {
            ids: vec![],
            all: true,
        })
        .unwrap();
    assert!(none.voices.is_empty());
    let posts = b.http.posts();
    let free = b
        .core
        .gear_voice_sample(&sample(&["Callum"], &[m], false, false))
        .unwrap();
    assert!(!free.needs_confirm);
    assert_eq!(b.http.posts(), posts);
    assert_eq!(gallery(&b).voices.len(), 1);
}

/// A second core on the same folders, as after a relaunch.
fn bench_at(b: &Bench) -> Bench {
    let http = b.http.clone();
    let mut env = quadcam_lib::gear::Env::fake(vec![], Arc::new(FakePorts::new(vec![])));
    env.tts = Arc::new(Fake(http.clone()));
    env.keys = b.keys.clone();
    let core = Core::new(
        b.dir.path().join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(b.dir.path().join("support/settings.json"))
    .with_gear_env(env);
    Bench {
        core,
        http,
        keys: b.keys.clone(),
        dir: tempfile::tempdir().unwrap(),
    }
}

#[test]
fn samples_in_the_cache_move_to_the_gear_folder_once() {
    let b = bench(50000, true);
    let m = "eleven_turbo_v2_5";
    // A 0.19 sample: no list beside it, named for the voice's name and the model.
    let old = b
        .dir
        .path()
        .join("cache/voice/samples/callum-eleven-turbo-v2-5/SOUNDS/en");
    std::fs::create_dir_all(&old).unwrap();
    let wav = [b"RIFF".as_slice(), &[0u8; 6440]].concat();
    std::fs::write(old.join("armed.wav"), &wav).unwrap();
    let g = gallery(&b);
    let card = &g.voices[0];
    // The name reads back from the folder's slug.
    assert_eq!(
        (card.voice_name.as_str(), card.model.as_str()),
        ("Callum", m)
    );
    assert_eq!(card.voice, "");
    assert_eq!(card.takes[0].text, "Armed");
    assert_eq!(card.takes[0].ms, 100);
    let moved = b
        .dir
        .path()
        .join("support/gear/voice-samples/callum-eleven-turbo-v2-5/SOUNDS/en/armed.wav");
    assert_eq!(std::fs::read(&moved).unwrap(), wav);
    assert!(!b.dir.path().join("cache/voice/samples").exists());
    // Flags work on it; a re-take needs the voice's id, which it lacks.
    flag(&b, &card.id, "", "SOUNDS/en/armed.wav", "noise");
    assert!(b
        .core
        .gear_voice_retake(&quadcam_lib::core::RetakeParams {
            sample: card.id.clone(),
            ..Default::default()
        })
        .is_err());
    // A sample of that voice and model takes the folder over: one card, the voice known.
    paid_sample(&b, sample(&["Callum"], &[m], false, true));
    let g = gallery(&b);
    let cards: Vec<_> = g.voices.iter().filter(|v| v.kind == "sample").collect();
    assert_eq!(cards.len(), 1, "{g:?}");
    assert_eq!(cards[0].voice, "v-callum");
    assert_eq!(cards[0].takes.len(), 12);
    // The new take of Armed replaced the old one, and its flag went with it.
    assert!(cards[0].takes.iter().all(|t| t.flag.is_none()));
}

#[test]
fn a_flagged_sample_is_listed_and_a_retake_makes_it_again() {
    let wrong = Arc::new(std::sync::Mutex::new(vec![
        "SOUNDS/en/SYSTEM/0006.wav".to_string()
    ]));
    let (b, _) = bench_hearing(wrong.clone());
    let m = "eleven_turbo_v2_5";
    paid_sample(&b, sample(&["Callum"], &[m], false, true));
    let card = gallery(&b).voices.remove(0);
    // The speech check's mismatch shows on the card.
    let six = card
        .takes
        .iter()
        .find(|t| t.line == "SOUNDS/en/SYSTEM/0006.wav")
        .unwrap();
    assert_eq!(six.heard_ok, Some(false));
    assert_eq!(six.heard.as_deref(), Some("Battery."));
    assert!(b
        .core
        .gear_voice_flag(&quadcam_lib::core::FlagParams {
            sample: card.id.clone(),
            line: "SOUNDS/en/armed.wav".into(),
            reason: "loud".into(),
            ..Default::default()
        })
        .is_err());
    let after = flag(&b, &card.id, "", "SOUNDS/en/armed.wav", "mispronounced");
    let armed = after
        .takes
        .iter()
        .find(|t| t.line == "SOUNDS/en/armed.wav")
        .unwrap();
    let f = armed.flag.clone().unwrap();
    assert_eq!(
        (f.reason.as_str(), f.note.as_str()),
        ("mispronounced", "sounds like a sneeze")
    );
    let flags = b.core.gear_voice_flags().unwrap().flags;
    assert_eq!(flags.len(), 1);
    let row = &flags[0];
    assert_eq!(
        (
            row.kind.as_str(),
            row.voice.as_str(),
            row.voice_name.as_str(),
            row.model.as_str()
        ),
        ("sample", "v-callum", "Callum", m)
    );
    assert_eq!((row.speed, row.seed), (1.0, Some(0)));
    assert_eq!(
        (row.line.as_str(), row.text.as_str()),
        ("SOUNDS/en/armed.wav", "Armed")
    );
    assert_eq!(row.heard.as_deref(), Some("Armed."));
    assert!(row.retake && std::path::Path::new(&row.file).is_file());
    assert!(
        quadcam_lib::core::voice_flags_text(&b.core.gear_voice_flags().unwrap())
            .contains("mispronounced")
    );
    // The flagged take is a re-take now: the same sample again sends nothing and leaves it out.
    let posts = b.http.posts();
    let same = b
        .core
        .gear_voice_sample(&sample(&["Callum"], &[m], false, false))
        .unwrap();
    assert!(!same.needs_confirm);
    assert!(!same.items.iter().any(|i| i.line == "SOUNDS/en/armed.wav"));
    assert_eq!(b.http.posts(), posts);
    // A re-take prices the flagged line only, waits for confirm, then makes it again.
    let ask = |confirm: bool, digest: Option<String>| {
        b.core
            .gear_voice_retake(&quadcam_lib::core::RetakeParams {
                sample: card.id.clone(),
                confirm,
                digest,
                ..Default::default()
            })
            .unwrap()
    };
    let r = ask(false, None);
    assert!(r.needs_confirm && r.seed > 0, "{r:?}");
    assert_eq!(r.lines, ["SOUNDS/en/armed.wav"]);
    assert_eq!(r.estimate.as_ref().unwrap().batches, 1);
    assert_eq!(b.http.posts(), posts);
    assert!(b
        .core
        .gear_voice_retake(&quadcam_lib::core::RetakeParams {
            sample: card.id.clone(),
            confirm: true,
            digest: Some("0000".into()),
            ..Default::default()
        })
        .is_err());
    let done = ask(true, Some(r.digest.clone()));
    assert_eq!(done.cleared, ["SOUNDS/en/armed.wav"], "{done:?}");
    assert_eq!(b.http.posts(), posts + 1);
    let card = gallery(&b).voices.remove(0);
    let armed = card
        .takes
        .iter()
        .find(|t| t.line == "SOUNDS/en/armed.wav")
        .unwrap();
    assert!(armed.flag.is_none());
    assert_eq!(armed.seed, Some(r.seed));
    assert_eq!(card.takes.len(), 12);
    assert!(b.core.gear_voice_flags().unwrap().flags.is_empty());
    // Clear works on its own too.
    flag(&b, &card.id, "", "SOUNDS/en/turtle.wav", "wrong_tone");
    b.core
        .gear_voice_flag(&quadcam_lib::core::FlagParams {
            sample: card.id.clone(),
            line: "SOUNDS/en/turtle.wav".into(),
            clear: true,
            ..Default::default()
        })
        .unwrap();
    assert!(b.core.gear_voice_flags().unwrap().flags.is_empty());
    let posts = b.http.posts();
    let free = b
        .core
        .gear_voice_sample(&sample(&["Callum"], &[m], false, false))
        .unwrap();
    assert!(!free.needs_confirm);
    assert!(free.items.iter().any(|i| i.line == "SOUNDS/en/turtle.wav"));
    assert_eq!(b.http.posts(), posts);
}

#[test]
fn a_flagged_pack_line_shows_on_its_card_and_a_retake_puts_a_new_take_in() {
    let b = bench(99_000, true);
    let m = "eleven_turbo_v2_5";
    let r = paid_render(&b, render("Callum", m, &["sample"], false, true));
    const LINE: &str = "SOUNDS/en/armed.wav";
    let card = gallery(&b)
        .voices
        .into_iter()
        .find(|v| v.kind == "pack")
        .unwrap();
    assert_eq!(card.id, r.pack);
    assert_eq!(card.takes.len(), 12);
    let pack = b.dir.path().join("support/gear/voices").join(&r.pack);
    let flagged = flag(&b, "", &r.pack, LINE, "clipped");
    assert!(flagged
        .takes
        .iter()
        .any(|t| t.line == LINE && t.flag.is_some()));
    assert!(pack.join("flags.json").is_file());
    let k = b
        .core
        .gear_voice(&quadcam_lib::core::VoiceParams::default())
        .unwrap()
        .packs
        .into_iter()
        .find(|k| k.id == r.pack)
        .unwrap();
    assert_eq!(k.flags.len(), 1);
    let row = b.core.gear_voice_flags().unwrap().flags.remove(0);
    assert_eq!(
        (row.kind.as_str(), row.id.as_str()),
        ("pack", r.pack.as_str())
    );
    assert!(row.retake);
    // A re-take: priced, confirmed with the digest, one batch, and the flag goes.
    let posts = b.http.posts();
    let ask = b
        .core
        .gear_voice_retake(&quadcam_lib::core::RetakeParams {
            pack: r.pack.clone(),
            ..Default::default()
        })
        .unwrap();
    assert!(ask.needs_confirm);
    let done = b
        .core
        .gear_voice_retake(&quadcam_lib::core::RetakeParams {
            pack: r.pack.clone(),
            confirm: true,
            digest: Some(ask.digest),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(done.cleared, [LINE], "{done:?}");
    assert_eq!(b.http.posts(), posts + 1);
    assert!(pack.join(LINE).is_file());
    let card = gallery(&b)
        .voices
        .into_iter()
        .find(|v| v.kind == "pack")
        .unwrap();
    assert!(card
        .takes
        .iter()
        .all(|t| t.seed == Some(done.seed) && t.flag.is_none()));
    assert!(b.core.gear_voice_flags().unwrap().flags.is_empty());
    assert!(!pack.join("flags.json").exists());
    // Every other line stayed in the pack.
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(pack.join("pack.json")).unwrap()).unwrap();
    assert_eq!(m["files"].as_array().unwrap().len(), 12);
}

#[test]
fn a_line_heard_wrong_plays_its_held_take_and_a_retake_by_name_puts_a_new_take_in() {
    const LOW: &str = "SOUNDS/en/lowbat.wav";
    let wrong = Arc::new(std::sync::Mutex::new(vec![LOW.to_string()]));
    let (b, _helper) = bench_hearing(wrong.clone());
    let m = "eleven_turbo_v2_5";
    let r = paid_render(&b, render("Callum", m, &["quad"], false, true));
    assert!(r.retakes.iter().any(|t| t.path == LOW), "{r:?}");
    let pack = b.dir.path().join("support/gear/voices").join(&r.pack);
    // The held take plays, though it is out of the pack.
    let played = b
        .core
        .gear_voice_preview(&quadcam_lib::core::VoicePreviewParams {
            line: LOW.into(),
            pack: Some(r.pack.clone()),
            radio: None,
        })
        .unwrap();
    assert!(std::path::Path::new(&played).is_file());
    // A line that is neither flagged nor a re-take is refused.
    let e = b
        .core
        .gear_voice_retake(&quadcam_lib::core::RetakeParams {
            pack: r.pack.clone(),
            lines: vec!["SOUNDS/en/armed.wav".into()],
            dry_run: true,
            ..Default::default()
        })
        .unwrap_err();
    assert!(e.to_string().contains("needs no re-take"), "{e}");
    // Named, the re-take line needs no flag: a dry run prices it and sends nothing.
    let posts = b.http.posts();
    let ask = b
        .core
        .gear_voice_retake(&quadcam_lib::core::RetakeParams {
            pack: r.pack.clone(),
            lines: vec![LOW.into()],
            dry_run: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(ask.lines, [LOW]);
    assert!(
        ask.dry_run && ask.estimate.as_ref().is_some_and(|e| e.chars > 0),
        "{ask:?}"
    );
    assert_eq!(b.http.posts(), posts);
    // A confirm without the digest is refused and sends nothing.
    assert!(b
        .core
        .gear_voice_retake(&quadcam_lib::core::RetakeParams {
            pack: r.pack.clone(),
            lines: vec![LOW.into()],
            confirm: true,
            ..Default::default()
        })
        .is_err());
    assert_eq!(b.http.posts(), posts);
    // The check hears the new take right: it goes into the pack.
    wrong.lock().unwrap().clear();
    let done = b
        .core
        .gear_voice_retake(&quadcam_lib::core::RetakeParams {
            pack: r.pack.clone(),
            lines: vec![LOW.into()],
            confirm: true,
            digest: Some(ask.digest.clone()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(done.cleared, [LOW], "{done:?}");
    assert_eq!(done.seed, ask.seed);
    assert_eq!(b.http.posts(), posts + 1);
    assert!(pack.join(LOW).is_file());
    assert!(!pack.join("flags.json").exists());
    let k = b
        .core
        .gear_voice(&Default::default())
        .unwrap()
        .packs
        .into_iter()
        .find(|k| k.id == r.pack)
        .unwrap();
    assert!(!k.retakes.iter().any(|t| t.path == LOW), "{:?}", k.retakes);
}

#[test]
fn a_held_line_named_to_the_check_is_heard_again_and_goes_back_when_right() {
    const LOW: &str = "SOUNDS/en/lowbat.wav";
    let wrong = Arc::new(std::sync::Mutex::new(vec![LOW.to_string()]));
    let (b, _helper) = bench_hearing(wrong.clone());
    let r = paid_render(
        &b,
        render("Callum", "eleven_turbo_v2_5", &["quad"], false, true),
    );
    let pack = b.dir.path().join("support/gear/voices").join(&r.pack);
    assert!(!pack.join(LOW).exists());
    let named = |b: &Bench| {
        b.core
            .gear_voice_check(&quadcam_lib::core::VoiceCheckParams {
                pack: r.pack.clone(),
                lines: vec![LOW.into()],
                ..Default::default()
            })
            .unwrap()
    };
    // Still heard wrong: it stays out, with what was heard.
    let again = named(&b);
    assert_eq!(
        (again.mismatches, again.held, again.back),
        (1, 0, 0),
        "{again:?}"
    );
    assert!(!pack.join(LOW).exists());
    // Heard right now (a better check, the same take): back in the pack, at no cost.
    let posts = b.http.posts();
    wrong.lock().unwrap().clear();
    let back = named(&b);
    assert_eq!((back.mismatches, back.back), (0, 1), "{back:?}");
    assert!(pack.join(LOW).is_file());
    assert_eq!(b.http.posts(), posts);
    let k = b
        .core
        .gear_voice(&Default::default())
        .unwrap()
        .packs
        .into_iter()
        .find(|k| k.id == r.pack)
        .unwrap();
    assert!(!k.retakes.iter().any(|t| t.path == LOW), "{:?}", k.retakes);
}

#[test]
fn a_grouped_retake_tries_a_line_other_ways_and_the_pack_keeps_the_way_that_won() {
    const LOW: &str = "SOUNDS/en/lowbat.wav";
    // The check hears "Battery low" wrong twice: in the render, and in the first try.
    let left = Arc::new(std::sync::Mutex::new(2u32));
    let mut texts: Vec<(String, String)> = Vec::new();
    for id in quadcam_lib::gear::voice::sets::ids() {
        if id == "custom" {
            continue;
        }
        for l in quadcam_lib::gear::voice::sets::lines_of(id).unwrap() {
            texts.push((l.line.path, l.line.text));
        }
    }
    let counter = left.clone();
    let helper = Arc::new(quadcam_lib::intel::fake::FakeHelper::new(Box::new(
        move |f: &str| {
            let mut n = counter.lock().unwrap();
            if f.ends_with(LOW) && *n > 0 {
                *n -= 1;
                return Some("Battery.".into());
            }
            texts
                .iter()
                .find(|(p, _)| f.ends_with(p.as_str()))
                .map(|(_, t)| format!("{t}."))
        },
    )));
    let mut b = bench(99_000, true);
    let intel = quadcam_lib::intel::Intel::with("/helper/quadcam-intel".into(), helper);
    b.core = b.core.with_intel(Arc::new(intel));
    let r = paid_render(
        &b,
        render("Callum", "eleven_turbo_v2_5", &["quad"], false, true),
    );
    assert!(r.retakes.iter().any(|t| t.path == LOW), "{r:?}");
    let pack = b.dir.path().join("support/gear/voices").join(&r.pack);
    // A dry run groups the line by the word it got wrong and prices every try; it sends
    // nothing.
    let posts = b.http.posts();
    let grouped = |confirm: bool, digest: Option<String>| {
        b.core
            .gear_voice_retake(&quadcam_lib::core::RetakeParams {
                pack: r.pack.clone(),
                grouped: true,
                confirm,
                digest,
                ..Default::default()
            })
            .unwrap()
    };
    let ask = grouped(false, None);
    assert!(ask.needs_confirm, "{ask:?}");
    assert_eq!(ask.groups.len(), 1, "{ask:?}");
    let g = &ask.groups[0];
    assert_eq!((g.word.as_str(), g.sample.as_str()), ("low", LOW));
    assert_eq!(g.tries.len(), 6);
    assert_eq!(g.tries[0].sentence, "The word is Battery low.");
    assert_eq!(g.tries[1].sentence, "Say Battery low.");
    assert!(ask.estimate.as_ref().is_some_and(|e| e.chars > 0));
    assert_eq!(b.http.posts(), posts);
    // A confirm without the digest is refused.
    assert!(b
        .core
        .gear_voice_retake(&quadcam_lib::core::RetakeParams {
            pack: r.pack.clone(),
            grouped: true,
            confirm: true,
            ..Default::default()
        })
        .is_err());
    assert_eq!(b.http.posts(), posts);
    // The first try is heard wrong, the second (another carrier) right: it wins, and the pack
    // keeps that way for the line.
    let done = grouped(true, Some(ask.digest.clone()));
    let g = &done.groups[0];
    assert!(g.won, "{done:?}");
    assert!(g.tries[0].tried && !g.tries[0].ok);
    assert!(g.tries[1].tried && g.tries[1].ok);
    assert!(!g.tries[2].tried);
    assert_eq!(done.cleared, [LOW]);
    assert_eq!(b.http.posts(), posts + 2);
    let sent: Vec<String> = b
        .http
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|q| q.url.contains("with-timestamps"))
        .filter_map(|q| {
            serde_json::from_str::<serde_json::Value>(q.body.as_deref()?)
                .ok()?
                .get("text")?
                .as_str()
                .map(String::from)
        })
        .collect();
    assert_eq!(sent.last().map(String::as_str), Some("Say Battery low."));
    assert!(pack.join(LOW).is_file());
    let m: quadcam_lib::gear::voice::packs::PackManifest =
        serde_json::from_slice(&std::fs::read(pack.join("pack.json")).unwrap()).unwrap();
    assert!(!m.retakes.iter().any(|t| t.path == LOW));
    assert_eq!(
        m.structures,
        [quadcam_lib::gear::voice::packs::Structure {
            word: "low".into(),
            say: "low".into(),
            carrier: Some("Say {line}.".into()),
            lines: vec![LOW.into()],
        }]
    );
    // The check still compares with the line's text, and a recut of the pack's take costs
    // nothing: the way it was said is the pack's.
    let again = check(&b, &r.pack, &[]);
    assert_eq!(again.mismatches, 0, "{again:?}");
    let posts = b.http.posts();
    let redo = b
        .core
        .gear_voice_render(&VoiceRenderParams {
            lines: vec![LOW.into()],
            ..render("Callum", "eleven_turbo_v2_5", &["quad"], true, false)
        })
        .unwrap();
    assert_eq!(
        redo.estimate.map(|e| e.chars),
        Some(0),
        "the take is reused"
    );
    assert_eq!(b.http.posts(), posts);
}

#[test]
fn the_mcp_tools_list_flags_and_retake_only_with_confirm() {
    let b = bench(50000, true);
    let m = "eleven_turbo_v2_5";
    paid_sample(&b, sample(&["Callum"], &[m], false, true));
    let id = gallery(&b).voices[0].id.clone();
    let mut server = Server::new(LocalBackend(Arc::new(bench_at(&b).core)));
    let call = |server: &mut Server<LocalBackend>, name: &str, args: serde_json::Value| {
        let r = server.call_tool(name, args);
        assert_eq!(r["isError"], false, "{r}");
        r
    };
    let r = call(
        &mut server,
        "quadcam_gear_edit",
        json!({"action": "voice_flag", "sample": id, "line": "SOUNDS/en/armed.wav", "reason": "noise", "note": "hiss"}),
    );
    assert!(r.to_string().contains("Flagged"), "{r}");
    let r = call(
        &mut server,
        "quadcam_gear",
        json!({"action": "voice_flags"}),
    );
    let t = r.to_string();
    assert!(
        t.contains("hiss") && t.contains("v-callum") && t.contains("armed.wav"),
        "{t}"
    );
    let r = call(
        &mut server,
        "quadcam_gear",
        json!({"action": "voice_samples"}),
    );
    assert!(r.to_string().contains("FLAGGED noise"), "{r}");
    let posts = b.http.posts();
    let r = call(
        &mut server,
        "quadcam_gear_edit",
        json!({"action": "voice_retake", "sample": id}),
    );
    assert!(r.to_string().contains("Not rendered"), "{r}");
    assert_eq!(b.http.posts(), posts);
    let r = call(
        &mut server,
        "quadcam_gear_edit",
        json!({"action": "voice_samples_delete", "all_samples": true}),
    );
    assert!(r.to_string().contains("Not deleted"), "{r}");
    assert_eq!(gallery(&b).voices.len(), 1);
    call(
        &mut server,
        "quadcam_gear_edit",
        json!({"action": "voice_samples_delete", "all_samples": true, "confirm": true}),
    );
    assert!(gallery(&b).voices.is_empty());
}

#[test]
fn the_web_views_voice_folders_hold_every_sample_and_pack_take() {
    let b = bench(50000, true);
    paid_sample(&b, sample(&["Callum"], &["eleven_turbo_v2_5"], false, true));
    let r = paid_render(
        &b,
        render("Callum", "eleven_turbo_v2_5", &["quad"], false, true),
    );
    let gear = b.dir.path().join("support/gear");
    let dirs = b.core.voice_media_dirs();
    assert_eq!(
        dirs,
        vec![gear.join("voice-samples"), gear.join("voices")],
        "the narrowest folders, not the gear folder"
    );
    let g = b.core.gear_voice_samples().unwrap();
    let kinds: Vec<&str> = g.voices.iter().map(|v| v.kind.as_str()).collect();
    assert!(g.voices.iter().any(|v| v.id == r.pack), "{kinds:?}");
    let files: Vec<&str> = g
        .voices
        .iter()
        .flat_map(|v| v.takes.iter().map(|t| t.file.as_str()))
        .collect();
    assert!(files.len() > 12, "{files:?}");
    for f in files {
        let p = std::path::Path::new(f);
        assert!(p.is_file(), "{f}");
        assert!(
            dirs.iter().any(|d| p.starts_with(d)),
            "{f} is outside {dirs:?}"
        );
    }
}
