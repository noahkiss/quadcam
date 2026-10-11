//! The EdgeTX card engine (design 6.3, WP3): every synthetic card file parses and renders
//! byte for byte in both layouts and both line endings; each encoding rule has a test;
//! unknown lines refuse with the file and the line; edits are idempotent and keep the
//! radio's selected model; the writer backs up, writes per file, reads back, stops only
//! between files and never writes a real card. Every card here is a temporary folder.

use quadcam_lib::core::{CardParams, CardPreviewParams, Core, NoHooks};
use quadcam_lib::disk::{DiskInfo, Volume};
use quadcam_lib::gear::edgetx::card::{self, Card, RadioOp, WriteOptions};
use quadcam_lib::gear::edgetx::model::{
    self as em, Field, LsDef, MixLine, ModelOp, SfDef, SwitchWarning,
};
use quadcam_lib::gear::edgetx::synth::{self, Layout, SynthCard};
use quadcam_lib::gear::edgetx::yaml::Doc;
use quadcam_lib::gear::model::{Edit, RefusalCode};
use quadcam_lib::gear::serial::FakePorts;
use quadcam_lib::gear::Env;
use quadcam_lib::photos::Recorder;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

fn card_at(root: &Path, s: &SynthCard) -> Card {
    synth::write_card(root, s).unwrap();
    Card::open(root).unwrap()
}

fn model_doc(layout: Layout, crlf: bool) -> Doc {
    Doc::parse(
        "model00.yml",
        synth::model_yml("ALPHA", layout, "2.12.4", crlf).as_bytes(),
    )
    .unwrap()
}

fn apply(layout: Layout, ops: &[ModelOp]) -> Result<String, quadcam_lib::gear::model::Refusal> {
    let mut d = model_doc(layout, false);
    em::apply(&mut d, ops)?;
    Ok(String::from_utf8(d.render()).unwrap())
}

fn model_edit(ops: Vec<ModelOp>) -> Edit {
    Edit::Model {
        file: "model00.yml".into(),
        name: Some("ALPHA".into()),
        ops,
    }
}

#[test]
fn every_fixture_parses_and_renders_byte_for_byte() {
    for crlf in [true, false] {
        for layout in [Layout::Saved212, Layout::Hand210] {
            let raw = synth::model_yml("BRAVO 2", layout, "2.12.4", crlf);
            let d = Doc::parse("m.yml", raw.as_bytes()).unwrap();
            assert_eq!(d.render(), raw.as_bytes(), "{layout:?} crlf={crlf}");
            assert_eq!(d.is_crlf(), crlf);
            let v = em::view("m.yml", &d).unwrap();
            assert_eq!(v.name, "BRAVO 2");
            assert_eq!(v.mixes.len(), 6);
            assert_eq!(v.timers.len(), 2);
            assert_eq!(v.legacy_switch_warning, layout == Layout::Hand210);
        }
        let raw = synth::radio_yml("pocket", "2.12.4", 1, crlf);
        let d = Doc::parse("radio.yml", raw.as_bytes()).unwrap();
        assert_eq!(d.render(), raw.as_bytes());
    }
    // A Latin-1 byte (not UTF-8) survives.
    let mut raw = synth::model_yml("A", Layout::Saved212, "2.12.4", true).into_bytes();
    raw.extend_from_slice(b"ownerName: \"\xe9\xff\"\r\n");
    assert_eq!(Doc::parse("m.yml", &raw).unwrap().render(), raw);
}

#[test]
fn unknown_lines_refuse_with_file_and_line() {
    let mut raw = synth::model_yml("A", Layout::Saved212, "2.12.4", true);
    raw.push_str("weird line\r\n");
    let lines = raw.matches("\r\n").count();
    let e = Doc::parse("model03.yml", raw.as_bytes()).unwrap_err();
    assert_eq!(e.code, RefusalCode::ShapeUnknown);
    assert_eq!(
        e.reason,
        format!("model03.yml line {lines} is not understood; nothing was written.")
    );
    // Through a plan: the check refuses and nothing would be written.
    let dir = tempfile::tempdir().unwrap();
    let c = card_at(dir.path(), &SynthCard::default());
    std::fs::write(dir.path().join("MODELS/model00.yml"), &raw).unwrap();
    let plan = c
        .plan(
            &[model_edit(vec![ModelOp::SetChecklist { enabled: true }])],
            None,
        )
        .unwrap();
    assert!(!plan.ready());
    assert!(plan.files.is_empty());
    let r = plan.checks.last().unwrap().refusal.clone().unwrap();
    assert_eq!(r.code, RefusalCode::ShapeUnknown);
    assert!(r.reason.starts_with("model00.yml line "), "{r:?}");
}

#[test]
fn logical_switches_l1_is_index_0_empty_ones_are_omitted_delays_in_tenths() {
    assert_eq!(em::ls_name(0), "L1");
    assert_eq!(em::ls_index("L13"), Some(12));
    assert_eq!(em::ls_index("!L1"), Some(0));
    assert_eq!(em::ls_index("L65"), None);
    let out = apply(
        Layout::Saved212,
        &[
            ModelOp::SetLogicalSwitch {
                index: 2,
                ls: Some(LsDef {
                    func: "FUNC_VNEG".into(),
                    def: "tele({RxBt}),33".into(),
                    andsw: "NONE".into(),
                    delay: 20, // 2 s
                    duration: 0,
                }),
            },
            ModelOp::SetLogicalSwitch { index: 1, ls: None },
        ],
    )
    .unwrap();
    let d = Doc::parse("m.yml", out.as_bytes()).unwrap();
    let v = em::view("m.yml", &d).unwrap();
    let idx: Vec<u32> = v.logical_switches.iter().map(|l| l.index).collect();
    assert_eq!(idx, vec![0, 2, 3, 4], "L2 omitted, L3 in index order");
    let l3 = &v.logical_switches[1];
    assert_eq!((l3.def.as_str(), l3.delay), ("tele(2),33", 20));
    assert!(out.contains("   2:\n      func: FUNC_VNEG\n      def: \"tele(2),33\"\n      andsw: \"NONE\"\n      lsPersist: 0\n      lsState: 0\n      delay: 20\n      duration: 0\n"));
    assert!(apply(
        Layout::Saved212,
        &[ModelOp::SetLogicalSwitch {
            index: 64,
            ls: None
        }]
    )
    .is_err());
}

