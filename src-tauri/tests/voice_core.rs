//! Voice on the core (WP9 part B), with fake providers only: rendering QuadCam's lines into a
//! local pack (a cache hit calls no provider; a provider that may charge waits for confirm),
//! installing a pack from an index with a hash check, per-line overrides, and Choose voice,
//! which stages one card change, keeps overrides when asked, and applies to a synthetic
//! card with read-back. Every card is a temporary folder and "diskutil" is a closure.

use quadcam_lib::core::{
    BackupParams, Core, Hooks, NoHooks, PackInstallParams, VoiceChooseParams, VoiceEditParams,
    VoiceParams, VoiceRenderParams,
};
use quadcam_lib::disk::{DiskInfo, Volume};
use quadcam_lib::gear::apply::{ApplyPlanParams, ApplyRequest};
use quadcam_lib::gear::bf::cli::Timing;
use quadcam_lib::gear::changes::ChangeFilter;
use quadcam_lib::gear::cues::{CueService, RecordedCues};
use quadcam_lib::gear::edgetx::synth::{self, SynthCard};
use quadcam_lib::gear::events::Presence;
use quadcam_lib::gear::model::{ChangeStatus, Edit};
use quadcam_lib::gear::serial::FakePorts;
use quadcam_lib::gear::voice::tts::{ProviderConfig, Providers, Tts, TtsRequest};
use quadcam_lib::gear::voice::wav::{self, Pcm};
use quadcam_lib::gear::Env;
use quadcam_lib::photos::Recorder;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A provider that speaks a burst as long as the text.
struct Fake {
    calls: Arc<AtomicUsize>,
    paid: bool,
    tag: String,
}

impl Tts for Fake {
    fn id(&self) -> &str {
        "fake"
    }
    fn paid(&self) -> bool {
        self.paid
    }
    fn render(&self, req: &TtsRequest<'_>) -> anyhow::Result<Pcm> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let win = 160usize;
        let mut s = vec![2i16; win * 6];
        let amp: i16 = 6000 + 100 * (self.tag.len() as i16);
        for i in 0..win * (8 + req.text.chars().count()) {
            s.push(if (i / 18) % 2 == 0 { amp } else { -amp });
        }
        s.extend(std::iter::repeat_n(4i16, win * 10));
        Ok(Pcm {
            rate: 32000,
            samples: s,
        })
    }
}

struct FakeProviders {
    calls: Arc<AtomicUsize>,
    paid: Arc<AtomicBool>,
    last: Arc<Mutex<Option<ProviderConfig>>>,
}

impl Providers for FakeProviders {
    fn make(&self, cfg: &ProviderConfig) -> anyhow::Result<Box<dyn Tts>> {
        *self.last.lock().unwrap() = Some(cfg.clone());
        Ok(Box::new(Fake {
            calls: self.calls.clone(),
            paid: self.paid.load(Ordering::SeqCst),
            tag: cfg.base_url.clone(),
        }))
    }
}

const DISK: &str = "disk42";

fn volume(root: &std::path::Path, uuid: &str) -> Volume {
    Volume {
        mount: root.to_path_buf(),
        info: DiskInfo {
            volume_uuid: Some(uuid.into()),
            parent_whole_disk: DISK.into(),
            bus_protocol: Some("USB".into()),
            removable: true,
            ..Default::default()
        },
        is_card: false,
        source: None,
        is_radio: true,
        warnings: vec![],
    }
}

#[allow(dead_code)]
struct Bench {
    core: Arc<Core>,
    cues: Arc<RecordedCues>,
    root: PathBuf,
    id: String,
    /// The card is mounted now.
    mounted: Arc<AtomicBool>,
    /// The card is still plugged in (its disk node shows).
    present: Arc<AtomicBool>,
    /// What the fake `diskutil` was asked: "mount disk42", "unmount disk42".
    log: Arc<Mutex<Vec<String>>>,
    unmount_fails: Arc<AtomicBool>,
    dir: tempfile::TempDir,
    calls: Arc<AtomicUsize>,
    paid: Arc<AtomicBool>,
    last: Arc<Mutex<Option<ProviderConfig>>>,
}

struct Opts {
    card: SynthCard,
    fail_readback: Option<String>,
    hooks: Arc<dyn Hooks>,
    uuid: &'static str,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            card: SynthCard::default(),
            fail_readback: None,
            hooks: Arc::new(NoHooks),
            uuid: "11111111-2222-3333-4444-555555555555",
        }
    }
}

