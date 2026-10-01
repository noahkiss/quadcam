//! A library QuadCam 0.4.1 built (`tests/fixtures/legacy-library`, made with that release's
//! CLI) opens in this version: its 0.4 ids move to the current scheme where the files prove
//! them, the old ids keep working, and no media file changes.
//!
//! The fixture holds: `loops` with a kept original, four stars, a pick and an unsaved cut;
//! `loops_cut1`, an exported cut kept as its own clip; `hover` without an original and with
//! an unsaved cut; and `old-export`, a file QuadCam did not write, adopted by a rebuild.

mod common;

use quadcam_lib::core::{Core, NoHooks};
use quadcam_lib::library::{Filter, Flag, LibClip};
use quadcam_lib::photos::Recorder;
use quadcam_lib::session::Defaults;
use quadcam_lib::trash::DirTrash;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy-library")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

/// Every media file's bytes, by path relative to the library.
fn media(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                if !p.ends_with(".quadcam") {
                    stack.push(p);
                }
            } else {
                out.insert(
                    p.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(&p).unwrap(),
                );
            }
        }
    }
    out
}

struct Lab {
    _dir: tempfile::TempDir,
    root: PathBuf,
    cache: PathBuf,
    trash: PathBuf,
}

fn lab() -> Lab {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("library");
    copy_dir(&fixture(), &root);
    Lab {
        cache: dir.path().join("cache"),
        trash: dir.path().join("trash"),
        root,
        _dir: dir,
    }
}

fn core(l: &Lab) -> Core {
    let c = Core::new(
        l.cache.clone(),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_trash(Arc::new(DirTrash(l.trash.clone())));
    c.set_defaults(Defaults {
        output_dir: Some(l.root.clone()),
        ..Defaults::default()
    });
    c
}

/// The 0.4 ids, by clip name, from the fixture's index.
fn old_ids() -> BTreeMap<String, String> {
    let ix: Value =
        serde_json::from_slice(&std::fs::read(fixture().join(".quadcam/index.json")).unwrap())
            .unwrap();
    ix["clips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let name = if c["cut_of"].is_null() {
                c["title"].as_str().unwrap().to_string()
            } else {
                "loops cut".into()
            };
            let name = if name.is_empty() {
                "old export".into()
            } else {
                name
            };
            (name, c["id"].as_str().unwrap().to_string())
        })
        .collect()
}

fn by_path(clips: &[LibClip], end: &str) -> LibClip {
    clips
        .iter()
        .find(|c| c.path.to_string_lossy().ends_with(end))
        .unwrap_or_else(|| panic!("no clip {end}"))
        .clone()
}

#[test]
fn a_0_4_library_opens_with_its_ids_moved_and_its_files_untouched() {
    let l = lab();
    let old = old_ids();
    assert_eq!(old.len(), 4, "{old:?}");
    assert!(old.values().all(|id| quadcam_lib::identity::is_legacy(id)));
    let files_before = media(&l.root);

    let c = core(&l);
    let clips: Vec<LibClip> = c
        .library(&Filter::default())
        .unwrap()
        .clips
        .into_iter()
        .map(|i| i.clip)
        .collect();
    assert_eq!(clips.len(), 4);

    // The kept original proves the 0.4 id: the clip gets the original's new fingerprint.
    let loops = by_path(&clips, "_loops.mp4");
    let original = l.root.join(loops.original.as_ref().unwrap());
    assert_eq!(
        loops.id,
        quadcam_lib::identity::fingerprint(&original).unwrap()
    );
    assert!(loops.id.starts_with('x'));
    assert_eq!(loops.aliases, vec![old["loops"].clone()]);
    assert_eq!(loops.rating, 4);
    assert_eq!(loops.flag, Flag::Pick);
    assert_eq!(loops.pending_cuts.len(), 1, "unsaved cuts survive");
    assert_eq!(loops.pending_cuts[0].start, 0.5);

    // No original: the 0.4 id stays, and stays recognizable.
    let hover = by_path(&clips, "_hover.mp4");
    assert_eq!(hover.id, old["hover"]);
    assert!(hover.aliases.is_empty());
    assert_eq!(hover.pending_cuts.len(), 1);

    // A kept cut keeps its id, which names the 0.4 source.
    let cut = by_path(&clips, "_loops_cut1.mp4");
    assert_eq!(cut.id, old["loops cut"]);
    assert_eq!(cut.cut_of.as_ref().unwrap().0, old["loops"]);

    // A file QuadCam did not write gets its new head id.
    let adopted = by_path(&clips, "_old-export.mp4");
    assert!(adopted.id.starts_with("hx"), "{}", adopted.id);
    assert_eq!(adopted.aliases, vec![old["old export"].clone()]);
    assert_eq!(adopted.rating, 2);

    // The index on disk is moved; no media file changed.
    let ix: Value =
        serde_json::from_slice(&std::fs::read(l.root.join(".quadcam/index.json")).unwrap())
            .unwrap();
    assert_eq!(ix["id_scheme"], 2);
    assert_eq!(media(&l.root), files_before);

    // A rebuild from the files gives the same ids and keeps the unsaved cuts.
    c.library_rebuild().unwrap();
    let again: Vec<LibClip> = c
        .library(&Filter::default())
        .unwrap()
        .clips
        .into_iter()
        .map(|i| i.clip)
        .collect();
    for a in &clips {
        let b = again.iter().find(|b| b.path == a.path).unwrap();
        assert_eq!((&a.id, &a.aliases), (&b.id, &b.aliases), "{:?}", a.path);
        assert_eq!(a.pending_cuts, b.pending_cuts);
    }
    assert_eq!(media(&l.root), files_before);

    // A second launch finds nothing to move.
    let ix_before = std::fs::read(l.root.join(".quadcam/index.json")).unwrap();
    let c2 = core(&l);
    c2.library(&Filter::default()).unwrap();
    assert_eq!(
        std::fs::read(l.root.join(".quadcam/index.json")).unwrap(),
        ix_before
    );
}

#[test]
fn old_ids_still_find_their_clips() {
    let l = lab();
    let old = old_ids();
    let c = core(&l);
    let rated = c
        .library_rate(std::slice::from_ref(&old["loops"]), Some(5), None)
        .unwrap();
    assert_eq!(rated[0].rating, 5);
    assert!(rated[0].id.starts_with('x'));
    let adopted = c
        .library_rate(std::slice::from_ref(&old["old export"]), Some(1), None)
        .unwrap();
    assert_eq!(adopted[0].rating, 1);
    let lib = c.library(&Filter::default()).unwrap();
    assert_eq!(lib.clips.len(), 4, "no clip was added twice");
}

/// A card still holding a clip that is in the library counts it as known, by either id.
#[test]
fn a_card_with_a_known_clip_has_nothing_new() {
    let l = lab();
    let c = core(&l);
    c.library(&Filter::default()).unwrap();
    let card = tempfile::tempdir().unwrap();
    std::fs::create_dir(card.path().join("DCIM")).unwrap();
    std::fs::copy(
        l.root
            .join("2026/2026-10-01/originals/2026-10-01_loops.avi"),
        card.path().join("DCIM/PICT0001.AVI"),
    )
    .unwrap();
    let st = c.card_status(card.path()).unwrap();
    assert_eq!((st.clips, st.new), (1, 0));
    common::make_clip(&card.path().join("DCIM/PICT0002.AVI"), 1, false);
    let st = c.card_status(card.path()).unwrap();
    assert_eq!((st.clips, st.new), (2, 1));
}
