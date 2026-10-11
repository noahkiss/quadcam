//! The sim writer's plan (design 8, 6.6): the quad's rate profile onto a profile of each
//! sim, as a plan with checks, a diff, warnings and a digest. Nothing here writes a sim
//! file except `write_atomic`, which `core/sim_sync.rs` calls after the checks, the confirm
//! and the backup.
//!
//! A sim takes the Betaflight model, so a quad on Actual or Quick is fitted first. Only
//! values that differ are replaced; every other byte of the file stays as it was. A sim
//! without a throttle curve gets rates only, and the plan says so.

use super::{check, pass, Check};
use crate::gear::blobs;
use crate::gear::model::{
    ApplyPlan, DiffItem, DiffLine, Identity, LineOp, Refusal, RefusalCode, StagedChange,
};
use crate::gear::rates::{self, profile_of, RateProfileView, Rates, ThrottleCurve};
use crate::gear::sims::{self, Sim, SimFile, SimProfile};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};

/// The id the apply sheet and an agent's confirm request know a sim sync by.
pub const CHANGE_ID: &str = "sim-sync";
/// The `device` of a sim sync's report and its stand-in change.
pub const DEVICE: &str = "sims";

/// One profile of one sim to write.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SimTarget {
    /// A sim id (`liftoff`, `micro`, `uncrashed`, `zone`) or `all` for every sim found.
    pub sim: String,
    /// The file, as `gear sims` shows its path or its file name. Needed only when the sim
    /// has several files and the profile name does not pick one.
    #[serde(default)]
    pub file: Option<String>,
    /// The sim's profile to overwrite. Default: the one named like the quad's rate profile.
    #[serde(default)]
    pub profile: Option<String>,
}

/// `gear_sim_sync_plan`: which sim profiles take which rate profile of the quad.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct SimSyncParams {
    pub sims: Vec<SimTarget>,
    /// The quad, as for `gear_sims`: `dump all` or `diff all` files, a saved device's latest
    /// backup, or a backup id.
    #[serde(default)]
    pub paths: Vec<PathBuf>,
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub backup: Option<String>,
    /// The quad's rate profile index; default the one in use.
    #[serde(default)]
    pub profile: Option<u8>,
}

/// `gear_sim_sync`: the same params, the plan's digest and the confirm.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct SimSyncRequest {
    #[serde(flatten)]
    pub params: SimSyncParams,
    pub digest: String,
    #[serde(default)]
    pub confirm: bool,
}

/// One profile a write must leave holding the quad's rates.
#[derive(Debug, Clone)]
pub struct SimWant {
    pub profile: String,
    /// What the profile must read back as.
    pub want: Rates,
    pub want_throttle: Option<ThrottleCurve>,
}

/// One file the sync will write: every target profile in that file, in one write.
#[derive(Debug, Clone)]
pub struct SimWrite {
    pub sim: String,
    pub name: String,
    pub path: PathBuf,
    /// The path with `~` for the home folder.
    pub shown: String,
    /// The path from the home folder: the name inside the backup.
    pub rel: String,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
    /// The profiles the file must hold after the write, in plan order.
    pub profiles: Vec<SimWant>,
}

