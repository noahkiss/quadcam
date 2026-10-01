//! `Core`'s settings half: the settings file, saved places (and place search), and aircraft
//! profiles. Every surface reaches these through `Core` (and `dispatch`); every write goes
//! through `settings::update`, which keeps the keys it was not asked to change.

use super::Core;
use crate::geocode::{self, GeoResult};
use crate::metadata::{Location, Place, Profile};
use crate::session::Defaults;
use crate::settings::{self, Values};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The settings file: where it is, what it holds, and the defaults that come out of it.
#[derive(Debug, Clone, Serialize)]
pub struct SettingsView {
    pub path: PathBuf,
    /// The file's values, by file key.
    pub values: Values,
    /// What every surface uses: the values on top of the defaults.
    pub effective: Defaults,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaceRemoved {
    pub name: String,
    /// Profiles that used it as their default place; they now have none.
    pub profiles_cleared: Vec<String>,
}

/// Profile fields a save may set (everything but the name).
const PROFILE_FIELDS: &[&str] = &[
    "aircraft",
    "camera_make",
    "camera_model",
    "video_system",
    "keywords",
    "author",
    "place",
    "edgetx_models",
];

fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn names<T>(items: &[T], name: impl Fn(&T) -> &str) -> String {
    if items.is_empty() {
        return "none".into();
    }
    items.iter().map(name).collect::<Vec<_>>().join(", ")
}

fn clean_name(name: &str, what: &str) -> Result<String> {
    let n = name.trim();
    if n.is_empty() {
        bail!("A {what} needs a name.");
    }
    if n.chars().count() > 80 {
        bail!("A {what} name is at most 80 characters.");
    }
    Ok(n.to_string())
}

impl Core {
    pub fn settings_file(&self) -> Result<&Path> {
        self.settings_file
            .as_deref()
            .context("This quadcam has no settings file.")
    }

    /// Takes the defaults from the settings file again (after a write from anywhere).
    pub fn reload_settings(&self) {
        if let Some(f) = &self.settings_file {
            self.set_defaults(Defaults::with_app_settings(f));
        }
    }

    fn settings_written(&self) {
        self.reload_settings();
        self.hooks.settings_changed();
    }

    pub fn settings(&self) -> Result<SettingsView> {
        let path = self.settings_file()?.to_path_buf();
        let values = settings::read(&path)?;
        Ok(SettingsView {
            effective: Defaults::from_values(&values),
            path,
            values,
        })
    }

    /// Sets settings by file key or CLI/MCP name; null resets one to its default.
    pub fn settings_set(&self, changes: &Values) -> Result<SettingsView> {
        settings::set(self.settings_file()?, changes)?;
        self.settings_written();
        self.settings()
    }

    /// Changes the settings file under its lock and reports the change.
    fn settings_update<T>(&self, f: impl FnOnce(&mut Values) -> Result<T>) -> Result<T> {
        let (out, _) = settings::update(self.settings_file()?, f)?;
        self.settings_written();
        Ok(out)
    }

    pub fn places(&self) -> Result<Vec<Place>> {
        Ok(settings::list(
            &settings::read(self.settings_file()?)?,
            "places",
        ))
    }

    pub fn profiles(&self) -> Result<(Vec<Profile>, Option<String>)> {
        let v = settings::read(self.settings_file()?)?;
        let default = v
            .get("defaultProfile")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string);
        Ok((settings::list(&v, "profiles"), default))
    }

