//! Where QuadCam keeps its files, all under `$HOME`: the cache (staging, thumbnails,
//! previews, the session file, firmware and voice downloads), the support folder (settings,
//! control socket, the gear folder), and the default library folder. Every surface derives
//! its paths here, so a test that sets `HOME` to a temp folder never touches the real ones.

use std::path::PathBuf;

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

/// `~/Library/Application Support/app.quadcam`, where the control socket lives.
pub fn support_dir() -> PathBuf {
    home().join("Library/Application Support/app.quadcam")
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
