//! `Core::gear_osd`: a Betaflight OSD layout drawn per profile and checked (design 7.3).
//! It reads dump, diff or CLI files, or a device's latest FC backup (its `dump all`), with
//! files read after the backup on top.

use super::Core;
use crate::gear::changes::ChangeFilter;
use crate::gear::model::{ChangeStatus, Edit, StagedChange};
use crate::gear::osd::{self, OsdConfig, OsdCopy, OsdMove, OsdView};
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
    /// A saved device's id: reads its latest backup's `dump all` (`diff all` without one),
    /// before `paths`.
    #[serde(default)]
    pub device: Option<String>,
    /// `NTSC`, `PAL`, `HD` or `WxH`; empty for the files' `vcd_video_system`.
    #[serde(default)]
    pub grid: Option<String>,
    /// With a `device`: also apply the OSD edits its staged changes hold, so the view shows
    /// the layout after they apply (the editor's working copy).
    #[serde(default)]
    pub staged: bool,
}

/// `gear_osd_edit`: move or toggle elements, or copy one profile's layout onto another, as
/// a staged change for an FC. The edits join the device's open "OSD layout" change.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct OsdEditParams {
    pub device: String,
    #[serde(default)]
    pub moves: Vec<OsdMove>,
    #[serde(default)]
    pub copy: Option<OsdCopy>,
    /// Who stages it. The app and the CLI leave it out (the person); MCP says `agent`.
    #[serde(default)]
    pub editor: Option<crate::session::Editor>,
}

/// The title of the change the editor keeps its edits in.
pub const OSD_CHANGE: &str = "OSD layout";

impl Core {
    /// Reads the OSD layout from the files (or the device's backup), draws every OSD
    /// profile on the grid, and checks each for overlaps and cells off screen. Reads only.
    pub fn gear_osd(&self, p: &OsdParams) -> Result<OsdView> {
        let mut texts = Vec::new();
        let mut source = Vec::new();
        if let Some(id) = p.device.as_deref().filter(|d| !d.trim().is_empty()) {
            let (text, name) = self.osd_backup(id)?;
            texts.push(text);
            source.push(name);
        }
        if texts.is_empty() && p.paths.is_empty() {
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
        let mut config = OsdConfig::parse(texts.iter().map(String::as_str));
        let mut staged = 0;
        if p.staged {
            if let Some(id) = p.device.as_deref().filter(|d| !d.trim().is_empty()) {
                let edits = self.staged_osd_edits(id);
                staged = edits.len();
                config.apply_edits(&edits);
            }
        }
        let (grid, note) = osd::grid_for(&config, p.grid.as_deref())?;
        let mut view = osd::view(&config, grid, source);
        if let Some(n) = note {
            view.notes.insert(0, n);
        }
        if staged > 0 {
            view.notes.push(format!(
                "Shows {staged} staged OSD {}: not on the FC until applied.",
                if staged == 1 { "edit" } else { "edits" }
            ));
        }
        Ok(view)
    }

    /// The device's latest backup's `dump all` (`diff all` without one) and its name.
    fn osd_backup(&self, id: &str) -> Result<(String, String)> {
        let known = self.gear_store().devices()?.into_iter().any(|d| d.id == id);
        if !known {
            bail!("No device {id:?} in QuadCam's list (quadcam-cli gear devices).");
        }
        let snaps = crate::gear::backup::Snapshots::new(self.gear_store());
        let b = snaps
            .list(id)
            .into_iter()
            .rev()
            .find(|b| {
                b.files
                    .iter()
                    .any(|f| f.path == "dump all" || f.path == "diff all")
            })
            .with_context(|| {
                format!("No FC backup of {id:?} yet: back it up, or pass a dump file.")
            })?;
        // The dump holds every value; a diff alone shows only what differs from default.
        let file = ["dump all", "diff all"]
            .iter()
            .find_map(|c| b.files.iter().find(|f| f.path == *c))
            .context("the backup has no dump")?;
        let bytes = snaps.blobs().get(&crate::gear::backup::blob_of(file))?;
        Ok((
            String::from_utf8_lossy(&bytes).into_owned(),
            format!("{} {}", b.id, file.path),
        ))
    }

    /// The OSD edits of the device's staged changes, in order.
    fn staged_osd_edits(&self, device: &str) -> Vec<Edit> {
        self.changes()
            .list(&ChangeFilter {
                device: Some(device.to_string()),
                ..Default::default()
            })
            .into_iter()
            .flat_map(|c| c.edits)
            .filter(|e| matches!(e, Edit::OsdElement { .. }))
            .collect()
    }

    /// Stages OSD moves, toggles or a profile copy. The edits join the device's open
    /// "OSD layout" change (made if none): a later edit of an element replaces its earlier
    /// one, and an element put back where the FC holds it drops out. A change left with no
    /// edits is discarded. Writes only QuadCam's own data.
    pub fn gear_osd_edit(&self, p: &OsdEditParams) -> Result<StagedChange> {
        if p.moves.is_empty() && p.copy.is_none() {
            bail!("Pass at least one move, or a profile copy.");
        }
        let (text, _) = self.osd_backup(&p.device)?;
        let base = OsdConfig::parse([text.as_str()]);
        let open = self
            .changes()
            .list(&ChangeFilter {
                device: Some(p.device.clone()),
                ..Default::default()
            })
            .into_iter()
            .find(|c| {
                c.title == OSD_CHANGE
                    && matches!(c.status, ChangeStatus::Draft | ChangeStatus::Ready)
                    && c.edits.iter().all(|e| matches!(e, Edit::OsdElement { .. }))
            });
        // What the layout reads as with the open change on top.
        let mut now = base.clone();
        if let Some(c) = &open {
            now.apply_edits(&c.edits);
        }
        let mut fresh = Vec::new();
        for m in &p.moves {
            let e = now.resolve(m)?;
            now.apply_edits(std::slice::from_ref(&e));
            fresh.push(e);
        }
        if let Some(c) = p.copy {
            let es = now.copy_profile(c)?;
            now.apply_edits(&es);
            fresh.extend(es);
        }
        // One edit per element: the last word, and only where it differs from the FC.
        let mut keep: Vec<Edit> = Vec::new();
        let all = open.iter().flat_map(|c| c.edits.iter()).chain(fresh.iter());
        for e in all {
            let Edit::OsdElement { element, .. } = e else {
                continue;
            };
            keep.retain(|k| !matches!(k, Edit::OsdElement { element: n, .. } if n == element));
            keep.push(e.clone());
        }
        keep.retain(|e| {
            let Edit::OsdElement { element, .. } = e else {
                return true;
            };
            let mut probe = base.clone();
            probe.apply_edits(std::slice::from_ref(e));
            base.positions.get(element) != probe.positions.get(element)
        });
        match (open, keep.is_empty()) {
            (Some(c), true) => self.gear_change_discard(&c.id),
            (None, true) => bail!("Nothing changes: the FC already has that layout."),
            (Some(c), false) => self.gear_change_update(&super::ChangeUpdateParams {
                id: c.id,
                edits: Some(keep),
                ..Default::default()
            }),
            (None, false) => self.gear_change_stage(&super::StageParams {
                device: p.device.clone(),
                title: Some(OSD_CHANGE.into()),
                edits: keep,
                editor: p.editor,
                ..Default::default()
            }),
        }
    }
}
