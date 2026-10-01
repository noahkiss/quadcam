//! The one cut writer for the import session and the library, and the one way to list a
//! clip's exported cuts for the removed-cut rule in `trim`.

use crate::library::LibClip;
use crate::media::{self, Encoder, Format, Meta, Probe, Tools};
use crate::moments::Span;
use crate::pipeline::Outcome;
use crate::session::Session;
use crate::trim::ExportedCut;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// The container for a file extension: MOV for `.mov`, else MP4. A cut keeps its clip's.
pub fn format_for_ext(ext: &str) -> Format {
    if ext.eq_ignore_ascii_case("mov") {
        Format::Mov
    } else {
        Format::Mp4
    }
}

/// The hidden name a file is written under until it verifies: `.<name>.part`.
pub fn part_path(dst: &Path) -> PathBuf {
    dst.with_file_name(format!(
        ".{}.part",
        dst.file_name().unwrap_or_default().to_string_lossy()
    ))
}

/// Writes the QuickTime items into `file`, the location also as `©xyz`.
pub fn write_qt(file: &Path, qt: &[(String, String)]) -> Result<()> {
    let loc = qt
        .iter()
        .find(|(k, _)| k == "com.apple.quicktime.location.ISO6709")
        .map(|(_, v)| v.as_str());
    crate::qtmeta::write(file, qt, loc).context("writing QuickTime metadata")
}

/// One cut range to write as its own file.
pub struct Cut<'a> {
    /// The file to cut from, and its probe (the frame check compares against it).
    pub src: &'a Path,
    pub probe: &'a Probe,
    pub span: Span,
    pub format: Format,
    pub encoder: Encoder,
    /// The cut's own metadata: its start time and description.
    pub meta: &'a Meta,
    /// The QuickTime items to write; they are read back after.
    pub qt: &'a [(String, String)],
}

/// Writes `cut` to `dst` under a hidden `.part` name: the cut, its QuickTime items, then the
/// frame check and the metadata read-back. Renames it only after both pass, and never over
/// an existing file. Sets the file time to the cut's start. Returns the file's size.
pub fn write_cut(tools: &Tools, cut: &Cut, dst: &Path) -> Result<u64> {
    let tmp = part_path(dst);
    let res = media::cut(
        tools,
        cut.src,
        &tmp,
        cut.span,
        cut.format,
        cut.encoder,
        cut.meta,
    )
    .and_then(|_| write_qt(&tmp, cut.qt))
    .and_then(|_| media::verify_cut(tools, cut.probe, &tmp, cut.span))
    .and_then(|_| media::verify_qt(tools, &tmp, cut.qt))
    .and_then(|_| {
        if dst.exists() {
            bail!("{} appeared during export; not overwriting", dst.display());
        }
        std::fs::rename(&tmp, dst).context("renaming cut")
    });
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    let _ = media::set_mtime(dst, cut.meta.creation_time);
    Ok(dst.metadata().map(|m| m.len()).unwrap_or(0))
}

/// A session clip's cuts that verified and still exist as files.
pub fn session_exported(s: &Session, id: usize) -> Vec<ExportedCut> {
    s.results
        .iter()
        .rev()
        .find(|r| r.id == id && r.outcome == Outcome::Verified)
        .map(|r| {
            r.cuts
                .iter()
                .filter(|c| c.outcome == Outcome::Verified)
                .filter_map(|c| {
                    let p = c.output.clone()?;
                    p.is_file().then_some(ExportedCut {
                        span: Span {
                            start: c.start,
                            end: c.end,
                        },
                        path: p,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A library clip's cut files, with absolute paths.
pub fn library_exported(root: &Path, c: &LibClip) -> Vec<ExportedCut> {
    c.cuts
        .iter()
        .map(|x| ExportedCut {
            span: Span {
                start: x.start,
                end: x.end,
            },
            path: root.join(&x.path),
        })
        .collect()
}
