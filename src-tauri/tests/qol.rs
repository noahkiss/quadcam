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
