//! `Core::gear_osd`: a Betaflight OSD layout drawn per profile and checked (design 7.3).
//! It reads dump, diff or CLI files today. A device's latest backup (WP4) and a live read
//! of a connected FC (WP2) join here as more sources of the same text.

use super::Core;
use crate::gear::osd::{self, OsdConfig, OsdView};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Files larger than this are not a Betaflight dump.
const MAX_FILE: u64 = 4 * 1024 * 1024;

/// `gear_osd`: where to read the OSD from, and the grid to draw it on.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct OsdParams {
    /// `dump all`, `diff all` or CLI-line files, read in order: a later file's lines win,
    /// so a dump followed by an apply file shows the layout after the apply.
    #[serde(default)]
    pub paths: Vec<PathBuf>,
    /// A saved device's id: reads its latest backup. Not available until backups exist.
    #[serde(default)]
    pub device: Option<String>,
    /// `NTSC`, `PAL`, `HD` or `WxH`; empty for the files' `vcd_video_system`.
    #[serde(default)]
    pub grid: Option<String>,
}

impl Core {
    /// Reads the OSD layout from the files (or the device's backup), draws every OSD
    /// profile on the grid, and checks each for overlaps and cells off screen. Reads only.
    pub fn gear_osd(&self, p: &OsdParams) -> Result<OsdView> {
        let mut texts = Vec::new();
        let mut source = Vec::new();
        if let Some(id) = p.device.as_deref().filter(|d| !d.trim().is_empty()) {
            let known = self.gear_store().devices()?.into_iter().any(|d| d.id == id);
            if !known {
                bail!("No device {id:?} in QuadCam's list (quadcam-cli gear devices).");
            }
            bail!(
                "No device backup to read for {id:?} yet: pass a dump file. Device backups come in a later version."
            );
        }
        if p.paths.is_empty() {
            bail!("Pass a Betaflight dump or diff file, or a device.");
        }
        for path in &p.paths {
            let meta = std::fs::metadata(path)
                .with_context(|| format!("cannot read {}", path.display()))?;
            if !meta.is_file() || meta.len() > MAX_FILE {
                bail!(
                    "{} is not a Betaflight dump, diff or CLI file.",
                    path.display()
                );
            }
            let bytes =
                std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
            texts.push(String::from_utf8_lossy(&bytes).into_owned());
            source.push(
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string()),
            );
        }
        let config = OsdConfig::parse(texts.iter().map(String::as_str));
        let (grid, note) = osd::grid_for(&config, p.grid.as_deref())?;
        let mut view = osd::view(&config, grid, source);
        if let Some(n) = note {
            view.notes.insert(0, n);
        }
        Ok(view)
    }
}