#[test]
fn telemetry_sources_resolve_by_label_and_a_missing_label_refuses() {
    let d = model_doc(Layout::Saved212, true);
    assert_eq!(
        em::resolve_sensors(&d, "tele({RQly}),0").unwrap(),
        "tele(1),0"
    );
    assert_eq!(
        em::resolve_sensors(&d, "tele({Capa}),400").unwrap(),
        "tele(3),400"
    );
    let e = em::resolve_sensors(&d, "tele({Alt}),10").unwrap_err();
    assert_eq!(e.code, RefusalCode::BadSetting);
    assert!(e.reason.contains("discover sensors"), "{e}");
}

#[test]
fn switch_sources_and_stick_ranges() {
    assert_eq!(em::switch_source_index("SA0"), Some(0));
    assert_eq!(em::switch_source_index("SA2"), Some(2));
    assert_eq!(em::switch_source_index("SC1"), Some(7));
    assert_eq!(em::switch_source_index("SD3"), None);
    let ls = |def: &str| ModelOp::SetLogicalSwitch {
        index: 14,
        ls: Some(LsDef {
            func: "FUNC_VNEG".into(),
            def: def.into(),
            andsw: "NONE".into(),
            delay: 0,
            duration: 0,
        }),
    };
    assert!(apply(Layout::Saved212, &[ls("Thr,-90")]).is_ok());
    let e = apply(Layout::Saved212, &[ls("Thr,-1000")]).unwrap_err();
    assert!(e.reason.contains("-100..100"), "{e}");
    // Trims by name in an AND.
    let out = apply(
        Layout::Saved212,
        &[ModelOp::SetLogicalSwitch {
            index: 10,
            ls: Some(LsDef {
                func: "FUNC_AND".into(),
                def: "L10,TrimRudRight".into(),
                andsw: "NONE".into(),
                delay: 0,
                duration: 0,
            }),
        }],
    )
    .unwrap();
    assert!(out.contains("def: \"L10,TrimRudRight\""));
}

#[test]
fn switch_warnings_write_the_list_and_replace_the_legacy_line() {
    let want = vec![SwitchWarning {
        switch: "SA".into(),
        pos: "up".into(),
    }];
    for layout in [Layout::Hand210, Layout::Saved212] {
        let out = apply(
            layout,
            &[ModelOp::SetSwitchWarnings {
                warnings: want.clone(),
            }],
        )
        .unwrap();
        assert!(!out.contains("switchWarningState"), "{layout:?}");
        assert!(
            out.contains(
                "thrTraceSrc: Thr\nswitchWarning: \n   SA:\n      pos: up\nrssiSource: none\n"
            ),
            "{layout:?}: {out}"
        );
    }
    let bad = vec![SwitchWarning {
        switch: "SA".into(),
        pos: "sideways".into(),
    }];
    assert!(apply(
        Layout::Saved212,
        &[ModelOp::SetSwitchWarnings { warnings: bad }]
    )
    .is_err());
}

fn sf(sw: &str, func: &str, def: &str) -> SfDef {
    SfDef {
        swtch: sw.into(),
        func: func.into(),
        def: def.into(),
    }
}

#[test]
fn special_functions_limits_and_rules() {
    // 64 at most.
    let many: Vec<SfDef> = (0..61)
        .map(|i| sf("ON", "PLAY_TRACK", &format!("t{i},1,1x")))
        .collect();
    let e = apply(
        Layout::Saved212,
        &[ModelOp::SpecialFunctions {
            remove: vec![],
            add: many,
        }],
    )
    .unwrap_err();
    assert!(e.reason.contains("64"), "{e}");
    // Track names 8 characters at most; !1x and seconds are repeats.
    let add = |s: SfDef| {
        apply(
            Layout::Saved212,
            &[ModelOp::SpecialFunctions {
                remove: vec![],
                add: vec![s],
            }],
        )
    };
    assert!(add(sf("SC2", "PLAY_TRACK", "launches,1,!1x")).is_ok());
    assert!(add(sf("SC2", "PLAY_TRACK", "launchesx,1,1x")).is_err());
    assert!(add(sf("SC2", "PLAY_TRACK", "beep,1,30")).is_ok());
    assert!(add(sf("SC2", "PLAY_TRACK", "beep,1,2x")).is_err());
    // SET_SCREEN repeats while on: only on a FUNC_EDGE logical switch.
    let e = add(sf("SD2", "SET_SCREEN", "3,1,1x")).unwrap_err();
    assert!(e.reason.contains("FUNC_EDGE"), "{e}");
    let out = apply(
        Layout::Saved212,
        &[
            ModelOp::SetLogicalSwitch {
                index: 6,
                ls: Some(LsDef {
                    func: "FUNC_EDGE".into(),
                    def: "SD2,0,<".into(),
                    andsw: "NONE".into(),
                    delay: 0,
                    duration: 0,
                }),
            },
            ModelOp::SpecialFunctions {
                remove: vec![],
                add: vec![sf("L7", "SET_SCREEN", "3,1,1x")],
            },
        ],
    )
    .unwrap();
    assert!(
        out.contains("   4:\n      swtch: \"L7\"\n      func: SET_SCREEN\n      def: \"3,1,1x\"\n")
    );
    // Remove what an earlier change added; move keeps the place.
    let out = apply(
        Layout::Saved212,
        &[
            ModelOp::SpecialFunctions {
                remove: vec![sf("L1", "PLAY_TRACK", "armed,1,1x")],
                add: vec![],
            },
            ModelOp::MoveSpecialFunction {
                from: sf("!L1", "PLAY_TRACK", "disarm,1,!1x"),
                swtch: "!L13".into(),
            },
        ],
    )
    .unwrap();
    let v = em::view("m", &Doc::parse("m", out.as_bytes()).unwrap()).unwrap();
    assert_eq!(v.special_functions.len(), 3);
    assert_eq!(v.special_functions[0].swtch, "!L13");
    assert_eq!(v.special_functions[0].index, 0);
    // Logging is a special function too (every 0.1 s).
    let out = apply(
        Layout::Saved212,
        &[ModelOp::SpecialFunctions {
            remove: vec![],
            add: vec![sf("L1", "LOGS", "1,1")],
        }],
    )
    .unwrap();
    assert!(out.contains("func: LOGS\n      def: \"1,1\""));
}

