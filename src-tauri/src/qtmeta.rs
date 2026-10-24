//! QuickTime metadata that Apple's frameworks read, written straight into an MP4 or MOV
//! that ffmpeg made.
//!
//! Why not ffmpeg: with `-movflags use_metadata_tags`, ffmpeg puts the `mdta` keys under
//! `moov/udta/meta` as a full box. exiftool reads that, but AVFoundation (which Photos uses)
//! lists the items without their key names, so Photos sees no location, no creation date and
//! no keywords. Without that flag, ffmpeg writes a location only into a MOV (`©xyz`); an MP4
//! gets a 3GPP `loci` box that AVFoundation ignores.
//!
//! So after ffmpeg finishes, this module replaces the file's `moov/meta` with an Apple-style
//! one (`hdlr` = `mdta`, `keys`, `ilst`, no version field) and sets `moov/udta/©xyz` for the
//! location. ffmpeg's own tags stay as they are. The `moov` box must be the last box in the
//! file (ffmpeg's default without `+faststart`), so the rewrite never moves `mdat` and no
//! chunk offset changes.

use anyhow::{bail, Context, Result};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// One `mdta` item: a reverse-DNS key and a UTF-8 value.
pub type Item = (String, String);

/// An `mdta` item that is not UTF-8 text (a number, a picture), which another app wrote:
/// its key and its whole `data` box, written back as it was.
type RawItem = (String, Vec<u8>);

fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(body.len() + 8);
    v.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
    v.extend_from_slice(kind);
    v.extend_from_slice(body);
    v
}

/// An Apple `moov/meta` box with the given items, in order, then the raw items whose keys
/// `items` does not set.
fn meta_box(items: &[Item], raw: &[RawItem]) -> Vec<u8> {
    let raw: Vec<&RawItem> = raw
        .iter()
        .filter(|(k, _)| !items.iter().any(|(i, _)| i == k))
        .collect();
    let mut hdlr = vec![0u8; 8];
    hdlr.extend_from_slice(b"mdta");
    hdlr.extend_from_slice(&[0u8; 12]);
    hdlr.push(0);
    let mut keys = vec![0u8; 4];
    keys.extend_from_slice(&((items.len() + raw.len()) as u32).to_be_bytes());
    let mut ilst = Vec::new();
    let text = items.iter().map(|(k, v)| {
        let mut data = 1u32.to_be_bytes().to_vec(); // UTF-8
        data.extend_from_slice(&[0u8; 4]); // default locale
        data.extend_from_slice(v.as_bytes());
        (k, boxed(b"data", &data))
    });
    let raw = raw.iter().map(|(k, b)| (k, b.clone()));
    for (i, (k, item)) in text.chain(raw).enumerate() {
        keys.extend_from_slice(&((k.len() + 8) as u32).to_be_bytes());
        keys.extend_from_slice(b"mdta");
        keys.extend_from_slice(k.as_bytes());
        let mut entry = ((item.len() + 8) as u32).to_be_bytes().to_vec();
        entry.extend_from_slice(&((i + 1) as u32).to_be_bytes());
        entry.extend_from_slice(&item);
        ilst.extend_from_slice(&entry);
    }
    let mut body = boxed(b"hdlr", &hdlr);
    body.extend(boxed(b"keys", &keys));
    body.extend(boxed(b"ilst", &ilst));
    boxed(b"meta", &body)
}

/// A `©xyz` user-data box (QuickTime string: length, language `und`, text).
fn xyz_box(iso6709: &str) -> Vec<u8> {
    let mut body = (iso6709.len() as u16).to_be_bytes().to_vec();
    body.extend_from_slice(&0x55c4u16.to_be_bytes());
    body.extend_from_slice(iso6709.as_bytes());
    boxed(b"\xa9xyz", &body)
}

/// Child boxes of a container body: (type, whole box bytes).
fn children(body: &[u8]) -> Result<Vec<([u8; 4], &[u8])>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 8 <= body.len() {
        let size = u32::from_be_bytes(body[at..at + 4].try_into().unwrap()) as usize;
        let kind: [u8; 4] = body[at + 4..at + 8].try_into().unwrap();
        let (size, hdr) = match size {
            0 => (body.len() - at, 8),
            1 => {
                if at + 16 > body.len() {
                    bail!("truncated box");
                }
                let s = u64::from_be_bytes(body[at + 8..at + 16].try_into().unwrap());
                (usize::try_from(s).unwrap_or(usize::MAX), 16)
            }
            s => (s, 8),
        };
        if size < hdr || size > body.len() - at {
            bail!("bad box size in moov");
        }
        out.push((kind, &body[at..at + size]));
        at += size;
    }
    Ok(out)
}

