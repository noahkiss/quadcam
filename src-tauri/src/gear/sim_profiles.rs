//! Sim profiles fitted from a quad's own logs (sim design 6.1, 6.3): one file per profile,
//! `<gear>/sim/profiles/<id>.json`, written only through `Core`. The built-in presets ship
//! in `quadcam-sim`; a fitted profile is the person's data and never ships.

use super::blobs;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use quadcam_sim::SimProfile;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::io::Write;
use std::path::{Path, PathBuf};

/// `<gear>/sim/profiles/`.
pub fn dir(gear: &Path) -> PathBuf {
    gear.join("sim").join("profiles")
}

/// `<gear>/sim/profiles/<id>.json`.
pub fn file(gear: &Path, id: &str) -> PathBuf {
    dir(gear).join(format!("{id}.json"))
}

/// A fitted profile and where it came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedProfile {
    pub profile: SimProfile,
    /// The built-in profile the fit started from.
    pub base: String,
    #[serde(default)]
    pub aircraft: Option<String>,
    pub fitted_at: DateTime<Utc>,
    /// The logs and the FC settings the fit read.
    #[serde(default)]
    pub sources: Vec<String>,
}

/// One row of the profile list: a built-in preset or a fitted profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SimProfileInfo {
    pub id: String,
    pub label: String,
    /// `preset` (ships with QuadCam) or `fitted` (from this person's logs).
    pub kind: String,
    /// A fitted profile's preset.
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub aircraft: Option<String>,
    #[serde(default)]
    pub fitted_at: Option<DateTime<Utc>>,
    pub mass_g: f64,
    pub wheelbase_mm: f64,
}

impl SimProfileInfo {
    pub fn preset(p: &SimProfile) -> SimProfileInfo {
        SimProfileInfo {
            id: p.id.clone(),
            label: p.label.clone(),
            kind: "preset".into(),
            base: None,
            aircraft: None,
            fitted_at: None,
            mass_g: p.frame.mass.value * 1000.0,
            wheelbase_mm: p.frame.wheelbase.value * 1000.0,
        }
    }

    pub fn fitted(s: &SavedProfile) -> SimProfileInfo {
        SimProfileInfo {
            kind: "fitted".into(),
            base: Some(s.base.clone()),
            aircraft: s.aircraft.clone(),
            fitted_at: Some(s.fitted_at),
            ..SimProfileInfo::preset(&s.profile)
        }
    }
}

/// A profile id: 1 to 64 lowercase letters, digits, `-` and `_`, not a preset's.
pub fn check_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if !ok {
        bail!("A sim profile id is 1 to 64 lowercase letters, digits, - and _: {id:?}.");
    }
    if quadcam_sim::preset(id).is_some() {
        bail!("{id:?} is a built-in profile; give the fitted profile another id.");
    }
    Ok(())
}

/// A profile id from an aircraft name: `Meteor75 Pro II` gives `meteor75-pro-ii`.
pub fn id_from(name: &str) -> String {
    let mut s = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s: String = s.trim_end_matches('-').chars().take(64).collect();
    s.trim_end_matches('-').to_string()
}

/// The digest a save checks: the profile as it would be written.
pub fn digest(p: &SimProfile) -> Result<String> {
    Ok(blobs::hash(&serde_json::to_vec(p)?))
}

/// Every fitted profile, by id. A file that does not read is skipped.
pub fn list(gear: &Path) -> Vec<SavedProfile> {
    let Ok(rd) = std::fs::read_dir(dir(gear)) else {
        return Vec::new();
    };
    let mut out: Vec<SavedProfile> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| read_file(&p).ok())
        .collect();
    out.sort_by(|a, b| a.profile.id.cmp(&b.profile.id));
    out
}

fn read_file(p: &Path) -> Result<SavedProfile> {
    let b = std::fs::read(p).with_context(|| format!("cannot read {}", p.display()))?;
    serde_json::from_slice(&b).with_context(|| format!("{} is not a sim profile", p.display()))
}

/// A fitted profile by id; None when there is none.
pub fn read(gear: &Path, id: &str) -> Result<Option<SavedProfile>> {
    if check_id(id).is_err() {
        return Ok(None);
    }
    let p = file(gear, id);
    if !p.exists() {
        return Ok(None);
    }
    read_file(&p).map(Some)
}

/// Writes a profile through a temp file of this process, a sync and a rename. A link at
/// the path is refused: the file is QuadCam's own.
pub fn write(gear: &Path, s: &SavedProfile) -> Result<PathBuf> {
    check_id(&s.profile.id)?;
    let p = file(gear, &s.profile.id);
    if p.is_symlink() {
        bail!(
            "{} is a link; QuadCam writes only its own file there.",
            p.display()
        );
    }
    let d = dir(gear);
    std::fs::create_dir_all(&d).with_context(|| format!("creating {}", d.display()))?;
    let tmp = d.join(format!(".{}.json.{}.tmp", s.profile.id, std::process::id()));
    let written = (|| -> Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(s)?)?;
        f.sync_all()?;
        Ok(())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.context(format!("cannot write {}", p.display())));
    }
    if let Err(e) = std::fs::rename(&tmp, &p) {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow::Error::from(e).context(format!("cannot write {}", p.display())));
    }
    Ok(p)
}

/// Removes a fitted profile. False when there was none.
pub fn delete(gear: &Path, id: &str) -> Result<bool> {
    check_id(id)?;
    let p = file(gear, id);
    match std::fs::remove_file(&p) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("cannot remove {}", p.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(id: &str) -> SavedProfile {
        let mut profile = quadcam_sim::preset("meteor75").unwrap();
        profile.id = id.into();
        SavedProfile {
            profile,
            base: "meteor75".into(),
            aircraft: Some("Meteor75".into()),
            fitted_at: Utc::now(),
            sources: vec!["log.bbl".into()],
        }
    }

    #[test]
    fn profiles_save_list_read_and_delete() {
        let t = tempfile::tempdir().unwrap();
        let path = write(t.path(), &saved("meteor75-pro-ii")).unwrap();
        assert!(path.ends_with("sim/profiles/meteor75-pro-ii.json"));
        assert_eq!(list(t.path()).len(), 1);
        let back = read(t.path(), "meteor75-pro-ii").unwrap().unwrap();
        assert_eq!(back.profile.id, "meteor75-pro-ii");
        assert!(read(t.path(), "other").unwrap().is_none());
        assert!(delete(t.path(), "meteor75-pro-ii").unwrap());
        assert!(!delete(t.path(), "meteor75-pro-ii").unwrap());
        // No temp file is left behind.
        assert_eq!(std::fs::read_dir(dir(t.path())).unwrap().count(), 0);
    }

    #[test]
    fn ids_are_checked_and_made_from_names() {
        assert!(check_id("meteor75").is_err(), "a preset's id");
        assert!(check_id("../x").is_err());
        assert!(check_id("").is_err());
        assert!(check_id("air65-ii").is_ok());
        assert_eq!(id_from("Meteor75 Pro II"), "meteor75-pro-ii");
        assert_eq!(id_from("  Air65 II (O4)  "), "air65-ii-o4");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_at_the_path_is_refused() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir(t.path())).unwrap();
        let elsewhere = t.path().join("elsewhere.json");
        std::fs::write(&elsewhere, "{}").unwrap();
        std::os::unix::fs::symlink(&elsewhere, file(t.path(), "x")).unwrap();
        assert!(write(t.path(), &saved("x")).is_err());
        assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "{}");
    }
}
