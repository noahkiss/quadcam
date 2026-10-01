//! Params of the library methods.

use crate::core::{LibEdit, LibUpdate, Moved};
use crate::library::Flag;
use crate::moments::Span;
use crate::trim::RemovedCuts;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::PathBuf;

/// One library clip, by id.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct ClipIdParams {
    pub id: String,
}

/// `library_rate`: stars (0 clears) and a pick or reject flag for these clips.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct RateParams {
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default)]
    pub rating: Option<u8>,
    #[serde(default)]
    pub flag: Option<Flag>,
}

/// `library_edit`: one clip's details. Missing fields stay as they are.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct LibraryEditParams {
    pub id: String,
    #[serde(flatten)]
    pub edit: LibEdit,
}

/// `library_update`: stars, flag, name and details for these clips, checked before any
/// file changes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct LibraryUpdateParams {
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(flatten)]
    pub update: LibUpdate,
}

/// `library_rename`: a clip's new short name.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct RenameParams {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
}

/// `library_cuts`: a clip's new cut list, and what happens to exported cuts it drops.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct LibraryCutsParams {
    pub id: String,
    #[serde(default)]
    pub cuts: Vec<Span>,
    #[serde(default)]
    pub removed_cuts: Option<RemovedCuts>,
}

/// Library clips, by id. For `library_apply_name_format` and `library_strips`, none
/// means every clip.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct IdsParams {
    #[serde(default)]
    pub ids: Vec<String>,
}

/// `library_untrash`: the files `library_trash` moved.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct UntrashParams {
    pub moved: Vec<Moved>,
}

/// `library_photos`: the clips to add with their cuts, and the album (None: the setting).
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct LibraryPhotosParams {
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default)]
    pub album: Option<String>,
}

/// `card_status`: a card's mount point.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct MountParams {
    pub mount: PathBuf,
}