/// The new `moov` body: the old children without a previous `meta`, the `udta` with the
/// location replaced, then the new `meta`.
fn rebuild_moov(
    body: &[u8],
    items: &[Item],
    raw: &[RawItem],
    location: Option<&str>,
) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(body.len() + 512);
    let mut had_udta = false;
    for (kind, bytes) in children(body)? {
        match &kind {
            b"meta" => {}
            b"udta" => {
                had_udta = true;
                let hdr = if u32::from_be_bytes(bytes[0..4].try_into().unwrap()) == 1 {
                    16
                } else {
                    8
                };
                let mut u = Vec::new();
                for (k, b) in children(&bytes[hdr..])? {
                    if &k != b"\xa9xyz" {
                        u.extend_from_slice(b);
                    }
                }
                if let Some(l) = location {
                    u.extend(xyz_box(l));
                }
                out.extend(boxed(b"udta", &u));
            }
            _ => out.extend_from_slice(bytes),
        }
    }
    if !had_udta {
        if let Some(l) = location {
            out.extend(boxed(b"udta", &xyz_box(l)));
        }
    }
    if !items.is_empty() || !raw.is_empty() {
        out.extend(meta_box(items, raw));
    }
    Ok(out)
}

/// Finds the top-level `moov`: (offset, size, header length).
fn find_moov(f: &mut std::fs::File, path: &Path) -> Result<(u64, u64, u64)> {
    let len = f.metadata()?.len();
    let mut at = 0u64;
    let mut moov: Option<(u64, u64, u64)> = None;
    while at + 8 <= len {
        f.seek(SeekFrom::Start(at))?;
        let mut h = [0u8; 16];
        f.read_exact(&mut h[..8])?;
        let mut size = u32::from_be_bytes(h[0..4].try_into().unwrap()) as u64;
        let mut hdr = 8;
        if size == 1 {
            f.read_exact(&mut h[8..16])?;
            size = u64::from_be_bytes(h[8..16].try_into().unwrap());
            hdr = 16;
        } else if size == 0 {
            size = len - at;
        }
        if size < hdr {
            bail!("bad top-level box in {}", path.display());
        }
        if &h[4..8] == b"moov" {
            moov = Some((at, size, hdr));
        }
        at = at
            .checked_add(size)
            .with_context(|| format!("bad top-level box size in {}", path.display()))?;
    }
    moov.context("no moov box")
}

/// Where the top-level `moov` starts.
pub fn moov_offset(path: &Path) -> Result<u64> {
    let mut f = std::fs::File::open(path)?;
    Ok(find_moov(&mut f, path)?.0)
}

/// Writes `items` as Apple `mdta` metadata and `location` (ISO 6709) as `©xyz` into the
/// MP4 or MOV at `path`, replacing what an earlier call wrote.
pub fn write(path: &Path, items: &[Item], location: Option<&str>) -> Result<()> {
    let mut f = open_locked(path)?;
    let raw = foreign_items(&mut f, path)?;
    write_open(&mut f, path, items, &raw, location)
}

/// Opens `path` for writing under an exclusive lock, which every reader and writer in any
/// process waits for, after finishing a write that a crash interrupted. A volume without
/// locks is written unlocked.
fn open_locked(path: &Path) -> Result<std::fs::File> {
    let mut f = std::fs::File::options()
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let _ = f.lock();
    recover(&mut f, path)?;
    Ok(f)
}

/// Where a write keeps the old `moov` until the new one is on disk: `.<name>.moov` beside
/// the file.
fn journal_path(path: &Path) -> std::path::PathBuf {
    let mut name = std::ffi::OsString::from(".");
    name.push(path.file_name().unwrap_or_default());
    name.push(".moov");
    path.with_file_name(name)
}

const JOURNAL_MAGIC: &[u8; 8] = b"QCMOOV2\0";

