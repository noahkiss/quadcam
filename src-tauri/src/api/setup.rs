//! Params and results of the settings, places and profiles methods.

use crate::metadata::Profile;
use crate::settings::Values;
use serde::{Deserialize, Serialize};
use specta::Type;

/// `settings_set`: settings by file key or CLI/MCP name; null resets one.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct SetSettingsParams {
    #[specta(type = std::collections::BTreeMap<String, specta_typescript::Unknown>)]
    pub values: Values,
}

/// `place_search`: an address or a place name, and an optional provider and result limit.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct SearchParams {
    pub query: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// `place_save`: creates a place or changes the one with this name; `new_name` renames it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct PlaceSaveParams {
    pub name: String,
    #[serde(default)]
    pub new_name: Option<String>,
    #[serde(default)]
    pub lat: Option<f64>,
    #[serde(default)]
    pub lon: Option<f64>,
}

/// A place or profile, by name.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct NameParams {
    pub name: String,
}

/// `profile_save`: creates a profile or changes the given fields of the one with this name;
/// `new_name` renames it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct ProfileSaveParams {
    pub name: String,
    #[serde(default)]
    pub new_name: Option<String>,
    #[serde(default)]
    #[specta(type = std::collections::BTreeMap<String, specta_typescript::Unknown>)]
    pub fields: Values,
}

/// `profiles`' answer: every profile and the default's name.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ProfilesView {
    pub profiles: Vec<Profile>,
    pub default_profile: Option<String>,
}
