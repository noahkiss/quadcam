//! The model editors through the core (WP9 part A): a model reads from the mounted card or
//! the latest backup; an edit stages into the radio's one "Model edits" change, a later edit
//! of a setting replaces the earlier, an edit the radio already has drops out, a checklist
//! stages as text, and a staged edit applies to the synthetic card with read-back. Every
//! card is a temporary folder and "diskutil" is a closure.

use quadcam_lib::core::{
    BackupParams, Core, Hooks, ModelEditParams, ModelParams, NoHooks, StageParams,
};
use quadcam_lib::disk::{DiskInfo, Volume};
use quadcam_lib::gear::apply::{ApplyPlanParams, ApplyRequest};
use quadcam_lib::gear::bf::cli::Timing;
use quadcam_lib::gear::changes::ChangeFilter;
use quadcam_lib::gear::cues::{CueService, RecordedCues};
use quadcam_lib::gear::edgetx::editors::{CalloutDef, CalloutWhen, LoggingDef};
use quadcam_lib::gear::edgetx::model::{Field, ModelOp};
use quadcam_lib::gear::edgetx::synth::{self, SynthCard};
use quadcam_lib::gear::events::Presence;
use quadcam_lib::gear::model::{ChangeStatus, Edit};
use quadcam_lib::gear::serial::FakePorts;
use quadcam_lib::gear::Env;
use quadcam_lib::photos::Recorder;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const DISK: &str = "disk42";