/// How many bytes before the `moov` a journal keeps, to know the file it belongs to: the
/// tail of `mdat`, which a metadata write never touches.
const FINGERPRINT: u64 = 4096;

/// The bytes before `start` that tie a journal to its file.
fn fingerprint(f: &mut std::fs::File, start: u64) -> Result<Vec<u8>> {
    let from = start.saturating_sub(FINGERPRINT);
    let mut b = vec![0u8; (start - from) as usize];
    f.seek(SeekFrom::Start(from))?;
    f.read_exact(&mut b)?;
    Ok(b)
}

/// A whole journal: (where the old `moov` starts, the fingerprint, the old `moov`).
fn parse_journal(j: &[u8]) -> Option<(u64, &[u8], &[u8])> {
    let be = |at: usize| Some(u64::from_be_bytes(j.get(at..at + 8)?.try_into().ok()?));
    if j.get(..8)? != JOURNAL_MAGIC {
        return None;
    }
    let (start, fp, size) = (be(8)?, be(16)? as usize, be(24)? as usize);
    let fp_end = 32usize.checked_add(fp)?;
    let fingerprint = j.get(32..fp_end)?;
    let old = j.get(fp_end..)?;
    (old.len() == size && size >= 8 && &old[4..8] == b"moov").then_some((start, fingerprint, old))
}

/// Puts back the old `moov` a journal holds, so a write that a crash cut short leaves the
/// file as it was before that write. A journal cut short itself is dropped: the write had
/// not touched the file yet. So is a journal whose fingerprint does not match the file's
/// bytes: it belongs to another file, and putting it back would damage this one.
fn recover(f: &mut std::fs::File, path: &Path) -> Result<()> {
    let jp = journal_path(path);
    let Ok(j) = std::fs::read(&jp) else {
        return Ok(());
    };
    if let Some((start, fp, old)) = parse_journal(&j) {
        let len = f.metadata()?.len();
        let ours = start <= len && fingerprint(f, start).is_ok_and(|b| b == fp);
        if ours {
            f.seek(SeekFrom::Start(start))?;
            f.write_all(old)?;
            f.set_len(start + old.len() as u64)?;
            f.sync_all()?;
        } else {
            eprintln!(
                "quadcam: {}: a write journal that does not match the file; left the file as it is",
                path.display()
            );
        }
    }
    std::fs::remove_file(&jp)
        .with_context(|| format!("removing the write journal of {}", path.display()))
}

/// Replaces the file's `moov` (it must be the last box) with one carrying `items`, `raw`
/// and `location`. The old `moov` goes to a journal first, so a crash mid-write is undone
/// on the next open.
fn write_open(
    f: &mut std::fs::File,
    path: &Path,
    items: &[Item],
    raw: &[RawItem],
    location: Option<&str>,
) -> Result<()> {
    let len = f.metadata()?.len();
    let (start, size, hdr) = find_moov(f, path)?;
    if start + size != len {
        bail!("the moov box is not at the end of the file; cannot add metadata in place");
    }
    let mut old = vec![0u8; size as usize];
    f.seek(SeekFrom::Start(start))?;
    f.read_exact(&mut old)?;
    let new = boxed(
        b"moov",
        &rebuild_moov(&old[hdr as usize..], items, raw, location)?,
    );
    let jp = journal_path(path);
    let mut j = std::fs::File::create(&jp)
        .with_context(|| format!("writing the write journal of {}", path.display()))?;
    let fp = fingerprint(f, start)?;
    j.write_all(JOURNAL_MAGIC)?;
    j.write_all(&start.to_be_bytes())?;
    j.write_all(&(fp.len() as u64).to_be_bytes())?;
    j.write_all(&size.to_be_bytes())?;
    j.write_all(&fp)?;
    j.write_all(&old)?;
    j.sync_all()?;
    drop(j);
    f.seek(SeekFrom::Start(start))?;
    f.write_all(&new)?;
    f.set_len(start + new.len() as u64)?;
    f.sync_all()?;
    std::fs::remove_file(&jp)?;
    Ok(())
}

