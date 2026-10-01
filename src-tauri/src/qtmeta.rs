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

fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(body.len() + 8);
    v.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
    v.extend_from_slice(kind);
    v.extend_from_slice(body);
    v
}

/// An Apple `moov/meta` box with the given items, in order.
pub fn meta_box(items: &[Item]) -> Vec<u8> {
    let mut hdlr = vec![0u8; 8];
    hdlr.extend_from_slice(b"mdta");
    hdlr.extend_from_slice(&[0u8; 12]);
    hdlr.push(0);
    let mut keys = vec![0u8; 4];
    keys.extend_from_slice(&(items.len() as u32).to_be_bytes());
    let mut ilst = Vec::new();
    for (i, (k, v)) in items.iter().enumerate() {
        keys.extend_from_slice(&((k.len() + 8) as u32).to_be_bytes());
        keys.extend_from_slice(b"mdta");
        keys.extend_from_slice(k.as_bytes());
        let mut data = 1u32.to_be_bytes().to_vec(); // UTF-8
        data.extend_from_slice(&[0u8; 4]); // default locale
        data.extend_from_slice(v.as_bytes());
        let item = boxed(b"data", &data);
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
        let size = match size {
            0 => body.len() - at,
            1 => {
                if at + 16 > body.len() {
                    bail!("truncated box");
                }
                u64::from_be_bytes(body[at + 8..at + 16].try_into().unwrap()) as usize
            }
            s => s,
        };
        if size < 8 || at + size > body.len() {
            bail!("bad box size in moov");
        }
        out.push((kind, &body[at..at + size]));
        at += size;
    }
    Ok(out)
}

/// The new `moov` body: the old children without a previous `meta`, the `udta` with the
/// location replaced, then the new `meta`.
fn rebuild_moov(body: &[u8], items: &[Item], location: Option<&str>) -> Result<Vec<u8>> {
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
    if !items.is_empty() {
        out.extend(meta_box(items));
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
        at += size;
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
    let mut f = std::fs::File::options()
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let len = f.metadata()?.len();
    let (start, size, hdr) = find_moov(&mut f, path)?;
    if start + size != len {
        bail!("the moov box is not at the end of the file; cannot add metadata in place");
    }
    let mut body = vec![0u8; (size - hdr) as usize];
    f.seek(SeekFrom::Start(start + hdr))?;
    f.read_exact(&mut body)?;
    let new = boxed(b"moov", &rebuild_moov(&body, items, location)?);
    f.seek(SeekFrom::Start(start))?;
    f.write_all(&new)?;
    f.set_len(start + new.len() as u64)?;
    f.sync_all()?;
    Ok(())
}

/// The `mdta` items in a `moov/meta` body, in key order. Items that are not UTF-8 text are
/// left out.
fn parse_meta(meta: &[u8]) -> Result<Vec<Item>> {
    let mut keys: Vec<String> = Vec::new();
    let mut values: Vec<(usize, String)> = Vec::new();
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
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(values
        .into_iter()
        .filter_map(|(i, v)| keys.get(i.checked_sub(1)?).map(|k| (k.clone(), v)))
        .collect())
}

/// Sets the movie's creation and modification time in `moov/mvhd`, in place (the field
/// sizes never change). ffprobe reports this as `creation_time`.
pub fn set_movie_time(path: &Path, t: chrono::DateTime<chrono::Utc>) -> Result<()> {
    const MAC_EPOCH_OFFSET: i64 = 2_082_844_800; // 1904-01-01 to 1970-01-01, in seconds
    let secs = u64::try_from(t.timestamp() + MAC_EPOCH_OFFSET).context("time before 1904")?;
    let mut f = std::fs::File::options()
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
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

/// Reads the Apple `mdta` items quadcam (or anything else) wrote into `moov/meta`.
pub fn read(path: &Path) -> Result<Vec<Item>> {
    Ok(read_info(path)?.0)
}

/// The `mdta` items and the movie duration in seconds (from `mvhd`).
pub fn read_info(path: &Path) -> Result<(Vec<Item>, Option<f64>)> {
    let mut f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let (start, size, hdr) = find_moov(&mut f, path)?;
    if size > 64 << 20 {
        bail!("moov box too large in {}", path.display());
    }
    let mut body = vec![0u8; (size - hdr) as usize];
    f.seek(SeekFrom::Start(start + hdr))?;
    f.read_exact(&mut body)?;
    let mut items = Vec::new();
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
                items = parse_meta(&inner[skip.min(inner.len())..])?;
            }
            b"mvhd" => duration = mvhd_duration(&b[8..]),
            _ => {}
        }
    }
    Ok((items, duration))
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
    let mut items = read(path)?;
    edit(&mut items);
    items.retain(|(_, v)| !v.trim().is_empty());
    let loc = items
        .iter()
        .find(|(k, _)| k == "com.apple.quicktime.location.ISO6709")
        .map(|(_, v)| v.clone());
    write(path, &items, loc.as_deref())?;
    if let Some(t) = mtime {
        let _ = std::fs::File::options()
            .write(true)
            .open(path)
            .and_then(|f| f.set_modified(t));
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
        let once = rebuild_moov(&body, &items, Some("+40.6892-074.0445/")).unwrap();
        let twice = rebuild_moov(&once, &items, Some("+40.6892-074.0445/")).unwrap();
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
}
