//! The model editors (design 6.3, WP9 part A): each editor op in both file layouts, each
//! against a synthetic card; ownership of callouts; idempotence; the value rules. Every
//! card here is synthetic.

use quadcam_lib::gear::edgetx::editors::{self, CalloutDef, CalloutWhen, LoggingDef, SensorLog};
use quadcam_lib::gear::edgetx::model::{self as em, Field, ModelOp};
use quadcam_lib::gear::edgetx::synth::{self, Layout};
use quadcam_lib::gear::edgetx::yaml::Doc;
use quadcam_lib::gear::model::Refusal;

const LAYOUTS: [Layout; 2] = [Layout::Saved212, Layout::Hand210];

fn doc(layout: Layout) -> Doc {
    Doc::parse(
        "model00.yml",
        synth::model_yml("ALPHA", layout, "2.12.4", false).as_bytes(),
    )
    .unwrap()
}

fn run(layout: Layout, ops: &[ModelOp]) -> Result<(String, String), Refusal> {
    let mut d = doc(layout);
    let before = String::from_utf8(d.render()).unwrap();
    em::apply(&mut d, ops)?;
    Ok((before, String::from_utf8(d.render()).unwrap()))
}

/// The lines that left and the lines that came, as `-`/`+` rows (a multiset difference).
fn changes(before: &str, after: &str) -> Vec<String> {
    let mut b: Vec<&str> = before.lines().collect();
    let mut out = Vec::new();
    for l in after.lines() {
        match b.iter().position(|x| *x == l) {
            Some(i) => {
                b.remove(i);
            }
            None => out.push(format!("+{l}")),
        }
    }
    let mut gone: Vec<String> = b.iter().map(|l| format!("-{l}")).collect();
    gone.extend(out);
    gone.sort();
    gone
}

fn sorted(mut v: Vec<&str>) -> Vec<String> {
    v.sort();
    v.into_iter().map(String::from).collect()
}

fn once(layout: Layout, op: ModelOp) -> Vec<String> {
    let (b, a) = run(layout, &[op]).unwrap();
    changes(&b, &a)
}

fn refuses(layout: Layout, op: ModelOp) -> String {
    run(layout, &[op]).unwrap_err().reason
}

fn idempotent(layout: Layout, ops: &[ModelOp]) {
    let (_, once) = run(layout, ops).unwrap();
    let mut d = Doc::parse("model00.yml", once.as_bytes()).unwrap();
    em::apply(&mut d, ops).unwrap();
    assert_eq!(String::from_utf8(d.render()).unwrap(), once, "{ops:?}");
}

fn view(layout: Layout, ops: &[ModelOp]) -> editors::EditorView {
    let (_, a) = run(layout, ops).unwrap();
    editors::editor_view(&Doc::parse("m.yml", a.as_bytes()).unwrap()).unwrap()
}

fn logging(sw: &str, p: u32) -> ModelOp {
    ModelOp::SetLogging {
        logging: Some(LoggingDef {
            swtch: sw.into(),
            period_ds: p,
        }),
    }
}

fn callout(track: &str, when: CalloutWhen, repeat: Option<&str>) -> ModelOp {
    ModelOp::SetCallout {
        callout: CalloutDef {
            track: track.into(),
            when,
            repeat: repeat.map(Into::into),
        },
    }
}

fn below(source: &str, value: &str) -> CalloutWhen {
    CalloutWhen::Below {
        source: source.into(),
        value: value.into(),
        delay_ds: 20,
    }
}

