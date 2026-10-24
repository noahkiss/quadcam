//! "Delete clips after import": after an export, the clip files that verified go from the
//! card or folder they came from, and nothing else does. The setting is the only consent: a
//! run (core, CLI, MCP) can turn deletion off, never on. Every card here is a temp folder.

use crate::common::*;
use quadcam_lib::core::{Core, DeletionState, ImportOptions, NoHooks};
use quadcam_lib::mcp::{LocalBackend, Server};
use quadcam_lib::media::Format;
use quadcam_lib::photos::Recorder;
use quadcam_lib::session::{Editor, PlanPatch};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

/// An analog DVR card: two clips and a file that is not a clip.
fn analog_card(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let dir = root.join("DCIM");
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("PICT0001.AVI");
    let b = dir.join("PICT0002.AVI");
    make_clip(&a, 2, true);
    make_clip(&b, 1, true);
    let other = root.join("SETTINGS.TXT");
    std::fs::write(&other, b"dvr settings").unwrap();
    (a, b, other)
}

fn core(work: &Path, delete: bool) -> Core {
    let core = Core::new(
        work.join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    );
    let mut d = core.defaults();
    d.output_dir = Some(work.join("out"));
    std::fs::create_dir_all(work.join("out")).unwrap();
    d.layout = quadcam_lib::library::Layout::Flat;
    d.delete_clips_after_import = delete;
    core.set_defaults(d);
    core
}

fn mp4() -> ImportOptions {
    ImportOptions {
        format: Some(Format::Mp4),
        ..Default::default()
    }
}

#[test]
fn dji_verified_clip_goes_and_everything_else_stays() {
    let card = tempfile::tempdir().unwrap();
    let clip = dji_card(card.path(), 2);
    let work = tempfile::tempdir().unwrap();
    let core = core(work.path(), true);
    core.load(Some(card.path())).unwrap();

    let out = core.import(&mp4()).unwrap();
    assert_eq!(out.summary.imported, 1);
    let d = out.clip_deletion.expect("the setting is on");
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].state, DeletionState::Deleted, "{:?}", d[0].reason);
    assert_eq!(d[0].path, clip);
    assert!(!clip.exists(), "the MP4 is gone");
    // Only the clip: its .SRT, MISC/ and the empty DCIM folders stay.
    assert!(clip.with_extension("SRT").is_file());
    assert!(card.path().join("MISC/FC8770.db").is_file());
    assert!(card.path().join("MISC/THM").is_dir());
    assert!(card.path().join("DCIM/DJI_001").is_dir());
    // The library copy is whole.
    let output = out.summary.results[0].output.clone().unwrap();
    assert!(output.is_file());
    for v in core.verify(None).unwrap() {
        assert!(v.ok, "{:?}", v.error);
    }
}

#[test]
fn analog_skipped_clips_and_other_files_stay() {
    let card = tempfile::tempdir().unwrap();
    let (a, b, other) = analog_card(card.path());
    let work = tempfile::tempdir().unwrap();
    let core = core(work.path(), true);
    core.load(Some(card.path())).unwrap();
    core.patch(
        &[PlanPatch {
            id: 1,
            skip: Some(true),
            ..Default::default()
        }],
        Editor::User,
    )
    .unwrap();

    let out = core.import(&mp4()).unwrap();
    let d = out.clip_deletion.unwrap();
    let by = |id: usize| d.iter().find(|x| x.id == id).unwrap();
    assert_eq!(by(0).state, DeletionState::Deleted, "{:?}", by(0).reason);
    assert_eq!(by(1).state, DeletionState::Kept);
    assert_eq!(by(1).reason.as_deref(), Some("it was skipped"));
    assert!(!a.exists());
    assert!(b.is_file(), "a skipped clip stays");
    assert!(other.is_file(), "a file that is not a clip stays");
    assert!(card.path().join("DCIM").is_dir());
}

