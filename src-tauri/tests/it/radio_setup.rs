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

// ----- through the core -----

use quadcam_lib::core::{
    BackupParams, Core, NoHooks, PostFlashParams, PostFlashStep, RadioSetupEditParams,
    RadioSetupParams,
};
use quadcam_lib::disk::{DiskInfo, Volume};
use quadcam_lib::gear::apply::{ApplyPlanParams, ApplyRequest};
use quadcam_lib::gear::edgetx::setup::Assignment;
use quadcam_lib::gear::firmware::{FixtureFetch, FwEnv, Recorder as FlashRecorder};
use quadcam_lib::gear::serial::FakePorts;
use quadcam_lib::gear::Env;
use quadcam_lib::photos::Recorder;
use std::sync::Arc;

struct Bench {
    core: Arc<Core>,
    root: PathBuf,
    id: String,
    fetch: Arc<FixtureFetch>,
    _dir: tempfile::TempDir,
}

/// A core with a copy of a fixture card mounted (USB, never under /Volumes) and backed up.
fn bench(card: &str) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("CARD");
    copy_dir(&fixtures().join(card), &root);
    let vol = Volume {
        mount: root.clone(),
        info: DiskInfo {
            volume_uuid: Some("11111111-2222-3333-4444-555555555555".into()),
            parent_whole_disk: "disk42".into(),
            bus_protocol: Some("USB".into()),
            removable: true,
            ..Default::default()
        },
        is_card: false,
        source: None,
        is_radio: true,
        warnings: vec![],
    };
    let mut env = Env::fake(vec![], Arc::new(FakePorts::new(vec![])));
    env.volumes = Arc::new(move || vec![vol.clone()]);
    env.unmount = Arc::new(|_| Ok(()));
    env.mount = Arc::new(|_| Ok(()));
    let fetch = Arc::new(FixtureFetch::new());
    let core = Arc::new(
        Core::new(
            dir.path().join("cache"),
            None,
            Arc::new(NoHooks),
            Arc::new(Recorder::default()),
        )
        .with_settings(dir.path().join("support/settings.json"))
        .with_gear_env(env)
        .with_firmware_env(FwEnv {
            fetch: fetch.clone(),
            flasher: Arc::new(FlashRecorder::empty()),
        }),
    );
    let id = core
        .gear_connected()
        .unwrap()
        .into_iter()
        .find_map(|c| c.id)
        .expect("the card has an id");
    core.gear_backup(&BackupParams {
        device: Some(id.clone()),
        ..Default::default()
    })
    .unwrap();
    Bench {
        core,
        root,
        id,
        fetch,
        _dir: dir,
    }
}

fn apply(b: &Bench, change: &str) {
    let p = b
        .core
        .gear_apply_plan(&ApplyPlanParams {
            id: change.into(),
            port: None,
        })
        .unwrap();
    assert!(p.checks.iter().all(|k| k.ok), "{:?}", p.checks);
    b.core
        .gear_apply(&ApplyRequest {
            id: change.into(),
            digest: p.digest.clone(),
            confirm: true,
            port: None,
        })
        .unwrap();
}