/// A core with one synthetic radio card mounted, backed up once so the radio is a saved
/// device.
fn bench(o: Opts) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("CARD");
    synth::write_card(&root, &o.card).unwrap();
    let mounted = Arc::new(AtomicBool::new(true));
    let log: Arc<Mutex<Vec<String>>> = Arc::default();
    let unmount_fails = Arc::new(AtomicBool::new(false));
    let cues = Arc::new(RecordedCues::default());
    let vol = volume(&root, o.uuid);
    let mut env = Env::fake(vec![], Arc::new(FakePorts::new(vec![])));
    env.cues = Arc::new(CueService::inline(cues.clone()));
    let m = mounted.clone();
    env.volumes = Arc::new(move || {
        if m.load(Ordering::SeqCst) {
            vec![vol.clone()]
        } else {
            vec![]
        }
    });
    let present = Arc::new(AtomicBool::new(true));
    let p = present.clone();
    env.presence = Arc::new(move || {
        if p.load(Ordering::SeqCst) {
            vec![Presence::Disk { disk: DISK.into() }]
        } else {
            vec![]
        }
    });
    let (m, l, f) = (mounted.clone(), log.clone(), unmount_fails.clone());
    env.unmount = Arc::new(move |d| {
        l.lock().unwrap().push(format!("unmount {d}"));
        if f.load(Ordering::SeqCst) {
            anyhow::bail!("Unmount of {d} failed: at least one volume could not be unmounted")
        }
        m.store(false, Ordering::SeqCst);
        Ok(())
    });
    let (m, l) = (mounted.clone(), log.clone());
    env.mount = Arc::new(move |d| {
        l.lock().unwrap().push(format!("mount {d}"));
        m.store(true, Ordering::SeqCst);
        Ok(())
    });
    env.fail_readback = o.fail_readback;
    let calls = Arc::new(AtomicUsize::new(0));
    let paid = Arc::new(AtomicBool::new(false));
    let last = Arc::new(Mutex::new(None));
    env.tts = Arc::new(FakeProviders {
        calls: calls.clone(),
        paid: paid.clone(),
        last: last.clone(),
    });
    // Cues play every time here: the debounce would hide a second "safe to unplug".
    std::fs::create_dir_all(dir.path().join("support")).unwrap();
    std::fs::write(
        dir.path().join("support/settings.json"),
        r#"{"gearCues": {"debounce_s": 0}, "ttsVoice": "Test Voice", "ttsBaseUrl": "A"}"#,
    )
    .unwrap();
    let core = Arc::new(
        Core::new(
            dir.path().join("cache"),
            None,
            o.hooks,
            Arc::new(Recorder::default()),
        )
        .with_settings(dir.path().join("support/settings.json"))
        .with_gear_env(env)
        .with_fc_timing(Timing::fast()),
    );
    let id = core
        .gear_connected()
        .unwrap()
        .into_iter()
        .find_map(|c| c.id)
        .expect("the card has an id");
    core.gear_backup(&BackupParams {
        device: Some(id.clone()),
        ..Default::default()
    })
    .unwrap();
    // The backup unmounted the card; the tests start with it mounted, as a reader shows it.
    mounted.store(true, Ordering::SeqCst);
    log.lock().unwrap().clear();
    cues.played.lock().unwrap().clear();
    Bench {
        core,
        cues,
        root,
        id,
        mounted,
        present,
        log,
        unmount_fails,
        dir,
        calls,
        paid,
        last,
    }
}

fn view(b: &Bench) -> quadcam_lib::core::VoiceView {
    b.core
        .gear_voice(&VoiceParams {
            radio: Some(b.id.clone()),
            refresh_index: false,
        })
        .unwrap()
}

fn render(b: &Bench, lines: &[&str], dry: bool, confirm: bool) -> quadcam_lib::core::RenderReport {
    b.core
        .gear_voice_render(&VoiceRenderParams {
            voice: String::new(),
            lines: lines.iter().map(|s| s.to_string()).collect(),
            dry_run: dry,
            confirm,
            settings: None,
            ..Default::default()
        })
        .unwrap()
}

fn staged(b: &Bench) -> Vec<quadcam_lib::gear::model::StagedChange> {
    b.core.gear_changes(&ChangeFilter::default()).unwrap()
}

