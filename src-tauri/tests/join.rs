//! A recording the DVR split into files imports as one clip: one encode over every file,
//! verified against their sum, every file's identity in the library, and "Delete clips
//! after import" deleting a file only when the joined output verified. The fixture splits
//! one synthetic recording with ffmpeg the way an Echo does (600 s, then the rest); no real
//! Echo files are in the test corpus.

mod common;

use common::tools;
use quadcam_lib::core::{Core, DeletionState, ImportOptions, LogChoice, NoHooks};
use quadcam_lib::library::{self, Filter, Layout};
use quadcam_lib::media::Format;
use quadcam_lib::photos::Recorder;
use quadcam_lib::session::{Editor, PlanPatch};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

/// A card with one 630 s recording split into PICT0001.AVI (600 s) and PICT0002.AVI
/// (30 s), then a 3 s recording, PICT0003.AVI. Small frames keep it fast.
fn split_card(root: &Path, work: &Path) -> [PathBuf; 3] {
    let dcim = root.join("DCIM");
    std::fs::create_dir_all(&dcim).unwrap();
    let ff = |args: &[&str]| {
        let st = Command::new(tools().ffmpeg)
            .args(["-v", "error", "-y"])
            .args(args)
            .status()
            .unwrap();
        assert!(st.success(), "ffmpeg {args:?}");
    };
    let make = |secs: &str, out: &Path| {
        ff(&[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=160x120:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=8000",
            "-t",
            secs,
            "-c:v",
            "mjpeg",
            "-q:v",
            "10",
            "-pix_fmt",
            "yuvj420p",
            "-c:a",
            "pcm_s16le",
            "-ac",
            "1",
            "-f",
            "avi",
            out.to_str().unwrap(),
        ])
    };
    let long = work.join("long.avi");
    make("630", &long);
    let [a, b, c] = ["PICT0001.AVI", "PICT0002.AVI", "PICT0003.AVI"].map(|n| dcim.join(n));
    let l = long.to_str().unwrap();
    ff(&[
        "-i",
        l,
        "-t",
        "600",
        "-map",
        "0",
        "-c",
        "copy",
        "-f",
        "avi",
        a.to_str().unwrap(),
    ]);
    ff(&[
        "-ss",
        "600",
        "-i",
        l,
        "-map",
        "0",
        "-c",
        "copy",
        "-f",
        "avi",
        b.to_str().unwrap(),
    ]);
    make("3", &c);
    [a, b, c]
}

fn core(work: &Path, delete: bool, keep_originals: bool) -> Core {
    let core = Core::new(
        work.join("cache"),
        Some(work.join("session.json")),
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    );
    let mut d = core.defaults();
    d.output_dir = Some(work.join("library"));
    std::fs::create_dir_all(work.join("library")).unwrap();
    d.layout = Layout::Day;
    d.delete_clips_after_import = delete;
    d.keep_originals = keep_originals;
    core.set_defaults(d);
    core
}

fn opts(format: Format) -> ImportOptions {
    ImportOptions {
        format: Some(format),
        ..Default::default()
    }
}

#[test]
fn a_split_recording_imports_as_one_clip_with_every_files_identity() {
    let card = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let [a, b, _] = split_card(card.path(), work.path());
    let core = core(work.path(), false, true);
    let s = core.load(Some(card.path())).unwrap();
    let head = &s.clips[0];
    let j = head
        .join
        .as_ref()
        .expect("PICT0001 and PICT0002 are one recording");
    assert!(j.on);
    assert_eq!(j.parts, vec![1]);
    assert_eq!(s.clips[1].part_of, Some(0));
    assert!(s.clips[2].join.is_none() && s.clips[2].part_of.is_none());
    assert!((head.duration - 630.0).abs() < 0.2, "{}", head.duration);
    assert_eq!(head.probe.as_ref().unwrap().video_packets, 18900);

    // MP4: one H.264 encode over both files.
    let out = core.import(&opts(Format::Mp4)).unwrap();
    assert_eq!(out.summary.imported, 2, "{:?}", out.summary.results);
    assert!(
        out.summary.results.iter().all(|r| r.id != 1),
        "the part has no result of its own"
    );
    for v in core.verify(None).unwrap() {
        assert!(v.ok, "{:?}", v.error);
    }
    let r0 = out.summary.results.iter().find(|r| r.id == 0).unwrap();
    let output = r0.output.clone().unwrap();
    let p = quadcam_lib::media::probe(&tools(), &output).unwrap();
    assert_eq!(p.video_packets, 18900, "every frame of both files");

    // The library knows both files: the clip carries the second one's fingerprint.
    let lib = core.library(&Filter::default()).unwrap();
    let c = lib
        .clips
        .iter()
        .find(|c| c.clip.path.file_name() == output.file_name())
        .unwrap();
    assert_eq!(c.clip.id, s.clips[0].key);
    assert_eq!(c.clip.parts.len(), 1);
    assert_eq!(c.clip.parts[0].source, s.clips[1].key);
    assert_eq!(c.clip.dvr_files(), ["PICT0001.AVI", "PICT0002.AVI"]);
    // Inserting the same card again: nothing is new.
    let st = core.card_status(card.path()).unwrap();
    assert_eq!((st.clips, st.new), (3, 0));
    // Both originals are kept, byte for byte, under the clip's stem.
    let orig = c.clip.original.clone().expect("kept original");
    let root = work.path().join("library");
    let orig = root.join(orig);
    let part2 = orig.with_extension("part2.avi");
    assert_eq!(std::fs::read(&orig).unwrap(), std::fs::read(&a).unwrap());
    assert_eq!(std::fs::read(&part2).unwrap(), std::fs::read(&b).unwrap());
    assert!(c.clip.files(&root).contains(&part2));

    // A rebuild from the files alone keeps the parts; a rename takes .part2 along.
    core.library_rebuild().unwrap();
    let again = core.library(&Filter::default()).unwrap();
    let c = again
        .clips
        .iter()
        .find(|x| x.clip.id == s.clips[0].key)
        .unwrap();
    assert_eq!(c.clip.parts.len(), 1);
    let renamed = core.library_rename(&c.clip.id, "long one").unwrap();
    let new_orig = root.join(renamed.original.as_ref().unwrap());
    assert!(new_orig.with_extension("part2.avi").is_file());
    assert!(!part2.exists());
    assert!(matches!(
        library::read_file(&root, &renamed.path).unwrap(),
        library::Found::Clip(c) if c.parts.len() == 1
    ));
}