#[test]
fn a_saved_setup_stages_one_change_that_applies_and_then_the_card_matches() {
    let b = bench("pocket-rev9");
    // No setup saved yet: the board's default for the selected model.
    let d = b
        .core
        .gear_radio_setup(&RadioSetupParams {
            device: Some(b.id.clone()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(d.origin, "default");
    assert_eq!(d.source, "card");
    assert!(d
        .jobs
        .iter()
        .any(|j| j.job == "arm" && j.control.as_deref() == Some("SA")));
    let r = b
        .core
        .gear_radio_setup_edit(&RadioSetupEditParams {
            device: b.id.clone(),
            setup: Some(owner_setup()),
            stage: true,
            ..Default::default()
        })
        .unwrap();
    let c = r.change.expect("staged");
    assert_eq!(c.title, "Radio setup");
    assert!(r.detail.files.contains(&"MODELS/model00.yml".to_string()));
    apply(&b, &c.id);
    for f in ["MODELS/model00.yml", "MODELS/model01.yml"] {
        assert_eq!(
            std::fs::read(b.root.join(f)).unwrap(),
            std::fs::read(fixtures().join("pocket-rev11").join(f)).unwrap(),
            "{f}"
        );
    }
    // The saved setup now reads as matching the card, and staging again stages nothing.
    let d = b
        .core
        .gear_radio_setup(&RadioSetupParams {
            device: Some(b.id.clone()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(d.origin, "saved");
    assert!(d.files.is_empty(), "{:?}", d.files);
    let r = b
        .core
        .gear_radio_setup_edit(&RadioSetupEditParams {
            device: b.id.clone(),
            stage: true,
            ..Default::default()
        })
        .unwrap();
    assert!(r.change.is_none());
    assert!(r.notes[0].contains("already matches"), "{:?}", r.notes);
}

#[test]
fn moving_a_job_regenerates_what_it_owns_and_a_bad_move_refuses() {
    let b = bench("pocket-rev11");
    b.core
        .gear_radio_setup_edit(&RadioSetupEditParams {
            device: b.id.clone(),
            setup: Some(owner_setup()),
            ..Default::default()
        })
        .unwrap();
    // Rates on SB, which the OSD layout uses: refused, nothing saved.
    let e = b
        .core
        .gear_radio_setup_edit(&RadioSetupEditParams {
            device: b.id.clone(),
            assign: vec![Assignment {
                job: "rates".into(),
                control: "SB".into(),
                high: Some(0),
            }],
            ..Default::default()
        })
        .unwrap_err();
    assert!(e.to_string().contains("SB does two jobs"), "{e}");
    // Free the OSD layout, then rates on SB: the rates mix, callouts and the battery gate
    // follow SB; CH6 loses its mix.
    let r = b
        .core
        .gear_radio_setup_edit(&RadioSetupEditParams {
            device: b.id.clone(),
            free: vec!["osd".into()],
            assign: vec![Assignment {
                job: "rates".into(),
                control: "SB".into(),
                high: Some(0),
            }],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(r.detail.files, ["MODELS/model00.yml", "MODELS/model01.yml"]);
    let text: String = r.detail.diff.iter().map(|d| format!("{d:?}")).collect();
    for want in ["SB2", "free,1,!1x", "SB1,L4"] {
        assert!(text.contains(want), "{want} in the diff");
    }
    let rows = r.detail.jobs;
    assert!(rows.iter().any(|j| j.job == "osd" && j.control.is_none()));
}

fn zip_of(dir: &Path, files: &[(&str, &[u8])]) -> Vec<u8> {
    let src = dir.join("pack");
    for (p, b) in files {
        let f = src.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, b).unwrap();
    }
    let zip = dir.join("pack.zip");
    let st = std::process::Command::new("/usr/bin/ditto")
        .args(["-c", "-k", "--norsrc", "--noextattr", "--noacl", "--noqtn"])
        .arg(&src)
        .arg(&zip)
        .status()
        .unwrap();
    assert!(st.success());
    std::fs::read(zip).unwrap()
}

#[test]
fn the_steps_after_a_flash_plan_and_stage_from_fixture_downloads() {
    let b = bench("pocket-rev11");
    let tmp = tempfile::tempdir().unwrap();
    let pack = zip_of(
        tmp.path(),
        &[
            ("edgetx.sdcard.version", b"2.12.3\n"),
            ("SCRIPTS/TOOLS/readme.txt", b"tools"),
            ("RADIO/radio.yml", b"never"),
        ],
    );
    b.fetch.serve(
        "https://github.com/EdgeTX/edgetx-sdcard/releases/download/v2.12.3/bw128x64.zip",
        pack,
    );
    b.fetch.serve(
        "https://github.com/ExpressLRS/ExpressLRS/releases/download/4.1.0/elrs.lua",
        b"-- elrs tools\n".to_vec(),
    );
    std::fs::create_dir_all(b.root.join("SCRIPTS/TOOLS")).unwrap();
    std::fs::write(b.root.join("SCRIPTS/TOOLS/elrsV3.lua"), b"old").unwrap();
    b.core
        .gear_device_save(&quadcam_lib::core::DeviceSaveParams {
            id: b.id.clone(),
            name: Some("Pocket 2".into()),
            ..Default::default()
        })
        .unwrap();
    let plan = b
        .core
        .gear_radio_post_flash(&PostFlashParams {
            device: b.id.clone(),
            elrs_version: Some("4.1.0".into()),
            ..Default::default()
        })
        .unwrap();
    let step = |s: PostFlashStep| plan.steps.iter().find(|x| x.step == s).unwrap();
    let pk = step(PostFlashStep::StoragePack);
    assert!(pk.problem.is_none(), "{:?}", pk.problem);
    let pp = pk.pack.as_ref().unwrap();
    assert_eq!(pp.version_after.as_deref(), Some("2.12.3"));
    assert_eq!(pp.skipped, ["RADIO/radio.yml"]);
    assert!(pp.put.contains(&"SCRIPTS/TOOLS/readme.txt".to_string()));
    let lua = step(PostFlashStep::ElrsLua);
    assert_eq!(
        lua.files,
        ["SCRIPTS/TOOLS/elrs.lua", "SCRIPTS/TOOLS/elrsV3.lua"]
    );
    let label = step(PostFlashStep::VolumeLabel);
    assert_eq!(label.label_after.as_deref(), Some("POCKET2"));
    assert_eq!(label.label_before.as_deref(), Some("CARD"));
    assert!(plan.changes.is_empty());
    // Staged: one change per step; each applies.
    let staged = b
        .core
        .gear_radio_post_flash(&PostFlashParams {
            device: b.id.clone(),
            steps: vec![PostFlashStep::StoragePack, PostFlashStep::ElrsLua],
            elrs_version: Some("4.1.0".into()),
            stage: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(staged.changes.len(), 2);
    for c in &staged.changes {
        apply(&b, &c.id);
    }
    assert_eq!(
        std::fs::read(b.root.join("SCRIPTS/TOOLS/elrs.lua")).unwrap(),
        b"-- elrs tools\n"
    );
    assert!(!b.root.join("SCRIPTS/TOOLS/elrsV3.lua").exists());
    assert_eq!(
        std::fs::read(b.root.join("edgetx.sdcard.version")).unwrap(),
        b"2.12.3\n"
    );
    assert_ne!(
        std::fs::read(b.root.join("RADIO/radio.yml")).unwrap(),
        b"never"
    );
    // The label: refused without confirm, then renamed through diskutil (a fake here).
    let e = b
        .core
        .gear_volume_label(&quadcam_lib::core::VolumeLabelParams {
            device: b.id.clone(),
            ..Default::default()
        })
        .unwrap_err();
    assert!(e.to_string().contains("confirm"), "{e}");
    let l = b
        .core
        .gear_volume_label(&quadcam_lib::core::VolumeLabelParams {
            device: b.id.clone(),
            confirm: true,
            ..Default::default()
        })
        .unwrap();
    assert!(l.renamed);
    assert_eq!((l.before.as_str(), l.after.as_str()), ("CARD", "POCKET2"));
    let e = b
        .core
        .gear_volume_label(&quadcam_lib::core::VolumeLabelParams {
            device: b.id.clone(),
            label: Some("my radio".into()),
            confirm: true,
        })
        .unwrap_err();
    assert!(e.to_string().contains("upper case"), "{e}");
    // Without a release named, the Lua step says what it needs.
    let again = b
        .core
        .gear_radio_post_flash(&PostFlashParams {
            device: b.id.clone(),
            steps: vec![PostFlashStep::ElrsLua],
            ..Default::default()
        })
        .unwrap();
    assert!(again.steps[0]
        .problem
        .as_deref()
        .unwrap()
        .contains("elrs_version"));
}

// ----- the GX12 -----

use quadcam_lib::gear::edgetx::board;
use quadcam_lib::gear::edgetx::model as em;
use quadcam_lib::gear::edgetx::setup::defaults;
use quadcam_lib::gear::edgetx::yaml::{latin1, Doc};

/// The GX12's factory card files (`tests/fixtures/edgetx/gx12-factory`) at `semver` 2.12.5,
/// as the radio reads them after its flash.
fn gx12_card(dir: &Path) {
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/edgetx/gx12-factory");
    for f in ["RADIO/radio.yml", "MODELS/model00.yml"] {
        let to = dir.join(f);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        let b = latin1(&std::fs::read(from.join(f)).unwrap())
            .replace("semver: 2.11.0\r\n", "semver: 2.12.5\r\n");
        std::fs::write(&to, b.chars().map(|c| c as u32 as u8).collect::<Vec<u8>>()).unwrap();
    }
}

fn write_plan(root: &Path, plan: &card::CardPlan) {
    let mut backup = |_: &str, _: &[u8]| -> anyhow::Result<()> { Ok(()) };
    card::write(
        root,
        plan,
        &WriteOptions::reader(),
        &mut backup,
        &mut |_| {},
    )
    .unwrap();
}

/// The top-level keys of a model file, in order.
fn top_keys(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| !l.starts_with(' ') && !l.starts_with('-') && !l.is_empty())
        .filter_map(|l| l.split(':').next().map(str::to_string))
        .collect()
}

#[test]
fn the_default_gx12_setup_gives_valid_yaml_on_a_gx12_card() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    gx12_card(root);
    // A factory model has no telemetry sensors or logic blocks: the setup refuses it and
    // says why, writing nothing.
    let s = defaults::setup("gx12", "model00.yml").unwrap();
    let e = generate(&s, &Dir(root.to_path_buf())).unwrap_err();
    assert!(e.reason.contains("model00.yml"), "{e}");
    // The Pocket's model comes over first (stage 2's copy across radios, SE to SH).
    let pocket =
        latin1(&std::fs::read(fixtures().join("pocket-rev11/MODELS/model00.yml")).unwrap());
    let c = Card::open(root).unwrap();
    let plan = c
        .plan(
            &[Edit::ModelImport {
                from_board: "pocket".into(),
                text: pocket,
                to: "model01.yml".into(),
                name: "AIR65 II".into(),
                links: vec![],
                template: None,
            }],
            None,
        )
        .unwrap();
    assert!(plan.ready(), "{:?}", plan.checks);
    write_plan(root, &plan);
    let before = latin1(&std::fs::read(root.join("MODELS/model01.yml")).unwrap());
    let radio_before = latin1(&std::fs::read(root.join("RADIO/radio.yml")).unwrap());

    // The default GX12 assignment: arm SA, OSD SB, rates SC, finder SD, turtle SH, VTX P1.
    let s = defaults::setup("gx12", "model01.yml").unwrap();
    let g = generate(&s, &Dir(root.to_path_buf())).unwrap_or_else(|e| panic!("{e}"));
    let row = |j: &str| {
        g.jobs
            .iter()
            .find(|r| r.job == j)
            .unwrap()
            .control
            .clone()
            .unwrap()
    };
    assert_eq!(
        ["arm", "osd", "rates", "finder", "turtle", "vtx"].map(row),
        ["SA", "SB", "SC", "SD", "SH", "P1"]
    );
    let plan = Card::open(root).unwrap().plan(&g.edits, None).unwrap();
    assert!(plan.ready(), "{:?}", plan.checks);
    let paths: Vec<&str> = plan.files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"MODELS/model01.yml"), "{paths:?}");
    assert!(paths.contains(&"SCRIPTS/TELEMETRY/finder.lua"), "{paths:?}");
    write_plan(root, &plan);

    // Valid by the GX12's schema: the file renders back, reads as a model, keeps its key
    // order and every key it had, adds none the schema lacks, keeps the custom switches
    // as they were, and writes no legacy switch-warning line.
    let after = latin1(&std::fs::read(root.join("MODELS/model01.yml")).unwrap());
    let d = Doc::parse("model01.yml", after.as_bytes()).unwrap();
    assert_eq!(d.render(), after.as_bytes());
    let v = em::view("model01.yml", &d).unwrap();
    assert_eq!(v.name, "AIR65 II");
    assert_eq!(
        top_keys(&after),
        top_keys(&before),
        "the model's blocks stay in place"
    );
    assert_eq!(
        board::custom_switches(&d),
        board::custom_switches(&Doc::parse("b", before.as_bytes()).unwrap())
    );
    assert!(!after.contains("switchWarningState"));
    // Every switch position the model names is one the GX12 has; the turtle gate is on SH.
    let gx12: Vec<String> = board::spec(Some("gx12"))
        .unwrap()
        .controls()
        .into_iter()
        .map(|c| c.name)
        .collect();
    for l in &v.logical_switches {
        for part in l.def.split(',').chain([l.andsw.as_str()]) {
            let p = part.trim_start_matches('!');
            if p.len() == 3 && p.starts_with('S') && p.as_bytes()[2].is_ascii_digit() {
                assert!(
                    gx12.contains(&p[..2].to_string()),
                    "L{} names {p}",
                    l.index + 1
                );
            }
        }
    }
    assert!(v.logical_switches.iter().any(|l| l.def == "SH2,L15"));
    assert!(v.logical_switches.iter().any(|l| l.def == "L14,!SH2"));
    let mix = |ch: u32| -> Vec<(String, String)> {
        v.mixes
            .iter()
            .filter(|m| m.dest_ch == ch)
            .map(|m| (m.source.clone(), m.weight.clone()))
            .collect()
    };
    // Front toggles: position 2 sends 2000 µs, so profile 1 (1000 µs) is position 0, the
    // same meaning as the Pocket's up until stage 5 checks it on the radio.
    assert_eq!(mix(5), [("SB".to_string(), "100".to_string())]);
    assert_eq!(mix(6), [("SC".to_string(), "100".to_string())]);
    assert_eq!(mix(8), [("SH".to_string(), "100".to_string())]);
    assert_eq!(mix(10), [("P1".to_string(), "100".to_string())]);
    assert_eq!(
        v.switch_warnings
            .iter()
            .map(|w| (w.switch.as_str(), w.pos.as_str()))
            .collect::<Vec<_>>(),
        [("SA", "up")]
    );
    // radio.yml keeps its lower-case switch types (they were right) and every other line.
    let radio = latin1(&std::fs::read(root.join("RADIO/radio.yml")).unwrap());
    Doc::parse("radio.yml", radio.as_bytes()).unwrap();
    assert!(radio.contains("type: 2pos") && radio.contains("type: toggle"));
    assert_eq!(top_keys(&radio), top_keys(&radio_before));
    // The finder screen reads SD; a second run changes nothing.
    let lua = std::fs::read_to_string(root.join("SCRIPTS/TELEMETRY/finder.lua")).unwrap();
    assert!(lua.contains("local SWITCH = \"sd\""));
    let g = generate(&s, &Dir(root.to_path_buf())).unwrap();
    let again = Card::open(root).unwrap().plan(&g.edits, None).unwrap();
    assert!(
        again.files.is_empty(),
        "{:?}",
        again.files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
}

// ----- the editor's rows: fitted parts, problems in place, copy from another radio -----

use quadcam_lib::core::SetupCopy;
use quadcam_lib::gear::edgetx::setup::FittedSlot;
use std::sync::Mutex;

#[test]
fn a_fitted_part_changes_what_a_slot_can_do_and_reaches_radio_yml() {
    let b = bench("pocket-rev11");
    b.core
        .gear_radio_setup_edit(&RadioSetupEditParams {
            device: b.id.clone(),
            setup: Some(owner_setup()),
            ..Default::default()
        })
        .unwrap();
    let blank = || FittedSlot {
        control: "P1".into(),
        radio_type: "none".into(),
    };
    // A blanking plug in P1 while it does VTX power: refused, nothing saved.
    let e = b
        .core
        .gear_radio_setup_edit(&RadioSetupEditParams {
            device: b.id.clone(),
            fit: vec![blank()],
            ..Default::default()
        })
        .unwrap_err();
    assert!(e.to_string().contains("P1 is set as not fitted"), "{e}");
    // Read with that setup, the problem sits on its job and nothing is planned.
    let mut s = owner_setup();
    s.fitted.push(blank());
    let d = b
        .core
        .gear_radio_setup(&RadioSetupParams {
            device: Some(b.id.clone()),
            setup: Some(s),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(d.problems.len(), 1, "{:?}", d.problems);
    assert_eq!(d.problems[0].job.as_deref(), Some("vtx"));
    assert!(d.edits.is_empty() && d.files.is_empty());
    let p1 = d.slots.iter().find(|x| x.control == "P1").unwrap();
    assert_eq!(
        (
            p1.factory.as_str(),
            p1.fitted.as_str(),
            p1.on_radio.as_deref()
        ),
        ("without_detent", "none", Some("without_detent"))
    );
    // Free VTX power first: the plug saves, and the staged change writes P1's type.
    let r = b
        .core
        .gear_radio_setup_edit(&RadioSetupEditParams {
            device: b.id.clone(),
            free: vec!["vtx".into()],
            fit: vec![blank()],
            stage: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(r.detail.setup.fitted, vec![blank()]);
    assert!(!r.detail.controls.iter().any(|c| c.name == "P1"));
    assert!(r.detail.edits.iter().any(|e| matches!(e,
        Edit::Radio { ops } if ops.iter().any(|o| matches!(o,
            card::RadioOp::SetControlType { control, value } if control == "P1" && value == "none")))));
    assert!(r.detail.files.contains(&"RADIO/radio.yml".to_string()));
    // The factory part again takes the slot off the list.
    let r = b
        .core
        .gear_radio_setup_edit(&RadioSetupEditParams {
            device: b.id.clone(),
            fit: vec![FittedSlot {
                control: "P1".into(),
                radio_type: "without_detent".into(),
            }],
            ..Default::default()
        })
        .unwrap();
    assert!(r.detail.setup.fitted.is_empty());
    // A type the slot cannot take refuses.
    let e = b
        .core
        .gear_radio_setup_edit(&RadioSetupEditParams {
            device: b.id.clone(),
            fit: vec![FittedSlot {
                control: "SA".into(),
                radio_type: "slider".into(),
            }],
            ..Default::default()
        })
        .unwrap_err();
    assert!(e.to_string().contains("2POS, 3POS, TOGGLE, NONE"), "{e}");
}

/// Two radio cards on one core: `swap` changes which one is plugged in.
struct Two {
    core: Arc<Core>,
    roots: [PathBuf; 2],
    plugged: Arc<Mutex<usize>>,
    _dir: tempfile::TempDir,
}

fn volume_at(root: &Path, uuid: &str) -> Volume {
    Volume {
        mount: root.to_path_buf(),
        info: DiskInfo {
            volume_uuid: Some(uuid.into()),
            parent_whole_disk: "disk42".into(),
            bus_protocol: Some("USB".into()),
            removable: true,
            ..Default::default()
        },
        is_card: false,
        source: None,
        is_radio: true,
        warnings: vec![],
    }
}

fn two_radios() -> Two {
    let dir = tempfile::tempdir().unwrap();
    let pocket = dir.path().join("POCKET");
    copy_dir(&fixtures().join("pocket-rev11"), &pocket);
    let gx12 = dir.path().join("GX12");
    gx12_card(&gx12);
    let vols = [
        volume_at(&pocket, "11111111-2222-3333-4444-555555555555"),
        volume_at(&gx12, "66666666-7777-8888-9999-000000000000"),
    ];
    let plugged = Arc::new(Mutex::new(0usize));
    let p = plugged.clone();
    let mut env = Env::fake(vec![], Arc::new(FakePorts::new(vec![])));
    env.volumes = Arc::new(move || vec![vols[*p.lock().unwrap()].clone()]);
    env.unmount = Arc::new(|_| Ok(()));
    env.mount = Arc::new(|_| Ok(()));
    let core = Arc::new(
        Core::new(
            dir.path().join("cache"),
            None,
            Arc::new(NoHooks),
            Arc::new(Recorder::default()),
        )
        .with_settings(dir.path().join("support/settings.json"))
        .with_gear_env(env),
    );
    Two {
        core,
        roots: [pocket, gx12],
        plugged,
        _dir: dir,
    }
}

impl Two {
    /// Plugs radio `i` in, backs it up and answers its id.
    fn plug(&self, i: usize) -> String {
        *self.plugged.lock().unwrap() = i;
        let id = self
            .core
            .gear_connected()
            .unwrap()
            .into_iter()
            .find_map(|c| c.id)
            .expect("the card has an id");
        self.core
            .gear_backup(&BackupParams {
                device: Some(id.clone()),
                ..Default::default()
            })
            .unwrap();
        id
    }
}

#[test]
fn a_copy_from_another_radio_stages_the_models_and_the_setup_as_one_change() {
    let t = two_radios();
    let pocket = t.plug(0);
    t.core
        .gear_radio_setup_edit(&RadioSetupEditParams {
            device: pocket.clone(),
            setup: Some(owner_setup()),
            ..Default::default()
        })
        .unwrap();
    let gx12 = t.plug(1);
    assert_ne!(pocket, gx12);
    // The factory GX12 model alone: the setup saves, but the card does not take it.
    let d = t
        .core
        .gear_radio_setup(&RadioSetupParams {
            device: Some(gx12.clone()),
            ..Default::default()
        })
        .unwrap();
    assert!(d.problems.is_empty(), "{:?}", d.problems);
    assert!(d.card_problem.is_some());
    // Copy both Pocket models (from its latest backup) and its setup.
    let r = t
        .core
        .gear_radio_setup_edit(&RadioSetupEditParams {
            device: gx12.clone(),
            copy_from: Some(SetupCopy {
                from: pocket.clone(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap_or_else(|e| panic!("{e:#}"));
    let c = r.change.expect("a copy stages");
    assert!(matches!(c.edits[0], Edit::ModelImport { .. }));
    assert!(matches!(c.edits[1], Edit::ModelImport { .. }));
    let copied: Vec<(String, String)> = r
        .detail
        .copies
        .iter()
        .map(|m| (m.to.clone(), m.name.clone()))
        .collect();
    assert_eq!(
        copied,
        [
            ("model01.yml".to_string(), "AIR65 II".to_string()),
            ("model02.yml".to_string(), "METEOR75".to_string())
        ]
    );
    let control = |j: &str| {
        r.detail
            .setup
            .assignment(j)
            .map(|a| a.control.clone())
            .unwrap_or_default()
    };
    // The Pocket's momentary SE is the GX12's SH; the rest keep their names.
    assert_eq!(
        ["arm", "osd", "rates", "finder", "turtle", "vtx"].map(control),
        ["SA", "SB", "SC", "SD", "SH", "P1"]
    );
    let files: Vec<&str> = r
        .detail
        .setup
        .aircraft
        .iter()
        .map(|a| a.file.as_str())
        .collect();
    assert_eq!(files, ["model01.yml", "model02.yml"]);
    assert!(
        r.notes[0].contains("AIR65 II to model01.yml"),
        "{:?}",
        r.notes
    );
    // Reading again keeps the copies in front.
    let d = t
        .core
        .gear_radio_setup(&RadioSetupParams {
            device: Some(gx12.clone()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(d.copies.len(), 2);
    assert!(d.card_problem.is_none(), "{:?}", d.card_problem);
    // Apply it: both models land, and the setup then matches the card.
    let p = t
        .core
        .gear_apply_plan(&ApplyPlanParams {
            id: c.id.clone(),
            port: None,
        })
        .unwrap();
    assert!(p.checks.iter().all(|k| k.ok), "{:?}", p.checks);
    t.core
        .gear_apply(&ApplyRequest {
            id: c.id.clone(),
            digest: p.digest.clone(),
            confirm: true,
            port: None,
        })
        .unwrap_or_else(|e| panic!("{e:#}"));
    for f in ["MODELS/model01.yml", "MODELS/model02.yml"] {
        assert!(t.roots[1].join(f).is_file(), "{f}");
    }
    let d = t
        .core
        .gear_radio_setup(&RadioSetupParams {
            device: Some(gx12.clone()),
            ..Default::default()
        })
        .unwrap();
    assert!(d.copies.is_empty());
    assert!(d.files.is_empty(), "{:?}", d.files);
}
