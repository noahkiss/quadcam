//! Video analysis (`highlights`, `core/video.rs`) through the core, on synthetic clips with
//! the fake helper: the session's background job, the import that writes its answer, the
//! library job with its cache and `again`, log moments keeping priority, an older macOS, and
//! the MCP and CLI surfaces. No test runs the real analyzer; `intel.rs` has the one that may.

use crate::common::*;
use quadcam_lib::core::{Core, NoHooks, VideoParams};
use quadcam_lib::intel::fake::FakeHelper;
use quadcam_lib::intel::Intel;
use quadcam_lib::library as lib;
use quadcam_lib::moments::{Moment, MomentKind, Source};
use quadcam_lib::photos::Recorder;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;

fn helper() -> Arc<FakeHelper> {
    Arc::new(FakeHelper::new(Box::new(|_| None)))
}

fn core(dir: &Path, fake: &Arc<FakeHelper>, video_analysis: bool) -> Core {
    let c = Core::new(
        dir.join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_intel(Arc::new(Intel::with("/helper".into(), fake.clone())));
    let mut d = c.defaults();
    d.output_dir = Some(dir.join("out"));
    d.video_analysis = video_analysis;
    std::fs::create_dir_all(dir.join("out")).unwrap();
    c.set_defaults(d);
    c
}

/// A folder with one 7 s analog clip.
fn clips() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    make_clip(&d.path().join("PICT0001.AVI"), 7, false);
    d
}

fn item(file: &Path, key: &str) -> Option<String> {
    let items = quadcam_lib::qtmeta::read(file).unwrap();
    quadcam_lib::qtmeta::get(&items, key).map(str::to_string)
}

#[test]
fn the_session_job_finds_highlights_and_the_import_writes_them() {
    let src = clips();
    let work = tempfile::tempdir().unwrap();
    let fake = helper();
    let core = core(work.path(), &fake, true);
    core.load(Some(src.path())).unwrap();

    assert_eq!(core.analyse_video(), 1);
    let s = core.session().unwrap();
    let c = &s.clips[0];
    let v = c.video.clone().expect("the clip was analysed");
    // The fake's 20-30 s highlight lies past the 7 s clip.
    assert_eq!(v.highlights.len(), 1, "{v:?}");
    assert_eq!(v.key_frame, Some(3.0));
    let thumb = c.thumb.clone().unwrap();
    assert!(thumb.to_string_lossy().ends_with(".key.jpg"), "{thumb:?}");
    assert!(thumb.is_file());
    // No log: the plan shows the highlight, marked as video analysis.
    let m = &s.plans[0].moments;
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!(
        (m[0].kind, m[0].source),
        (MomentKind::Highlight, Source::VideoAnalysis)
    );
    assert_eq!((m[0].start, m[0].end), (2.0, 6.0));
    assert_eq!(fake.seen_files().len(), 1);
    // Analysed clips are not analysed again.
    assert_eq!(core.analyse_video(), 0);
    assert_eq!(fake.seen_files().len(), 1);

    core.import(&Default::default()).unwrap();
    let view = core.library(&Default::default()).unwrap();
    let it = &view.clips[0];
    assert_eq!(it.clip.moments.len(), 1);
    assert_eq!(it.clip.moments[0].source, Source::VideoAnalysis);
    assert_eq!(it.clip.video.as_ref().unwrap().key_frame, Some(3.0));
    // Highlights have their own item; the moments item stays for radio-log moments.
    assert!(item(&it.file, lib::KEY_VIDEO).is_some());
    assert_eq!(item(&it.file, lib::KEY_MOMENTS), None);
    assert!(quadcam_lib::highlights::cache_file(&work.path().join("cache"), &it.clip.id).is_file());

    // The library job has nothing left to do; the strips pass makes the key-frame picture.
    let r = core.library_analyze_video(&VideoParams::default()).unwrap();
    assert!(r.clips.is_empty() && r.unavailable.is_none(), "{r:?}");
    assert_eq!(fake.seen_files().len(), 1);
    core.library_strips(None).unwrap();
    let it = core.library(&Default::default()).unwrap().clips[0].clone();
    assert!(it.cover.as_ref().is_some_and(|c| c.is_file()), "{it:?}");

    // `again` runs the analyzer again, past the cache.
    let r = core
        .library_analyze_video(&VideoParams {
            ids: vec![it.clip.id.clone()],
            again: true,
        })
        .unwrap();
    assert_eq!(r.clips.len(), 1);
    assert!(!r.clips[0].cached);
    assert_eq!(r.clips[0].highlights, 1);
    assert_eq!(fake.seen_files().len(), 2);
}