#[test]
fn a_clip_whose_output_fails_the_recheck_stays() {
    let card = tempfile::tempdir().unwrap();
    let (a, b, _) = analog_card(card.path());
    let work = tempfile::tempdir().unwrap();
    // First import with the setting off: both verify, nothing goes.
    let core = core(work.path(), false);
    core.load(Some(card.path())).unwrap();
    let out = core.import(&mp4()).unwrap();
    assert!(out.clip_deletion.is_none());
    assert!(a.is_file() && b.is_file());

    // Damage clip 0's output, change clip 1 on the card, then run again with it on.
    let r0 = out.summary.results.iter().find(|r| r.id == 0).unwrap();
    std::fs::write(r0.output.as_ref().unwrap(), b"not a video").unwrap();
    let mut d = core.defaults();
    d.delete_clips_after_import = true;
    core.set_defaults(d);
    make_clip(&b, 2, false);

    let out = core.import(&mp4()).unwrap();
    let d = out.clip_deletion.unwrap();
    let by = |id: usize| d.iter().find(|x| x.id == id).unwrap();
    assert_eq!(by(0).state, DeletionState::Kept);
    assert!(
        by(0)
            .reason
            .as_deref()
            .unwrap()
            .contains("did not verify again"),
        "{:?}",
        by(0).reason
    );
    assert_eq!(by(1).state, DeletionState::Kept);
    assert_eq!(
        by(1).reason.as_deref(),
        Some("it changed since it was copied")
    );
    assert!(a.is_file() && b.is_file());
}

#[test]
fn setting_off_deletes_nothing() {
    let card = tempfile::tempdir().unwrap();
    let clip = dji_card(card.path(), 2);
    let work = tempfile::tempdir().unwrap();
    let core = core(work.path(), false);
    assert!(!core.defaults().delete_clips_after_import, "off by default");
    core.load(Some(card.path())).unwrap();
    let out = core.import(&mp4()).unwrap();
    assert_eq!(out.summary.imported, 1);
    assert!(out.clip_deletion.is_none());
    assert!(clip.is_file());
}

#[test]
fn keep_clips_overrides_the_setting_for_one_run() {
    let card = tempfile::tempdir().unwrap();
    let (a, b, _) = analog_card(card.path());
    let work = tempfile::tempdir().unwrap();
    let core = core(work.path(), true);
    core.load(Some(card.path())).unwrap();
    let out = core
        .import(&ImportOptions {
            keep_clips: true,
            ..mp4()
        })
        .unwrap();
    assert_eq!(out.summary.imported, 2);
    assert!(out.clip_deletion.is_none());
    assert!(a.is_file() && b.is_file());
}

fn mcp_call(s: &mut Server<LocalBackend>, tool: &str, args: Value) -> Value {
    let r = s.call_tool(tool, args);
    assert_eq!(r["isError"], false, "{tool}: {r}");
    r
}

#[test]
fn mcp_follows_the_setting_and_cannot_turn_it_on() {
    let card = tempfile::tempdir().unwrap();
    let (a, b, other) = analog_card(card.path());
    let work = tempfile::tempdir().unwrap();
    let core = Arc::new(core(work.path(), false));
    let mut s = Server::new(LocalBackend(core.clone()));
    mcp_call(&mut s, "quadcam_load_clips", json!({"source": card.path()}));

    // With the setting off, no export argument turns deletion on.
    for args in [
        json!({"keep_clips": false}),
        json!({"delete_clips": true}),
        json!({"delete_clips_after_import": true}),
    ] {
        let r = mcp_call(&mut s, "quadcam_export", args.clone());
        assert!(
            r["structuredContent"]["clip_deletion"].is_null(),
            "{args} -> {r}"
        );
        assert!(a.is_file() && b.is_file(), "{args}");
    }

    // With the setting on, keep_clips keeps them for that run.
    let mut d = core.defaults();
    d.delete_clips_after_import = true;
    core.set_defaults(d);
    let r = mcp_call(&mut s, "quadcam_export", json!({"keep_clips": true}));
    assert!(r["structuredContent"]["clip_deletion"].is_null());
    assert!(a.is_file() && b.is_file());

    // Then a plain export deletes them and says so per clip.
    let r = mcp_call(&mut s, "quadcam_export", json!({}));
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Delete clips after import: 2 deleted, 0 kept."),
        "{text}"
    );
    assert!(text.contains("Clip 0 deleted:"), "{text}");
    assert_eq!(
        r["structuredContent"]["clip_deletion"][1]["state"],
        "deleted"
    );
    assert!(!a.exists() && !b.exists());
    assert!(other.is_file());
}