fn volume(root: &std::path::Path, uuid: &str) -> Volume {
    Volume {
        mount: root.to_path_buf(),
        info: DiskInfo {
            volume_uuid: Some(uuid.into()),
            parent_whole_disk: DISK.into(),
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

#[allow(dead_code)]
struct Bench {
    core: Arc<Core>,
    cues: Arc<RecordedCues>,
    root: PathBuf,
    id: String,
    /// The card is mounted now.
    mounted: Arc<AtomicBool>,
    /// The card is still plugged in (its disk node shows).
    present: Arc<AtomicBool>,
    /// What the fake `diskutil` was asked: "mount disk42", "unmount disk42".
    log: Arc<Mutex<Vec<String>>>,
    unmount_fails: Arc<AtomicBool>,
    dir: tempfile::TempDir,
}

struct Opts {
    card: SynthCard,
    fail_readback: Option<String>,
    hooks: Arc<dyn Hooks>,
    uuid: &'static str,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            card: SynthCard::default(),
            fail_readback: None,
            hooks: Arc::new(NoHooks),
            uuid: "11111111-2222-3333-4444-555555555555",
        }
    }
}

/// A core with one synthetic radio card mounted, backed up once so the radio is a saved
/// device.
fn bench(o: Opts) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("CARD");
    synth::write_card(&root, &o.card).unwrap();
    let mounted = Arc::new(AtomicBool::new(true));
    let log: Arc<Mutex<Vec<String>>> = Arc::default();
    let unmount_fails = Arc::new(AtomicBool::new(false));
    let cues = Arc::new(RecordedCues::default());
    let vol = volume(&root, o.uuid);
    let mut env = Env::fake(vec![], Arc::new(FakePorts::new(vec![])));
    env.cues = Arc::new(CueService::inline(cues.clone()));
    let m = mounted.clone();
    env.volumes = Arc::new(move || {
        if m.load(Ordering::SeqCst) {
            vec![vol.clone()]
        } else {
            vec![]
        }
    });
    let present = Arc::new(AtomicBool::new(true));
    let p = present.clone();
    env.presence = Arc::new(move || {
        if p.load(Ordering::SeqCst) {
            vec![Presence::Disk { disk: DISK.into() }]
        } else {
            vec![]
        }
    });
    let (m, l, f) = (mounted.clone(), log.clone(), unmount_fails.clone());
    env.unmount = Arc::new(move |d| {
        l.lock().unwrap().push(format!("unmount {d}"));
        if f.load(Ordering::SeqCst) {
            anyhow::bail!("Unmount of {d} failed: at least one volume could not be unmounted")
        }
        m.store(false, Ordering::SeqCst);
        Ok(())
    });
    let (m, l) = (mounted.clone(), log.clone());
    env.mount = Arc::new(move |d| {
        l.lock().unwrap().push(format!("mount {d}"));
        m.store(true, Ordering::SeqCst);
        Ok(())
    });
    env.fail_readback = o.fail_readback;
    // Cues play every time here: the debounce would hide a second "safe to unplug".
    std::fs::create_dir_all(dir.path().join("support")).unwrap();
    std::fs::write(
        dir.path().join("support/settings.json"),
        r#"{"gearCues": {"debounce_s": 0}}"#,
    )
    .unwrap();
    let core = Arc::new(
        Core::new(
            dir.path().join("cache"),
            None,
            o.hooks,
            Arc::new(Recorder::default()),
        )
        .with_settings(dir.path().join("support/settings.json"))
        .with_gear_env(env)
        .with_fc_timing(Timing::fast()),
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
    // The backup unmounted the card; the tests start with it mounted, as a reader shows it.
    mounted.store(true, Ordering::SeqCst);
    log.lock().unwrap().clear();
    cues.played.lock().unwrap().clear();
    Bench {
        core,
        cues,
        root,
        id,
        mounted,
        present,
        log,
        unmount_fails,
        dir,
    }
}

fn model(b: &Bench) -> quadcam_lib::core::ModelDetail {
    b.core
        .gear_model(&ModelParams {
            device: b.id.clone(),
            model: Some("model00.yml".into()),
            staged: true,
        })
        .unwrap()
}

fn edit(
    b: &Bench,
    ops: Vec<ModelOp>,
    checklist: Option<&str>,
) -> anyhow::Result<quadcam_lib::gear::model::StagedChange> {
    b.core.gear_model_edit(&ModelEditParams {
        device: b.id.clone(),
        model: "model00.yml".into(),
        ops,
        checklist: checklist.map(Into::into),
        editor: None,
    })
}

fn staged(b: &Bench) -> Vec<quadcam_lib::gear::model::StagedChange> {
    b.core.gear_changes(&ChangeFilter::default()).unwrap()
}

fn lowbat(v: &str) -> ModelOp {
    ModelOp::SetCallout {
        callout: CalloutDef {
            track: "lowbat".into(),
            when: CalloutWhen::Below {
                source: "{RxBt}".into(),
                value: v.into(),
                delay_ds: 20,
            },
            repeat: Some("5".into()),
        },
    }
}

#[test]
fn a_model_reads_from_the_card_and_from_the_backup() {
    let b = bench(Opts::default());
    let m = model(&b);
    assert_eq!(m.source, "card");
    assert_eq!(m.view.name, "ALPHA");
    assert_eq!(m.models.len(), 2);
    assert!(m
        .models
        .iter()
        .any(|e| e.file == "model01.yml" && e.selected));
    assert_eq!(m.tracks, ["hello"]);
    assert_eq!(m.checklist_width, Some(20));
    assert!(m.editors.callouts.iter().any(|c| c.track == "armed"));
    // Unplugged: the same model from the latest backup.
    b.mounted.store(false, Ordering::SeqCst);
    let m = model(&b);
    assert!(m.source.starts_with("backup "), "{}", m.source);
    assert_eq!(m.view.name, "ALPHA");
    // The default model is the radio's selected one.
    let d = b
        .core
        .gear_model(&ModelParams {
            device: b.id.clone(),
            model: None,
            staged: true,
        })
        .unwrap();
    assert_eq!(d.view.file, "model01.yml");
}

#[test]
fn an_unknown_radio_or_model_says_so() {
    let b = bench(Opts::default());
    let e = b
        .core
        .gear_model(&ModelParams {
            device: "nope".into(),
            ..Default::default()
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("No device"), "{e:#}");
    let e = b
        .core
        .gear_model(&ModelParams {
            device: b.id.clone(),
            model: Some("model09.yml".into()),
            staged: true,
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("no MODELS/model09.yml"), "{e:#}");
    let e = b
        .core
        .gear_model(&ModelParams {
            device: b.id.clone(),
            model: Some("../x".into()),
            staged: true,
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("not a model file name"));
}

#[test]
fn edits_join_one_change_and_the_last_word_per_setting_wins() {
    let b = bench(Opts::default());
    let c1 = edit(&b, vec![lowbat("3.5")], None).unwrap();
    assert_eq!(c1.title, "Model edits");
    assert_eq!(c1.status, ChangeStatus::Ready);
    // A second setting joins; the callout again replaces its earlier value.
    let c2 = edit(
        &b,
        vec![
            lowbat("3.4"),
            ModelOp::SetTimer {
                index: 1,
                fields: vec![Field::new("minuteBeep", "0")],
            },
        ],
        None,
    )
    .unwrap();
    assert_eq!(c2.id, c1.id);
    assert_eq!(staged(&b).len(), 1);
    let Edit::Model { file, name, ops } = &c2.edits[0] else {
        panic!("{:?}", c2.edits)
    };
    assert_eq!(file, "model00.yml");
    assert_eq!(name.as_deref(), Some("ALPHA"));
    assert_eq!(ops.len(), 2, "{ops:?}");
    // Two edits of one timer merge their fields.
    let c3 = edit(
        &b,
        vec![ModelOp::SetTimer {
            index: 1,
            fields: vec![Field::new("countdownBeep", "2")],
        }],
        None,
    )
    .unwrap();
    let Edit::Model { ops, .. } = &c3.edits[0] else {
        panic!()
    };
    let t = ops
        .iter()
        .find_map(|o| match o {
            ModelOp::SetTimer { fields, .. } => Some(fields.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(t.len(), 2, "{t:?}");
    // The editor's own read shows the staged model.
    let m = model(&b);
    assert_eq!(m.staged, 2, "a callout and a timer: the timer edits merged");
    let lb = m
        .editors
        .callouts
        .iter()
        .find(|c| c.track == "lowbat")
        .unwrap();
    assert_eq!(
        lb.when,
        Some(CalloutWhen::Below {
            source: "{RxBt}".into(),
            value: "3.4".into(),
            delay_ds: 20
        })
    );
    assert!(model_unstaged(&b)
        .editors
        .callouts
        .iter()
        .all(|c| c.track != "lowbat"));
}

fn model_unstaged(b: &Bench) -> quadcam_lib::core::ModelDetail {
    b.core
        .gear_model(&ModelParams {
            device: b.id.clone(),
            model: Some("model00.yml".into()),
            staged: false,
        })
        .unwrap()
}

#[test]
fn an_edit_the_radio_already_has_drops_out_and_an_empty_change_is_discarded() {
    let b = bench(Opts::default());
    let e = edit(
        &b,
        vec![ModelOp::SetRfAlarms {
            warning: 45,
            critical: 42,
        }],
        None,
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("Nothing changes"), "{e:#}");
    edit(
        &b,
        vec![ModelOp::SetRfAlarms {
            warning: 50,
            critical: 40,
        }],
        None,
    )
    .unwrap();
    assert_eq!(staged(&b).len(), 1);
    // Putting the values back leaves nothing to stage: the change goes.
    let gone = edit(
        &b,
        vec![ModelOp::SetRfAlarms {
            warning: 45,
            critical: 42,
        }],
        None,
    )
    .unwrap();
    assert_eq!(gone.status, ChangeStatus::Discarded);
    assert!(staged(&b).is_empty());
}

#[test]
fn a_refused_edit_stages_nothing() {
    let b = bench(Opts::default());
    let e = edit(
        &b,
        vec![ModelOp::SetLogging {
            logging: Some(LoggingDef {
                swtch: "SA2".into(),
                period_ds: 0,
            }),
        }],
        None,
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("period"), "{e:#}");
    let e = edit(&b, vec![], Some("=123456789012345678901")).unwrap_err();
    assert!(format!("{e:#}").contains("20 characters"), "{e:#}");
    let e = edit(&b, vec![], None).unwrap_err();
    assert!(format!("{e:#}").contains("at least one"), "{e:#}");
    assert!(staged(&b).is_empty());
}

#[test]
fn a_checklist_stages_as_text_and_turns_the_checklist_on() {
    let b = bench(Opts::default());
    let c = edit(
        &b,
        vec![],
        Some("=Props tight\n=Battery strapped\nFail-safe checked"),
    )
    .unwrap();
    assert!(c
        .edits
        .iter()
        .any(|e| matches!(e, Edit::Checklist { model, text } if model == "model00.yml" && text.starts_with("=Props"))));
    let Edit::Model { ops, .. } = &c.edits[0] else {
        panic!("{:?}", c.edits)
    };
    assert!(ops.contains(&ModelOp::SetChecklist { enabled: true }));
    let m = model(&b);
    assert!(m.view.checklist);
    assert!(m.checklist.unwrap().contains("Battery strapped"));
    // Same text as the file has: it drops out.
    let c = b.core.gear_change_discard(&c.id).unwrap();
    assert_eq!(c.status, ChangeStatus::Discarded);
}

fn plan_and_apply(
    b: &Bench,
    c: &quadcam_lib::gear::model::StagedChange,
) -> quadcam_lib::gear::apply::ApplyReport {
    let p = b
        .core
        .gear_apply_plan(&ApplyPlanParams {
            id: c.id.clone(),
            port: None,
        })
        .unwrap();
    assert!(p.checks.iter().all(|k| k.ok), "{:?}", p.checks);
    b.core
        .gear_apply(&ApplyRequest {
            id: c.id.clone(),
            digest: p.digest,
            confirm: true,
            port: None,
        })
        .unwrap()
}

fn read(b: &Bench, rel: &str) -> String {
    String::from_utf8_lossy(&std::fs::read(b.root.join(rel)).unwrap()).to_string()
}

#[test]
fn a_staged_model_edit_applies_to_the_card_with_read_back_and_keeps_other_models() {
    let b = bench(Opts::default());
    let other = read(&b, "MODELS/model01.yml");
    let c = edit(
        &b,
        vec![
            lowbat("3.5"),
            ModelOp::SetLogging {
                logging: Some(LoggingDef {
                    swtch: "SA2".into(),
                    period_ds: 10,
                }),
            },
            ModelOp::SetScreenValues {
                index: 1,
                lines: vec![vec!["{RxBt}".into(), "Tmr1".into()]],
            },
        ],
        Some("=Props tight"),
    )
    .unwrap();
    let r = plan_and_apply(&b, &c);
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    let m = read(&b, "MODELS/model00.yml");
    assert!(m.contains("def: \"lowbat,1,5\""));
    assert!(m.contains("func: LOGS"));
    assert!(m.contains("val: tele(2)"));
    assert!(m.contains("displayChecklist: 1"));
    assert_eq!(read(&b, "MODELS/ALPHA.txt").trim(), "=Props tight");
    assert_eq!(read(&b, "MODELS/model01.yml"), other);
    // The next edit replaces the applied callout by its track: one function, new value.
    b.mounted.store(true, Ordering::SeqCst);
    let c = edit(&b, vec![lowbat("3.3")], None).unwrap();
    let r = plan_and_apply(&b, &c);
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    let m = read(&b, "MODELS/model00.yml");
    assert_eq!(m.matches("lowbat").count(), 1);
    assert!(m.contains("tele(2),33"));
    assert_eq!(m.matches("tele(2),35").count(), 1, "only the fixture's own");
}

#[test]
fn staged_model_edits_on_a_stage_call_use_the_same_engine() {
    // The generic stage call still takes a model edit; the editor's change and a generic
    // one do not mix in one change.
    let b = bench(Opts::default());
    b.core
        .gear_change_stage(&StageParams {
            device: b.id.clone(),
            edits: vec![Edit::Model {
                file: "model00.yml".into(),
                name: Some("ALPHA".into()),
                ops: vec![ModelOp::Rename {
                    name: "ALPHA 2".into(),
                }],
            }],
            ..Default::default()
        })
        .unwrap();
    edit(&b, vec![lowbat("3.5")], None).unwrap();
    assert_eq!(staged(&b).len(), 2, "the editor keeps its own change");
}
