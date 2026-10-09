//! `Core::gear_rates` and `Core::gear_sims`: an FC's rate profiles from its latest backup,
//! a backup or dump files, and the rate profiles of the sims on this Mac next to the quad's
//! (design 6.6). Both read only.

use super::Core;
use crate::gear::bf::dump::Config;
use crate::gear::rates::{self, RateProfileView, RatesView};
use crate::gear::sims::{self, SimStatus};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Files larger than this are not a Betaflight dump.
const MAX_FILE: u64 = 4 * 1024 * 1024;

/// `gear_rates`: where to read the rate profiles from.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct RatesParams {
    /// `dump all`, `diff all` or CLI-line files, read in order: a later file's lines win.
    #[serde(default)]
    pub paths: Vec<PathBuf>,
    /// A saved device's id: reads its latest backup's `dump all` (`diff all` without one),
    /// before `paths`.
    #[serde(default)]
    pub device: Option<String>,
    /// A backup id (`<device>/<name>`) from `gear_backups`, instead of the latest.
    #[serde(default)]
    pub backup: Option<String>,
}

/// `gear_sims`: the quad to compare with; none for the sims alone.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct SimsParams {
    #[serde(default)]
    pub paths: Vec<PathBuf>,
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub backup: Option<String>,
    /// The rate profile to compare with; default the one the FC uses.
    #[serde(default)]
    pub profile: Option<u8>,
}

/// `gear_rates_preview`: a rate profile as edited, to draw its curves; with `to`, the same
/// curve fitted onto another rate model first.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct RatesPreviewParams {
    pub profile: RateProfileView,
    /// `betaflight`, `actual`, `quick`, `raceflight` or `kiss`.
    #[serde(default)]
    pub to: Option<String>,
}

/// The edited profile with fresh curves, and, after a conversion, how far each axis is off.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RatesPreview {
    pub profile: RateProfileView,
    /// Largest gap from the source curve per axis (deg/s); 0 without a conversion.
    pub fit_error: Vec<f64>,
    /// The same as a share of the source's maximum rate (0-1).
    pub fit_share: Vec<f64>,
}

impl RatesParams {
    fn any(&self) -> bool {
        !self.paths.is_empty() || self.device.is_some() || self.backup.is_some()
    }
}

impl Core {
    /// The quad's rate profile `profile` (default: the one in use) from a device's latest
    /// backup, a backup or dump files.
    pub(super) fn quad_profile(
        &self,
        paths: &[PathBuf],
        device: &Option<String>,
        backup: &Option<String>,
        profile: Option<u8>,
    ) -> Result<RateProfileView> {
        let view = self.gear_rates(&RatesParams {
            paths: paths.to_vec(),
            device: device.clone(),
            backup: backup.clone(),
        })?;
        let want = profile.or(view.active);
        let found = view
            .profiles
            .iter()
            .find(|x| Some(x.index) == want)
            .or_else(|| (view.profiles.len() == 1).then(|| &view.profiles[0]))
            .cloned();
        match found {
            Some(f) => Ok(f),
            None => bail!("The source does not say which rate profile to use; pass profile."),
        }
    }