/// The setting is the person's consent: an agent's settings write may turn it off, never
/// on, by its name or its file key. Then an export keeps every clip.
#[test]
fn mcp_settings_cannot_turn_it_on() {
    let card = tempfile::tempdir().unwrap();
    let (a, b, _) = analog_card(card.path());
    let work = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(work.path().join("support")).unwrap();
    let core =
        Arc::new(core(work.path(), false).with_settings(work.path().join("support/settings.json")));
    let mut s = Server::new(LocalBackend(core.clone()));
    for values in [
        json!({"delete_clips_after_import": true}),
        json!({"deleteClipsAfterImport": true}),
        json!({"default_name": "x", "delete_clips_after_import": "yes"}),
    ] {
        let r = s.call_tool(
            "quadcam_settings",
            json!({"action": "write", "values": values}),
        );
        assert_eq!(r["isError"], true, "{values} -> {r}");
        let t = r["content"][0]["text"].as_str().unwrap();
        assert!(t.contains("turn it off, not on"), "{t}");
    }
    assert!(!core.defaults().delete_clips_after_import);
    let out = work.path().join("out");
    for values in [
        json!({"delete_clips_after_import": false}),
        json!({"delete_clips_after_import": null, "output_dir": out, "layout": "flat"}),
    ] {
        mcp_call(
            &mut s,
            "quadcam_settings",
            json!({"action": "write", "values": values}),
        );
    }
    mcp_call(&mut s, "quadcam_load_clips", json!({"source": card.path()}));
    let r = mcp_call(&mut s, "quadcam_export", json!({}));
    assert!(r["structuredContent"]["clip_deletion"].is_null(), "{r}");
    assert!(a.is_file() && b.is_file());
}

/// A Photos library that tries to load another folder while the import adds to it.
struct LoadsDuringPhotos {
    core: std::sync::OnceLock<std::sync::Weak<Core>>,
    other: PathBuf,
    answer: std::sync::Mutex<Option<String>>,
}

impl quadcam_lib::photos::PhotosLibrary for LoadsDuringPhotos {
    fn add_videos(&self, _: &[PathBuf], _: Option<&str>) -> anyhow::Result<Vec<PathBuf>> {
        let core = self.core.get().and_then(|w| w.upgrade()).unwrap();
        let r = core.load(Some(&self.other));
        *self.answer.lock().unwrap() = Some(match r {
            Ok(_) => "loaded".into(),
            Err(e) => format!("{e:#}"),
        });
        Ok(Vec::new())
    }
}

/// The import holds its claim through the delete, Photos and the unmount: a load in the
/// middle is refused, so those steps finish on the session they started on.
#[test]
fn a_load_waits_for_the_whole_import() {
    let card = tempfile::tempdir().unwrap();
    let (a, b, _) = analog_card(card.path());
    let other = tempfile::tempdir().unwrap();
    analog_card(other.path());
    let work = tempfile::tempdir().unwrap();
    let photos = Arc::new(LoadsDuringPhotos {
        core: Default::default(),
        other: other.path().to_path_buf(),
        answer: Default::default(),
    });
    let core = Arc::new(Core::new(
        work.path().join("cache"),
        None,
        Arc::new(NoHooks),
        photos.clone(),
    ));
    photos.core.set(Arc::downgrade(&core)).unwrap();
    let mut d = core.defaults();
    d.output_dir = Some(work.path().join("out"));
    std::fs::create_dir_all(work.path().join("out")).unwrap();
    d.layout = quadcam_lib::library::Layout::Flat;
    d.delete_clips_after_import = true;
    core.set_defaults(d);
    core.load(Some(card.path())).unwrap();
    let out = core
        .import(&ImportOptions {
            add_to_photos: true,
            ..mp4()
        })
        .unwrap();
    let answer = photos.answer.lock().unwrap().clone().unwrap();
    assert!(answer.contains("Another step is still running"), "{answer}");
    assert_eq!(out.summary.imported, 2);
    assert!(!a.exists() && !b.exists());
    let source = core.session().unwrap().source;
    assert!(
        source.ends_with(card.path().file_name().unwrap()),
        "{}",
        source.display()
    );
}

