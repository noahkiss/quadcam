//! Sim sync, the write side of WP8 (design 6.6, 8): each adapter reads and writes a
//! synthetic file byte-exact, the plan (checks, diff, warnings, digest), the apply (backup,
//! atomic write, read back, roll back), the refusal while a game "runs" (faked), and the
//! CLI. Every file lives in a temporary home; no real sim file is read or written.

use quadcam_lib::core::{BackupFilter, BackupReadParams, Core, Hooks, NoHooks};
use quadcam_lib::gear::apply::sim::{SimSyncParams, SimSyncRequest, SimTarget};
use quadcam_lib::gear::model::{ApplyPlan, ChangeStatus, Refusal, RefusalCode, StagedChange};
use quadcam_lib::gear::rates::{RateAxis, Rates, RatesType, ThrottleCurve};
use quadcam_lib::gear::sims::{self, Sim};
use quadcam_lib::photos::Recorder;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

fn fixture(dir: &str, name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(dir)
        .join(name)
}

fn core_with(dir: &Path, hooks: Arc<dyn Hooks>) -> Arc<Core> {
    Arc::new(
        Core::new(
            dir.join("cache"),
            None,
            hooks,
            Arc::new(Recorder::default()),
        )
        .with_settings(dir.join("support/settings.json"))
        .with_gear_env(quadcam_lib::gear::Env::fake(
            vec![],
            Arc::new(quadcam_lib::gear::serial::FakePorts::new(vec![])),
        )),
    )
}

fn core(dir: &Path) -> Arc<Core> {
    core_with(dir, Arc::new(NoHooks))
}

fn three() -> PathBuf {
    fixture("rates", "three-profiles.dump_all.txt")
}

fn put(home: &Path, rel: &str, bytes: &[u8]) -> PathBuf {
    let p = home.join("Library/Application Support").join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, bytes).unwrap();
    p
}

/// An Uncrashed profile: 12 floats after the property header.
fn gvas(floats: &[f32]) -> Vec<u8> {
    let mut b = b"GVAS".to_vec();
    b.extend_from_slice(&[3, 0, 0, 0, 10, 2, 0, 0]);
    for s in ["Rates", "ArrayProperty", "FloatProperty"] {
        b.extend_from_slice(&(s.len() as i32 + 1).to_le_bytes());
        b.extend_from_slice(s.as_bytes());
        b.push(0);
    }
    b.push(0);
    b.extend_from_slice(&(floats.len() as i32).to_le_bytes());
    for f in floats {
        b.extend_from_slice(&f.to_le_bytes());
    }
    b.extend_from_slice(b"\x05\0\0\0None\0");
    b
}

const LIFTOFF: &str = "LuGus Studios/Liftoff/Saves/Player/UserData.xml";
const MICRO: &str = "Steam/steamapps/common/Liftoff Micro Drones/Liftoff Micro Drones.app/Contents/Saves/Player/UserData.xml";
const UNCRASHED: &str = "Uncrashed/abc123/rates/FREE.sav";
const ZONE: &str = "Godot/app_userdata/The Zone/settings.cfg";

struct Paths {
    liftoff: PathBuf,
    micro: PathBuf,
    uncrashed: PathBuf,
    zone: PathBuf,
}

/// A home with the four sims' synthetic files. Uncrashed holds the old rates (the FREE
/// profile), every other file holds the shapes of the fixtures.
fn home() -> (tempfile::TempDir, Paths) {
    let home = tempfile::tempdir().unwrap();
    let fx = |n: &str| std::fs::read(fixture("sims", n)).unwrap();
    let paths = Paths {
        liftoff: put(home.path(), LIFTOFF, &fx("liftoff.UserData.xml")),
        micro: put(home.path(), MICRO, &fx("micro.UserData.xml")),
        uncrashed: put(
            home.path(),
            UNCRASHED,
            &gvas(&[
                0.50, 1.00, 0.10, 0.50, 1.00, 0.10, 0.50, 1.00, 0.0, 0.0, 0.50, 0.00,
            ]),
        ),
        zone: put(home.path(), ZONE, &fx("zone.settings.cfg")),
    };
    (home, paths)
}

fn target(sim: &str, profile: &str) -> SimTarget {
    SimTarget {
        sim: sim.into(),
        file: None,
        profile: Some(profile.into()),
    }
}

