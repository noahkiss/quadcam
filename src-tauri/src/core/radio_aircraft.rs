//! A radio's aircraft. An FC sits in one aircraft, so its device record names that one
//! (`Device.aircraft`). A radio flies many: each aircraft profile names its radio
//! (`ProfileGear.radio`), and that is the one source of truth. Reads list a radio's aircraft
//! in `Device.radio_aircraft`, each with its EdgeTX model and whether that model is on the
//! mounted card, else in the latest backup.
//!
//! `gear.json` files from before kept one aircraft on a radio too. `move_radio_links` moves
//! that link onto the profile the first time Core sees the file: the profile gets this radio
//! when it names none yet, and the radio's field goes. A link the profile cannot take (it
//! names another radio, or the profile is gone) stays in the entry as `legacy_aircraft`, so
//! nothing is lost.

use super::switchmap::{CardSource, InBackup};
use super::Core;
use crate::gear::edgetx::card::{is_model_file, selected_in};
use crate::gear::edgetx::model::model_name;
use crate::gear::edgetx::yaml::Doc;
use crate::gear::model::{Device, DeviceKind, RadioAircraft};
use crate::gear::store::{Store, Values};
use crate::metadata::Profile;
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The `gear.json` the move last checked, and its stamp then.
pub(super) type Checked = (PathBuf, Option<(std::time::SystemTime, u64)>);

/// The key a radio link the profiles could not take moves to, in the device's entry.
pub const LEGACY_AIRCRAFT: &str = "legacy_aircraft";

/// The models on a radio: where QuadCam looked, each model file with its name, and the
/// selected one.
pub(super) struct Models {
    pub checked: String,
    pub files: Vec<(String, String)>,
    pub selected: Option<String>,
}

fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// The models of a card source, from the model file paths given.
fn models_in(card: &dyn CardSource, paths: Vec<String>, checked: &str) -> Models {
    let mut files: Vec<(String, String)> = paths
        .iter()
        .filter_map(|p| p.strip_prefix("MODELS/"))
        .filter(|f| is_model_file(f))
        .map(|f| {
            let name = card
                .read(&format!("MODELS/{f}"))
                .and_then(|b| Doc::parse(f, &b).ok())
                .and_then(|d| model_name(&d).ok().flatten())
                .unwrap_or_default();
            (f.to_string(), name)
        })
        .collect();
    files.sort();
    let selected = card
        .read("RADIO/radio.yml")
        .and_then(|b| Doc::parse("radio.yml", &b).ok())
        .and_then(|d| selected_in(&d));
    Models {
        checked: checked.to_string(),
        files,
        selected,
    }
}

/// A mounted card: reads its `MODELS/` folder only, never the whole card.
struct Mounted(PathBuf);

