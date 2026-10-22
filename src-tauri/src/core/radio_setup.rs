//! `Core::gear_radio_setup`, `Core::gear_radio_setup_edit` and `Core::gear_radio_post_flash`:
//! a radio setup as data (`docs/radio-design.md` 4.2, 5 and stage 3). The setup is read
//! against the radio's mounted card, else its latest backup, else a card folder; the
//! generator's edits are planned on a copy, so the answer says which files would change.
//! An edit saves the setup in `gear.json` (QuadCam's own data); `stage` puts the generated
//! edits in the radio's one "Radio setup" change for the apply sheet. The steps after a
//! flash (storage pack, ELRS Lua, volume label) plan here too; their downloads come from
//! upstream at run time.

use super::model_edit::Base;
use super::switchmap::{CardSource, Folder};
use super::Core;
use crate::gear::changes::ChangeFilter;
use crate::gear::edgetx::card::{identity_from_radio_yml, selected_in, Card, RADIO_FILE};
use crate::gear::edgetx::setup::generate::{self, Generated, JobRow, SoundCheck, Source};
use crate::gear::edgetx::setup::postflash::{self, PackPlan};
use crate::gear::edgetx::setup::{controls, defaults, Assignment, Control, RadioSetup};
use crate::gear::edgetx::yaml::Doc;
use crate::gear::model::{CardFile, ChangeStatus, DiffItem, Edit, StagedChange};
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The title of the change a setup stages.
pub const SETUP_CHANGE: &str = "Radio setup";
/// The `gear.json` key that holds each radio's saved setup.
const SETUPS_KEY: &str = "radio_setups";

/// A radio's saved setup.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SavedSetup {
    pub device: String,
    pub setup: RadioSetup,
    pub saved_at: DateTime<Utc>,
}

/// `gear_radio_setup`: a radio, or a card folder, and the setup to read against it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct RadioSetupParams {
    /// A saved radio's device id: its saved setup, against its mounted card or latest backup.
    #[serde(default)]
    pub device: Option<String>,
    /// A card folder (a mounted card, or a copy) to read instead of the radio's card.
    #[serde(default)]
    pub mount: Option<String>,
    /// A setup to read instead of the saved one.
    #[serde(default)]
    pub setup: Option<RadioSetup>,
}

/// A setup and what it gives on the card.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RadioSetupDetail {
    #[serde(default)]
    pub device: Option<String>,
    /// `card` (mounted now), `backup <id> (<date>)` or `folder <path>`.
    pub source: String,
    /// `saved` (the radio's), `given` or `default` (the starting setup for the board).
    pub origin: String,
    pub setup: RadioSetup,
    /// The board's controls a job may use.
    pub controls: Vec<Control>,
    pub jobs: Vec<JobRow>,
    pub sounds: Vec<SoundCheck>,
    pub warnings: Vec<String>,
    /// The generated edits (what `stage` puts in the "Radio setup" change).
    pub edits: Vec<Edit>,
    /// The card files the edits would change; empty when the card already matches.
    pub files: Vec<String>,
    pub diff: Vec<DiffItem>,
}

/// `gear_radio_setup_edit`: changes to a radio's saved setup, and its staging.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct RadioSetupEditParams {
    pub device: String,
    /// The whole setup, replacing the saved one.
    #[serde(default)]
    pub setup: Option<RadioSetup>,
    /// Start again from the board's default setup.
    #[serde(default)]
    pub reset: bool,
    /// Controls for jobs: each replaces the job's control (`{job, control, high}`).
    #[serde(default)]
    pub assign: Vec<Assignment>,
    /// Jobs that no control does any more.
    #[serde(default)]
    pub free: Vec<String>,
    /// Stage the generated edits as the radio's "Radio setup" change.
    #[serde(default)]
    pub stage: bool,
    /// Who stages it. The app and the CLI leave it out (the person); MCP says `agent`.
    #[serde(default)]
    pub editor: Option<crate::session::Editor>,
}

/// What an edit saved, and the change it staged.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RadioSetupEdited {
    pub detail: RadioSetupDetail,
    #[serde(default)]
    pub change: Option<StagedChange>,
    pub notes: Vec<String>,
}

/// A step after a flash.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum PostFlashStep {
    /// EdgeTX's SD-card files for the radio's screen.
    StoragePack,
    /// `SCRIPTS/TOOLS/elrs.lua` from an ExpressLRS release; the old names removed.
    ElrsLua,
    /// A volume label from the radio's saved name.
    VolumeLabel,
}

