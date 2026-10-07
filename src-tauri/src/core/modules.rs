//! `Core`'s module methods: list, install (after the license prompt), remove, and the update
//! check. The work is in `crate::modules`.

use super::Core;
use crate::modules::ModuleStatus;
use anyhow::Result;

impl Core {
    /// Every module with its pin, a newer pin from the last check, and what is installed.
    pub fn modules(&self) -> Vec<ModuleStatus> {
        self.modules.list()
    }

    /// Downloads and installs a module's newest known pin. Without `confirm`, refuses with
    /// the license prompt and downloads nothing.
    pub fn module_install(&self, name: &str, confirm: bool) -> Result<ModuleStatus> {
        let s = self.modules.install(name, confirm)?;
        self.hooks.settings_changed();
        Ok(s)
    }

    /// Deletes a module's folder.
    pub fn module_remove(&self, name: &str) -> Result<ModuleStatus> {
        let s = self.modules.remove(name)?;
        self.hooks.settings_changed();
        Ok(s)
    }

    /// Reads the newest pins from the latest QuadCam release. Installs nothing.
    pub fn modules_check(&self) -> Result<Vec<ModuleStatus>> {
        self.modules.check()
    }
}
