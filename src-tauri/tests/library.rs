//! The library: where imports land, the index and its rebuild from the files, ratings in
//! the files, adopting an older export folder, and cuts removed after they were exported.

mod common;

use chrono::NaiveDate;
use common::{make_clip, tools};
use quadcam_lib::core::{Core, ImportOptions, LibEdit, LogChoice, NoHooks};
use quadcam_lib::library::{self, Filter, Flag, Layout};
use quadcam_lib::media::Format;
use quadcam_lib::metadata::Place;
use quadcam_lib::moments::Span;
use quadcam_lib::photos::Recorder;
use quadcam_lib::session::{Defaults, Editor, PlanPatch};
use quadcam_lib::trash::DirTrash;
use quadcam_lib::trim::{CutChange, RemovedCuts};
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct Lab {
    _dir: tempfile::TempDir,
    root: PathBuf,
    trash: PathBuf,
    src: PathBuf,
    core: Core,
}

fn lab(layout: Layout, place_folders: bool, keep_originals: bool) -> Lab {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("library");
    std::fs::create_dir(&root).unwrap();
    let src = dir.path().join("card");
    std::fs::create_dir_all(src.join("DCIM")).unwrap();
    make_clip(&src.join("DCIM/PICT0001.AVI"), 3, true);
    make_clip(&src.join("DCIM/PICT0002.AVI"), 2, false);
    let trash = dir.path().join("trash");
    let core = Core::new(
        dir.path().join("cache"),
        Some(dir.path().join("session.json")),
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_trash(Arc::new(DirTrash(trash.clone())));
    core.set_defaults(Defaults {
        output_dir: Some(root.clone()),
        format: Format::Mp4,
        layout,
        place_folders,
        keep_originals,
        places: vec![Place {
            name: "Home field".into(),
            lat: 40.6892,
            lon: -74.0445,
        }],
        ..Defaults::default()
    });
    Lab {
        _dir: dir,
        root,
        trash,
        src,
        core,
    }
}

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 27).unwrap()
}

/// Loads the card folder, names and dates the clips, puts clip 0 at a place with one cut,
/// and imports.
fn import(l: &Lab) {
    l.core.stage(Some(&l.src)).unwrap();
    l.core.analyse().unwrap();
    l.core.plan_dates(LogChoice::None, None).unwrap();
    l.core
        .patch(
            &[
                PlanPatch {
                    id: 0,
                    name: Some("backyard loops".into()),
                    note: Some("two flips".into()),
                    date: Some(day()),
                    place: Some("Home field".into()),
                    cuts: Some(vec![Span {
                        start: 0.5,
                        end: 1.5,
                    }]),
                    ..Default::default()
                },
                PlanPatch {
                    id: 1,
                    name: Some("gap run".into()),
                    date: Some(day()),
                    ..Default::default()
                },
            ],
            Editor::User,
        )
        .unwrap();
    let out = l.core.import(&ImportOptions::default()).unwrap();
    assert_eq!(out.summary.imported, 2, "{:?}", out.summary.results);
}

fn all(l: &Lab) -> Vec<quadcam_lib::core::LibItem> {
    l.core.library(&Filter::default()).unwrap().clips
}

fn by_name(l: &Lab, name: &str) -> quadcam_lib::core::LibItem {
    all(l)
        .into_iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no clip {name}"))
}

#[test]
fn imports_land_in_year_and_day_folders_with_place_and_originals() {
    let l = lab(Layout::YearDay, true, true);
    import(&l);
    let at_place = l.root.join("2026/2026-09-27 Home field");
    assert!(at_place.join("2026-09-27_backyard_loops.mp4").is_file());
    assert!(at_place
        .join("2026-09-27_backyard_loops_cut1.mp4")
        .is_file());
    assert!(at_place
        .join("originals/2026-09-27_backyard_loops.avi")
        .is_file());
    // No place: the plain day folder.
    assert!(l
        .root
        .join("2026/2026-09-27/2026-09-27_gap_run.mp4")
        .is_file());

    // The index knows both, as the last import, with the cut and the original.
    let v = l.core.library(&Filter::default()).unwrap();
    assert_eq!(v.clips.len(), 2);
    assert!(v.clips.iter().all(|c| c.last_import));
    assert_eq!(v.unindexed, 0);
    let a = by_name(&l, "backyard loops");
    assert_eq!(a.clip.date, day());
    assert_eq!(a.clip.note, "two flips");
    assert_eq!(a.clip.place.as_deref(), Some("Home field"));
    assert_eq!(a.clip.cuts.len(), 1);
    assert!(a.clip.original.is_some());
    assert_eq!(a.clip.dvr.as_deref(), Some("PICT0001.AVI"));
    assert!((a.clip.duration - 3.0).abs() < 0.2, "{}", a.clip.duration);
    // Identity is the DVR content, not the file name.
    let s = l.core.session().unwrap();
    assert_eq!(a.clip.id, s.clips[0].key);
}

