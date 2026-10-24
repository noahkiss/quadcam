//! The Betaflight link through the core (WP2): identify over MSP, a CLI read, the write
//! engine's checks, the port released after every job with one cue, the USB heat timer,
//! board notes, and the MCP and CLI surfaces. Every FC is `FakeFc`; no real port opens.

use quadcam_lib::core::{Core, FcPortParams, FcReadParams, NoHooks, USB_PROBE};
use quadcam_lib::gear::bf::cli::Timing;
use quadcam_lib::gear::bf::fake::FakeFc;
use quadcam_lib::gear::cues::{CueService, RecordedCues};
use quadcam_lib::gear::model::{device_id, DeviceKind, Refusal, RefusalCode, Section};
use quadcam_lib::gear::serial::{lock_port, FakePorts, PortInfo, Ports};
use quadcam_lib::gear::Env;
use quadcam_lib::mcp::{LocalBackend, Server};
use quadcam_lib::photos::Recorder;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

const PORT: &str = "/dev/cu.usbmodemFAKE1";
const G473: &str = include_str!("../fixtures/bf/g473-2025.12.5.dump_all.txt");
const G473_AFTER: &str = include_str!("../fixtures/bf/g473-2025.12.5.after.dump_all.txt");
const V2: &str = include_str!("../fixtures/bf/g473v2-2026.6.0.dump_all.txt");

const UID: [u8; 12] = [
    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x10, 0x32, 0x54, 0x76,
];

struct Bench {
    core: Arc<Core>,
    cues: Arc<RecordedCues>,
    locks: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

fn bench_with(ports: Arc<dyn Ports>, locks: std::path::PathBuf, dir: tempfile::TempDir) -> Bench {
    let cues = Arc::new(RecordedCues::default());
    let mut env = Env::fake(vec![], ports);
    env.cues = Arc::new(CueService::inline(cues.clone()));
    let core = Arc::new(
        Core::new(
            dir.path().join("cache"),
            None,
            Arc::new(NoHooks),
            Arc::new(Recorder::default()),
        )
        .with_settings(dir.path().join("support/settings.json"))
        .with_gear_env(env)
        .with_fc_timing(Timing::fast()),
    );
    Bench {
        core,
        cues,
        locks,
        _dir: dir,
    }
}

fn bench(fc: &FakeFc) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let locks = dir.path().join("locks");
    let ports = Arc::new(fc.ports(PORT, Some(locks.clone())));
    bench_with(ports, locks, dir)
}

/// The port's lock is free: no QuadCam link holds it.
fn released(locks: &Path) -> bool {
    lock_port(locks, PORT).is_ok()
}

#[test]
fn identify_over_msp_releases_the_port_and_cues_once() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    // Before: the serial FC has no id.
    assert_eq!(b.core.gear_connected().unwrap()[0].id, None);
    let j = b.core.gear_fc_identify(&FcPortParams::default()).unwrap();
    let i = &j.result;
    assert_eq!(i.identity.board.as_deref(), Some("BETAFPVG473"));
    assert_eq!(i.identity.firmware.as_deref(), Some("Betaflight"));
    assert_eq!(i.identity.version.as_deref(), Some("2025.12.5-alpha"));
    let want = device_id(DeviceKind::Fc, &format!("bf-uid:{}", fc.uid_hex()));
    assert_eq!(i.id.as_deref(), Some(want.as_str()));
    assert!(
        !i.id.as_deref().unwrap().contains(&fc.uid_hex()),
        "the UID is hashed"
    );
    assert!(i.read_only.is_none(), "this board and build are proven");
    assert_eq!(i.usb_board_minutes, Some(10));
    assert_eq!(j.notes.len(), 1, "the 2025.12.5 beeper note");
    assert!(
        fc.log().iter().all(|l| l.starts_with("msp ")),
        "MSP only: {:?}",
        fc.log()
    );
    assert_eq!(fc.exits(), 0, "no CLI, no reboot");
    assert!(released(&b.locks));
    assert_eq!(b.cues.spoken(), vec!["The FC done, safe to unplug."]);
    // The next look fills the id: the poll reports it identified.
    let c = &b.core.gear_connected().unwrap()[0];
    assert_eq!(c.id.as_deref(), Some(want.as_str()));
    // Same id through the CLI's mcu_id (a read): MSP and dump agree.
    let r = b.core.gear_fc_read(&FcReadParams::default()).unwrap();
    assert_eq!(r.result.info.id.as_deref(), Some(want.as_str()));
}