/// `gear_radio_post_flash`: the steps after a flash, planned, and staged on request.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct PostFlashParams {
    pub device: String,
    /// The steps; every step when empty.
    #[serde(default)]
    pub steps: Vec<PostFlashStep>,
    /// The storage pack release (`2.12.3`); default the newest QuadCam checked for the
    /// radio's EdgeTX series.
    #[serde(default)]
    pub pack_version: Option<String>,
    /// The ExpressLRS release whose Lua to put on (`4.1.0`); the Lua step needs it.
    #[serde(default)]
    pub elrs_version: Option<String>,
    /// Stage the storage pack and the Lua as card changes.
    #[serde(default)]
    pub stage: bool,
    #[serde(default)]
    pub editor: Option<crate::session::Editor>,
}

/// One planned step.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct PostFlashPlanned {
    pub step: PostFlashStep,
    /// What it would do, in a sentence.
    pub summary: String,
    /// Nothing to do: the card already has it.
    pub done: bool,
    #[serde(default)]
    pub pack: Option<PackPlan>,
    /// Files the step puts or removes.
    #[serde(default)]
    pub files: Vec<String>,
    /// The label now, and the label it would get.
    #[serde(default)]
    pub label_before: Option<String>,
    #[serde(default)]
    pub label_after: Option<String>,
    /// Why the step cannot be planned.
    #[serde(default)]
    pub problem: Option<String>,
}

/// The steps after a flash.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct PostFlashPlan {
    pub device: String,
    pub source: String,
    pub steps: Vec<PostFlashPlanned>,
    /// Changes staged (with `stage`).
    pub changes: Vec<StagedChange>,
}

/// `gear_volume_label`: rename a radio's mounted volume.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct VolumeLabelParams {
    pub device: String,
    /// The label; default one made from the radio's saved name. Upper case letters, digits,
    /// `_` and `-`, 11 characters at most.
    #[serde(default)]
    pub label: Option<String>,
    /// Must be true.
    #[serde(default)]
    pub confirm: bool,
}

/// The volume's label before and after.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct VolumeLabeled {
    pub device: String,
    pub before: String,
    pub after: String,
    /// False when the volume already had the label.
    pub renamed: bool,
}

/// The core's card readers as the generator's source.
struct Src<'a>(&'a dyn CardSource);

impl Source for Src<'_> {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.0.read(path)
    }
    fn files(&self) -> Vec<String> {
        self.0.files()
    }
}

/// The files a plan reads, copied from a backup into a folder `Card::plan` can open.
fn materialize(src: &dyn CardSource, dir: &Path) -> Result<()> {
    for f in src.files() {
        let wanted = f == RADIO_FILE
            || f.starts_with("MODELS/")
            || (f.starts_with("SCRIPTS/") && (f.ends_with(".lua") || f.ends_with(".luac")))
            || f == postflash::PACK_VERSION_FILE;
        if !wanted || !crate::gear::edgetx::card::inside_card(&f) {
            continue;
        }
        if let Some(b) = src.read(&f) {
            let p = dir.join(&f);
            std::fs::create_dir_all(p.parent().context("no parent")?)?;
            std::fs::write(p, b)?;
        }
    }
    Ok(())
}

impl Core {
    fn saved_setup(&self, device: &str) -> Result<Option<RadioSetup>> {
        Ok(self
            .gear_store()
            .list::<SavedSetup>(SETUPS_KEY)?
            .into_iter()
            .find(|s| s.device == device)
            .map(|s| s.setup))
    }

    fn save_setup(&self, device: &str, setup: &RadioSetup) -> Result<()> {
        self.gear_store().update(|v| {
            let mut list: Vec<SavedSetup> = crate::gear::store::list(v, SETUPS_KEY);
            list.retain(|s| s.device != device);
            list.push(SavedSetup {
                device: device.to_string(),
                setup: setup.clone(),
                saved_at: Utc::now(),
            });
            list.sort_by(|a, b| a.device.cmp(&b.device));
            crate::gear::store::set_list(v, SETUPS_KEY, &list)
        })?;
        Ok(())
    }