/// The quad: `three()`'s rate profile `index` (0 FREE Betaflight, 1 RACE Actual).
fn params(sims: Vec<SimTarget>, index: u8) -> SimSyncParams {
    SimSyncParams {
        sims,
        paths: vec![three()],
        profile: Some(index),
        ..Default::default()
    }
}

fn refused(e: &anyhow::Error) -> RefusalCode {
    e.downcast_ref::<Refusal>()
        .unwrap_or_else(|| panic!("not a refusal: {e:#}"))
        .code
}

fn never(_: &str) -> bool {
    false
}

fn sim_of(id: &str) -> &'static dyn Sim {
    sims::all().into_iter().find(|s| s.id() == id).unwrap()
}

fn bf(rc: f64, sup: f64, expo: f64) -> Rates {
    let a = RateAxis {
        rc_rate: rc,
        srate: sup,
        expo,
    };
    Rates {
        rates_type: RatesType::Betaflight,
        axes: [a, a, a],
    }
}

// ----- adapters: byte-exact writes -----

#[test]
fn each_adapter_writes_its_synthetic_file_byte_exact() {
    let (_h, p) = home();
    let throttle = ThrottleCurve {
        mid: 30.0,
        expo: 40.0,
        ..ThrottleCurve::default()
    };
    for (id, path) in [
        ("liftoff", &p.liftoff),
        ("micro", &p.micro),
        ("uncrashed", &p.uncrashed),
        ("zone", &p.zone),
    ] {
        let sim = sim_of(id);
        let raw = std::fs::read(path).unwrap();
        let file = sim.parse_file(&raw, path).unwrap();
        assert_eq!(
            file.doc.render(),
            raw.as_slice(),
            "{id} reads back unchanged"
        );
        let prof = &file.profiles[0];
        let have = prof.rates.unwrap();

        // Writing a profile its own values is a no-op, byte for byte.
        let same = sims::write_profile(sim, &file, 0, &have, prof.throttle.as_ref()).unwrap();
        assert_eq!(
            same.render(),
            raw.as_slice(),
            "{id} writes its own values unchanged"
        );

        // New values: only that profile's value bytes move; the file reads back as wanted
        // and renders to itself.
        let want = bf(150.0, 80.0, 25.0);
        let wt = prof.throttle.map(|_| &throttle);
        let doc = sims::write_profile(sim, &file, 0, &want, wt).unwrap();
        assert_ne!(doc.render(), raw.as_slice(), "{id}");
        let again = sim.parse_file(doc.render(), path).unwrap();
        assert_eq!(again.doc.render(), doc.render());
        let r = again.profiles[0].rates.unwrap();
        for a in 0..3 {
            assert_eq!(
                (r.axes[a].rc_rate, r.axes[a].srate, r.axes[a].expo),
                (150.0, 80.0, 25.0),
                "{id} axis {a}"
            );
        }
        if let Some(t) = again.profiles[0].throttle {
            assert_eq!((t.mid, t.expo), (30.0, 40.0), "{id} throttle");
        }
        // Everything outside the replaced spans is untouched: the same bytes before the
        // first span and after the last, and the other profiles read the same.
        let first = prof.spans.iter().map(|s| s.start).min().unwrap();
        assert_eq!(&doc.render()[..first], &raw[..first], "{id} head");
        for (i, other) in file.profiles.iter().enumerate().skip(1) {
            assert_eq!(again.profiles[i].rates, other.rates, "{id} profile {i}");
        }
    }
}

#[test]
fn an_adapter_refuses_a_profile_it_did_not_read_and_a_non_betaflight_model() {
    let (_h, p) = home();
    let zone = sim_of("zone");
    let raw = std::fs::read(&p.zone).unwrap();
    let file = zone.parse_file(&raw, &p.zone).unwrap();
    // Profile 1 uses the actual type.
    assert!(sims::write_profile(zone, &file, 1, &bf(100.0, 70.0, 0.0), None).is_err());
    let mut actual = bf(100.0, 70.0, 0.0);
    actual.rates_type = RatesType::Actual;
    assert!(sims::write_profile(zone, &file, 0, &actual, None).is_err());
    assert!(sims::write_profile(zone, &file, 9, &bf(1.0, 1.0, 1.0), None).is_err());
}

// ----- the plan -----