#[test]
fn a_backup_read_reboots_releases_and_cues_once() {
    let fc = FakeFc::new(V2).with_uid(UID).slow_dumps(50);
    let b = bench(&fc);
    let j = b.core.gear_fc_read(&FcReadParams::default()).unwrap();
    let lines: Vec<&str> = j.result.replies.iter().map(|r| r.line.as_str()).collect();
    assert_eq!(lines, ["version", "status", "diff all", "dump all"]);
    let dump = j.result.file("dump all").unwrap();
    assert!(dump.starts_with("dump all\n# version\n"), "{}", &dump[..60]);
    assert_eq!(
        j.result.info.identity.board.as_deref(),
        Some("BETAFPVG473_V2")
    );
    assert!(j.notes.is_empty(), "no note for this build");
    assert_eq!(
        (fc.saves(), fc.exits()),
        (0, 1),
        "exit: the FC reboots, nothing saved"
    );
    assert!(released(&b.locks));
    assert_eq!(b.cues.spoken().len(), 1);
    // A command that is not a read is refused before anything is sent, with no cue.
    let fc2 = FakeFc::new(V2);
    let b2 = bench(&fc2);
    let e = b2
        .core
        .gear_fc_read(&FcReadParams {
            port: None,
            commands: vec!["set osd_ah_pos = 1".into()],
        })
        .unwrap_err();
    assert!(format!("{e}").starts_with("Refused"), "{e}");
    assert!(fc2.log().is_empty());
    assert!(b2.cues.spoken().is_empty());
}

#[test]
fn the_write_engine_checks_the_fc_again_right_before_it_writes() {
    let lines: Vec<String> = {
        let before = quadcam_lib::gear::bf::dump::Config::parse(G473);
        let after = quadcam_lib::gear::bf::dump::Config::parse(G473_AFTER);
        after
            .sets()
            .filter(|(s, n, v)| before.get(*s, n) != Some(*v))
            .map(|(_, n, v)| format!("set {n} = {v}"))
            .collect()
    };
    // Proven board and build, the planned FC: written, saved, read back, verified.
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    let id = b
        .core
        .gear_fc_identify(&FcPortParams::default())
        .unwrap()
        .result
        .id
        .unwrap();
    let j = b.core.gear_fc_run(None, Some(&id), &lines).unwrap();
    assert!(
        j.result.saved && j.result.verify.is_empty(),
        "{:?}",
        j.result
    );
    assert_eq!(
        fc.saved_value(Section::Master, "osd_cap_alarm").as_deref(),
        Some("400")
    );
    assert!(released(&b.locks));
    // One cue per job; the same cue for the same FC within 30 s plays once (debounce).
    let spoken = b.cues.spoken();
    assert!(
        spoken.len() == 1 && spoken[0].contains("safe to unplug"),
        "{spoken:?}"
    );

    // Another FC than planned: refused after `version`, nothing written.
    let fc = FakeFc::new(G473).with_uid([7; 12]);
    let b = bench(&fc);
    let e = b.core.gear_fc_run(None, Some(&id), &lines).unwrap_err();
    assert_eq!(
        e.downcast_ref::<Refusal>().unwrap().code,
        RefusalCode::DeviceChanged
    );
    assert_eq!(fc.saves(), 0);
    assert!(!fc.log().iter().any(|l| l.starts_with("set ")));
    assert!(released(&b.locks));

    // A board QuadCam has not proven: read only, with the reason.
    let other = G473.replace("board_name BETAFPVG473", "board_name BETAFPVF411");
    let fc = FakeFc::new(&other);
    let b = bench(&fc);
    let i = b
        .core
        .gear_fc_identify(&FcPortParams::default())
        .unwrap()
        .result;
    let ro = i.read_only.unwrap();
    assert_eq!(ro.code, RefusalCode::UnknownBoard);
    assert!(ro.reason.contains("BETAFPVF411"), "{}", ro.reason);
    let e = b.core.gear_fc_run(None, None, &lines).unwrap_err();
    assert_eq!(
        e.downcast_ref::<Refusal>().unwrap().code,
        RefusalCode::UnknownBoard
    );
    assert_eq!((fc.saves(), fc.exits()), (0, 1));
    // The proven build on the other proven board is still read only.
    let swapped = G473.replace("board_name BETAFPVG473", "board_name BETAFPVG473_V2");
    let fc = FakeFc::new(&swapped);
    let b = bench(&fc);
    let e = b.core.gear_fc_run(None, None, &lines).unwrap_err();
    assert_eq!(
        e.downcast_ref::<Refusal>().unwrap().code,
        RefusalCode::UnknownVersion
    );

    // A rejected line: nothing saved; the report names the line.
    let fc = FakeFc::new(G473).reject(&lines[2]);
    let b = bench(&fc);
    let j = b.core.gear_fc_run(None, None, &lines).unwrap();
    assert!(!j.result.saved);
    assert_eq!(j.result.failed.unwrap().line, lines[2]);
    assert_eq!(fc.saves(), 0);
    assert!(released(&b.locks));
}

