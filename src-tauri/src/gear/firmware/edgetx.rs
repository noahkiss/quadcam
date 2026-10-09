//! EdgeTX firmware for a radio (design 6.5, 7.5): the release's board binary, its checks, and
//! the splash patch.
//!
//! - The release comes from the EdgeTX GitHub releases (`edgetx-firmware-vX.Y.Z.zip`, one
//!   binary per board). The zip is downloaded on the person's action into
//!   `<cache>/firmware/edgetx/<version>/` and checked against the release's SHA-256 when
//!   GitHub reports one; otherwise the hash is recorded at the first download and shown in
//!   the plan. QuadCam bundles and redistributes nothing.
//! - The binary for a board is the one zip entry whose name, less a `fw-` or `edgetx-`
//!   prefix and the version, is one of the board's names. None, or two, refuses.
//! - The image must be a full image: the bootloader at `0x08000000` and the firmware's own
//!   vector table `app_offset` bytes in. A firmware-only image would overwrite the
//!   bootloader, so it refuses (a refusal is safe; a wrong flash is not).

use super::check::{parse_release, Asset, Ver};
use super::fetch_bytes;
use crate::gear::dfu::FLASH_BASE;
use crate::gear::model::{Refusal, RefusalCode};
use crate::modules::fetch::Fetch;
use crate::modules::install::unpack;
use crate::modules::run::sha256_file;
use anyhow::{anyhow, bail, Result};
use std::path::{Path, PathBuf};

/// Where releases are listed by tag.
pub fn release_url(version: &str) -> String {
    format!("https://api.github.com/repos/EdgeTX/edgetx/releases/tags/v{version}")
}

/// Upstream release assets only.
pub const DOWNLOAD_PREFIX: &str = "https://github.com/EdgeTX/edgetx/releases/download/";

pub fn asset_name(version: &str) -> String {
    format!("edgetx-firmware-v{version}.zip")
}

/// What QuadCam knows of a board's firmware.
#[derive(Debug, Clone, Copy)]
pub struct BoardSpec {
    /// The `board:` value of `radio.yml`.
    pub id: &'static str,
    /// Names the release gives its binary, less prefix and version.
    pub names: &'static [&'static str],
    /// The image size guard, in KB.
    pub min_kb: usize,
    pub max_kb: usize,
    /// Where the firmware's own vector table sits in a full image (the bootloader's size).
    pub app_offset: usize,
}

pub const BOARDS: &[BoardSpec] = &[BoardSpec {
    id: "pocket",
    names: &["pocket", "radiomaster-pocket"],
    min_kb: 400,
    max_kb: 1000,
    app_offset: 0x8000,
}];

pub fn spec(board: &str) -> Option<&'static BoardSpec> {
    let b = board.trim().to_ascii_lowercase();
    BOARDS.iter().find(|s| s.id == b)
}

fn bad_image(reason: impl Into<String>) -> Refusal {
    Refusal::new(RefusalCode::BadImage, reason)
}

