//! The voice pipeline (design 7.4, WP9 part B), with fake providers only: the spelling
//! rules, the raw cache (a hit calls no provider), the normalisation golden WAV, the
//! providers' requests (no text, key or URL in argv), the pack build (a zip and an index
//! entry), and the install checks. No real voice or network is reached.

use quadcam_lib::gear::voice::lines::{self, Line, Spelling};
use quadcam_lib::gear::voice::packs::{self, BuildOpts, PackIndexEntry};
use quadcam_lib::gear::voice::render::{self, Cache, Ctx, RenderSettings};
use quadcam_lib::gear::voice::sha256_hex;
use quadcam_lib::gear::voice::tts::{OpenAi, Post, Run, Say, Tts, TtsRequest};
use quadcam_lib::gear::voice::wav::{self, Pcm};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A provider that speaks a burst as long as the text, and counts its calls.
struct Fake {
    calls: Arc<AtomicUsize>,
    rate: u32,
}

impl Tts for Fake {
    fn id(&self) -> &str {
        "fake"
    }
    fn paid(&self) -> bool {
        false
    }
    fn render(&self, req: &TtsRequest<'_>) -> anyhow::Result<Pcm> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let win = (self.rate / 200) as usize;
        let mut s = vec![2i16; win * 6];
        for i in 0..win * (8 + req.text.chars().count()) {
            s.push(if (i / 18) % 2 == 0 { 9000 } else { -9000 });
        }
        s.extend(std::iter::repeat_n(4i16, win * 10));
        Ok(Pcm {
            rate: self.rate,
            samples: s,
        })
    }
}

fn fake(rate: u32) -> (Fake, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    (
        Fake {
            calls: calls.clone(),
            rate,
        },
        calls,
    )
}

#[test]
fn the_spelling_rules_have_a_golden() {
    let sp = Spelling::builtin().unwrap();
    let cases = [
        ("DVR recording", "D.V.R. recording"),
        ("VTX power and TX power", "V.T.X. power and T.X. power"),
        ("PID, GPS, LQ and RF", "P.I.D., G.P.S., L.Q. and R.F."),
        ("-80 dBm", "-80 D.B.M."),
        ("3 dB down", "3 D.B. down"),
        ("2000 mAh used", "2000 milliamp hours used"),
        ("OSD and 3D and FPV", "OSD and 3D and FPV"),
        ("The GPSfix is not a word", "The GPSfix is not a word"),
        ("tx is not TX", "tx is not T.X."),
        ("RTH now", "return to home now"),
        ("", ""),
    ];
    for (readable, spoken) in cases {
        assert_eq!(sp.apply(readable), spoken, "{readable:?}");
    }
    // The repo's own lines: a rule never changes a plain number word.
    for l in lines::builtin().unwrap() {
        if l.group == "numbers" {
            assert_eq!(sp.apply(&l.text), l.text);
        }
    }
    let gps = lines::builtin()
        .unwrap()
        .into_iter()
        .find(|l| l.path.ends_with("gpsfix.wav"))
        .unwrap();
    assert_eq!(sp.apply(&gps.text), "G.P.S. fix");
}

#[test]
fn a_plain_word_with_a_rule_is_a_mistake() {
    let e = Spelling::parse("[spell]\nOSD = \"O.S.D.\"\n[plain]\nwords = [\"OSD\"]\n").unwrap_err();
    assert!(e.to_string().contains("plain word"), "{e}");
}

fn ctx_in<'a>(tts: &'a dyn Tts, cache: &'a Cache, settings: &'a RenderSettings) -> Ctx<'a> {
    Ctx {
        tts,
        cache,
        voice: "Test",
        model: "m1",
        settings,
        tools: None,
    }
}

#[test]
fn a_cache_hit_renders_with_no_provider_call() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::new(dir.path());
    let (f, calls) = fake(32000);
    let s = RenderSettings::default();
    let ctx = ctx_in(&f, &cache, &s);
    let (first, hit) = render::render_line(&ctx, "Armed").unwrap();
    assert!(!hit);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // The same line again, and the same line with other trim and tempo-free settings.
    let (again, hit) = render::render_line(&ctx, "Armed").unwrap();
    assert!(hit);
    assert_eq!(again, first);
    let tighter = RenderSettings {
        trim_db: -40.0,
        tail_ms: 80,
        ..Default::default()
    };
    let (other, hit) = render::render_line(&ctx_in(&f, &cache, &tighter), "Armed").unwrap();
    assert!(hit, "trim and tail are not part of the cache key");
    assert_ne!(other, first);
    assert_eq!(calls.load(Ordering::SeqCst), 1, "no second provider call");
    // Another provider speed is another take.
    let slower = RenderSettings {
        speed: 0.9,
        ..Default::default()
    };
    let (_, hit) = render::render_line(&ctx_in(&f, &cache, &slower), "Armed").unwrap();
    assert!(!hit);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    // The plan names the cost before a render.
    let p = render::plan(
        &ctx,
        &[
            "Armed".into(),
            "Disarmed".into(),
            "Disarmed".into(),
            "Armed".into(),
        ],
    );
    assert_eq!((p.lines, p.cached, p.to_render, p.chars), (4, 3, 1, 8));
}

