//! The content-addressed blob store (design 7.1): every file a backup holds is stored once,
//! by content, under `<gear>/blobs/<xx>/<key>`.
//!
//! - A blob's key is its XXH64 (16 hex digits, seed 0, the hash `identity` specifies) and
//!   its size: `<xxh64>-<size>`. Two files of different sizes never share a key.
//! - A put whose key exists compares the bytes before it reuses the blob. Different bytes
//!   under one key (a hash collision) refuse, so two files are never merged.
//! - A put writes `blobs/tmp/<random>`, fsyncs it and renames it into place. A crash leaves
//!   at most a temporary file or a blob no manifest names; `collect` removes both.
//! - `lock` is the store's write lock: a snapshot holds it from its first blob to its
//!   manifest, and `collect` holds it, so a collection never takes a blob a snapshot in
//!   another process is about to name.
//!
//! Blobs are plain, uncompressed bytes: a person can read them without QuadCam.

use super::store::Store;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use xxhash_rust::xxh64::Xxh64;

/// A blob's identity: its content hash and size.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
pub struct BlobRef {
    /// XXH64 as 16 hex digits.
    pub xxh64: String,
    pub size: u64,
}

impl BlobRef {
    /// The blob's name in the store: `<xxh64>-<size>`.
    pub fn key(&self) -> String {
        key(&self.xxh64, self.size)
    }
}

/// The blob name for a hash and a size.
pub fn key(xxh64: &str, size: u64) -> String {
    format!("{xxh64}-{size}")
}

/// XXH64 of bytes as 16 hex digits.
pub fn hash(bytes: &[u8]) -> String {
    format!("{:016x}", xxhash_rust::xxh64::xxh64(bytes, 0))
}

/// The blob store in a gear folder.
#[derive(Debug, Clone)]
pub struct Blobs {
    store: Store,
}

/// What a collection removed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct Collected {
    /// Blobs no manifest, log or staged change named.
    pub blobs: u32,
    pub bytes: u64,
    /// Temporary files a crash left.
    pub temp_files: u32,
}

/// Reads in pieces this size, so progress moves on slow links.
const CHUNK: usize = 256 * 1024;

impl Blobs {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// `<gear>/blobs/<xx>/<xxh64>-<size>`.
    pub fn path(&self, b: &BlobRef) -> PathBuf {
        self.store.blob_path(&b.key())
    }

    fn tmp_dir(&self) -> PathBuf {
        self.store.blobs_dir().join("tmp")
    }

    /// True when the blob is stored.
    pub fn has(&self, b: &BlobRef) -> bool {
        self.path(b).is_file()
    }

