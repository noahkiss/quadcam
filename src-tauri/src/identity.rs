//! Content identity: the ids that name a clip in the library and on a card. They are
//! written into files (`app.quadcam.source`) and the index, and compared on every card
//! insert, so each is a specified hash that no toolchain update can change.
//!
//! - **Source fingerprint** (a DVR clip): `x` and 16 lowercase hex digits of XXH64, seed 0,
//!   over the file length as 8 little-endian bytes, the first min(length, 1 MiB) bytes, and,
//!   when the file is longer than 1 MiB, its last min(length - 1 MiB, 1 MiB) bytes.
//! - **Head id** (a library file QuadCam did not write): `hx` and 16 hex digits of XXH64,
//!   seed 0, over the file's first bytes up to 1 MiB or the `moov` box, whichever comes
//!   first. Metadata rewrites touch only `moov`, so the id holds.
//!
//! Ids from QuadCam 0.4 and earlier (16 hex digits, or `h` and 16) came from Rust's
//! `DefaultHasher`. `legacy` computes them again with an explicit SipHash-1-3 over the same
//! bytes, so old ids stay recognizable after the standard library changes its hasher.

use crate::qtmeta;
use anyhow::Result;
use std::hash::Hasher;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use xxhash_rust::xxh64::Xxh64;

/// Bytes read from each end of a clip for its fingerprint.
const SAMPLE_BYTES: u64 = 1 << 20;

/// The length and the sampled head and tail of a clip.
struct Sample {
    len: u64,
    head: Vec<u8>,
    tail: Option<Vec<u8>>,
}

fn sample(path: &Path) -> Result<Sample> {
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let mut head = vec![0u8; SAMPLE_BYTES.min(len) as usize];
    f.read_exact(&mut head)?;
    let tail = if len > SAMPLE_BYTES {
        let n = SAMPLE_BYTES.min(len - SAMPLE_BYTES);
        f.seek(SeekFrom::Start(len - n))?;
        let mut buf = vec![0u8; n as usize];
        f.read_exact(&mut buf)?;
        Some(buf)
    } else {
        None
    };
    Ok(Sample { len, head, tail })
}

/// A library file's first bytes before `moov`, up to 1 MiB.
fn head(path: &Path) -> Result<Vec<u8>> {
    let limit = qtmeta::moov_offset(path)
        .ok()
        .filter(|&o| o > 0)
        .unwrap_or(u64::MAX)
        .min(SAMPLE_BYTES);
    let mut buf = Vec::with_capacity(limit as usize);
    std::fs::File::open(path)?
        .take(limit)
        .read_to_end(&mut buf)?;
    Ok(buf)
}

fn xxh(parts: &[&[u8]]) -> u64 {
    let mut h = Xxh64::new(0);
    for p in parts {
        h.update(p);
    }
    h.digest()
}

fn fingerprint_of(s: &Sample) -> String {
    let len = s.len.to_le_bytes();
    let tail = s.tail.as_deref().unwrap_or(&[]);
    format!("x{:016x}", xxh(&[&len, &s.head, tail]))
}

/// A DVR clip's source fingerprint.
pub fn fingerprint(path: &Path) -> Result<String> {
    Ok(fingerprint_of(&sample(path)?))
}

/// A DVR clip's source fingerprint and its 0.4 id, from one read.
pub fn fingerprints(path: &Path) -> Result<(String, String)> {
    let s = sample(path)?;
    Ok((fingerprint_of(&s), legacy::fingerprint_of(&s)))
}

/// The id of a library file QuadCam did not write.
pub fn head_id(path: &Path) -> Result<String> {
    Ok(format!("hx{:016x}", xxh(&[&head(path)?])))
}

/// A library file's head id and its 0.4 id, from one read.
pub fn head_ids(path: &Path) -> Result<(String, String)> {
    let h = head(path)?;
    Ok((format!("hx{:016x}", xxh(&[&h])), legacy::head_id_of(&h)))
}

/// True for an id QuadCam 0.4 or earlier made (a detached cut's `id@start-end` included).
pub fn is_legacy(id: &str) -> bool {
    let base = id.split('@').next().unwrap_or(id);
    let hex = |s: &str| s.len() == 16 && s.bytes().all(|c| c.is_ascii_hexdigit());
    hex(base) || base.strip_prefix('h').is_some_and(hex)
}