#[test]
fn logging_writes_one_logs_function_and_replaces_it_in_place() {
    for layout in LAYOUTS {
        let c = once(layout, logging("SA2", 10));
        assert_eq!(
            c,
            sorted(vec![
                "+   4:",
                "+      swtch: \"SA2\"",
                "+      func: LOGS",
                "+      def: \"10,1\"",
            ]),
            "{layout:?}"
        );
        // A second call moves the same function: no second LOGS, the same place.
        let v = view(layout, &[logging("SA2", 10), logging("ON", 5)]);
        assert_eq!(
            v.logging.logging,
            Some(LoggingDef {
                swtch: "ON".into(),
                period_ds: 5
            })
        );
        let (_, a) = run(layout, &[logging("SA2", 10), logging("ON", 5)]).unwrap();
        assert_eq!(a.matches("func: LOGS").count(), 1);
        idempotent(layout, &[logging("SA2", 10)]);
        // None removes every LOGS function and leaves the others.
        let (b, a) = run(
            layout,
            &[logging("SA2", 10), ModelOp::SetLogging { logging: None }],
        )
        .unwrap();
        assert_eq!(b, a);
    }
}

#[test]
fn logging_period_and_switch_rules() {
    for layout in LAYOUTS {
        assert!(refuses(layout, logging("SA2", 0)).contains("period"));
        assert!(refuses(layout, logging("SA2", 256)).contains("period"));
        assert!(refuses(layout, logging("", 10)).contains("switch"));
    }
}

#[test]
fn sensor_logs_flip_one_line_per_sensor() {
    for layout in LAYOUTS {
        let c = once(
            layout,
            ModelOp::SetSensorLogs {
                sensors: vec![SensorLog {
                    label: "RxBt".into(),
                    logs: false,
                }],
            },
        );
        assert_eq!(
            c,
            sorted(vec!["-      logs: 1", "+      logs: 0"]),
            "{layout:?}"
        );
        let v = view(
            layout,
            &[ModelOp::SetSensorLogs {
                sensors: vec![SensorLog {
                    label: "RxBt".into(),
                    logs: false,
                }],
            }],
        );
        let rx = v
            .logging
            .sensors
            .iter()
            .find(|s| s.label == "RxBt")
            .unwrap();
        assert!(!rx.logs);
        assert!(v.logging.sensors.iter().filter(|s| s.logs).count() == 3);
        let missing = refuses(
            layout,
            ModelOp::SetSensorLogs {
                sensors: vec![SensorLog {
                    label: "Nope".into(),
                    logs: true,
                }],
            },
        );
        assert!(missing.contains("discover sensors"), "{missing}");
    }
}

#[test]
fn rf_alarms_set_both_levels_in_range() {
    for layout in LAYOUTS {
        let c = once(
            layout,
            ModelOp::SetRfAlarms {
                warning: 50,
                critical: 40,
            },
        );
        assert_eq!(
            c,
            sorted(vec![
                "-   warning: 45",
                "-   critical: 42",
                "+   warning: 50",
                "+   critical: 40"
            ]),
            "{layout:?}"
        );
        idempotent(
            layout,
            &[ModelOp::SetRfAlarms {
                warning: 50,
                critical: 40,
            }],
        );
        let r = view(
            layout,
            &[ModelOp::SetRfAlarms {
                warning: 50,
                critical: 40,
            }],
        )
        .rf_alarms;
        assert_eq!(
            r,
            Some(editors::RfAlarms {
                warning: 50,
                critical: 40
            })
        );
        for (w, cr) in [(0, 0), (128, 40), (40, 50)] {
            assert!(!refuses(
                layout,
                ModelOp::SetRfAlarms {
                    warning: w,
                    critical: cr
                }
            )
            .is_empty());
        }
    }
}

