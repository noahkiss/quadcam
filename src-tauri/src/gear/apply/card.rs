//! The card's plan and checks (design 8.2), pure over what the core read. `plan` writes
//! nothing; `core/apply_card.rs` mounts the card, gathers the inputs and runs the job.
//!
//! The checks, in order: one card plugged in, the same card as planned (its id), known
//! version, the edits understood (shape, round trip, model name, values, selected model
//! kept; all from `Card::plan`), the card check state, nothing else writing, and not a
//! Read first change. The digest covers the card's id, board and version, and for every
//! file the plan touches its path and the hashes of the bytes before and after.

use super::check;
use crate::gear::blobs;
use crate::gear::edgetx::card::{
    inside_card, with_timeout, write_file_atomic, Card, CardPlan, Stuck, WriteOptions,
};
use crate::gear::model::{
    ApplyPlan, Check, Connected, Edit, Identity, Refusal, RefusalCode, StagedChange,
};
use std::path::{Path, PathBuf};

/// A radio card mounted now, as the plan sees it.
#[derive(Debug, Clone)]
pub struct CardCand {
    pub connected: Connected,
    pub root: PathBuf,
}

/// What the core knows about the card besides its files.
#[derive(Debug, Clone, Default)]
pub struct CardFacts {
    /// The last card check failed: its reason.
    pub check_failed: Option<String>,
    /// A job already runs on the card's link: its step.
    pub busy: Option<String>,
}

/// A card plan, with the card it would write to.
#[derive(Debug, Clone)]
pub struct CardPlanned {
    pub plan: ApplyPlan,
    pub cand: Option<CardCand>,
    /// The files the plan writes; None when a check stopped it first.
    pub files: Option<CardPlan>,
    /// Things to know that do not refuse.
    pub warnings: Vec<String>,
}

/// What a change asks of the card: engine edits, or the files of a restore.
#[derive(Debug, Clone)]
pub enum CardWork {
    Edits(Vec<Edit>),
    /// Paths and the bytes they must read as (None: absent).
    Files(Vec<(String, Option<Vec<u8>>)>),
}

/// XXH64 over the card's id, its board and version, and each file's path and hashes.
pub fn digest(device: &str, plan: &CardPlan) -> String {
    let h = |b: &Option<Vec<u8>>| b.as_deref().map_or("-".to_string(), blobs::hash);
    let mut text = format!(
        "card|{device}|{}|{}",
        plan.identity.board.clone().unwrap_or_default(),
        plan.identity.version.clone().unwrap_or_default()
    );
    for f in &plan.files {
        text.push_str(&format!("\n{}|{}|{}", f.path, h(&f.before), h(&f.after)));
    }
    blobs::hash(text.as_bytes())
}

/// The gate every staged change passes before a plan: a change marked Read first is not
/// applied until the person has read the real value and marked it Ready.
pub fn read_first(change: &StagedChange) -> Check {
    check(
        "Read first",
        if change.status == crate::gear::model::ChangeStatus::ReadFirst {
            Err(Refusal::new(
                RefusalCode::ReadFirst,
                "This change is marked Read first. Read the real value on the device, then mark it Ready.",
            ))
        } else {
            Ok(())
        },
    )
}

