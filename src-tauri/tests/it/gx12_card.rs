//! The GX12's card (design 6.3, "Boards"): its factory files and a synthetic 2.12.5 card
//! render byte for byte; the card reads as a GX12 with its custom switches; every card edit
//! op plans valid YAML on it and leaves the custom-switch setup alone; a model copied from
//! a Pocket maps its controls (SE to SH), takes the GX12 card's custom-switch setup and
//! lists what it cannot map. Every card here is a temporary folder.

use quadcam_lib::core::{CardPreviewParams, Core, ModelImportParams, NoHooks};
use quadcam_lib::gear::edgetx::board::{
    self, BoardControlKind, ControlLink, ControlSource, CustomLayout,
};
use quadcam_lib::gear::edgetx::card::{Card, RadioOp, WriteOptions};
use quadcam_lib::gear::edgetx::model::{self as em, MixLine, ModelOp, SfDef, SwitchWarning};
use quadcam_lib::gear::edgetx::synth::{self, Layout, SynthCard};
use quadcam_lib::gear::edgetx::yaml::{latin1, Doc};
use quadcam_lib::gear::model::{Edit, RefusalCode};
use quadcam_lib::gear::serial::FakePorts;
use quadcam_lib::gear::switchmap;
use quadcam_lib::gear::Env;
use quadcam_lib::photos::Recorder;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/edgetx/gx12-factory")
}

fn today() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 10, 22).unwrap()
}

/// The factory files in a temporary folder, `radio.yml` saying `semver` (the radio rewrites
/// it on its first boot of a new release; the model files keep their 2.11 layout).
fn factory_card(dir: &Path, semver: &str) -> Card {
    for f in ["RADIO/radio.yml", "MODELS/model00.yml"] {
        let to = dir.join(f);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        let b = std::fs::read(fixture().join(f)).unwrap();
        let b = latin1(&b).replace("semver: 2.11.0\r\n", &format!("semver: {semver}\r\n"));
        let b: Vec<u8> = b.chars().map(|c| c as u32 as u8).collect();
        std::fs::write(&to, b).unwrap();
    }
    Card::open(dir).unwrap()
}

fn synth_card(dir: &Path) -> Card {
    synth::write_card(dir, &SynthCard::gx12()).unwrap();
    Card::open(dir).unwrap()
}

fn after(plan: &quadcam_lib::gear::edgetx::card::CardPlan, path: &str) -> String {
    let f = plan
        .files
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path} not in the plan: {:?}", plan.files));
    latin1(f.after.as_ref().unwrap())
}

/// The lines from `key:` up to the next top-level key.
fn block(text: &str, key: &str) -> String {
    let mut out = String::new();
    let mut on = false;
    for l in text.split_inclusive('\n') {
        if !l.starts_with(' ') && !l.starts_with('-') {
            on = l.starts_with(&format!("{key}:"));
        }
        if on {
            out.push_str(l);
        }
    }
    out
}

