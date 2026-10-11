//! The pure pieces of an ExpressLRS flash (design 6.4): which targets QuadCam will flash, the
//! files and offsets of one, and the `esptool` argument list. Everything that touches a
//! device or the network is in `core/elrs.rs`.
//!
//! QuadCam flashes only the unified targets it has the layout of: the ESP8285 receivers
//! (through an FC) and the ESP32 transmitter modules (through a radio). Any other platform,
//! and any target the release does not name exactly once, refuses.

use super::image::{self, Bundle, Configure, Side, Target};
use crate::gear::model::{Refusal, RefusalCode};
use crate::modules::run::sha256_file;
use anyhow::Result;
use std::ffi::OsString;
use std::path::Path;

/// A platform QuadCam writes, with the `esptool` chip name and the side it serves.
#[derive(Debug)]
pub struct Platform {
    pub id: &'static str,
    pub chip: &'static str,
    pub side: Side,
    /// The unified firmware folder.
    pub firmware: &'static str,
}

pub const PLATFORMS: &[Platform] = &[
    Platform {
        id: "esp8285",
        chip: "esp8266",
        side: Side::Rx,
        firmware: "Unified_ESP8285_2400_RX",
    },
    Platform {
        id: "esp32",
        chip: "esp32",
        side: Side::Tx,
        firmware: "Unified_ESP32_2400_TX",
    },
];

fn refuse(code: RefusalCode, reason: impl Into<String>) -> Refusal {
    Refusal::new(code, reason)
}

/// QuadCam's own version order for `x.y.z`.
fn triple(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.trim().trim_start_matches(['v', 'V']).split(['.', '-']);
    Some((
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    ))
}

/// The guard: a target on a platform QuadCam writes, a unified firmware it knows, and a
/// version the target supports.
pub fn check_target(t: &Target, version: &str) -> Result<&'static Platform, Refusal> {
    let p = PLATFORMS
        .iter()
        .find(|p| p.id == t.platform && p.side == t.side && p.firmware == t.firmware)
        .ok_or_else(|| {
            refuse(
                RefusalCode::UnknownBoard,
                format!(
                    "QuadCam does not flash {} ({} on {}). It flashes {}.",
                    t.product_name,
                    t.firmware,
                    t.platform,
                    PLATFORMS
                        .iter()
                        .map(|p| p.firmware)
                        .collect::<Vec<_>>()
                        .join(" and ")
                ),
            )
        })?;
    let v = triple(version).ok_or_else(|| {
        refuse(
            RefusalCode::UnknownVersion,
            format!("`{version}` is not a release version."),
        )
    })?;
    if let Some(min) = t.min_version.as_deref().and_then(triple) {
        if v < min {
            return Err(refuse(
                RefusalCode::UnknownVersion,
                format!(
                    "{} needs ExpressLRS {} or newer.",
                    t.product_name,
                    t.min_version.as_deref().unwrap_or("")
                ),
            ));
        }
    }
    if v.0 < 3 {
        return Err(refuse(
            RefusalCode::UnknownVersion,
            "QuadCam flashes ExpressLRS 3 and newer.",
        ));
    }
    Ok(p)
}

/// Whether what a receiver printed as it restarted into its bootloader names `t`: its prior
/// target name, its product name, or its unified firmware (`UNIFIED_ESP8285_2400_RX`). An
/// empty reply, or a bare `UNIFIED` that every unified receiver prints, names nothing.
pub fn bootloader_names(said: &str, t: &Target) -> bool {
    let said = said.trim().to_ascii_uppercase();
    !said.is_empty()
        && [&t.prior_target_name, &t.product_name, &t.firmware]
            .iter()
            .map(|n| n.trim().to_ascii_uppercase())
            .any(|n| !n.is_empty() && said.contains(&n))
}

/// One file `esptool` writes.
#[derive(Debug, Clone)]
pub struct FlashFile {
    pub offset: &'static str,
    pub name: &'static str,
    pub bytes: Vec<u8>,
    pub sha256: String,
}

/// Everything one flash writes.
#[derive(Debug, Clone)]
pub struct Built {
    pub target: Target,
    pub chip: &'static str,
    pub region: String,
    pub files: Vec<FlashFile>,
    /// SHA-256 of the configured `firmware.bin`.
    pub image_sha: String,
}

