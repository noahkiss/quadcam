//! Blackbox pull on connect: read only the used bytes over MSP, verify, store in the blob
//! store, erase only after a verified pull and only when the setting allows, the USB heat
//! timer, paused and held ports, the USB disk path, listing with guessed flights, export and
//! the manual erase. Every FC is `FakeFc`; no real port, disk or FC is touched.

use quadcam_lib::api;
use quadcam_lib::core::{
    blackbox_hooks, BlackboxEraseParams, BlackboxExportParams, BlackboxFilter, BlackboxPullParams,
    Core, EraseState, HookOutcome, NoHooks, PollPauseParams, PullMode, USB_PROBE,
};
use quadcam_lib::disk::{DiskInfo, Volume};
use quadcam_lib::gear::bf::blackbox::{synth_image, MIN_API_MINOR};
use quadcam_lib::gear::bf::cli::Timing;
use quadcam_lib::gear::bf::fake::FakeFc;
use quadcam_lib::gear::blackbox::Method;
use quadcam_lib::gear::blobs::Blobs;
use quadcam_lib::gear::cues::{CueService, RecordedCues};
use quadcam_lib::gear::flights::synth as flight_synth;
use quadcam_lib::gear::model::{Refusal, RefusalCode};
use quadcam_lib::gear::serial::lock_port;
use quadcam_lib::gear::store::Store;
use quadcam_lib::gear::Env;
use quadcam_lib::mcp::{LocalBackend, Server};
use quadcam_lib::photos::Recorder;
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PORT: &str = "/dev/cu.usbmodemFAKE1";
const G473: &str = include_str!("fixtures/bf/g473-2025.12.5.dump_all.txt");
const FLASH: u32 = 16 * 1024 * 1024;
const NOTHING: &str = "0000-01-01T00:00:00.000+00:00";

struct Bench {
    core: Arc<Core>,
    cues: Arc<RecordedCues>,
    locks: PathBuf,
    dir: tempfile::TempDir,
}

impl Bench {
    fn settings(&self, json: &str) {
        let p = self.dir.path().join("support/settings.json");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, json).unwrap();
    }
    fn blobs(&self) -> Blobs {
        Blobs::new(Store::new(self.dir.path().join("support/gear")))
    }
    fn released(&self) -> bool {
        lock_port(&self.locks, PORT).is_ok()
    }
}

fn bench_env(fc: &FakeFc, edit: impl FnOnce(&mut Env)) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let locks = dir.path().join("locks");
    let ports = Arc::new(fc.ports(PORT, Some(locks.clone())));
    let cues = Arc::new(RecordedCues::default());
    let mut env = Env::fake(vec![], ports);
    env.cues = Arc::new(CueService::inline(cues.clone()));
    edit(&mut env);
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
        dir,
    }
}

fn bench(fc: &FakeFc) -> Bench {
    bench_env(fc, |_| {})
}

fn image() -> Vec<u8> {
    synth_image(&[("Whoop", 9_000), ("Whoop", 5_000)], NOTHING)
}

fn fc_with(img: Vec<u8>) -> FakeFc {
    FakeFc::new(G473)
        .with_uid([7; 12])
        .with_dataflash(img, FLASH)
}

fn pull(b: &Bench) -> anyhow::Result<quadcam_lib::core::BlackboxPullResult> {
    b.core
        .gear_blackbox_pull(&BlackboxPullParams::default())
        .map(|j| j.result)
}

fn refusal(e: &anyhow::Error) -> Option<RefusalCode> {
    e.downcast_ref::<Refusal>().map(|r| r.code)
}

