//! A radio setup as data (`docs/radio-design.md` stage 3): the owner's Pocket setup, as data,
//! generates edits that plan no file on a card the card patcher (revision 11) wrote, and
//! that turn a revision 9 card into the same model files. Fixtures are scrubbed copies of
//! real cards (`tests/fixtures/radio-setup/`): registration ids, the owner id, the Bluetooth
//! name and the radio's total time replaced; the patcher's Lua screens replaced by stubs.

use quadcam_lib::gear::edgetx::card::{self, Card, WriteOptions};
use quadcam_lib::gear::edgetx::setup::generate::{generate, Source};
use quadcam_lib::gear::edgetx::setup::RadioSetup;
use quadcam_lib::gear::model::Edit;
use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/radio-setup")
}

/// A card folder as the generator reads it.
pub struct Dir(pub PathBuf);

impl Source for Dir {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.0.join(path)).ok()
    }
    fn files(&self) -> Vec<String> {
        fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(root, &p, out);
                } else {
                    out.push(
                        p.strip_prefix(root)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.0, &self.0, &mut out);
        out.sort();
        out
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        let t = to.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &t);
        } else {
            std::fs::copy(&p, &t).unwrap();
        }
    }
}

fn owner_setup() -> RadioSetup {
    serde_json::from_slice(&std::fs::read(fixtures().join("pocket-setup.json")).unwrap()).unwrap()
}

fn plan_files(root: &Path, setup: &RadioSetup) -> (Vec<Edit>, card::CardPlan, Vec<String>) {
    let g = generate(setup, &Dir(root.to_path_buf())).unwrap_or_else(|e| panic!("{e}"));
    let plan = Card::open(root).unwrap().plan(&g.edits, None).unwrap();
    (g.edits, plan, g.warnings)
}

/// The acceptance check of stage 3 on any card folder at revision 10 or 11.
fn plans_no_files(root: &Path) {
    let (edits, plan, warnings) = plan_files(root, &owner_setup());
    assert!(plan.ready(), "{:?}", plan.checks);
    let changed: Vec<String> = plan.files.iter().map(|f| f.path.clone()).collect();
    for f in &plan.files {
        if f.path.ends_with(".yml") {
            let a = String::from_utf8_lossy(f.before.as_deref().unwrap_or_default()).to_string();
            let b = String::from_utf8_lossy(f.after.as_deref().unwrap_or_default()).to_string();
            for l in card::line_diff(&a, &b) {
                eprintln!("{:?} {}", l.op, l.text);
            }
        }
    }
    assert!(changed.is_empty(), "files: {changed:?}");
    assert!(!edits.is_empty());
    // The patcher's own Lua screens stay: QuadCam did not write them.
    assert!(
        warnings.iter().any(|w| w.contains("QuadCam did not write")),
        "{warnings:?}"
    );
}

#[test]
fn the_owners_setup_plans_no_files_on_a_revision_11_card() {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&fixtures().join("pocket-rev11"), dir.path());
    plans_no_files(dir.path());
}

/// The same check on a real card copy: `QUADCAM_SETUP_CARD=<folder>` (a scratch copy of a
/// Pocket card the patcher brought to revision 10 or 11).
#[test]
#[ignore]
fn the_owners_setup_plans_no_files_on_a_real_card_copy() {
    let root = std::env::var("QUADCAM_SETUP_CARD").expect("QUADCAM_SETUP_CARD");
    plans_no_files(Path::new(&root));
}

#[test]
fn the_owners_setup_turns_a_revision_9_card_into_the_revision_11_models() {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&fixtures().join("pocket-rev9"), dir.path());
    let (_, plan, _) = plan_files(dir.path(), &owner_setup());
    assert!(plan.ready(), "{:?}", plan.checks);
    let mut backup = |_: &str, _: &[u8]| -> anyhow::Result<()> { Ok(()) };
    card::write(
        dir.path(),
        &plan,
        &WriteOptions::reader(),
        &mut backup,
        &mut |_| {},
    )
    .unwrap();
    for f in [
        "MODELS/model00.yml",
        "MODELS/model01.yml",
        "RADIO/radio.yml",
    ] {
        let want = std::fs::read(fixtures().join("pocket-rev11").join(f)).unwrap();
        let got = std::fs::read(dir.path().join(f)).unwrap();
        if want != got {
            for l in card::line_diff(
                &String::from_utf8_lossy(&want),
                &String::from_utf8_lossy(&got),
            ) {
                eprintln!("{:?} {}", l.op, l.text);
            }
            panic!("{f} differs from the patcher's");
        }
    }
    // A second run on the written card plans nothing but the Lua screens it put on.
    let (_, again, _) = plan_files(dir.path(), &owner_setup());
    assert!(
        again.files.is_empty(),
        "{:?}",
        again.files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
}
