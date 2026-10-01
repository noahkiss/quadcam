//! Core behaviour behind the app's quality-of-life features: previews made after analyse,
//! batched plan edits, dropped files, and putting trashed clips back.

mod common;

use common::*;
use quadcam_lib::core::{Core, NoHooks};
use quadcam_lib::photos::Recorder;
use quadcam_lib::session::{Editor, PlanPatch};
use std::path::Path;
use std::sync::Arc;

fn core(dir: &Path) -> Core {
    let c = Core::new(
        dir.join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    );
    let mut d = c.defaults();
    d.output_dir = Some(dir.join("out"));
    std::fs::create_dir_all(dir.join("out")).unwrap();
    c.set_defaults(d);
    c
}

fn clips_folder() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    make_clip(&d.path().join("PICT0001.AVI"), 2, true);
    make_clip(&d.path().join("PICT0002.AVI"), 1, true);
    d
}

#[test]
fn previews_are_made_ahead_and_reused() {
    let dir = tempfile::tempdir().unwrap();
    let c = core(dir.path());
    let src = clips_folder();
    c.load(Some(src.path())).unwrap();
    assert_eq!(c.make_previews(), 2);
    let a = c.preview(0).unwrap();
    let b = c.preview(1).unwrap();
    assert!(a.is_file() && b.is_file());
    assert_ne!(a, b);
    let made = std::fs::metadata(&a).unwrap().modified().unwrap();
    assert_eq!(c.preview(0).unwrap(), a);
    assert_eq!(std::fs::metadata(&a).unwrap().modified().unwrap(), made);
}

#[test]
fn one_patch_call_edits_every_clip() {
    let dir = tempfile::tempdir().unwrap();
    let c = core(dir.path());
    let src = clips_folder();
    c.load(Some(src.path())).unwrap();
    let day = chrono::NaiveDate::from_ymd_opt(2026, 9, 27).unwrap();
    let s = c
        .patch(
            &[0, 1].map(|id| PlanPatch {
                id,
                date: Some(day),
                note: Some("windy".into()),
                ..Default::default()
            }),
            Editor::User,
        )
        .unwrap();
    assert!(s.plans.iter().all(|p| p.date == day && p.note == "windy"));
}

#[test]
fn trashed_clips_can_be_put_back() {
    let dir = tempfile::tempdir().unwrap();
    let c = core(dir.path()).with_trash(Arc::new(quadcam_lib::trash::DirTrash(
        dir.path().join("trash"),
    )));
    let src = clips_folder();
    c.load(Some(src.path())).unwrap();
    c.patch(
        &[PlanPatch {
            id: 0,
            name: Some("loops".into()),
            cuts: Some(vec![quadcam_lib::moments::Span {
                start: 0.0,
                end: 1.0,
            }]),
            ..Default::default()
        }],
        Editor::User,
    )
    .unwrap();
    c.import(&Default::default()).unwrap();
    let lib = c.library(&Default::default()).unwrap();
    assert_eq!(lib.clips.len(), 2);
    let loops = lib.clips.iter().find(|x| x.name == "loops").unwrap();
    let id = loops.clip.id.clone();
    c.library_rate(std::slice::from_ref(&id), Some(4), None)
        .unwrap();

    let r = c.library_trash(std::slice::from_ref(&id)).unwrap();
    assert_eq!(r.moved.len(), 2, "the clip and its cut: {r:?}");
    assert!(!loops.file.exists());
    assert_eq!(c.library(&Default::default()).unwrap().clips.len(), 1);

    assert_eq!(
        c.library_untrash(&r.moved).unwrap(),
        std::slice::from_ref(&id)
    );
    assert!(loops.file.is_file());
    let lib = c.library(&Default::default()).unwrap();
    let back = lib.clips.iter().find(|x| x.clip.id == id).unwrap();
    assert_eq!(back.clip.rating, 4, "details live in the file");
    assert_eq!(back.clip.cuts.len(), 1);
    // A second put-back finds nothing in the Trash and changes nothing.
    assert!(c.library_untrash(&r.moved).is_err());
}
