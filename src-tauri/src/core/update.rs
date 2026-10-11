//! The app's own updates: what the GUI's updater (`src/updater.rs`) found and did, for the
//! banner and `status.update`, and the rule that holds the restart back while a job runs.
//! The core never reaches the network here; the GUI checks, downloads and installs.

use super::Core;
use chrono::{DateTime, Utc};
use serde::Serialize;

/// Where an update stands.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePhase {
    /// Not checked since launch.
    #[default]
    Idle,
    Checking,
    /// The running version is the newest.
    Current,
    /// A newer version is published.
    Available,
    Downloading,
    /// Downloaded and verified; installs at the restart.
    Ready,
    /// The last check or download failed (`error`).
    Failed,
}

/// The update state the GUI shows and `status.update` reports.
#[derive(Debug, Clone, Default, Serialize, PartialEq, specta::Type)]
pub struct UpdateStatus {
    pub phase: UpdatePhase,
    /// The running version.
    pub current: String,
    /// The newer version, from `available` on.
    pub version: Option<String>,
    /// Its release page.
    pub notes_url: Option<String>,
    /// Bytes downloaded, and the size when the server sent one.
    pub done: u64,
    pub total: Option<u64>,
    pub error: Option<String>,
    /// When the last check ended.
    pub checked_at: Option<DateTime<Utc>>,
    /// The job that holds the restart back now, as words ("the device job"). None: free to restart.
    pub waiting_for: Option<String>,
}

/// The release page of a version.
pub fn notes_url(version: &str) -> String {
    format!("https://github.com/noahkiss/quadcam/releases/tag/v{version}")
}

/// What holds a restart back: a session or library step (`busy`: load, import, format, Photos,
/// Trash), a job that holds a device link (card and FC writes, flashes, reads), or a Gear
/// backup or card check. None when nothing runs.
pub fn restart_blocker(busy: bool, held_links: usize, gear_jobs: usize) -> Option<String> {
    if busy {
        Some("the import or card step".into())
    } else if held_links > 0 {
        Some("the device job".into())
    } else if gear_jobs > 0 {
        Some("the backup or card check".into())
    } else {
        None
    }
}

impl Core {
    /// The job that holds a restart back now (`restart_blocker`).
    pub fn restart_blocker(&self) -> Option<String> {
        restart_blocker(
            self.busy.load(std::sync::atomic::Ordering::SeqCst),
            self.gear_working().len(),
            self.gear_jobs().len(),
        )
    }

    /// The update state, with the job that holds the restart back now. None without a GUI:
    /// only the app updates itself.
    pub fn update_status(&self) -> Option<UpdateStatus> {
        if !self.has_gui() {
            return None;
        }
        let mut s = self.update.lock().unwrap().clone();
        s.waiting_for = self.restart_blocker();
        Some(s)
    }

    /// Changes the update state and tells the GUI.
    pub fn set_update(&self, f: impl FnOnce(&mut UpdateStatus)) -> UpdateStatus {
        {
            let mut s = self.update.lock().unwrap();
            f(&mut s);
        }
        let now = self.update_status().unwrap_or_default();
        self.hooks
            .event(crate::api::Event::Update(crate::api::UpdateChanged(
                now.clone(),
            )));
        now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_running_frees_the_restart() {
        assert_eq!(restart_blocker(false, 0, 0), None);
    }

    #[test]
    fn every_job_holds_the_restart() {
        assert!(restart_blocker(true, 0, 0).unwrap().contains("import"));
        assert!(restart_blocker(false, 1, 0).unwrap().contains("device"));
        assert!(restart_blocker(false, 0, 2).unwrap().contains("backup"));
        // The session step is named first when several run.
        assert!(restart_blocker(true, 3, 1).unwrap().contains("import"));
    }

    /// A job's hold on a device holds the restart back until the job ends; a headless core
    /// reports no update.
    #[test]
    fn a_held_device_holds_the_restart() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(
            dir.path().to_path_buf(),
            None,
            std::sync::Arc::new(super::super::NoHooks),
            std::sync::Arc::new(crate::photos::Recorder::default()),
        );
        assert_eq!(core.restart_blocker(), None);
        {
            let _hold = core.gear_hold("serial:/dev/cu.test");
            assert!(core.restart_blocker().unwrap().contains("device"));
        }
        assert_eq!(core.restart_blocker(), None);
        {
            let _busy = core.claim().unwrap();
            assert!(core.restart_blocker().unwrap().contains("import"));
        }
        assert_eq!(core.restart_blocker(), None);
        assert!(core.update_status().is_none());
        assert!(core.status().update.is_none());
    }

    #[test]
    fn notes_link_names_the_tag() {
        assert_eq!(
            notes_url("0.13.0"),
            "https://github.com/noahkiss/quadcam/releases/tag/v0.13.0"
        );
    }
}