#[test]
fn script_names_six_characters_and_screens_in_order() {
    let out = apply(
        Layout::Saved212,
        &[ModelOp::SetScreen {
            index: 1,
            script: Some("volt".into()),
        }],
    )
    .unwrap();
    assert!(out.contains("   1:\n      type: SCRIPT\n      u: \n         script: \n            file: \"volt\"\nview: 0"));
    assert!(apply(
        Layout::Saved212,
        &[ModelOp::SetScreen {
            index: 1,
            script: Some("toolong".into())
        }]
    )
    .is_err());
    let e = apply(
        Layout::Saved212,
        &[ModelOp::SetScreen {
            index: 3,
            script: Some("gps".into()),
        }],
    )
    .unwrap_err();
    assert!(e.reason.contains("missing"), "{e}");
}

#[test]
fn model_id_zero_means_no_block() {
    let out = apply(
        Layout::Saved212,
        &[ModelOp::SetModelId { module: 0, id: 1 }],
    )
    .unwrap();
    assert!(out.contains(
        "header: \n   name: \"ALPHA\"\n   modelId: \n      0:\n         val: 1\ntimers: "
    ));
    let mut d = Doc::parse("m", out.as_bytes()).unwrap();
    em::apply(&mut d, &[ModelOp::SetModelId { module: 0, id: 0 }]).unwrap();
    assert_eq!(
        String::from_utf8(d.render()).unwrap(),
        synth::model_yml("ALPHA", Layout::Saved212, "2.12.4", false)
    );
}

#[test]
fn timer_swap_trades_fields_value_and_every_reference() {
    let out = apply(Layout::Saved212, &[ModelOp::SwapTimers { a: 0, b: 1 }]).unwrap();
    let d = Doc::parse("m", out.as_bytes()).unwrap();
    let v = em::view("m", &d).unwrap();
    assert_eq!((v.timers[0].name.as_str(), v.timers[0].value), ("FLT", 0));
    assert_eq!((v.timers[1].name.as_str(), v.timers[1].value), ("TOT", 120));
    // The SF said Timer 2, the screen showed Timer 1: both follow their timer.
    assert!(out.contains("def: \"Tmr1,1,1x\""));
    assert!(out.contains("val: Tmr2"));
    assert_eq!(
        em::swap_tmr_refs("Tmr1 Tmr2 Tmr3 xTmr1 Tmr12", 0, 1),
        "Tmr2 Tmr1 Tmr3 xTmr1 Tmr12"
    );
    // Set never touches the stored value; a new timer copies the file's layout.
    let out = apply(
        Layout::Saved212,
        &[ModelOp::SetTimer {
            index: 2,
            fields: vec![Field::new("name", "DVR"), Field::new("swtch", "L1")],
        }],
    )
    .unwrap();
    assert!(out.contains("   2:\n      start: 0\n      swtch: \"L1\"\n      value: 0\n"));
    assert!(apply(
        Layout::Saved212,
        &[ModelOp::SetTimer {
            index: 0,
            fields: vec![Field::new("value", "0")]
        }]
    )
    .is_err());
}

#[test]
fn mixes_copy_the_files_own_layout_and_quoting() {
    let want = vec![
        MixLine {
            source: "MAX".into(),
            weight: 100,
            swtch: "NONE".into(),
            mltpx: "ADD".into(),
        },
        MixLine {
            source: "MAX".into(),
            weight: -100,
            swtch: "SC1".into(),
            mltpx: "REPL".into(),
        },
    ];
    let saved = apply(
        Layout::Saved212,
        &[ModelOp::SetMixes {
            channel: 15,
            lines: want.clone(),
        }],
    )
    .unwrap();
    assert!(saved.contains(" -\n   destCh: 15\n   srcRaw: \"MAX\"\n   carryTrim: 0\n   mixWarn: 0\n   mltpx: REPL\n   delayPrec: 0\n   speedPrec: 0\n   flightModes: 000000000\n   weight: -100\n   offset: 0\n   swtch: \"SC1\"\n"));
    let hand = apply(
        Layout::Hand210,
        &[ModelOp::SetMixes {
            channel: 15,
            lines: want,
        }],
    )
    .unwrap();
    assert!(hand.contains(" -\n   weight: -100\n   destCh: 15\n   srcRaw: MAX\n"));
    assert!(!hand.contains("delayPrec"));
    // The same mixes again change nothing.
    let mut d = Doc::parse("m", saved.as_bytes()).unwrap();
    let v = em::view("m", &d).unwrap();
    let ch16: Vec<_> = v.mixes.iter().filter(|m| m.dest_ch == 15).collect();
    assert_eq!(ch16.len(), 2);
    em::apply(
        &mut d,
        &[ModelOp::SetMixes {
            channel: 15,
            lines: ch16
                .iter()
                .map(|m| MixLine {
                    source: m.source.clone(),
                    weight: m.weight.parse().unwrap(),
                    swtch: m.swtch.clone(),
                    mltpx: m.mltpx.clone(),
                })
                .collect(),
        }],
    )
    .unwrap();
    assert!(!d.changed());
}