#[test]
fn the_factory_files_render_byte_for_byte_and_read_as_a_gx12() {
    for f in ["RADIO/radio.yml", "MODELS/model00.yml"] {
        let raw = std::fs::read(fixture().join(f)).unwrap();
        let d = Doc::parse(f, &raw).unwrap();
        assert_eq!(d.render(), raw, "{f}");
        assert!(d.is_crlf());
        d.tree().unwrap();
    }
    // As shipped (2.11.0) the card reads but is not written.
    let dir = tempfile::tempdir().unwrap();
    let c = factory_card(dir.path(), "2.11.0");
    let v = c.view(Some("model00.yml"), today()).unwrap();
    assert_eq!(v.identity.board.as_deref(), Some("gx12"));
    assert_eq!(
        v.read_only.as_ref().map(|r| r.code),
        Some(RefusalCode::UnknownVersion)
    );
    assert_eq!(v.selected_model.as_deref(), Some("model00.yml"));
    assert_eq!(v.selected_name.as_deref(), Some("RM-GX12"));
    assert_eq!(v.voice_folder.as_deref(), Some("SOUNDS/en"));
    // The switch types are read in either case: 2.11 wrote `2pos`.
    let ty = |n: &str| {
        v.controls
            .iter()
            .find(|c| c.name == n)
            .unwrap_or_else(|| panic!("no {n}: {:?}", v.controls))
            .clone()
    };
    for (n, t) in [
        ("SA", "2POS"),
        ("SB", "3POS"),
        ("SE", "3POS"),
        ("SF", "3POS"),
        ("SG", "TOGGLE"),
        ("SH", "TOGGLE"),
    ] {
        assert_eq!(ty(n).switch_type.as_deref(), Some(t), "{n}");
        assert_eq!(ty(n).source, ControlSource::Radio);
    }
    for n in ["SW1", "SW6"] {
        let c = ty(n);
        assert_eq!(c.kind, BoardControlKind::Custom);
        assert_eq!(c.switch_type.as_deref(), Some("2POS"));
        assert_eq!(c.source, ControlSource::Model);
    }
    assert!(ty("P1").detent && ty("P2").detent);
    assert_eq!(ty("SL1").kind, BoardControlKind::Slider);
    assert_eq!(v.controls.len(), 8 + 6 + 2 + 2);
    // The model's custom switches, from the 2.11 bit fields.
    let m = v.model.unwrap();
    assert_eq!(m.custom_switches.len(), 6);
    assert!(m
        .custom_switches
        .iter()
        .all(|c| c.layout == CustomLayout::Legacy));
    assert!(m.custom_switches[0].state);
    assert_eq!(m.custom_switches[0].on_color, Some([248, 0, 0]));
    assert_eq!(m.custom_switches[5].off_color, Some([0, 0, 248]));
    assert_eq!(m.switch_warnings.len(), 4);
    // After the flash `radio.yml` says 2.12.5: the card is written.
    let dir = tempfile::tempdir().unwrap();
    let c = factory_card(dir.path(), "2.12.5");
    assert!(c.view(None, today()).unwrap().read_only.is_none());
    // Another release stays read-only until it is proven.
    let dir = tempfile::tempdir().unwrap();
    let c = factory_card(dir.path(), "2.12.6");
    assert!(c.view(None, today()).unwrap().read_only.is_some());
}