#[test]
fn a_pull_reads_the_used_bytes_verifies_stores_and_keeps_the_flash_by_default() {
    let img = image();
    let fc = fc_with(img.clone());
    let b = bench(&fc);
    let r = pull(&b).unwrap();
    // Only the used bytes were asked for: 4 KB at a time, none past the end.
    assert_eq!(fc.dataflash_reads() as usize, img.len().div_ceil(4096));
    assert_eq!(r.read_bytes as usize, img.len());
    assert_eq!(r.method, Some(Method::Msp));
    let p = r.pull.clone().unwrap();
    assert!(r.new);
    assert_eq!((p.used as usize, p.total), (img.len(), FLASH as u64));
    assert_eq!(p.logs.len(), 2);
    assert_eq!(p.craft.as_deref(), Some("Whoop"));
    assert!(p
        .firmware
        .as_deref()
        .unwrap()
        .starts_with("Betaflight 2025.12.5"));
    assert!(!p.logs[0].dated, "an FC without a clock logs 0000-01-01");
    // The blob reads back equal, from the content-addressed store.
    assert_eq!(b.blobs().get(&p.blob).unwrap(), img);
    // The setting is off: the flash is untouched and nothing says it was erased.
    assert_eq!(r.erase, EraseState::Off);
    assert!(!p.erased);
    assert_eq!(fc.dataflash_erases(), 0);
    assert_eq!(fc.dataflash(), img);
    // The port is free, nothing but MSP was sent, and one cue played.
    assert!(b.released());
    assert!(
        fc.log().iter().all(|l| l.starts_with("msp ")),
        "{:?}",
        fc.log()
    );
    assert_eq!(fc.exits(), 0, "no CLI, no reboot");
    assert_eq!(b.cues.spoken(), vec!["The FC done, safe to unplug."]);
    // Pulling the same bytes again stores no second record.
    let again = pull(&b).unwrap();
    assert!(!again.new);
    assert_eq!(
        b.core
            .gear_blackbox(&BlackboxFilter::default())
            .unwrap()
            .len(),
        1
    );
    // An empty flash is not a pull.
    let empty = fc_with(Vec::new());
    let r = pull(&bench(&empty)).unwrap();
    assert!(r.pull.is_none() && r.notes[0].contains("empty"));
}

