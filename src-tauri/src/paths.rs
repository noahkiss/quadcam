//! Where QuadCam keeps its files, all under `$HOME`: the cache (staging, thumbnails,
//! previews, the session file, firmware and voice downloads), the support folder (settings,
//! control socket, the gear folder), and the default library folder. Every surface derives
//! its paths here, so a test that sets `HOME` to a temp folder never touches the real ones.
//!
//! A process started by cargo (tests, `cargo run`, `cargo tauri dev`, and what they spawn;
//! they carry `CARGO_MANIFEST_DIR`) may not use the real support folder: while `HOME` is the
//! account's own home, `support_dir` and the paths under it panic unless `QUADCAM_PATHS=real`.
//! Such a process sets `HOME` to a temp folder or passes `Core::with_settings` a temp file.

use std::path::{Path, PathBuf};

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// The home folder (`$HOME`); sim adapters look for their files under it.
pub fn home_dir() -> PathBuf {
    home()
}

/// `~/Library/Caches/app.quadcam`, the same folder the GUI uses.
pub fn cache_dir() -> PathBuf {
    home().join("Library/Caches/app.quadcam")
}

/// The account's own home folder from the user database, whatever `HOME` says.
pub fn account_home() -> Option<PathBuf> {
    use std::ffi::{CStr, OsStr};
    use std::os::unix::ffi::OsStrExt;
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    // SAFETY: every pointer is valid for the call; `pw`'s strings point into `buf`.
    let rc = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            &mut pw,
            buf.as_mut_ptr(),
            buf.len(),
            &mut found,
        )
    };
    if rc != 0 || found.is_null() || pw.pw_dir.is_null() {
        return None;
    }
    // SAFETY: `pw_dir` is a NUL-terminated string in `buf`, which is still alive.
    let dir = unsafe { CStr::from_ptr(pw.pw_dir) };
    Some(PathBuf::from(OsStr::from_bytes(dir.to_bytes())))
}

/// Whether this process may use the support folder under `home`: always outside cargo, and
/// under cargo only with `QUADCAM_PATHS=real` or a `home` that is not the account's own (an
/// unknown account home counts as the account's own).
pub fn support_allowed(
    paths_env: Option<&str>,
    under_cargo: bool,
    home: &Path,
    account_home: Option<&Path>,
) -> bool {
    !under_cargo || paths_env == Some("real") || account_home.is_some_and(|a| a != home)
}

/// `~/Library/Application Support/app.quadcam`, or the reason this process may not use it
/// (see the module docs).
pub fn try_support_dir() -> Result<PathBuf, String> {
    let home = home();
    let dir = home.join("Library/Application Support/app.quadcam");
    if support_allowed(
        std::env::var("QUADCAM_PATHS").ok().as_deref(),
        std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
        &home,
        account_home().as_deref(),
    ) {
        Ok(dir)
    } else {
        Err(format!(
            "a process started by cargo may not use the real support folder ({}): set HOME to a \
             temp folder, pass Core::with_settings a temp file, or set QUADCAM_PATHS=real",
            dir.display()
        ))
    }
}

/// `~/Library/Application Support/app.quadcam`: settings, the control socket, the gear
/// folder. Panics in a process started by cargo while `HOME` is the account's own, unless
/// `QUADCAM_PATHS=real` (see the module docs).
pub fn support_dir() -> PathBuf {
    try_support_dir().unwrap_or_else(|e| panic!("{e}"))
}

/// The support folder, or for a process that may not use it a temporary stand-in, for code
/// that only finds things there (the installed modules, the tool settings).
pub fn support_dir_or_stand_in() -> PathBuf {
    try_support_dir().unwrap_or_else(|_| {
        std::env::temp_dir().join(format!("quadcam-test-support-{}", std::process::id()))
    })
}

/// The settings file the app, the CLI and the MCP server share.
pub fn default_settings_file() -> PathBuf {
    support_dir().join("settings.json")
}

/// The gear folder until the `gearDir` setting points elsewhere: `<support>/gear`.
pub fn default_gear_dir() -> PathBuf {
    support_dir().join("gear")
}

/// Firmware downloads, one folder per product and version: `<cache>/firmware`.
pub fn firmware_cache_dir() -> PathBuf {
    cache_dir().join("firmware")
}

/// Raw voice takes, so a re-render costs nothing: `<cache>/voice`.
pub fn voice_cache_dir() -> PathBuf {
    cache_dir().join("voice")
}

/// The session file shared by CLI runs and a headless MCP server.
pub fn default_session_file() -> PathBuf {
    cache_dir().join("session.json")
}

/// Where the output folder setting points until the user picks one, relative to `$HOME`.
pub const DEFAULT_OUTPUT_REL: &str = "Movies/quadcam";

/// `~/Movies/quadcam`, resolved from `$HOME` at runtime.
pub fn default_output_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join(DEFAULT_OUTPUT_REL))
}

/// The default folder for telemetry overlay files (`core/overlay.rs`), outside the default
/// library folder.
pub fn default_overlay_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join("Movies/quadcam-overlays"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_may_not_use_the_account_home() {
        let me = Path::new("/Users/someone");
        assert!(!support_allowed(None, true, me, Some(me)));
        assert!(!support_allowed(Some("1"), true, me, Some(me)));
        assert!(!support_allowed(None, true, me, None));
        assert!(support_allowed(Some("real"), true, me, Some(me)));
        assert!(support_allowed(
            None,
            true,
            Path::new("/tmp/home"),
            Some(me)
        ));
        assert!(support_allowed(None, false, me, Some(me)));
    }

    #[test]
    fn the_account_home_is_known() {
        assert!(account_home().is_some_and(|h| h.is_absolute()));
    }
}