    /// The card to read: a folder, or the device's mounted card or latest backup.
    fn setup_base<'a>(
        &self,
        device: Option<&str>,
        mount: Option<&str>,
        snaps: &'a crate::gear::backup::Snapshots,
    ) -> Result<Base<'a>> {
        if let Some(m) = mount {
            let root = PathBuf::from(m);
            Card::open(&root)?;
            return Ok(Base {
                source: format!("folder {}", root.display()),
                reader: Box::new(Folder(root)),
            });
        }
        let device = device.context("Name a radio (device) or a card folder (mount).")?;
        self.model_base(device, snaps)
    }

    /// The setup to read: given, saved, or the board's default.
    fn pick_setup(
        &self,
        device: Option<&str>,
        given: Option<&RadioSetup>,
        base: &Base<'_>,
    ) -> Result<(RadioSetup, String)> {
        if let Some(s) = given {
            return Ok((s.clone(), "given".into()));
        }
        if let Some(s) = device.map(|d| self.saved_setup(d)).transpose()?.flatten() {
            return Ok((s, "saved".into()));
        }
        Ok((default_for(base)?, "default".into()))
    }

    fn setup_detail(
        &self,
        device: Option<&str>,
        base: &Base<'_>,
        setup: RadioSetup,
        origin: String,
    ) -> Result<RadioSetupDetail> {
        let g: Generated =
            generate::generate(&setup, &Src(base.reader.as_ref())).map_err(|r| anyhow!(r))?;
        let tmp = postflash::Scratch::new(&self.cache.join("radio-setup"))?;
        materialize(base.reader.as_ref(), tmp.path())?;
        let plan = Card::open(tmp.path())?.plan(&g.edits, None)?;
        if let Some(r) = plan.checks.iter().find_map(|c| c.refusal.clone()) {
            return Err(anyhow!(r));
        }
        let files = plan
            .files
            .iter()
            .filter(|f| {
                f.path != crate::gear::edgetx::card::NEVER_INDEX
                    && f.path != crate::gear::edgetx::card::MARKER_FILE
            })
            .map(|f| f.path.clone())
            .collect::<Vec<_>>();
        let controls = controls::controls(&setup.board).unwrap_or_default();
        let mut warnings = g.warnings;
        warnings.extend(plan.warnings.iter().cloned());
        Ok(RadioSetupDetail {
            device: device.map(str::to_string),
            source: base.source.clone(),
            origin,
            controls,
            jobs: g.jobs,
            sounds: g.sounds,
            warnings,
            edits: g.edits,
            diff: if files.is_empty() { vec![] } else { plan.diff },
            files,
            setup,
        })
    }

    /// Reads a setup against a radio's card or a card folder: the jobs and their controls,
    /// the generated edits and the files they would change. Writes nothing.
    pub fn gear_radio_setup(&self, p: &RadioSetupParams) -> Result<RadioSetupDetail> {
        let snaps = self.snapshots();
        let base = self.setup_base(p.device.as_deref(), p.mount.as_deref(), &snaps)?;
        let (setup, origin) = self.pick_setup(p.device.as_deref(), p.setup.as_ref(), &base)?;
        self.setup_detail(p.device.as_deref(), &base, setup, origin)
    }

    /// Changes a radio's saved setup (the whole setup, a reset, or job controls), checks it
    /// against the board and the card, saves it, and with `stage` stages its edits as the
    /// radio's "Radio setup" change. Writes only QuadCam's own data.
    pub fn gear_radio_setup_edit(&self, p: &RadioSetupEditParams) -> Result<RadioSetupEdited> {
        let snaps = self.snapshots();
        let base = self.setup_base(Some(&p.device), None, &snaps)?;
        let mut setup = match (&p.setup, p.reset) {
            (Some(_), true) => bail!("Pass a setup or reset, not both."),
            (Some(s), false) => s.clone(),
            (None, true) => default_for(&base)?,
            (None, false) => match self.saved_setup(&p.device)? {
                Some(s) => s,
                None => default_for(&base)?,
            },
        };
        for a in &p.assign {
            setup.assignments.retain(|x| x.job != a.job);
            setup.assignments.push(a.clone());
        }
        for j in &p.free {
            if setup.job(j).is_none() {
                bail!("{j:?} is not a job of this setup.");
            }
            setup.assignments.retain(|x| &x.job != j);
        }
        // Keep the job order for a stable file.
        let order: Vec<String> = setup.jobs.iter().map(|j| j.id.clone()).collect();
        setup
            .assignments
            .sort_by_key(|a| order.iter().position(|j| *j == a.job).unwrap_or(usize::MAX));
        generate::check(&setup).map_err(|r| anyhow!(r))?;
        let detail = self.setup_detail(Some(&p.device), &base, setup.clone(), "saved".into())?;
        self.save_setup(&p.device, &setup)?;
        let mut notes = Vec::new();
        let open = self
            .changes()
            .list(&ChangeFilter {
                device: Some(p.device.clone()),
                ..Default::default()
            })
            .into_iter()
            .find(|c| {
                c.title == SETUP_CHANGE
                    && matches!(c.status, ChangeStatus::Draft | ChangeStatus::Ready)
            });
        let mut change = None;
        if p.stage {
            change = Some(match (open, detail.files.is_empty()) {
                (Some(c), true) => {
                    notes.push(
                        "The card already matches the setup; the open change was dropped.".into(),
                    );
                    self.gear_change_discard(&c.id)?
                }
                (None, true) => {
                    notes.push("The card already matches the setup; nothing was staged.".into());
                    return Ok(RadioSetupEdited {
                        detail,
                        change: None,
                        notes,
                    });
                }
                (Some(c), false) => self.gear_change_update(&super::ChangeUpdateParams {
                    id: c.id,
                    edits: Some(detail.edits.clone()),
                    ..Default::default()
                })?,
                (None, false) => self.gear_change_stage(&super::StageParams {
                    device: p.device.clone(),
                    title: Some(SETUP_CHANGE.into()),
                    edits: detail.edits.clone(),
                    editor: p.editor,
                    ..Default::default()
                })?,
            });
        }
        Ok(RadioSetupEdited {
            detail,
            change,
            notes,
        })
    }

    /// Plans the steps after a flash on a radio's card (mounted, else its latest backup),
    /// and with `stage` stages the storage pack and the ELRS Lua as card changes. Downloads
    /// the pack and the Lua from upstream; writes no card.
    pub fn gear_radio_post_flash(&self, p: &PostFlashParams) -> Result<PostFlashPlan> {
        let snaps = self.snapshots();
        let base = self.setup_base(Some(&p.device), None, &snaps)?;
        let src = Src(base.reader.as_ref());
        let identity = base
            .reader
            .read(RADIO_FILE)
            .map(|b| identity_from_radio_yml(&b))
            .unwrap_or_default();
        let steps = if p.steps.is_empty() {
            vec![
                PostFlashStep::StoragePack,
                PostFlashStep::ElrsLua,
                PostFlashStep::VolumeLabel,
            ]
        } else {
            p.steps.clone()
        };
        let scratch = self.cache.join("post-flash");
        let mut out = PostFlashPlan {
            device: p.device.clone(),
            source: base.source.clone(),
            steps: vec![],
            changes: vec![],
        };
        let empty = |step, problem: String| PostFlashPlanned {
            step,
            summary: String::new(),
            done: false,
            pack: None,
            files: vec![],
            label_before: None,
            label_after: None,
            problem: Some(problem),
        };
        for step in steps {
            match step {
                PostFlashStep::StoragePack => {
                    let board = identity.board.clone().unwrap_or_default();
                    let Some(name) = postflash::pack_name(&board) else {
                        out.steps.push(empty(
                            step,
                            format!("QuadCam has no storage pack matched to the board {board:?}."),
                        ));
                        continue;
                    };
                    let version = match p.pack_version.clone().or_else(|| {
                        identity
                            .version
                            .as_deref()
                            .and_then(postflash::pack_for)
                            .map(str::to_string)
                    }) {
                        Some(v) if postflash::valid_version(&v) => v,
                        _ => {
                            out.steps.push(empty(
                                step,
                                "Name the storage pack release (pack_version).".into(),
                            ));
                            continue;
                        }
                    };
                    let files = match self.pack_files(&scratch, &version, name) {
                        Ok(f) => f,
                        Err(e) => {
                            out.steps.push(empty(step, format!("{e:#}")));
                            continue;
                        }
                    };
                    let (plan, put) = postflash::plan_pack(&src, &files);
                    let done = put.is_empty();
                    let summary = if done {
                        format!("The card has the {name} pack {version}.")
                    } else {
                        format!(
                            "Put {} files ({} KB) of the {name} pack {version} on the card; {} are already there.",
                            plan.put.len(),
                            plan.bytes.div_ceil(1024),
                            plan.unchanged
                        )
                    };
                    if p.stage && !done {
                        let blobs = snaps.blobs();
                        let mut cf = Vec::new();
                        for (path, bytes) in &put {
                            let r = blobs.put(bytes)?;
                            cf.push(CardFile {
                                path: path.clone(),
                                xxh64: r.xxh64,
                                size: r.size,
                            });
                        }
                        out.changes
                            .push(self.gear_change_stage(&super::StageParams {
                                device: p.device.clone(),
                                title: Some(format!("Storage pack {version}")),
                                edits: vec![Edit::CardFiles {
                                    put: cf,
                                    delete: vec![],
                                }],
                                editor: p.editor,
                                ..Default::default()
                            })?);
                    }
                    out.steps.push(PostFlashPlanned {
                        step,
                        summary,
                        done,
                        files: plan.put.clone(),
                        pack: Some(plan),
                        label_before: None,
                        label_after: None,
                        problem: None,
                    });
                }
                PostFlashStep::ElrsLua => {
                    let Some(v) = p
                        .elrs_version
                        .clone()
                        .filter(|v| postflash::valid_version(v))
                    else {
                        out.steps.push(empty(
                            step,
                            "Name the ExpressLRS release whose Lua to put on (elrs_version)."
                                .into(),
                        ));
                        continue;
                    };
                    let lua = match crate::gear::firmware::fetch_bytes(
                        self.firmware.fetch.as_ref(),
                        &postflash::elrs_lua_url(&v),
                        &scratch,
                    ) {
                        Ok(b) => b,
                        Err(e) => {
                            out.steps.push(empty(step, format!("{e:#}")));
                            continue;
                        }
                    };
                    let edits = postflash::plan_elrs_lua(&src, &lua).map_err(|r| anyhow!(r))?;
                    let files: Vec<String> = edits
                        .iter()
                        .filter_map(|e| match e {
                            Edit::CardText { path, .. } => Some(path.clone()),
                            _ => None,
                        })
                        .collect();
                    let done = edits.is_empty();
                    if p.stage && !done {
                        out.changes
                            .push(self.gear_change_stage(&super::StageParams {
                                device: p.device.clone(),
                                title: Some(format!("ELRS Lua {v}")),
                                edits,
                                editor: p.editor,
                                ..Default::default()
                            })?);
                    }
                    out.steps.push(PostFlashPlanned {
                        step,
                        summary: if done {
                            format!("The card has the ELRS {v} Lua.")
                        } else {
                            format!("Put the ELRS {v} Lua in SCRIPTS/TOOLS/ and remove the older names.")
                        },
                        done,
                        pack: None,
                        files,
                        label_before: None,
                        label_after: None,
                        problem: None,
                    });
                }
                PostFlashStep::VolumeLabel => {
                    let device = self.gear_store().device(&p.device)?;
                    let name = device.map(|d| d.name).unwrap_or_default();
                    let mounted = self.mounted_label(&p.device);
                    match postflash::volume_label(&name) {
                        Ok(label) => {
                            let done = mounted.as_deref() == Some(label.as_str());
                            out.steps.push(PostFlashPlanned {
                                step,
                                summary: match (&mounted, done) {
                                    (_, true) => format!("The volume is already {label}."),
                                    (Some(m), false) => {
                                        format!("Rename the volume {m} to {label}.")
                                    }
                                    (None, false) => format!(
                                        "Rename the volume to {label} (plug the radio in, USB Storage)."
                                    ),
                                },
                                done,
                                pack: None,
                                files: vec![],
                                label_before: mounted,
                                label_after: Some(label),
                                problem: None,
                            });
                        }
                        Err(r) => out.steps.push(empty(
                            step,
                            format!("{} Save a name for the radio first.", r.reason),
                        )),
                    }
                }
            }
        }
        Ok(out)
    }

    /// Renames the radio's mounted volume (`diskutil rename`) to a label from its saved name,
    /// or the one given. Needs `confirm`. QuadCam knows the card by its marker and volume
    /// UUID, so the label is for the person only.
    pub fn gear_volume_label(&self, p: &VolumeLabelParams) -> Result<VolumeLabeled> {
        if !p.confirm {
            bail!("Refused: renaming the radio's volume needs confirm=true.");
        }
        let name = match &p.label {
            Some(l) => l.clone(),
            None => self
                .gear_store()
                .device(&p.device)?
                .map(|d| d.name)
                .unwrap_or_default(),
        };
        let label = postflash::volume_label(&name).map_err(|r| anyhow!(r))?;
        if p.label.as_deref().is_some_and(|l| l != label) {
            bail!("A volume label is upper case letters, digits, _ and -, 11 characters at most: {label:?}, not {:?}.", p.label.as_deref().unwrap_or_default());
        }
        let mount = self
            .gear_connected()?
            .into_iter()
            .find_map(|c| match &c.link {
                crate::gear::model::Link::Volume { mount, .. }
                    if c.id.as_deref() == Some(p.device.as_str()) =>
                {
                    Some(mount.clone())
                }
                _ => None,
            })
            .with_context(|| {
                format!(
                    "{} is not mounted: plug the radio in (USB Storage).",
                    p.device
                )
            })?;
        let before = mount
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if before == label {
            return Ok(VolumeLabeled {
                device: p.device.clone(),
                before,
                after: label,
                renamed: false,
            });
        }
        let out = self.gear.disk.run(
            &[
                "rename".to_string(),
                mount.display().to_string(),
                label.clone(),
            ],
            None,
            std::time::Duration::from_secs(60),
        )?;
        if out.code != Some(0) {
            bail!("diskutil rename failed: {}", out.output.trim());
        }
        (self.gear.refresh_volumes)();
        Ok(VolumeLabeled {
            device: p.device.clone(),
            before,
            after: label,
            renamed: true,
        })
    }

    /// The volume name of the radio's mounted card, when it is mounted.
    fn mounted_label(&self, device: &str) -> Option<String> {
        self.gear_connected()
            .ok()?
            .into_iter()
            .find_map(|c| match &c.link {
                crate::gear::model::Link::Volume { mount, .. }
                    if c.id.as_deref() == Some(device) =>
                {
                    mount.file_name().map(|n| n.to_string_lossy().to_string())
                }
                _ => None,
            })
    }

    /// The storage pack's files, downloaded once into QuadCam's cache and unpacked.
    fn pack_files(
        &self,
        scratch: &Path,
        version: &str,
        name: &str,
    ) -> Result<Vec<(String, Vec<u8>)>> {
        let dir = scratch.join(format!("sdcard-{version}"));
        std::fs::create_dir_all(&dir)?;
        let zip = dir.join(format!("{name}.zip"));
        if !zip.is_file() {
            let bytes = crate::gear::firmware::fetch_bytes(
                self.firmware.fetch.as_ref(),
                &postflash::pack_url(version, name),
                &dir,
            )?;
            std::fs::write(&zip, bytes)?;
        }
        postflash::unpack(&zip, &dir)
    }
}