#[test]
fn the_synthetic_gx12_card_renders_and_reads_both_custom_switch_layouts() {
    for crlf in [true, false] {
        for layout in [Layout::Saved212, Layout::Hand210] {
            let mut raw = synth::model_yml("ALPHA", layout, "2.12.5", crlf);
            raw += &synth::gx12_custom_switches(layout, crlf);
            let d = Doc::parse("m.yml", raw.as_bytes()).unwrap();
            assert_eq!(d.render(), raw.as_bytes());
            let cs = board::custom_switches(&d);
            assert_eq!(cs.len(), 6, "{layout:?}");
            assert!(cs.iter().all(|c| c.switch_type == "2POS" && c.group == 1));
        }
        let raw = synth::radio_yml("gx12", "2.12.5", 1, crlf);
        assert_eq!(
            Doc::parse("radio.yml", raw.as_bytes()).unwrap().render(),
            raw.as_bytes()
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let c = synth_card(dir.path());
    let v = c.view(Some("model00.yml"), today()).unwrap();
    assert!(v.read_only.is_none(), "{:?}", v.read_only);
    let m = v.model.unwrap();
    assert_eq!(m.custom_switches[0].start, "on");
    assert_eq!(m.custom_switches[1].start, "off");
    assert_eq!(m.custom_switches[0].layout, CustomLayout::Blocks);
    assert_eq!(m.custom_switches[2].on_color, Some([255, 120, 255]));
    // The selected model (model01, the 2.11 bit fields) sets the inventory's custom switches.
    assert!(v
        .controls
        .iter()
        .any(|c| c.name == "SW3" && c.source == ControlSource::Model));
}

fn sf(sw: &str, func: &str, def: &str) -> SfDef {
    SfDef {
        swtch: sw.into(),
        func: func.into(),
        def: def.into(),
    }
}

/// Every model op the editors and the card patcher's rules use.
fn every_model_op() -> Vec<ModelOp> {
    vec![
        ModelOp::Rename {
            name: "GX ONE".into(),
        },
        ModelOp::SetModelId { module: 0, id: 7 },
        ModelOp::SetChecklist { enabled: true },
        ModelOp::SetMixes {
            channel: 8,
            lines: vec![MixLine {
                source: "SH".into(),
                weight: 100,
                swtch: "NONE".into(),
                mltpx: "ADD".into(),
            }],
        },
        ModelOp::SetLogicalSwitch {
            index: 9,
            ls: Some(em::LsDef {
                func: "FUNC_EDGE".into(),
                def: "SH2,0,0".into(),
                andsw: "NONE".into(),
                delay: 0,
                duration: 0,
            }),
        },
        ModelOp::SpecialFunctions {
            remove: vec![],
            add: vec![
                sf("SW12", "PLAY_TRACK", "launch,1,!1x"),
                sf("L10", "HAPTIC", "1"),
            ],
        },
        ModelOp::SetSwitchWarnings {
            warnings: vec![
                SwitchWarning {
                    switch: "SA".into(),
                    pos: "up".into(),
                },
                SwitchWarning {
                    switch: "SF".into(),
                    pos: "up".into(),
                },
            ],
        },
        ModelOp::SetScreen {
            index: 1,
            script: Some("volt".into()),
        },
    ]
}

#[test]
fn each_edit_op_on_a_gx12_card_keeps_the_custom_switch_setup() {
    for (label, synth) in [("factory", false), ("synthetic", true)] {
        let dir = tempfile::tempdir().unwrap();
        let c = if synth {
            synth_card(dir.path())
        } else {
            factory_card(dir.path(), "2.12.5")
        };
        let (file, name) = if synth {
            ("model00.yml", "ALPHA")
        } else {
            ("model00.yml", "RM-GX12")
        };
        let before = latin1(&c.read(&format!("MODELS/{file}")).unwrap().unwrap());
        let edits = vec![
            Edit::Model {
                file: file.into(),
                name: Some(name.into()),
                ops: every_model_op(),
            },
            Edit::Checklist {
                model: file.into(),
                text: "=Props tight\n=Antenna on\n".into(),
            },
            Edit::Radio {
                ops: vec![RadioOp::SetScalar {
                    key: "hapticMode".into(),
                    value: "mode_alarms".into(),
                }],
            },
            Edit::ModelCopy {
                from: file.into(),
                to: "model05.yml".into(),
                name: "GX COPY".into(),
            },
        ];
        let plan = c.plan(&edits, None).unwrap();
        assert!(plan.ready(), "{label}: {:?}", plan.checks);
        let model = after(&plan, &format!("MODELS/{file}"));
        let d = Doc::parse(file, model.as_bytes()).unwrap();
        d.tree().unwrap();
        // Every custom-switch line stays as it was.
        for k in [
            "customSwitches",
            "cfsGroupOn",
            "functionSwitchConfig",
            "functionSwitchLedONColor",
        ] {
            assert_eq!(block(&model, k), block(&before, k), "{label} {k}");
        }
        assert!(model.contains("srcRaw: \"SH\""), "{label}");
        assert!(model.contains("def: \"SH2,0,0\""), "{label}");
        assert!(model.contains("swtch: \"SW12\""), "{label}");
        assert!(
            model.contains("   SF:\r\n      pos: up\r\n")
                || model.contains("   SF:\n      pos: up\n"),
            "{label}"
        );
        assert!(!model.contains("switchWarningState"), "{label}");
        // The view reads the result back.
        let v = em::view(file, &d).unwrap();
        assert_eq!(v.name, "GX ONE");
        assert_eq!(v.custom_switches.len(), 6);
        // The checklist: 20 characters a line, as on the Pocket.
        assert!(plan.files.iter().any(|f| f.path == "MODELS/GX_ONE.txt"));
        // radio.yml: the value, checksum 0 (the file has a checksum line).
        let radio = after(&plan, "RADIO/radio.yml");
        assert!(radio.starts_with("checksum: 0"), "{label}");
        assert!(radio.contains("hapticMode: mode_alarms"));
        assert!(
            plan.warnings.iter().any(|w| w.contains("haptic")),
            "{:?}",
            plan.warnings
        );
        // The copy keeps the custom switches, timers 0, no model id.
        let copy = after(&plan, "MODELS/model05.yml");
        assert_eq!(
            board::custom_switches(&Doc::parse("c", copy.as_bytes()).unwrap()).len(),
            6
        );
        assert!(copy.contains("name: \"GX COPY\""));
        // Idempotent: the same edits on the result plan no change to the model.
        for f in &plan.files {
            let to = dir.path().join(&f.path);
            match &f.after {
                Some(b) => std::fs::write(to, b).unwrap(),
                None => std::fs::remove_file(to).unwrap(),
            }
        }
        let mut again = edits[..3].to_vec();
        if let Edit::Model { name, .. } = &mut again[0] {
            *name = Some("GX ONE".into());
        }
        let again = Card::open(dir.path()).unwrap().plan(&again, None).unwrap();
        assert!(again.ready(), "{:?}", again.checks);
        assert!(again.files.is_empty(), "{label}: {:?}", again.files);
        // Delete the copy (never the selected model).
        let c = Card::open(dir.path()).unwrap();
        let del = c
            .plan(
                &[Edit::ModelDelete {
                    file: "model05.yml".into(),
                }],
                None,
            )
            .unwrap();
        assert!(del.ready());
        assert!(del
            .files
            .iter()
            .any(|f| f.path == "MODELS/model05.yml" && f.after.is_none()));
    }
}

#[test]
fn a_checklist_line_over_20_characters_refuses_on_a_gx12() {
    let dir = tempfile::tempdir().unwrap();
    let c = synth_card(dir.path());
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

/// A Pocket model that uses every Pocket control: SE as a mix source and in a logical
/// switch, the P1 wheel, SA and SD in the switch warnings.
fn pocket_model() -> String {
    let mut d = Doc::parse(
        "model01.yml",
        synth::model_yml("BRAVO 2", Layout::Saved212, "2.12.4", true).as_bytes(),
    )
    .unwrap();
    em::apply(
        &mut d,
        &[
            ModelOp::SetMixes {
                channel: 8,
                lines: vec![MixLine {
                    source: "SE".into(),
                    weight: 100,
                    swtch: "NONE".into(),
                    mltpx: "ADD".into(),
                }],
            },
            ModelOp::SetMixes {
                channel: 9,
                lines: vec![MixLine {
                    source: "P1".into(),
                    weight: 100,
                    swtch: "!SE2".into(),
                    mltpx: "ADD".into(),
                }],
            },
            ModelOp::SetLogicalSwitch {
                index: 14,
                ls: Some(em::LsDef {
                    func: "FUNC_AND".into(),
                    def: "SE2,SC1".into(),
                    andsw: "SD2".into(),
                    delay: 0,
                    duration: 0,
                }),
            },
            ModelOp::SpecialFunctions {
                remove: vec![],
                // A track named like a control stays a track.
                add: vec![sf("SE2", "PLAY_TRACK", "SE,1,1x")],
            },
            ModelOp::SetSwitchWarnings {
                warnings: vec![
                    SwitchWarning {
                        switch: "SA".into(),
                        pos: "up".into(),
                    },
                    SwitchWarning {
                        switch: "SD".into(),
                        pos: "up".into(),
                    },
                ],
            },
        ],
    )
    .unwrap();
    latin1(&d.render())
}

fn import(text: &str, from_board: &str, links: Vec<ControlLink>) -> Edit {
    Edit::ModelImport {
        from_board: from_board.into(),
        text: text.into(),
        to: "model02.yml".into(),
        name: "FROM POCKET".into(),
        links,
        template: None,
    }
}

#[test]
fn a_pocket_model_copied_to_a_gx12_maps_its_controls_and_takes_the_custom_switches() {
    let dir = tempfile::tempdir().unwrap();
    let c = synth_card(dir.path());
    let plan = c
        .plan(&[import(&pocket_model(), "pocket", vec![])], None)
        .unwrap();
    assert!(plan.ready(), "{:?}", plan.checks);
    let m = after(&plan, "MODELS/model02.yml");
    let d = Doc::parse("model02.yml", m.as_bytes()).unwrap();
    d.tree().unwrap();
    // SE (momentary) became SH; the rest kept their names; positions kept.
    assert!(m.contains("srcRaw: \"SH\""), "{m}");
    assert!(m.contains("swtch: \"!SH2\""));
    assert!(m.contains("def: \"SH2,SC1\""));
    assert!(m.contains("andsw: \"SD2\""));
    assert!(m.contains("srcRaw: \"P1\""));
    assert!(
        m.contains("def: \"SE,1,1x\""),
        "a track name is not a control"
    );
    assert!(!m.contains("srcRaw: \"SE\"") && !m.contains("SE2"), "{m}");
    assert!(m.contains("   SA:\r\n      pos: up\r\n   SD:\r\n      pos: up\r\n"));
    // A fresh copy: its name, timers 0, the radio's release.
    assert!(m.contains("name: \"FROM POCKET\""));
    assert!(m.contains("semver: 2.12.5"));
    assert!(!m.contains("value: 120"));
    // The custom switches come from the selected model (model01: the 2.11 bit fields).
    let tmpl = latin1(&c.read("MODELS/model01.yml").unwrap().unwrap());
    assert_eq!(
        block(&m, "functionSwitchConfig"),
        block(&tmpl, "functionSwitchConfig")
    );
    assert_eq!(
        block(&m, "functionSwitchLedONColor"),
        block(&tmpl, "functionSwitchLedONColor")
    );
    assert_eq!(board::custom_switches(&d).len(), 6);
    assert!(
        plan.warnings.iter().any(|w| w.contains("custom switches")),
        "{:?}",
        plan.warnings
    );
    // Only the new model is written.
    let paths: Vec<&str> = plan.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["MODELS/model02.yml", ".metadata_never_index"]);

    // A named template: model00, the 2.12 block.
    let mut e = import(&pocket_model(), "pocket", vec![]);
    if let Edit::ModelImport { template, .. } = &mut e {
        *template = Some("model00.yml".into());
    }
    let plan = c.plan(&[e], None).unwrap();
    let m = after(&plan, "MODELS/model02.yml");
    assert!(m.contains("customSwitches: \r\n   SW1:\r\n"), "{m}");
    assert!(m.contains("cfsGroupOn: \r\n"));
    assert!(!m.contains("functionSwitchConfig"));
}

#[test]
fn a_link_the_owner_sets_wins_and_mismatches_are_flagged() {
    let dir = tempfile::tempdir().unwrap();
    let c = synth_card(dir.path());
    // SE on SW1 (a latching button), SC (3-pos) on SA (2-pos): both flagged.
    let links = vec![
        ControlLink {
            from: "SE".into(),
            to: "SW1".into(),
        },
        ControlLink {
            from: "SC".into(),
            to: "SA".into(),
        },
    ];
    let plan = c
        .plan(&[import(&pocket_model(), "pocket", links)], None)
        .unwrap();
    assert!(plan.ready(), "{:?}", plan.checks);
    let m = after(&plan, "MODELS/model02.yml");
    assert!(m.contains("srcRaw: \"SW1\""));
    assert!(m.contains("def: \"SW12,SA1\""), "{m}");
    let w = plan.warnings.join("\n");
    assert!(w.contains("SE to SW1: SE springs back"), "{w}");
    assert!(w.contains("SC to SA: SC has a middle position"), "{w}");
    // A link to a control the GX12 lacks refuses, naming it.
    let bad = vec![ControlLink {
        from: "SE".into(),
        to: "SZ".into(),
    }];
    let plan = c
        .plan(&[import(&pocket_model(), "pocket", bad)], None)
        .unwrap();
    let r = plan.checks.last().unwrap();
    assert_eq!(r.name, "Controls fit the radio");
    let r = r.refusal.as_ref().unwrap();
    assert_eq!(r.code, RefusalCode::Incompatible);
    assert!(r.reason.contains("SE"), "{}", r.reason);
    assert!(plan.files.is_empty());
}

#[test]
fn a_gx12_model_copied_to_a_pocket_lists_what_it_cannot_map() {
    let gx = tempfile::tempdir().unwrap();
    let g = synth_card(gx.path());
    let plan = g
        .plan(
            &[Edit::Model {
                file: "model00.yml".into(),
                name: Some("ALPHA".into()),
                ops: every_model_op(),
            }],
            None,
        )
        .unwrap();
    let text = after(&plan, "MODELS/model00.yml");
    let src = Doc::parse("m", text.as_bytes()).unwrap();
    let from = board::inventory(Some("gx12"), None, Some(&src));
    let to = board::spec(Some("pocket")).unwrap().controls();
    let links = board::default_links("gx12", &from, "pocket", &to);
    let t = board::plan_translation(&src, &from, &to, &links).unwrap();
    assert_eq!(t.unmapped, ["SF", "SW1"], "{:?}", t.used);

    let pocket = tempfile::tempdir().unwrap();
    synth::write_card(pocket.path(), &SynthCard::default()).unwrap();
    let p = Card::open(pocket.path()).unwrap();
    let plan = p.plan(&[import(&text, "gx12", vec![])], None).unwrap();
    let r = plan.checks.last().unwrap().refusal.clone().unwrap();
    assert_eq!(r.code, RefusalCode::Incompatible);
    assert!(r.reason.contains("SF, SW1"), "{}", r.reason);
    // Mapped by hand, it goes, and the GX12-only keys stay behind.
    let links = vec![
        ControlLink {
            from: "SF".into(),
            to: "SC".into(),
        },
        ControlLink {
            from: "SW1".into(),
            to: "SD".into(),
        },
    ];
    let plan = p.plan(&[import(&text, "gx12", links)], None).unwrap();
    assert!(plan.ready(), "{:?}", plan.checks);
    let m = after(&plan, "MODELS/model02.yml");
    for k in ["customSwitches", "cfsGroupOn", "functionSwitch"] {
        assert!(!m.contains(k), "{k} left in a Pocket model");
    }
    assert!(m.contains("swtch: \"SD2\""), "SW12 became SD2");
    assert!(m.contains("srcRaw: \"SE\""), "SH became the Pocket's SE");
    assert!(m.contains("semver: 2.12.4"));
}

#[test]
fn an_unknown_board_refuses_an_import() {
    let dir = tempfile::tempdir().unwrap();
    let c = synth_card(dir.path());
    let plan = c
        .plan(&[import(&pocket_model(), "tx16s", vec![])], None)
        .unwrap();
    let r = plan.checks.last().unwrap().refusal.clone().unwrap();
    assert_eq!(r.code, RefusalCode::Incompatible);
    assert!(r.reason.contains("tx16s"));
}

#[test]
fn the_switch_map_reads_gx12_switches_and_custom_switches() {
    let dir = tempfile::tempdir().unwrap();
    let c = synth_card(dir.path());
    let plan = c
        .plan(
            &[Edit::Model {
                file: "model00.yml".into(),
                name: Some("ALPHA".into()),
                ops: every_model_op(),
            }],
            None,
        )
        .unwrap();
    let text = after(&plan, "MODELS/model00.yml");
    let d = Doc::parse("model00.yml", text.as_bytes()).unwrap();
    let radio = Doc::parse("radio.yml", &c.read("RADIO/radio.yml").unwrap().unwrap()).unwrap();
    let inputs = switchmap::Inputs {
        model: Some(em::view("model00.yml", &d).unwrap()),
        expos: switchmap::expos(&d),
        limits: switchmap::limits(&d),
        switches: switchmap::switch_types(&radio),
        ..Default::default()
    };
    let map = switchmap::build(&inputs);
    let row = |id: &str| map.rows.iter().find(|r| r.id == id);
    assert_eq!(
        row("SH").unwrap().switch_type.as_deref(),
        Some("TOGGLE"),
        "{:?}",
        map.rows.iter().map(|r| &r.id).collect::<Vec<_>>()
    );
    assert_eq!(row("SF").unwrap().positions.len(), 3);
    let sw1 = row("SW1").expect("a custom switch the model uses has a row");
    assert_eq!(sw1.switch_type.as_deref(), Some("2POS"));
    assert_eq!(sw1.positions.len(), 2);
    // An unused custom switch has no row.
    assert!(row("SW2").is_none());
}

#[test]
fn the_preview_estimates_a_gx12_write_at_its_usb_speed() {
    let o = WriteOptions::for_card(false, Some("gx12"));
    assert_eq!(o.bytes_per_s, 360_000);
    // 360 KB in 1 s, plus each file's own cost.
    assert_eq!(o.eta_s(360_000, 0), 1);
    assert_eq!(o.eta_s(3_600_000, 4), 12);
    // A Pocket card in a reader is fast; over its USB, 0.3 MB/s.
    assert_eq!(
        WriteOptions::for_card(false, Some("pocket")).bytes_per_s,
        10_000_000
    );
    assert_eq!(
        WriteOptions::for_card(true, Some("pocket")).bytes_per_s,
        300_000
    );

    let dir = tempfile::tempdir().unwrap();
    synth::write_card(dir.path(), &SynthCard::gx12()).unwrap();
    let core = core(dir.path());
    let p = core
        .gear_card_preview(&CardPreviewParams {
            mount: Some(dir.path().to_path_buf()),
            device: None,
            edits: vec![Edit::Model {
                file: "model00.yml".into(),
                name: Some("ALPHA".into()),
                ops: vec![ModelOp::SetChecklist { enabled: true }],
            }],
        })
        .unwrap();
    assert!(p.ready, "{:?}", p.checks);
    assert!(p.radio_usb, "a GX12 is always over its USB");
    assert_eq!(p.files, ["MODELS/model00.yml", ".metadata_never_index"]);
    assert_eq!(
        p.eta_s,
        WriteOptions::for_card(true, Some("gx12")).eta_s(p.bytes, 2)
    );
    assert!(p.eta_s >= 1);
}

fn core(dir: &Path) -> Core {
    let env = Env::fake(vec![], Arc::new(FakePorts::new(vec![])));
    Core::new(
        dir.join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    )
    .with_settings(dir.join("support/settings.json"))
    .with_gear_env(env)
}

#[test]
fn model_import_through_core_and_mcp() {
    use quadcam_lib::mcp::{LocalBackend, Server};
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let gx = dir.path().join("GX12");
    let pocket = dir.path().join("POCKET");
    synth::write_card(&gx, &SynthCard::gx12()).unwrap();
    synth::write_card(&pocket, &SynthCard::default()).unwrap();
    std::fs::write(pocket.join("MODELS/model01.yml"), pocket_model()).unwrap();
    let core = core(dir.path());
    let r = core
        .gear_model_import(&ModelImportParams {
            mount: Some(gx.clone()),
            from: pocket.display().to_string(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(r.from_board, "pocket");
    assert_eq!(r.to_board.as_deref(), Some("gx12"));
    assert!(r.translation.unmapped.is_empty());
    let se = r.translation.used.iter().find(|u| u.from == "SE").unwrap();
    assert_eq!(se.to.as_deref(), Some("SH"));
    assert!(r.preview.ready, "{:?}", r.preview.checks);
    assert!(r.preview.radio_usb);
    // The first free slot, the source's name, the selected model of the source.
    let Edit::ModelImport { to, name, .. } = &r.edit else {
        panic!("{:?}", r.edit)
    };
    assert_eq!(to, "model02.yml");
    assert_eq!(name, "BRAVO 2");
    assert!(
        !gx.join("MODELS/model02.yml").exists(),
        "nothing is written"
    );
    // One model file of a card copy finds its board in radio.yml two folders up.
    let r = core
        .gear_model_import(&ModelImportParams {
            mount: Some(gx.clone()),
            from: pocket.join("MODELS/model00.yml").display().to_string(),
            to: Some("model09.yml".into()),
            links: vec![ControlLink {
                from: "SB".into(),
                to: "SF".into(),
            }],
            ..Default::default()
        })
        .unwrap();
    assert!(r.links.iter().any(|l| l.from == "SB" && l.to == "SF"));
    assert!(r.preview.files.contains(&"MODELS/model09.yml".to_string()));
    // Not a card, a file or a backup.
    assert!(core
        .gear_model_import(&ModelImportParams {
            mount: Some(gx.clone()),
            from: "nowhere/at/all".into(),
            ..Default::default()
        })
        .is_err());

    let mut s = Server::new(LocalBackend(Arc::new(core)));
    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "model_import", "mount": gx.display().to_string(),
               "from": pocket.display().to_string(), "links": ["SE=SG"]}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("SE -> SG"), "{text}");
    assert!(text.contains("ok Known version"), "{text}");
    assert!(text.contains("+   srcRaw: \"SG\""), "{text}");
    let edit = r["structuredContent"]["edit"].clone();
    assert_eq!(edit["kind"], "model_import", "{r}");
    // The card read names the controls and the voice folder.
    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "card", "mount": gx.display().to_string()}),
    );
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("EdgeTX 2.12.5 on gx12"), "{text}");
    assert!(text.contains("SH TOGGLE"), "{text}");
    assert!(text.contains("SW1 2POS"), "{text}");
    assert!(text.contains("P1 knob (detent)"), "{text}");
    assert!(text.contains("Voice folder: SOUNDS/en."), "{text}");
}