#[test]
fn the_library_job_analyses_older_clips_once_and_log_moments_win() {
    let src = clips();
    let work = tempfile::tempdir().unwrap();
    let fake = helper();
    // The setting off: the session job does nothing.
    let core = core(work.path(), &fake, false);
    core.load(Some(src.path())).unwrap();
    assert_eq!(core.analyse_video(), 0);
    core.import(&Default::default()).unwrap();
    let it = core.library(&Default::default()).unwrap().clips[0].clone();
    assert!(it.clip.video.is_none() && it.clip.moments.is_empty());
    assert!(fake.seen_files().is_empty());

    let e = core
        .library_analyze_video(&VideoParams {
            ids: vec!["nosuchclip".into()],
            again: false,
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("No clip nosuchclip"), "{e:#}");

    // The explicit run works with the setting off.
    let r = core.library_analyze_video(&VideoParams::default()).unwrap();
    assert_eq!(r.clips.len(), 1, "{r:?}");
    assert_eq!(
        (r.clips[0].highlights, r.clips[0].key_frame),
        (1, Some(3.0))
    );
    assert!(!r.clips[0].cached);
    let now = core.library(&Default::default()).unwrap().clips[0].clone();
    assert_eq!(now.clip.moments[0].kind, MomentKind::Highlight);
    assert!(now.cover.is_some());
    assert!(core
        .library_analyze_video(&VideoParams::default())
        .unwrap()
        .clips
        .is_empty());

    // With the file's item gone, the cache answers: no second run.
    lib::write_keys(&now.file, &[(lib::KEY_VIDEO, String::new())]).unwrap();
    core.library_rebuild().unwrap();
    let r = core.library_analyze_video(&VideoParams::default()).unwrap();
    assert_eq!(r.clips.len(), 1);
    assert!(r.clips[0].cached, "{r:?}");
    assert_eq!(fake.seen_files().len(), 1);

    // A radio-log moment written later wins; the highlights stay in the file.
    let roll = Moment {
        kind: MomentKind::Roll,
        start: 1.0,
        end: 1.5,
        score: 0.8,
        source: Source::RadioLog,
        detail: String::new(),
    };
    lib::write_keys(
        &now.file,
        &[(lib::KEY_MOMENTS, lib::moments_value(&[roll]))],
    )
    .unwrap();
    core.library_rebuild().unwrap();
    let c = core.library(&Default::default()).unwrap().clips[0]
        .clip
        .clone();
    assert_eq!(c.moments.len(), 1);
    assert_eq!(c.moments[0].kind, MomentKind::Roll);
    assert_eq!(c.video.unwrap().highlights.len(), 1);
}

#[test]
fn an_older_macos_skips_video_analysis_silently() {
    let src = clips();
    let work = tempfile::tempdir().unwrap();
    let mut f = FakeHelper::new(Box::new(|_| None));
    f.caps = r#"{"kind":"caps","helper":1,"os":"26.1","features":[{"name":"video","available":false,"min_os":"27.0","reason":"Video analysis on this Mac needs macOS 27 or later."}]}"#.into();
    let fake = Arc::new(f);
    let core = core(work.path(), &fake, true);
    core.load(Some(src.path())).unwrap();
    assert_eq!(core.analyse_video(), 0);
    assert!(core.session().unwrap().clips[0].video.is_none());
    core.import(&Default::default()).unwrap();
    let it = core.library(&Default::default()).unwrap().clips[0].clone();
    let before = std::fs::read(&it.file).unwrap();
    let r = core.library_analyze_video(&VideoParams::default()).unwrap();
    assert!(r.unavailable.unwrap().contains("macOS 27"));
    assert!(r.clips.is_empty());
    assert!(fake.seen_files().is_empty());
    assert_eq!(std::fs::read(&it.file).unwrap(), before, "nothing written");
    assert!(!core.video_feature().available);
}

#[test]
fn a_clip_the_analyzer_cannot_read_is_reported_and_the_rest_go_on() {
    let src = tempfile::tempdir().unwrap();
    make_clip(&src.path().join("PICT0001.AVI"), 7, false);
    make_clip(&src.path().join("PICT0002.AVI"), 8, false);
    let work = tempfile::tempdir().unwrap();
    let mut f = FakeHelper::new(Box::new(|_| None));
    let first = std::sync::Mutex::new(true);
    f.see = Box::new(move |file| {
        let mut first = first.lock().unwrap();
        let fail = *first;
        *first = false;
        (!fail).then(|| quadcam_lib::intel::fake::seen(file))
    });
    let fake = Arc::new(f);
    let core = core(work.path(), &fake, false);
    core.load(Some(src.path())).unwrap();
    core.import(&Default::default()).unwrap();
    let r = core.library_analyze_video(&VideoParams::default()).unwrap();
    assert_eq!(r.clips.len(), 2, "{r:?}");
    assert_eq!(r.clips.iter().filter(|c| c.error.is_some()).count(), 1);
    assert_eq!(r.clips.iter().filter(|c| c.highlights == 1).count(), 1);
    // The failed one is still to do, and nothing failed was cached.
    let r = core.library_analyze_video(&VideoParams::default()).unwrap();
    assert_eq!(r.clips.len(), 1);
    assert!(r.clips[0].error.is_none(), "{r:?}");
}

#[test]
fn mcp_analyze_video_matches_the_core() {
    use quadcam_lib::mcp::{LocalBackend, Server};
    let src = clips();
    let work = tempfile::tempdir().unwrap();
    let fake = helper();
    let c = core(work.path(), &fake, false);
    c.load(Some(src.path())).unwrap();
    c.import(&Default::default()).unwrap();
    let mut s = Server::new(LocalBackend(Arc::new(c)));
    let r = s.call_tool("quadcam_library_files", json!({"action": "analyze_video"}));
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("1 highlights, key frame 3.0 s"), "{text}");
    let r = s.call_tool("quadcam_library_files", json!({"action": "analyze_video"}));
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Nothing to analyse"));
    let r = s.call_tool(
        "quadcam_library_files",
        json!({"action": "analyze_video", "again": true}),
    );
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("1 highlights"));
    assert_eq!(fake.seen_files().len(), 2);
    let r = s.call_tool("quadcam_library", json!({}));
    assert_eq!(r["isError"], false, "{r}");
    assert!(
        r.to_string().contains("video_analysis"),
        "moments carry their source: {r}"
    );
}

#[test]
fn the_cli_reports_video_analysis_unavailable_under_cargo() {
    let home = tempfile::tempdir().unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
        .env("HOME", home.path())
        .env("QUADCAM_PHOTOS", "dry-run")
        .env_remove("QUADCAM_INTEL")
        .args(["--json", "library", "analyze-video", "--again"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    assert!(v["result"]["unavailable"]
        .as_str()
        .unwrap()
        .contains("cargo"));
    assert_eq!(v["result"]["clips"], json!([]));
}