impl CardSource for Mounted {
    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        std::fs::read(self.0.join(rel)).ok()
    }
    fn files(&self) -> Vec<String> {
        std::fs::read_dir(self.0.join("MODELS"))
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .map(|f| format!("MODELS/{f}"))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The aircraft whose profiles name `radio`, in profile order, each with its model as
/// `models` shows it (None: QuadCam could not look).
pub(super) fn aircraft_on(
    profiles: &[Profile],
    radio: &str,
    models: Option<&Models>,
) -> Vec<RadioAircraft> {
    profiles
        .iter()
        .filter(|p| p.gear.radio.as_deref() == Some(radio))
        .map(|p| {
            let named = p
                .gear
                .edgetx_model
                .as_deref()
                .map(str::trim)
                .filter(|m| !m.is_empty());
            let hit = models.and_then(|m| {
                m.files.iter().find(|(f, n)| match named {
                    Some(file) => same(f, file),
                    None => !n.is_empty() && p.edgetx_models.iter().any(|x| same(x, n)),
                })
            });
            let model = named
                .map(str::to_string)
                .or_else(|| hit.map(|h| h.0.clone()));
            RadioAircraft {
                profile: p.name.clone(),
                selected: match (models.and_then(|m| m.selected.as_deref()), &model) {
                    (Some(s), Some(f)) => same(s, f),
                    _ => false,
                },
                model_name: hit.map(|h| h.1.clone()).filter(|n| !n.is_empty()),
                checked: models.map(|m| m.checked.clone()),
                found: models.map(|_| hit.is_some()),
                model,
            }
        })
        .collect()
}

/// Moves the radios' own aircraft links onto the profiles: a profile named by a radio's
/// `aircraft` that names no radio gets that radio. True when a profile changed.
pub(super) fn move_to_profiles(gear: &Values, settings: &mut Values) -> bool {
    let links = radio_links(gear);
    let Some(Value::Array(profiles)) = settings.get_mut("profiles") else {
        return false;
    };
    let mut changed = false;
    for (radio, aircraft) in links {
        let Some(p) = profiles
            .iter_mut()
            .find(|p| p["name"].as_str().is_some_and(|n| same(n, &aircraft)))
        else {
            continue;
        };
        let has = p["gear"]["radio"].as_str().is_some_and(|r| !r.is_empty());
        if !has {
            set_radio(p, Some(&radio));
            changed = true;
        }
    }
    changed
}

/// Drops the radios' own aircraft links once the profiles hold them. A link the profile
/// named does not hold (it names another radio, or it is gone) moves to
/// `legacy_aircraft`. True when an entry changed.
pub(super) fn drop_from_radios(gear: &mut Values, settings: &Values) -> bool {
    let profiles = settings.get("profiles").and_then(Value::as_array);
    let Some(Value::Array(devices)) = gear.get_mut(crate::gear::store::DEVICES) else {
        return false;
    };
    let mut changed = false;
    for d in devices.iter_mut().filter(|d| is_radio(d)) {
        let Value::Object(m) = d else {
            continue;
        };
        let Some(a) = m
            .get("aircraft")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let id = m.get("id").and_then(Value::as_str).unwrap_or("");
        let held = profiles.into_iter().flatten().any(|p| {
            p["name"].as_str().is_some_and(|n| same(n, &a))
                && p["gear"]["radio"].as_str() == Some(id)
        });
        if !held && !a.trim().is_empty() {
            m.insert(LEGACY_AIRCRAFT.into(), Value::String(a));
        }
        m.insert("aircraft".into(), Value::Null);
        changed = true;
    }
    changed
}

fn is_radio(d: &Value) -> bool {
    d["kind"].as_str() == Some("radio")
}

/// Each radio's own aircraft link, (radio id, profile name).
fn radio_links(gear: &Values) -> Vec<(String, String)> {
    gear.get(crate::gear::store::DEVICES)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|d| is_radio(d))
        .filter_map(|d| {
            Some((
                d["id"].as_str()?.to_string(),
                d["aircraft"]
                    .as_str()
                    .filter(|a| !a.trim().is_empty())?
                    .to_string(),
            ))
        })
        .collect()
}

/// Sets or clears a profile entry's `gear.radio`, keeping every other key.
fn set_radio(p: &mut Value, radio: Option<&str>) {
    let Value::Object(m) = p else { return };
    let g = m
        .entry("gear")
        .or_insert_with(|| Value::Object(Default::default()));
    if !g.is_object() {
        *g = Value::Object(Default::default());
    }
    let Value::Object(g) = g else { return };
    match radio {
        Some(r) => {
            g.insert("radio".into(), Value::String(r.to_string()));
        }
        None => {
            g.remove("radio");
        }
    }
    if g.is_empty() {
        m.remove("gear");
    }
}

impl Core {
    /// Moves the radio links of a `gear.json` from before onto the profiles, once per
    /// version of the file. Errors leave the file as it was; the next look tries again.
    pub(super) fn move_radio_links(&self, store: &Store) {
        let stamp = store.stamp();
        let key = (store.gear_file(), stamp);
        {
            let mut seen = self.radio_links_checked.lock().unwrap();
            if seen.as_ref() == Some(&key) {
                return;
            }
            *seen = Some(key);
        }
        if stamp.is_none() || self.settings_file.is_none() {
            return;
        }
        let gear = match store.read() {
            Ok(g) => g,
            Err(_) => return,
        };
        if radio_links(&gear).is_empty() {
            return;
        }
        let moved = self.settings_update(|v| Ok(move_to_profiles(&gear, v)));
        let Ok(settings) = moved.and_then(|_| crate::settings::read(self.settings_file()?)) else {
            *self.radio_links_checked.lock().unwrap() = None;
            return;
        };
        match store.update(|g| Ok(drop_from_radios(g, &settings))) {
            Ok(_) => {
                *self.radio_links_checked.lock().unwrap() =
                    Some((store.gear_file(), store.stamp()));
            }
            Err(_) => *self.radio_links_checked.lock().unwrap() = None,
        }
    }