#[test]
fn the_plan_shows_checks_the_diff_warnings_and_a_digest() {
    let dir = tempfile::tempdir().unwrap();
    let (h, _p) = home();
    let c = core(dir.path());
    // Quad profile 0 (FREE) is Betaflight 127/72/40 roll and pitch, 100/75/0 yaw.
    let plan = c
        .gear_sim_sync_plan_at(
            &params(
                vec![target("liftoff", "Race"), target("uncrashed", "FREE")],
                0,
            ),
            h.path(),
            &never,
        )
        .unwrap();
    assert!(plan.ready(), "{:?}", plan.checks);
    assert!(!plan.digest.is_empty());
    assert_eq!(plan.change, "sim-sync");
    for name in [
        "Sim closed (Liftoff)",
        "File understood (Liftoff)",
        "Rewrites unchanged (Liftoff)",
        "Writable (Liftoff)",
        "Sim closed (Uncrashed)",
    ] {
        assert!(plan.checks.iter().any(|k| k.name == name && k.ok), "{name}");
    }
    // The diff lists the values that change, old line out, new line in.
    let text = format!("{:?}", plan.diff);
    assert!(text.contains("Roll RC rate 100"), "{text}");
    assert!(text.contains("Roll RC rate 127"), "{text}");
    assert!(text.contains("Throttle mid"), "{text}");
    // Warnings: Liftoff has no throttle curve; both shapes are read from a real install but
    // only Uncrashed's write has been seen loading.
    let w = plan.warnings.join("\n");
    assert!(w.contains("Liftoff has no throttle curve"), "{w}");
    assert!(w.contains("Unverified: QuadCam read Liftoff"), "{w}");
    assert!(!w.contains("Unverified: QuadCam read Uncrashed"), "{w}");
    // The same inputs give the same digest; another quad profile gives another.
    let again = c
        .gear_sim_sync_plan_at(
            &params(
                vec![target("liftoff", "Race"), target("uncrashed", "FREE")],
                0,
            ),
            h.path(),
            &never,
        )
        .unwrap();
    assert_eq!(plan.digest, again.digest);
}

#[test]
fn an_actual_quad_is_fitted_and_the_plan_says_how_far_off() {
    let dir = tempfile::tempdir().unwrap();
    let (h, _p) = home();
    let c = core(dir.path());
    let plan = c
        .gear_sim_sync_plan_at(
            &params(vec![target("zone", "Profile 0")], 1),
            h.path(),
            &never,
        )
        .unwrap();
    assert!(plan.ready(), "{:?}", plan.checks);
    let w = plan.warnings.join("\n");
    assert!(w.contains("uses the actual model"), "{w}");
    assert!(w.contains("largest gap"), "{w}");
}

#[test]
fn the_plan_refuses_a_running_sim_a_missing_profile_and_an_unknown_sim() {
    let dir = tempfile::tempdir().unwrap();
    let (h, _p) = home();
    let c = core(dir.path());
    let code_of = |plan: &ApplyPlan| {
        plan.checks
            .iter()
            .find_map(|k| k.refusal.as_ref().map(|r| (k.name.clone(), r.code)))
    };
    let running = |n: &str| n == "Liftoff";
    let plan = c
        .gear_sim_sync_plan_at(
            &params(vec![target("liftoff", "Race")], 0),
            h.path(),
            &running,
        )
        .unwrap();
    assert!(!plan.ready());
    assert!(plan.digest.is_empty());
    assert_eq!(
        code_of(&plan),
        Some(("Sim closed (Liftoff)".into(), RefusalCode::SimRunning))
    );
    let reason = plan.checks.iter().find_map(|k| k.refusal.as_ref()).unwrap();
    assert_eq!(reason.reason, "Quit Liftoff first.");

    let plan = c
        .gear_sim_sync_plan_at(
            &params(vec![target("liftoff", "Nope")], 0),
            h.path(),
            &never,
        )
        .unwrap();
    assert_eq!(code_of(&plan).unwrap().1, RefusalCode::ShapeUnknown);
    let plan = c
        .gear_sim_sync_plan_at(
            &params(vec![target("flightgear", "x")], 0),
            h.path(),
            &never,
        )
        .unwrap();
    assert_eq!(code_of(&plan).unwrap().1, RefusalCode::ShapeUnknown);
    let plan = c
        .gear_sim_sync_plan_at(&params(vec![], 0), h.path(), &never)
        .unwrap();
    assert!(!plan.ready());
    // Velocidrone ships off.
    let plan = c
        .gear_sim_sync_plan_at(
            &params(vec![target("velocidrone", "x")], 0),
            h.path(),
            &never,
        )
        .unwrap();
    assert_eq!(code_of(&plan).unwrap().1, RefusalCode::Disabled);
    // A profile in another model is not written.
    let plan = c
        .gear_sim_sync_plan_at(
            &params(vec![target("zone", "Profile 1")], 0),
            h.path(),
            &never,
        )
        .unwrap();
    assert_eq!(code_of(&plan).unwrap().1, RefusalCode::ShapeUnknown);
}