fn sha(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn read_file(p: &Path, what: &str) -> Result<Vec<u8>, Refusal> {
    std::fs::read(p)
        .map_err(|_| refuse(RefusalCode::BadImage, format!("The release has no {what}.")))
}

/// Takes the target's stock image from the bundle and configures it. `region` is `FCC` or
/// `LBT`.
#[allow(clippy::too_many_arguments)]
pub fn build(
    bundle: &Bundle,
    target: &Target,
    platform: &Platform,
    region: &str,
    uid: [u8; 6],
    wifi_interval: Option<u32>,
    discriminator: u32,
) -> Result<Built, Refusal> {
    if region != "FCC" && region != "LBT" {
        return Err(refuse(RefusalCode::BadSetting, "The region is FCC or LBT."));
    }
    let dir = bundle.root.join(region).join(platform.firmware);
    let stock = read_file(
        &dir.join("firmware.bin"),
        &format!("{region}/{}/firmware.bin", platform.firmware),
    )?;
    let layout_path = bundle
        .root
        .join("hardware")
        .join(target.side.dir())
        .join(&target.layout_file);
    let layout: serde_json::Value = serde_json::from_slice(&read_file(
        &layout_path,
        &format!("hardware layout {}", target.layout_file),
    )?)
    .map_err(|e| {
        refuse(
            RefusalCode::BadImage,
            format!("The hardware layout is not JSON: {e}"),
        )
    })?;
    let firmware = image::configure(
        &stock,
        &Configure {
            product: target.product_name.clone(),
            lua_name: target.lua_name.clone(),
            layout,
            prior_target_name: target.prior_target_name.clone(),
            uid,
            wifi_interval,
            discriminator,
        },
    )?;
    let image_sha = sha(&firmware);
    let mut files = Vec::new();
    let file = |offset, name, bytes: Vec<u8>| FlashFile {
        offset,
        sha256: sha(&bytes),
        name,
        bytes,
    };
    if platform.id == "esp32" {
        for (offset, name) in [
            ("0x1000", "bootloader.bin"),
            ("0x8000", "partitions.bin"),
            ("0xe000", "boot_app0.bin"),
        ] {
            files.push(file(offset, name, read_file(&dir.join(name), name)?));
        }
        files.push(file("0x10000", "firmware.bin", firmware));
    } else {
        files.push(file("0x0000", "firmware.bin", firmware));
    }
    Ok(Built {
        target: target.clone(),
        chip: platform.chip,
        region: region.into(),
        files,
        image_sha,
    })
}

/// The `esptool` arguments for `built`, with the files written under `dir` and the device on
/// `port`. The bootloader is already running (the radio's boot pin, or the receiver's
/// bootloader request), so `esptool` does not reset the chip first.
pub fn esptool_args(built: &Built, port: &str, baud: u32, dir: &Path) -> Vec<OsString> {
    let mut a: Vec<OsString> = Vec::new();
    let mut words = vec![
        "--chip".to_string(),
        built.chip.to_string(),
        "--port".to_string(),
        port.to_string(),
        "--baud".to_string(),
        baud.to_string(),
        "--before".to_string(),
        "no-reset".to_string(),
        "--after".to_string(),
        if built.chip == "esp8266" {
            "soft-reset"
        } else {
            "hard-reset"
        }
        .to_string(),
        "write-flash".to_string(),
    ];
    if built.chip != "esp8266" {
        words.extend(
            [
                "-z",
                "--flash-mode",
                "dio",
                "--flash-freq",
                "40m",
                "--flash-size",
                "detect",
            ]
            .map(String::from),
        );
    }
    a.extend(words.into_iter().map(OsString::from));
    for f in &built.files {
        a.push(OsString::from(f.offset));
        a.push(dir.join(f.name).into_os_string());
    }
    a
}

/// Writes the files of `built` into `dir` (a fresh folder) and checks each one's digest.
pub fn stage_files(built: &Built, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    for f in &built.files {
        let p = dir.join(f.name);
        std::fs::write(&p, &f.bytes)?;
        anyhow::ensure!(
            sha256_file(&p)? == f.sha256,
            "{} changed while it was staged",
            f.name
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gear::elrs::image::fixtures::*;

    fn bundle(dir: &Path) -> Bundle {
        let root = dir.join("firmware");
        let fw = root.join("FCC").join("Unified_ESP8285_2400_RX");
        std::fs::create_dir_all(&fw).unwrap();
        std::fs::write(fw.join("firmware.bin"), stock_8285(3)).unwrap();
        std::fs::create_dir_all(root.join("hardware/RX")).unwrap();
        std::fs::write(
            root.join("hardware/RX/Generic 2400.json"),
            layout().to_string(),
        )
        .unwrap();
        Bundle {
            version: "4.1.0".into(),
            commit: "abc".into(),
            sha256: "00".into(),
            pinned: false,
            root,
            url: String::new(),
        }
    }

    fn rx() -> Target {
        image::parse_targets(&targets_json()).unwrap().remove(0)
    }

    #[test]
    fn only_known_platforms_and_versions_pass() {
        let t = rx();
        assert_eq!(check_target(&t, "4.1.0").unwrap().chip, "esp8266");
        assert_eq!(
            check_target(&t, "3.0.0").unwrap_err().code,
            RefusalCode::UnknownVersion,
            "min 3.1.0"
        );
        assert_eq!(
            check_target(&t, "2.5.1").unwrap_err().code,
            RefusalCode::UnknownVersion
        );
        assert!(check_target(&t, "nonsense").is_err());
        let mut other = t.clone();
        other.platform = "esp32-c3".into();
        assert_eq!(
            check_target(&other, "4.1.0").unwrap_err().code,
            RefusalCode::UnknownBoard
        );
        let mut wrong_side = t.clone();
        wrong_side.side = Side::Tx;
        assert!(check_target(&wrong_side, "4.1.0").is_err());
    }

    #[test]
    fn the_bootloader_reply_must_name_the_planned_target() {
        let t = rx();
        assert!(bootloader_names("DIY_2400_RX_ESP8285_SX1280\n", &t));
        assert!(bootloader_names("vendor 2.4ghz aio rx", &t));
        assert!(bootloader_names("ELRS UNIFIED_ESP8285_2400_RX", &t));
        assert!(!bootloader_names("", &t), "an empty reply");
        assert!(!bootloader_names("  \n", &t));
        assert!(
            !bootloader_names("UNIFIED", &t),
            "any unified receiver says that"
        );
        assert!(!bootloader_names("OTHER_RX", &t));
        // A target with no prior name is not matched by everything.
        let mut bare = t.clone();
        bare.prior_target_name = String::new();
        assert!(!bootloader_names("OTHER_RX", &bare));
    }

    #[test]
    fn a_receiver_image_is_built_and_its_arguments_listed() {
        let d = tempfile::tempdir().unwrap();
        let b = bundle(d.path());
        let t = rx();
        let p = check_target(&t, "4.1.0").unwrap();
        let built = build(&b, &t, p, "FCC", [1, 2, 3, 4, 5, 6], Some(60), 5).unwrap();
        assert_eq!(built.files.len(), 1);
        assert_eq!(built.files[0].offset, "0x0000");
        let blocks = image::read_blocks(&built.files[0].bytes).unwrap();
        assert_eq!(blocks.product, "Vendor 2.4GHz AIO RX");
        let args: Vec<String> = esptool_args(&built, "/dev/x", 420000, Path::new("/w"))
            .iter()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args.join(" "),
            "--chip esp8266 --port /dev/x --baud 420000 --before no-reset --after soft-reset write-flash 0x0000 /w/firmware.bin"
        );
        let out = d.path().join("staged");
        stage_files(&built, &out).unwrap();
        assert!(out.join("firmware.bin").is_file());
        assert!(build(&b, &t, p, "EU", [0; 6], None, 0).is_err());
        assert_eq!(built.image_sha, built.files[0].sha256);
    }

    #[test]
    fn a_bundle_without_the_image_refuses() {
        let d = tempfile::tempdir().unwrap();
        let mut b = bundle(d.path());
        b.root = d.path().join("empty");
        let t = rx();
        let p = check_target(&t, "4.1.0").unwrap();
        assert_eq!(
            build(&b, &t, p, "FCC", [0; 6], None, 0).unwrap_err().code,
            RefusalCode::BadImage
        );
    }
}