#[test]
fn checklist_name_width_and_on_off() {
    let dir = tempfile::tempdir().unwrap();
    let c = card_at(dir.path(), &SynthCard::default());
    let plan = c
        .plan(
            &[
                Edit::Checklist {
                    model: "model01.yml".into(),
                    text: "=Props tight\n=Antenna clear\n".into(),
                },
                Edit::Model {
                    file: "model01.yml".into(),
                    name: Some("BRAVO 2".into()),
                    ops: vec![ModelOp::SetChecklist { enabled: true }],
                },
            ],
            None,
        )
        .unwrap();
    assert!(plan.ready(), "{:?}", plan.checks);
    let paths: Vec<&str> = plan.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "MODELS/BRAVO_2.txt",
            "MODELS/model01.yml",
            ".metadata_never_index"
        ]
    );
    let after = String::from_utf8(plan.files[1].after.clone().unwrap()).unwrap();
    assert!(after.contains("displayChecklist: 1\r\n"));
    assert!(after.contains("checklistInteractive: 1\r\n"));
    assert!(
        after.starts_with("checksum: 0\r\n"),
        "an edited file with a checksum gets 0"
    );
    // 20 characters per line on a 128x64 radio.
    let plan = c
        .plan(
            &[Edit::Checklist {
                model: "model00.yml".into(),
                text: "=This line is too long for it\n".into(),
            }],
            None,
        )
        .unwrap();
    assert!(!plan.ready());
}

#[test]
fn radio_edits_never_change_the_selected_model_unless_asked() {
    let dir = tempfile::tempdir().unwrap();
    let c = card_at(dir.path(), &SynthCard::default());
    assert_eq!(c.selected_model().as_deref(), Some("model01.yml"));
    let plan = c
        .plan(
            &[Edit::Radio {
                ops: vec![RadioOp::SetScalar {
                    key: "hapticMode".into(),
                    value: "mode_alarms".into(),
                }],
            }],
            None,
        )
        .unwrap();
    assert!(plan.ready());
    assert_eq!(plan.warnings.len(), 1, "mode_alarms drops SF haptics");
    let radio = String::from_utf8(plan.files[0].after.clone().unwrap()).unwrap();
    assert!(radio.contains("currModel: 1\r\n"));
    assert!(radio.starts_with("checksum: 0\r\n"));
    // currModel is never a plain value.
    let plan = c
        .plan(
            &[Edit::Radio {
                ops: vec![RadioOp::SetScalar {
                    key: "currModel".into(),
                    value: "0".into(),
                }],
            }],
            None,
        )
        .unwrap();
    assert!(!plan.ready());
    // Asked for: it changes.
    let plan = c
        .plan(
            &[Edit::Radio {
                ops: vec![RadioOp::SelectModel {
                    file: "model00.yml".into(),
                }],
            }],
            None,
        )
        .unwrap();
    assert!(plan.ready());
    let radio = String::from_utf8(plan.files[0].after.clone().unwrap()).unwrap();
    assert!(radio.contains("currModel: 0\r\n"));
    // Deleting the selected model refuses; copying keeps the selection.
    let plan = c
        .plan(
            &[Edit::ModelDelete {
                file: "model01.yml".into(),
            }],
            None,
        )
        .unwrap();
    assert!(!plan.ready());
    let plan = c
        .plan(
            &[Edit::ModelCopy {
                from: "model01.yml".into(),
                to: "model02.yml".into(),
                name: "CHARLIE".into(),
            }],
            None,
        )
        .unwrap();
    assert!(plan.ready(), "{:?}", plan.checks);
    let copy = String::from_utf8(plan.files[0].after.clone().unwrap()).unwrap();
    assert!(copy.contains("name: \"CHARLIE\""));
    assert!(!copy.contains("value: 120"), "timer values start at 0");
}

#[test]
fn model_identity_and_version_guards() {
    let dir = tempfile::tempdir().unwrap();
    let c = card_at(dir.path(), &SynthCard::default());
    let plan = c
        .plan(
            &[Edit::Model {
                file: "model00.yml".into(),
                name: Some("OTHER".into()),
                ops: vec![],
            }],
            None,
        )
        .unwrap();
    let r = plan.checks.last().unwrap().refusal.clone().unwrap();
    assert_eq!(r.code, RefusalCode::DeviceChanged);
    assert!(r.reason.contains("wrong card?"));
    let old = tempfile::tempdir().unwrap();
    let c = card_at(
        old.path(),
        &SynthCard {
            semver: "2.11.3".into(),
            ..Default::default()
        },
    );
    let plan = c.plan(&[model_edit(vec![])], None).unwrap();
    assert_eq!(
        plan.checks[0].refusal.as_ref().unwrap().code,
        RefusalCode::UnknownVersion
    );
    let v = c
        .view(None, chrono::NaiveDate::from_ymd_opt(2026, 5, 3).unwrap())
        .unwrap();
    assert!(v.read_only.is_some());
}