#[test]
fn a_profile_is_found_by_the_quads_profile_name_and_all_picks_every_sim() {
    let dir = tempfile::tempdir().unwrap();
    let (h, _p) = home();
    let c = core(dir.path());
    // No profile named: the quad's profile is called FREE; Uncrashed's file is FREE.sav.
    let p = SimTarget {
        sim: "uncrashed".into(),
        ..Default::default()
    };
    let plan = c
        .gear_sim_sync_plan_at(&params(vec![p], 0), h.path(), &never)
        .unwrap();
    assert!(plan.ready(), "{:?}", plan.checks);
    // Liftoff has no FREE profile: the plan says what it has.
    let p = SimTarget {
        sim: "liftoff".into(),
        ..Default::default()
    };
    let plan = c
        .gear_sim_sync_plan_at(&params(vec![p], 0), h.path(), &never)
        .unwrap();
    let r = plan.checks.iter().find_map(|k| k.refusal.as_ref()).unwrap();
    assert!(r.reason.contains("Freestyle, Race, Cine"), "{}", r.reason);
    // `all` takes each sim's profile named like the quad's: only Uncrashed has FREE.
    let all = SimTarget {
        sim: "all".into(),
        ..Default::default()
    };
    let plan = c
        .gear_sim_sync_plan_at(&params(vec![all], 0), h.path(), &never)
        .unwrap();
    assert_eq!(
        plan.checks.iter().filter(|k| !k.ok).count(),
        3,
        "{:?}",
        plan.checks
    );
}

// ----- the apply -----

fn request(p: SimSyncParams, plan: &ApplyPlan, confirm: bool) -> SimSyncRequest {
    SimSyncRequest {
        params: p,
        digest: plan.digest.clone(),
        confirm,
    }
}

#[test]
fn a_sync_backs_up_writes_reads_back_and_leaves_the_rest_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (h, paths) = home();
    let c = core(dir.path());
    let p = params(
        vec![target("liftoff", "Race"), target("uncrashed", "FREE")],
        0,
    );
    let before_lift = std::fs::read(&paths.liftoff).unwrap();
    let before_unc = std::fs::read(&paths.uncrashed).unwrap();
    let plan = c.gear_sim_sync_plan_at(&p, h.path(), &never).unwrap();
    // Nothing is written by a plan.
    assert_eq!(std::fs::read(&paths.liftoff).unwrap(), before_lift);

    // No confirm, no write.
    let e = c
        .sim_sync_at(&request(p.clone(), &plan, false), true, h.path(), &never)
        .unwrap_err();
    assert!(format!("{e:#}").contains("confirm=true"));
    assert_eq!(std::fs::read(&paths.liftoff).unwrap(), before_lift);

    // A wrong digest refuses.
    let mut bad = request(p.clone(), &plan, true);
    bad.digest = "nope".into();
    let e = c.sim_sync_at(&bad, true, h.path(), &never).unwrap_err();
    assert_eq!(refused(&e), RefusalCode::BeforeMismatch);

    let r = c
        .sim_sync_at(&request(p.clone(), &plan, true), true, h.path(), &never)
        .unwrap();
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    assert!(r.steps.iter().all(|s| format!("{:?}", s.state) == "Done"));
    for name in [
        "Back up Liftoff",
        "Write Liftoff",
        "Read back Liftoff",
        "Write Uncrashed",
    ] {
        assert!(r.steps.iter().any(|s| s.name == name), "{name}");
    }
    assert_eq!(r.files.len(), 2);

    // The files hold the quad's rates now; the others are untouched.
    let after = sims::all()
        .into_iter()
        .find(|s| s.id() == "liftoff")
        .unwrap()
        .parse(&std::fs::read(&paths.liftoff).unwrap())
        .unwrap();
    let race = after.profiles.iter().find(|p| p.name == "Race").unwrap();
    assert_eq!(race.rates.unwrap().axes[0].rc_rate, 127.0);
    assert_eq!(race.rates.unwrap().axes[2].srate, 75.0);
    assert_eq!(
        after
            .profiles
            .iter()
            .find(|p| p.name == "Freestyle")
            .unwrap()
            .rates
            .unwrap()
            .axes[0]
            .rc_rate,
        127.0
    );
    let unc = std::fs::read(&paths.uncrashed).unwrap();
    assert_ne!(unc, before_unc);
    assert_eq!(unc.len(), before_unc.len());

    // The old bytes are in the gear store, one backup per sim, kept.
    for (sim, bytes) in [
        ("sim-liftoff", &before_lift),
        ("sim-uncrashed", &before_unc),
    ] {
        let list = c
            .gear_backups(&BackupFilter {
                device: Some(sim.into()),
            })
            .unwrap();
        assert_eq!(list.len(), 1, "{sim}");
        let content = c
            .gear_backup_read(&BackupReadParams {
                id: list[0].id.clone(),
                path: None,
            })
            .unwrap();
        assert_eq!(content.backup.files.len(), 1);
        let one = c
            .gear_backup_read(&BackupReadParams {
                id: list[0].id.clone(),
                path: Some(content.backup.files[0].path.clone()),
            })
            .unwrap();
        if let Some(t) = one.text {
            assert_eq!(t.as_bytes(), bytes.as_slice());
        }
    }

    // Planning again: the sims hold these rates, so there is nothing to write.
    let again = c.gear_sim_sync_plan_at(&p, h.path(), &never).unwrap();
    assert!(!again.ready());
    let r = again
        .checks
        .iter()
        .find_map(|k| k.refusal.as_ref())
        .unwrap();
    assert_eq!(
        r.reason,
        "The sims already hold these rates; nothing to write."
    );
    assert_eq!(
        c.gear_sims_at(
            &quadcam_lib::core::SimsParams {
                paths: vec![three()],
                ..Default::default()
            },
            h.path(),
            &never,
        )
        .unwrap()
        .iter()
        .find(|s| s.id == "uncrashed")
        .unwrap()
        .in_sync,
        Some(true)
    );
}