    /// Puts an aircraft on a radio: that profile's `gear.radio` becomes this radio. An
    /// empty name takes every aircraft off it.
    pub(super) fn radio_link(&self, radio: &str, aircraft: &str) -> Result<()> {
        let aircraft = aircraft.trim();
        self.settings_update(|v| {
            let Some(Value::Array(profiles)) = v.get_mut("profiles") else {
                if aircraft.is_empty() {
                    return Ok(());
                }
                anyhow::bail!("No aircraft profile {aircraft:?}. Profiles: none");
            };
            if aircraft.is_empty() {
                for p in profiles
                    .iter_mut()
                    .filter(|p| p["gear"]["radio"].as_str() == Some(radio))
                {
                    set_radio(p, None);
                }
                return Ok(());
            }
            let names: Vec<String> = profiles
                .iter()
                .filter_map(|p| p["name"].as_str().map(str::to_string))
                .collect();
            let p = profiles
                .iter_mut()
                .find(|p| p["name"].as_str().is_some_and(|n| same(n, aircraft)))
                .with_context(|| {
                    format!(
                        "No aircraft profile {aircraft:?}. Profiles: {}",
                        if names.is_empty() {
                            "none".to_string()
                        } else {
                            names.join(", ")
                        }
                    )
                })?;
            set_radio(p, Some(radio));
            Ok(())
        })
    }

    /// The models on a radio: its mounted card when `mount` is given, else its latest
    /// backup. None when there is neither.
    pub(super) fn radio_models(&self, radio: &str, mount: Option<&Path>) -> Option<Models> {
        if let Some(m) = mount {
            let card = Mounted(m.to_path_buf());
            let paths = card.files();
            return Some(models_in(&card, paths, "card"));
        }
        let snaps = self.snapshots();
        let backup = snaps.latest(radio)?;
        let paths = backup.files.iter().map(|f| f.path.clone()).collect();
        let card = InBackup {
            snaps: &snaps,
            backup,
        };
        Some(models_in(&card, paths, "latest backup"))
    }