impl SimWrite {
    /// The profile names, comma separated.
    pub fn profile_names(&self) -> String {
        self.profiles
            .iter()
            .map(|p| p.profile.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// A plan and the writes it stands for (empty unless every check passed).
#[derive(Debug, Clone)]
pub struct SimPlanned {
    pub plan: ApplyPlan,
    pub writes: Vec<SimWrite>,
}

/// The backup device id of a sim: `sim-liftoff`.
pub fn backup_device(sim: &str) -> String {
    format!("sim-{sim}")
}

/// XXH64 over each file's path and the hashes of its bytes before and after.
pub fn digest(writes: &[SimWrite]) -> String {
    let mut text = String::from("sims");
    for w in writes {
        text.push_str(&format!(
            "\n{}|{}|{}|{}",
            w.rel,
            w.profile_names(),
            blobs::hash(&w.before),
            blobs::hash(&w.after)
        ));
    }
    blobs::hash(text.as_bytes())
}

/// The stand-in change an agent's confirm request and the apply sheet carry.
pub fn pseudo_change(plan: &ApplyPlan) -> StagedChange {
    StagedChange {
        id: CHANGE_ID.into(),
        device: DEVICE.into(),
        title: "Sync sims".into(),
        status: crate::gear::model::ChangeStatus::Ready,
        edits: Vec::new(),
        base_backup: String::new(),
        editor: crate::session::Editor::User,
        note: plan.warnings.join(" "),
        order: 0,
        history: Vec::new(),
        reverts: None,
    }
}

pub(crate) fn fail(name: &str, code: RefusalCode, reason: impl Into<String>) -> Check {
    check(name, Err(Refusal::new(code, reason)))
}

/// True when this process may write a sim file at `path`. A process started by cargo only
/// writes under the temporary folder: never a real player's files.
pub fn writes_allowed(path: &Path, under_cargo: bool) -> bool {
    if !under_cargo {
        return true;
    }
    let tmp = std::env::temp_dir();
    let tmp = tmp.canonicalize().unwrap_or(tmp);
    // The file may not exist yet: resolve its folder.
    let p = path.canonicalize().unwrap_or_else(|_| {
        match (
            path.parent().and_then(|d| d.canonicalize().ok()),
            path.file_name(),
        ) {
            (Some(d), Some(n)) => d.join(n),
            _ => path.to_path_buf(),
        }
    });
    p.starts_with(tmp)
}

#[cfg(target_os = "macos")]
fn full_fsync(f: &std::fs::File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    extern "C" {
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    }
    const F_FULLFSYNC: i32 = 51;
    // SAFETY: fcntl on an open descriptor this File owns; F_FULLFSYNC takes no argument.
    let r = unsafe { fcntl(f.as_raw_fd(), F_FULLFSYNC) };
    if r == -1 {
        f.sync_all()
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
fn full_fsync(f: &std::fs::File) -> std::io::Result<()> {
    f.sync_all()
}

/// Writes a sim file: a temporary name beside it with the file's permissions, the bytes, a
/// full sync, a rename, then a read-back. True when the bytes read back equal.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<bool> {
    use std::io::Write;
    let dir = path.parent().context("no parent folder")?;
    let name = path
        .file_name()
        .context("no file name")?
        .to_string_lossy()
        .to_string();
    let tmp = dir.join(format!(".{name}.quadcam-tmp"));
    let perms = std::fs::metadata(path).ok().map(|m| m.permissions());
    let written = (|| -> Result<()> {
        let mut f =
            std::fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(bytes)?;
        if let Some(p) = &perms {
            f.set_permissions(p.clone())?;
        }
        full_fsync(&f)?;
        Ok(())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow::Error::from(e).context(format!("renaming onto {}", path.display())));
    }
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(std::fs::read(path)? == bytes)
}

/// True when every profile of `w` in the file now holds its wanted rates (and throttle when
/// it has one).
pub fn reads_as(sim: &dyn Sim, path: &Path, raw: &[u8], w: &SimWrite) -> bool {
    let Ok(f) = sim.parse_file(raw, path) else {
        return false;
    };
    w.profiles.iter().all(|want| {
        f.profiles
            .iter()
            .find(|p| p.name == want.profile)
            .is_some_and(|p| holds(p, &want.want, want.want_throttle.as_ref()))
    })
}

fn holds(p: &SimProfile, want: &Rates, throttle: Option<&ThrottleCurve>) -> bool {
    let Some(r) = p.rates else { return false };
    let same_rates = r.axes.iter().zip(&want.axes).all(|(a, b)| {
        a.rc_rate.round() == b.rc_rate.round()
            && a.srate.round() == b.srate.round()
            && a.expo.round() == b.expo.round()
    });
    let same_throttle = match (p.throttle, throttle) {
        (Some(a), Some(b)) => a.mid.round() == b.mid.round() && a.expo.round() == b.expo.round(),
        _ => true,
    };
    same_rates && same_throttle
}

pub(crate) fn tilde(home: &Path, p: &Path) -> String {
    match p.strip_prefix(home) {
        Ok(r) => format!("~/{}", r.display()),
        Err(_) => p.display().to_string(),
    }
}

/// The target's file and profile in `sim`, or why there is none.
fn find(
    sim: &dyn Sim,
    home: &Path,
    t: &SimTarget,
    quad_name: Option<&str>,
) -> Result<(PathBuf, Vec<u8>, SimFile, usize)> {
    let mut files = sim.files(home);
    if let Some(f) = t.file.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        files.retain(|p| tilde(home, p) == f || p.file_name().is_some_and(|n| n == f));
        if files.is_empty() {
            anyhow::bail!("{} has no file {f:?}", sim.name());
        }
    }
    if files.is_empty() {
        anyhow::bail!("{} has no rate file on this Mac", sim.name());
    }
    let want = t
        .profile
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .or(quad_name);
    let mut hits: Vec<(PathBuf, Vec<u8>, SimFile, usize)> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for path in files {
        let raw =
            std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?;
        let file = sim.parse_file(&raw, &path)?;
        for (i, p) in file.profiles.iter().enumerate() {
            names.push(p.name.clone());
            if want.is_none_or(|w| p.name.eq_ignore_ascii_case(w)) {
                hits.push((path.clone(), raw.clone(), file.clone(), i));
            }
        }
    }
    match (hits.len(), want) {
        (1, _) => Ok(hits.remove(0)),
        (0, Some(w)) => anyhow::bail!(
            "{} has no profile named {w:?}. It has: {}.",
            sim.name(),
            if names.is_empty() {
                "none".to_string()
            } else {
                names.join(", ")
            }
        ),
        (0, None) => anyhow::bail!("{} has no profile to write", sim.name()),
        (_, Some(w)) => anyhow::bail!(
            "{} has several profiles named {w:?}; name the file as well.",
            sim.name()
        ),
        (_, None) => anyhow::bail!(
            "Name the {} profile to overwrite. It has: {}.",
            sim.name(),
            names.join(", ")
        ),
    }
}

pub(crate) fn can_write(path: &Path) -> bool {
    std::fs::OpenOptions::new().write(true).open(path).is_ok()
        && path
            .parent()
            .is_some_and(|d| std::fs::metadata(d).is_ok_and(|m| !m.permissions().readonly()))
}

const AXIS_NAMES: [&str; 3] = ["Roll", "Pitch", "Yaw"];

/// The diff of one profile: the values that change, old line out, new line in.
pub(crate) fn diff_lines(
    have: &Rates,
    want: &Rates,
    ht: Option<ThrottleCurve>,
    wt: Option<&ThrottleCurve>,
) -> Vec<DiffLine> {
    let mut out = Vec::new();
    let mut change = |what: String, a: f64, b: f64| {
        if a.round() != b.round() {
            out.push(DiffLine {
                op: LineOp::Remove,
                text: format!("{what} {}", a.round()),
            });
            out.push(DiffLine {
                op: LineOp::Add,
                text: format!("{what} {}", b.round()),
            });
        }
    };
    for (i, name) in AXIS_NAMES.iter().enumerate() {
        change(
            format!("{name} RC rate"),
            have.axes[i].rc_rate,
            want.axes[i].rc_rate,
        );
        change(
            format!("{name} super rate"),
            have.axes[i].srate,
            want.axes[i].srate,
        );
        change(format!("{name} expo"), have.axes[i].expo, want.axes[i].expo);
    }
    if let (Some(h), Some(w)) = (ht, wt) {
        change("Throttle mid".into(), h.mid, w.mid);
        change("Throttle expo".into(), h.expo, w.expo);
    }
    out
}

/// Plans the sync of `quad` (a rate profile of the quad) onto the targets. `running` says
/// whether a process runs. Reads the sim files; writes nothing.
pub fn plan(
    quad: &RateProfileView,
    targets: &[SimTarget],
    home: &Path,
    running: &dyn Fn(&str) -> bool,
) -> SimPlanned {
    let q = profile_of(quad);
    let (bf, fits) = rates::to_betaflight(&q);
    let mut checks: Vec<Check> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut diff: Vec<DiffItem> = Vec::new();
    let mut writes: Vec<SimWrite> = Vec::new();

    let label = quad
        .name
        .clone()
        .unwrap_or_else(|| format!("rate profile {}", quad.index + 1));
    let fit_error = fits.iter().map(|f| f.max_diff).fold(0.0, f64::max);
    if quad.rates_type != "betaflight" {
        warnings.push(format!(
            "{label} uses the {} model; sims take Betaflight rates, so QuadCam fitted them. The largest gap is {:.0} deg/s.",
            quad.rates_type, fit_error
        ));
    }

    // Every sim named once; `all` is every enabled sim with a file.
    let mut picked: Vec<(&'static dyn Sim, SimTarget)> = Vec::new();
    for t in targets {
        let id = t.sim.trim().to_ascii_lowercase();
        if id == "all" {
            for s in sims::all() {
                if s.enabled() && !s.files(home).is_empty() {
                    picked.push((
                        s,
                        SimTarget {
                            sim: s.id().into(),
                            ..t.clone()
                        },
                    ));
                }
            }
        } else if let Some(s) = sims::all().into_iter().find(|s| s.id() == id) {
            picked.push((
                s,
                SimTarget {
                    sim: id,
                    ..t.clone()
                },
            ));
        } else {
            checks.push(fail(
                &format!("Sim {}", t.sim),
                RefusalCode::ShapeUnknown,
                format!(
                    "{:?} is not a sim QuadCam knows (liftoff, micro, uncrashed, zone, or all).",
                    t.sim
                ),
            ));
        }
    }
    if picked.is_empty() && checks.is_empty() {
        checks.push(fail(
            "Sims picked",
            RefusalCode::Incompatible,
            "Name the sims to sync, or `all`.",
        ));
    }

    for (sim, t) in &picked {
        let who = sim.name();
        if !sim.enabled() {
            checks.push(fail(
                &format!("Adapter on ({who})"),
                RefusalCode::Disabled,
                format!("The {who} adapter is off."),
            ));
            continue;
        }
        let (path, raw, mut file, idx) = match find(*sim, home, t, quad.name.as_deref()) {
            Ok(x) => x,
            Err(e) => {
                checks.push(fail(
                    &format!("File understood ({who})"),
                    RefusalCode::ShapeUnknown,
                    format!("{e:#}"),
                ));
                continue;
            }
        };
        // A second target in a file already planned edits that plan's bytes, so one write
        // carries both profiles.
        let earlier = writes.iter().position(|w| w.path == path);
        if let Some(i) = earlier {
            match sim.parse_file(&writes[i].after, &path) {
                Ok(f) => file = f,
                Err(e) => {
                    checks.push(fail(
                        &format!("File understood ({who})"),
                        RefusalCode::ShapeUnknown,
                        format!("{e:#}"),
                    ));
                    continue;
                }
            }
        }
        let shown = tilde(home, &path);
        let prof = &file.profiles[idx];
        checks.push(if running(sim.process()) {
            fail(
                &format!("Sim closed ({who})"),
                RefusalCode::SimRunning,
                format!("Quit {who} first."),
            )
        } else {
            pass(&format!("Sim closed ({who})"))
        });
        let Some(have) = prof.rates else {
            checks.push(fail(
                &format!("File understood ({who})"),
                RefusalCode::ShapeUnknown,
                format!(
                    "{} in {shown} holds {}; QuadCam writes Betaflight rates only.",
                    prof.name,
                    prof.note.as_deref().unwrap_or("rates it cannot read")
                ),
            ));
            continue;
        };
        checks.push(pass(&format!("File understood ({who})")));
        let throttle = prof.throttle.is_some().then_some(&bf.throttle);
        // The new bytes must parse and read back as wanted. Every byte outside the replaced
        // spans stays as it was (`Doc::replaced`); the adapters' tests prove the encoder
        // writes each fixture's number format byte for byte.
        let after = sims::write_profile(*sim, &file, idx, &bf.rates, throttle)
            .ok()
            .filter(|after| {
                sim.parse_file(after.render(), &path)
                    .ok()
                    .and_then(|f| f.profiles.get(idx).cloned())
                    .is_some_and(|p| holds(&p, &bf.rates, throttle))
            });
        let Some(after) = after else {
            checks.push(fail(
                &format!("Reads back as written ({who})"),
                RefusalCode::RoundTrip,
                format!("QuadCam cannot rewrite {shown} reliably; nothing was written."),
            ));
            continue;
        };
        checks.push(pass(&format!("Reads back as written ({who})")));
        checks.push(if can_write(&path) {
            pass(&format!("Writable ({who})"))
        } else {
            fail(
                &format!("Writable ({who})"),
                RefusalCode::NotWritable,
                format!("{shown} cannot be written: check its permissions and its folder."),
            )
        });

        let lines = diff_lines(&have, &bf.rates, prof.throttle, throttle);
        let mut shown_lines = vec![DiffLine {
            op: LineOp::Same,
            text: format!("Profile {}", prof.name),
        }];
        if lines.is_empty() {
            shown_lines.push(DiffLine {
                op: LineOp::Same,
                text: "already holds these rates".into(),
            });
        }
        shown_lines.extend(lines);
        diff.push(DiffItem::Lines {
            label: format!("{who}: {shown}"),
            lines: shown_lines,
        });
        if prof.throttle.is_none() {
            warnings.push(format!(
                "{who} has no throttle curve: only rates are written."
            ));
        }
        if !sim.write_verified() {
            warnings.push(format!(
                "Unverified: QuadCam read {who}'s file shape from a real install but has not yet seen the game load a file QuadCam wrote. Check the game's rates screen after the sync; the backup holds the old file."
            ));
        }
        if sim.id() == "micro" {
            warnings
                .push("Liftoff: Micro Drones keeps this file inside the game's app bundle.".into());
        }
        let rel = path
            .strip_prefix(home)
            .map(|r| r.display().to_string())
            .unwrap_or_else(|_| path.display().to_string());
        let want = SimWant {
            profile: prof.name.clone(),
            want: bf.rates,
            want_throttle: throttle.copied(),
        };
        if let Some(i) = earlier {
            let w = &mut writes[i];
            w.after = after.render().to_vec();
            w.profiles.push(want);
        } else {
            writes.push(SimWrite {
                sim: sim.id().into(),
                name: who.into(),
                path,
                shown,
                rel,
                before: raw,
                after: after.render().to_vec(),
                profiles: vec![want],
            });
        }
    }

    if checks.iter().all(|c| c.ok)
        && !writes.is_empty()
        && writes.iter().all(|w| w.before == w.after)
    {
        checks.push(fail(
            "Something differs",
            RefusalCode::Incompatible,
            "The sims already hold these rates; nothing to write.",
        ));
    }
    let ready = checks.iter().all(|c| c.ok);
    let digest = if ready {
        digest(&writes)
    } else {
        String::new()
    };
    SimPlanned {
        plan: ApplyPlan {
            change: CHANGE_ID.into(),
            device: Identity::default(),
            checks,
            diff,
            digest,
            warnings,
        },
        writes: if ready { writes } else { Vec::new() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_stay_under_the_temporary_folder_under_cargo() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(writes_allowed(&tmp.path().join("x"), true));
        assert!(!writes_allowed(Path::new("/Users/someone/Library/x"), true));
        assert!(writes_allowed(Path::new("/Users/someone/Library/x"), false));
    }

    #[test]
    fn an_atomic_write_keeps_the_permissions_and_reads_back() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.sav");
        std::fs::write(&p, b"old").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(write_atomic(&p, b"new bytes").unwrap());
        assert_eq!(std::fs::read(&p).unwrap(), b"new bytes");
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert!(!d.path().join(".a.sav.quadcam-tmp").exists());
    }
}