#[test]
fn ports_are_asked_for_never_guessed() {
    let dir = tempfile::tempdir().unwrap();
    let info = |p: &str| PortInfo {
        port: p.into(),
        vid: 0x0483,
        pid: 0x5740,
        ..Default::default()
    };
    let ports = Arc::new(FakePorts::new(vec![
        info("/dev/cu.usbmodemA"),
        info("/dev/cu.usbmodemB"),
    ]));
    let b = bench_with(ports, dir.path().join("locks"), dir);
    let e = b
        .core
        .gear_fc_identify(&FcPortParams::default())
        .unwrap_err();
    assert_eq!(
        e.downcast_ref::<Refusal>().unwrap().code,
        RefusalCode::SeveralDevices
    );
    let e = b
        .core
        .gear_fc_identify(&FcPortParams {
            port: Some("/dev/cu.usbmodemZ".into()),
        })
        .unwrap_err();
    assert!(format!("{e}").starts_with("No device"), "{e}");
    let dir = tempfile::tempdir().unwrap();
    let b = bench_with(
        Arc::new(FakePorts::new(vec![])),
        dir.path().join("locks"),
        dir,
    );
    let e = b.core.gear_fc_read(&FcReadParams::default()).unwrap_err();
    assert!(format!("{e}").starts_with("No device"), "{e}");
    assert!(b.cues.spoken().is_empty());
}

#[test]
fn a_port_open_elsewhere_is_busy_and_fails_quietly() {
    let fc = FakeFc::new(G473);
    let b = bench(&fc);
    let held = lock_port(&b.locks, PORT).unwrap();
    let e = b
        .core
        .gear_fc_identify(&FcPortParams::default())
        .unwrap_err();
    assert_eq!(
        e.downcast_ref::<Refusal>().unwrap().code,
        RefusalCode::PortBusy
    );
    assert!(b.cues.spoken().is_empty());
    drop(held);
}

#[test]
fn the_usb_timer_warns_once_at_the_board_limit() {
    let fc = FakeFc::new(V2).with_uid(UID);
    let b = bench(&fc);
    let t0 = Instant::now();
    // USB only: no timer running.
    let t = b.core.gear_usb_tick(t0);
    assert_eq!(t.len(), 1);
    assert!(!t[0].battery);
    assert!(t[0].id.is_some(), "the timer identifies the FC first");
    // Battery in: the board's 10 minutes, under the 20 minute setting.
    fc.set_battery(7.6);
    let t = b.core.gear_usb_tick(t0 + USB_PROBE);
    assert!(t[0].battery);
    assert_eq!(t[0].limit_s, Some(600));
    let start = t0 + USB_PROBE;
    let at = |m: u64| start + Duration::from_secs(m * 60);
    // A tick before the next probe reads nothing.
    let opens = fc.opens();
    b.core.gear_usb_tick(start + Duration::from_secs(5));
    assert_eq!(fc.opens(), opens);
    let t = b.core.gear_usb_tick(at(5));
    assert_eq!(t[0].remaining_s, Some(300));
    assert!(b.cues.spoken().is_empty(), "nothing before the limit");
    let t = b.core.gear_usb_tick(at(10));
    assert_eq!(t[0].remaining_s, Some(0));
    assert!(t[0].warned);
    assert_eq!(
        b.cues.spoken(),
        vec!["Unplug The FC now. It has run on USB with a battery for 10 minutes."]
    );
    b.core.gear_usb_tick(at(12));
    assert_eq!(b.cues.spoken().len(), 1, "one warning per battery session");
    assert!(released(&b.locks), "the timer never holds the port");
    // Battery out: the session ends; a new one starts from zero.
    fc.set_battery(0.0);
    let t = b.core.gear_usb_tick(at(13));
    assert!(!t[0].battery && !t[0].warned && t[0].elapsed_s == 0);
    // The setting off turns the timer off.
    std::fs::create_dir_all(b._dir.path().join("support")).unwrap();
    std::fs::write(
        b._dir.path().join("support/settings.json"),
        r#"{"gearUsbMinutes":0}"#,
    )
    .unwrap();
    fc.set_battery(7.6);
    let t = b.core.gear_usb_tick(at(14));
    assert_eq!(t[0].limit_s, None);
    // Status carries the timers for the device row.
    let s = b.core.gear_status().unwrap();
    assert_eq!(s.usb_timers.len(), 1);
}