fn read(b: &Bench, rel: &str) -> Vec<u8> {
    std::fs::read(b.root.join(rel)).unwrap()
}

const ARMED: &str = "SOUNDS/en/armed.wav";
const LOWBAT: &str = "SOUNDS/en/lowbat.wav";

#[test]
fn the_voice_view_lists_lines_with_their_spoken_text_and_the_provider() {
    let b = bench(Opts::default());
    let v = view(&b);
    assert!(v.provider.ready, "{:?}", v.provider.problem);
    assert_eq!(
        (v.provider.provider.as_str(), v.provider.voice.as_str()),
        ("say", "Test Voice")
    );
    assert!(!v.provider.paid);
    let gps = v
        .lines
        .iter()
        .find(|l| l.path.ends_with("gpsfix.wav"))
        .unwrap();
    assert_eq!(
        (gps.text.as_str(), gps.spoken.as_str()),
        ("GPS fix", "G.P.S. fix")
    );
    assert!(!gps.custom && gps.packs.is_empty() && gps.override_.is_none());
    assert!(v.packs.is_empty());
    assert!(v.chosen.is_none());
}

#[test]
fn rendering_makes_a_local_pack_and_a_second_render_is_all_cache() {
    let b = bench(Opts::default());
    let dry = render(&b, &[], true, false);
    assert_eq!(dry.plan.lines, 45);
    assert_eq!(
        (dry.plan.cached, dry.plan.to_render),
        (0, 45),
        "{:?}",
        dry.plan
    );
    assert_eq!(b.calls.load(Ordering::SeqCst), 0, "a dry run calls nothing");
    assert!(view(&b).packs.is_empty());
    let r = render(&b, &[], false, false);
    assert_eq!(r.pack, "local-fake-test-voice");
    assert!(b.calls.load(Ordering::SeqCst) > 0);
    let first_calls = b.calls.load(Ordering::SeqCst);
    assert_eq!(r.rendered as usize, first_calls);
    let v = view(&b);
    let p = v.packs.iter().find(|p| p.id == r.pack).unwrap();
    assert!(p.installed && p.local && !p.stale);
    assert_eq!(p.lines, 45);
    assert!(v.lines.iter().all(|l| l.packs == vec![r.pack.clone()]));
    // Again: every take comes from the cache.
    let again = render(&b, &[], false, false);
    assert_eq!(again.rendered, 0);
    assert_eq!(
        b.calls.load(Ordering::SeqCst),
        first_calls,
        "no provider call on a cache hit"
    );
    // A subset adds to the pack and keeps the other lines.
    let one = render(&b, &[ARMED], false, false);
    assert_eq!(one.plan.lines, 1);
    assert_eq!(view(&b).packs[0].lines, 45);
}

#[test]
fn a_provider_that_may_charge_waits_for_confirm() {
    let b = bench(Opts::default());
    b.paid.store(true, Ordering::SeqCst);
    let held = render(&b, &[ARMED, LOWBAT], false, false);
    assert!(held.needs_confirm && held.paid);
    assert_eq!(held.rendered, 0);
    assert_eq!(held.plan.chars, 16, "Armed + Battery low");
    assert_eq!(b.calls.load(Ordering::SeqCst), 0);
    assert!(view(&b).packs.is_empty());
    let done = render(&b, &[ARMED, LOWBAT], false, true);
    assert!(!done.needs_confirm);
    assert_eq!(done.rendered, 2);
    // Cached takes need no confirm.
    let free = render(&b, &[ARMED, LOWBAT], false, false);
    assert!(!free.needs_confirm);
    assert_eq!(free.rendered, 0);
}

