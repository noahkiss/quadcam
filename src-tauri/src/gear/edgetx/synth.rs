//! The synthetic EdgeTX card: radio and model files in both layouts QuadCam meets, with
//! made-up names and values, plus `LOGS/`, `SOUNDS/` and `SCRIPTS/`. Tests and the mock
//! core use it; no fixture comes from a real card.
//!
//! - The 2.12 saved layout: `mixData` entries start with `destCh`, quote `srcRaw`, carry
//!   `delayPrec`/`speedPrec`; switch warnings as the `switchWarning:` list; no checksum.
//! - The 2.10 hand-edited layout: entries start with `weight`, `srcRaw` unquoted, the
//!   legacy `switchWarningState:` line, and a `checksum:` line.

use anyhow::Result;
use std::path::Path;

/// Which layout a model file is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// As EdgeTX 2.12 saves it.
    Saved212,
    /// A 2.10-era hand-edited file.
    Hand210,
}

/// What the synthetic card holds.
#[derive(Debug, Clone)]
pub struct SynthCard {
    pub board: String,
    pub semver: String,
    /// CRLF line endings (EdgeTX writes CRLF).
    pub crlf: bool,
    /// `(file, header name, layout)` for each model.
    pub models: Vec<(String, String, Layout)>,
    /// `currModel`.
    pub curr_model: u32,
    /// Log file names in `LOGS/`.
    pub logs: Vec<String>,
}

impl Default for SynthCard {
    fn default() -> Self {
        Self {
            board: "pocket".into(),
            semver: "2.12.4".into(),
            crlf: true,
            models: vec![
                ("model00.yml".into(), "ALPHA".into(), Layout::Saved212),
                ("model01.yml".into(), "BRAVO 2".into(), Layout::Hand210),
            ],
            curr_model: 1,
            logs: vec![
                "ALPHA-2026-05-01.csv".into(),
                "BRAVO 2-2026-05-02.csv".into(),
            ],
        }
    }
}

fn eol(text: String, crlf: bool) -> String {
    if crlf {
        text.replace('\n', "\r\n")
    } else {
        text
    }
}

/// `RADIO/radio.yml`.
pub fn radio_yml(board: &str, semver: &str, curr_model: u32, crlf: bool) -> String {
    let mut s = format!(
        "checksum: 12345\nmanuallyEdited: 0\ntimezoneMinutes: 0\nsemver: {semver}\nboard: {board}\ncalib: \n"
    );
    for (k, mid) in [("LH", 1000), ("LV", 1010), ("RV", 1020), ("RH", 1030)] {
        s += &format!("   {k}:\n      mid: {mid}\n      spanNeg: 700\n      spanPos: 700\n");
    }
    s += &format!("currModel: {curr_model}\ncontrast: 20\nvBatWarn: 66\n");
    s += "trainer: \n   mix: \n";
    for i in 0..4 {
        s += &format!(
            "      {i}:\n         srcChn: {i}\n         mode: REPL\n         studWeight: 100\n"
        );
    }
    s += "view: 0\nbeepMode: mode_all\nhapticMode: mode_all\ninactivityTimer: 10\n";
    s += "ttsLanguage: \"en\"\nuiLanguage: \"en\"\n";
    s += "customFn: \n   0:\n      swtch: \"ON\"\n      func: VOLUME\n      def: \"P1,1\"\n";
    s += "serialPort: \n   VCP:\n      mode: OFF\n      power: 0\n";
    s += "switchConfig: \n";
    for (sw, t) in [
        ("SA", "2POS"),
        ("SB", "3POS"),
        ("SC", "3POS"),
        ("SD", "2POS"),
        ("SE", "TOGGLE"),
    ] {
        s += &format!("   {sw}:\n      name: \"\"\n      type: {t}\n");
    }
    s += "bluetoothName: \"TEST-\\x80\"\nownerRegistrationID: \"    test\"\nmodelQuickSelect: 1\n";
    eol(s, crlf)
}