#[test]
fn normalisation_matches_the_golden_wav() {
    // A square burst, built without floating point: the same bytes on every machine.
    let win = 160;
    let mut samples = vec![3i16; win * 8];
    for i in 0..win * 12 {
        samples.push(if (i / 18) % 2 == 0 { 8000 } else { -8000 });
    }
    samples.extend(std::iter::repeat_n(5i16, win * 12));
    let out = render::normalise(
        &Pcm {
            rate: 32000,
            samples,
        },
        &RenderSettings::default(),
        None,
    )
    .unwrap();
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/voice/golden.wav");
    if std::env::var_os("QUADCAM_UPDATE_GOLDEN").is_some() {
        std::fs::write(&golden, &out).unwrap();
    }
    let want = std::fs::read(&golden).expect("run with QUADCAM_UPDATE_GOLDEN=1 to write it");
    assert_eq!(out.len(), want.len());
    assert!(out == want, "the normalised WAV differs from the golden");
    // The shape the design asks for: RIFF, 32 kHz, 16-bit, mono; 20 ms lead, 150 ms tail.
    assert_eq!(&out[0..4], b"RIFF");
    let p = wav::read(&out).unwrap();
    assert_eq!(p.rate, 32000);
    assert_eq!(p.samples.len(), 32 * 20 + 160 * 12 + 32 * 150);
}

#[test]
fn tempo_and_rate_run_in_ffmpeg() {
    let Ok(tools) = quadcam_lib::media::find_tools() else {
        eprintln!("ffmpeg is not installed; skipping");
        return;
    };
    let take = Pcm {
        rate: 24000,
        samples: {
            let mut s = vec![0i16; 2400];
            for i in 0..24000 {
                s.push(if (i / 20) % 2 == 0 { 7000 } else { -7000 });
            }
            s.extend(vec![0i16; 2400]);
            s
        },
    };
    let at = |tempo: f64| {
        let w = render::normalise(
            &take,
            &RenderSettings {
                tempo,
                ..Default::default()
            },
            Some(&tools),
        )
        .unwrap();
        wav::read(&w).unwrap()
    };
    let (normal, fast) = (at(1.0), at(1.25));
    assert_eq!(normal.rate, 32000);
    // 1 s of speech plus 170 ms of pads; faster speech is shorter, the pads keep their length.
    let speech = |p: &Pcm| p.samples.len() as i64 - 32 * 170;
    assert!(
        (speech(&normal) - 32000).abs() < 1200,
        "{}",
        speech(&normal)
    );
    assert!((speech(&fast) - 25600).abs() < 1600, "{}", speech(&fast));
}

// ----- providers -----

#[derive(Default)]
struct RunLog {
    calls: Mutex<Vec<(String, Vec<String>, Vec<u8>)>>,
}

struct FakeRun(Arc<RunLog>);

impl Run for FakeRun {
    fn run(&self, program: &str, args: &[String], stdin: &[u8]) -> anyhow::Result<()> {
        self.0
            .calls
            .lock()
            .unwrap()
            .push((program.into(), args.to_vec(), stdin.to_vec()));
        let out = args
            .iter()
            .position(|a| a == "-o")
            .map(|i| args[i + 1].clone())
            .unwrap();
        std::fs::write(
            out,
            wav::write(&Pcm {
                rate: 32000,
                samples: vec![100, -100, 200],
            }),
        )?;
        Ok(())
    }
}

#[test]
fn say_gets_the_text_on_stdin_and_a_wav_back() {
    let log = Arc::new(RunLog::default());
    let dir = tempfile::tempdir().unwrap();
    let say = Say {
        run: Arc::new(FakeRun(log.clone())),
        dir: dir.path().to_path_buf(),
    };
    let p = say
        .render(&TtsRequest {
            text: "Battery low",
            voice: "Samantha",
            model: "",
            speed: 1.2,
            seed: 0,
        })
        .unwrap();
    assert_eq!(p.samples, vec![100, -100, 200]);
    let calls = log.calls.lock().unwrap();
    let (prog, args, stdin) = &calls[0];
    assert_eq!(prog, "/usr/bin/say");
    assert_eq!(stdin, b"Battery low");
    assert!(!args.iter().any(|a| a.contains("Battery")), "{args:?}");
    assert_eq!(&args[0..4], ["-v", "Samantha", "-r", "210"]);
    assert!(args.contains(&"--data-format=LEI16@32000".to_string()));
    assert!(!say.paid());
    assert!(
        std::fs::read_dir(dir.path()).unwrap().next().is_none(),
        "the scratch file goes"
    );
}

