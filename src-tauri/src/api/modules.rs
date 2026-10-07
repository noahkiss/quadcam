//! Params of the module methods.

use serde::{Deserialize, Serialize};
use specta::Type;

/// `module_install`: a module by name. `confirm` says the person saw its license prompt.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct ModuleParams {
    pub name: String,
    #[serde(default)]
    pub confirm: bool,
}