#[test]
fn a_sync_refuses_while_the_sim_runs_even_if_it_started_after_the_plan() {
    let dir = tempfile::tempdir().unwrap();
    let (h, paths) = home();
    let c = core(dir.path());
    let p = params(vec![target("uncrashed", "FREE")], 0);
    let before = std::fs::read(&paths.uncrashed).unwrap();
    let plan = c.gear_sim_sync_plan_at(&p, h.path(), &never).unwrap();
    // The game starts between the plan and the click.
    let running = |n: &str| n == "Uncrashed";
    let e = c
        .sim_sync_at(&request(p.clone(), &plan, true), true, h.path(), &running)
        .unwrap_err();
    assert_eq!(refused(&e), RefusalCode::SimRunning);
    assert!(format!("{e:#}").contains("Quit Uncrashed first."));
    assert_eq!(std::fs::read(&paths.uncrashed).unwrap(), before);
    // No backup was taken for a refused write.
    assert!(c
        .gear_backups(&BackupFilter {
            device: Some("sim-uncrashed".into())
        })
        .unwrap()
        .is_empty());
}

#[test]
fn a_file_that_changed_since_the_plan_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let (h, paths) = home();
    let c = core(dir.path());
    let p = params(vec![target("uncrashed", "FREE")], 0);
    let plan = c.gear_sim_sync_plan_at(&p, h.path(), &never).unwrap();
    // The game rewrote the file (another value) after the plan.
    std::fs::write(
        &paths.uncrashed,
        gvas(&[0.6, 1.1, 0.2, 0.6, 1.1, 0.2, 0.6, 1.1, 0.0, 0.0, 0.5, 0.0]),
    )
    .unwrap();
    let e = c
        .sim_sync_at(&request(p, &plan, true), true, h.path(), &never)
        .unwrap_err();
    assert_eq!(refused(&e), RefusalCode::BeforeMismatch);
}

struct Sabotage(Mutex<Option<PathBuf>>);

impl Hooks for Sabotage {
    fn has_gui(&self) -> bool {
        true
    }
    /// The person's click: here it also makes one folder read-only, so the second write
    /// fails after the first succeeded.
    fn confirm_apply(&self, c: &StagedChange, _p: &ApplyPlan) -> anyhow::Result<()> {
        assert_eq!(c.id, "sim-sync");
        use std::os::unix::fs::PermissionsExt;
        if let Some(dir) = self.0.lock().unwrap().take() {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        }
        Ok(())
    }
}

