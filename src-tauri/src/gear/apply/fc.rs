//! The FC's plan and checks (design 8.2), pure over what the core read. `plan` writes
//! nothing and reads nothing; `core/apply.rs` gathers the inputs and runs the job.

use super::{check, pass};
use crate::gear::bf::dump::Config;
use crate::gear::bf::{self};
use crate::gear::blobs;
use crate::gear::changes::{render_fc, FcRender, SetRef};
use crate::gear::model::{
    ApplyPlan, Backup, Check, DiffItem, Edit, Identity, Refusal, RefusalCode, StagedChange,
};

/// An FC plugged in, as the plan sees it.
#[derive(Debug, Clone, Default)]
pub struct Cand {
    pub port: String,
    /// None until an identify read it (or when another program holds the port).
    pub id: Option<String>,
    pub identity: Identity,
    /// A battery is in (more than 1 V on the lead).
    pub battery: bool,
    /// Seconds left on the USB timer; Some(0) is past the limit.
    pub remaining_s: Option<u32>,
    /// Other processes that have the port open.
    pub holders: Vec<(u32, String)>,
}

/// The device's latest backup and its `dump all`.
#[derive(Debug, Clone)]
pub struct Base {
    pub backup: Backup,
    pub dump: String,
}

/// A plan, with the port it would use and the lines it would send.
#[derive(Debug, Clone)]
pub struct Planned {
    pub plan: ApplyPlan,
    pub port: Option<String>,
    pub render: FcRender,
}

/// XXH64 over the device id, the dump's identity, the before state and the lines.
pub fn digest(device: &str, base: &Config, render: &FcRender) -> String {
    let i = base.identity();
    let text = format!(
        "fc|{device}|{}|{}|{}|{}\n{}\n--\n{}",
        i.board.unwrap_or_default(),
        i.firmware.unwrap_or_default(),
        i.version.unwrap_or_default(),
        i.build.unwrap_or_default(),
        render.before.join("\n"),
        render.lines.join("\n")
    );
    blobs::hash(text.as_bytes())
}

/// The refusal for a `set` the FC's `get` answer does not allow, else None. The answer
/// holds `Allowed range: LO - HI` or `Allowed values: A, B`; any other shape passes.
pub fn range_problem(set: &SetRef, reply: &str) -> Option<Refusal> {
    let name = &set.name;
    if reply.contains("###ERROR") || reply.contains("Invalid") {
        return Some(Refusal::new(
            RefusalCode::BadSetting,
            format!("`{name}` is not a setting on this FC."),
        ));
    }
    for l in reply.lines() {
        let l = l.trim();
        if let Some(r) = l.strip_prefix("Allowed range:") {
            let (lo, hi) = r.split_once(" - ")?;
            let (lo, hi) = (
                lo.trim().parse::<i64>().ok()?,
                hi.trim().parse::<i64>().ok()?,
            );
            return match set.value.parse::<i64>() {
                Ok(v) if (lo..=hi).contains(&v) => None,
                _ => Some(Refusal::new(
                    RefusalCode::BadSetting,
                    format!("`{name}` takes {lo}-{hi}."),
                )),
            };
        }
        if let Some(r) = l.strip_prefix("Allowed values:") {
            let allowed: Vec<&str> = r.split(',').map(str::trim).collect();
            return if allowed.iter().any(|a| a.eq_ignore_ascii_case(&set.value)) {
                None
            } else {
                Some(Refusal::new(
                    RefusalCode::BadSetting,
                    format!("`{name}` takes {}.", allowed.join(", ")),
                ))
            };
        }
    }
    None
}