/// The FC's ports, with the port left out of the next `hide` lists: the FC rebooting after a
/// CLI job, as the detection poll sees it.
struct Blinking {
    inner: quadcam_lib::gear::serial::FakePorts,
    hide: std::sync::atomic::AtomicU32,
}

impl Ports for Blinking {
    fn list(&self) -> Vec<PortInfo> {
        use std::sync::atomic::Ordering::SeqCst;
        // A compare-exchange loop: `fetch_update` is deprecated on newer toolchains.
        let mut n = self.hide.load(SeqCst);
        let mut hidden = false;
        while n > 0 {
            match self.hide.compare_exchange(n, n - 1, SeqCst, SeqCst) {
                Ok(_) => {
                    hidden = true;
                    break;
                }
                Err(now) => n = now,
            }
        }
        if hidden {
            return Vec::new();
        }
        self.inner.list()
    }
    fn open(
        &self,
        port: &str,
        baud: u32,
    ) -> anyhow::Result<Box<dyn quadcam_lib::gear::serial::SerialLink>> {
        self.inner.open(port, baud)
    }
}

#[test]
fn the_usb_timer_keeps_counting_while_the_fc_reboots() {
    let fc = FakeFc::new(V2).with_uid(UID);
    let dir = tempfile::tempdir().unwrap();
    let locks = dir.path().join("locks");
    let ports = Arc::new(Blinking {
        inner: fc.ports(PORT, Some(locks.clone())),
        hide: Default::default(),
    });
    let b = bench_with(ports.clone(), locks, dir);
    fc.set_battery(7.6);
    let t0 = Instant::now();
    let t = b.core.gear_usb_tick(t0);
    assert!(t[0].battery);
    // A read ends in a reboot; the poll misses the port twice while the FC restarts.
    b.core.gear_fc_read(&FcReadParams::default()).unwrap();
    assert!(fc.exits() > 0, "the read rebooted the FC");
    ports.hide.store(2, std::sync::atomic::Ordering::SeqCst);
    assert!(b.core.gear_connected().unwrap().is_empty());
    let s = b.core.gear_status().unwrap();
    assert!(s.connected.is_empty());
    assert_eq!(s.usb_timers.len(), 1, "the timer outlives the reboot");
    assert!(s.usb_timers[0].battery);
    assert_eq!(b.core.gear_connected().unwrap().len(), 1, "the FC is back");
    // Five minutes on: the timer counts from the first battery reading, not from the reboot.
    let t = b.core.gear_usb_tick(t0 + Duration::from_secs(300));
    assert_eq!(t.len(), 1);
    assert!(t[0].battery);
    assert!(t[0].elapsed_s >= 300, "{t:?}");
    assert_eq!(t[0].limit_s, Some(600));
}

/// Two FCs on one port path: the port goes for `hide` lists, and `second` picks the FC
/// that answers after.
struct Swapping {
    first: FakePorts,
    other: FakePorts,
    second: std::sync::atomic::AtomicBool,
    hide: std::sync::atomic::AtomicU32,
}

impl Ports for Swapping {
    fn list(&self) -> Vec<PortInfo> {
        use std::sync::atomic::Ordering::SeqCst;
        if self.hide.load(SeqCst) > 0 {
            self.hide.fetch_sub(1, SeqCst);
            return Vec::new();
        }
        self.pick().list()
    }
    fn open(
        &self,
        port: &str,
        baud: u32,
    ) -> anyhow::Result<Box<dyn quadcam_lib::gear::serial::SerialLink>> {
        self.pick().open(port, baud)
    }
}