/// The `mdta` items in a `moov/meta` body, in key order: the UTF-8 text items, and the
/// others as raw `data` boxes.
fn parse_meta(meta: &[u8]) -> Result<(Vec<Item>, Vec<RawItem>)> {
    let mut keys: Vec<String> = Vec::new();
    let mut values: Vec<(usize, String)> = Vec::new();
    let mut raws: Vec<(usize, Vec<u8>)> = Vec::new();
    for (kind, b) in children(meta)? {
        match &kind {
            b"keys" if b.len() >= 16 => {
                let mut at = 16;
                while at + 8 <= b.len() {
                    let size = u32::from_be_bytes(b[at..at + 4].try_into().unwrap()) as usize;
                    if size < 8 || at + size > b.len() {
                        break;
                    }
                    keys.push(String::from_utf8_lossy(&b[at + 8..at + size]).to_string());
                    at += size;
                }
            }
            b"ilst" => {
                for (idx, entry) in children(&b[8..])? {
                    let index = u32::from_be_bytes(idx) as usize;
                    for (k, data) in children(&entry[8..])? {
                        if &k == b"data" && data.len() >= 16 {
                            let kind = u32::from_be_bytes(data[8..12].try_into().unwrap());
                            if kind == 1 {
                                values.push((
                                    index,
                                    String::from_utf8_lossy(&data[16..]).to_string(),
                                ));
                            } else {
                                raws.push((index, data.to_vec()));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let key = |i: usize| keys.get(i.checked_sub(1)?).cloned();
    Ok((
        values
            .into_iter()
            .filter_map(|(i, v)| Some((key(i)?, v)))
            .collect(),
        raws.into_iter()
            .filter_map(|(i, b)| Some((key(i)?, b)))
            .collect(),
    ))
}

/// Sets the movie's creation and modification time in `moov/mvhd`, in place (the field
/// sizes never change). ffprobe reports this as `creation_time`.
pub fn set_movie_time(path: &Path, t: chrono::DateTime<chrono::Utc>) -> Result<()> {
    const MAC_EPOCH_OFFSET: i64 = 2_082_844_800; // 1904-01-01 to 1970-01-01, in seconds
    let secs = u64::try_from(t.timestamp() + MAC_EPOCH_OFFSET).context("time before 1904")?;
    let mut f = open_locked(path)?;
    let (start, size, hdr) = find_moov(&mut f, path)?;
    if size > 64 << 20 {
        bail!("moov box too large in {}", path.display());
    }
    let mut body = vec![0u8; (size - hdr) as usize];
    f.seek(SeekFrom::Start(start + hdr))?;
    f.read_exact(&mut body)?;
    let mut at = 0usize;
    for (kind, b) in children(&body)? {
        if &kind == b"mvhd" && b.len() >= 28 {
            let field = start + hdr + at as u64 + 12; // after the box header and version/flags
            f.seek(SeekFrom::Start(field))?;
            if b[8] == 1 {
                f.write_all(&secs.to_be_bytes())?;
                f.write_all(&secs.to_be_bytes())?;
            } else {
                let s = u32::try_from(secs).context("time past 2040 needs a version 1 mvhd")?;
                f.write_all(&s.to_be_bytes())?;
                f.write_all(&s.to_be_bytes())?;
            }
            f.sync_all()?;
            return Ok(());
        }
        at += b.len();
    }
    bail!("no mvhd box in {}", path.display())
}

/// True when the file has a top-level `moov` that ends inside the file: a recording that
/// was closed properly. A recorder that lost power leaves `mdat` and no `moov`.
pub fn has_whole_moov(path: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(len) = f.metadata().map(|m| m.len()) else {
        return false;
    };
    find_moov(&mut f, path).is_ok_and(|(start, size, _)| start + size <= len)
}

/// The movie's creation time from `moov/mvhd`, which ffprobe reports as `creation_time`.
/// None when it is zero (not set).
pub fn movie_time(path: &Path) -> Result<Option<chrono::DateTime<chrono::Utc>>> {
    const MAC_EPOCH_OFFSET: i64 = 2_082_844_800;
    let mut f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let (start, size, hdr) = find_moov(&mut f, path)?;
    if size > 64 << 20 {
        bail!("moov box too large in {}", path.display());
    }
    let mut body = vec![0u8; (size - hdr) as usize];
    f.seek(SeekFrom::Start(start + hdr))?;
    f.read_exact(&mut body)?;
    for (kind, b) in children(&body)? {
        if &kind == b"mvhd" && b.len() >= 28 {
            let secs = if b[8] == 1 {
                u64::from_be_bytes(b[12..20].try_into().unwrap())
            } else {
                u32::from_be_bytes(b[12..16].try_into().unwrap()) as u64
            };
            if secs == 0 {
                return Ok(None);
            }
            let unix = i64::try_from(secs).context("mvhd time out of range")? - MAC_EPOCH_OFFSET;
            return Ok(chrono::DateTime::from_timestamp(unix, 0));
        }
    }
    bail!("no mvhd box in {}", path.display())
}

/// Reads the Apple `mdta` items quadcam (or anything else) wrote into `moov/meta`.
pub fn read(path: &Path) -> Result<Vec<Item>> {
    Ok(read_info(path)?.0)
}

/// The `mdta` items and the movie duration in seconds (from `mvhd`).
pub fn read_info(path: &Path) -> Result<(Vec<Item>, Option<f64>)> {
    if journal_path(path).exists() {
        // A write that a crash cut short: put the file back first.
        drop(open_locked(path)?);
    }
    let mut f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let _ = f.lock_shared();
    let (items, _, duration) = read_open(&mut f, path)?;
    Ok((items, duration))
}

/// The other apps' items that are not text, which a write keeps.
fn foreign_items(f: &mut std::fs::File, path: &Path) -> Result<Vec<RawItem>> {
    Ok(read_open(f, path)?.1)
}

fn read_open(
    f: &mut std::fs::File,
    path: &Path,
) -> Result<(Vec<Item>, Vec<RawItem>, Option<f64>)> {
    let (start, size, hdr) = find_moov(f, path)?;
    if size > 64 << 20 {
        bail!("moov box too large in {}", path.display());
    }
    let mut body = vec![0u8; (size - hdr) as usize];
    f.seek(SeekFrom::Start(start + hdr))?;
    f.read_exact(&mut body)?;
    let mut items = Vec::new();
    let mut raw = Vec::new();
    let mut duration = None;
    for (kind, b) in children(&body)? {
        match &kind {
            b"meta" => {
                // Apple's meta has no version field; ISO's (ffmpeg's) has four bytes of it.
                let inner = &b[8..];
                let skip = if inner.len() >= 8 && &inner[4..8] == b"hdlr" {
                    0
                } else {
                    4
                };
                (items, raw) = parse_meta(&inner[skip.min(inner.len())..])?;
            }
            b"mvhd" => duration = mvhd_duration(&b[8..]),
            _ => {}
        }
    }
    Ok((items, raw, duration))
}

fn mvhd_duration(b: &[u8]) -> Option<f64> {
    let be32 = |at: usize| Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?) as u64);
    let (scale, dur) = if *b.first()? == 1 {
        (
            be32(20)?,
            u64::from_be_bytes(b.get(24..32)?.try_into().ok()?),
        )
    } else {
        (be32(12)?, be32(16)?)
    };
    (scale > 0).then(|| dur as f64 / scale as f64)
}

/// Reads the items, lets `edit` change them, and writes them back. The location item, if
/// any, is also written as `©xyz`. The file's modification time is kept.
pub fn update(path: &Path, edit: impl FnOnce(&mut Vec<Item>)) -> Result<()> {
    let mtime = path.metadata().and_then(|m| m.modified()).ok();
    // One lock from the read to the write: two edits of one file never interleave.
    let mut f = open_locked(path)?;
    let (mut items, raw, _) = read_open(&mut f, path)?;
    let before = items.clone();
    edit(&mut items);
    // An empty value the edit set removes the item; another app's blank item stays.
    items.retain(|it| !it.1.trim().is_empty() || before.contains(it));
    let loc = items
        .iter()
        .find(|(k, _)| k == "com.apple.quicktime.location.ISO6709")
        .map(|(_, v)| v.clone());
    write_open(&mut f, path, &items, &raw, loc.as_deref())?;
    if let Some(t) = mtime {
        let _ = f.set_modified(t);
    }
    Ok(())
}

/// Sets one item, replacing an earlier value; an empty value removes it.
pub fn set(items: &mut Vec<Item>, key: &str, value: &str) {
    items.retain(|(k, _)| k != key);
    if !value.trim().is_empty() {
        items.push((key.to_string(), value.to_string()));
    }
}

/// The value of one item.
pub fn get<'a>(items: &'a [Item], key: &str) -> Option<&'a str> {
    items
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuilds_moov_and_replaces_its_own_meta() {
        let mvhd = boxed(b"mvhd", &[0u8; 100]);
        let old_xyz = xyz_box("+00.0000+000.0000/");
        let udta = boxed(
            b"udta",
            &[boxed(b"\xa9swr", b"\0\x03\x55\xc4abc"), old_xyz].concat(),
        );
        let body = [mvhd.clone(), udta].concat();
        let items = vec![(
            "com.apple.quicktime.make".to_string(),
            "Fat Shark".to_string(),
        )];
        let once = rebuild_moov(&body, &items, &[], Some("+40.6892-074.0445/")).unwrap();
        let twice = rebuild_moov(&once, &items, &[], Some("+40.6892-074.0445/")).unwrap();
        assert_eq!(once, twice, "running it again changes nothing");
        let kids = children(&once).unwrap();
        let kinds: Vec<&[u8; 4]> = kids.iter().map(|(k, _)| k).collect();
        assert_eq!(kinds, [b"mvhd", b"udta", b"meta"]);
        let udta = children(&kids[1].1[8..]).unwrap();
        assert_eq!(udta.len(), 2, "©swr kept, one ©xyz");
        assert!(udta[1].1.ends_with(b"+40.6892-074.0445/"));
        let meta = kids[2].1;
        let inner = children(&meta[8..]).unwrap();
        assert_eq!(&inner[0].0, b"hdlr");
        assert_eq!(&inner[0].1[16..20], b"mdta");
        assert!(inner[1]
            .1
            .windows(24)
            .any(|w| w == b"com.apple.quicktime.make"));
        assert!(inner[2].1.ends_with(b"Fat Shark"));
    }

    /// A small file shaped as ffmpeg writes one: `ftyp`, `mdat`, then `moov` last.
    fn movie(dir: &Path) -> std::path::PathBuf {
        let p = dir.join("a.mp4");
        let mvhd = boxed(b"mvhd", &[0u8; 100]);
        let bytes = [
            boxed(b"ftyp", b"isom\0\0\x02\0"),
            boxed(b"mdat", &[7u8; 1000]),
            boxed(b"moov", &mvhd),
        ]
        .concat();
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn item(k: &str, v: &str) -> Item {
        (k.to_string(), v.to_string())
    }

    #[test]
    fn a_write_cut_short_is_undone_on_the_next_read() {
        let dir = tempfile::tempdir().unwrap();
        let p = movie(dir.path());
        write(&p, &[item("app.quadcam.rating", "3")], None).unwrap();
        let before = std::fs::read(&p).unwrap();
        assert!(!journal_path(&p).exists(), "a finished write leaves no journal");

        // A crash halfway: the journal is on disk, the new moov half written.
        let mut f = std::fs::File::options()
            .read(true)
            .write(true)
            .open(&p)
            .unwrap();
        let (start, size, _) = find_moov(&mut f, &p).unwrap();
        let fp = fingerprint(&mut f, start).unwrap();
        let mut j = JOURNAL_MAGIC.to_vec();
        j.extend_from_slice(&start.to_be_bytes());
        j.extend_from_slice(&(fp.len() as u64).to_be_bytes());
        j.extend_from_slice(&size.to_be_bytes());
        j.extend_from_slice(&fp);
        j.extend_from_slice(&before[start as usize..]);
        std::fs::write(journal_path(&p), &j).unwrap();
        f.seek(SeekFrom::Start(start)).unwrap();
        f.write_all(&[0xffu8; 40]).unwrap();
        f.set_len(start + 40).unwrap();
        drop(f);

        assert_eq!(get(&read(&p).unwrap(), "app.quadcam.rating"), Some("3"));
        assert_eq!(std::fs::read(&p).unwrap(), before);
        assert!(!journal_path(&p).exists());

        // A journal from another file of this name (its mdat differs) is not put back.
        let mut other = std::fs::read(&p).unwrap();
        other[start as usize - 1] ^= 0xff;
        std::fs::write(&p, &other).unwrap();
        std::fs::write(journal_path(&p), &j).unwrap();
        let mut f = std::fs::File::options().write(true).open(&p).unwrap();
        f.seek(SeekFrom::Start(start)).unwrap();
        f.write_all(b"\0\0\0\x08free").unwrap();
        drop(f);
        let damaged = std::fs::read(&p).unwrap();
        assert!(read(&p).is_err(), "the file's own moov is gone");
        assert_eq!(std::fs::read(&p).unwrap(), damaged, "left as it is");
        assert!(!journal_path(&p).exists());
        std::fs::write(&p, &before).unwrap();

        // A journal cut short: the write had not started, so the file stays as it is.
        std::fs::write(journal_path(&p), &JOURNAL_MAGIC[..5]).unwrap();
        update(&p, |items| set(items, "app.quadcam.flag", "pick")).unwrap();
        let items = read(&p).unwrap();
        assert_eq!(get(&items, "app.quadcam.rating"), Some("3"));
        assert_eq!(get(&items, "app.quadcam.flag"), Some("pick"));
        assert!(!journal_path(&p).exists());
    }

    #[test]
    fn edits_from_many_threads_all_land() {
        let dir = tempfile::tempdir().unwrap();
        let p = movie(dir.path());
        std::thread::scope(|s| {
            for n in 0..8 {
                let p = &p;
                s.spawn(move || {
                    let key = format!("app.quadcam.test{n}");
                    update(p, |items| set(items, &key, &"x".repeat(50 * n + 1))).unwrap();
                });
            }
        });
        let items = read(&p).unwrap();
        for n in 0..8 {
            assert!(get(&items, &format!("app.quadcam.test{n}")).is_some(), "{n}");
        }
    }

    #[test]
    fn another_apps_number_and_picture_items_survive_an_edit() {
        let dir = tempfile::tempdir().unwrap();
        let p = movie(dir.path());
        write(
            &p,
            &[item("com.apple.quicktime.make", "DJI"), item("com.other.note", " ")],
            None,
        )
        .unwrap();
        // Add a type-21 (integer) item the way another app writes one.
        let mut f = std::fs::File::open(&p).unwrap();
        let (start, size, hdr) = find_moov(&mut f, &p).unwrap();
        let mut body = vec![0u8; (size - hdr) as usize];
        f.seek(SeekFrom::Start(start + hdr)).unwrap();
        f.read_exact(&mut body).unwrap();
        drop(f);
        let int = boxed(b"data", &[0, 0, 0, 21, 0, 0, 0, 0, 0, 0, 0, 42]);
        let new = boxed(
            b"moov",
            &rebuild_moov(
                &body,
                &[item("com.apple.quicktime.make", "DJI"), item("com.other.note", " ")],
                &[("com.dji.gimbal".to_string(), int.clone())],
                None,
            )
            .unwrap(),
        );
        let mut bytes = std::fs::read(&p).unwrap();
        bytes.truncate(start as usize);
        bytes.extend_from_slice(&new);
        std::fs::write(&p, bytes).unwrap();

        update(&p, |items| set(items, "app.quadcam.rating", "4")).unwrap();
        let mut f = std::fs::File::open(&p).unwrap();
        let (items, raw, _) = read_open(&mut f, &p).unwrap();
        assert_eq!(get(&items, "com.apple.quicktime.make"), Some("DJI"));
        assert_eq!(get(&items, "app.quadcam.rating"), Some("4"));
        assert_eq!(get(&items, "com.other.note"), Some(" "), "a blank item stays");
        assert_eq!(raw, vec![("com.dji.gimbal".to_string(), int)]);
    }

    #[test]
    fn malformed_box_sizes_are_errors_not_panics() {
        // A largesize box whose size is shorter than its own 16-byte header.
        let mut udta = 1u32.to_be_bytes().to_vec();
        udta.extend_from_slice(b"udta");
        udta.extend_from_slice(&12u64.to_be_bytes());
        let body = [boxed(b"mvhd", &[0u8; 100]), udta].concat();
        assert!(rebuild_moov(&body, &[], &[], Some("+1+2/")).is_err());

        // A top-level largesize near u64::MAX.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("b.mp4");
        let mut bytes = boxed(b"ftyp", b"isom");
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(b"free");
        bytes.extend_from_slice(&(u64::MAX - 4).to_be_bytes());
        bytes.extend(boxed(b"moov", &[]));
        std::fs::write(&p, bytes).unwrap();
        assert!(read(&p).is_err());
    }
}