fn mix(layout: Layout, ch: u32, src: &str, weight: i32, sw: &str, mx: &str) -> String {
    match layout {
        Layout::Saved212 => format!(
            " -\n   destCh: {ch}\n   srcRaw: \"{src}\"\n   carryTrim: 0\n   mixWarn: 0\n   mltpx: {mx}\n   delayPrec: 0\n   speedPrec: 0\n   flightModes: 000000000\n   weight: {weight}\n   offset: 0\n   swtch: \"{sw}\"\n   delayUp: 0\n   delayDown: 0\n   speedUp: 0\n   speedDown: 0\n   name: \"\"\n"
        ),
        Layout::Hand210 => format!(
            " -\n   weight: {weight}\n   destCh: {ch}\n   srcRaw: {src}\n   carryTrim: 0\n   mixWarn: 0\n   mltpx: {mx}\n   offset: 0\n   swtch: \"{sw}\"\n   flightModes: 000000000\n   delayUp: 0\n   delayDown: 0\n   speedUp: 0\n   speedDown: 0\n   name: \"\"\n"
        ),
    }
}

fn ls(i: u32, func: &str, def: &str, delay: u32) -> String {
    format!(
        "   {i}:\n      func: {func}\n      def: \"{def}\"\n      andsw: \"NONE\"\n      lsPersist: 0\n      lsState: 0\n      delay: {delay}\n      duration: 0\n"
    )
}

fn sensor(slot: u32, label: &str, id: u32) -> String {
    format!(
        "   {slot}:\n      id1: \n         id: {id}\n      id2: \n         instance: 0\n      label: \"{label}\"\n      subId: 0\n      type: TYPE_CUSTOM\n      unit: 1\n      prec: 1\n      autoOffset: 0\n      filter: 0\n      logs: 1\n      persistent: 0\n      onlyPositive: 0\n      cfg: \n         custom: \n            ratio: 0\n            offset: 0\n"
    )
}

fn timer(i: u32, sw: &str, value: u32, name: &str) -> String {
    format!(
        "   {i}:\n      start: 0\n      swtch: \"{sw}\"\n      value: {value}\n      mode: ON\n      countdownBeep: 0\n      minuteBeep: 1\n      persistent: 0\n      countdownStart: 0\n      showElapsed: 0\n      extraHaptic: 0\n      name: \"{name}\"\n"
    )
}