#[test]
fn a_failed_write_puts_every_file_back() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (h, paths) = home();
    let hooks = Arc::new(Sabotage(Mutex::new(Some(
        paths.uncrashed.parent().unwrap().to_path_buf(),
    ))));
    let c = core_with(dir.path(), hooks);
    let p = params(
        vec![target("liftoff", "Race"), target("uncrashed", "FREE")],
        0,
    );
    let before_lift = std::fs::read(&paths.liftoff).unwrap();
    let plan = c.gear_sim_sync_plan_at(&p, h.path(), &never).unwrap();
    assert!(plan.ready());
    // Not from the GUI's own click: the hook runs as the person's confirm.
    let r = c
        .sim_sync_at(&request(p, &plan, true), false, h.path(), &never)
        .unwrap();
    std::fs::set_permissions(
        paths.uncrashed.parent().unwrap(),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert_eq!(r.status, ChangeStatus::Failed, "{}", r.message);
    assert!(
        r.message.contains("Every file was put back as it was."),
        "{}",
        r.message
    );
    assert!(r.steps.iter().any(|s| s.name == "Roll back"));
    assert!(r.files.is_empty());
    // Liftoff (written first) is back to its old bytes.
    assert_eq!(std::fs::read(&paths.liftoff).unwrap(), before_lift);
    // The backups stay.
    assert_eq!(
        c.gear_backups(&BackupFilter {
            device: Some("sim-liftoff".into())
        })
        .unwrap()
        .len(),
        1
    );
}

#[test]
fn a_sim_write_outside_the_temporary_folder_is_off_under_cargo() {
    // The guard every write passes: under cargo only the temp folder is writable.
    use quadcam_lib::gear::apply::sim::writes_allowed;
    assert!(!writes_allowed(
        Path::new("/Users/anyone/Library/Application Support/x"),
        true
    ));
    assert!(writes_allowed(&std::env::temp_dir().join("x"), true));
}

// ----- surfaces -----

#[test]
fn the_cli_plans_then_syncs_a_temporary_home() {
    let dir = tempfile::tempdir().unwrap();
    let _ = dir;
    let (h, paths) = home();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
            .args(args)
            .env("HOME", h.path())
            .env("QUADCAM_PHOTOS", "dry-run")
            .env_remove("QUADCAM_SIMS_RUNNING")
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };
    let quad = three();
    let quad = quad.to_str().unwrap();
    let (code, out) = run(&[
        "--json",
        "gear",
        "sims",
        quad,
        "--profile",
        "0",
        "--sync",
        "--to",
        "uncrashed:FREE",
    ]);
    assert_eq!(code, 0, "{out}");
    let j: Value = serde_json::from_str(&out).unwrap();
    let digest = j["result"]["digest"].as_str().unwrap().to_string();
    assert!(!digest.is_empty());
    assert!(j["result"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["ok"] == true));
    let before = std::fs::read(&paths.uncrashed).unwrap();

    // Without --yes: refused (exit 3), nothing written.
    let (code, out) = run(&[
        "--json",
        "gear",
        "sims",
        quad,
        "--profile",
        "0",
        "--sync",
        "--to",
        "uncrashed:FREE",
        "--digest",
        &digest,
    ]);
    assert_eq!(code, 3, "{out}");
    assert_eq!(std::fs::read(&paths.uncrashed).unwrap(), before);

    // While the game "runs": refused, exit 3.
    let out = Command::new(env!("CARGO_BIN_EXE_quadcam-cli"))
        .args([
            "--json",
            "gear",
            "sims",
            quad,
            "--profile",
            "0",
            "--sync",
            "--to",
            "uncrashed:FREE",
            "--digest",
            &digest,
            "--yes",
        ])
        .env("HOME", h.path())
        .env("QUADCAM_PHOTOS", "dry-run")
        .env("QUADCAM_SIMS_RUNNING", "Uncrashed")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(std::fs::read(&paths.uncrashed).unwrap(), before);

    let (code, out) = run(&[
        "--json",
        "gear",
        "sims",
        quad,
        "--profile",
        "0",
        "--sync",
        "--to",
        "uncrashed:FREE",
        "--digest",
        &digest,
        "--yes",
    ]);
    assert_eq!(code, 0, "{out}");
    let j: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(j["result"]["status"], "verified", "{out}");
    assert_ne!(std::fs::read(&paths.uncrashed).unwrap(), before);
}