/// The ids QuadCam 0.4 and earlier wrote: `DefaultHasher` as it was then, which is
/// SipHash-1-3 with zero keys over the bytes below.
pub mod legacy {
    use super::*;
    use siphasher::sip::SipHasher13;

    fn sip(parts: &[&[u8]]) -> u64 {
        let mut h = SipHasher13::new_with_keys(0, 0);
        for p in parts {
            h.write(p);
        }
        h.finish()
    }

    /// `len.hash(h); head.hash(h); tail.hash(h)`: the length, then each sample after its
    /// own length (a slice's hash writes its length first).
    pub(super) fn fingerprint_of(s: &Sample) -> String {
        let len = s.len.to_le_bytes();
        let head_len = (s.head.len() as u64).to_le_bytes();
        let v = match &s.tail {
            Some(t) => {
                let tail_len = (t.len() as u64).to_le_bytes();
                sip(&[&len, &head_len, &s.head, &tail_len, t])
            }
            None => sip(&[&len, &head_len, &s.head]),
        };
        format!("{v:016x}")
    }

    pub(super) fn head_id_of(h: &[u8]) -> String {
        format!("h{:016x}", sip(&[&(h.len() as u64).to_le_bytes(), h]))
    }

    /// The 0.4 source fingerprint of a DVR clip.
    pub fn fingerprint(path: &Path) -> Result<String> {
        Ok(fingerprint_of(&sample(path)?))
    }

    /// The 0.4 id of a library file QuadCam did not write.
    pub fn head_id(path: &Path) -> Result<String> {
        Ok(head_id_of(&head(path)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::tests::pattern;

    /// The ids 0.4 wrote for these bytes (`DefaultHasher`, recorded before it was
    /// replaced). The legacy functions must keep giving them.
    #[test]
    fn legacy_ids_match_what_0_4_wrote() {
        let d = tempfile::tempdir().unwrap();
        let small = d.path().join("small");
        std::fs::write(&small, pattern(1000)).unwrap();
        let big = d.path().join("big");
        std::fs::write(&big, pattern(3 * (1 << 20) + 123)).unwrap();
        assert_eq!(legacy::fingerprint(&small).unwrap(), "544ed76b6260909f");
        assert_eq!(legacy::fingerprint(&big).unwrap(), "7d3742677467c662");
        let head = d.path().join("head");
        std::fs::write(&head, pattern(2 << 20)).unwrap();
        assert_eq!(legacy::head_id(&small).unwrap(), "h59ffcab2ee5d62ae");
        assert_eq!(legacy::head_id(&head).unwrap(), "hec51a6f45c0fb3e4");
    }

    /// The current ids for the same bytes. A change here breaks every library.
    #[test]
    fn current_ids_test_vector() {
        let d = tempfile::tempdir().unwrap();
        let small = d.path().join("small");
        std::fs::write(&small, pattern(1000)).unwrap();
        let big = d.path().join("big");
        std::fs::write(&big, pattern(3 * (1 << 20) + 123)).unwrap();
        let head = d.path().join("head");
        std::fs::write(&head, pattern(2 << 20)).unwrap();
        assert_eq!(fingerprint(&small).unwrap(), "x1c63a88ed3c3c2d9");
        assert_eq!(fingerprint(&big).unwrap(), "x472bd6ccd6749ca4");
        assert_eq!(head_id(&head).unwrap(), "hxd516db1b487859c5");
        assert_eq!(
            fingerprints(&big).unwrap(),
            (
                fingerprint(&big).unwrap(),
                legacy::fingerprint(&big).unwrap()
            )
        );
        assert_eq!(
            head_ids(&head).unwrap(),
            (head_id(&head).unwrap(), legacy::head_id(&head).unwrap())
        );
    }

    /// XXH64 itself, against the reference test vectors.
    #[test]
    fn xxh64_reference() {
        assert_eq!(xxh(&[b""]), 0xef46db3751d8e999);
        assert_eq!(xxh(&[b"a"]), 0xd24ec4f1a98c6e5b);
        assert_eq!(xxh(&[b"ab", b"c"]), 0x44bc2cf5ad770999);
    }

    #[test]
    fn legacy_ids_are_recognized() {
        assert!(is_legacy("544ed76b6260909f"));
        assert!(is_legacy("h59ffcab2ee5d62ae"));
        assert!(is_legacy("544ed76b6260909f@0.2-0.9"));
        assert!(!is_legacy("x544ed76b6260909f"));
        assert!(!is_legacy("hx59ffcab2ee5d62ae"));
    }
}