/// Builds the plan for a card change. `cands` are the radio cards mounted now.
pub fn plan(
    change: &StagedChange,
    work: &CardWork,
    cands: &[CardCand],
    facts: &CardFacts,
) -> CardPlanned {
    let mut checks: Vec<Check> = vec![read_first(change)];
    let out = |checks: Vec<Check>, cand: Option<CardCand>, identity: Identity| CardPlanned {
        plan: ApplyPlan {
            change: change.id.clone(),
            device: identity,
            checks,
            diff: Vec::new(),
            digest: String::new(),
            warnings: Vec::new(),
        },
        cand,
        files: None,
        warnings: Vec::new(),
    };

    let mine: Vec<&CardCand> = cands
        .iter()
        .filter(|c| c.connected.id.as_deref() == Some(change.device.as_str()))
        .collect();
    let (chosen, found): (Option<&CardCand>, Result<(), Refusal>) = match (cands.len(), mine.len())
    {
        (0, _) => (
            None,
            Err(Refusal::new(
                RefusalCode::NoDevice,
                "No radio card is mounted. Plug the radio in (USB Storage) or insert its card.",
            )),
        ),
        (_, 1) => (Some(mine[0]), Ok(())),
        (_, n) if n > 1 => (
            None,
            Err(Refusal::new(
                RefusalCode::SeveralDevices,
                "Two cards claim this id; remove one.",
            )),
        ),
        _ => (None, Ok(())),
    };
    checks.push(check("One card plugged in", found));
    checks.push(check(
        "Same card as planned",
        if !cands.is_empty() && chosen.is_none() && mine.is_empty() {
            Err(Refusal::new(
                RefusalCode::DeviceChanged,
                "This is not the card the change was planned for.",
            ))
        } else {
            Ok(())
        },
    ));
    let Some(cand) = chosen else {
        return out(checks, None, Identity::default());
    };

    checks.push(check(
        "Card check",
        match &facts.check_failed {
            Some(why) => Err(Refusal::new(
                RefusalCode::CardCheck,
                format!("The card's last check found errors: {why}. Repair it first."),
            )),
            None => Ok(()),
        },
    ));
    checks.push(check(
        "Nothing else writing",
        match &facts.busy {
            Some(step) => Err(Refusal::new(
                RefusalCode::PortBusy,
                format!("{step} is running on this card. Wait for it to finish."),
            )),
            None => Ok(()),
        },
    ));

    let card = match Card::open(&cand.root) {
        Ok(c) => c,
        Err(e) => {
            checks.push(check(
                "Card readable",
                Err(Refusal::new(RefusalCode::ShapeUnknown, format!("{e:#}"))),
            ));
            return out(checks, Some(cand.clone()), cand.connected.identity.clone());
        }
    };
    let planned = match work {
        CardWork::Edits(edits) => card.plan(edits, None),
        CardWork::Files(files) => card.plan_files(files.clone()),
    };
    let cp = match planned {
        Ok(p) => p,
        Err(e) => {
            checks.push(check(
                "Edits understood",
                Err(Refusal::new(RefusalCode::ShapeUnknown, format!("{e:#}"))),
            ));
            return out(checks, Some(cand.clone()), card.identity());
        }
    };
    checks.extend(cp.checks.iter().cloned());
    if cp.checks.iter().all(|c| c.ok) {
        checks.push(check(
            "Something to write",
            if cp.files.is_empty() {
                Err(Refusal::new(
                    RefusalCode::ShapeUnknown,
                    "No file changes: the card already matches.",
                ))
            } else {
                Ok(())
            },
        ));
    }
    let digest = if cp.files.is_empty() {
        String::new()
    } else {
        digest(&change.device, &cp)
    };
    CardPlanned {
        plan: ApplyPlan {
            change: change.id.clone(),
            device: cp.identity.clone(),
            checks,
            diff: cp.diff.clone(),
            digest,
            warnings: Vec::new(),
        },
        cand: Some(cand.clone()),
        warnings: cp.warnings.clone(),
        files: Some(cp),
    }
}

/// What `roll_back` did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RollBack {
    /// Files put back to their backed-up bytes (or removed, for a new file).
    pub restored: Vec<String>,
    /// Files that could not be put back, or that do not read back as they were, with why.
    pub failed: Vec<String>,
    /// A put-back that ran past its timeout and was still running after `stuck_wait`: the
    /// roll back stopped there and checked nothing.
    pub still_writing: Option<String>,
}

impl RollBack {
    /// Every file reads back as it was before the plan.
    pub fn as_it_was(&self) -> bool {
        self.failed.is_empty() && self.still_writing.is_none()
    }
}