#[test]
fn a_battery_callout_makes_a_switch_and_a_function_and_owns_them_by_track() {
    for layout in LAYOUTS {
        let op = callout("lowbat", below("{RxBt}", "3.5"), Some("5"));
        let c = once(layout, op.clone());
        // L3 (index 2) is the first free switch; the value is stored at precision 1.
        assert_eq!(
            c,
            sorted(vec![
                "+   2:",
                "+      func: FUNC_VNEG",
                "+      def: \"tele(2),35\"",
                "+      andsw: \"NONE\"",
                "+      lsPersist: 0",
                "+      lsState: 0",
                "+      delay: 20",
                "+      duration: 0",
                "+   4:",
                "+      swtch: \"L3\"",
                "+      func: PLAY_TRACK",
                "+      def: \"lowbat,1,5\"",
            ]),
            "{layout:?}"
        );
        idempotent(layout, std::slice::from_ref(&op));
        // The same track again replaces both: the same switch, the new value.
        let (_, a) = run(
            layout,
            &[
                op.clone(),
                callout("lowbat", below("{RxBt}", "3.4"), Some("5")),
            ],
        )
        .unwrap();
        assert!(a.contains("def: \"tele(2),34\""));
        assert_eq!(
            a.matches("tele(2),35").count(),
            1,
            "only the fixture's own switch"
        );
        assert_eq!(a.matches("lowbat").count(), 1);
        // Read back.
        let v = view(layout, std::slice::from_ref(&op));
        let got = v.callouts.iter().find(|c| c.track == "lowbat").unwrap();
        assert_eq!(got.swtch, "L3");
        assert_eq!(got.repeat, "5");
        assert_eq!(
            got.when,
            Some(CalloutWhen::Below {
                source: "{RxBt}".into(),
                value: "3.5".into(),
                delay_ds: 20
            })
        );
        // Removing it takes the function and the switch it made.
        let (b0, a0) = run(
            layout,
            &[
                op,
                ModelOp::RemoveCallout {
                    track: "lowbat".into(),
                },
            ],
        )
        .unwrap();
        assert_eq!(a0, b0, "{layout:?}");
    }
}

#[test]
fn a_callout_keeps_a_switch_something_else_uses() {
    for layout in LAYOUTS {
        // `armed` plays on L1, and a timer uses L1: moving the callout must not empty it.
        let c = once(
            layout,
            callout(
                "armed",
                CalloutWhen::Switch {
                    swtch: "SA2".into(),
                },
                Some("!1x"),
            ),
        );
        assert_eq!(
            c,
            sorted(vec![
                "-      swtch: \"L1\"",
                "-      def: \"armed,1,1x\"",
                "+      swtch: \"SA2\"",
                "+      def: \"armed,1,!1x\"",
            ]),
            "{layout:?}"
        );
        let (_, a) = run(
            layout,
            &[
                callout(
                    "armed",
                    CalloutWhen::Switch {
                        swtch: "SA2".into(),
                    },
                    None,
                ),
                ModelOp::RemoveCallout {
                    track: "armed".into(),
                },
            ],
        )
        .unwrap();
        assert!(!a.contains("armed"));
        assert!(a.contains("func: FUNC_VPOS"), "L1 survives: {layout:?}");
    }
}

#[test]
fn callout_rules() {
    for layout in LAYOUTS {
        let b = |t: &str, w: CalloutWhen, r: Option<&str>| refuses(layout, callout(t, w, r));
        assert!(b("toolongname", below("{RxBt}", "3.5"), None).contains("8"));
        assert!(b("", below("{RxBt}", "3.5"), None).contains("track"));
        assert!(b("a b", below("{RxBt}", "3.5"), None).contains("track"));
        assert!(b("lowbat", below("{Nope}", "3.5"), None).contains("discover sensors"));
        assert!(b("lowbat", below("{RxBt}", "abc"), None).contains("not a number"));
        assert!(b("lowbat", below("{RxBt}", "3.5"), Some("often")).contains("Repeat"));
        assert!(b("lowbat", CalloutWhen::Switch { swtch: "".into() }, None).contains("switch"));
    }
}

#[test]
fn a_callout_takes_a_free_switch_or_refuses_when_all_64_are_used() {
    let layout = Layout::Saved212;
    // Fill L1..L64 with edge switches, then ask for one more.
    let mut ops: Vec<ModelOp> = (0..64)
        .filter(|i| ![0, 1, 3, 4].contains(i))
        .map(|i| ModelOp::SetLogicalSwitch {
            index: i,
            ls: Some(em::LsDef {
                func: "FUNC_VPOS".into(),
                def: "ch(1),0".into(),
                andsw: "NONE".into(),
                delay: 0,
                duration: 0,
            }),
        })
        .collect();
    ops.push(callout("lowbat", below("{RxBt}", "3.5"), None));
    let e = run(layout, &ops).unwrap_err();
    assert!(e.reason.contains("64 logical switches"), "{}", e.reason);
}