fn control_type(control: &str, value: &str) -> Edit {
    Edit::Radio {
        ops: vec![RadioOp::SetControlType {
            control: control.into(),
            value: value.into(),
        }],
    }
}

#[test]
fn a_swapped_switch_module_or_a_blanked_pot_is_a_radio_edit() {
    // The factory file writes types in lower case; the edit keeps that case.
    let dir = tempfile::tempdir().unwrap();
    let c = factory_card(dir.path(), "2.12.5");
    let plan = c
        .plan(
            &[control_type("SB", "2POS"), control_type("P2", "none")],
            None,
        )
        .unwrap();
    assert!(plan.ready(), "{:?}", plan.checks);
    let radio = after(&plan, "RADIO/radio.yml");
    assert!(radio.contains("   SB:\r\n      type: 2pos\r\n"), "{radio}");
    assert!(radio.contains("   P2:\r\n      type: none\r\n"));
    assert!(radio.starts_with("checksum: 0\r\n"));
    // The radio's own fit is the inventory: SB 2-position, P2 gone.
    std::fs::write(dir.path().join("RADIO/radio.yml"), radio).unwrap();
    let v = Card::open(dir.path()).unwrap().view(None, today()).unwrap();
    let sb = v.controls.iter().find(|c| c.name == "SB").unwrap();
    assert_eq!(sb.switch_type.as_deref(), Some("2POS"));
    assert!(!v.controls.iter().any(|c| c.name == "P2"));

    // An upper-case file stays upper case.
    let dir = tempfile::tempdir().unwrap();
    let c = synth_card(dir.path());
    let plan = c.plan(&[control_type("SG", "2pos")], None).unwrap();
    assert!(after(&plan, "RADIO/radio.yml").contains("      type: 2POS\r\n"));
    // What a slot cannot take, a custom switch and a slot the file lacks refuse.
    for (ctl, val) in [
        ("SL1", "with_detent"),
        ("P1", "2pos"),
        ("SA", "slider"),
        ("SW1", "toggle"),
        ("SZ", "2pos"),
    ] {
        let plan = c.plan(&[control_type(ctl, val)], None).unwrap();
        assert!(!plan.ready(), "{ctl}={val}");
        assert!(plan.files.is_empty());
    }
}