    /// Fills `radio_aircraft` on a radio's record (`mount`: its card, mounted now). Other
    /// kinds stay as they are.
    pub(super) fn fill_radio_aircraft(
        &self,
        d: &mut Device,
        profiles: &[Profile],
        mount: Option<&Path>,
    ) {
        if d.kind != DeviceKind::Radio {
            return;
        }
        let models = self.radio_models(&d.id, mount);
        d.radio_aircraft = Some(aircraft_on(profiles, &d.id, models.as_ref()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(v: Value) -> Values {
        v.as_object().unwrap().clone()
    }

    fn profile(name: &str, gear: Value) -> Profile {
        serde_json::from_value(json!({
            "name": name, "aircraft": "", "camera_make": "", "camera_model": "",
            "video_system": "Analog", "keywords": [], "author": "", "place": null,
            "edgetx_models": [name.to_uppercase()], "gear": gear,
        }))
        .unwrap()
    }

    #[test]
    fn a_radios_link_moves_onto_a_profile_without_a_radio() {
        let mut gear = obj(json!({"devices": [
            {"id": "radio-a", "kind": "radio", "aircraft": "Whoop", "name": "R", "extra": 1},
            {"id": "fc-a", "kind": "fc", "aircraft": "Whoop"},
        ]}));
        let mut settings = obj(json!({"profiles": [
            {"name": "whoop", "aircraft": "", "keep": true, "gear": {"fc": "fc-a"}},
        ], "other": 2}));
        assert!(move_to_profiles(&gear, &mut settings));
        assert_eq!(
            settings["profiles"][0]["gear"],
            json!({"fc": "fc-a", "radio": "radio-a"})
        );
        assert_eq!(settings["profiles"][0]["keep"], json!(true));
        assert!(drop_from_radios(&mut gear, &settings));
        let r = &gear["devices"][0];
        assert_eq!(r["aircraft"], Value::Null);
        assert!(r.get(LEGACY_AIRCRAFT).is_none());
        assert_eq!(r["extra"], json!(1));
        // An FC keeps its one aircraft.
        assert_eq!(gear["devices"][1]["aircraft"], json!("Whoop"));
        // Done once: a second pass changes nothing.
        assert!(!move_to_profiles(&gear, &mut settings));
        assert!(!drop_from_radios(&mut gear, &settings));
    }

    #[test]
    fn a_link_the_profile_cannot_take_is_kept() {
        let mut gear = obj(json!({"devices": [
            {"id": "radio-a", "kind": "radio", "aircraft": "Whoop"},
            {"id": "radio-b", "kind": "radio", "aircraft": "Gone"},
        ]}));
        let mut settings = obj(json!({"profiles": [
            {"name": "Whoop", "gear": {"radio": "radio-z"}},
        ]}));
        assert!(!move_to_profiles(&gear, &mut settings));
        assert_eq!(settings["profiles"][0]["gear"]["radio"], json!("radio-z"));
        assert!(drop_from_radios(&mut gear, &settings));
        assert_eq!(gear["devices"][0]["aircraft"], Value::Null);
        assert_eq!(gear["devices"][0][LEGACY_AIRCRAFT], json!("Whoop"));
        assert_eq!(gear["devices"][1][LEGACY_AIRCRAFT], json!("Gone"));
    }

    #[test]
    fn a_radios_aircraft_come_from_the_profiles() {
        let profiles = vec![
            profile(
                "Whoop",
                json!({"radio": "radio-a", "edgetx_model": "model02.yml"}),
            ),
            profile("Cine", json!({"radio": "radio-a"})),
            profile("Other", json!({"radio": "radio-b"})),
            profile(
                "Lost",
                json!({"radio": "radio-a", "edgetx_model": "model09.yml"}),
            ),
        ];
        let models = Models {
            checked: "latest backup".into(),
            files: vec![
                ("model01.yml".into(), "CINE".into()),
                ("model02.yml".into(), "Whoopy".into()),
            ],
            selected: Some("model02.yml".into()),
        };
        let list = aircraft_on(&profiles, "radio-a", Some(&models));
        let names: Vec<&str> = list.iter().map(|a| a.profile.as_str()).collect();
        assert_eq!(names, ["Whoop", "Cine", "Lost"]);
        assert_eq!(list[0].model.as_deref(), Some("model02.yml"));
        assert_eq!(list[0].model_name.as_deref(), Some("Whoopy"));
        assert_eq!(list[0].found, Some(true));
        assert!(list[0].selected);
        // No model file named: the model whose name is one of the profile's.
        assert_eq!(list[1].model.as_deref(), Some("model01.yml"));
        assert!(!list[1].selected);
        assert_eq!(list[2].found, Some(false));
        assert_eq!(list[2].model.as_deref(), Some("model09.yml"));
        // Nowhere to look.
        let blind = aircraft_on(&profiles, "radio-a", None);
        assert_eq!(blind[0].found, None);
        assert_eq!(blind[1].model, None);
        assert_eq!(blind[0].checked, None);
    }

    #[test]
    fn set_radio_keeps_the_other_gear() {
        let mut p = json!({"name": "A", "gear": {"fc": "fc-1", "radio": "r"}});
        set_radio(&mut p, None);
        assert_eq!(p["gear"], json!({"fc": "fc-1"}));
        let mut q = json!({"name": "B", "gear": {"radio": "r"}});
        set_radio(&mut q, None);
        assert!(q.get("gear").is_none());
        set_radio(&mut q, Some("s"));
        assert_eq!(q["gear"], json!({"radio": "s"}));
    }
}