/// The board's default setup on the base's card, for its selected model.
fn default_for(base: &Base<'_>) -> Result<RadioSetup> {
    let radio = base
        .reader
        .read(RADIO_FILE)
        .with_context(|| format!("The {} has no {RADIO_FILE}.", base.source))?;
    let board = identity_from_radio_yml(&radio)
        .board
        .context("radio.yml names no board.")?;
    let model = Doc::parse("radio.yml", &radio)
        .ok()
        .and_then(|d| selected_in(&d))
        .unwrap_or_else(|| "model00.yml".into());
    defaults::setup(&board, &model).with_context(|| {
        format!("QuadCam has no control list for the board {board:?}, so it has no setup for it.")
    })
}

/// The setup as text, for the CLI and MCP.
pub fn render_text(d: &RadioSetupDetail) -> String {
    use std::fmt::Write;
    let mut s = format!(
        "Radio setup ({}, {} setup) for the {}, from the {}\n",
        d.device.as_deref().unwrap_or("folder"),
        d.origin,
        d.setup.board,
        d.source
    );
    for j in &d.jobs {
        let states: Vec<String> = j
            .states
            .iter()
            .map(|x| match &x.position {
                Some(p) => format!("{p} {}", x.words),
                None => x.words.clone(),
            })
            .collect();
        let _ = writeln!(
            s,
            "  {:<15} {:<5} {:<13} {}",
            j.label,
            j.channel.as_deref().unwrap_or("-"),
            j.control.as_deref().unwrap_or("(none)"),
            states.join(" / ")
        );
    }
    let _ = writeln!(
        s,
        "Edits: {}; files that would change: {}",
        d.edits.len(),
        if d.files.is_empty() {
            "none (the card matches)".to_string()
        } else {
            d.files.join(", ")
        }
    );
    for w in &d.warnings {
        let _ = writeln!(s, "  note: {w}");
    }
    s
}
