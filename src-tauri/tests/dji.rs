//! A DJI card end to end on a synthetic DJI-like MP4: stage, analyse, date from the clip
//! clock, import by copy (MP4) and by remux (MOV), verify, and read the metadata back.
//! The synthetic clip has no DJI data streams (see `common::make_dji_clip`); the manual run
//! on a real recording covers those.

mod common;

use chrono::{Local, NaiveDate, TimeZone, Utc};
use common::*;
use quadcam_lib::core::{Core, ImportOptions, LogChoice, NoHooks};
use quadcam_lib::media::Format;
use quadcam_lib::metadata::Profile;
use quadcam_lib::photos::Recorder;
use quadcam_lib::pipeline::{ClipStatus, DateSource, Outcome};
use quadcam_lib::sources::{EncodePlan, SourceKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const NAME: &str = "DJI_20260105120000_0001_D.MP4";

/// A card like an O4 air unit shows over USB: `DCIM/DJI_001/` with one clip and its `.SRT`,
/// and DJI's `MISC/` housekeeping.
fn dji_card(root: &Path) -> PathBuf {
    let dir = root.join("DCIM/DJI_001");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(root.join("MISC/THM")).unwrap();
    std::fs::write(root.join("MISC/FC8770.db"), b"housekeeping").unwrap();
    let clip = dir.join(NAME);
    make_dji_clip(&clip, 3, "2026-01-05T17:00:00Z");
    std::fs::write(
        clip.with_extension("SRT"),
        "1\n00:00:00,000 --> 00:00:01,000\nsubtitle\n",
    )
    .unwrap();
    clip
}

fn core(work: &Path) -> Core {
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
    d.profiles = vec![
        Profile {
            name: "Analog quad".into(),
            video_system: "Analog".into(),
            ..Default::default()
        },
        Profile {
            name: "DJI quad".into(),
            video_system: "DJI".into(),
            camera_make: "DJI".into(),
            camera_model: "O4".into(),
            edgetx_models: vec!["NOT-IN-THIS-TEST".into()],
            ..Default::default()
        },
    ];
    d.default_profile = Some("Analog quad".into());
    core.set_defaults(d);
    core
}

fn noon_jan_5() -> chrono::DateTime<Utc> {
    Local
        .with_ymd_and_hms(2026, 1, 5, 12, 0, 0)
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn dji_clip_imports_by_copy_with_its_clock() {
    let card = tempfile::tempdir().unwrap();
    let src = dji_card(card.path());
    let work = tempfile::tempdir().unwrap();
    let core = core(work.path());

    let s = core.stage(Some(card.path())).unwrap();
    assert_eq!(s.kind, SourceKind::Dji);
    assert_eq!(s.clips.len(), 1, "MISC is ignored");
    assert_eq!(s.clips[0].rel, format!("DCIM/DJI_001/{NAME}"));
    assert_eq!(s.clips[0].sidecars.len(), 1, "the .SRT is staged too");

    let s = core.analyse().unwrap();
    let c = &s.clips[0];
    assert_eq!(c.status, ClipStatus::Ok, "{}", c.detail);
    let p = c.probe.as_ref().unwrap();
    assert_eq!(
        p.video_streams, 1,
        "the cover picture is not a video stream"
    );
    assert_eq!(p.video_packets, 90);
    assert_eq!((p.width, p.height), (Some(640), Some(360)));
    assert_eq!(c.clock, Some(noon_jan_5()), "the name's local time");
    assert!(c.thumb.as_ref().unwrap().is_file());
    assert!(c.signal.is_none(), "no dead-air scan for DJI");
    assert_eq!(
        quadcam_lib::sources::get(c.kind).encode_plan(p, Format::Mp4),
        EncodePlan::Copy
    );

    let s = core.plan_dates(LogChoice::None, None).unwrap();
    let plan = &s.plans[0];
    assert_eq!(plan.source, DateSource::Clip);
    assert_eq!(plan.date, NaiveDate::from_ymd_opt(2026, 1, 5).unwrap());
    assert_eq!(plan.time.unwrap().to_string(), "12:00:00");

    let out = core
        .import(&ImportOptions {
            format: Some(Format::Mp4),
            keep_originals: Some(true),
            ..Default::default()
        })
        .unwrap();
    let r = &out.summary.results[0];
    assert_eq!(r.outcome, Outcome::Verified, "{:?}", r.error);
    assert_eq!(
        r.encoder,
        Some(quadcam_lib::media::Encoded::Copy),
        "a byte copy, no encoder"
    );
    let file = r.output.clone().unwrap();
    assert_eq!(file.extension().unwrap(), "mp4");
    // A byte copy plus the new metadata: the media data is the source's.
    let (a, b) = (
        std::fs::metadata(&src).unwrap().len(),
        std::fs::metadata(&file).unwrap().len(),
    );
    assert!(b >= a && b < a + 8192, "source {a} bytes, output {b}");
    for v in core.verify(None).unwrap() {
        assert!(v.ok, "{:?}", v.error);
    }

    // The QuickTime keys, the movie time, and DJI's own comment, which stays.
    let items = quadcam_lib::qtmeta::read(&file).unwrap();
    let get = |k: &str| quadcam_lib::qtmeta::get(&items, k).map(str::to_string);
    assert_eq!(
        get("com.apple.quicktime.description").as_deref(),
        Some(format!("DJI {NAME}; date source: clip clock").as_str())
    );
    assert!(get("com.apple.quicktime.creationdate")
        .unwrap()
        .starts_with("2026-01-05T12:00:00"));
    assert_eq!(get("com.apple.quicktime.make").as_deref(), Some("DJI"));
    assert_eq!(get("app.quadcam.video_system").as_deref(), Some("DJI"));
    assert_eq!(get("app.quadcam.time").as_deref(), Some("clip"));
    let tags = format_tags(&file);
    assert_eq!(tags["comment"], "EIS:RS;FOV:Linear;");
    let ct: chrono::DateTime<Utc> = tags["creation_time"].as_str().unwrap().parse().unwrap();
    assert_eq!(ct, noon_jan_5());

    // The original and its subtitle file sit together, under the output's stem.
    let orig = r.original.clone().unwrap();
    assert_eq!(orig.extension().unwrap(), "mp4");
    assert_eq!(
        std::fs::metadata(&orig).unwrap().len(),
        std::fs::metadata(&src).unwrap().len()
    );
    assert!(orig.with_extension("srt").is_file());

    // The library reads the clip back with its time.
    let lib = core.library(&Default::default()).unwrap();
    let item = lib
        .clips
        .iter()
        .find(|c| c.clip.path.ends_with(file.file_name().unwrap()))
        .unwrap();
    assert_eq!(item.clip.time.as_deref(), Some("12:00"));

    // A DJI card is never formatted.
    let e = core.format_plan(None).unwrap_err().to_string();
    assert!(
        e.starts_with("Refused: QuadCam does not format DJI cards"),
        "{e}"
    );
}

#[test]
fn dji_clip_remuxes_into_mov() {
    let card = tempfile::tempdir().unwrap();
    dji_card(card.path());
    let work = tempfile::tempdir().unwrap();
    let core = core(work.path());
    core.load(Some(&card.path().join("DCIM/DJI_001"))).unwrap();
    let out = core
        .import(&ImportOptions {
            format: Some(Format::Mov),
            ..Default::default()
        })
        .unwrap();
    let r = &out.summary.results[0];
    assert_eq!(r.outcome, Outcome::Verified, "{:?}", r.error);
    assert_eq!(r.encoder, Some(quadcam_lib::media::Encoded::Remux));
    let file = r.output.clone().unwrap();
    assert_eq!(file.extension().unwrap(), "mov");
    let p = quadcam_lib::media::probe(&tools(), &file).unwrap();
    assert_eq!(p.video_packets, 90);
    assert_eq!(
        format_tags(&file)["com.apple.quicktime.description"],
        format!("DJI {NAME}; date source: clip clock")
    );
}

/// The kept `.srt` follows its original through a rename, a redate to a new day, and the
/// Trash.
#[test]
fn kept_srt_moves_with_its_clip() {
    let card = tempfile::tempdir().unwrap();
    dji_card(card.path());
    let work = tempfile::tempdir().unwrap();
    let trash = work.path().join("trash");
    std::fs::create_dir_all(&trash).unwrap();
    let core = core(work.path()).with_trash(Arc::new(quadcam_lib::trash::DirTrash(trash.clone())));
    let mut d = core.defaults();
    d.layout = quadcam_lib::library::Layout::YearDay;
    core.set_defaults(d);
    let out_dir = work.path().join("out");

    core.stage(Some(card.path())).unwrap();
    core.analyse().unwrap();
    core.plan_dates(LogChoice::None, None).unwrap();
    let out = core
        .import(&ImportOptions {
            format: Some(Format::Mp4),
            keep_originals: Some(true),
            ..Default::default()
        })
        .unwrap();
    let r = &out.summary.results[0];
    assert_eq!(r.outcome, Outcome::Verified, "{:?}", r.error);
    assert!(r.original.as_ref().unwrap().with_extension("srt").is_file());

    let id = core.library(&Default::default()).unwrap().clips[0]
        .clip
        .id
        .clone();
    let srt_of = |c: &quadcam_lib::library::LibClip| {
        out_dir
            .join(c.original.as_ref().unwrap())
            .with_extension("srt")
    };

    // Rename: the original and its subtitle file take the new stem.
    let before = core.library_rename(&id, "x").unwrap();
    let c = core.library_rename(&id, "fence flips").unwrap();
    let orig = out_dir.join(c.original.as_ref().unwrap());
    assert!(
        orig.ends_with("2026/2026-01-05/originals/2026-01-05_fence_flips.mp4"),
        "{}",
        orig.display()
    );
    assert!(srt_of(&c).is_file(), "the .srt moved with the rename");
    assert!(!srt_of(&before).exists(), "no .srt left under the old stem");
    assert_eq!(c.sidecars(&out_dir), [srt_of(&c)]);

    // Redate to a new day: the subtitle file moves to that day's originals/.
    let day = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
    let c2 = core
        .library_edit(
            &id,
            &quadcam_lib::core::LibEdit {
                date: Some(day),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(srt_of(&c2).ends_with("2026/2026-01-07/originals/2026-01-07_fence_flips.srt"));
    assert!(srt_of(&c2).is_file());
    assert!(
        !out_dir.join("2026/2026-01-05").exists(),
        "the old day folder is gone"
    );

    // Trash: the subtitle file goes too, and comes back with the clip.
    let srt = srt_of(&c2);
    let rep = core.library_trash(std::slice::from_ref(&id)).unwrap();
    assert!(rep.failed.is_empty(), "{:?}", rep.failed);
    assert!(rep.trashed.contains(&srt), "{:?}", rep.trashed);
    assert!(!srt.exists());
}
