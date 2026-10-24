//! `Core`'s sim profiles (sim design 6.1, 6.3): the built-in presets, the profiles fitted
//! from a quad's own logs (`gear::sim_profiles`), and the fit.
//!
//! - `gear_sim_profiles` lists both kinds; `sim_profile` resolves an id for the sim and the
//!   harness.
//! - `gear_sim_fit` fits a profile from blackbox logs (an aircraft's pulls, or files), the
//!   quad's `diff all` and its weight, and validates it and its base on the same logs. It
//!   writes nothing and answers a digest.
//! - `gear_sim_fit_save` fits again and writes the profile only with `confirm` and that
//!   digest, so the profile changes only after the person saw the fit.

use super::Core;
use crate::gear::backup::{blob_of, Snapshots};
use crate::gear::blackbox::Pulls;
use crate::gear::blobs::Blobs;
use crate::gear::model::DeviceKind;
use crate::gear::sim_profiles::{self, SavedProfile, SimProfileInfo};
use anyhow::{bail, Context, Result};
use quadcam_sim::fit::{fit, FitInputs, FitReport};
use quadcam_sim::log::{self, LogData, Scales, MIN_FLIGHT_SECONDS};
use quadcam_sim::validate::{validate, ValidationReport};
use quadcam_sim::SimProfile;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::PathBuf;

/// `gear_sim_fit`: what the fit reads.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct SimFitParams {
    /// The built-in profile the fit starts from and keeps what the logs cannot give:
    /// `meteor75`, `air65ii`, `five_inch` or `seven_inch`.
    pub base: String,
    /// An aircraft profile: its FC's blackbox pulls and the settings of its latest backup.
    #[serde(default)]
    pub aircraft: Option<String>,
    /// Blackbox files (`.bbl`), flash images, decoded CSV, or folders of them, read besides
    /// the aircraft's pulls.
    #[serde(default)]
    pub logs: Vec<PathBuf>,
    /// Betaflight diff or dump files (a later file's lines win); default: the aircraft's FC's
    /// latest backup.
    #[serde(default)]
    pub fc: Vec<PathBuf>,
    /// The quad weighed with a pack (g); default: the base profile's.
    #[serde(default)]
    pub mass_g: Option<f64>,
    /// The pack's weight (g).
    #[serde(default)]
    pub pack_g: Option<f64>,
    /// The new profile's id; default: from the aircraft's name, else `<base>-fitted`.
    #[serde(default)]
    pub id: Option<String>,
    /// Its label; default: the aircraft's name, else the base's label with "(fitted)".
    #[serde(default)]
    pub label: Option<String>,
}

/// The fit, and how the fitted profile and its base do on the same logs.
#[derive(Debug, Clone, Serialize, Type)]
pub struct SimFit {
    pub report: FitReport,
    pub validation: ValidationReport,
    pub base_validation: ValidationReport,
    /// The pulls and files the fit read.
    pub sources: Vec<String>,
    /// A fitted profile with this id exists; a save replaces it.
    pub replaces: bool,
    /// What `gear_sim_fit_save` needs.
    pub digest: String,
}

/// `gear_sim_fit_save`: the same fit, the digest `gear_sim_fit` gave, and `confirm`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct SimFitSaveParams {
    pub fit: SimFitParams,
    pub digest: String,
    #[serde(default)]
    pub confirm: bool,
}

/// `gear_sim_profile_delete`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct SimProfileDeleteParams {
    pub id: String,
    #[serde(default)]
    pub confirm: bool,
}

/// What the fit is fitted to.
struct Inputs {
    base: SimProfile,
    fit: FitInputs,
    logs: Vec<LogData>,
    sources: Vec<String>,
}

impl Core {
    /// The built-in presets, then the fitted profiles. Reads only.
    pub fn gear_sim_profiles(&self) -> Result<Vec<SimProfileInfo>> {
        let mut out: Vec<SimProfileInfo> = quadcam_sim::presets()
            .iter()
            .map(SimProfileInfo::preset)
            .collect();
        out.extend(
            sim_profiles::list(self.gear_store().root())
                .iter()
                .map(SimProfileInfo::fitted),
        );
        Ok(out)
    }

    /// A profile by id: a built-in preset or a fitted profile.
    pub(crate) fn sim_profile(&self, id: &str) -> Result<SimProfile> {
        let id = id.trim();
        if let Some(p) = quadcam_sim::preset(id) {
            return Ok(p);
        }
        if let Some(s) = sim_profiles::read(self.gear_store().root(), id)? {
            return Ok(s.profile);
        }
        let ids: Vec<String> = self
            .gear_sim_profiles()?
            .into_iter()
            .map(|p| p.id)
            .collect();
        bail!("No sim profile {id:?}. Profiles: {}.", ids.join(", "))
    }

    /// Fits a sim profile from a quad's blackbox logs and validates it and its base on the
    /// same logs. Writes nothing.
    pub fn gear_sim_fit(&self, p: &SimFitParams) -> Result<SimFit> {
        let inputs = self.sim_fit_inputs(p)?;
        let report = fit(&inputs.base, &inputs.fit, &inputs.logs).map_err(anyhow::Error::msg)?;
        let profile = report.profile.clone().context("the fit made no profile")?;
        let digest = sim_profiles::digest(&profile)?;
        let replaces = sim_profiles::file(self.gear_store().root(), &profile.id).exists();
        Ok(SimFit {
            validation: validate(&profile, &inputs.logs),
            base_validation: validate(&inputs.base, &inputs.logs),
            report,
            sources: inputs.sources,
            replaces,
            digest,
        })
    }

