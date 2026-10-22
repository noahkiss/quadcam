//! Publishing a voice pack (`docs/gear.md`, "Publish a pack"): the speech check's gate, the
//! bundle (zip, AAC previews, index entry) and its review, Approve through a fake `gh` or the
//! printed commands, the digest that ties Approve to the review, MCP that prepares and never
//! approves, and an index pack's previews and install from a release address. Nothing reaches
//! GitHub: `gh` is a recording fake and the index is a local file or a 127.0.0.1 server.

use crate::common::server::{sha256, Server};
use quadcam_lib::core::{
    Core, NoHooks, PackInstallParams, PackPreviewParams, PublishApproveParams, PublishParams,
};
use quadcam_lib::gear::serial::FakePorts;
use quadcam_lib::gear::voice::packs::{self, HeardLine, PackManifest, VoiceIndex};
use quadcam_lib::gear::voice::publish::{self, Gh};
use quadcam_lib::gear::voice::render::RenderSettings;
use quadcam_lib::gear::voice::wav::{self, Pcm};
use quadcam_lib::gear::Env;
use quadcam_lib::mcp::{LocalBackend, Server as Mcp};
use quadcam_lib::modules::run::Runner;
use quadcam_lib::photos::Recorder;
use serde_json::{json, Value};
use std::ffi::OsString;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Output};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const PACK: &str = "local-elevenlabs-0123456789ab-eleven_v3";

/// A `gh` that answers each command from a table and records it.
#[derive(Default)]
struct FakeGh {
    calls: Mutex<Vec<Vec<String>>>,
    signed_in: bool,
    /// The repo's `index.json` before the publish.
    index: String,
}

fn out(code: i32, stdout: &str) -> Output {
    Output {
        status: ExitStatus::from_raw(code << 8),
        stdout: stdout.as_bytes().to_vec(),
        stderr: if code == 0 { vec![] } else { b"no".to_vec() },
    }
}