impl Swapping {
    fn pick(&self) -> &FakePorts {
        if self.second.load(std::sync::atomic::Ordering::SeqCst) {
            &self.other
        } else {
            &self.first
        }
    }
}

#[test]
fn another_fc_on_the_same_port_starts_its_own_usb_timer() {
    let a = FakeFc::new(V2).with_uid(UID);
    let mut uid = UID;
    uid[0] ^= 0xff;
    let other = FakeFc::new(V2).with_uid(uid);
    let dir = tempfile::tempdir().unwrap();
    let locks = dir.path().join("locks");
    let ports = Arc::new(Swapping {
        first: a.ports(PORT, Some(locks.clone())),
        other: other.ports(PORT, Some(locks.clone())),
        second: Default::default(),
        hide: Default::default(),
    });
    let b = bench_with(ports.clone(), locks, dir);
    a.set_battery(7.6);
    other.set_battery(7.6);
    let t0 = Instant::now();
    let t = b.core.gear_usb_tick(t0);
    assert!(t[0].battery);
    let first_id = t[0].id.clone();
    b.core.gear_usb_tick(t0 + Duration::from_secs(480));
    // The person swaps FCs within the grace: the port goes and comes back.
    ports.hide.store(1, std::sync::atomic::Ordering::SeqCst);
    assert!(b.core.gear_connected().unwrap().is_empty());
    ports
        .second
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(b.core.gear_connected().unwrap().len(), 1);
    let t = b
        .core
        .gear_usb_tick(t0 + Duration::from_secs(480) + USB_PROBE);
    assert_eq!(t.len(), 1);
    assert_ne!(t[0].id, first_id, "the timer names the new FC");
    assert!(
        t[0].elapsed_s < 60,
        "the old FC's minutes do not carry over: {t:?}"
    );
}

