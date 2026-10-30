//! `Core`'s staging folders under `<cache>/staging`: who owns each one, and when one goes.
//!
//! A stage writes `.owners` into its folder: the session files that staged it, one per line.
//! A load that replaces a session deletes the old session's folder (`release_staging`). The
//! sweep (`sweep_staging`, at app launch and after each stage) deletes a folder only when it
//! has `.owners`, no owner's session file names it, this core's session is not in it, and
//! nothing in it changed for a day. A folder without `.owners` (staged by an older QuadCam
//! or by a core with no session file) is never swept: nothing says who owns it.
//!
//! A session keeps its staged files after the export: a second import, a cut change,
//! previews and the delete from the card all read them. So nothing deletes staging when every
//! clip verified; only a replace or the sweep does.

use super::Core;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The owners list inside a staging folder.
const OWNERS: &str = ".owners";

/// How long a folder must stay untouched before the sweep may delete it. A stage in another
/// process writes into its folder all the time, so it is never this old.
pub const SWEEP_AGE: Duration = Duration::from_secs(24 * 60 * 60);

impl Core {
    fn staging_root(&self) -> PathBuf {
        self.cache.join("staging")
    }

    /// Adds this core's session file to the owners of `folder`.
    pub(super) fn own_staging(&self, folder: &Path) {
        let Some(file) = &self.session_file else {
            return;
        };
        let list = folder.join(OWNERS);
        let mut owners = read_owners(folder);
        if owners.iter().any(|o| o == file) {
            return;
        }
        owners.push(file.clone());
        let text: String = owners
            .iter()
            .map(|o| format!("{}\n", o.display()))
            .collect();
        if let Err(e) = std::fs::write(&list, text) {
            eprintln!("quadcam: {} not written: {e}", list.display());
        }
    }

    /// Deletes `old`, the staging folder of the session a load just replaced, unless the new
    /// session uses it too or another owner's session file still names it.
    pub(super) fn release_staging(&self, old: &Path, new: &Path) {
        if same(old, new) || !self.in_staging_root(old) || !old.is_dir() {
            return;
        }
        if read_owners(old).iter().any(|o| names(o, old)) {
            return;
        }
        if let Err(e) = std::fs::remove_dir_all(old) {
            eprintln!("quadcam: {} not deleted: {e}", old.display());
        }
    }

    /// Deletes the staging folders no session owns (see the module docs). Returns the
    /// folders it deleted.
    pub fn sweep_staging(&self) -> Vec<PathBuf> {
        self.sweep_staging_older_than(SWEEP_AGE)
    }

    /// `sweep_staging` with another age limit (tests pass zero).
    pub fn sweep_staging_older_than(&self, age: Duration) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(self.staging_root()) else {
            return Vec::new();
        };
        let live = self.session().map(|s| s.staging);
        let cutoff = SystemTime::now().checked_sub(age);
        let mut deleted = Vec::new();
        for dir in entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
            if live.as_deref().is_some_and(|l| same(l, &dir)) {
                continue;
            }
            if self.session_file.as_deref().is_some_and(|f| names(f, &dir)) {
                continue;
            }
            let owners = read_owners(&dir);
            if !dir.join(OWNERS).is_file() || owners.iter().any(|o| names(o, &dir)) {
                continue;
            }
            if cutoff.is_none_or(|c| newest_change(&dir).is_none_or(|t| t > c)) {
                continue;
            }
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => deleted.push(dir),
                Err(e) => eprintln!("quadcam: {} not deleted: {e}", dir.display()),
            }
        }
        deleted
    }

    /// True when `dir` is a folder directly inside this core's staging root.
    fn in_staging_root(&self, dir: &Path) -> bool {
        let root = self.staging_root();
        dir.parent()
            .is_some_and(|p| same(p, &root) && dir.file_name().is_some())
    }
}

fn read_owners(folder: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(folder.join(OWNERS))
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Whether the session file `file` names `folder` as its staging. A missing file names
/// nothing; a file that cannot be read or parsed counts as naming it, so it is kept.
fn names(file: &Path, folder: &Path) -> bool {
    let bytes = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    };
    // The raw field, not `Session::load`: a session file from another version still owns
    // its folder.
    match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(v) => v
            .get("staging")
            .and_then(|s| s.as_str())
            .is_none_or(|s| same(Path::new(s), folder)),
        Err(_) => true,
    }
}

/// The same path, as written or once resolved (the GUI's and the CLI's cache paths may be
/// spelled differently).
fn same(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (a.canonicalize(), b.canonicalize()),
            (Ok(x), Ok(y)) if x == y
        )
}

/// The latest change time of `dir` and the files directly in it.
fn newest_change(dir: &Path) -> Option<SystemTime> {
    let own = dir.metadata().and_then(|m| m.modified()).ok();
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| e.metadata().and_then(|m| m.modified()).ok())
        .chain(own)
        .max()
}