impl Runner for FakeGh {
    fn run(
        &self,
        _p: &Path,
        args: &[OsString],
        _cwd: &Path,
        _t: Duration,
    ) -> anyhow::Result<Output> {
        let a: Vec<String> = args
            .iter()
            .map(|x| x.to_string_lossy().to_string())
            .collect();
        self.calls.lock().unwrap().push(a.clone());
        let s: Vec<&str> = a.iter().map(String::as_str).collect();
        Ok(match s.as_slice() {
            ["auth", "status", ..] => out(if self.signed_in { 0 } else { 1 }, ""),
            ["release", "create", tag, ..] => out(
                0,
                &format!("https://github.com/noahkiss/quadcam-voices/releases/tag/{tag}"),
            ),
            ["api", "-X", "GET", p, ..] if p.ends_with("/git/ref/heads/main") => {
                out(0, r#"{"object":{"sha":"base1"}}"#)
            }
            ["api", "-X", "GET", p, ..] if p.ends_with("/git/commits/base1") => {
                out(0, r#"{"tree":{"sha":"tree1"}}"#)
            }
            ["api", "-X", "GET", p, ..] if p.contains("/contents/index.json") => {
                use base64::Engine;
                let b = base64::engine::general_purpose::STANDARD.encode(&self.index);
                out(0, &json!({ "content": b }).to_string())
            }
            ["api", "-X", "POST", p, "--input", f] => {
                let body: Value = serde_json::from_slice(&std::fs::read(f).unwrap()).unwrap();
                if p.ends_with("/git/blobs") {
                    let n = self.calls.lock().unwrap().len();
                    out(0, &json!({"sha": format!("blob{n}")}).to_string())
                } else if p.ends_with("/git/trees") {
                    assert_eq!(body["base_tree"], "tree1");
                    out(0, r#"{"sha":"tree2"}"#)
                } else if p.ends_with("/git/commits") {
                    assert_eq!(body["parents"], json!(["base1"]));
                    out(0, r#"{"sha":"commit2"}"#)
                } else {
                    out(0, r#"{"ref":"x"}"#)
                }
            }
            ["pr", "create", ..] => out(0, "https://github.com/noahkiss/quadcam-voices/pull/1"),
            _ => out(1, ""),
        })
    }
}

struct Bench {
    core: Core,
    dir: tempfile::TempDir,
    gh: Arc<FakeGh>,
}

impl Bench {
    fn voices(&self) -> PathBuf {
        self.dir.path().join("gear/voices")
    }
}

fn bench_with(gh: Option<FakeGh>, settings: Value) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("support")).unwrap();
    let mut s = json!({"gearDir": dir.path().join("gear")});
    for (k, v) in settings.as_object().unwrap() {
        s[k] = v.clone();
    }
    std::fs::write(dir.path().join("support/settings.json"), s.to_string()).unwrap();
    let mut env = Env::fake(vec![], Arc::new(FakePorts::new(vec![])));
    let fake = Arc::new(gh.unwrap_or_default());
    if fake.signed_in {
        env.gh = Arc::new(Gh::with("/opt/fake/gh".into(), fake.clone()));
    }
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
        dir,
        gh: fake,
    }
}

fn bench() -> Bench {
    bench_with(None, json!({}))
}

fn tone(hz: f64) -> Vec<u8> {
    let samples = (0..8000)
        .map(|i| ((i as f64 * hz * std::f64::consts::TAU / 32000.0).sin() * 8000.0) as i16)
        .collect();
    wav::write(&Pcm {
        rate: 32000,
        samples,
    })
}

/// QuadCam's lines in the `sample` set, then one of the person's own.
const LINES: &[(&str, &str)] = &[
    ("SOUNDS/en/armed.wav", "Armed"),
    ("SOUNDS/en/disarm.wav", "Disarmed"),
    ("SOUNDS/en/lowbat.wav", "Battery low"),
    ("SOUNDS/en/critbat.wav", "Battery critical"),
    ("SOUNDS/en/failsafe.wav", "Failsafe"),
    ("SOUNDS/en/mine.wav", "My own words"),
];

/// A rendered pack on this Mac, every line heard: `wrong` heard otherwise, `accepted` of
/// those accepted, and `unheard` never heard.
fn pack(b: &Bench, provider: &str, wrong: &[&str], accepted: &[&str], unheard: &[&str]) {
    let dir = b.voices().join(PACK);
    let mut files = Vec::new();
    let mut heard = Vec::new();
    for (i, (path, text)) in LINES.iter().enumerate() {
        let f = dir.join(path);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, tone(300.0 + 40.0 * i as f64)).unwrap();
        files.push(path.to_string());
        if unheard.contains(path) {
            continue;
        }
        let bad = wrong.contains(path);
        heard.push(HeardLine {
            path: path.to_string(),
            text: text.to_string(),
            heard: if bad {
                "something else".into()
            } else {
                text.to_string()
            },
            ok: !bad,
            accepted: accepted.contains(path),
            confidence: Some(0.9),
        });
    }
    let m = PackManifest {
        id: PACK.into(),
        voice: "Callum".into(),
        lang: "en".into(),
        provider: provider.into(),
        model: "eleven_v3".into(),
        settings: RenderSettings::default(),
        lines: files.len() as u32,
        license: String::new(),
        attribution: String::new(),
        lines_csv_sha: "csv".into(),
        version: String::new(),
        files,
        retakes: vec![],
        kept_batches: vec!["k".into()],
        firmware: "edgetx".into(),
        voice_id: "v-callum".into(),
        heard,

        structures: vec![],
    };
    std::fs::write(dir.join("pack.json"), serde_json::to_vec(&m).unwrap()).unwrap();
}

fn params() -> PublishParams {
    PublishParams {
        pack: PACK.into(),
        ..Default::default()
    }
}

#[test]
fn prepare_needs_a_clean_speech_check() {
    let b = bench();
    pack(&b, "elevenlabs", &[], &[], &["SOUNDS/en/lowbat.wav"]);
    let e = b.core.gear_voice_publish(&params()).unwrap_err();
    assert!(
        format!("{e:#}").contains("has not heard 1 of the pack's 6 lines"),
        "{e:#}"
    );
    pack(&b, "elevenlabs", &["SOUNDS/en/lowbat.wav"], &[], &[]);
    let e = b.core.gear_voice_publish(&params()).unwrap_err();
    assert!(
        format!("{e:#}").contains(
            "1 lines the speech check heard wrong are not accepted (SOUNDS/en/lowbat.wav)"
        ),
        "{e:#}"
    );
    pack(&b, "say", &[], &[], &[]);
    let e = b.core.gear_voice_publish(&params()).unwrap_err();
    assert!(format!("{e:#}").contains("macOS say"), "{e:#}");
    pack(&b, "openai", &[], &[], &[]);
    let e = b.core.gear_voice_publish(&params()).unwrap_err();
    assert!(
        format!("{e:#}").contains("Give the pack a license"),
        "{e:#}"
    );
    let e = b
        .core
        .gear_voice_publish(&PublishParams {
            id: Some("local-x".into()),
            license: Some("CC0".into()),
            ..params()
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("local pack's id"), "{e:#}");
    // Nothing was built or published by a refusal.
    assert!(b.gh.calls.lock().unwrap().is_empty());
}

#[test]
fn the_default_id_is_the_voices_short_name() {
    let b = bench();
    pack(
        &b,
        "elevenlabs",
        &["SOUNDS/en/lowbat.wav"],
        &["SOUNDS/en/lowbat.wav"],
        &[],
    );
    let r = b
        .core
        .gear_voice_publish(&PublishParams {
            voice_name: Some("Xavier - Dominating, Metalic Announcer".into()),
            ..params()
        })
        .unwrap();
    assert_eq!(r.entry.id, "en-xavier");
    let r = b
        .core
        .gear_voice_publish(&PublishParams {
            voice_name: Some("Johnny  Dynamite!!".into()),
            ..params()
        })
        .unwrap();
    assert_eq!(r.entry.id, "en-johnny-dynamite");
}

#[test]
fn prepare_builds_the_bundle_and_the_review() {
    let b = bench();
    pack(
        &b,
        "elevenlabs",
        &["SOUNDS/en/lowbat.wav"],
        &["SOUNDS/en/lowbat.wav"],
        &[],
    );
    let r = b.core.gear_voice_publish(&params()).unwrap();
    let e = &r.entry;
    assert_eq!(
        (e.id.as_str(), e.version.as_str(), e.voice.as_str()),
        ("en-callum", "1", "Callum")
    );
    assert_eq!(e.lines, 5, "the person's own line stays out");
    assert_eq!(r.left_out, vec!["SOUNDS/en/mine.wav".to_string()]);
    assert!(e.license.contains("commercial use"));
    assert_eq!(e.attribution, "Voice: Callum (ElevenLabs)");
    assert_eq!(
        e.url,
        "https://github.com/noahkiss/quadcam-voices/releases/download/en-callum-v1/voice-en-callum-1.zip"
    );
    assert_eq!(e.date.len(), 10);
    // Accepted lines first, then card order; every line has a file to play.
    assert_eq!(r.lines[0].path, "SOUNDS/en/lowbat.wav");
    assert!(r.lines[0].accepted && !r.lines[0].ok);
    assert_eq!(r.accepted, 1);
    assert!(r.lines.iter().all(|l| Path::new(&l.file).is_file()));
    assert_eq!(
        r.spot_check.len(),
        5,
        "a pack this small is spot-checked whole"
    );
    assert!(r.lines.iter().all(|l| l.spot));
    assert!(r.lines.iter().all(|l| l.preview));
    // The previews: AAC files with their hashes, in the sample set's order.
    assert_eq!(e.preview.len(), 5);
    assert_eq!(e.preview[0].path, "SOUNDS/en/armed.wav");
    assert_eq!(e.preview[0].file, "previews/en-callum-1/01.m4a");
    for (i, p) in e.preview.iter().enumerate() {
        let f = Path::new(&r.dir)
            .join("previews")
            .join(format!("{:02}.m4a", i + 1));
        let bytes = std::fs::read(&f).unwrap();
        assert_eq!(&bytes[4..8], b"ftyp", "an MP4 container");
        assert_eq!(sha256(&bytes), p.sha256);
    }
    // The zip installs as the index entry says, as published.
    let zip = std::fs::read(&r.zip).unwrap();
    assert_eq!(
        (sha256(&zip), zip.len() as u64),
        (e.sha256.clone(), e.zip_bytes)
    );
    let inst = packs::install(&b.dir.path().join("other"), Path::new(&r.zip), e).unwrap();
    assert_eq!(inst.manifest.id, "en-callum");
    assert_eq!(inst.manifest.files.len(), 5);
    assert!(inst.manifest.kept_batches.is_empty());
    assert!(!inst.manifest.files.iter().any(|f| f.contains("mine")));
    // The review says nothing is published, and gh is not there.
    assert!(!r.gh.ready);
    assert!(r.commands[0].starts_with("gh release create en-callum-v1 "));
    assert!(b.gh.calls.lock().unwrap().is_empty());
}

#[test]
fn approve_without_gh_prints_the_commands() {
    let b = bench();
    pack(&b, "elevenlabs", &[], &[], &[]);
    let r = b.core.gear_voice_publish(&params()).unwrap();
    let a = b
        .core
        .voice_publish_approve(&PublishApproveParams {
            digest: r.digest.clone(),
            id: Some("en-callum".into()),
        })
        .unwrap();
    assert!(a.published.is_none());
    assert_eq!(a.commands, r.commands);
    assert!(
        a.notes[0].starts_with("Nothing was published"),
        "{:?}",
        a.notes
    );
}

#[test]
fn approve_publishes_through_gh_and_checks_the_digest() {
    let b = bench_with(
        Some(FakeGh {
            signed_in: true,
            index: r#"{"schema":1,"packs":[]}"#.into(),
            ..Default::default()
        }),
        json!({}),
    );
    pack(&b, "elevenlabs", &[], &[], &[]);
    let r = b
        .core
        .gear_voice_publish(&PublishParams {
            version: Some("2".into()),
            ..params()
        })
        .unwrap();
    assert!(r.gh.ready);
    assert_eq!(b.gh.calls.lock().unwrap().len(), 1, "only gh auth status");
    // A wrong id or digest publishes nothing.
    let e = b
        .core
        .voice_publish_approve(&PublishApproveParams {
            digest: r.digest.clone(),
            id: Some("en-other".into()),
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("not \"en-other\""), "{e:#}");
    assert!(b
        .core
        .voice_publish_approve(&PublishApproveParams {
            digest: "0000".into(),
            id: None,
        })
        .is_err());
    // A zip changed after the review publishes nothing.
    let zip = std::fs::read(&r.zip).unwrap();
    std::fs::write(&r.zip, b"changed").unwrap();
    let e = b
        .core
        .voice_publish_approve(&PublishApproveParams {
            digest: r.digest.clone(),
            id: None,
        })
        .unwrap_err();
    assert!(
        format!("{e:#}").contains("changed after the review"),
        "{e:#}"
    );
    std::fs::write(&r.zip, zip).unwrap();
    let before = b.gh.calls.lock().unwrap().len();
    assert_eq!(before, 1, "refusals ran no gh");

    let a = b
        .core
        .voice_publish_approve(&PublishApproveParams {
            digest: r.digest.clone(),
            id: Some("en-callum".into()),
        })
        .unwrap();
    let p = a.published.unwrap();
    assert_eq!(p.branch, "pack/en-callum-v2");
    assert!(p.release_url.ends_with("/releases/tag/en-callum-v2"));
    assert!(p.pr_url.ends_with("/pull/1"));
    let calls = b.gh.calls.lock().unwrap().clone();
    let heads: Vec<String> = calls[1..]
        .iter()
        .map(|c| c[..2].join(" ") + " " + c.get(3).map(String::as_str).unwrap_or(""))
        .collect();
    assert!(heads[0].starts_with("auth status"));
    assert!(
        heads[1].starts_with("release create"),
        "the release comes first: {heads:?}"
    );
    assert_eq!(calls[2][3], r.zip, "the reviewed zip");
    // One blob for the index and one per preview, then the tree, the commit, the branch, the PR.
    let blobs: Vec<&Vec<String>> = calls
        .iter()
        .filter(|c| c.get(3).is_some_and(|p| p.ends_with("/git/blobs")))
        .collect();
    assert_eq!(blobs.len(), 1 + r.entry.preview.len());
    let tree = calls
        .iter()
        .find(|c| c.get(3).is_some_and(|p| p.ends_with("/git/trees")))
        .unwrap();
    let body: Value = serde_json::from_slice(&std::fs::read(&tree[5]).unwrap()).unwrap();
    let paths: Vec<&str> = body["tree"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths[0], "index.json");
    assert!(paths[1..]
        .iter()
        .all(|p| p.starts_with("previews/en-callum-2/")));
    let refs = calls
        .iter()
        .find(|c| c.get(3).is_some_and(|p| p.ends_with("/git/refs")))
        .unwrap();
    let body: Value = serde_json::from_slice(&std::fs::read(&refs[5]).unwrap()).unwrap();
    assert_eq!(
        body,
        json!({"ref": "refs/heads/pack/en-callum-v2", "sha": "commit2"})
    );
    assert_eq!(
        calls.last().unwrap()[..2],
        ["pr".to_string(), "create".to_string()]
    );
}

#[test]
fn mcp_prepares_and_cannot_approve() {
    let b = bench();
    pack(&b, "elevenlabs", &[], &[], &[]);
    let core = Arc::new(b.core);
    let mut s = Mcp::new(LocalBackend(core.clone()));
    let r = s.call_tool(
        "quadcam_gear_edit",
        json!({"action": "voice_publish", "pack": PACK, "id": "en-callum", "version": "3", "name": "Callum"}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let t = r["content"][0]["text"].as_str().unwrap();
    assert!(t.contains("Prepared en-callum v3"), "{t}");
    assert!(t.contains("Only the person approves"), "{t}");
    for action in ["voice_publish_approve", "voice_approve", "approve"] {
        let r = s.call_tool(
            "quadcam_gear_edit",
            json!({"action": action, "pack": PACK, "digest": "x"}),
        );
        assert_eq!(r["isError"], true, "{action}: {r}");
    }
    assert!(core
        .dispatch("voice_publish_approve", json!({"digest": "x"}))
        .is_err());
    assert!(core
        .dispatch("gear_voice_publish_approve", json!({"digest": "x"}))
        .is_err());
}

#[test]
fn an_index_pack_previews_and_installs_from_its_release_address() {
    let server = Server::start();
    let b = bench_with(None, json!({}));
    pack(&b, "elevenlabs", &[], &[], &[]);
    let r = b.core.gear_voice_publish(&params()).unwrap();
    let mut e = r.entry.clone();
    e.url = server.put(
        "/releases/voice-en-callum-1.zip",
        std::fs::read(&r.zip).unwrap(),
    );
    for (i, p) in e.preview.iter().enumerate() {
        let f = Path::new(&r.dir)
            .join("previews")
            .join(format!("{:02}.m4a", i + 1));
        server.put(&format!("/site/{}", p.file), std::fs::read(f).unwrap());
    }
    let idx = VoiceIndex {
        schema: Some(1),
        packs: vec![e.clone()],
    };
    let index_url = server.put("/site/index.json", serde_json::to_vec(&idx).unwrap());
    let b2 = bench_with(None, json!({"voiceIndex": index_url}));

    let takes = b2
        .core
        .gear_voice_pack_preview(&PackPreviewParams {
            pack: "en-callum".into(),
            source: None,
        })
        .unwrap();
    assert_eq!(takes.len(), e.preview.len());
    assert_eq!(takes[0].text, "Armed");
    assert!(takes.iter().all(|t| Path::new(&t.file).is_file()));
    assert!(server
        .hits()
        .contains(&"/site/previews/en-callum-1/01.m4a".to_string()));
    // A second preview reads the cache.
    let n = server.hits().len();
    b2.core
        .gear_voice_pack_preview(&PackPreviewParams {
            pack: "en-callum".into(),
            source: None,
        })
        .unwrap();
    assert_eq!(server.hits().len(), n + 1, "only the index again");

    let view = b2.core.gear_voice(&Default::default()).unwrap();
    let listed = view.packs.iter().find(|k| k.id == "en-callum").unwrap();
    assert!(!listed.installed);
    assert_eq!(listed.preview.len(), e.preview.len());
    assert_eq!(listed.zip_bytes, e.zip_bytes);

    let k = b2
        .core
        .gear_voice_pack_install(&PackInstallParams {
            pack: "en-callum".into(),
            source: None,
        })
        .unwrap();
    assert!(k.installed);
    assert!(server
        .hits()
        .contains(&"/releases/voice-en-callum-1.zip".to_string()));

    // A preview whose bytes differ from the index refuses.
    server.put("/site/previews/en-callum-1/02.m4a", b"other".to_vec());
    let b3 = bench_with(None, json!({"voiceIndex": index_url}));
    let e = b3
        .core
        .gear_voice_pack_preview(&PackPreviewParams {
            pack: "en-callum".into(),
            source: None,
        })
        .unwrap_err();
    assert!(
        format!("{e:#}").contains("does not match the hash"),
        "{e:#}"
    );
}

#[test]
fn the_default_index_is_the_voices_repo() {
    let b = bench();
    let v = b.core.gear_voice(&Default::default()).unwrap();
    assert_eq!(v.index_source.as_deref(), Some(publish::INDEX_URL));
    let off = bench_with(None, json!({"voiceIndex": ""}));
    let v = off.core.gear_voice(&Default::default()).unwrap();
    assert_eq!(v.index_source, None);
}
