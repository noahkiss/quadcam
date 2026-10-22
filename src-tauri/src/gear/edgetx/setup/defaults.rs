//! The starting setup: the jobs of `docs/radio-design.md` 5.1 with the channels Betaflight
//! reads, the default assignment per board, and a template that does the radio side of
//! each job (the arm latch, the turtle gate, the finder screen, callouts, logging). The
//! person changes all of it; their setup is saved in the gear folder, never here.

use super::{
    Aircraft, Assignment, RadioControlKind, FuncPrefix, Job, LsSpec, LuaKind, LuaScreen, MixSpec,
    ModelTemplate, Owns, RadioPart, RadioSetup, ScreenSpec, SfSpec, SwitchFunc, TimerSpec,
};
use crate::gear::edgetx::model::{Field, LsDef, MixLine, SfDef};

fn job(id: &str, label: &str, channel: Option<u32>, fits: &[RadioControlKind], states: &[&str]) -> Job {
    Job {
        id: id.into(),
        label: label.into(),
        channel,
        fits: fits.to_vec(),
        states: states.iter().map(|s| s.to_string()).collect(),
    }
}

/// The jobs of design 5.1. AUXn is CH(n+4); ELRS Wide mode sends CH5 and CH6-CH12.
pub fn jobs() -> Vec<Job> {
    use RadioControlKind::*;
    let mut v = vec![
        job(
            "arm",
            "Arm",
            Some(4),
            &[TwoPos],
            &["disarmed", "armed"],
        ),
        job(
            "osd",
            "OSD layout",
            Some(5),
            &[ThreePos],
            &["OSD profile 1", "OSD profile 2", "OSD profile 3"],
        ),
        job(
            "rates",
            "Rates",
            Some(6),
            &[ThreePos],
            &["rate profile 1", "rate profile 2", "rate profile 3"],
        ),
        job(
            "finder",
            "Finder",
            Some(7),
            &[TwoPos, Custom],
            &["off", "beeper on"],
        ),
        job(
            "turtle",
            "Turtle",
            Some(8),
            &[Momentary],
            &["off", "turtle while held"],
        ),
        job(
            "blackbox",
            "Blackbox erase",
            Some(9),
            &[Trim, Momentary],
            &["off", "erase while pressed"],
        ),
        job(
            "vtx",
            "VTX power",
            Some(10),
            &[Pot, Slider, ThreePos],
            &["25 mW", "200 mW", "400 mW"],
        ),
    ];
    for i in 1..=4 {
        v.push(job(
            &format!("sim{i}"),
            &format!("Sim button {i}"),
            Some(10 + i),
            &[Trim, Momentary],
            &["released", "pressed"],
        ));
    }
    v
}

fn a(job: &str, control: &str, high: Option<u8>) -> Assignment {
    Assignment {
        job: job.into(),
        control: control.into(),
        high,
    }
}

/// The default assignment of a board; None for a board without a control list.
///
/// On the Pocket, position 2 of the top-edge toggles SB and SC is the end the owner calls
/// up, and up is profile 1 (1000 µs), so position 0 sends 2000 µs. The GX12's front toggles
/// are taken to read the other way (up is position 0), which stage 5 checks on the radio.
pub fn assignments(board: &str) -> Option<Vec<Assignment>> {
    let (osd_rates_high, turtle) = match board.trim().to_ascii_lowercase().as_str() {
        "pocket" => (0, "SE"),
        "gx12" => (2, "SH"),
        _ => return None,
    };
    Some(vec![
        a("arm", "SA", Some(2)),
        a("osd", "SB", Some(osd_rates_high)),
        a("rates", "SC", Some(osd_rates_high)),
        a("finder", "SD", Some(2)),
        a("turtle", turtle, Some(2)),
        a("blackbox", "TrimRudRight", None),
        a("vtx", "P1", None),
        a("sim1", "TrimAilRight", None),
        a("sim2", "TrimEleUp", None),
        a("sim3", "TrimEleDown", None),
        a("sim4", "TrimAilLeft", None),
    ])
}

pub(super) fn mix(
    channel: u32,
    source: &str,
    weight: i32,
    swtch: &str,
    mltpx: &str,
    job: Option<&str>,
) -> MixSpec {
    MixSpec {
        channel,
        line: MixLine {
            source: source.into(),
            weight,
            swtch: swtch.into(),
            mltpx: mltpx.into(),
        },
        job: job.map(str::to_string),
    }
}

