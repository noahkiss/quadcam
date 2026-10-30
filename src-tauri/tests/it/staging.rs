//! Staging folders under `<cache>/staging`: a replaced session's folder goes, and the sweep
//! deletes only folders with an owners list that no session file names. Every card and cache
//! here is a temp folder.

use quadcam_lib::core::{Core, NoHooks};
use quadcam_lib::photos::Recorder;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// A DVR card folder with one clip. Staging only copies, so the bytes need not be video.
fn card(root: &Path, name: &str) -> PathBuf {
    let dcim = root.join(name).join("DCIM");
    std::fs::create_dir_all(&dcim).unwrap();
    std::fs::write(dcim.join("PICT0001.AVI"), name.repeat(1000)).unwrap();
    dcim.parent().unwrap().to_path_buf()
}

fn core(work: &Path, session_file: &str) -> Core {
    Core::new(
        work.join("cache"),
        Some(work.join(session_file)),
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
}

/// A folder's staging key is the time to the second; two stages need two seconds.
fn next_second() {
    std::thread::sleep(Duration::from_millis(1100));
}

/// Sets back the change time of `dir` and its files by two days.
fn age(dir: &Path) {
    let old = SystemTime::now() - Duration::from_secs(2 * 24 * 60 * 60);
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let f = std::fs::File::options().write(true).open(e.path()).unwrap();
        f.set_modified(old).unwrap();
    }
    std::fs::File::open(dir).unwrap().set_modified(old).unwrap();
}

#[test]
fn a_new_load_deletes_the_replaced_sessions_staging() {
    let work = tempfile::tempdir().unwrap();
    let core = core(work.path(), "session.json");
    let a = core.stage(Some(&card(work.path(), "a"))).unwrap().staging;
    assert!(a.join("PICT0001.AVI").is_file());
    let owners = std::fs::read_to_string(a.join(".owners")).unwrap();
    assert_eq!(
        owners.trim(),
        work.path().join("session.json").to_str().unwrap()
    );

    next_second();
    let b = core.stage(Some(&card(work.path(), "b"))).unwrap().staging;
    assert_ne!(a, b);
    assert!(!a.exists(), "the replaced session's folder is gone");
    assert!(b.join("PICT0001.AVI").is_file());
    assert!(core.session().unwrap().staged_files_exist());
}

#[test]
fn a_folder_another_session_file_names_stays() {
    let work = tempfile::tempdir().unwrap();
    let core = core(work.path(), "session.json");
    let a = core.stage(Some(&card(work.path(), "a"))).unwrap().staging;
    // A `--session other.json` core owns the same folder.
    let other = work.path().join("other.json");
    std::fs::copy(work.path().join("session.json"), &other).unwrap();
    let owners = std::fs::read_to_string(a.join(".owners")).unwrap();
    std::fs::write(a.join(".owners"), format!("{owners}{}\n", other.display())).unwrap();

    next_second();
    core.stage(Some(&card(work.path(), "b"))).unwrap();
    assert!(a.is_dir(), "other.json still names it");

    // Once that session is gone, the sweep takes the folder.
    std::fs::remove_file(&other).unwrap();
    assert_eq!(
        core.sweep_staging_older_than(Duration::ZERO),
        vec![a.clone()]
    );
    assert!(!a.exists());
}

#[test]
fn the_sweep_spares_unowned_live_fresh_and_unreadable_folders() {
    let work = tempfile::tempdir().unwrap();
    let core = core(work.path(), "session.json");
    let live = core.stage(Some(&card(work.path(), "a"))).unwrap().staging;
    let root = work.path().join("cache/staging");

    // Staged by an older QuadCam: no owners list, so nothing says who owns it.
    let legacy = root.join("legacy");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("PICT0001.AVI"), b"old").unwrap();
    age(&legacy);
    // Its owner's session file is gone: an orphan.
    let orphan = root.join("orphan");
    std::fs::create_dir_all(&orphan).unwrap();
    std::fs::write(orphan.join("PICT0001.AVI"), b"old").unwrap();
    std::fs::write(
        orphan.join(".owners"),
        format!("{}\n", work.path().join("gone.json").display()),
    )
    .unwrap();
    // Its owner's session file cannot be parsed: kept.
    let unreadable = root.join("unreadable");
    std::fs::create_dir_all(&unreadable).unwrap();
    let bad = work.path().join("bad.json");
    std::fs::write(&bad, b"{ not json").unwrap();
    std::fs::write(unreadable.join(".owners"), format!("{}\n", bad.display())).unwrap();
    age(&unreadable);

    // The orphan changed just now (a stage in another process could be writing it).
    assert!(core.sweep_staging().is_empty());
    age(&orphan);
    assert_eq!(core.sweep_staging(), vec![orphan.clone()]);
    assert!(!orphan.exists());
    assert!(legacy.is_dir());
    assert!(unreadable.is_dir());
    assert!(
        live.join("PICT0001.AVI").is_file(),
        "the live session's folder stays"
    );

    // A folder the session file names stays even after the core forgets the session.
    age(&live);
    let restored = Core::new(
        work.path().join("cache"),
        Some(work.path().join("session.json")),
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    );
    restored.forget_unrestorable();
    assert!(restored.session().is_none(), "never analysed");
    assert!(restored.sweep_staging().is_empty());
    assert!(live.is_dir());
}