#[test]
fn edits_are_idempotent_and_the_writer_backs_up_writes_and_reads_back() {
    let dir = tempfile::tempdir().unwrap();
    let c = card_at(dir.path(), &SynthCard::default());
    let edits = vec![
        model_edit(vec![
            ModelOp::SetChecklist { enabled: true },
            ModelOp::SetSwitchWarnings {
                warnings: vec![SwitchWarning {
                    switch: "SA".into(),
                    pos: "up".into(),
                }],
            },
        ]),
        Edit::Model {
            file: "model01.yml".into(),
            name: None,
            ops: vec![ModelOp::SetSwitchWarnings { warnings: vec![] }],
        },
    ];
    let plan = c.plan(&edits, Some("uuid:TEST")).unwrap();
    assert!(plan.ready());
    let paths: Vec<&str> = plan.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "MODELS/model00.yml",
            "MODELS/model01.yml",
            ".quadcam-id",
            ".metadata_never_index"
        ]
    );
    let mut backed = Vec::new();
    let mut seen = Vec::new();
    let report = card::write(
        dir.path(),
        &plan,
        &WriteOptions::reader(),
        &mut |p, b| {
            backed.push((p.to_string(), b.len()));
            Ok(())
        },
        &mut |p| seen.push(p.clone()),
    )
    .unwrap();
    assert_eq!(report.written.len(), 4);
    assert_eq!(backed.len(), 2, "the two existing files");
    assert_eq!(seen.len(), 4);
    assert_eq!(seen[3].file, 4);
    assert!(seen[0].eta_s <= 1);
    assert_eq!(card::read_marker(dir.path()).as_deref(), Some("uuid:TEST"));
    // No temporary file is left.
    let left: Vec<_> = std::fs::read_dir(dir.path().join("MODELS"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.contains("quadcam-tmp"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
    // A second plan of the same edits writes nothing.
    let again = c.plan(&edits, Some("uuid:TEST")).unwrap();
    assert!(again.ready());
    assert!(again.files.is_empty(), "{:?}", again.files);
    // A file that changed after the plan refuses, before any backup.
    let plan = c
        .plan(
            &[model_edit(vec![ModelOp::SetChecklist { enabled: false }])],
            None,
        )
        .unwrap();
    std::fs::write(
        dir.path().join("MODELS/model00.yml"),
        synth::model_yml("ALPHA", Layout::Saved212, "2.12.4", true),
    )
    .unwrap();
    let mut called = false;
    let e = card::write(
        dir.path(),
        &plan,
        &WriteOptions::reader(),
        &mut |_, _| {
            called = true;
            Ok(())
        },
        &mut |_| {},
    )
    .unwrap_err();
    assert!(!called);
    assert_eq!(
        e.downcast_ref::<quadcam_lib::gear::model::Refusal>()
            .unwrap()
            .code,
        RefusalCode::BeforeMismatch
    );
    // A failed backup writes nothing.
    let plan = c
        .plan(
            &[model_edit(vec![ModelOp::SetChecklist { enabled: true }])],
            None,
        )
        .unwrap();
    let before = std::fs::read(dir.path().join("MODELS/model00.yml")).unwrap();
    let e = card::write(
        dir.path(),
        &plan,
        &WriteOptions::reader(),
        &mut |_, _| anyhow::bail!("disk full"),
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(
        e.downcast_ref::<quadcam_lib::gear::model::Refusal>()
            .unwrap()
            .code,
        RefusalCode::NoBackup
    );
    assert_eq!(
        std::fs::read(dir.path().join("MODELS/model00.yml")).unwrap(),
        before
    );
}

#[test]
fn a_stop_takes_effect_between_files() {
    let dir = tempfile::tempdir().unwrap();
    let c = card_at(dir.path(), &SynthCard::default());
    let plan = c
        .plan(
            &[
                model_edit(vec![ModelOp::SetChecklist { enabled: true }]),
                Edit::Model {
                    file: "model01.yml".into(),
                    name: None,
                    ops: vec![ModelOp::SetChecklist { enabled: true }],
                },
            ],
            None,
        )
        .unwrap();
    let opts = WriteOptions::radio_usb();
    let stop = opts.stop.clone();
    let mut progress = Vec::new();
    let report = card::write(dir.path(), &plan, &opts, &mut |_, _| Ok(()), &mut |p| {
        progress.push(p.clone());
        // The person presses Cancel while the first file is being written.
        stop.store(true, Ordering::SeqCst);
    })
    .unwrap();
    assert!(report.stopped);
    assert_eq!(
        report.written,
        vec!["MODELS/model00.yml"],
        "the current file finishes"
    );
    assert_eq!(report.remaining.len(), plan.files.len() - 1);
    assert!(
        progress.iter().any(|p| p.stopping),
        "\"finishing the current file\""
    );
    // The card is whole: each file is either old or new, and each parses.
    for f in ["MODELS/model00.yml", "MODELS/model01.yml"] {
        Doc::parse(f, &std::fs::read(dir.path().join(f)).unwrap()).unwrap();
    }
}

#[test]
fn timeouts_report_a_stuck_disk_instead_of_hanging() {
    let t = std::time::Instant::now();
    let e = card::run_with_timeout(
        std::process::Command::new("/bin/sleep").arg("5"),
        "unmounting the test disk",
        std::time::Duration::from_millis(200),
    )
    .unwrap_err();
    assert!(t.elapsed() < std::time::Duration::from_secs(3));
    assert!(format!("{e}").contains("a reboot may be needed"), "{e}");
    let e = card::with_timeout(std::time::Duration::from_millis(100), "writing X", || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        Ok(())
    })
    .unwrap_err();
    assert!(format!("{e}").contains("writing X"));
    // The error can wait for the thread it left running, and does not tell the person to
    // leave the card alone: the caller decides what comes next.
    let s = e.downcast_ref::<card::Stuck>().expect("a Stuck error");
    assert!(!format!("{e}").contains("do not pull"), "{e}");
    assert!(
        s.wait(std::time::Duration::from_secs(10)),
        "the thread ended"
    );
    assert!(card::run_with_timeout(
        std::process::Command::new("/bin/echo").arg("ok"),
        "echo",
        std::time::Duration::from_secs(5)
    )
    .unwrap()
    .status
    .success());
    // Timeouts grow with the file: 37 MB over a radio's USB is minutes, not seconds.
    let o = WriteOptions::radio_usb();
    assert!(o.timeout_for(37_000_000) > std::time::Duration::from_secs(300));
}

#[test]
fn tests_never_write_or_unmount_a_real_card() {
    assert!(!card::writes_allowed(
        Path::new("/Volumes/RADIO"),
        None,
        true
    ));
    assert!(card::writes_allowed(
        Path::new("/Volumes/RADIO"),
        Some("real"),
        true
    ));
    assert!(card::writes_allowed(Path::new("/tmp/card"), None, true));
    if std::env::var("QUADCAM_SERIAL").as_deref() != Ok("real") {
        let e = card::release("disk99", std::time::Duration::from_secs(1)).unwrap_err();
        assert_eq!(
            e.downcast_ref::<quadcam_lib::gear::model::Refusal>()
                .unwrap()
                .code,
            RefusalCode::Disabled
        );
    }
}

#[test]
fn apple_double_files_are_removed_beside_written_files() {
    let dir = tempfile::tempdir().unwrap();
    let ad = dir.path().join("._model00.yml");
    std::fs::write(&ad, [0x00, 0x05, 0x16, 0x07, 0, 0]).unwrap();
    let other = dir.path().join("._keep.txt");
    std::fs::write(&other, b"not apple double").unwrap();
    assert!(card::write_file_atomic(&dir.path().join("model00.yml"), b"a: 1\r\n").unwrap());
    assert!(!ad.exists());
    card::remove_apple_double(dir.path(), "keep.txt");
    assert!(other.exists(), "only AppleDouble files are removed");
}

#[test]
fn the_radio_clock_check() {
    let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 7).unwrap();
    let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let ok = card::clock_check(&names(&["A-2026-10-05.csv", "B-2026-10-07.csv"]), today);
    assert!(ok.ok);
    assert_eq!(ok.newest_log.as_deref(), Some("B-2026-10-07.csv"));
    let reset = card::clock_check(&names(&["A-2000-01-01.csv"]), today);
    assert!(!reset.ok);
    assert!(reset.message.unwrap().contains("looks reset"));
    let once = card::clock_check(&names(&["A-2000-01-01.csv", "B-2026-10-07.csv"]), today);
    assert!(!once.ok);
    assert!(once.message.unwrap().contains("clock battery"));
    let ahead = card::clock_check(&names(&["A-2027-01-01.csv"]), today);
    assert!(!ahead.ok);
    assert!(card::clock_check(&names(&["readme.txt"]), today).ok);
}

fn volume(root: &Path, uuid: &str) -> Volume {
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
        is_radio: quadcam_lib::disk::looks_like_radio(root),
        warnings: vec![],
    }
}