#[test]
fn the_rows_and_the_mcp_actions_need_the_digest_and_confirm() {
    let dir = tempfile::tempdir().unwrap();
    let c = core(dir.path());
    // An empty target list plans nothing and touches no file.
    let v = c
        .dispatch(
            "gear_sim_sync_plan",
            json!({"sims": [], "paths": [three()]}),
        )
        .unwrap();
    assert_eq!(v["change"], "sim-sync");
    assert_eq!(v["digest"], "");

    let mut s = quadcam_lib::mcp::Server::new(quadcam_lib::mcp::LocalBackend(c));
    let r = s.call_tool(
        "quadcam_gear",
        json!({"action": "sim_sync_plan", "paths": [three()], "sim_targets": ["flightgear"]}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("REFUSED Sim flightgear"), "{text}");
    assert!(!text.contains("To write:"), "{text}");
    let r = s.call_tool(
        "quadcam_gear_apply",
        json!({"action": "sim_sync", "sim_targets": ["all"]}),
    );
    assert_eq!(r["isError"], true);
    let r = s.call_tool(
        "quadcam_gear_apply",
        json!({"action": "sim_sync", "digest": "x", "sim_targets": ["all"]}),
    );
    assert_eq!(r["isError"], true);
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("confirm=true"));
}

// ----- the restore -----

use quadcam_lib::core::{SimRestoreParams, SimRestoreRequest};

fn restore_params(sim: &str) -> SimRestoreParams {
    SimRestoreParams {
        sim: sim.into(),
        backup: None,
    }
}

fn restore_request(p: &SimRestoreParams, plan: &ApplyPlan) -> SimRestoreRequest {
    SimRestoreRequest {
        params: p.clone(),
        digest: plan.digest.clone(),
        confirm: true,
    }
}

/// Syncs Liftoff's Race profile in a fresh home; returns the file's bytes before.
fn synced(c: &Core, h: &Path, paths: &Paths) -> Vec<u8> {
    let before = std::fs::read(&paths.liftoff).unwrap();
    let p = params(vec![target("liftoff", "Race")], 0);
    let plan = c.gear_sim_sync_plan_at(&p, h, &never).unwrap();
    let r = c
        .sim_sync_at(&request(p, &plan, true), true, h, &never)
        .unwrap();
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    assert_ne!(std::fs::read(&paths.liftoff).unwrap(), before);
    before
}

#[test]
fn a_restore_without_a_backup_or_for_an_unknown_sim_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let (h, _) = home();
    let c = core(dir.path());
    let plan = c
        .gear_sim_restore_plan_at(&restore_params("liftoff"), h.path(), &never)
        .unwrap();
    assert!(!plan.ready());
    assert_eq!(
        plan.checks[0].refusal.as_ref().unwrap().code,
        RefusalCode::NoBackup
    );
    assert!(plan.digest.is_empty());
    let e = c
        .gear_sim_restore_plan_at(&restore_params("nosuchsim"), h.path(), &never)
        .unwrap_err();
    assert!(format!("{e:#}").contains("not a sim QuadCam knows"));
}

#[test]
fn a_restore_puts_the_synced_file_back_and_can_be_undone() {
    let dir = tempfile::tempdir().unwrap();
    let (h, paths) = home();
    let c = core(dir.path());
    let before = synced(&c, h.path(), &paths);
    let after_sync = std::fs::read(&paths.liftoff).unwrap();

    let p = restore_params("liftoff");
    let plan = c.gear_sim_restore_plan_at(&p, h.path(), &never).unwrap();
    assert!(plan.ready(), "{:?}", plan.checks);
    assert!(!plan.digest.is_empty());
    assert!(plan
        .warnings
        .iter()
        .any(|w| w.contains("Changes made in the game")));
    let text = serde_json::to_string(&plan.diff).unwrap();
    assert!(text.contains("Profile Race"), "{text}");
    assert_eq!(
        std::fs::read(&paths.liftoff).unwrap(),
        after_sync,
        "a plan writes nothing"
    );

    // No confirm, no write; a wrong digest refuses.
    let mut no = restore_request(&p, &plan);
    no.confirm = false;
    let e = c.sim_restore_at(&no, true, h.path(), &never).unwrap_err();
    assert!(format!("{e:#}").contains("confirm=true"));
    let mut bad = restore_request(&p, &plan);
    bad.digest = "nope".into();
    let e = c.sim_restore_at(&bad, true, h.path(), &never).unwrap_err();
    assert_eq!(refused(&e), RefusalCode::BeforeMismatch);
    assert_eq!(std::fs::read(&paths.liftoff).unwrap(), after_sync);

    let r = c
        .sim_restore_at(&restore_request(&p, &plan), true, h.path(), &never)
        .unwrap();
    assert_eq!(r.status, ChangeStatus::Verified, "{}", r.message);
    assert_eq!(
        std::fs::read(&paths.liftoff).unwrap(),
        before,
        "the original bytes are back"
    );
    for name in ["Back up Liftoff", "Write Liftoff", "Read back Liftoff"] {
        assert!(r.steps.iter().any(|s| s.name == name), "{name}");
    }

    // The file as it was just before the restore is kept: restoring again undoes it.
    let list = c
        .gear_backups(&BackupFilter {
            device: Some("sim-liftoff".into()),
        })
        .unwrap();
    assert_eq!(list.len(), 2);
    let undo = c.gear_sim_restore_plan_at(&p, h.path(), &never).unwrap();
    assert!(undo.ready());
    let r = c
        .sim_restore_at(&restore_request(&p, &undo), true, h.path(), &never)
        .unwrap();
    assert_eq!(r.status, ChangeStatus::Verified);
    assert_eq!(std::fs::read(&paths.liftoff).unwrap(), after_sync);
}

