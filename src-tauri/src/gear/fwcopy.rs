//! Firmware copies: the flash of a radio read over DFU, kept as plain files under
//! `<gear>/firmware/<device>/`. They are not card snapshots. A snapshot holding only
//! `firmware.bin` would become the radio's latest backup, which the model editors and the
//! apply engine read cards from, and the backup pruner would collect a blob nothing in a
//! snapshot names.
//!
//! One copy is two files: `<id>.bin` (the image, without the erased tail) and `<id>.json`
//! (what QuadCam knows of it). A write goes to a temporary name, is fsynced and renamed,
//! then is read back from disk and compared before the copy counts as saved.

use super::store::Store;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;
use std::io::Write;
use std::path::PathBuf;

/// Why a copy was taken.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CopyKind {
    /// The person ran "Read radio firmware".
    Read,
    /// The flash took it before it erased.
    BeforeFlash,
}

impl CopyKind {
    fn tag(self) -> &'static str {
        match self {
            CopyKind::Read => "read",
            CopyKind::BeforeFlash => "before-flash",
        }
    }
}

/// One saved copy.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct FwCopy {
    /// `<device>/<stamp>-<kind>`.
    pub id: String,
    pub device: String,
    pub taken_at: DateTime<Utc>,
    pub kind: CopyKind,
    /// Bytes in the saved image.
    pub size: u64,
    /// SHA-256 of the saved image.
    pub sha256: String,
    /// The board the image names itself (`pocket`), when it carries the EdgeTX string.
    pub image_board: Option<String>,
    /// The EdgeTX version the image names itself.
    pub image_version: Option<String>,
}

fn safe(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn dir(store: &Store, device: &str) -> PathBuf {
    store.root().join("firmware").join(safe(device))
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn write_synced(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    let mut f =
        std::fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path).with_context(|| format!("saving {}", path.display()))?;
    Ok(())
}

/// Saves `bytes` as a copy of `device`'s firmware, reads it back from disk and compares.
pub fn save(
    store: &Store,
    device: &str,
    kind: CopyKind,
    taken_at: DateTime<Utc>,
    bytes: &[u8],
    image: Option<(String, String)>,
) -> Result<FwCopy> {
    if bytes.is_empty() {
        bail!("There is no firmware to save.");
    }
    let d = dir(store, device);
    std::fs::create_dir_all(&d).with_context(|| format!("creating {}", d.display()))?;
    // Two copies in the same second get -2, -3: a copy never replaces another.
    let base = format!("{}-{}", taken_at.format("%Y-%m-%dT%H%M%S"), kind.tag());
    let mut stem = base.clone();
    let mut n = 2;
    while d.join(format!("{stem}.bin")).exists() || d.join(format!("{stem}.json")).exists() {
        stem = format!("{base}-{n}");
        n += 1;
    }
    let (image_board, image_version) = match image {
        Some((b, v)) => (Some(b), Some(v)),
        None => (None, None),
    };
    let copy = FwCopy {
        id: format!("{}/{stem}", safe(device)),
        device: device.to_string(),
        taken_at,
        kind,
        size: bytes.len() as u64,
        sha256: sha256(bytes),
        image_board,
        image_version,
    };
    let bin = d.join(format!("{stem}.bin"));
    write_synced(&bin, bytes)?;
    let back = std::fs::read(&bin).with_context(|| format!("reading back {}", bin.display()))?;
    if back != bytes {
        let _ = std::fs::remove_file(&bin);
        bail!("The saved copy does not match what was read; the disk may be failing.");
    }
    write_synced(
        &d.join(format!("{stem}.json")),
        &serde_json::to_vec_pretty(&copy)?,
    )?;
    Ok(copy)
}

/// A device's copies, newest first.
pub fn list(store: &Store, device: &str) -> Vec<FwCopy> {
    let mut out: Vec<FwCopy> = std::fs::read_dir(dir(store, device))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
        .collect();
    out.sort_by(|a, b| b.taken_at.cmp(&a.taken_at).then(b.id.cmp(&a.id)));
    out
}

/// The image of a copy, checked against its recorded hash.
pub fn read(store: &Store, copy: &FwCopy) -> Result<Vec<u8>> {
    let stem = copy.id.rsplit('/').next().unwrap_or(&copy.id);
    let bytes = std::fs::read(dir(store, &copy.device).join(format!("{}.bin", safe(stem))))
        .with_context(|| format!("No firmware copy {}.", copy.id))?;
    if sha256(&bytes) != copy.sha256 {
        bail!(
            "The firmware copy {} does not match its recorded hash.",
            copy.id
        );
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::new(d.path().to_path_buf());
        (d, s)
    }

    #[test]
    fn a_copy_is_saved_read_back_listed_and_read() {
        let (_d, s) = store();
        let at = "2026-10-09T12:00:00Z".parse().unwrap();
        let bytes: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        let c = save(
            &s,
            "radio-1",
            CopyKind::Read,
            at,
            &bytes,
            Some(("pocket".into(), "2.12.4".into())),
        )
        .unwrap();
        assert_eq!(c.id, "radio-1/2026-10-09T120000-read");
        assert_eq!(c.size, 5000);
        assert_eq!(list(&s, "radio-1"), std::slice::from_ref(&c));
        assert_eq!(read(&s, &c).unwrap(), bytes);
        // A damaged file fails its hash.
        let p = dir(&s, "radio-1").join("2026-10-09T120000-read.bin");
        std::fs::write(&p, b"damaged").unwrap();
        assert!(read(&s, &c).is_err());
        assert!(save(&s, "radio-1", CopyKind::Read, at, &[], None).is_err());
        assert!(list(&s, "radio-2").is_empty());
    }

    #[test]
    fn copies_in_the_same_second_get_their_own_names() {
        let (_d, s) = store();
        let at = "2026-10-09T12:00:00Z".parse().unwrap();
        let ids: Vec<String> = [b"one".as_slice(), b"two", b"three"]
            .iter()
            .map(|b| {
                save(&s, "radio-1", CopyKind::BeforeFlash, at, b, None)
                    .unwrap()
                    .id
            })
            .collect();
        assert_eq!(
            ids,
            [
                "radio-1/2026-10-09T120000-before-flash",
                "radio-1/2026-10-09T120000-before-flash-2",
                "radio-1/2026-10-09T120000-before-flash-3",
            ]
        );
        // Each copy keeps its own bytes; none replaced another.
        let copies = list(&s, "radio-1");
        assert_eq!(copies.len(), 3);
        for (c, want) in copies
            .iter()
            .rev()
            .zip([b"one".as_slice(), b"two", b"three"])
        {
            assert_eq!(read(&s, c).unwrap(), want);
        }
    }

    #[test]
    fn copies_list_newest_first_and_do_not_touch_snapshots() {
        let (d, s) = store();
        for (t, k) in [
            ("2026-10-09T12:00:00Z", CopyKind::Read),
            ("2026-10-09T13:00:00Z", CopyKind::BeforeFlash),
        ] {
            save(&s, "r", k, t.parse().unwrap(), &[1, 2, 3], None).unwrap();
        }
        let l = list(&s, "r");
        assert_eq!(l[0].kind, CopyKind::BeforeFlash);
        assert!(!d.path().join("snapshots").exists());
    }
}