    /// Searches for an address or a place by name, with the `geocoder` setting's provider
    /// unless `provider` names another.
    pub fn place_search(
        &self,
        query: &str,
        provider: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<GeoResult>> {
        let provider = provider
            .map(str::to_string)
            .unwrap_or_else(|| self.defaults().geocoder);
        geocode::search(&provider, query, limit.unwrap_or(5))
    }

    /// Saves a place: creates it, or updates the one with this name (any case). `new_name`
    /// renames it, and profiles that use it follow.
    pub fn place_save(
        &self,
        name: &str,
        lat: Option<f64>,
        lon: Option<f64>,
        new_name: Option<&str>,
    ) -> Result<Place> {
        let name = clean_name(name, "place")?;
        let new_name = new_name.map(|n| clean_name(n, "place")).transpose()?;
        self.settings_update(|v| {
            let mut places: Vec<Place> = settings::list(v, "places");
            let at = places.iter().position(|p| same(&p.name, &name));
            let final_name = new_name
                .clone()
                .unwrap_or_else(|| at.map(|i| places[i].name.clone()).unwrap_or(name.clone()));
            if places
                .iter()
                .enumerate()
                .any(|(i, p)| Some(i) != at && same(&p.name, &final_name))
            {
                bail!("A place named {final_name:?} exists already.");
            }
            let place = match at {
                Some(i) => {
                    let old = &places[i];
                    Place {
                        name: final_name.clone(),
                        lat: lat.unwrap_or(old.lat),
                        lon: lon.unwrap_or(old.lon),
                    }
                }
                None => {
                    if new_name.is_some() {
                        bail!(
                            "No saved place {name:?} to rename; saved places: {}",
                            names(&places, |p| &p.name)
                        );
                    }
                    let (Some(lat), Some(lon)) = (lat, lon) else {
                        bail!("A new place needs lat and lon (or pick a place search result).");
                    };
                    Place {
                        name: final_name.clone(),
                        lat,
                        lon,
                    }
                }
            };
            Location {
                lat: place.lat,
                lon: place.lon,
                name: None,
            }
            .check()?;
            match at {
                Some(i) => {
                    let old = std::mem::replace(&mut places[i], place.clone());
                    if old.name != place.name {
                        let mut profiles: Vec<Profile> = settings::list(v, "profiles");
                        for p in &mut profiles {
                            if p.place.as_deref().is_some_and(|x| same(x, &old.name)) {
                                p.place = Some(place.name.clone());
                            }
                        }
                        v.insert("profiles".into(), serde_json::to_value(profiles)?);
                    }
                }
                None => places.push(place.clone()),
            }
            v.insert("places".into(), serde_json::to_value(places)?);
            Ok(place)
        })
    }

    /// Deletes a saved place. Profiles that used it lose their default place. Clips keep
    /// the location already written into them.
    pub fn place_delete(&self, name: &str) -> Result<PlaceRemoved> {
        self.settings_update(|v| {
            let mut places: Vec<Place> = settings::list(v, "places");
            let at = places
                .iter()
                .position(|p| same(&p.name, name))
                .with_context(|| {
                    format!(
                        "No saved place {name:?}; saved places: {}",
                        names(&places, |p| &p.name)
                    )
                })?;
            let gone = places.remove(at);
            let mut profiles: Vec<Profile> = settings::list(v, "profiles");
            let mut cleared = Vec::new();
            for p in &mut profiles {
                if p.place.as_deref().is_some_and(|x| same(x, &gone.name)) {
                    p.place = None;
                    cleared.push(p.name.clone());
                }
            }
            v.insert("places".into(), serde_json::to_value(places)?);
            if !cleared.is_empty() {
                v.insert("profiles".into(), serde_json::to_value(profiles)?);
            }
            Ok(PlaceRemoved {
                name: gone.name,
                profiles_cleared: cleared,
            })
        })
    }

    /// Saves an aircraft profile: creates it, or changes the given fields of the one with
    /// this name (any case). `new_name` renames it; the default follows. Clips already in
    /// the library keep the name written into them.
    pub fn profile_save(
        &self,
        name: &str,
        fields: &Values,
        new_name: Option<&str>,
    ) -> Result<Profile> {
        let name = clean_name(name, "profile")?;
        let new_name = new_name.map(|n| clean_name(n, "profile")).transpose()?;
        if let Some(k) = fields
            .keys()
            .find(|k| !PROFILE_FIELDS.contains(&k.as_str()))
        {
            bail!(
                "unknown profile field {k:?}; fields: name, {}",
                PROFILE_FIELDS.join(", ")
            );
        }
        self.settings_update(|v| {
            let mut profiles: Vec<Profile> = settings::list(v, "profiles");
            let places: Vec<Place> = settings::list(v, "places");
            let at = profiles.iter().position(|p| same(&p.name, &name));
            if at.is_none() && new_name.is_some() {
                bail!(
                    "No profile {name:?} to rename; profiles: {}",
                    names(&profiles, |p| &p.name)
                );
            }
            let base = match at {
                Some(i) => profiles[i].clone(),
                None => Profile {
                    name: name.clone(),
                    video_system: "Analog".into(),
                    ..Default::default()
                },
            };
            let mut merged = serde_json::to_value(&base)?;
            for (k, x) in fields {
                merged[k] = x.clone();
            }
            let mut p: Profile = serde_json::from_value(merged).context(
                "profile fields: text for aircraft, camera_make, camera_model, video_system, author and place; lists of text for keywords and edgetx_models",
            )?;
            p.name = new_name.clone().unwrap_or(p.name);
            p.keywords = crate::metadata::clean_keywords(p.keywords.iter().map(String::as_str));
            p.edgetx_models = crate::metadata::clean_keywords(p.edgetx_models.iter().map(String::as_str));
            p.place = match p.place.as_deref().map(str::trim) {
                None | Some("") => None,
                Some(n) => Some(
                    places
                        .iter()
                        .find(|x| same(&x.name, n))
                        .map(|x| x.name.clone())
                        .with_context(|| {
                            format!(
                                "No saved place {n:?}; saved places: {}",
                                names(&places, |x| &x.name)
                            )
                        })?,
                ),
            };
            if profiles
                .iter()
                .enumerate()
                .any(|(i, x)| Some(i) != at && same(&x.name, &p.name))
            {
                bail!("A profile named {:?} exists already.", p.name);
            }
            let old_name = at.map(|i| profiles[i].name.clone());
            match at {
                Some(i) => profiles[i] = p.clone(),
                None => profiles.push(p.clone()),
            }
            v.insert("profiles".into(), serde_json::to_value(&profiles)?);
            let default = v.get("defaultProfile").and_then(Value::as_str).unwrap_or("");
            if old_name.is_some_and(|o| same(&o, default)) || (profiles.len() == 1 && default.is_empty()) {
                v.insert("defaultProfile".into(), json!(p.name));
            }
            Ok(p)
        })
    }

    /// Deletes an aircraft profile. When it was the default, the first one left becomes it.
    pub fn profile_delete(&self, name: &str) -> Result<Profile> {
        self.settings_update(|v| {
            let mut profiles: Vec<Profile> = settings::list(v, "profiles");
            let at = profiles
                .iter()
                .position(|p| same(&p.name, name))
                .with_context(|| {
                    format!(
                        "No profile {name:?}; profiles: {}",
                        names(&profiles, |p| &p.name)
                    )
                })?;
            let gone = profiles.remove(at);
            v.insert("profiles".into(), serde_json::to_value(&profiles)?);
            let default = v
                .get("defaultProfile")
                .and_then(Value::as_str)
                .unwrap_or("");
            if same(default, &gone.name) {
                v.insert(
                    "defaultProfile".into(),
                    json!(profiles.first().map(|p| p.name.as_str()).unwrap_or("")),
                );
            }
            Ok(gone)
        })
    }

    /// Sets the default profile; an empty name means none.
    pub fn profile_default(&self, name: &str) -> Result<Option<String>> {
        self.settings_update(|v| {
            let profiles: Vec<Profile> = settings::list(v, "profiles");
            let chosen = if name.trim().is_empty() {
                None
            } else {
                Some(
                    profiles
                        .iter()
                        .find(|p| same(&p.name, name))
                        .map(|p| p.name.clone())
                        .with_context(|| {
                            format!(
                                "No profile {name:?}; profiles: {}",
                                names(&profiles, |p| &p.name)
                            )
                        })?,
                )
            };
            v.insert(
                "defaultProfile".into(),
                json!(chosen.clone().unwrap_or_default()),
            );
            Ok(chosen)
        })
    }
}