    /// Fits again and saves the profile, when `confirm` is true and the fit still gives the
    /// digest `gear_sim_fit` answered (the same logs, settings and weight).
    pub fn gear_sim_fit_save(&self, p: &SimFitSaveParams) -> Result<SimProfileInfo> {
        if !p.confirm {
            bail!("Saving a fitted sim profile needs confirm=true, after the person saw the fit.");
        }
        let inputs = self.sim_fit_inputs(&p.fit)?;
        let report = fit(&inputs.base, &inputs.fit, &inputs.logs).map_err(anyhow::Error::msg)?;
        let profile = report.profile.context("the fit made no profile")?;
        if sim_profiles::digest(&profile)? != p.digest.trim() {
            bail!("The fit changed since the digest was given (new logs or settings): fit again and check it.");
        }
        let saved = SavedProfile {
            profile,
            base: inputs.base.id.clone(),
            aircraft: p.fit.aircraft.clone(),
            fitted_at: chrono::Utc::now(),
            sources: inputs.sources,
        };
        sim_profiles::write(self.gear_store().root(), &saved)?;
        Ok(SimProfileInfo::fitted(&saved))
    }

    /// Removes a fitted profile (never a preset). Needs `confirm`. False when there was none.
    pub fn gear_sim_profile_delete(&self, p: &SimProfileDeleteParams) -> Result<bool> {
        if !p.confirm {
            bail!("Deleting a fitted sim profile needs confirm=true, after the person agreed.");
        }
        sim_profiles::delete(self.gear_store().root(), p.id.trim())
    }

    fn sim_fit_inputs(&self, p: &SimFitParams) -> Result<Inputs> {
        let base_id = p.base.trim();
        let Some(base) = quadcam_sim::preset(base_id) else {
            let ids: Vec<String> = quadcam_sim::presets().into_iter().map(|p| p.id).collect();
            bail!(
                "The fit starts from a built-in profile: {}. Not {base_id:?}.",
                ids.join(", ")
            );
        };
        let aircraft = p
            .aircraft
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty());
        let store = self.gear_store();
        let fcs: Vec<String> = match aircraft {
            Some(a) => {
                let fcs: Vec<String> = store
                    .devices()?
                    .into_iter()
                    .filter(|d| d.kind == DeviceKind::Fc && d.aircraft.as_deref() == Some(a))
                    .map(|d| d.id)
                    .collect();
                if fcs.is_empty() && p.logs.is_empty() {
                    bail!("No FC is linked to the aircraft {a:?} (gear devices); link it, or pass logs.");
                }
                fcs
            }
            None => Vec::new(),
        };
        let mut sources = Vec::new();

        // The FC's settings: the files, else the newest backup of the aircraft's FC.
        let mut texts = Vec::new();
        for f in &p.fc {
            let bytes = std::fs::read(f).with_context(|| format!("cannot read {}", f.display()))?;
            texts.push(String::from_utf8_lossy(&bytes).into_owned());
            sources.push(f.display().to_string());
        }
        if texts.is_empty() {
            let snaps = Snapshots::new(store.clone());
            let newest = fcs
                .iter()
                .filter_map(|d| {
                    snaps.list(d).into_iter().rev().find_map(|b| {
                        ["diff all", "dump all"].iter().find_map(|name| {
                            b.files
                                .iter()
                                .find(|f| f.path == *name)
                                .map(|f| (b.taken_at, b.id.clone(), blob_of(f), *name))
                        })
                    })
                })
                .max_by_key(|x| x.0);
            if let Some((_, id, blob, name)) = newest {
                let bytes = snaps.blobs().get(&blob)?;
                texts.push(String::from_utf8_lossy(&bytes).into_owned());
                sources.push(format!("backup {id} ({name})"));
            }
        }
        let diff = (!texts.is_empty()).then(|| texts.join("\n"));
        let poles = diff
            .as_deref()
            .map(|d| quadcam_sim::diff::FcConfig::from_diff(d).motor_poles as f64)
            .unwrap_or(base.motor.poles.value);
        let scales = Scales {
            motor_poles: poles,
            ..Scales::default()
        };

        // The logs: each erased pull (it holds logs no later pull has) and the newest pull,
        // which holds everything since the last erase; then the files.
        let mut logs = Vec::new();
        let pulls = Pulls::new(store.clone());
        let blobs = Blobs::new(store.clone());
        for d in &fcs {
            let list = pulls.list(d);
            let last = list.len().saturating_sub(1);
            for (i, pull) in list.iter().enumerate() {
                if !(pull.erased || i == last) {
                    continue;
                }
                let image = blobs.get(&pull.blob)?;
                let name = format!("pull {}", pull.id);
                logs.extend(
                    log::from_image(&name, &image, scales, MIN_FLIGHT_SECONDS)
                        .map_err(anyhow::Error::msg)?,
                );
                sources.push(name);
            }
        }
        for f in &p.logs {
            logs.extend(log::read_path(f, scales).map_err(anyhow::Error::msg)?);
            sources.push(f.display().to_string());
        }
        if logs.is_empty() {
            bail!("No blackbox logs to fit from: pull the FC's blackbox (gear blackbox pull), or pass logs.");
        }

        let id = match p.id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(id) => id.to_string(),
            None => match aircraft
                .map(sim_profiles::id_from)
                .filter(|s| !s.is_empty())
            {
                Some(id) => id,
                None => format!("{}-fitted", base.id),
            },
        };
        sim_profiles::check_id(&id)?;
        let label = p
            .label
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| aircraft.map(String::from))
            .unwrap_or_else(|| format!("{} (fitted)", base.label));
        let fit = FitInputs {
            id,
            label,
            mass: p.mass_g.map(|g| g / 1000.0),
            pack_mass: p.pack_g.map(|g| g / 1000.0),
            diff,
            radio: Vec::new(),
        };
        Ok(Inputs {
            base,
            fit,
            logs,
            sources,
        })
    }
}