fn core_on(
    dir: &Path,
    vols: Vec<Volume>,
    usb: bool,
    unmount_ok: bool,
) -> (Core, Arc<quadcam_lib::gear::cues::RecordedCues>) {
    let cues = Arc::new(quadcam_lib::gear::cues::RecordedCues::default());
    let mut env = Env::fake(vols, Arc::new(FakePorts::new(vec![])));
    env.cues = Arc::new(quadcam_lib::gear::cues::CueService::inline(cues.clone()));
    if usb {
        env.usb = Arc::new(|| quadcam_lib::gear::detect::parse_ioreg_usb(IOREG.as_bytes()));
    }
    let calls = Arc::new(Mutex::new(0));
    env.unmount = Arc::new(move |_| {
        *calls.lock().unwrap() += 1;
        if unmount_ok {
            Ok(())
        } else {
            anyhow::bail!(card::stuck_message(
                "unmounting disk42",
                std::time::Duration::from_secs(60)
            ))
        }
    });
    let core = Core::new(
        dir.join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(dir.join("support/settings.json"))
    .with_gear_env(env);
    (core, cues)
}

/// `ioreg -a` output for a radio in USB Storage mode, made up (values like a real one).
const IOREG: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><array><dict>
<key>IORegistryEntryName</key><string>Test Radio Mass Storage</string>
<key>idVendor</key><integer>1155</integer>
<key>idProduct</key><integer>22304</integer>
<key>bcdDevice</key><integer>530</integer>
<key>USB Vendor Name</key><string>OpenTX</string>
<key>USB Product Name</key><string>Test Radio Mass Storage</string>
<key>USB Serial Number</key><string>00000000001B</string>
<key>IORegistryEntryChildren</key><array><dict>
  <key>IORegistryEntryChildren</key><array>
    <dict><key>BSD Name</key><string>disk42</string>
      <key>IORegistryEntryChildren</key><array><dict><key>BSD Name</key><string>disk42s1</string></dict></array>
    </dict>
  </array>
</dict></array>
</dict></array></plist>"#;

#[test]
fn a_radio_over_usb_is_identified_and_its_card_reads() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("RADIO");
    synth::write_card(&root, &SynthCard::default()).unwrap();
    let (core, _) = core_on(dir.path(), vec![volume(&root, "TEST-UUID")], true, true);
    let found = core.gear_connected().unwrap();
    let usb = found[0].usb.clone().expect("the radio's USB device");
    assert_eq!((usb.vid, usb.pid), (0x0483, 0x5720));
    assert_eq!(
        usb.version.as_deref(),
        Some("2.12"),
        "bcdDevice 0x0212 (BCD)"
    );
    // The serial is generic; the id is the volume's until the card has a marker.
    let uuid_id = quadcam_lib::gear::model::device_id(
        quadcam_lib::gear::model::DeviceKind::Radio,
        "TEST-UUID",
    );
    assert_eq!(found[0].id.as_deref(), Some(uuid_id.as_str()));
    std::fs::write(root.join(".quadcam-id"), "TEST-UUID\n").unwrap();
    assert_eq!(
        core.gear_connected().unwrap()[0].id.as_deref(),
        Some(uuid_id.as_str())
    );

    // A profile names the selected model's EdgeTX name.
    let s = dir.path().join("support/settings.json");
    std::fs::create_dir_all(s.parent().unwrap()).unwrap();
    std::fs::write(
        &s,
        r#"{"profiles":[{"name":"Whoop B","aircraft":"","camera_make":"","camera_model":"","video_system":"","keywords":[],"author":"","place":null,"edgetx_models":["bravo 2"]}]}"#,
    )
    .unwrap();
    let v = core.gear_card(&CardParams::default()).unwrap();
    assert!(v.radio_usb);
    assert_eq!(v.card.selected_model.as_deref(), Some("model01.yml"));
    assert_eq!(v.card.selected_name.as_deref(), Some("BRAVO 2"));
    assert_eq!(v.selected_aircraft.as_deref(), Some("Whoop B"));
    assert!(v.card.read_only.is_none());
    assert_eq!(v.card.models.len(), 2);
    let v = core
        .gear_card(&CardParams {
            model: Some("model00.yml".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(v.card.model.unwrap().sensors.len(), 4);

    let p = core
        .gear_card_preview(&CardPreviewParams {
            edits: vec![model_edit(vec![ModelOp::SetChecklist { enabled: true }])],
            ..Default::default()
        })
        .unwrap();
    assert!(p.ready && p.radio_usb);
    assert_eq!(p.files, vec!["MODELS/model00.yml", ".metadata_never_index"]);
    // The preview wrote nothing.
    assert!(!root.join(".metadata_never_index").exists());
}

#[test]
fn the_radio_serial_port_is_a_radio_not_an_fc() {
    use quadcam_lib::gear::detect::classify_port;
    use quadcam_lib::gear::model::DeviceKind;
    use quadcam_lib::gear::serial::PortInfo;
    let p = PortInfo {
        port: "/dev/cu.usbmodemTEST1".into(),
        vid: 0x0483,
        pid: 0x5740,
        manufacturer: Some("OpenTX".into()),
        product: Some("Test Radio Serial Port".into()),
        serial_number: None,
    };
    assert_eq!(classify_port(&p), Some(DeviceKind::Radio));
    let fc = PortInfo {
        manufacturer: Some("Betaflight".into()),
        product: Some("STM32 Virtual ComPort".into()),
        ..p
    };
    assert_eq!(classify_port(&fc), Some(DeviceKind::Fc));
}

#[test]
fn safe_to_unplug_only_after_the_unmount() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("RADIO");
    synth::write_card(&root, &SynthCard::default()).unwrap();
    let (core, cues) = core_on(dir.path(), vec![volume(&root, "U")], false, true);
    let c = core.gear_connected().unwrap().remove(0);
    core.gear_release_card(&c).unwrap();
    assert_eq!(cues.spoken(), vec!["The Radio done, safe to unplug."]);

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("RADIO");
    synth::write_card(&root, &SynthCard::default()).unwrap();
    let (core, cues) = core_on(dir.path(), vec![volume(&root, "U")], false, false);
    let c = core.gear_connected().unwrap().remove(0);
    let e = core.gear_release_card(&c).unwrap_err();
    assert!(format!("{e}").contains("a reboot may be needed"));
    let said = cues.spoken();
    assert_eq!(said.len(), 1);
    assert!(!said[0].contains("safe to unplug"), "{said:?}");
}

#[test]
fn golden_edits_in_both_layouts() {
    let ops = vec![
        ModelOp::Rename {
            name: "ALPHA 2".into(),
        },
        ModelOp::SetModelId { module: 0, id: 3 },
        ModelOp::SetChecklist { enabled: true },
        ModelOp::SwapTimers { a: 0, b: 1 },
        ModelOp::SetMixes {
            channel: 2,
            lines: vec![
                MixLine {
                    source: "I2".into(),
                    weight: 100,
                    swtch: "NONE".into(),
                    mltpx: "ADD".into(),
                },
                MixLine {
                    source: "MAX".into(),
                    weight: -100,
                    swtch: "L16".into(),
                    mltpx: "REPL".into(),
                },
            ],
        },
        ModelOp::SetLogicalSwitch {
            index: 9,
            ls: Some(LsDef {
                func: "FUNC_VPOS".into(),
                def: "tele({RQly}),0".into(),
                andsw: "NONE".into(),
                delay: 0,
                duration: 0,
            }),
        },
        ModelOp::SpecialFunctions {
            remove: vec![sf("L2", "PLAY_VALUE", "tele({RxBt}),1,10")],
            add: vec![
                sf("SC2", "PLAY_TRACK", "launch,1,!1x"),
                sf("L1", "LOGS", "1,1"),
            ],
        },
        ModelOp::SetSwitchWarnings {
            warnings: vec![SwitchWarning {
                switch: "SA".into(),
                pos: "up".into(),
            }],
        },
        ModelOp::SetScreen {
            index: 1,
            script: Some("volt".into()),
        },
    ];
    for layout in [Layout::Saved212, Layout::Hand210] {
        let out = apply(layout, &ops).unwrap();
        insta::assert_snapshot!(format!("edgetx_golden_{layout:?}"), out);
        // Idempotent: the same ops again change nothing (the swap aside, which trades back).
        let mut d = Doc::parse("m", out.as_bytes()).unwrap();
        let again: Vec<ModelOp> = ops
            .iter()
            .filter(|o| !matches!(o, ModelOp::SwapTimers { .. }))
            .cloned()
            .collect();
        em::apply(&mut d, &again).unwrap();
        assert!(!d.changed(), "{layout:?}");
    }
}

#[test]
fn the_mcp_card_actions_read_and_preview() {
    use quadcam_lib::mcp::{LocalBackend, Server};
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("RADIO");
    synth::write_card(&root, &SynthCard::default()).unwrap();
    let (core, _) = core_on(dir.path(), vec![volume(&root, "U")], false, true);
    let mut s = Server::new(LocalBackend(Arc::new(core)));
    let r = s.call_tool("quadcam_gear", json!({"action": "card"}));
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("EdgeTX 2.12.4 on pocket"), "{text}");
    assert!(
        text.contains("model01.yml \"BRAVO 2\" (selected)"),
        "{text}"
    );
    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "card_preview", "edits": [
            {"kind": "model", "file": "model00.yml", "name": "ALPHA",
             "ops": [{"op": "set_checklist", "enabled": true}]}
        ]}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("ok Known version"), "{text}");
    assert!(text.contains("-displayChecklist: 0"), "{text}");
    assert!(text.contains("+displayChecklist: 1"), "{text}");
    assert!(
        !root.join(".metadata_never_index").exists(),
        "a preview writes nothing"
    );
    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "card_preview", "edits": [
            {"kind": "radio", "ops": [{"op": "set_scalar", "key": "currModel", "value": "0"}]}
        ]}),
    );
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("REFUSED"), "{text}");
}