#[derive(Default)]
struct PostLog {
    calls: Mutex<Vec<(String, Option<String>, String)>>,
}

struct FakePost(Arc<PostLog>, Vec<u8>);

impl Post for FakePost {
    fn post(&self, url: &str, key: Option<&str>, body: &str) -> anyhow::Result<Vec<u8>> {
        self.0
            .calls
            .lock()
            .unwrap()
            .push((url.into(), key.map(Into::into), body.into()));
        Ok(self.1.clone())
    }
}

#[test]
fn the_openai_provider_posts_json_and_reads_a_wav() {
    let log = Arc::new(PostLog::default());
    let reply = wav::write(&Pcm {
        rate: 24000,
        samples: vec![1, 2, 3],
    });
    let t = OpenAi {
        post: Arc::new(FakePost(log.clone(), reply)),
        base_url: "http://127.0.0.1:8880".into(),
        key: Some("sekrit".into()),
    };
    let p = t
        .render(&TtsRequest {
            text: "Armed",
            voice: "af_heart",
            model: "kokoro",
            speed: 1.0,
            seed: 7,
        })
        .unwrap();
    assert_eq!((p.rate, p.samples.len()), (24000, 3));
    let calls = log.calls.lock().unwrap();
    let (url, key, body) = &calls[0];
    assert_eq!(url, "http://127.0.0.1:8880/v1/audio/speech");
    assert_eq!(key.as_deref(), Some("sekrit"));
    let b: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(b["input"], "Armed");
    assert_eq!(b["voice"], "af_heart");
    assert_eq!(b["model"], "kokoro");
    assert_eq!(b["response_format"], "wav");
    assert!(!body.contains("sekrit"), "the key is not in the body");
    // A server that answers with something else is told so.
    let bad = OpenAi {
        post: Arc::new(FakePost(Arc::default(), b"{\"error\":\"nope\"}".to_vec())),
        base_url: "http://127.0.0.1:8880/v1".into(),
        key: None,
    };
    let e = bad
        .render(&TtsRequest {
            text: "x",
            voice: "",
            model: "",
            speed: 1.0,
            seed: 0,
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("did not return a WAV"), "{e:#}");
}

// ----- packs -----

fn opts(id: &str) -> BuildOpts {
    BuildOpts {
        id: id.into(),
        version: "1".into(),
        voice: "Test".into(),
        lang: "en".into(),
        license: "CC BY 4.0".into(),
        attribution: "Rendered by QuadCam's tests".into(),
        lines_csv_sha: lines::builtin_sha(),
    }
}

fn few() -> Vec<Line> {
    lines::builtin()
        .unwrap()
        .into_iter()
        .filter(|l| {
            ["/armed.wav", "/lowbat.wav", "/0003.wav", "/hello.wav"]
                .iter()
                .any(|n| l.path.ends_with(n))
        })
        .collect()
}

#[test]
fn build_pack_writes_a_zip_and_an_index_entry() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::new(&dir.path().join("cache"));
    let (f, calls) = fake(32000);
    let s = RenderSettings::default();
    let ctx = ctx_in(&f, &cache, &s);
    let sp = Spelling::builtin().unwrap();
    let out = dir.path().join("out");
    let mut seen = Vec::new();
    let (zip, entry) = packs::build(
        &opts("en-test-v1"),
        &ctx,
        &few(),
        &sp,
        &out,
        &dir.path().join("work"),
        &mut |i, n| seen.push((i, n)),
    )
    .unwrap();
    assert_eq!(zip.file_name().unwrap(), "voice-en-test-v1-1.zip");
    assert_eq!(entry.lines, 4);
    assert_eq!(entry.file, "voice-en-test-v1-1.zip");
    assert_eq!(entry.sha256, sha256_hex(&std::fs::read(&zip).unwrap()));
    assert_eq!(entry.provider, "fake");
    assert_eq!(entry.model, "m1");
    assert_eq!(entry.lines_csv_sha, lines::builtin_sha());
    assert_eq!(seen.last(), Some(&(4, 4)));
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    // The index holds the entry; a second build replaces it.
    let idx = packs::read_index(&out.join("voices.json")).unwrap();
    assert_eq!(idx.packs, vec![entry.clone()]);
    packs::build(
        &opts("en-test-v1"),
        &ctx,
        &few(),
        &sp,
        &out,
        &dir.path().join("work"),
        &mut |_, _| {},
    )
    .unwrap();
    assert_eq!(
        packs::read_index(&out.join("voices.json"))
            .unwrap()
            .packs
            .len(),
        1
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        4,
        "the second build is all cache hits"
    );
    // The zip holds pack.json and the sounds at their card paths.
    let names = String::from_utf8(
        std::process::Command::new("/usr/bin/unzip")
            .args(["-Z1"])
            .arg(&zip)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(names.lines().any(|n| n == "pack.json"));
    assert!(names.lines().any(|n| n == "SOUNDS/en/armed.wav"));
    assert!(names.lines().any(|n| n == "SOUNDS/en/SYSTEM/hello.wav"));
    // The scratch folder is gone.
    assert!(!dir.path().join("work/stage-en-test-v1").exists());
}

#[test]
fn install_checks_the_hash_and_the_paths() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::new(&dir.path().join("cache"));
    let (f, _) = fake(32000);
    let s = RenderSettings::default();
    let ctx = ctx_in(&f, &cache, &s);
    let sp = Spelling::builtin().unwrap();
    let out = dir.path().join("out");
    let (zip, entry) = packs::build(
        &opts("en-test-v1"),
        &ctx,
        &few(),
        &sp,
        &out,
        &dir.path().join("work"),
        &mut |_, _| {},
    )
    .unwrap();
    let voices = dir.path().join("voices");
    let inst = packs::install(&voices, &zip, &entry).unwrap();
    assert_eq!(inst.manifest.id, "en-test-v1");
    assert!(voices.join("en-test-v1/SOUNDS/en/armed.wav").is_file());
    assert_eq!(packs::installed(&voices).len(), 1);
    let files = packs::files_of(&inst).unwrap();
    assert_eq!(files.len(), 4);
    // A changed zip does not install.
    let bad_hash = PackIndexEntry {
        sha256: "00".repeat(32),
        ..entry.clone()
    };
    let e = packs::install(&voices, &zip, &bad_hash).unwrap_err();
    assert!(e.to_string().contains("does not match the hash"), "{e}");
    // A pack that says it is another id does not install.
    let other = PackIndexEntry {
        id: "other".into(),
        sha256: entry.sha256.clone(),
        ..entry.clone()
    };
    assert!(packs::install(&voices, &zip, &other)
        .unwrap_err()
        .to_string()
        .contains("the index says"));
    // A zip with a path out of its folder does not install, and writes nothing.
    let evil = dir.path().join("evil.zip");
    std::fs::write(
        &evil,
        stored_zip(&[("pack.json", b"{}"), ("SOUNDS/en/../../x.wav", b"x")]),
    )
    .unwrap();
    let e = packs::install(
        &voices,
        &evil,
        &PackIndexEntry {
            id: "evil".into(),
            sha256: String::new(),
            ..entry.clone()
        },
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("outside its folder"), "{e:#}");
    assert!(!voices.join("evil").exists());
    assert!(!dir.path().join("x.wav").exists());
}