#[test]
fn a_restore_names_its_backup_and_refuses_one_that_is_not_the_sims() {
    let dir = tempfile::tempdir().unwrap();
    let (h, paths) = home();
    let c = core(dir.path());
    let before = synced(&c, h.path(), &paths);
    let list = c
        .gear_backups(&BackupFilter {
            device: Some("sim-liftoff".into()),
        })
        .unwrap();
    let p = SimRestoreParams {
        sim: "liftoff".into(),
        backup: Some(list[0].id.clone()),
    };
    let plan = c.gear_sim_restore_plan_at(&p, h.path(), &never).unwrap();
    assert!(plan.ready());
    c.sim_restore_at(&restore_request(&p, &plan), true, h.path(), &never)
        .unwrap();
    assert_eq!(std::fs::read(&paths.liftoff).unwrap(), before);
    // Now the file equals that backup: nothing to restore.
    let again = c.gear_sim_restore_plan_at(&p, h.path(), &never).unwrap();
    assert!(again
        .checks
        .iter()
        .any(|k| k.name == "Something differs" && !k.ok));
    // A backup of another device is not this sim's.
    let other = SimRestoreParams {
        sim: "uncrashed".into(),
        backup: Some(list[0].id.clone()),
    };
    let plan = c
        .gear_sim_restore_plan_at(&other, h.path(), &never)
        .unwrap();
    assert!(!plan.ready());
}

#[test]
fn a_restore_refuses_while_the_sim_runs_or_when_the_file_changed() {
    let dir = tempfile::tempdir().unwrap();
    let (h, paths) = home();
    let c = core(dir.path());
    synced(&c, h.path(), &paths);
    let p = restore_params("liftoff");
    let running = |n: &str| n == sim_of("liftoff").process();
    let plan = c.gear_sim_restore_plan_at(&p, h.path(), &running).unwrap();
    assert!(plan
        .checks
        .iter()
        .any(|k| !k.ok && k.refusal.as_ref().unwrap().code == RefusalCode::SimRunning));

    let plan = c.gear_sim_restore_plan_at(&p, h.path(), &never).unwrap();
    let e = c
        .sim_restore_at(&restore_request(&p, &plan), true, h.path(), &running)
        .unwrap_err();
    assert_eq!(refused(&e), RefusalCode::SimRunning);

    // The file changed after the plan.
    let mut bytes = std::fs::read(&paths.liftoff).unwrap();
    bytes.extend_from_slice(b"\n<!-- edited -->");
    std::fs::write(&paths.liftoff, &bytes).unwrap();
    let e = c
        .sim_restore_at(&restore_request(&p, &plan), true, h.path(), &never)
        .unwrap_err();
    assert_eq!(refused(&e), RefusalCode::BeforeMismatch);
    assert_eq!(std::fs::read(&paths.liftoff).unwrap(), bytes);

    // The file is gone: the plan refuses.
    std::fs::remove_file(&paths.liftoff).unwrap();
    let plan = c.gear_sim_restore_plan_at(&p, h.path(), &never).unwrap();
    assert!(!plan.ready());
}