#[test]
fn keep_separate_per_run_per_clip_and_by_setting() {
    let card = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    split_card(card.path(), work.path());
    let core = core(work.path(), false, false);
    // This run keeps them apart; the recording is still known.
    let s = core.load_with(Some(card.path()), Some(false)).unwrap();
    assert!(!s.clips[0].join.as_ref().unwrap().on);
    assert!((s.clips[0].duration - 600.0).abs() < 0.1);
    assert_eq!(s.clips[1].part_of, None);
    // Joined by hand, then apart again.
    let joined = |on: bool| {
        core.patch(
            &[PlanPatch {
                id: 0,
                joined: Some(on),
                ..Default::default()
            }],
            Editor::User,
        )
        .unwrap()
    };
    assert_eq!(joined(true).clips[1].part_of, Some(0));
    let s = joined(false);
    assert_eq!(s.clips[1].part_of, None);
    let out = core.import(&opts(Format::Mov)).unwrap();
    assert_eq!(out.summary.imported, 3, "{:?}", out.summary.results);
    // Imported: the files cannot be joined now.
    assert!(core
        .patch(
            &[PlanPatch {
                id: 0,
                joined: Some(true),
                ..Default::default()
            }],
            Editor::User,
        )
        .is_err());

    // The setting off: apart; a run may turn it on.
    let work2 = tempfile::tempdir().unwrap();
    let core = self::core(work2.path(), false, false);
    let mut d = core.defaults();
    d.join_split_recordings = false;
    core.set_defaults(d);
    let s = core.load(Some(card.path())).unwrap();
    assert!(!s.clips[0].join.as_ref().unwrap().on);
    let s = core.load_with(Some(card.path()), Some(true)).unwrap();
    assert!(s.clips[0].join.as_ref().unwrap().on);
    // A re-analysis keeps the run's choice.
    let s = core.analyse().unwrap();
    assert!(s.clips[0].join.as_ref().unwrap().on);
    core.plan_dates(LogChoice::None, None).unwrap();
}

#[test]
fn delete_after_import_takes_every_file_only_when_the_joined_output_verifies() {
    let card = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let [a, b, c] = split_card(card.path(), work.path());
    // First import with the setting off: everything stays.
    let core = core(work.path(), false, false);
    core.load(Some(card.path())).unwrap();
    let out = core.import(&opts(Format::Mov)).unwrap();
    assert!(out.clip_deletion.is_none());
    // Damage the joined output, then run again with the setting on.
    let r0 = out.summary.results.iter().find(|r| r.id == 0).unwrap();
    std::fs::write(r0.output.as_ref().unwrap(), b"not a video").unwrap();
    let mut d = core.defaults();
    d.delete_clips_after_import = true;
    core.set_defaults(d);
    let out = core.import(&opts(Format::Mov)).unwrap();
    let d = out.clip_deletion.unwrap();
    let by = |id: usize| d.iter().find(|x| x.id == id).unwrap().clone();
    for id in [0, 1] {
        assert_eq!(by(id).state, DeletionState::Kept, "clip {id}");
        assert!(
            by(id).reason.unwrap().contains("did not verify again"),
            "clip {id}"
        );
    }
    assert_eq!(by(2).state, DeletionState::Deleted, "{:?}", by(2).reason);
    assert!(a.is_file() && b.is_file() && !c.exists());

    // A fresh import of the same files with the setting on: both files of the recording go.
    let work2 = tempfile::tempdir().unwrap();
    let core = self::core(work2.path(), true, false);
    core.load(Some(card.path())).unwrap();
    let out = core.import(&opts(Format::Mov)).unwrap();
    let d = out.clip_deletion.unwrap();
    assert_eq!(d.len(), 2);
    assert!(d.iter().all(|x| x.state == DeletionState::Deleted), "{d:?}");
    assert!(!a.exists() && !b.exists());
}