/// A zip of stored (uncompressed) entries, built by hand so a hostile name can be tested.
fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    fn crc(b: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for x in b {
            c ^= u32::from(*x);
            for _ in 0..8 {
                c = if c & 1 == 1 {
                    (c >> 1) ^ 0xEDB8_8320
                } else {
                    c >> 1
                };
            }
        }
        !c
    }
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, data) in files {
        let off = out.len() as u32;
        let c = crc(data);
        let hdr = |out: &mut Vec<u8>, sig: u32, central: bool| {
            out.extend(sig.to_le_bytes());
            if central {
                out.extend(20u16.to_le_bytes());
            }
            out.extend(20u16.to_le_bytes());
            out.extend(0u16.to_le_bytes());
            out.extend(0u16.to_le_bytes());
            out.extend(0u32.to_le_bytes());
            out.extend(c.to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend((name.len() as u16).to_le_bytes());
            out.extend(0u16.to_le_bytes());
        };
        hdr(&mut out, 0x0403_4b50, false);
        out.extend(name.as_bytes());
        out.extend(*data);
        hdr(&mut central, 0x0201_4b50, true);
        central.extend(0u16.to_le_bytes());
        central.extend(0u16.to_le_bytes());
        central.extend(0u16.to_le_bytes());
        central.extend(0u32.to_le_bytes());
        central.extend(off.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let start = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(start.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}