/// The entry of `names` that is this board's binary.
pub fn select_binary(names: &[String], spec: &BoardSpec) -> Result<usize, Refusal> {
    let hits: Vec<usize> = names
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            let file = n.rsplit('/').next().unwrap_or(n).to_ascii_lowercase();
            let Some(stem) = file.strip_suffix(".bin") else {
                return false;
            };
            let mut stem = stem.replace('_', "-");
            if let Some((head, tail)) = stem.rsplit_once('-') {
                if Ver::parse(tail).is_some() {
                    stem = head.to_string();
                }
            }
            for p in ["fw-", "firmware-", "edgetx-"] {
                if let Some(rest) = stem.strip_prefix(p) {
                    stem = rest.to_string();
                }
            }
            spec.names.contains(&stem.as_str())
        })
        .map(|(i, _)| i)
        .collect();
    match hits.as_slice() {
        [one] => Ok(*one),
        [] => Err(bad_image(format!(
            "The release has no firmware file for the {} (looked for {}).",
            spec.id,
            spec.names.join(", ")
        ))),
        many => Err(bad_image(format!(
            "{} firmware files in the release match the {}: {}.",
            many.len(),
            spec.id,
            many.iter()
                .map(|i| names[*i].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn word(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4)
        .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
}

fn is_vector_table(b: &[u8], at: usize, lo: u32, hi: u32) -> bool {
    let (Some(sp), Some(reset)) = (word(b, at), word(b, at + 4)) else {
        return false;
    };
    let ram = sp & 0xFFF0_0000 == 0x2000_0000 || sp & 0xFFF0_0000 == 0x1000_0000;
    ram && reset & 1 == 1 && (lo..hi).contains(&(reset & !1))
}

/// The checks every image passes before it is planned: size, and a full image with the
/// bootloader first and the firmware after it.
pub fn check_image(bin: &[u8], spec: &BoardSpec) -> Result<(), Refusal> {
    let kb = bin.len() / 1024;
    if kb < spec.min_kb || kb > spec.max_kb {
        return Err(bad_image(format!(
            "The firmware is {kb} KB; a {} image is {} to {} KB.",
            spec.id, spec.min_kb, spec.max_kb
        )));
    }
    let app = FLASH_BASE + spec.app_offset as u32;
    if !is_vector_table(bin, 0, FLASH_BASE, app) {
        return Err(bad_image(
            "The file does not start with a bootloader's vector table; it is not a full image.",
        ));
    }
    if !is_vector_table(bin, spec.app_offset, app, FLASH_BASE + 0x10_0000) {
        return Err(bad_image(
            "The file has no firmware after the bootloader; it is not a full image.",
        ));
    }
    Ok(())
}

/// Where a release's hash came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashSource {
    /// The `digest` GitHub reports for the asset.
    Release,
    /// Recorded at the first download; the plan shows it.
    FirstDownload,
}

/// A downloaded release.
#[derive(Debug, Clone)]
pub struct Release {
    pub version: String,
    pub zip: PathBuf,
    pub sha256: String,
    pub hash_source: HashSource,
    pub url: String,
}

fn release_dir(cache: &Path, version: &str) -> PathBuf {
    cache.join("firmware").join("edgetx").join(version)
}

fn valid_version(v: &str) -> bool {
    !v.is_empty()
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && Ver::parse(v).is_some()
}

/// The release zip for `version`: from the cache when its recorded hash still matches,
/// else downloaded from upstream.
pub fn obtain(fetch: &dyn Fetch, cache: &Path, version: &str) -> Result<Release> {
    if !valid_version(version) {
        bail!("`{version}` is not an EdgeTX version.");
    }
    let dir = release_dir(cache, version);
    std::fs::create_dir_all(&dir)?;
    let name = asset_name(version);
    let zip = dir.join(&name);
    let record = dir.join(format!("{name}.sha256"));
    let url_file = dir.join(format!("{name}.url"));
    if let (Ok(want), Ok(url), true) = (
        std::fs::read_to_string(&record),
        std::fs::read_to_string(&url_file),
        zip.is_file(),
    ) {
        let (want, source) = want.split_once(' ').unwrap_or((want.as_str(), "first"));
        if sha256_file(&zip)? == want.trim() {
            return Ok(Release {
                version: version.into(),
                zip,
                sha256: want.trim().into(),
                hash_source: if source.trim() == "release" {
                    HashSource::Release
                } else {
                    HashSource::FirstDownload
                },
                url: url.trim().into(),
            });
        }
        let _ = std::fs::remove_file(&zip);
    }
    let rel = parse_release(&fetch_bytes(fetch, &release_url(version), &dir)?)?;
    let asset: &Asset = rel
        .assets
        .iter()
        .find(|a| a.name == name)
        .ok_or_else(|| anyhow!("EdgeTX {version} has no {name} in its release."))?;
    if !asset.url.starts_with(DOWNLOAD_PREFIX) {
        bail!(
            "The release points outside the EdgeTX downloads: {}",
            asset.url
        );
    }
    fetch.download(&asset.url, &zip)?;
    let sha = sha256_file(&zip)?;
    let source = match asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
    {
        Some(want) => {
            if !want.eq_ignore_ascii_case(&sha) {
                let _ = std::fs::remove_file(&zip);
                bail!("The download does not match the expected checksum.");
            }
            HashSource::Release
        }
        None => HashSource::FirstDownload,
    };
    std::fs::write(
        &record,
        format!(
            "{sha} {}",
            if source == HashSource::Release {
                "release"
            } else {
                "first"
            }
        ),
    )?;
    std::fs::write(&url_file, &asset.url)?;
    Ok(Release {
        version: version.into(),
        zip,
        sha256: sha,
        hash_source: source,
        url: asset.url.clone(),
    })
}

/// A board binary taken from a release, checked.
#[derive(Debug, Clone)]
pub struct Binary {
    pub name: String,
    pub bytes: Vec<u8>,
    pub sha256: String,
}

fn bin_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let n = e.file_name().to_string_lossy().to_string();
        if n.starts_with('.') || n == "__MACOSX" {
            continue;
        }
        if p.is_dir() {
            bin_files(&p, out);
        } else if n.to_ascii_lowercase().ends_with(".bin") {
            out.push(p);
        }
    }
}

/// Unpacks the release and returns the board's binary, after the image checks.
pub fn board_binary(rel: &Release, board: &str) -> Result<Binary> {
    let spec = spec(board).ok_or_else(|| {
        Refusal::new(
            RefusalCode::UnknownBoard,
            format!("Board {board} is not proven."),
        )
    })?;
    let dir = rel.zip.parent().unwrap_or(Path::new(".")).join("unpacked");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    unpack(&rel.zip, &dir)?;
    let mut files = Vec::new();
    bin_files(&dir, &mut files);
    let rels: Vec<String> = files
        .iter()
        .map(|p| {
            p.strip_prefix(&dir)
                .unwrap_or(p)
                .to_string_lossy()
                .to_string()
        })
        .collect();
    let i = select_binary(&rels, spec)?;
    let bytes = std::fs::read(&files[i])?;
    check_image(&bytes, spec)?;
    let sha256 = sha256_file(&files[i])?;
    Ok(Binary {
        name: rels[i].rsplit('/').next().unwrap_or(&rels[i]).to_string(),
        bytes,
        sha256,
    })
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use crate::gear::splash;
    use std::process::Command;

    /// A full image: a bootloader vector table, the firmware's at 0x8000, filler and the
    /// splash markers, 600 KB in all. `salt` makes two images differ.
    pub(crate) fn full_image(salt: u8) -> Vec<u8> {
        let mut b = vec![salt; 600 * 1024];
        b[0..4].copy_from_slice(&0x2002_0000u32.to_le_bytes());
        b[4..8].copy_from_slice(&0x0800_0101u32.to_le_bytes());
        b[0x8000..0x8004].copy_from_slice(&0x2001_FFF0u32.to_le_bytes());
        b[0x8004..0x8008].copy_from_slice(&0x0800_8201u32.to_le_bytes());
        let s = splash::tests::synthetic_binary();
        // Filler of `salt` never forms the markers; the synthetic block does.
        let at = 0x20000;
        let start = s.windows(4).position(|w| w == splash::START).unwrap();
        let block = &s[start..start + 4 + 2 + splash::IMAGE_BYTES + 3];
        b[at..at + block.len()].copy_from_slice(block);
        b
    }

    /// A release zip with a pocket binary and a binary of another radio, as `ditto` makes it.
    pub(crate) fn release_zip(
        out_dir: &Path,
        version: &str,
        pocket: &[u8],
        extra: &[(&str, &[u8])],
    ) -> PathBuf {
        let src = out_dir.join(format!("zip-src-{version}"));
        let _ = std::fs::remove_dir_all(&src);
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join(format!("fw-radiomaster-pocket-v{version}.bin")),
            pocket,
        )
        .unwrap();
        std::fs::write(src.join(format!("fw-tx16s-v{version}.bin")), full_image(7)).unwrap();
        for (n, b) in extra {
            std::fs::write(src.join(n), b).unwrap();
        }
        let zip = out_dir.join(asset_name(version));
        let st = Command::new("/usr/bin/ditto")
            .args(["-c", "-k", "--sequesterRsrc"])
            .arg(&src)
            .arg(&zip)
            .status()
            .unwrap();
        assert!(st.success());
        zip
    }

    /// Serves a release (its JSON and the zip) from fixtures.
    pub(crate) fn serve_release(
        f: &super::super::FixtureFetch,
        version: &str,
        zip: &Path,
        digest: bool,
    ) {
        let bytes = std::fs::read(zip).unwrap();
        let url = format!("{DOWNLOAD_PREFIX}v{version}/{}", asset_name(version));
        let mut asset = serde_json::json!({
            "name": asset_name(version), "browser_download_url": url, "size": bytes.len()
        });
        if digest {
            asset["digest"] = format!("sha256:{}", sha256_file(zip).unwrap()).into();
        }
        f.serve(
            &release_url(version),
            serde_json::to_vec(&serde_json::json!({
                "tag_name": format!("v{version}"), "prerelease": false, "draft": false, "assets": [asset]
            }))
            .unwrap(),
        );
        f.serve(&url, bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::gear::firmware::FixtureFetch;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_board_binary_is_the_one_named_for_the_board() {
        let pocket = spec("pocket").unwrap();
        let n = names(&[
            "fw-tx16s-v2.12.4.bin",
            "fw-radiomaster-pocket-v2.12.4.bin",
            "readme.txt",
        ]);
        assert_eq!(select_binary(&n, pocket).unwrap(), 1);
        assert_eq!(
            select_binary(&names(&["dir/pocket-2.12.4.bin"]), pocket).unwrap(),
            0
        );
        assert_eq!(
            select_binary(&names(&["radiomaster_pocket_v2.12.4.bin"]), pocket).unwrap(),
            0
        );
        // Another radio is not a near match: pocket is not "pocket-x".
        let e = select_binary(
            &names(&["fw-pocketx-v2.12.4.bin", "fw-tx16s-v2.12.4.bin"]),
            pocket,
        )
        .unwrap_err();
        assert_eq!(e.code, RefusalCode::BadImage);
        assert!(
            e.reason.contains("no firmware file for the pocket"),
            "{}",
            e.reason
        );
        let e = select_binary(
            &names(&[
                "a/fw-pocket-v2.12.4.bin",
                "b/fw-radiomaster-pocket-v2.12.4.bin",
            ]),
            pocket,
        )
        .unwrap_err();
        assert!(e.reason.starts_with("2 firmware files"), "{}", e.reason);
    }

    #[test]
    fn boards_without_a_spec_have_none() {
        assert!(spec("Pocket").is_some());
        assert!(spec("tx16s").is_none());
    }

    #[test]
    fn image_checks_size_and_the_two_vector_tables() {
        let pocket = spec("pocket").unwrap();
        assert!(check_image(&full_image(0), pocket).is_ok());
        assert!(check_image(&vec![0u8; 100 * 1024], pocket)
            .unwrap_err()
            .reason
            .contains("100 KB"));
        assert!(check_image(&vec![0u8; 2000 * 1024], pocket).is_err());
        // No bootloader table first.
        let mut b = full_image(0);
        b[4..8].copy_from_slice(&0x0800_8201u32.to_le_bytes());
        assert!(check_image(&b, pocket)
            .unwrap_err()
            .reason
            .contains("bootloader"));
        // A firmware-only image: its own table first, nothing after.
        let mut fw_only = full_image(0)[0x8000..].to_vec();
        fw_only.resize(600 * 1024, 0xAA);
        assert!(check_image(&fw_only, pocket).is_err());
        // No firmware table at the offset.
        let mut b = full_image(0);
        b[0x8000..0x8004].copy_from_slice(&0u32.to_le_bytes());
        assert!(check_image(&b, pocket)
            .unwrap_err()
            .reason
            .contains("no firmware"));
    }

    #[test]
    fn a_release_downloads_once_checks_its_hash_and_unpacks_the_board_binary() {
        let d = tempfile::tempdir().unwrap();
        let cache = d.path().join("cache");
        let img = full_image(1);
        let zip = release_zip(d.path(), "2.12.4", &img, &[]);
        let f = FixtureFetch::new();
        serve_release(&f, "2.12.4", &zip, true);
        let rel = obtain(&f, &cache, "2.12.4").unwrap();
        assert_eq!(rel.hash_source, HashSource::Release);
        assert_eq!(rel.sha256, sha256_file(&zip).unwrap());
        let bin = board_binary(&rel, "pocket").unwrap();
        assert_eq!(bin.bytes, img);
        assert_eq!(bin.name, "fw-radiomaster-pocket-v2.12.4.bin");
        assert_eq!(
            bin.sha256,
            sha256_file(&rel.zip.parent().unwrap().join("unpacked").join(&bin.name)).unwrap()
        );
        // The second call needs no network.
        let again = obtain(&f, &cache, "2.12.4").unwrap();
        assert_eq!(again.sha256, rel.sha256);
        assert_eq!(again.hash_source, HashSource::Release);
        assert_eq!(f.count(&release_url("2.12.4")), 1);
        assert_eq!(f.requested.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_hash_that_does_not_match_deletes_the_download() {
        let d = tempfile::tempdir().unwrap();
        let cache = d.path().join("cache");
        let zip = release_zip(d.path(), "2.12.4", &full_image(1), &[]);
        let f = FixtureFetch::new();
        serve_release(&f, "2.12.4", &zip, true);
        // Serve other bytes at the asset URL.
        let url = format!("{DOWNLOAD_PREFIX}v2.12.4/{}", asset_name("2.12.4"));
        f.serve(&url, b"tampered".to_vec());
        let e = obtain(&f, &cache, "2.12.4").unwrap_err();
        assert_eq!(
            e.to_string(),
            "The download does not match the expected checksum."
        );
        assert!(!release_dir(&cache, "2.12.4")
            .join(asset_name("2.12.4"))
            .exists());
    }

    #[test]
    fn without_a_digest_the_first_hash_is_recorded_and_a_changed_cache_downloads_again() {
        let d = tempfile::tempdir().unwrap();
        let cache = d.path().join("cache");
        let zip = release_zip(d.path(), "2.12.4", &full_image(1), &[]);
        let f = FixtureFetch::new();
        serve_release(&f, "2.12.4", &zip, false);
        let rel = obtain(&f, &cache, "2.12.4").unwrap();
        assert_eq!(rel.hash_source, HashSource::FirstDownload);
        std::fs::write(&rel.zip, b"damaged").unwrap();
        let rel2 = obtain(&f, &cache, "2.12.4").unwrap();
        assert_eq!(rel2.sha256, rel.sha256, "fetched again");
        assert_eq!(f.count(&release_url("2.12.4")), 2);
    }

    #[test]
    fn urls_outside_upstream_and_odd_versions_are_refused() {
        let d = tempfile::tempdir().unwrap();
        let f = FixtureFetch::new();
        f.serve(
            &release_url("2.12.4"),
            serde_json::to_vec(&serde_json::json!({"tag_name": "v2.12.4", "assets": [
                {"name": asset_name("2.12.4"), "browser_download_url": "https://evil.example/x.zip", "size": 1}
            ]}))
            .unwrap(),
        );
        let e = obtain(&f, d.path(), "2.12.4").unwrap_err();
        assert!(
            e.to_string().contains("outside the EdgeTX downloads"),
            "{e}"
        );
        assert!(obtain(&f, d.path(), "../x").is_err());
        assert!(obtain(&f, d.path(), "").is_err());
    }

    #[test]
    fn a_board_without_a_spec_refuses_with_unknown_board() {
        let d = tempfile::tempdir().unwrap();
        let zip = release_zip(d.path(), "2.12.4", &full_image(1), &[]);
        let f = FixtureFetch::new();
        serve_release(&f, "2.12.4", &zip, true);
        let rel = obtain(&f, &d.path().join("c"), "2.12.4").unwrap();
        let e = board_binary(&rel, "tx16s").unwrap_err();
        let r = e.downcast_ref::<Refusal>().unwrap();
        assert_eq!(r.code, RefusalCode::UnknownBoard);
    }
}