/// A model file with made-up content in `layout`.
pub fn model_yml(name: &str, layout: Layout, semver: &str, crlf: bool) -> String {
    let mut s = String::new();
    if layout == Layout::Hand210 {
        s += "checksum: 4242\n";
    }
    s += &format!("semver: {semver}\nheader: \n   name: \"{name}\"\n");
    s += "timers: \n";
    s += &timer(0, "L1", 120, "TOT");
    s += &timer(1, "L1", 0, "FLT");
    s += "telemetryProtocol: 0\nthrTrim: 0\ndisableThrottleWarning: 0\ndisplayChecklist: 0\n";
    s += "extendedLimits: 1\ndisableTelemetryWarning: 0\nchecklistInteractive: 0\nbeepANACenter: 0\n";
    s += "mixData: \n";
    for (ch, src) in [(0, "Ail"), (1, "Ele"), (2, "I2"), (3, "Rud"), (4, "SA")] {
        s += &mix(layout, ch, src, 100, "NONE", "ADD");
    }
    s += &mix(layout, 5, "SB", 100, "NONE", "ADD");
    s += "limitData: \n";
    for i in [0, 1] {
        s += &format!("   {i}:\n      min: 0\n      max: 0\n      ppmCenter: 0\n      offset: 0\n      symetrical: 0\n      revert: 0\n      curve: 32\n      name: \"\"\n");
    }
    s += "expoData: \n";
    for (i, src) in ["Rud", "Ele", "Thr", "Ail"].iter().enumerate() {
        s += &format!(" -\n   mode: 3\n   scale: 0\n   trimSource: 0\n   srcRaw: \"{src}\"\n   weight: 100\n   offset: 0\n   swtch: \"NONE\"\n   curve: \n      type: 1\n      value: 0\n   chn: {i}\n   flightModes: 000000000\n   name: \"\"\n");
    }
    s += "curves: \n   31:\n      type: 1\n      smooth: 0\n      points: -1\n      name: \"CV\"\npoints: \n   155:\n      val: -100\n   158:\n      val: 100\n";
    s += "logicalSw: \n";
    s += &ls(0, "FUNC_VPOS", "ch(4),0", 0);
    s += &ls(1, "FUNC_AND", "L4,L5", 0);
    s += &ls(3, "FUNC_VPOS", "tele(2),10", 0);
    s += &ls(4, "FUNC_VNEG", "tele(2),35", 20);
    s += "customFn: \n";
    s += "   0:\n      swtch: \"L1\"\n      func: PLAY_TRACK\n      def: \"armed,1,1x\"\n";
    s += "   1:\n      swtch: \"!L1\"\n      func: PLAY_TRACK\n      def: \"disarm,1,!1x\"\n";
    s += "   2:\n      swtch: \"L2\"\n      func: PLAY_VALUE\n      def: \"tele(2),1,10\"\n";
    s += "   3:\n      swtch: \"TrimThrDown\"\n      func: PLAY_VALUE\n      def: \"Tmr2,1,1x\"\n";
    s += "flightModeData: \n   0:\n      trim: \n";
    for i in 0..4 {
        s += &format!("         {i}:\n            value: 0\n            mode: 31\n");
    }
    s += "      name: \"\"\n      swtch: \"NONE\"\n      fadeIn: 0\n      fadeOut: 0\n      gvars: \n";
    for i in 0..3 {
        s += &format!("         {i}:\n            val: 0\n");
    }
    s += "thrTraceSrc: Thr\n";
    match layout {
        Layout::Saved212 => s += "switchWarning: \n   SA:\n      pos: up\n   SB:\n      pos: up\n",
        Layout::Hand210 => s += "switchWarningState: up,up,none,none,none\n",
    }
    s += "rssiSource: none\nrfAlarms: \n   warning: 45\n   critical: 42\nthrTrimSw: 0\npotsWarnMode: WARN_OFF\njitterFilter: GLOBAL\n";
    s += "moduleData: \n   0:\n      type: TYPE_CROSSFIRE\n      subType: 0\n      channelsStart: 0\n      channelsCount: 16\n      failsafeMode: NOT_SET\n      mod: \n         crsf: \n            telemetryBaudrate: 0\n";
    s += "trainerData: \n   mode: SLAVE\n   channelsStart: 0\n   channelsCount: 0\n";
    s += "inputNames: \n   0:\n      val: \"Rud\"\n   1:\n      val: \"Ele\"\n";
    s += "telemetrySensors: \n";
    s += &sensor(0, "1RSS", 20);
    s += &sensor(1, "RQly", 20);
    s += &sensor(2, "RxBt", 8);
    s += &sensor(3, "Capa", 8);
    s += "screens: \n   0:\n      type: VALUES\n      u: \n         lines: \n            0:\n               sources: \n                  0:\n                     val: tele(2)\n                  1:\n                     val: Tmr1\n";
    s += "view: 0\nmodelRegistrationID: \"TESTREG1\"\nusbJoystickIfMode: JOYSTICK\nmodelSFDisabled: GLOBAL\n";
    eol(s, crlf)
}

/// Writes the card tree under `root` (a temporary folder; never a mounted card).
pub fn write_card(root: &Path, card: &SynthCard) -> Result<()> {
    for d in ["RADIO", "MODELS", "LOGS", "SOUNDS/en", "SCRIPTS/TELEMETRY"] {
        std::fs::create_dir_all(root.join(d))?;
    }
    std::fs::write(
        root.join("RADIO/radio.yml"),
        radio_yml(&card.board, &card.semver, card.curr_model, card.crlf),
    )?;
    for (file, name, layout) in &card.models {
        std::fs::write(
            root.join("MODELS").join(file),
            model_yml(name, *layout, &card.semver, card.crlf),
        )?;
    }
    for l in &card.logs {
        std::fs::write(
            root.join("LOGS").join(l),
            "Date,Time,1RSS(dB),RQly(%),RxBt(V)\n2026-05-01,12:00:00.000,-60,100,4.1\n",
        )?;
    }
    std::fs::write(root.join("SOUNDS/en/hello.wav"), b"RIFF\0\0\0\0WAVE")?;
    std::fs::write(root.join("edgetx.sdcard.version"), &card.semver)?;
    Ok(())
}