pub(super) fn ls(index: u32, func: &str, def: &str, delay: u32, job: Option<&str>) -> LsSpec {
    LsSpec {
        index,
        ls: Some(LsDef {
            func: func.into(),
            def: def.into(),
            andsw: "NONE".into(),
            delay,
            duration: 0,
        }),
        job: job.map(str::to_string),
    }
}

pub(super) fn sf(swtch: &str, func: &str, def: &str, job: Option<&str>) -> SfSpec {
    SfSpec {
        sf: SfDef {
            swtch: swtch.into(),
            func: func.into(),
            def: def.into(),
        },
        job: job.map(str::to_string),
    }
}

/// Timer fields as EdgeTX 2.12 writes them, without the stored value.
pub(super) fn timer(index: u32, swtch: &str, persistent: u32, name: &str) -> TimerSpec {
    let f = |k: &str, v: &str| Field::new(k, v);
    TimerSpec {
        index,
        fields: vec![
            f("start", "0"),
            f("swtch", swtch),
            f("mode", "ON"),
            f("countdownBeep", "0"),
            f("minuteBeep", "0"),
            f("persistent", &persistent.to_string()),
            f("countdownStart", "0"),
            f("showElapsed", "0"),
            f("extraHaptic", "0"),
            f("name", name),
        ],
    }
}