/// A half-written clip imports as its recovered copy, which may hold less than the card's
/// file: that file stays, unless Keep originals put it in the library.
#[test]
fn a_recovered_clip_stays_unless_its_original_was_kept() {
    for keep_originals in [false, true] {
        let card = tempfile::tempdir().unwrap();
        let (a, _, _) = analog_card(card.path());
        let long = card.path().join("long.tmp");
        make_clip(&long, 4, true);
        let half = card.path().join("DCIM/PICT0003.AVI");
        truncate_copy(&long, &half, 0.5);
        std::fs::remove_file(&long).unwrap();
        let work = tempfile::tempdir().unwrap();
        let core = core(work.path(), true);
        core.load(Some(card.path())).unwrap();
        let s = core.session().unwrap();
        let id = s
            .clips
            .iter()
            .find(|c| c.name == "PICT0003.AVI")
            .unwrap()
            .id;
        assert!(s.clips[id].recovered.is_some(), "{}", s.clips[id].detail);
        let out = core
            .import(&ImportOptions {
                keep_originals: Some(keep_originals),
                ..mp4()
            })
            .unwrap();
        let d = out.clip_deletion.unwrap();
        let row = d.iter().find(|x| x.id == id).unwrap();
        assert!(!a.exists(), "a whole clip goes");
        if keep_originals {
            assert_eq!(row.state, DeletionState::Deleted, "{row:?}");
            assert!(!half.exists());
        } else {
            assert_eq!(row.state, DeletionState::Kept);
            assert!(
                row.reason.as_deref().unwrap().contains("recovered"),
                "{row:?}"
            );
            assert!(half.is_file());
        }
    }
}

/// Runs `quadcam-cli --json <args>` with HOME in `home`; returns (exit code, JSON).
fn cli(home: &Path, args: &[&str]) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
        .env("HOME", home)
        .env("QUADCAM_PHOTOS", "dry-run")
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let v: Value =
        serde_json::from_str(text.trim()).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"));
    (out.status.code().unwrap_or(-1), v)
}

#[test]
fn cli_follows_the_setting_and_cannot_turn_it_on() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    let card = tempfile::tempdir().unwrap();
    let (a, b, other) = analog_card(card.path());
    let src = card.path().to_str().unwrap();
    let ok = |args: &[&str]| {
        let (code, v) = cli(h, args);
        assert_eq!(code, 0, "{args:?} -> {v}");
        v["result"].clone()
    };

    // Setting off (the default). A plan file that asks for deletion changes nothing, and
    // there is no flag for it.
    ok(&["stage", src]);
    ok(&["analyze"]);
    let plan = h.join("plan.json");
    std::fs::write(
        &plan,
        r#"{"clips":[],"format":"mp4","delete_clips":true,"delete_clips_after_import":true,"keep_clips":false}"#,
    )
    .unwrap();
    let r = ok(&["import", "--plan", plan.to_str().unwrap()]);
    assert!(r["clip_deletion"].is_null(), "{r}");
    let (code, _) = cli(h, &["import", "--delete-clips"]);
    assert_eq!(code, 2, "no such flag");
    assert!(a.is_file() && b.is_file());

    // Setting on: --keep-clips keeps them for one run, a plain import deletes them.
    ok(&["settings", "set", "delete_clips_after_import=true"]);
    let r = ok(&["import", "--keep-clips"]);
    assert!(r["clip_deletion"].is_null());
    assert!(a.is_file() && b.is_file());
    let r = ok(&["import"]);
    let d = r["clip_deletion"].as_array().unwrap();
    assert_eq!(d.len(), 2);
    assert!(d.iter().all(|x| x["state"] == "deleted"), "{r}");
    assert!(!a.exists() && !b.exists());
    assert!(other.is_file());
}

#[test]
fn a_path_outside_the_card_is_never_deleted() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    let card = tempfile::tempdir().unwrap();
    let (a, b, _) = analog_card(card.path());
    let elsewhere = tempfile::tempdir().unwrap();
    let copy = elsewhere.path().join("PICT0001.AVI");
    std::fs::copy(&a, &copy).unwrap();
    let ok = |args: &[&str]| {
        let (code, v) = cli(h, args);
        assert_eq!(code, 0, "{args:?} -> {v}");
        v["result"].clone()
    };
    ok(&["stage", card.path().to_str().unwrap()]);
    ok(&["analyze"]);
    // Point clip 0 at an identical file outside the card, as a damaged session might.
    let file = h.join("Library/Caches/app.quadcam/session.json");
    let mut session: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    session["clips"][0]["card_path"] = json!(copy);
    std::fs::write(&file, serde_json::to_vec(&session).unwrap()).unwrap();

    ok(&["settings", "set", "delete_clips_after_import=true"]);
    let r = ok(&["import"]);
    let d = &r["clip_deletion"];
    assert_eq!(d[0]["state"], "kept", "{r}");
    assert!(
        d[0]["reason"]
            .as_str()
            .unwrap()
            .starts_with("it is outside"),
        "{r}"
    );
    assert_eq!(d[1]["state"], "deleted", "{r}");
    assert!(copy.is_file() && a.is_file());
    assert!(!b.exists());
}