#[test]
fn with_the_setting_on_the_flash_is_erased_after_verify_and_the_cue_follows_the_erase() {
    let img = image();
    let fc = fc_with(img.clone()).with_erase_polls(3);
    let b = bench(&fc);
    b.settings(r#"{"gearEraseBlackbox":true}"#);
    let r = pull(&b).unwrap();
    assert_eq!(r.erase, EraseState::Done);
    assert!(r.pull.as_ref().unwrap().erased);
    assert_eq!(fc.dataflash_erases(), 1);
    assert!(
        fc.dataflash().is_empty(),
        "the erase finished before the job did"
    );
    // The record says erased and the blob is still there.
    let list = b.core.gear_blackbox(&BlackboxFilter::default()).unwrap();
    assert!(list[0].pull.erased);
    assert_eq!(b.blobs().get(&list[0].pull.blob).unwrap(), img);
    assert_eq!(b.cues.spoken(), vec!["The FC done, safe to unplug."]);
    // `keep` skips the erase for one run, whatever the setting says.
    let fc = fc_with(img.clone());
    let b = bench(&fc);
    b.settings(r#"{"gearEraseBlackbox":true}"#);
    let r = b
        .core
        .gear_blackbox_pull(&BlackboxPullParams {
            keep: true,
            ..Default::default()
        })
        .unwrap()
        .result;
    assert_eq!(r.erase, EraseState::Off);
    assert_eq!(fc.dataflash_erases(), 0);
}

#[test]
fn nothing_is_erased_when_the_read_does_not_verify() {
    // The flash claims more bytes than it holds: the read cannot finish.
    let fc = fc_with(image()).lie_used(100);
    let b = bench(&fc);
    b.settings(r#"{"gearEraseBlackbox":true}"#);
    let e = pull(&b).unwrap_err();
    assert!(
        format!("{e:#}").contains("Reading the blackbox failed"),
        "{e:#}"
    );
    assert_eq!(fc.dataflash_erases(), 0);
    assert_eq!(b.cues.spoken(), vec!["Blackbox failed on The FC."]);
    assert!(b
        .core
        .gear_blackbox(&BlackboxFilter::default())
        .unwrap()
        .is_empty());
    // Bytes that are no log: the read is whole and the headers fail.
    let fc = fc_with(b"this is not a blackbox log at all".repeat(50));
    let b = bench(&fc);
    b.settings(r#"{"gearEraseBlackbox":true}"#);
    let e = pull(&b).unwrap_err();
    assert!(format!("{e:#}").contains("No log header"), "{e:#}");
    assert_eq!(fc.dataflash_erases(), 0);
    assert!(b
        .core
        .gear_blackbox(&BlackboxFilter::default())
        .unwrap()
        .is_empty());
    // A compressed reply is an error too, never data.
    let fc = fc_with(image()).compress_replies();
    let b = bench(&fc);
    let e = pull(&b).unwrap_err();
    assert!(format!("{e:#}").contains("compressed"), "{e:#}");
    // A pull that cannot read the port at all (an FC that answers no flash summary).
    let sdcard = FakeFc::new(G473).with_uid([7; 12]);
    let b = bench(&sdcard);
    let e = pull(&b).unwrap_err();
    assert!(format!("{e:#}").contains("SD card"), "{e:#}");
    assert_eq!(MIN_API_MINOR, 40);
}

#[test]
fn an_erase_that_does_not_finish_keeps_the_pull_and_says_so() {
    let fc = fc_with(image()).stuck_erase();
    let b = bench(&fc);
    b.settings(r#"{"gearEraseBlackbox":true}"#);
    let e = pull(&b).unwrap_err();
    assert!(format!("{e:#}").contains("did not finish"), "{e:#}");
    // The pull is stored, the record is not marked erased, and no "safe to unplug" played.
    let list = b.core.gear_blackbox(&BlackboxFilter::default()).unwrap();
    assert_eq!(list.len(), 1);
    assert!(!list[0].pull.erased);
    assert!(list[0]
        .pull
        .erase_note
        .as_deref()
        .unwrap()
        .contains("did not finish"));
    assert_eq!(b.cues.spoken(), vec!["Blackbox failed on The FC."]);
}

#[test]
fn the_usb_timer_refuses_a_pull_that_would_outlast_it_and_never_starts_an_erase_it_cannot_finish() {
    // About 650 KB: 7.7 s at the measured rate.
    let big = synth_image(&[("Whoop", 650_000)], NOTHING);
    let fc = fc_with(big.clone());
    fc.set_battery(7.6);
    let b = bench(&fc);
    b.settings(r#"{"gearEraseBlackbox":true}"#);
    // A battery session that began 597 s ago on a board with a 600 s limit: 3 s left.
    let t0 = Instant::now() - Duration::from_secs(597);
    let t = b.core.gear_usb_tick(t0);
    assert_eq!(t[0].limit_s, Some(600));
    let left = b.core.gear_usb_timers()[0].remaining_s.unwrap();
    assert!((1..=3).contains(&left), "{left}");
    let e = pull(&b).unwrap_err();
    assert_eq!(refusal(&e), Some(RefusalCode::UsbHeat), "{e:#}");
    assert_eq!(fc.dataflash_reads(), 0, "nothing was read");
    assert_eq!(fc.dataflash_erases(), 0);
    assert!(b.cues.spoken().is_empty(), "a refusal plays no cue");
    // Forced: the pull happens, the erase does not (no time to finish it), and the answer says why.
    let r = b
        .core
        .gear_blackbox_pull(&BlackboxPullParams {
            force: true,
            ..Default::default()
        })
        .unwrap()
        .result;
    assert_eq!(r.erase, EraseState::Skipped);
    assert!(
        r.erase_note.as_deref().unwrap().contains("USB timer"),
        "{:?}",
        r.erase_note
    );
    assert!(r.notes.iter().any(|n| n.contains("forced")));
    assert_eq!(fc.dataflash_erases(), 0);
    assert_eq!(fc.dataflash(), big);
    let rec = &b.core.gear_blackbox(&BlackboxFilter::default()).unwrap()[0].pull;
    assert!(!rec.erased && rec.erase_note.is_some());
    // With the battery out there is no timer, and the same pull goes through and erases.
    let fc = fc_with(big.clone());
    let b = bench(&fc);
    b.settings(r#"{"gearEraseBlackbox":true}"#);
    let r = pull(&b).unwrap();
    assert_eq!(r.erase, EraseState::Done);
    let _ = USB_PROBE;
}

#[test]
fn a_battery_with_no_usb_timer_is_a_warning_on_a_pull() {
    let fc = fc_with(image());
    fc.set_battery(7.6);
    // No probe has run yet: the battery is in, but no timer counts.
    let b = bench(&fc);
    let r = pull(&b).unwrap();
    assert!(r.pull.is_some());
    assert!(
        r.notes.iter().any(|n| n.contains("not counting")),
        "{:?}",
        r.notes
    );
    // Without a battery there is nothing to say.
    let fc = fc_with(image());
    let r = pull(&bench(&fc)).unwrap();
    assert!(!r.notes.iter().any(|n| n.contains("not counting")));
}

#[test]
fn a_flash_that_grew_after_the_read_is_not_erased() {
    // A second reader appends while we work: simulate by growing the flash before the erase
    // check runs, through a pull whose image is already stored.
    let fc = fc_with(image());
    let b = bench(&fc);
    b.settings(r#"{"gearEraseBlackbox":true}"#);
    // First pull without erase (keep), then the flash grows, then a manual erase refuses.
    b.core
        .gear_blackbox_pull(&BlackboxPullParams {
            keep: true,
            ..Default::default()
        })
        .unwrap();
    fc.append_dataflash(&synth_image(&[("Whoop", 100)], NOTHING));
    let e = b
        .core
        .gear_blackbox_erase(&BlackboxEraseParams {
            port: None,
            confirm: true,
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("Pull first"), "{e:#}");
    assert_eq!(fc.dataflash_erases(), 0);
}

#[test]
fn a_port_another_program_holds_is_left_alone_and_a_paused_port_skips_the_hook() {
    let fc = fc_with(image());
    let b = bench_env(&fc, |env| {
        env.holders = Arc::new(|_| vec![(4242, "Configurator".to_string())]);
    });
    let e = pull(&b).unwrap_err();
    assert_eq!(refusal(&e), Some(RefusalCode::PortBusy), "{e:#}");
    assert!(format!("{e}").contains("Configurator"));
    assert_eq!(fc.opens(), 0, "not even an open");
    assert!(b.cues.spoken().is_empty());

    // The on-connect step: off until the settings list it for FCs.
    let fc = fc_with(image());
    let b = bench(&fc);
    for h in blackbox_hooks() {
        b.core.gear_add_hook(h);
    }
    let c = b.core.gear_connected().unwrap()[0].clone();
    let runs = b.core.gear_on_connect(&c);
    assert_eq!(runs[0].name, "Blackbox");
    assert_eq!(
        runs[0].outcome,
        HookOutcome::Off,
        "no consent: off by default"
    );
    assert_eq!(fc.dataflash_reads(), 0);
    // Consent: the fc kind lists blackbox. A paused port skips it.
    b.settings(r#"{"gearOnConnect":{"fc":["blackbox"]},"gearEraseBlackbox":true}"#);
    b.core
        .gear_poll_pause(&PollPauseParams {
            port: None,
            paused: true,
        })
        .unwrap();
    let runs = b.core.gear_on_connect(&c);
    assert_eq!(runs[0].outcome, HookOutcome::Off, "paused");
    assert_eq!(fc.dataflash_reads(), 0);
    b.core
        .gear_poll_pause(&PollPauseParams {
            port: None,
            paused: false,
        })
        .unwrap();
    let runs = b.core.gear_on_connect(&c);
    assert_eq!(runs[0].outcome, HookOutcome::Ran);
    assert!(fc.dataflash().is_empty(), "pulled, verified, erased");
    let list = b.core.gear_blackbox(&BlackboxFilter::default()).unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].pull.erased);
    // One cue for the whole on-connect run.
    assert_eq!(b.cues.spoken(), vec!["The FC done, safe to unplug."]);
}

/// A USB disk volume with the FC's logs, present only while the FC is in USB disk mode.
fn msc_bench(fc: &FakeFc, files: &[(&str, Vec<u8>)], dir: &std::path::Path) -> Bench {
    let mount = dir.join("BTFL");
    std::fs::create_dir_all(&mount).unwrap();
    for (n, bytes) in files {
        std::fs::write(mount.join(n), bytes).unwrap();
    }
    let vol = Volume {
        mount: mount.clone(),
        info: DiskInfo {
            parent_whole_disk: "disk77".into(),
            removable: true,
            bus_protocol: Some("USB".into()),
            ..Default::default()
        },
        is_card: false,
        source: None,
        is_radio: false,
        warnings: vec![],
    };
    let (f1, f2) = (fc.clone(), fc.clone());
    let released: Arc<Mutex<Vec<String>>> = Arc::default();
    let r2 = released.clone();
    bench_env(fc, move |env| {
        env.volumes = Arc::new(move || {
            if f1.msc_active() {
                vec![vol.clone()]
            } else {
                vec![]
            }
        });
        env.unmount = Arc::new(move |d| {
            r2.lock().unwrap().push(d.to_string());
            f2.msc_release();
            Ok(())
        });
    })
}

#[test]
fn usb_disk_mode_copies_the_files_and_falls_back_to_msp_when_the_fc_lacks_it() {
    // Two log files on the disk; the flash holds the same bytes for the summary and the erase.
    let a = synth_image(&[("Whoop", 9_000)], NOTHING);
    let c = synth_image(&[("Whoop", 5_000)], NOTHING);
    let whole: Vec<u8> = [a.clone(), c.clone()].concat();
    let tmp = tempfile::tempdir().unwrap();
    let fc = fc_with(whole.clone()).with_msc();
    let b = msc_bench(&fc, &[("BTFL_002.BBL", c), ("BTFL_001.BBL", a)], tmp.path());
    b.settings(r#"{"gearBlackboxMsc":true,"gearEraseBlackbox":true}"#);
    let r = pull(&b).unwrap();
    assert_eq!(r.method, Some(Method::Msc));
    assert_eq!(
        fc.dataflash_reads(),
        0,
        "not one MSP read: the disk gave the bytes"
    );
    let p = r.pull.unwrap();
    assert_eq!(
        b.blobs().get(&p.blob).unwrap(),
        whole,
        "files joined in name order"
    );
    assert_eq!(p.method, Method::Msc);
    // The FC came back to serial and the erase ran over MSP.
    assert_eq!(r.erase, EraseState::Done);
    assert!(fc.dataflash().is_empty());
    assert!(!fc.msc_active());
    assert_eq!(b.cues.spoken(), vec!["The FC done, safe to unplug."]);

    // An FC without `msc` in its CLI: the same pull reads over MSP and says so.
    let fc = fc_with(whole.clone());
    let b = bench(&fc);
    b.settings(r#"{"gearBlackboxMsc":true}"#);
    let r = pull(&b).unwrap();
    assert_eq!(r.method, Some(Method::Msp));
    assert!(
        r.notes.iter().any(|n| n.contains("no USB disk mode")),
        "{:?}",
        r.notes
    );
    assert!(fc.dataflash_reads() > 0);
    // Asking for the USB disk mode by name fails instead of falling back.
    let fc = fc_with(whole);
    let b = bench(&fc);
    let e = b
        .core
        .gear_blackbox_pull(&BlackboxPullParams {
            mode: Some(PullMode::Msc),
            ..Default::default()
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("no USB disk mode"), "{e:#}");
    // Without the setting or the mode, MSC is never tried: the CLI is never entered.
    let fc = fc_with(synth_image(&[("Whoop", 100)], NOTHING)).with_msc();
    let b = bench(&fc);
    pull(&b).unwrap();
    assert_eq!(fc.exits(), 0);
    assert!(!fc.log().iter().any(|l| l == "help" || l == "msc"));
}

#[test]
fn an_msc_pull_that_does_not_match_the_flash_is_not_stored_or_erased() {
    // The disk shows fewer bytes than the flash reports used.
    let whole = synth_image(&[("Whoop", 9_000)], NOTHING);
    let tmp = tempfile::tempdir().unwrap();
    let fc = fc_with(whole.clone()).with_msc();
    let part = whole[..whole.len() - 10].to_vec();
    let b = msc_bench(&fc, &[("BTFL_001.BBL", part)], tmp.path());
    b.settings(r#"{"gearBlackboxMsc":true,"gearEraseBlackbox":true}"#);
    let e = pull(&b).unwrap_err();
    assert!(format!("{e:#}").contains("did not verify"), "{e:#}");
    assert_eq!(fc.dataflash_erases(), 0);
    assert!(b
        .core
        .gear_blackbox(&BlackboxFilter::default())
        .unwrap()
        .is_empty());
}

fn flights_bench(fc: &FakeFc) -> Bench {
    let b = bench(fc);
    b.settings(r#"{"profiles":[{"name":"Whoop","edgetx_models":["Whoop"],"place":"Field"}]}"#);
    let logs = b.dir.path().join("logs");
    flight_synth::write_known(&logs).unwrap();
    b.core
        .gear_flight_folders(&api::FlightFoldersParams {
            add: Some(logs),
            remove: None,
        })
        .unwrap();
    b
}

#[test]
fn a_pull_lists_with_its_logs_paired_to_flights_by_order_and_labelled_a_guess() {
    // Three flight-sized logs and the synthetic radio log's three flights.
    let img = synth_image(
        &[("Whoop", 40_000), ("Whoop", 80_000), ("Whoop", 40_000)],
        NOTHING,
    );
    let fc = fc_with(img);
    let b = flights_bench(&fc);
    // Link the FC to the aircraft first (identify saves nothing; a pull saves the device).
    let id = b
        .core
        .gear_fc_identify(&Default::default())
        .unwrap()
        .result
        .id
        .unwrap();
    // Not saved yet: an unlinked FC pairs nothing and says why.
    pull(&b).unwrap();
    let e = &b.core.gear_blackbox(&BlackboxFilter::default()).unwrap()[0];
    assert!(e.flights.links.is_empty());
    assert!(e.flights.note.contains("No aircraft"), "{}", e.flights.note);
    // Link it, then pull a second time (new bytes: the flash grew).
    b.core
        .gear_device_save(&quadcam_lib::core::DeviceSaveParams {
            id: id.clone(),
            name: None,
            aircraft: Some("Whoop".into()),
        })
        .unwrap();
    fc.append_dataflash(&synth_image(&[("Whoop", 100)], NOTHING));
    let r = pull(&b).unwrap();
    assert_eq!(r.pull.as_ref().unwrap().aircraft.as_deref(), Some("Whoop"));
    let list = b
        .core
        .gear_blackbox(&BlackboxFilter { device: Some(id) })
        .unwrap();
    assert_eq!(list.len(), 2, "newest first");
    let fl = &list[0].flights;
    // Three big logs and the tiny fourth (a test arm), three flights: paired by order.
    assert_eq!(fl.links.len(), 3, "{fl:?}");
    assert_eq!(fl.short_logs, 1);
    assert_eq!((fl.unpaired_logs, fl.unpaired_flights), (0, 0));
    assert!(fl.note.starts_with("Guess"), "{}", fl.note);
    let flights = b.core.gear_flights(&Default::default()).unwrap();
    let oldest_first: Vec<String> = flights
        .flights
        .iter()
        .rev()
        .map(|f| f.flight.id.clone())
        .collect();
    let got: Vec<String> = fl.links.iter().map(|l| l.flight.clone()).collect();
    assert_eq!(got, oldest_first);
    assert_eq!(
        fl.links.iter().map(|l| l.log).collect::<Vec<_>>(),
        [1, 2, 3]
    );
}

#[test]
fn export_writes_the_image_and_each_log_and_never_overwrites() {
    let img = image();
    let fc = fc_with(img.clone());
    let b = bench(&fc);
    let p = pull(&b).unwrap().pull.unwrap();
    let out = b.dir.path().join("out");
    let x = b
        .core
        .gear_blackbox_export(&BlackboxExportParams {
            id: p.id.clone(),
            to: out.clone(),
            split: true,
        })
        .unwrap();
    assert_eq!(x.files.len(), 3);
    assert_eq!(std::fs::read(&x.files[0]).unwrap(), img);
    let (l1, l2) = (&p.logs[0], &p.logs[1]);
    assert_eq!(
        std::fs::read(&x.files[1]).unwrap(),
        img[l1.offset as usize..(l1.offset + l1.size) as usize]
    );
    assert_eq!(
        std::fs::read(&x.files[2]).unwrap(),
        img[l2.offset as usize..(l2.offset + l2.size) as usize]
    );
    assert!(x.files[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("Whoop_"));
    let again = b.core.gear_blackbox_export(&BlackboxExportParams {
        id: p.id,
        to: out,
        split: false,
    });
    assert!(format!("{:#}", again.unwrap_err()).contains("already exists"));
    assert!(b
        .core
        .gear_blackbox_export(&BlackboxExportParams {
            id: "nope/2026-01-01T000000".into(),
            to: b.dir.path().join("o2"),
            split: false,
        })
        .is_err());
}

#[test]
fn the_manual_erase_needs_confirm_and_a_stored_pull_of_exactly_the_flash() {
    let img = image();
    let fc = fc_with(img.clone());
    let b = bench(&fc);
    let ask = |confirm| {
        b.core.gear_blackbox_erase(&BlackboxEraseParams {
            port: None,
            confirm,
        })
    };
    // No pull yet: refused. No confirm: refused before any port work.
    assert!(format!("{:#}", ask(false).unwrap_err()).contains("confirm=true"));
    assert!(fc.log().is_empty());
    let e = ask(true).unwrap_err();
    assert!(format!("{e:#}").contains("Pull first"), "{e:#}");
    assert_eq!(fc.dataflash_erases(), 0);
    // After a pull of exactly this flash it erases, and the record says so.
    pull(&b).unwrap();
    let done = ask(true).unwrap();
    assert!(done.result.pull.contains('/'));
    assert!(fc.dataflash().is_empty());
    assert!(
        b.core.gear_blackbox(&BlackboxFilter::default()).unwrap()[0]
            .pull
            .erased
    );
    assert_eq!(
        b.cues.spoken().len(),
        1,
        "debounced: pull and erase are one cue within 30 s"
    );
    // Empty flash: say so.
    let e = ask(true).unwrap_err();
    assert!(format!("{e:#}").contains("already empty"), "{e:#}");
}

#[test]
fn stored_pulls_survive_a_prune() {
    let fc = fc_with(image());
    let b = bench(&fc);
    let p = pull(&b).unwrap().pull.unwrap();
    b.core.gear_prune(&Default::default()).unwrap();
    assert_eq!(
        b.blobs().get(&p.blob).unwrap(),
        image(),
        "the blob is named by the record"
    );
}

#[test]
fn the_surfaces_exist_and_respect_the_setting() {
    let img = image();
    let fc = fc_with(img.clone());
    let b = bench(&fc);
    let mut s = Server::new(LocalBackend(b.core.clone()));
    // MCP pull: the setting is off, so nothing is erased, and no argument turns it on.
    let r = s.call_tool("quadcam_gear_apply", json!({"action": "blackbox_pull"}));
    assert_eq!(r["isError"], true, "needs confirm: {r}");
    assert_eq!(fc.dataflash_reads(), 0);
    let r = s.call_tool(
        "quadcam_gear_apply",
        json!({"action": "blackbox_pull", "confirm": true}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("2 logs verified and stored"), "{text}");
    assert!(text.contains("not erased"), "{text}");
    // The tool has no argument that turns the erase on.
    let edit = quadcam_lib::mcp::gear::tools()
        .into_iter()
        .find(|t| t["name"] == "quadcam_gear_apply")
        .unwrap();
    let props = edit["inputSchema"]["properties"].as_object().unwrap();
    assert!(
        props.contains_key("keep") && !props.contains_key("erase"),
        "no `erase` argument"
    );
    assert_eq!(fc.dataflash_erases(), 0);
    // MCP list shows the pull and the guess label.
    let r = s.call_tool("quadcam_gear", json!({"action": "blackbox"}));
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("2 logs") && text.contains("No aircraft"),
        "{text}"
    );
    let id = r["structuredContent"]["blackbox"][0]["pull"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    // Erase by hand needs confirm.
    let r = s.call_tool("quadcam_gear_apply", json!({"action": "blackbox_erase"}));
    assert_eq!(r["isError"], true);
    assert!(fc.dataflash_erases() == 0);
    let r = s.call_tool(
        "quadcam_gear_apply",
        json!({"action": "blackbox_erase", "confirm": true}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Safe to unplug"));
    // Export through MCP.
    let out = b.dir.path().join("mcp-out");
    let r = s.call_tool(
        "quadcam_gear_edit",
        json!({"action": "blackbox_export", "id": id, "to": out.to_string_lossy()}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 1);
    // Dispatch rows exist for the socket and the GUI.
    let v: serde_json::Value = b.core.dispatch("gear_blackbox", json!({})).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1);
}

#[test]
fn the_cli_pulls_lists_and_refuses_an_erase_without_confirm() {
    // The CLI binary runs its own core; with no serial access under cargo it reports so.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
        .args(["gear", "blackbox", "erase"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("confirm=true"), "{err}");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
        .args(["gear", "blackbox", "pull", "--help"])
        .output()
        .unwrap();
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(
        help.contains("--keep") && !help.contains("--erase"),
        "{help}"
    );
}