#[test]
fn a_paused_port_is_never_probed_and_a_busy_one_is_skipped() {
    use quadcam_lib::core::PollPauseParams;
    let fc = FakeFc::new(V2).with_uid(UID);
    let b = bench(&fc);
    let t0 = Instant::now();
    let paused = b
        .core
        .gear_poll_pause(&PollPauseParams {
            port: None,
            paused: true,
        })
        .unwrap();
    assert_eq!(paused, vec![PORT.to_string()]);
    assert_eq!(b.core.gear_status().unwrap().paused, paused);
    b.core.gear_usb_tick(t0);
    b.core.gear_usb_tick(t0 + USB_PROBE * 2);
    assert_eq!(fc.opens(), 0, "a paused port is not opened");
    // Another process holds the port: the probe is refused, nothing breaks, and the next
    // probe after it is released reads.
    b.core
        .gear_poll_pause(&PollPauseParams {
            port: Some(PORT.into()),
            paused: false,
        })
        .unwrap();
    assert!(b.core.gear_status().unwrap().paused.is_empty());
    // The same switch through the MCP tool.
    let mut srv = Server::new(LocalBackend(b.core.clone()));
    let r = srv.call_tool("quadcam_gear_edit", json!({"action": "poll_pause"}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(b.core.gear_paused_ports(), vec![PORT.to_string()]);
    let r = srv.call_tool(
        "quadcam_gear_edit",
        json!({"action": "poll_pause", "paused": false}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert!(b.core.gear_paused_ports().is_empty());
    let held = lock_port(&b.locks, PORT).unwrap();
    let t = b.core.gear_usb_tick(t0 + USB_PROBE * 3);
    assert!(t.iter().all(|x| x.volts.is_none()));
    assert_eq!(fc.opens(), 0, "a busy port is not probed");
    drop(held);
    let t = b.core.gear_usb_tick(t0 + USB_PROBE * 5);
    assert_eq!(t.len(), 1);
    assert!(fc.opens() > 0);
}

#[test]
fn mcp_and_dispatch_surfaces() {
    let fc = FakeFc::new(G473).with_uid(UID);
    let b = bench(&fc);
    let mut s = Server::new(LocalBackend(b.core.clone()));
    let r = s.call_tool("quadcam_gear", json!({"action": "fc_identify"}));
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("BETAFPVG473"), "{text}");
    assert!(text.contains("Note: After a USB session"), "{text}");
    assert!(!text.contains(&fc.uid_hex()), "no raw UID: {text}");
    assert!(!r["structuredContent"].to_string().contains(&fc.uid_hex()));
    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "board_notes", "board": "betafpvg473"}),
    );
    assert_eq!(
        r["structuredContent"]["notes"].as_array().unwrap().len(),
        1,
        "{r}"
    );
    let r = s.call_tool("quadcam_gear", json!({"action": "usb_timers"}));
    assert_eq!(r["isError"], false, "{r}");
    let r = s.call_tool(
        "quadcam_gear_edit",
        json!({"action": "fc_read", "commands": ["version", "get osd_ah_pos"]}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("osd_ah_pos = 4281"), "{text}");
    let r = s.call_tool(
        "quadcam_gear_edit",
        json!({"action": "fc_read", "commands": ["save"]}),
    );
    assert_eq!(r["isError"], true);
    assert_eq!(fc.saves(), 0);
    // Dispatch rows exist for the socket and the GUI.
    let v: Value = b.core.dispatch("gear_board_notes", json!({})).unwrap();
    assert!(!v.as_array().unwrap().is_empty());
}

#[test]
fn an_unknown_radio_shows_its_board_profile_over_mcp() {
    // A radio QuadCam has never saved, on its USB serial port: the product names the board.
    let dir = tempfile::tempdir().unwrap();
    let locks = dir.path().join("locks");
    let ports = Arc::new(FakePorts::new(vec![PortInfo {
        port: "/dev/cu.usbmodemFAKE9".into(),
        vid: 0x0483,
        pid: 0x5740,
        product: Some("Jumper T20 V2 Serial Port".into()),
        ..Default::default()
    }]));
    let b = bench_with(ports, locks, dir);
    let status = b.core.gear_status().unwrap();
    let r = status.connected[0].radio.as_ref().expect("a profile");
    assert_eq!(status.connected[0].kind, DeviceKind::Radio);
    assert_eq!(r.profile.board, "t20v2");
    assert!(!r.proven);
    assert!(r.dfu_steps.iter().any(|s| s.contains("toothpick")));
    let mut s = Server::new(LocalBackend(b.core.clone()));
    let r = s.call_tool("quadcam_gear", json!({"action": "status"}));
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Jumper T20 V2 (t20v2, read-only"), "{text}");
    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "radio_boards", "board": "t20v2"}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("t20v2 | Jumper T20 V2 | STM32F407xG | bin | read-only"),
        "{text}"
    );
    assert!(text.contains("DFU: Turn the radio off."), "{text}");
    assert!(
        text.contains("Warning: The T20 and the T20 V2 look alike"),
        "{text}"
    );
    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "radio_boards", "usb_product": "HelloRadioSky V12 Joystick"}),
    );
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("DFU: unknown."), "{text}");
    assert!(text.contains("older than 2.12.3"), "{text}");
    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "radio_boards", "board": "no-such-board"}),
    );
    assert_eq!(r["isError"], true, "{r}");
    let v: Value = b.core.dispatch("gear_radio_boards", json!({})).unwrap();
    let all = v.as_array().unwrap();
    assert!(all.len() > 40, "{}", all.len());
    assert_eq!(
        all.iter().filter(|r| r["proven"] == true).count(),
        2,
        "the Pocket and the GX12"
    );
}

#[test]
fn the_cli_checks_offline_and_finds_no_fc_under_cargo() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path();
    let diff = dir.join("x.diff_all.txt");
    std::fs::write(
        &diff,
        include_str!("../fixtures/bf/g473v2-2026.6.0.diff_all.txt"),
    )
    .unwrap();
    let exp = dir.join("expected.cli");
    std::fs::write(&exp, "# the OSD\nfeature OSD\nset osd_ah_pos = 4174\n").unwrap();
    let cli = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
            .env("HOME", dir)
            .env_remove("QUADCAM_SERIAL")
            .arg("--json")
            .args(args)
            .output()
            .unwrap()
    };
    let out = cli(&[
        "gear",
        "fc",
        "check",
        diff.to_str().unwrap(),
        exp.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    std::fs::write(&exp, "set osd_ah_pos = 1\n").unwrap();
    let out = cli(&[
        "gear",
        "fc",
        "check",
        diff.to_str().unwrap(),
        exp.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    // No real port under cargo: "no device", exit 4.
    let out = cli(&["gear", "fc", "identify"]);
    assert_eq!(
        out.status.code(),
        Some(4),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["code"], "no_device");
    let out = cli(&["gear", "fc", "notes", "--board", "BETAFPVG473"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["result"].as_array().unwrap().len(), 1, "{v}");
}