#[test]
fn value_screens_resolve_labels_and_keep_the_order() {
    for layout in LAYOUTS {
        let ops = vec![ModelOp::SetScreenValues {
            index: 1,
            lines: vec![
                vec!["{RxBt}".into(), "Tmr1".into()],
                vec![],
                vec!["{Capa}".into()],
            ],
        }];
        let c = once(layout, ops[0].clone());
        assert_eq!(
            c,
            sorted(vec![
                "+   1:",
                "+      type: VALUES",
                "+      u: ",
                "+         lines: ",
                "+            0:",
                "+               sources: ",
                "+                  0:",
                "+                     val: tele(2)",
                "+                  1:",
                "+                     val: Tmr1",
                "+            2:",
                "+               sources: ",
                "+                  0:",
                "+                     val: tele(3)",
            ]),
            "{layout:?}"
        );
        idempotent(layout, &ops);
        let v = view(layout, &ops);
        let s = v.screens.iter().find(|s| s.index == 1).unwrap();
        assert_eq!(s.kind, "VALUES");
        assert_eq!(s.labels[0], vec!["{RxBt}", "Tmr1"]);
        assert_eq!(s.labels[2], vec!["{Capa}"]);
        // A value screen replaces the script screen it follows.
        let (_, a) = run(
            layout,
            &[
                ModelOp::SetScreen {
                    index: 1,
                    script: Some("volt".into()),
                },
                ops[0].clone(),
            ],
        )
        .unwrap();
        assert!(!a.contains("volt"));
    }
}

#[test]
fn value_screen_rules() {
    for layout in LAYOUTS {
        let r = |index, lines: Vec<Vec<&str>>| {
            refuses(
                layout,
                ModelOp::SetScreenValues {
                    index,
                    lines: lines
                        .into_iter()
                        .map(|l| l.into_iter().map(String::from).collect())
                        .collect(),
                },
            )
        };
        assert!(r(4, vec![vec!["Tmr1"]]).contains("1-4"));
        assert!(r(2, vec![vec!["Tmr1"]]).contains("missing"));
        assert!(r(0, vec![vec![]]).contains("at least one source"));
        assert!(r(0, vec![vec!["a"]; 5]).contains("4 lines"));
        assert!(r(0, vec![vec!["a", "b", "c", "d"]]).contains("3 sources"));
        assert!(r(0, vec![vec!["{Nope}"]]).contains("discover sensors"));
        assert!(r(0, vec![vec!["bad name"]]).contains("not a source"));
    }
}

#[test]
fn timer_fields_are_checked_and_set() {
    for layout in LAYOUTS {
        let set = |k: &str, v: &str| ModelOp::SetTimer {
            index: 1,
            fields: vec![Field::new(k, v)],
        };
        let c = once(layout, set("countdownBeep", "2"));
        assert_eq!(
            c,
            sorted(vec!["-      countdownBeep: 0", "+      countdownBeep: 2"])
        );
        for (k, v) in [
            ("countdownBeep", "4"),
            ("minuteBeep", "2"),
            ("persistent", "3"),
            ("showElapsed", "x"),
            ("countdownStart", "9"),
            ("mode", "on"),
        ] {
            assert!(refuses(layout, set(k, v)).contains(k), "{k}={v} {layout:?}");
        }
        idempotent(layout, &[set("persistent", "1")]);
    }
}

#[test]
fn checklist_rules() {
    assert!(editors::check_checklist("=Props tight\nBattery strapped", Some(20)).is_ok());
    assert!(editors::check_checklist("=123456789012345678901", Some(20))
        .unwrap_err()
        .reason
        .contains("20 characters"));
    assert!(editors::check_checklist("=123456789012345678901", None).is_ok());
    let many = "x\n".repeat(100);
    assert!(editors::check_checklist(&many, None)
        .unwrap_err()
        .reason
        .contains("99 lines"));
    assert!(editors::check_checklist("snow \u{2603}", None)
        .unwrap_err()
        .reason
        .contains("Latin-1"));
}