#[test]
fn a_write_past_its_timeout_reports_stuck_and_can_be_waited_for() {
    let dir = tempfile::tempdir().unwrap();
    let c = card_at(dir.path(), &SynthCard::default());
    let plan = c
        .plan(
            &[model_edit(vec![ModelOp::SetChecklist { enabled: true }])],
            None,
        )
        .unwrap();
    let mut opts = WriteOptions::reader();
    opts.base_timeout = std::time::Duration::from_millis(100);
    opts.floor_bytes_per_s = u64::MAX;
    opts.stall = Some((
        "MODELS/model00.yml".into(),
        std::time::Duration::from_millis(600),
    ));
    let e = card::write(dir.path(), &plan, &opts, &mut |_, _| Ok(()), &mut |_| {}).unwrap_err();
    let s = e.downcast_ref::<card::Stuck>().expect("a Stuck error");
    // The write goes on after the timeout: the file is not new yet, then it is.
    assert!(s.wait(std::time::Duration::from_secs(20)));
    let f = plan
        .files
        .iter()
        .find(|f| f.path == "MODELS/model00.yml")
        .unwrap();
    assert_eq!(
        std::fs::read(dir.path().join(&f.path)).ok(),
        f.after,
        "the late write landed"
    );
    // No file after the stuck one was started.
    assert!(!dir.path().join(".metadata_never_index").exists());
}