/// Builds the plan for an FC change. `edits` are the change's edits with restores already
/// resolved to lines. Every guard that can run without touching the FC runs here.
pub fn plan(
    change: &StagedChange,
    edits: &[Edit],
    base: Option<&Base>,
    cands: &[Cand],
    port: Option<&str>,
) -> Planned {
    let cfg = base.map(|b| Config::parse(&b.dump));
    let render = render_fc(edits, cfg.as_ref());
    let mut checks: Vec<Check> = Vec::new();

    // Which FC: the one with the change's id, else the one named, else the only one.
    let pool: Vec<&Cand> = match port {
        Some(p) => cands.iter().filter(|c| c.port == p).collect(),
        None => cands.iter().collect(),
    };
    let known: Vec<&&Cand> = pool
        .iter()
        .filter(|c| c.id.as_deref() == Some(change.device.as_str()))
        .collect();
    let unknown: Vec<&&Cand> = pool.iter().filter(|c| c.id.is_none()).collect();
    let (chosen, found): (Option<&Cand>, Result<(), Refusal>) = if pool.is_empty() {
        (
            None,
            Err(Refusal::new(
                RefusalCode::NoDevice,
                "No FC is plugged in. Plug USB in before the battery.",
            )),
        )
    } else if known.len() == 1 {
        (Some(*known[0]), Ok(()))
    } else if known.len() > 1 || (known.is_empty() && unknown.len() > 1) {
        (
            None,
            Err(Refusal::new(
                RefusalCode::SeveralDevices,
                format!(
                    "{} FCs are connected; pick one with a port: {}.",
                    pool.len(),
                    pool.iter()
                        .map(|c| c.port.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )),
        )
    } else if known.is_empty() && unknown.len() == 1 {
        (Some(*unknown[0]), Ok(()))
    } else {
        (None, Ok(()))
    };
    checks.push(check("One FC plugged in", found));

    checks.push(check(
        "Same FC as planned",
        match chosen {
            Some(c) if c.id.is_some() && c.id.as_deref() != Some(change.device.as_str()) => {
                Err(Refusal::new(
                    RefusalCode::DeviceChanged,
                    "This is not the FC the change was planned for.",
                ))
            }
            None if !pool.is_empty() && known.is_empty() && unknown.is_empty() => {
                Err(Refusal::new(
                    RefusalCode::DeviceChanged,
                    "This is not the FC the change was planned for.",
                ))
            }
            _ => Ok(()),
        },
    ));

    let identity = chosen
        .filter(|c| c.identity.board.is_some())
        .map(|c| c.identity.clone())
        .or_else(|| base.map(|b| b.backup.identity.clone()))
        .unwrap_or_default();
    checks.push(check(
        "Known board and version",
        if identity.board.is_none() && identity.version.is_none() {
            Err(Refusal::new(
                RefusalCode::UnknownBoard,
                "QuadCam has not read this FC's board yet.",
            ))
        } else {
            bf::writable(&identity)
        },
    ));

    checks.push(check(
        "Backup to compare with",
        if base.is_some() {
            Ok(())
        } else {
            Err(Refusal::new(
                RefusalCode::NoBackup,
                "This FC has no backup yet. Back it up first; the plan compares with it.",
            ))
        },
    ));

    let lines_ok = render
        .problems
        .iter()
        .find(|p| p.code == RefusalCode::ShapeUnknown)
        .cloned();
    let empty = render.lines.is_empty() && render.problems.is_empty();
    checks.push(check(
        "Lines understood",
        lines_ok.map_or(
            if empty {
                Err(Refusal::new(
                    RefusalCode::ShapeUnknown,
                    "No line changes anything; the change is empty or the FC already matches.",
                ))
            } else {
                Ok(())
            },
            Err,
        ),
    ));
    let setting = render
        .problems
        .iter()
        .find(|p| p.code == RefusalCode::BadSetting)
        .cloned();
    checks.push(check("Settings exist", setting.map_or(Ok(()), Err)));

    checks.push(check(
        "Port free",
        match chosen.and_then(|c| c.holders.first().map(|h| (c, h))) {
            Some((c, (_, name))) => Err(Refusal::new(
                RefusalCode::PortBusy,
                format!(
                    "{} is open in {name}. Close it there; QuadCam does not share a port.",
                    c.port
                ),
            )),
            None => Ok(()),
        },
    ));

    checks.push(match chosen {
        Some(c) if c.battery && c.remaining_s == Some(0) => check(
            "USB heat",
            Err(Refusal::new(
                RefusalCode::UsbHeat,
                "This FC has run on USB with its battery in past its limit. Unplug the battery and let it cool.",
            )),
        ),
        Some(c) if c.battery => pass(&format!(
            "USB heat: battery in{}",
            c.remaining_s
                .map(|s| format!(", {} min left", s.div_ceil(60)))
                .unwrap_or_default()
        )),
        _ => pass("USB heat"),
    });

    let diff = vec![DiffItem::Lines {
        label: if change.title.is_empty() {
            "FC settings".into()
        } else {
            change.title.clone()
        },
        lines: render.diff.clone(),
    }];
    let digest = cfg
        .as_ref()
        .map(|c| digest(&change.device, c, &render))
        .unwrap_or_default();
    Planned {
        plan: ApplyPlan {
            change: change.id.clone(),
            device: identity,
            checks,
            diff,
            digest,
            warnings: Vec::new(),
        },
        port: chosen.map(|c| c.port.clone()),
        render,
    }
}