/// Puts every file of the plan back to the bytes it had before (design 8.4): after a
/// read-back mismatch, or a write that stopped on an error. A file that already reads as it
/// did is left alone. Each write runs under the engine's timeouts. A put-back that runs past
/// its timeout is waited for (`stuck_wait`) before the next file. At the end every file is
/// read again: one that does not read as it was is in `failed`.
pub fn roll_back(root: &Path, plan: &CardPlan, opts: &WriteOptions) -> RollBack {
    let mut out = RollBack::default();
    for f in plan.files.iter().rev() {
        if !inside_card(&f.path) {
            out.failed
                .push(format!("{}: not a file path inside the card", f.path));
            continue;
        }
        let path = root.join(&f.path);
        let now = std::fs::read(&path).ok();
        if now == f.before {
            continue;
        }
        let res = match &f.before {
            Some(old) => {
                let (p, b) = (path.clone(), old.clone());
                with_timeout(
                    opts.timeout_for(old.len() as u64),
                    &format!("restoring {}", f.path),
                    move || write_file_atomic(&p, &b),
                )
                .and_then(|same| {
                    if same {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("the restored bytes read back wrong"))
                    }
                })
            }
            None => {
                let p = path.clone();
                with_timeout(
                    opts.base_timeout,
                    &format!("removing {}", f.path),
                    move || match std::fs::remove_file(&p) {
                        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                        _ => Ok(()),
                    },
                )
            }
        };
        match res {
            Ok(()) => out.restored.push(f.path.clone()),
            Err(e) => match e.downcast_ref::<Stuck>() {
                // It ended: the read below says whether the file is back.
                Some(s) if s.wait(opts.stuck_wait) => {}
                Some(_) => {
                    out.still_writing = Some(format!("{}: {e:#}", f.path));
                    return out;
                }
                None => out.failed.push(format!("{}: {e:#}", f.path)),
            },
        }
    }
    for f in &plan.files {
        if !inside_card(&f.path)
            || out
                .failed
                .iter()
                .any(|x| x.starts_with(&format!("{}: ", f.path)))
        {
            continue;
        }
        match std::fs::read(root.join(&f.path)) {
            Ok(b) if f.before.as_ref() == Some(&b) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && f.before.is_none() => {}
            Ok(_) | Err(_) => out
                .failed
                .push(format!("{}: does not read back as it was", f.path)),
        }
    }
    out
}