    /// Takes the store's write lock (`<gear>/blobs/store.lock`) until the guard drops.
    pub fn lock(&self) -> Result<File> {
        let dir = self.store.blobs_dir();
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let f = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join("store.lock"))
            .context("opening the backup store lock")?;
        f.lock().context("locking the backup store")?;
        Ok(f)
    }

    /// Stores bytes; returns their ref. A blob that exists is compared, not written.
    pub fn put(&self, bytes: &[u8]) -> Result<BlobRef> {
        let r = BlobRef {
            xxh64: hash(bytes),
            size: bytes.len() as u64,
        };
        let path = self.path(&r);
        if path.is_file() {
            if std::fs::read(&path)? != bytes {
                bail!(collision(&r));
            }
            return Ok(r);
        }
        let (tmp, mut f) = self.temp()?;
        f.write_all(bytes)?;
        self.finish(tmp, f, &r)?;
        Ok(r)
    }

    /// Stores a file, reading it once: the bytes go to a temporary file while they are
    /// hashed. `progress` gets the bytes read so far. A blob that exists is compared, and
    /// the copy dropped.
    pub fn put_file(&self, src: &Path, progress: &mut dyn FnMut(u64)) -> Result<BlobRef> {
        let mut input = File::open(src).with_context(|| format!("opening {}", src.display()))?;
        let (tmp, mut out) = self.temp()?;
        let mut h = Xxh64::new(0);
        let mut buf = vec![0u8; CHUNK];
        let mut size = 0u64;
        let read = (|| -> Result<()> {
            loop {
                let n = input
                    .read(&mut buf)
                    .with_context(|| format!("reading {}", src.display()))?;
                if n == 0 {
                    return Ok(());
                }
                h.update(&buf[..n]);
                out.write_all(&buf[..n])?;
                size += n as u64;
                progress(size);
            }
        })();
        if let Err(e) = read {
            drop(out);
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        let r = BlobRef {
            xxh64: format!("{:016x}", h.digest()),
            size,
        };
        let path = self.path(&r);
        if path.is_file() {
            drop(out);
            let same = same_bytes(&tmp, &path);
            let _ = std::fs::remove_file(&tmp);
            if !same? {
                bail!(collision(&r));
            }
            return Ok(r);
        }
        self.finish(tmp, out, &r)?;
        Ok(r)
    }

    /// The blob's bytes, checked against its ref.
    pub fn get(&self, b: &BlobRef) -> Result<Vec<u8>> {
        let path = self.path(b);
        let bytes = std::fs::read(&path).with_context(|| {
            format!(
                "blob {} is missing from the backup store ({})",
                b.key(),
                path.display()
            )
        })?;
        if bytes.len() as u64 != b.size || hash(&bytes) != b.xxh64 {
            bail!(
                "blob {} does not match its hash: the store is damaged",
                b.key()
            );
        }
        Ok(bytes)
    }

    /// True when the blob is stored and its bytes match its ref.
    pub fn verify(&self, b: &BlobRef) -> bool {
        self.get(b).is_ok()
    }

    /// Every stored blob, with its size on disk.
    pub fn list(&self) -> Result<Vec<(String, u64)>> {
        let mut out = Vec::new();
        let dir = self.store.blobs_dir();
        let Ok(fans) = std::fs::read_dir(&dir) else {
            return Ok(out);
        };
        for fan in fans.flatten() {
            let name = fan.file_name().to_string_lossy().to_string();
            if name.len() != 2 || !fan.path().is_dir() {
                continue;
            }
            for b in std::fs::read_dir(fan.path())?.flatten() {
                if let Ok(m) = b.metadata() {
                    if m.is_file() {
                        out.push((b.file_name().to_string_lossy().to_string(), m.len()));
                    }
                }
            }
        }
        out.sort();
        Ok(out)
    }

    /// Removes every blob whose key is not in `keep`, and the temporary files a crash
    /// left. The caller holds `lock` and builds `keep` under it.
    pub fn collect(&self, keep: &HashSet<String>) -> Result<Collected> {
        let mut c = Collected::default();
        for (k, size) in self.list()? {
            if !keep.contains(&k) {
                let p = self.store.blob_path(&k);
                std::fs::remove_file(&p).with_context(|| format!("removing {}", p.display()))?;
                c.blobs += 1;
                c.bytes += size;
            }
        }
        if let Ok(rd) = std::fs::read_dir(self.tmp_dir()) {
            for t in rd.flatten() {
                if std::fs::remove_file(t.path()).is_ok() {
                    c.temp_files += 1;
                }
            }
        }
        Ok(c)
    }

    fn temp(&self) -> Result<(PathBuf, File)> {
        let dir = self.tmp_dir();
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        loop {
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let p = dir.join(format!("put-{}-{n}", std::process::id()));
            match File::options().write(true).create_new(true).open(&p) {
                Ok(f) => return Ok((p, f)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e).context("creating a temporary blob"),
            }
        }
    }

    fn finish(&self, tmp: PathBuf, f: File, r: &BlobRef) -> Result<()> {
        f.sync_all()?;
        drop(f);
        let path = self.path(r);
        let dir = path.parent().context("blob path has no folder")?;
        std::fs::create_dir_all(dir)?;
        std::fs::rename(&tmp, &path).with_context(|| format!("storing blob {}", r.key()))?;
        Ok(())
    }
}

