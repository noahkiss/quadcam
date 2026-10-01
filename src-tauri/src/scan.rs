//! Card scanning: find DVR clips anywhere on a volume, and inspect AVI structure.

use serde::Serialize;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Video extensions analog DVRs write (MJPEG in AVI).
const VIDEO_EXTS: &[&str] = &["avi"];

/// Deepest folder level searched. Cards hold `DCIM/<n>/` at most; this only stops runaway walks.
const MAX_DEPTH: usize = 6;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FoundClip {
    pub path: PathBuf,
    /// Path relative to the volume root, for display.
    pub rel: String,
    pub name: String,
    pub size: u64,
}

/// Recursively lists video files under `root`, skipping Mac hidden files (`._*`,
/// `.fseventsd`, `.Spotlight-V100`, `.Trashes`, any dot entry). Sorted in PICT-number order.
pub fn find_clips(root: &Path) -> Vec<FoundClip> {
    let mut out = Vec::new();
    walk(root, root, 0, &mut out);
    out.sort_by_key(|c| clip_order_key(&c.rel));
    out
}

/// True when the volume holds at least one clip. Stops at the first hit.
pub fn has_clips(root: &Path) -> bool {
    fn any(dir: &Path, depth: usize) -> bool {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return false;
        };
        let mut subdirs = Vec::new();
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                subdirs.push(e.path());
            } else if ft.is_file() && is_video(&name) {
                return true;
            }
        }
        depth < MAX_DEPTH && subdirs.iter().any(|d| any(d, depth + 1))
    }
    any(root, 0)
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<FoundClip>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        let path = e.path();
        if ft.is_dir() {
            if depth < MAX_DEPTH {
                walk(root, &path, depth + 1, out);
            }
        } else if ft.is_file() && is_video(&name) {
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            out.push(FoundClip {
                path,
                rel,
                name,
                size,
            });
        }
    }
}

pub fn is_video(name: &str) -> bool {
    let lower = name.to_lowercase();
    VIDEO_EXTS
        .iter()
        .any(|ext| lower.ends_with(&format!(".{ext}")))
}

/// Sort key: folder, then the number in the file name (`PICT0012` after `PICT0002`), then name.
fn clip_order_key(rel: &str) -> (String, u64, String) {
    let p = Path::new(rel);
    let parent = p
        .parent()
        .map(|x| x.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let stem = p
        .file_stem()
        .map(|x| x.to_string_lossy().to_string())
        .unwrap_or_default();
    let digits: String = stem.chars().filter(|c| c.is_ascii_digit()).collect();
    let n = digits.parse().unwrap_or(u64::MAX);
    (parent, n, stem.to_lowercase())
}

/// What the RIFF structure says about a file (half-written detection).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AviCheck {
    /// RIFF size field + 8 equals the file size.
    pub riff_size_ok: bool,
    /// A top-level `idx1` chunk exists.
    pub has_index: bool,
    /// The file starts with `RIFF....AVI `.
    pub is_avi: bool,
}

impl AviCheck {
    pub fn complete(&self) -> bool {
        self.is_avi && self.riff_size_ok && self.has_index
    }
}

pub fn check_avi(path: &Path) -> std::io::Result<AviCheck> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let mut hdr = [0u8; 12];
    if len < 12 || f.read_exact(&mut hdr).is_err() {
        return Ok(AviCheck {
            riff_size_ok: false,
            has_index: false,
            is_avi: false,
        });
    }
    let is_avi = &hdr[0..4] == b"RIFF" && &hdr[8..12] == b"AVI ";
    let riff_size = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as u64;
    let riff_size_ok = is_avi && riff_size + 8 == len;

    // Walk top-level chunks looking for idx1. Chunks are word-aligned.
    let mut has_index = false;
    let mut pos = 12u64;
    while is_avi && pos + 8 <= len {
        f.seek(SeekFrom::Start(pos))?;
        let mut ch = [0u8; 8];
        if f.read_exact(&mut ch).is_err() {
            break;
        }
        let size = u32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]) as u64;
        if &ch[0..4] == b"idx1" {
            has_index = pos + 8 + size <= len;
            break;
        }
        pos += 8 + size + (size & 1);
    }
    Ok(AviCheck {
        riff_size_ok,
        has_index,
        is_avi,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_recursively_and_skips_hidden() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir_all(r.join("DCIM/100")).unwrap();
        std::fs::create_dir_all(r.join(".Spotlight-V100")).unwrap();
        std::fs::create_dir_all(r.join(".fseventsd")).unwrap();
        for f in [
            "PICT0012.AVI",
            "DCIM/100/PICT0002.AVI",
            "DCIM/100/PICT0010.avi",
            "DCIM/100/._PICT0002.AVI",
            ".Spotlight-V100/x.avi",
            "notes.txt",
        ] {
            std::fs::write(r.join(f), b"x").unwrap();
        }
        let got: Vec<String> = find_clips(r).into_iter().map(|c| c.rel).collect();
        assert_eq!(
            got,
            vec![
                "PICT0012.AVI",
                "DCIM/100/PICT0002.AVI",
                "DCIM/100/PICT0010.avi"
            ]
        );
        assert!(has_clips(r));
        let empty = tempfile::tempdir().unwrap();
        std::fs::write(empty.path().join("readme.txt"), b"x").unwrap();
        assert!(!has_clips(empty.path()));
    }
}