/// True when every file of the plan reads as the plan says now: the new bytes, or absent.
/// The step after the write, on fresh reads (design 8.4).
pub fn verify(root: &Path, plan: &CardPlan) -> Vec<String> {
    plan.files
        .iter()
        .filter(|f| std::fs::read(root.join(&f.path)).ok() != f.after)
        .map(|f| f.path.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gear::edgetx::card::RadioOp;
    use crate::gear::edgetx::synth::{self, SynthCard};
    use crate::gear::model::{ChangeStatus, DeviceKind, Link};
    use crate::session::Editor;

    fn change(status: ChangeStatus, device: &str, edits: Vec<Edit>) -> StagedChange {
        StagedChange {
            id: "c1".into(),
            device: device.into(),
            title: "t".into(),
            status,
            edits,
            base_backup: String::new(),
            editor: Editor::User,
            note: String::new(),
            order: 0,
            history: vec![],
            reverts: None,
        }
    }

    fn contrast(v: &str) -> Edit {
        Edit::Radio {
            ops: vec![RadioOp::SetScalar {
                key: "contrast".into(),
                value: v.into(),
            }],
        }
    }

    fn cand(root: &Path, id: &str) -> CardCand {
        CardCand {
            connected: Connected {
                id: Some(id.into()),
                kind: DeviceKind::Radio,
                link: Link::Volume {
                    mount: root.to_path_buf(),
                    volume_uuid: None,
                    bus_protocol: None,
                    whole_disk: None,
                },
                identity: Identity::default(),
                device: None,
                usb: None,
                also: Vec::new(),
            },
            root: root.to_path_buf(),
        }
    }

    fn card() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("CARD");
        synth::write_card(&root, &SynthCard::default()).unwrap();
        (d, root)
    }

    fn first_failed(p: &CardPlanned) -> Option<RefusalCode> {
        p.plan
            .checks
            .iter()
            .find_map(|c| c.refusal.as_ref().map(|r| r.code))
    }

    #[test]
    fn another_job_on_the_card_refuses() {
        let (_d, root) = card();
        let c = change(ChangeStatus::Ready, "r1", vec![]);
        let work = CardWork::Edits(vec![contrast("25")]);
        let ok = plan(&c, &work, &[cand(&root, "r1")], &CardFacts::default());
        assert_eq!(first_failed(&ok), None, "{:?}", ok.plan.checks);
        assert!(!ok.plan.digest.is_empty());
        let busy = plan(
            &c,
            &work,
            &[cand(&root, "r1")],
            &CardFacts {
                busy: Some("Backing up".into()),
                ..Default::default()
            },
        );
        assert_eq!(first_failed(&busy), Some(RefusalCode::PortBusy));
        let sick = plan(
            &c,
            &work,
            &[cand(&root, "r1")],
            &CardFacts {
                check_failed: Some("errors".into()),
                ..Default::default()
            },
        );
        assert_eq!(first_failed(&sick), Some(RefusalCode::CardCheck));
    }

    #[test]
    fn the_digest_follows_the_card_and_the_edits() {
        let (_d, root) = card();
        let c = change(ChangeStatus::Ready, "r1", vec![]);
        let at = |v: &str| {
            plan(
                &c,
                &CardWork::Edits(vec![contrast(v)]),
                &[cand(&root, "r1")],
                &CardFacts::default(),
            )
            .plan
            .digest
        };
        assert_eq!(at("25"), at("25"));
        assert_ne!(at("25"), at("26"));
        // The same edits for another card id are another digest.
        let other = plan(
            &change(ChangeStatus::Ready, "r2", vec![]),
            &CardWork::Edits(vec![contrast("25")]),
            &[cand(&root, "r2")],
            &CardFacts::default(),
        );
        assert_ne!(other.plan.digest, at("25"));
        // A change on the card changes the digest.
        let before = at("25");
        let f = root.join("RADIO/radio.yml");
        let t = std::fs::read_to_string(&f)
            .unwrap()
            .replace("vBatWarn: 66", "vBatWarn: 67");
        std::fs::write(&f, t).unwrap();
        assert_ne!(at("25"), before);
    }

    #[test]
    fn nothing_to_write_and_read_first_refuse() {
        let (_d, root) = card();
        // contrast is 20 already: no file changes.
        let c = change(ChangeStatus::Ready, "r1", vec![]);
        let same = plan(
            &c,
            &CardWork::Edits(vec![contrast("20")]),
            &[cand(&root, "r1")],
            &CardFacts::default(),
        );
        assert_eq!(first_failed(&same), Some(RefusalCode::ShapeUnknown));
        let rf = plan(
            &change(ChangeStatus::ReadFirst, "r1", vec![]),
            &CardWork::Edits(vec![contrast("25")]),
            &[cand(&root, "r1")],
            &CardFacts::default(),
        );
        assert_eq!(first_failed(&rf), Some(RefusalCode::ReadFirst));
    }

    #[test]
    fn roll_back_puts_old_bytes_back_and_removes_new_files() {
        let (_d, root) = card();
        let c = change(ChangeStatus::Ready, "r1", vec![]);
        let p = plan(
            &c,
            &CardWork::Edits(vec![contrast("25")]),
            &[cand(&root, "r1")],
            &CardFacts::default(),
        );
        let cp = p.files.unwrap();
        let radio_before = std::fs::read(root.join("RADIO/radio.yml")).unwrap();
        // The first file went out, then the write failed.
        for f in &cp.files {
            if let Some(after) = &f.after {
                write_file_atomic(&root.join(&f.path), after).unwrap();
            }
        }
        assert_eq!(verify(&root, &cp), Vec::<String>::new());
        assert_ne!(
            std::fs::read(root.join("RADIO/radio.yml")).unwrap(),
            radio_before
        );
        let rb = roll_back(&root, &cp, &WriteOptions::reader());
        assert!(rb.failed.is_empty(), "{:?}", rb.failed);
        assert_eq!(
            std::fs::read(root.join("RADIO/radio.yml")).unwrap(),
            radio_before
        );
        assert!(!root.join(".metadata_never_index").exists());
        assert_eq!(verify(&root, &cp).len(), cp.files.len());
        // Nothing changed: nothing to put back.
        assert_eq!(
            roll_back(&root, &cp, &WriteOptions::reader()),
            RollBack::default()
        );
    }
}