#[test]
fn the_writer_refuses_a_path_outside_the_card() {
    for p in [
        "MODELS/model00.yml",
        "RADIO/radio.yml",
        ".metadata_never_index",
        "SOUNDS/en/a b.wav",
    ] {
        assert!(card::inside_card(p), "{p}");
    }
    for p in [
        "",
        "/tmp/x.txt",
        "../x.txt",
        "MODELS/../../x.txt",
        "MODELS/./x",
        "MODELS//x",
        "MODELS\\..\\x",
    ] {
        assert!(!card::inside_card(p), "{p}");
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("CARD");
    let c = card_at(&root, &SynthCard::default());
    for bad in [
        "../outside.txt",
        "MODELS/../../outside.txt",
        "/tmp/quadcam-outside.txt",
    ] {
        let mut plan = c
            .plan(
                &[model_edit(vec![ModelOp::SetChecklist { enabled: true }])],
                None,
            )
            .unwrap();
        plan.files.push(card::FileChange {
            path: bad.into(),
            before: None,
            after: Some(b"x".to_vec()),
        });
        let before = std::fs::read(root.join("MODELS/model00.yml")).unwrap();
        let e = card::write(
            &root,
            &plan,
            &WriteOptions::reader(),
            &mut |_, _| Ok(()),
            &mut |_| {},
        )
        .unwrap_err();
        assert_eq!(
            e.downcast_ref::<quadcam_lib::gear::model::Refusal>()
                .unwrap()
                .code,
            RefusalCode::ShapeUnknown
        );
        assert!(!dir.path().join("outside.txt").exists());
        assert_eq!(
            std::fs::read(root.join("MODELS/model00.yml")).unwrap(),
            before,
            "nothing was written"
        );
    }
}

#[test]
fn a_model_name_that_leaves_models_is_refused() {
    for bad in ["../../../tmp/x", "a/b", ".hidden", "x..y", "..", "a\\b"] {
        assert!(
            apply(Layout::Saved212, &[ModelOp::Rename { name: bad.into() }]).is_err(),
            "{bad}"
        );
    }
    assert!(apply(
        Layout::Saved212,
        &[ModelOp::Rename {
            name: "ALPHA 2.1".into()
        }]
    )
    .is_ok());
    // A name already on the card that would escape: the checklist edit refuses.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("CARD");
    let c = card_at(&root, &SynthCard::default());
    std::fs::write(
        root.join("MODELS/model00.yml"),
        synth::model_yml("../../x", Layout::Saved212, "2.12.4", true),
    )
    .unwrap();
    let plan = c
        .plan(
            &[Edit::Checklist {
                model: "model00.yml".into(),
                text: "=Props\n".into(),
            }],
            None,
        )
        .unwrap();
    assert!(!plan.ready());
    let why = plan
        .checks
        .iter()
        .find_map(|k| k.refusal.as_ref())
        .unwrap()
        .to_string();
    assert!(why.contains("outside MODELS/"), "{why}");
    assert!(plan.files.iter().all(|f| card::inside_card(&f.path)));
}
