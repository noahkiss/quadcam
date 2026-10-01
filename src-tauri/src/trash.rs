//! Moving files to the Trash. The app uses the macOS Trash (Finder can put a file back).
//! A process started by cargo gets a stand-in that moves files into a temporary folder, so
//! no test ever fills the real Trash; see `real_trash`.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub trait Trash: Send + Sync {
    /// Moves `path` to the Trash. Returns where it went, when the Trash says.
    fn trash(&self, path: &Path) -> Result<Option<PathBuf>>;
}

/// Moves files into a folder. For tests, and for `QUADCAM_TRASH=<folder>`.
pub struct DirTrash(pub PathBuf);

impl Trash for DirTrash {
    fn trash(&self, path: &Path) -> Result<Option<PathBuf>> {
        std::fs::create_dir_all(&self.0)?;
        let name = path.file_name().context("no file name")?;
        let mut dst = self.0.join(name);
        let mut n = 2;
        while dst.exists() {
            dst = self.0.join(format!("{n}-{}", name.to_string_lossy()));
            n += 1;
        }
        move_file(path, &dst)?;
        Ok(Some(dst))
    }
}

#[cfg(target_os = "macos")]
pub struct MacTrash;

#[cfg(target_os = "macos")]
impl Trash for MacTrash {
    fn trash(&self, path: &Path) -> Result<Option<PathBuf>> {
        use objc2_foundation::{NSFileManager, NSString, NSURL};
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        let fm = NSFileManager::defaultManager();
        let mut went = None;
        fm.trashItemAtURL_resultingItemURL_error(&url, Some(&mut went))
            .map_err(|e| anyhow::anyhow!("{}", e.localizedDescription()))
            .with_context(|| format!("moving {} to the Trash", path.display()))?;
        Ok(went
            .and_then(|u| u.path())
            .map(|p| PathBuf::from(p.to_string())))
    }
}

/// Renames `from` to `to`, or copies and deletes it across volumes.
pub fn move_file(from: &Path, to: &Path) -> Result<()> {
    std::fs::rename(from, to)
        .or_else(|_| std::fs::copy(from, to).and_then(|_| std::fs::remove_file(from)))
        .with_context(|| format!("moving {} to {}", from.display(), to.display()))
}

/// The Trash to use. `QUADCAM_TRASH=<folder>` moves files into that folder instead. A
/// process started by cargo (it carries `CARGO_MANIFEST_DIR`) always gets a temporary
/// folder, so tests never touch the real Trash.
pub fn real_trash() -> Arc<dyn Trash> {
    if let Some(dir) = std::env::var_os("QUADCAM_TRASH").filter(|d| !d.is_empty()) {
        return Arc::new(DirTrash(PathBuf::from(dir)));
    }
    if std::env::var_os("CARGO_MANIFEST_DIR").is_some() {
        return Arc::new(DirTrash(
            std::env::temp_dir().join(format!("quadcam-test-trash-{}", std::process::id())),
        ));
    }
    #[cfg(target_os = "macos")]
    return Arc::new(MacTrash);
    #[cfg(not(target_os = "macos"))]
    return Arc::new(DirTrash(std::env::temp_dir().join("quadcam-trash")));
}