#[test]
fn a_copy_maps_against_the_target_radios_current_fit() {
    // The GX12's SH module swapped for a 2-position switch: the Pocket's momentary SE has
    // no default home and the copy refuses until it is mapped.
    let dir = tempfile::tempdir().unwrap();
    let c = synth_card(dir.path());
    let plan = c.plan(&[control_type("SH", "2pos")], None).unwrap();
    std::fs::write(
        dir.path().join("RADIO/radio.yml"),
        after(&plan, "RADIO/radio.yml"),
    )
    .unwrap();
    let c = Card::open(dir.path()).unwrap();
    let plan = c
        .plan(&[import(&pocket_model(), "pocket", vec![])], None)
        .unwrap();
    let r = plan.checks.last().unwrap().refusal.clone().unwrap();
    assert_eq!(r.code, RefusalCode::Incompatible);
    assert!(r.reason.contains("SE"), "{}", r.reason);
    // Mapped onto SG (still momentary) it goes; onto SH (now 2-position) it warns.
    let to = |t: &str| {
        vec![ControlLink {
            from: "SE".into(),
            to: t.into(),
        }]
    };
    let plan = c
        .plan(&[import(&pocket_model(), "pocket", to("SG"))], None)
        .unwrap();
    assert!(
        plan.ready() && !plan.warnings.iter().any(|w| w.contains("SE to")),
        "{:?}",
        plan.warnings
    );
    let plan = c
        .plan(&[import(&pocket_model(), "pocket", to("SH"))], None)
        .unwrap();
    assert!(plan.ready());
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.contains("SE to SH: SE springs back")),
        "{:?}",
        plan.warnings
    );
    // A slot set to none is no target at all.
    let plan = c.plan(&[control_type("SG", "none")], None).unwrap();
    std::fs::write(
        dir.path().join("RADIO/radio.yml"),
        after(&plan, "RADIO/radio.yml"),
    )
    .unwrap();
    let c = Card::open(dir.path()).unwrap();
    let plan = c
        .plan(&[import(&pocket_model(), "pocket", to("SG"))], None)
        .unwrap();
    let r = plan.checks.last().unwrap().refusal.clone().unwrap();
    assert_eq!(r.code, RefusalCode::Incompatible);
}