/// The template: the radio side of each job. Logical switches, by index:
///
/// | LS | Does |
/// |---|---|
/// | L1 | flying: set when armed and linked (L22), cleared when the arm switch is released |
/// | L2, L3 | low and critical battery, only with a real reading (L4) |
/// | L4-L6 | the quad's battery (`RxBt`) above 1.0 V, under 3.5 V and under 3.3 V for 2 s |
/// | L7, L8 | the finder control turns on, turns off (one-tick edges for the screen) |
/// | L10 | the ELRS link is up (`RQly` > 0) |
/// | L11, L12 | the blackbox-erase trim and the voltage read-out trim, only with a link |
/// | L13 | armed (the arm channel above 0) |
/// | L14-L18 | the turtle gate: held with the throttle at the bottom for 0.2 s, the arm channel low, high, then low again on release |
/// | L22 | armed and linked |
pub fn template() -> ModelTemplate {
    let arm = Some("arm");
    let turtle = Some("turtle");
    let finder = Some("finder");
    let rates = Some("rates");
    let mut mixes = vec![
        mix(2, "I2", 100, "NONE", "ADD", None),
        mix(2, "MAX", -100, "L16", "REPL", turtle),
        mix(4, "@arm", 100, "NONE", "ADD", arm),
        mix(4, "MAX", -100, "L16", "REPL", turtle),
        mix(4, "MAX", 100, "L17", "REPL", turtle),
        mix(4, "MAX", -100, "L18", "REPL", turtle),
        mix(5, "@osd", 100, "NONE", "ADD", Some("osd")),
        mix(6, "@rates", 100, "NONE", "ADD", rates),
        mix(7, "@finder", 100, "NONE", "ADD", finder),
        mix(8, "@turtle", 100, "NONE", "ADD", turtle),
        mix(9, "MAX", 100, "@blackbox", "ADD", Some("blackbox")),
        mix(10, "@vtx", 100, "NONE", "ADD", Some("vtx")),
    ];
    for i in 1..=4u32 {
        let j = format!("sim{i}");
        mixes.push(mix(10 + i, "MAX", 100, &format!("@{j}"), "ADD", Some(&j)));
    }
    mixes.push(mix(15, "MAX", 100, "NONE", "ADD", None));
    let mut lss = vec![
        ls(0, "FUNC_STICKY", "L22,!L13", 0, arm),
        ls(1, "FUNC_AND", "L4,L5", 0, None),
        ls(2, "FUNC_AND", "L4,L6", 0, None),
        ls(3, "FUNC_VPOS", "tele({RxBt}),10", 0, None),
        ls(4, "FUNC_VNEG", "tele({RxBt}),35", 20, None),
        ls(5, "FUNC_VNEG", "tele({RxBt}),33", 20, None),
        ls(6, "FUNC_EDGE", "@finder.hi,0,<", 0, finder),
        ls(7, "FUNC_EDGE", "@finder.lo,0,<", 0, finder),
        ls(9, "FUNC_VPOS", "tele({RQly}),0", 0, None),
        ls(10, "FUNC_AND", "L10,@blackbox", 0, Some("blackbox")),
        ls(11, "FUNC_AND", "L10,TrimThrUp", 0, None),
        ls(12, "FUNC_VPOS", "ch(@arm.ch),0", 0, arm),
        ls(13, "FUNC_AND", "@turtle.hi,L15", 2, turtle),
        ls(14, "FUNC_VNEG", "Thr,-90", 0, turtle),
        ls(15, "FUNC_STICKY", "L14,!@turtle.hi", 0, turtle),
        ls(16, "FUNC_AND", "L16,ON", 1, turtle),
        ls(17, "FUNC_EDGE", "L16,0,-", 0, turtle),
        ls(21, "FUNC_AND", "L13,L10", 0, arm),
    ];
    if let Some(l) = lss.iter_mut().find(|l| l.index == 17) {
        if let Some(d) = l.ls.as_mut() {
            d.duration = 2;
        }
    }
    ModelTemplate {
        timers: vec![timer(0, "L1", 2, "TOT")],
        checklist_on: Some(false),
        flags: vec![],
        mixes,
        logical_switches: lss,
        functions: vec![
            sf("@rates.lo", "PLAY_TRACK", "ratefree,1,!1x", rates),
            sf("@rates.mid", "PLAY_TRACK", "raterace,1,!1x", rates),
            sf("@rates.hi", "PLAY_TRACK", "ratecine,1,!1x", rates),
            sf("@finder.hi", "PLAY_TRACK", "finder,1,!1x", finder),
            sf("L7", "SET_SCREEN", "3,1,1x", finder),
            sf("L8", "SET_SCREEN", "0,1,1x", finder),
            sf("L16", "PLAY_TRACK", "turtle,1,1x", turtle),
            sf("L16", "HAPTIC", "1,1,1x", turtle),
            sf("L1", "LOGS", "1,1", None),
            sf("L2", "HAPTIC", "1,1,10", None),
            sf("L3", "HAPTIC", "2,1,5", None),
        ],
        keep_functions: vec![
            sf("L13", "PLAY_TRACK", "armed,1,1x", arm),
            sf("!L13", "PLAY_TRACK", "disarm,1,!1x", arm),
            sf("L13", "HAPTIC", "1,1,1x", arm),
            sf("L2", "PLAY_VALUE", "tele({RxBt}),1,10", None),
            sf("L3", "PLAY_TRACK", "lowbat,1,5", None),
            sf("L12", "PLAY_VALUE", "tele({RxBt}),1,1x", None),
            sf("L11", "PLAY_TRACK", "bbclear,1,1x", Some("blackbox")),
        ],
        owns: Owns {
            funcs: vec!["SET_SCREEN".into(), "LOGS".into()],
            tracks: vec![],
            switch_funcs: ["L2", "L3", "L16"]
                .iter()
                .map(|s| SwitchFunc {
                    swtch: s.to_string(),
                    func: "HAPTIC".into(),
                })
                .collect(),
            def_prefixes: Vec::<FuncPrefix>::new(),
        },
        switch_warnings: vec!["arm".into()],
        screens: vec![
            ScreenSpec::Values {
                index: 0,
                lines: vec![
                    vec!["{RQly}".into(), "{1RSS}".into()],
                    vec!["{RxBt}".into(), "Tmr1".into()],
                ],
            },
            ScreenSpec::Script {
                index: 1,
                script: "volt".into(),
            },
            ScreenSpec::Script {
                index: 2,
                script: "finder".into(),
            },
        ],
        lua: vec![
            LuaScreen {
                name: "volt".into(),
                kind: LuaKind::Volt { timer: 0 },
            },
            LuaScreen {
                name: "finder".into(),
                kind: LuaKind::Finder {
                    job: "finder".into(),
                },
            },
        ],
    }
}

/// The starting setup for a board and one model file; None for a board without a control
/// list.
pub fn setup(board: &str, model_file: &str) -> Option<RadioSetup> {
    Some(RadioSetup {
        board: board.trim().to_ascii_lowercase(),
        jobs: jobs(),
        assignments: assignments(board)?,
        template: template(),
        aircraft: vec![Aircraft {
            file: model_file.into(),
            name: None,
            was_named: vec![],
            model_id: None,
            copy_from: None,
            checklist: None,
            drop_timers: vec![],
            logical_switches: vec![],
            drop_functions: vec![],
            functions: vec![],
        }],
        radio: RadioPart {
            scalars: vec![Field::new("hapticMode", "mode_nokeys")],
            select_model: None,
            remove_functions: vec![],
            control_types: true,
            delete_other_models: false,
        },
    })
}