#[test]
fn flat_and_day_layouts() {
    let l = lab(Layout::Flat, true, false);
    import(&l);
    assert!(l.root.join("2026-09-27_backyard_loops.mp4").is_file());
    let l = lab(Layout::Day, false, false);
    import(&l);
    assert!(l
        .root
        .join("2026-09-27/2026-09-27_backyard_loops.mp4")
        .is_file());
}

#[test]
fn ratings_and_details_survive_a_rebuild_from_the_files() {
    let l = lab(Layout::YearDay, false, false);
    import(&l);
    let a = by_name(&l, "backyard loops");
    let b = by_name(&l, "gap run");
    l.core
        .library_rate(std::slice::from_ref(&a.clip.id), Some(4), Some(Flag::Pick))
        .unwrap();
    l.core
        .library_rate(std::slice::from_ref(&b.clip.id), None, Some(Flag::Reject))
        .unwrap();
    l.core
        .library_edit(
            &a.clip.id,
            &LibEdit {
                keywords: Some(vec!["windy".into(), "park".into()]),
                ..Default::default()
            },
        )
        .unwrap();
    // A rating changes the file's metadata, never its picture or its identity.
    let items = quadcam_lib::qtmeta::read(&a.file).unwrap();
    assert_eq!(
        quadcam_lib::qtmeta::get(&items, library::KEY_RATING),
        Some("4")
    );
    quadcam_lib::media::verify_qt(
        &tools(),
        &a.file,
        &[("app.quadcam.flag".into(), "pick".into())],
    )
    .unwrap();

    // Throw the index away and rebuild it from the files alone.
    std::fs::remove_dir_all(l.root.join(".quadcam")).unwrap();
    let r = l.core.library_rebuild().unwrap();
    assert_eq!((r.clips, r.cuts), (2, 1), "{:?}", r.problems);
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    let a2 = by_name(&l, "backyard loops");
    assert_eq!(a2.clip.id, a.clip.id);
    assert_eq!(a2.clip.rating, 4);
    assert_eq!(a2.clip.flag, Flag::Pick);
    assert_eq!(a2.clip.keywords, ["windy", "park"]);
    assert_eq!(a2.clip.cuts.len(), 1);
    assert!((a2.clip.cuts[0].start - 0.5).abs() < 0.001);
    assert!(a2.last_import, "the last import is in the files too");
    assert_eq!(by_name(&l, "gap run").clip.flag, Flag::Reject);

    // Groups and search.
    let rejected = l
        .core
        .library(&Filter {
            group: Some("rejected".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(rejected.clips.len(), 1);
    let found = l
        .core
        .library(&Filter {
            query: Some("windy".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(found.clips.len(), 1);

    // Move the rejected clip to the Trash.
    let t = l
        .core
        .library_trash(std::slice::from_ref(&b.clip.id))
        .unwrap();
    assert_eq!(t.trashed.len(), 1);
    assert!(!b.file.exists());
    assert!(l.trash.join("2026-09-27_gap_run.mp4").is_file());
    assert_eq!(all(&l).len(), 1);
}

/// An MP4 with no quadcam metadata at all, as an older export or another tool writes it.
fn plain_mp4(path: &Path, secs: u32) {
    let st = std::process::Command::new(tools().ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc=size=320x240:rate=30")
        .args([
            "-t",
            &secs.to_string(),
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(path)
        .status()
        .unwrap();
    assert!(st.success());
}

#[test]
fn an_existing_export_folder_is_adopted_without_moving_anything() {
    let l = lab(Layout::YearDay, false, false);
    let a = l.root.join("2026-08-02_old-flight.mp4");
    let c = l.root.join("2026-08-02_old-flight_cut1.mp4");
    plain_mp4(&a, 2);
    plain_mp4(&c, 1);
    let before: Vec<_> = [&a, &c].iter().map(|p| std::fs::read(p).unwrap()).collect();

    let v = l.core.library(&Filter::default()).unwrap();
    assert_eq!(v.clips.len(), 0);
    assert_eq!(v.unindexed, 2, "the app offers to scan them");
    assert!(library::needs_scan(&l.root));

    let r = l.core.library_rebuild().unwrap();
    assert_eq!((r.clips, r.cuts), (1, 1), "{:?}", r.problems);
    let v = l.core.library(&Filter::default()).unwrap();
    assert_eq!(v.unindexed, 0);
    let clip = &v.clips[0];
    assert_eq!(clip.name, "old flight");
    assert_eq!(clip.clip.date, NaiveDate::from_ymd_opt(2026, 8, 2).unwrap());
    assert!(clip.clip.id.starts_with('h'));
    // Nothing moved, nothing written.
    assert!(a.is_file() && c.is_file());
    let after: Vec<_> = [&a, &c].iter().map(|p| std::fs::read(p).unwrap()).collect();
    assert_eq!(before, after);

    // Rating an adopted clip writes into it, and its identity holds.
    l.core
        .library_rate(std::slice::from_ref(&clip.clip.id), Some(3), None)
        .unwrap();
    std::fs::remove_dir_all(l.root.join(".quadcam")).unwrap();
    l.core.library_rebuild().unwrap();
    let v = l.core.library(&Filter::default()).unwrap();
    assert_eq!(v.clips[0].clip.id, clip.clip.id);
    assert_eq!(v.clips[0].clip.rating, 3);
}

#[test]
fn removing_an_exported_cut_asks_then_keeps_or_trashes_the_file() {
    let l = lab(Layout::YearDay, false, false);
    import(&l);
    let a = by_name(&l, "backyard loops");
    let id = a.clip.id.clone();
    let cut1 = l.root.join(&a.clip.cuts[0].path);

    // Add two ranges and write them.
    let two = vec![
        Span {
            start: 0.5,
            end: 1.5,
        },
        Span {
            start: 1.0,
            end: 2.0,
        },
        Span {
            start: 2.0,
            end: 2.9,
        },
    ];
    let ch = l.core.library_set_cuts(&id, &two, None).unwrap();
    assert!(matches!(ch, CutChange::Applied { .. }), "{ch:?}");
    assert_eq!(by_name(&l, "backyard loops").clip.pending_cuts.len(), 2);
    let made = l.core.library_export_cuts(&id).unwrap();
    assert_eq!(made.len(), 2);
    let a = by_name(&l, "backyard loops");
    assert_eq!(a.clip.cuts.len(), 3);
    assert!(a.clip.pending_cuts.is_empty());
    let cut3 = l.root.join(&a.clip.cuts[2].path);
    assert!(cut3.to_string_lossy().ends_with("_cut3.mp4"), "{cut3:?}");

    // Dropping cut 1 without a decision changes nothing and names the file.
    let only23 = two[1..].to_vec();
    let ch = l.core.library_set_cuts(&id, &only23, None).unwrap();
    assert_eq!(
        ch,
        CutChange::Confirm {
            files: vec![cut1.clone()]
        }
    );
    assert_eq!(by_name(&l, "backyard loops").clip.cuts.len(), 3);
    assert!(cut1.is_file());

    // Keep: the file stays and becomes a clip of its own, also after a rebuild.
    let ch = l
        .core
        .library_set_cuts(&id, &only23, Some(RemovedCuts::Keep))
        .unwrap();
    assert!(
        matches!(&ch, CutChange::Applied { kept, .. } if kept == &vec![cut1.clone()]),
        "{ch:?}"
    );
    assert!(cut1.is_file());
    assert_eq!(by_name(&l, "backyard loops").clip.cuts.len(), 2);
    assert_eq!(all(&l).len(), 3);
    l.core.library_rebuild().unwrap();
    assert_eq!(all(&l).len(), 3);
    assert_eq!(by_name(&l, "backyard loops").clip.cuts.len(), 2);

    // Trash: the file goes to the Trash.
    let ch = l
        .core
        .library_set_cuts(&id, &two[1..2], Some(RemovedCuts::Trash))
        .unwrap();
    assert!(matches!(ch, CutChange::Applied { .. }), "{ch:?}");
    assert!(!cut3.exists());
    assert!(std::fs::read_dir(&l.trash).unwrap().count() == 1);
    assert_eq!(by_name(&l, "backyard loops").clip.cuts.len(), 1);
}

#[test]
fn the_session_asks_too_before_dropping_an_exported_cut() {
    let l = lab(Layout::YearDay, false, false);
    import(&l);
    let ch = l.core.set_session_cuts(0, &[], None).unwrap();
    let CutChange::Confirm { files } = ch else {
        panic!("expected a question, got {ch:?}");
    };
    assert_eq!(files.len(), 1);
    assert!(files[0].is_file());
    // An agent's patch without a decision is refused with the reason.
    let e = l
        .core
        .patch(
            &[PlanPatch {
                id: 0,
                cuts: Some(vec![]),
                ..Default::default()
            }],
            Editor::Agent,
        )
        .unwrap_err();
    assert!(format!("{e:#}").contains("removed_cuts"), "{e:#}");
    let ch = l
        .core
        .set_session_cuts(0, &[], Some(RemovedCuts::Trash))
        .unwrap();
    assert!(matches!(ch, CutChange::Applied { .. }));
    assert!(!files[0].exists());
    assert_eq!(by_name(&l, "backyard loops").clip.cuts.len(), 0);
}

#[test]
fn rename_moves_the_clip_its_cuts_and_its_original() {
    let l = lab(Layout::YearDay, false, true);
    import(&l);
    let a = by_name(&l, "backyard loops");
    let c = l.core.library_rename(&a.clip.id, "fence flips").unwrap();
    assert_eq!(c.id, a.clip.id);
    let dir = l.root.join("2026/2026-09-27");
    assert!(dir.join("2026-09-27_fence_flips.mp4").is_file());
    assert!(dir.join("2026-09-27_fence_flips_cut1.mp4").is_file());
    assert!(dir.join("originals/2026-09-27_fence_flips.avi").is_file());
    assert!(!a.file.exists());
    assert_eq!(by_name(&l, "fence flips").clip.cuts.len(), 1);
}

fn qt(path: &Path, key: &str) -> Option<String> {
    let items = quadcam_lib::qtmeta::read(path).unwrap();
    quadcam_lib::qtmeta::get(&items, key).map(str::to_string)
}

fn local_mtime(path: &Path) -> chrono::NaiveDateTime {
    chrono::DateTime::<chrono::Local>::from(path.metadata().unwrap().modified().unwrap())
        .naive_local()
}

#[test]
fn a_manual_time_at_import_and_a_date_edit_that_moves_the_files() {
    use chrono::{NaiveTime, Timelike};
    let l = lab(Layout::YearDay, true, true);
    l.core.stage(Some(&l.src)).unwrap();
    l.core.analyse().unwrap();
    l.core.plan_dates(LogChoice::None, None).unwrap();
    let bad = l.core.patch(
        &[PlanPatch {
            id: 0,
            time: Some("25:00".into()),
            ..Default::default()
        }],
        Editor::User,
    );
    assert!(format!("{:#}", bad.unwrap_err()).contains("HH:MM"));
    l.core
        .patch(
            &[PlanPatch {
                id: 0,
                name: Some("backyard loops".into()),
                date: Some(day()),
                time: Some("07:45".into()),
                place: Some("Home field".into()),
                cuts: Some(vec![Span {
                    start: 1.0,
                    end: 2.0,
                }]),
                ..Default::default()
            }],
            Editor::User,
        )
        .unwrap();
    l.core.import(&ImportOptions::default()).unwrap();
    let a = by_name(&l, "backyard loops");
    assert_eq!(
        a.clip.time.as_deref(),
        Some("07:45"),
        "the manual time is in the file"
    );
    assert_eq!(
        local_mtime(&a.file).time(),
        NaiveTime::from_hms_opt(7, 45, 0).unwrap()
    );
    let old_dir = l.root.join("2026/2026-09-27 Home field");
    assert!(a.file.starts_with(&old_dir));

    // A new day: the clip, its cut and its original move and take the new date's name.
    let new_day = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
    let c = l
        .core
        .library_edit(
            &a.clip.id,
            &LibEdit {
                date: Some(new_day),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(c.id, a.clip.id, "identity is the content, not the file");
    assert_eq!(c.date, new_day);
    assert_eq!(
        c.time.as_deref(),
        Some("07:45"),
        "a new date keeps the time"
    );
    let dir = l.root.join("2026/2026-09-30 Home field");
    let clip = dir.join("2026-09-30_backyard_loops.mp4");
    let cut = dir.join("2026-09-30_backyard_loops_cut1.mp4");
    assert_eq!(l.root.join(&c.path), clip);
    assert!(cut.is_file());
    assert!(dir
        .join("originals/2026-09-30_backyard_loops.avi")
        .is_file());
    assert!(!old_dir.join("originals").exists(), "emptied folders go");
    assert!(qt(&clip, "com.apple.quicktime.creationdate")
        .unwrap()
        .starts_with("2026-09-30T07:45:00"),);
    assert!(qt(&cut, "com.apple.quicktime.creationdate")
        .unwrap()
        .starts_with("2026-09-30T07:45:01"));
    assert_eq!(local_mtime(&clip).date(), new_day);
    // The movie header (what ffprobe calls creation_time) moved too.
    let probe = quadcam_lib::media::probe(&tools(), &clip).unwrap();
    let ct = chrono::DateTime::parse_from_rfc3339(&probe.tags["creation_time"]).unwrap();
    assert_eq!(
        ct.with_timezone(&chrono::Local).naive_local(),
        new_day.and_hms_opt(7, 45, 0).unwrap()
    );

    // Only the time: the file stays where it is. Empty is local noon.
    let c = l
        .core
        .library_edit(
            &a.clip.id,
            &LibEdit {
                time: Some("".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(l.root.join(&c.path), clip);
    assert_eq!(local_mtime(&clip).hour(), 12);

    // The index made again from the files agrees.
    l.core.library_rebuild().unwrap();
    let r = by_name(&l, "backyard loops");
    assert_eq!(
        (r.clip.date, r.clip.time.as_deref()),
        (new_day, None),
        "an empty time is noon, and noon is not shown"
    );
    assert_eq!(r.clip.cuts.len(), 1);
    assert!(r.clip.original.is_some());
}

#[test]
fn a_new_aircraft_profile_after_export_rewrites_the_gear() {
    use quadcam_lib::metadata::Profile;
    let l = lab(Layout::Day, false, false);
    let mut d = l.core.defaults();
    d.profiles = vec![
        Profile {
            name: "Whoop".into(),
            aircraft: "65 mm whoop".into(),
            camera_make: "Maker".into(),
            camera_model: "Goggles".into(),
            video_system: "Analog".into(),
            keywords: vec!["tinywhoop".into()],
            author: "Pilot".into(),
            ..Default::default()
        },
        Profile {
            name: "Five".into(),
            aircraft: "5-inch".into(),
            camera_make: "Other".into(),
            camera_model: "Box".into(),
            video_system: "HDZero".into(),
            keywords: vec!["freestyle".into()],
            ..Default::default()
        },
    ];
    d.default_profile = Some("Whoop".into());
    l.core.set_defaults(d);
    import(&l);
    let a = by_name(&l, "backyard loops");
    assert_eq!(a.clip.aircraft.as_deref(), Some("Whoop"));
    assert_eq!(
        a.clip.time, None,
        "no log, no time set: no 12:00 placeholder"
    );
    assert!(a.clip.keywords.iter().any(|k| k == "tinywhoop"));

    let e = l
        .core
        .library_edit(
            &a.clip.id,
            &LibEdit {
                profile: Some("Ghost".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(format!("{e:#}").contains("profiles: Whoop, Five"), "{e:#}");

    let c = l
        .core
        .library_edit(
            &a.clip.id,
            &LibEdit {
                profile: Some("five".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(c.aircraft.as_deref(), Some("Five"));
    assert!(c.keywords.iter().any(|k| k == "freestyle"));
    assert!(!c.keywords.iter().any(|k| k == "tinywhoop"));
    assert_eq!(c.keywords[0], "FPV");
    assert_eq!(c.author, None, "the old profile's author went with it");
    for f in
        std::iter::once(l.root.join(&c.path)).chain(c.cuts.iter().map(|x| l.root.join(&x.path)))
    {
        assert_eq!(qt(&f, "com.apple.quicktime.make").as_deref(), Some("Other"));
        assert_eq!(qt(&f, "com.apple.quicktime.model").as_deref(), Some("Box"));
        assert_eq!(qt(&f, "app.quadcam.aircraft").as_deref(), Some("5-inch"));
        assert_eq!(
            qt(&f, "app.quadcam.video_system").as_deref(),
            Some("HDZero")
        );
    }
    // An empty profile removes the gear.
    let c = l
        .core
        .library_edit(
            &a.clip.id,
            &LibEdit {
                profile: Some(String::new()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(c.aircraft, None);
    assert_eq!(qt(&l.root.join(&c.path), "com.apple.quicktime.make"), None);
}

#[test]
fn a_clip_without_a_picture_is_marked_once_not_retried() {
    let l = lab(Layout::Day, false, false);
    import(&l);
    assert_eq!(l.core.library_strips(None).unwrap(), 2);
    assert!(all(&l).iter().all(|c| c.strip.is_some()));
    // A clip whose file no longer decodes (the index still lists it).
    let a = by_name(&l, "gap run");
    l.core.library_strips(None).unwrap();
    let strip = a.strip.clone().unwrap();
    std::fs::remove_file(&strip).unwrap();
    std::fs::write(&a.file, b"not a video").unwrap();
    assert_eq!(l.core.library_strips(None).unwrap(), 0);
    let a = by_name(&l, "gap run");
    assert!(a.no_picture && a.strip.is_none() && a.poster.is_none());
    // Nothing is tried again until asked for by id.
    assert_eq!(l.core.library_strips(None).unwrap(), 0);
    assert!(by_name(&l, "gap run").no_picture);
}

#[test]
fn the_short_date_format_and_renaming_existing_clips_to_it() {
    use quadcam_lib::naming::DateFormat;
    let l = lab(Layout::YearDay, false, true);
    import(&l); // long format
    let mut d = l.core.defaults();
    d.name_date_format = DateFormat::Short;
    l.core.set_defaults(d);
    let r = l.core.library_apply_name_format(None).unwrap();
    assert_eq!(r.renamed.len(), 2, "{r:?}");
    let dir = l.root.join("2026/2026-09-27");
    assert!(dir.join("26.09.27_backyard_loops.mp4").is_file());
    assert!(dir.join("26.09.27_backyard_loops_cut1.mp4").is_file());
    assert!(dir.join("originals/26.09.27_backyard_loops.avi").is_file());
    assert!(!dir.join("2026-09-27_backyard_loops.mp4").exists());
    let a = by_name(&l, "backyard loops");
    assert_eq!(a.clip.cuts.len(), 1);
    assert!(a.clip.original.is_some());
    // Run again: nothing to do.
    let again = l.core.library_apply_name_format(None).unwrap();
    assert_eq!((again.renamed.len(), again.unchanged), (0, 2));
    // The index made from the files reads both formats.
    l.core.library_rebuild().unwrap();
    let a = by_name(&l, "backyard loops");
    assert_eq!(a.clip.date, day());
    assert_eq!(a.clip.cuts.len(), 1);
    // A new import and a rename use the short format too.
    let c = l.core.library_rename(&a.clip.id, "fence flips").unwrap();
    assert_eq!(
        c.path,
        PathBuf::from("2026/2026-09-27/26.09.27_fence_flips.mp4")
    );
    // A name without a date is left alone.
    let m = by_name(&l, "gap run");
    let plain = l.root.join("2026/2026-09-27/gap.mp4");
    std::fs::rename(&m.file, &plain).unwrap();
    l.core.library_rebuild().unwrap();
    let r = l.core.library_apply_name_format(None).unwrap();
    assert_eq!(r.skipped.len(), 1, "{r:?}");
    assert!(plain.is_file());
}

/// A library cut carries its clip's details, its own range and start time, and none of
/// the clip's rating, flag or Photos state; ffprobe reads every item back (the same check a
/// session cut gets before it counts).
#[test]
fn a_library_cut_carries_the_clips_details_and_is_read_back() {
    let l = lab(Layout::YearDay, false, true);
    import(&l);
    let a = by_name(&l, "backyard loops");
    let id = a.clip.id.clone();
    l.core
        .library_rate(std::slice::from_ref(&id), Some(5), Some(Flag::Pick))
        .unwrap();
    let ranges = vec![
        Span {
            start: 0.5,
            end: 1.5,
        },
        Span {
            start: 1.5,
            end: 2.5,
        },
    ];
    l.core.library_set_cuts(&id, &ranges, None).unwrap();
    let made = l.core.library_export_cuts(&id).unwrap();
    assert_eq!(made.len(), 1);
    let cut = l.root.join(&made[0].path);
    assert!(
        cut.file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("_cut2.mp4"),
        "numbered after the existing cut: {}",
        cut.display()
    );
    let clip = l.root.join(&a.clip.path);
    assert_eq!(qt(&cut, library::KEY_CUT).as_deref(), Some("1.500-2.500"));
    assert_eq!(
        qt(&cut, library::KEY_SOURCE),
        qt(&clip, library::KEY_SOURCE)
    );
    assert_eq!(qt(&cut, library::KEY_PLACE).as_deref(), Some("Home field"));
    assert_eq!(
        qt(&cut, "com.apple.quicktime.title"),
        qt(&clip, "com.apple.quicktime.title")
    );
    assert_eq!(qt(&cut, library::KEY_RATING), None);
    assert_eq!(qt(&cut, library::KEY_FLAG), None);
    assert!(qt(&cut, "com.apple.quicktime.description")
        .unwrap()
        .ends_with("; cut 1.5-2.5 s"));
    let start = |p: &Path| {
        chrono::DateTime::parse_from_str(
            &qt(p, "com.apple.quicktime.creationdate").unwrap(),
            "%Y-%m-%dT%H:%M:%S%z",
        )
        .unwrap()
    };
    // The creation date has whole seconds.
    let offset = (start(&cut) - start(&clip)).num_milliseconds();
    assert!(
        (1000..=2000).contains(&offset),
        "the cut starts 1.5 s into the clip: {offset} ms"
    );
    let items: Vec<(String, String)> = quadcam_lib::qtmeta::read(&cut).unwrap();
    quadcam_lib::media::verify_qt(&tools(), &cut, &items).unwrap();
    assert!(by_name(&l, "backyard loops").clip.pending_cuts.is_empty());
}

/// Several Play clicks on a MOV library clip at once: one preview is made, and every
/// caller gets the same playable file.
#[test]
fn concurrent_previews_make_one_file() {
    let l = lab(Layout::YearDay, false, false);
    l.core.set_defaults(Defaults {
        format: Format::Mov,
        ..l.core.defaults()
    });
    import(&l);
    let id = by_name(&l, "backyard loops").clip.id;
    let paths: Vec<PathBuf> = std::thread::scope(|s| {
        let calls: Vec<_> = (0..4)
            .map(|_| s.spawn(|| l.core.library_preview(&id).unwrap()))
            .collect();
        calls.into_iter().map(|c| c.join().unwrap()).collect()
    });
    assert!(paths.windows(2).all(|w| w[0] == w[1]), "{paths:?}");
    let p = quadcam_lib::media::probe(&tools(), &paths[0]).unwrap();
    assert!(p.video_packets > 0);
    let dir = paths[0].parent().unwrap();
    let names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(names.len(), 1, "no half-made previews left: {names:?}");
    let made = paths[0].metadata().unwrap().modified().unwrap();
    assert_eq!(l.core.library_preview(&id).unwrap(), paths[0]);
    assert_eq!(
        paths[0].metadata().unwrap().modified().unwrap(),
        made,
        "reused, not made again"
    );
}

/// One update checks every id and value before it writes anything.
#[test]
fn library_update_checks_everything_first() {
    use quadcam_lib::core::LibUpdate;
    let l = lab(Layout::YearDay, false, false);
    import(&l);
    let a = by_name(&l, "backyard loops").clip.id;
    let b = by_name(&l, "gap run").clip.id;
    let rated = |r: u8| LibUpdate {
        rating: Some(r),
        ..Default::default()
    };
    // An unknown id, a name for two clips, an unknown place: nothing is written.
    let bad = [
        (vec![a.clone(), "nope".to_string()], rated(3)),
        (
            vec![a.clone(), b.clone()],
            LibUpdate {
                name: Some("x".into()),
                ..rated(3)
            },
        ),
        (
            vec![a.clone()],
            LibUpdate {
                edit: LibEdit {
                    place: Some("Nowhere".into()),
                    ..Default::default()
                },
                ..rated(3)
            },
        ),
    ];
    for (ids, u) in &bad {
        assert!(l.core.library_update(ids, u).is_err());
        assert_eq!(by_name(&l, "backyard loops").clip.rating, 0, "{u:?}");
    }
    let out = l
        .core
        .library_update(
            &[a.clone(), b.clone()],
            &LibUpdate {
                flag: Some(Flag::Pick),
                edit: LibEdit {
                    note: Some("good light".into()),
                    ..Default::default()
                },
                ..rated(4)
            },
        )
        .unwrap();
    assert_eq!(out.len(), 2);
    for c in [by_name(&l, "backyard loops"), by_name(&l, "gap run")] {
        assert_eq!(c.clip.rating, 4);
        assert_eq!(c.clip.flag, Flag::Pick);
        assert_eq!(c.clip.note, "good light");
    }
    assert!(l
        .core
        .library_update(&[a], &LibUpdate::default())
        .unwrap_err()
        .to_string()
        .starts_with("Nothing to change"));
}

/// "Split by flight" on a library clip: one cut per pack in the file's flight numbers, added
/// to the cuts it has, written as `_cutN` files like any other cut. A clip without packs has
/// nothing to split.
#[test]
fn split_by_flight_in_the_library() {
    let l = lab(Layout::YearDay, false, false);
    import(&l);
    let a = by_name(&l, "backyard loops");
    let id = a.clip.id.clone();
    let stats = quadcam_lib::metadata::FlightStats {
        armed_s: 1.6,
        packs: 2,
        pack_spans: vec![
            Span {
                start: 0.2,
                end: 1.0,
            },
            Span {
                start: 1.8,
                end: 2.6,
            },
        ],
        ..Default::default()
    };
    library::write_keys(
        &l.root.join(&a.clip.path),
        &[(library::KEY_STATS, serde_json::to_string(&stats).unwrap())],
    )
    .unwrap();
    l.core.library_rebuild().unwrap();
    let change = l.core.library_split_by_flight(&id).unwrap();
    let span = |a, b| Span { start: a, end: b };
    // The 0.8 s gap gives 0.4 s each side; the exported 0.5-1.5 cut stays.
    let CutChange::Applied { cuts, .. } = change else {
        panic!("no question expected: {change:?}");
    };
    assert_eq!(cuts, vec![span(0.0, 1.4), span(0.5, 1.5), span(1.4, 3.0)]);
    let made = l.core.library_export_cuts(&id).unwrap();
    let names: Vec<String> = made
        .iter()
        .map(|m| m.path.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert_eq!(
        names,
        [
            "2026-09-27_backyard_loops_cut2.mp4",
            "2026-09-27_backyard_loops_cut3.mp4"
        ]
    );
    // A second split adds nothing new.
    let CutChange::Applied { cuts, .. } = l.core.library_split_by_flight(&id).unwrap() else {
        panic!()
    };
    assert_eq!(cuts.len(), 3);
    assert!(by_name(&l, "backyard loops").clip.pending_cuts.is_empty());
    // The other clip has no radio-log packs.
    let b = by_name(&l, "gap run");
    let err = l.core.library_split_by_flight(&b.clip.id).unwrap_err();
    assert!(err.to_string().contains("nothing to split"), "{err:#}");
}