fn collision(r: &BlobRef) -> String {
    format!(
        "Refused: another file with hash {} and size {} is already stored with different bytes; this file was not stored.",
        r.xxh64, r.size
    )
}

fn same_bytes(a: &Path, b: &Path) -> Result<bool> {
    let (mut x, mut y) = (File::open(a)?, File::open(b)?);
    let (mut bx, mut by) = (vec![0u8; CHUNK], vec![0u8; CHUNK]);
    loop {
        let n = read_full(&mut x, &mut bx)?;
        let m = read_full(&mut y, &mut by)?;
        if n != m || bx[..n] != by[..m] {
            return Ok(false);
        }
        if n == 0 {
            return Ok(true);
        }
    }
}

fn read_full(f: &mut File, buf: &mut [u8]) -> Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        let k = f.read(&mut buf[n..])?;
        if k == 0 {
            break;
        }
        n += k;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blobs() -> (tempfile::TempDir, Blobs) {
        let d = tempfile::tempdir().unwrap();
        let b = Blobs::new(Store::new(d.path().join("gear")));
        (d, b)
    }

    #[test]
    fn put_once_get_back() {
        let (_d, b) = blobs();
        let r = b.put(b"hello").unwrap();
        assert_eq!(r.size, 5);
        assert_eq!(r.xxh64, hash(b"hello"));
        assert_eq!(b.get(&r).unwrap(), b"hello");
        assert_eq!(b.put(b"hello").unwrap(), r);
        assert_eq!(b.list().unwrap().len(), 1);
        assert!(b
            .path(&r)
            .ends_with(format!("{}/{}", &r.xxh64[..2], r.key())));
    }

    #[test]
    fn put_file_streams_and_dedupes() {
        let (d, b) = blobs();
        let src = d.path().join("big.bin");
        let data: Vec<u8> = (0..(CHUNK * 2 + 17)).map(|i| (i % 251) as u8).collect();
        std::fs::write(&src, &data).unwrap();
        let mut seen = Vec::new();
        let r = b.put_file(&src, &mut |n| seen.push(n)).unwrap();
        assert_eq!(
            r,
            BlobRef {
                xxh64: hash(&data),
                size: data.len() as u64
            }
        );
        assert_eq!(seen.last(), Some(&(data.len() as u64)));
        assert!(seen.len() >= 3, "progress per chunk");
        let again = b.put_file(&src, &mut |_| {}).unwrap();
        assert_eq!(again, r);
        assert_eq!(b.list().unwrap().len(), 1);
        assert_eq!(
            std::fs::read_dir(b.tmp_dir()).unwrap().count(),
            0,
            "no temp left"
        );
    }

    #[test]
    fn a_collision_refuses_and_keeps_the_stored_bytes() {
        let (_d, b) = blobs();
        let r = b.put(b"first").unwrap();
        // Plant other bytes of the same size under the key, as a collision would.
        std::fs::write(b.path(&r), b"FIRST").unwrap();
        let e = b.put(b"first").unwrap_err();
        assert!(e.to_string().starts_with("Refused"), "{e}");
        assert_eq!(std::fs::read(b.path(&r)).unwrap(), b"FIRST");
        assert!(!b.verify(&r), "get checks the hash");
    }

    #[test]
    fn collect_keeps_named_blobs_and_clears_temp() {
        let (_d, b) = blobs();
        let a = b.put(b"a").unwrap();
        let x = b.put(b"x").unwrap();
        std::fs::create_dir_all(b.tmp_dir()).unwrap();
        std::fs::write(b.tmp_dir().join("put-crashed"), b"half").unwrap();
        let _lock = b.lock().unwrap();
        let c = b.collect(&HashSet::from([a.key()])).unwrap();
        assert_eq!((c.blobs, c.bytes, c.temp_files), (1, 1, 1));
        assert!(b.has(&a));
        assert!(!b.has(&x));
    }
}