/// Builds a pack in `dir` and returns its index path.
fn build(b: &Bench, dir: &std::path::Path, id: &str, tail_ms: u32) -> PathBuf {
    let (zip, entry, _) = b
        .core
        .voice_build_pack(&quadcam_lib::core::BuildPackParams {
            voice: "Test Voice".into(),
            id: Some(id.into()),
            version: Some("1".into()),
            out: dir.to_path_buf(),
            license: Some("CC BY 4.0".into()),
            attribution: Some("test".into()),
            settings: Some(quadcam_lib::gear::voice::render::RenderSettings {
                tail_ms,
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap();
    assert!(zip.is_file());
    assert_eq!(entry.id, id);
    dir.join("voices.json")
}

#[test]
fn a_pack_installs_from_an_index_after_a_hash_check() {
    let b = bench(Opts::default());
    let out = b.dir.path().join("release");
    let idx = build(&b, &out, "en-test-v1", 150);
    let p = b
        .core
        .gear_voice_pack_install(&PackInstallParams {
            pack: "en-test-v1".into(),
            source: Some(idx.display().to_string()),
        })
        .unwrap();
    assert!(p.installed && !p.local && p.lines == 45);
    assert_eq!(p.license, "CC BY 4.0");
    assert!(b
        .dir
        .path()
        .join("support/gear/voices/en-test-v1/pack.json")
        .is_file());
    // The view lists it as installed and, with the index cached, knows its size.
    assert!(view(&b)
        .packs
        .iter()
        .any(|p| p.id == "en-test-v1" && p.installed));
    // An unknown pack lists what the index has.
    let e = b
        .core
        .gear_voice_pack_install(&PackInstallParams {
            pack: "nope".into(),
            source: Some(idx.display().to_string()),
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("en-test-v1"), "{e:#}");
    // A zip that changed after the index was written does not install.
    let zip = out.join("voice-en-test-v1-1.zip");
    let mut bytes = std::fs::read(&zip).unwrap();
    let n = bytes.len() / 2;
    bytes[n] ^= 0xFF;
    std::fs::write(&zip, bytes).unwrap();
    let e = b
        .core
        .gear_voice_pack_install(&PackInstallParams {
            pack: "en-test-v1".into(),
            source: Some(idx.display().to_string()),
        })
        .unwrap_err();
    assert!(
        format!("{e:#}").contains("does not match the hash"),
        "{e:#}"
    );
}

#[test]
fn choose_voice_stages_one_change_and_keeps_overrides_when_asked() {
    let b = bench(Opts::default());
    let out = b.dir.path().join("release");
    let idx = build(&b, &out, "en-a-v1", 150);
    build(&b, &out, "en-b-v1", 90);
    for id in ["en-a-v1", "en-b-v1"] {
        b.core
            .gear_voice_pack_install(&PackInstallParams {
                pack: id.into(),
                source: Some(idx.display().to_string()),
            })
            .unwrap();
    }
    let dir = |id: &str, f: &str| {
        std::fs::read(b.dir.path().join("support/gear/voices").join(id).join(f)).unwrap()
    };
    // Override one line with the other pack's take, another with the person's own text.
    let l = b
        .core
        .gear_voice_edit(&VoiceEditParams {
            radio: b.id.clone(),
            line: ARMED.into(),
            pack: Some("en-b-v1".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(l.override_.unwrap().kind, "pack");
    let calls_before = b.calls.load(Ordering::SeqCst);
    let l = b
        .core
        .gear_voice_edit(&VoiceEditParams {
            radio: b.id.clone(),
            line: LOWBAT.into(),
            text: Some("Pack low, land now".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(l.override_.as_ref().unwrap().kind, "text");
    assert_eq!(b.calls.load(Ordering::SeqCst), calls_before + 1);
    // Choose pack a, keeping overrides: one change, the two overrides win on their lines.
    let c = b
        .core
        .gear_voice_choose(&VoiceChooseParams {
            radio: b.id.clone(),
            pack: "en-a-v1".into(),
            keep_overrides: true,
            editor: None,
        })
        .unwrap();
    assert_eq!(c.status, ChangeStatus::Ready);
    assert!(c.title.starts_with("Voice: "));
    assert_eq!(c.edits.len(), 1);
    let Edit::CardFiles { put, delete } = &c.edits[0] else {
        panic!("{:?}", c.edits[0])
    };
    assert_eq!(put.len(), 45);
    assert!(delete.is_empty(), "sounds no pack knows are never deleted");
    assert_eq!(view(&b).chosen.as_deref(), Some("en-a-v1"));
    // Apply: backed up, written, read back.
    let plan = b
        .core
        .gear_apply_plan(&ApplyPlanParams {
            id: c.id.clone(),
            port: None,
        })
        .unwrap();
    assert!(plan.checks.iter().all(|k| k.ok), "{:?}", plan.checks);
    let r = b
        .core
        .gear_apply(&ApplyRequest {
            id: c.id.clone(),
            digest: plan.digest,
            confirm: true,
            port: None,
        })
        .unwrap();
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    assert_eq!(
        read(&b, "SOUNDS/en/gpsfix.wav"),
        dir("en-a-v1", "SOUNDS/en/gpsfix.wav")
    );
    assert_eq!(
        read(&b, ARMED),
        dir("en-b-v1", ARMED),
        "the picked take stays"
    );
    assert_ne!(read(&b, ARMED), dir("en-a-v1", ARMED));
    let own = wav::read(&read(&b, LOWBAT)).unwrap();
    assert!(
        own.samples.len() > 32 * 170,
        "the person's own render is on the card"
    );
    assert_ne!(read(&b, LOWBAT), dir("en-a-v1", LOWBAT));
    assert_eq!(
        read(&b, "SOUNDS/en/hello.wav"),
        b"RIFF\0\0\0\0WAVE",
        "a file no pack knows stays"
    );
    // Choose again without keeping overrides: the overrides go, the pack's takes return.
    b.mounted.store(true, Ordering::SeqCst);
    let c2 = b
        .core
        .gear_voice_choose(&VoiceChooseParams {
            radio: b.id.clone(),
            pack: "en-a-v1".into(),
            keep_overrides: false,
            editor: None,
        })
        .unwrap();
    let plan = b
        .core
        .gear_apply_plan(&ApplyPlanParams {
            id: c2.id.clone(),
            port: None,
        })
        .unwrap();
    assert!(plan.checks.iter().all(|k| k.ok), "{:?}", plan.checks);
    let files: Vec<&str> = plan
        .diff
        .iter()
        .filter_map(|d| match d {
            quadcam_lib::gear::model::DiffItem::Files { put, .. } => {
                Some(put.iter().map(String::as_str).collect::<Vec<_>>())
            }
            _ => None,
        })
        .flatten()
        .collect();
    assert!(
        files.contains(&ARMED) && files.contains(&LOWBAT),
        "{files:?}"
    );
    assert!(view(&b).lines.iter().all(|l| l.override_.is_none()));
}

#[test]
fn choosing_again_updates_the_open_change_and_the_overrides_survive_a_restage() {
    let b = bench(Opts::default());
    let out = b.dir.path().join("release");
    let idx = build(&b, &out, "en-a-v1", 150);
    build(&b, &out, "en-b-v1", 90);
    for id in ["en-a-v1", "en-b-v1"] {
        b.core
            .gear_voice_pack_install(&PackInstallParams {
                pack: id.into(),
                source: Some(idx.display().to_string()),
            })
            .unwrap();
    }
    let choose = |pack: &str| {
        b.core
            .gear_voice_choose(&VoiceChooseParams {
                radio: b.id.clone(),
                pack: pack.into(),
                keep_overrides: true,
                editor: None,
            })
            .unwrap()
    };
    let c = choose("en-a-v1");
    b.core
        .gear_voice_edit(&VoiceEditParams {
            radio: b.id.clone(),
            line: ARMED.into(),
            text: Some("Motors live".into()),
            ..Default::default()
        })
        .unwrap();
    let c2 = choose("en-b-v1");
    assert_eq!(c2.id, c.id, "one voice change per radio");
    assert_eq!(staged(&b).len(), 1);
    assert!(c2.title.ends_with("Test Voice"));
    // Resetting an override removes it.
    let l = b
        .core
        .gear_voice_edit(&VoiceEditParams {
            radio: b.id.clone(),
            line: ARMED.into(),
            ..Default::default()
        })
        .unwrap();
    assert!(l.override_.is_none());
}

#[test]
fn custom_lines_are_the_persons_own_and_never_in_a_built_pack() {
    let b = bench(Opts::default());
    let l = b
        .core
        .gear_voice_edit(&VoiceEditParams {
            radio: b.id.clone(),
            line: "SOUNDS/en/mycall.wav".into(),
            text: Some("Gate three".into()),
            ..Default::default()
        })
        .unwrap();
    assert!(l.custom);
    assert_eq!(l.group, "extras");
    // A line QuadCam does not know, with no text, is refused.
    let e = b
        .core
        .gear_voice_edit(&VoiceEditParams {
            radio: b.id.clone(),
            line: "SOUNDS/en/other.wav".into(),
            ..Default::default()
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("no line"), "{e:#}");
    // Rendering includes it; building a pack does not.
    let r = render(&b, &[], false, false);
    assert_eq!(r.plan.lines, 46);
    let out = b.dir.path().join("release");
    build(&b, &out, "en-test-v1", 150);
    let idx = quadcam_lib::gear::voice::packs::read_index(&out.join("voices.json")).unwrap();
    assert_eq!(idx.packs[0].lines, 45);
}

#[test]
fn bad_calls_say_what_is_wrong() {
    let b = bench(Opts::default());
    let e = b
        .core
        .gear_voice_choose(&VoiceChooseParams {
            radio: b.id.clone(),
            pack: "nope".into(),
            keep_overrides: true,
            editor: None,
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("not installed"), "{e:#}");
    let e = b
        .core
        .gear_voice_edit(&VoiceEditParams {
            radio: "nope".into(),
            line: ARMED.into(),
            text: Some("x".into()),
            ..Default::default()
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("No device"), "{e:#}");
    let e = b
        .core
        .gear_voice_edit(&VoiceEditParams {
            radio: b.id.clone(),
            line: "../x.wav".into(),
            text: Some("x".into()),
            ..Default::default()
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("not SOUNDS"), "{e:#}");
    let e = b
        .core
        .gear_voice_pack_install(&PackInstallParams {
            pack: "x".into(),
            source: None,
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("No pack index"), "{e:#}");
    // The provider setting reaches the provider.
    render(&b, &[ARMED], true, false);
    let cfg = b.last.lock().unwrap().clone().unwrap();
    assert_eq!((cfg.provider.as_str(), cfg.base_url.as_str()), ("say", "A"));
}

#[test]
fn the_rows_and_the_mcp_actions() {
    use quadcam_lib::mcp::{LocalBackend, Server};
    use serde_json::json;
    let b = bench(Opts::default());
    let v = b
        .core
        .dispatch("gear_voice", json!({"radio": b.id}))
        .unwrap();
    assert_eq!(v["lines"].as_array().unwrap().len(), 45);
    let r = b
        .core
        .dispatch(
            "gear_voice_render",
            json!({"lines": [ARMED], "dry_run": true}),
        )
        .unwrap();
    assert_eq!(r["plan"]["to_render"], 1);
    let mut s = Server::new(LocalBackend(b.core.clone()));
    let r = s.call_tool("quadcam_gear", json!({"action": "voice", "device": b.id}));
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("45 lines"), "{text}");
    assert!(text.contains("Provider: say"), "{text}");
    let r = s.call_tool(
        "quadcam_gear_edit",
        json!({"action": "voice_render", "lines": [ARMED], "dry_run": true}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("would render 1"));
    let r = s.call_tool(
        "quadcam_gear_edit",
        json!({"action": "voice_choose", "device": b.id, "pack": "nope"}),
    );
    assert_eq!(r["isError"], true);
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("not installed"));
}

#[test]
fn a_take_copies_into_the_cache_so_the_app_can_play_it() {
    let b = bench(Opts::default());
    let out = b.dir.path().join("release");
    let idx = build(&b, &out, "en-a-v1", 150);
    b.core
        .gear_voice_pack_install(&PackInstallParams {
            pack: "en-a-v1".into(),
            source: Some(idx.display().to_string()),
        })
        .unwrap();
    let p = b
        .core
        .gear_voice_preview(&quadcam_lib::core::VoicePreviewParams {
            line: ARMED.into(),
            pack: Some("en-a-v1".into()),
            radio: None,
        })
        .unwrap();
    assert!(
        p.starts_with(b.dir.path().join("cache").to_str().unwrap()),
        "{p}"
    );
    assert!(wav::read(&std::fs::read(&p).unwrap()).is_ok());
    b.core
        .gear_voice_edit(&VoiceEditParams {
            radio: b.id.clone(),
            line: ARMED.into(),
            text: Some("Motors live".into()),
            ..Default::default()
        })
        .unwrap();
    let own = b
        .core
        .gear_voice_preview(&quadcam_lib::core::VoicePreviewParams {
            line: ARMED.into(),
            pack: None,
            radio: Some(b.id.clone()),
        })
        .unwrap();
    assert_ne!(own, p);
    assert!(wav::read(&std::fs::read(&own).unwrap()).is_ok());
    let e = b
        .core
        .gear_voice_preview(&quadcam_lib::core::VoicePreviewParams {
            line: LOWBAT.into(),
            pack: None,
            radio: Some(b.id.clone()),
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("no override"), "{e:#}");
}