    /// Every rate profile of an FC: names, curves per axis, the throttle curve. Reads only.
    pub fn gear_rates(&self, p: &RatesParams) -> Result<RatesView> {
        if !p.any() {
            bail!("Pass a device, a backup id, or a Betaflight dump or diff file.");
        }
        let mut texts = Vec::new();
        let mut source = Vec::new();
        let snaps = crate::gear::backup::Snapshots::new(self.gear_store());
        let backup = if let Some(id) = p.backup.as_deref().filter(|d| !d.trim().is_empty()) {
            Some(snaps.get(id)?)
        } else if let Some(id) = p.device.as_deref().filter(|d| !d.trim().is_empty()) {
            let known = self.gear_store().devices()?.into_iter().any(|d| d.id == id);
            if !known {
                bail!("No device {id:?} in QuadCam's list (quadcam-cli gear devices).");
            }
            let b = snaps
                .list(id)
                .into_iter()
                .rev()
                .find(|b| {
                    b.files
                        .iter()
                        .any(|f| f.path == "dump all" || f.path == "diff all")
                })
                .with_context(|| {
                    format!("No FC backup of {id:?} yet: back it up, or pass a dump file.")
                })?;
            Some(b)
        } else {
            None
        };
        if let Some(b) = backup {
            // The dump holds every value; a diff alone shows only what differs from default.
            let file = ["dump all", "diff all"]
                .iter()
                .find_map(|c| b.files.iter().find(|f| f.path == *c))
                .with_context(|| format!("The backup {} has no dump.", b.id))?;
            let bytes = snaps.blobs().get(&crate::gear::backup::blob_of(file))?;
            texts.push(String::from_utf8_lossy(&bytes).into_owned());
            source.push(format!("{} {}", b.id, file.path));
        }
        for path in &p.paths {
            let meta = std::fs::metadata(path)
                .with_context(|| format!("cannot read {}", path.display()))?;
            if !meta.is_file() || meta.len() > MAX_FILE {
                bail!(
                    "{} is not a Betaflight dump, diff or CLI file.",
                    path.display()
                );
            }
            let bytes =
                std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
            texts.push(String::from_utf8_lossy(&bytes).into_owned());
            source.push(
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string()),
            );
        }
        // Later texts win: one config from all of them, in order.
        let config = Config::parse(&texts.join("\n"));
        Ok(rates::read(&config, source))
    }

    /// A profile as edited: its curves from the one implementation of the math, so the
    /// editor draws what the FC will do. With `to`, the profile is fitted onto that model
    /// first (the existing least-squares fit). Writes nothing.
    pub fn gear_rates_preview(&self, p: &RatesPreviewParams) -> Result<RatesPreview> {
        let v = &p.profile;
        let nums = v
            .axes
            .iter()
            .flat_map(|a| [a.rc_rate, a.srate, a.expo, a.rate_limit])
            .chain([v.throttle.mid, v.throttle.expo, v.throttle.limit_percent]);
        if v.axes.len() != 3 || nums.into_iter().any(|x| !x.is_finite()) {
            bail!("A rate profile has roll, pitch and yaw, and every value is a number.");
        }
        if rates::type_of(&v.rates_type).is_none() {
            bail!(
                "{:?} is not a rate model (betaflight, actual, quick, raceflight, kiss).",
                v.rates_type
            );
        }
        let prof = rates::profile_of(v);
        let (prof, fits) = match p.to.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            None => (prof, None),
            Some(t) => {
                let to =
                    rates::type_of(t).with_context(|| format!("{t:?} is not a rate model."))?;
                let (c, f) = rates::convert(&prof, to);
                (c, Some(f))
            }
        };
        let mut view = prof.view(v.index, v.name.clone(), v.active, v.complete);
        // The limit shown is the one given, not the clamp the math applies.
        for (a, src) in view.axes.iter_mut().zip(&v.axes) {
            a.rate_limit = src.rate_limit;
        }
        Ok(RatesPreview {
            profile: view,
            fit_error: fits.map_or(vec![0.0; 3], |f| f.iter().map(|x| x.max_diff).collect()),
            fit_share: fits.map_or(vec![0.0; 3], |f| {
                f.iter().map(|x| x.max_diff_share).collect()
            }),
        })
    }

    /// The sims on this Mac: each one's rate profiles, whether it runs, and, with a quad,
    /// how each profile differs from it. Reads only.
    pub fn gear_sims(&self, p: &SimsParams) -> Result<Vec<SimStatus>> {
        let running = sims::running_processes();
        self.gear_sims_at(p, &crate::paths::home_dir(), &|name| {
            running.iter().any(|r| r == name)
        })
    }

    /// `gear_sims` with the home folder and the process check given (tests).
    pub fn gear_sims_at(
        &self,
        p: &SimsParams,
        home: &std::path::Path,
        running: &dyn Fn(&str) -> bool,
    ) -> Result<Vec<SimStatus>> {
        let quad = if p.paths.is_empty() && p.device.is_none() && p.backup.is_none() {
            None
        } else {
            Some(self.quad_profile(&p.paths, &p.device, &p.backup, p.profile)?)
        };
        Ok(sims::status(home, running, quad.as_ref()))
    }
}
