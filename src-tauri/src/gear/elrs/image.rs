//! The official ExpressLRS release bundle and the options block of its images (design 6.4).
//!
//! **The bundle.** `index.json` on the ExpressLRS artifactory maps a release tag to a commit;
//! `<commit>/firmware.zip` holds `FCC/<firmware>/` and `LBT/<firmware>/` per unified target
//! (`firmware.bin`, and for an ESP32 also `bootloader.bin`, `partitions.bin`, `boot_app0.bin`)
//! and `hardware/targets.json` with each device (`product_name`, `lua_name`, `layout_file`,
//! `platform`, `firmware`, `prior_target_name`, `min_version`, and an optional `overlay` of
//! layout keys that replace the layout file's) and `hardware/{RX,TX}/<layout>.json`.
//!
//! **The options block.** A unified image carries, after its ESP segments, blocks that name
//! the device and configure it. QuadCam learned their places by comparing a stock image with
//! the same image configured by ExpressLRS's own tools, and writes them itself:
//!
//! | Block | Where | Size | Holds |
//! |---|---|---|---|
//! | Product name | `end` | 128 | NUL-padded text |
//! | Lua name | `end + 128` | 16 | NUL-padded text |
//! | Options | `end + 144` | 512 | JSON: `uid`, `wifi-on-interval`, `flash-discriminator` |
//! | Hardware | `end + 656` | 2048 | The layout file's JSON, the target's `overlay` keys over it |
//! | Trailer | image end | 5 + name | `BE EF CA FE` and the prior target name in capitals, NUL; none for a target without one |
//!
//! The options hold only what QuadCam sets, as ExpressLRS's own configurator writes them: the
//! stock block's build defaults (`lock-on-first-connection`, `domain`) are not carried over.
//!
//! `end` is the end of the last ESP segment, rounded to 16 after the checksum byte (plus 32
//! for an ESP32 image, which holds a 32-byte digest after the checksum). A write reads every
//! block back and checks that no other byte of the image changed. Nothing here was tried on
//! a real device; see `docs/gear.md`.

use crate::gear::firmware::fetch_bytes;
use crate::gear::model::{Refusal, RefusalCode};
use crate::modules::fetch::Fetch;
use crate::modules::install::unpack;
use crate::modules::run::sha256_file;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const ARTIFACTORY: &str = "https://artifactory.expresslrs.org/ExpressLRS";
pub const INDEX_URL: &str = "https://artifactory.expresslrs.org/ExpressLRS/index.json";

const PRODUCT_AT: usize = 0;
const PRODUCT_LEN: usize = 128;
const LUA_AT: usize = 128;
const LUA_LEN: usize = 16;
const OPTIONS_AT: usize = 144;
const OPTIONS_LEN: usize = 512;
const HARDWARE_AT: usize = 656;
const HARDWARE_LEN: usize = 2048;
const TRAILER_MAGIC: [u8; 4] = [0xBE, 0xEF, 0xCA, 0xFE];

fn bad_image(reason: impl Into<String>) -> Refusal {
    Refusal::new(RefusalCode::BadImage, reason)
}

/// The commit the index names for `version`.
pub fn index_commit(index: &[u8], version: &str) -> Result<String> {
    let v: Value = serde_json::from_slice(index).context("The ExpressLRS index is not JSON")?;
    let tags = v["tags"]
        .as_object()
        .ok_or_else(|| anyhow!("The ExpressLRS index has no tags"))?;
    let want = version.trim_start_matches(['v', 'V']);
    let hit = tags
        .iter()
        .find(|(k, _)| k.trim_start_matches(['v', 'V']) == want)
        .and_then(|(_, c)| c.as_str())
        .ok_or_else(|| anyhow!("ExpressLRS {version} is not in the release index."))?;
    if hit.len() < 7 || !hit.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("The index gives a commit that is not a hash: {hit}");
    }
    Ok(hit.to_ascii_lowercase())
}

/// A downloaded release bundle, unpacked.
#[derive(Debug, Clone)]
pub struct Bundle {
    pub version: String,
    pub commit: String,
    pub sha256: String,
    /// True when the person gave this digest and it matched.
    pub pinned: bool,
    /// The folder with `FCC/`, `LBT/` and `hardware/`.
    pub root: PathBuf,
    pub url: String,
}

fn valid_version(v: &str) -> bool {
    !v.is_empty()
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && v.split('.').count() >= 3
        && v.as_bytes()[0].is_ascii_digit()
}

fn find_root(dir: &Path) -> Option<PathBuf> {
    if dir.join("hardware").join("targets.json").is_file() {
        return Some(dir.to_path_buf());
    }
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().is_dir() && e.file_name() != "__MACOSX")
        .find_map(|e| find_root(&e.path()))
}

/// The release bundle for `version`. The cache keeps the zip with its SHA-256; a cached zip
/// that no longer matches is downloaded again. With `expected` (a digest the person got from
/// a trusted place) a mismatch deletes the download and refuses.
pub fn obtain(
    fetch: &dyn Fetch,
    cache: &Path,
    version: &str,
    expected: Option<&str>,
) -> Result<Bundle> {
    if !valid_version(version) {
        bail!("`{version}` is not an ExpressLRS version (such as 4.1.0).");
    }
    let dir = cache.join("firmware").join("elrs").join(version);
    std::fs::create_dir_all(&dir)?;
    let index = fetch_bytes(fetch, INDEX_URL, &dir)?;
    let commit = index_commit(&index, version)?;
    let url = format!("{ARTIFACTORY}/{commit}/firmware.zip");
    let zip = dir.join(format!("{commit}.zip"));
    let record = dir.join(format!("{commit}.zip.sha256"));
    let mut sha = String::new();
    if zip.is_file() {
        if let Ok(want) = std::fs::read_to_string(&record) {
            if sha256_file(&zip)? == want.trim() {
                sha = want.trim().to_string();
            }
        }
        if sha.is_empty() {
            let _ = std::fs::remove_file(&zip);
        }
    }
    if sha.is_empty() {
        fetch.download(&url, &zip)?;
        sha = sha256_file(&zip)?;
        std::fs::write(&record, &sha)?;
    }
    let pinned = match expected
        .map(|e| e.trim().to_ascii_lowercase())
        .filter(|e| !e.is_empty())
    {
        Some(e) if e == sha => true,
        Some(_) => {
            let _ = std::fs::remove_file(&zip);
            let _ = std::fs::remove_file(&record);
            bail!("The download does not match the expected checksum.");
        }
        None => false,
    };
    let unpacked = dir.join(format!("{commit}-unpacked"));
    if find_root(&unpacked).is_none() {
        let _ = std::fs::remove_dir_all(&unpacked);
        std::fs::create_dir_all(&unpacked)?;
        unpack(&zip, &unpacked)?;
    }
    let root = find_root(&unpacked).ok_or_else(|| {
        bad_image("The release has no hardware/targets.json; it is not a bundle.")
    })?;
    Ok(Bundle {
        version: version.into(),
        commit,
        sha256: sha,
        pinned,
        root,
        url,
    })
}

/// Which side of the link a device is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Tx,
    Rx,
}

impl Side {
    pub fn dir(self) -> &'static str {
        match self {
            Side::Tx => "TX",
            Side::Rx => "RX",
        }
    }
}

/// One device of `targets.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// `vendor.group.model` (`radiomaster.tx_2400.pocket`).
    pub path: String,
    pub product_name: String,
    pub lua_name: String,
    pub layout_file: String,
    pub platform: String,
    pub firmware: String,
    pub prior_target_name: String,
    pub min_version: Option<String>,
    pub side: Side,
    /// Layout keys that replace the layout file's (`power_values`, pins).
    pub overlay: Option<serde_json::Map<String, Value>>,
    /// A picture a target with a screen appends after the hardware block.
    pub logo_file: Option<String>,
}

/// Every device in `targets.json` with a layout and a firmware.
pub fn parse_targets(json: &[u8]) -> Result<Vec<Target>> {
    let v: Value = serde_json::from_slice(json).context("targets.json is not JSON")?;
    let mut out = Vec::new();
    let Some(vendors) = v.as_object() else {
        bail!("targets.json is not an object");
    };
    for (vendor, groups) in vendors {
        let Some(groups) = groups.as_object() else {
            continue;
        };
        if vendor == "version" {
            continue;
        }
        for (group, models) in groups {
            let Some(models) = models.as_object() else {
                continue;
            };
            for (model, t) in models {
                let s = |k: &str| t[k].as_str().map(str::to_string);
                let (Some(product), Some(layout), Some(platform), Some(firmware)) = (
                    s("product_name"),
                    s("layout_file"),
                    s("platform"),
                    s("firmware"),
                ) else {
                    continue;
                };
                let side = if firmware.ends_with("_TX") {
                    Side::Tx
                } else if firmware.ends_with("_RX") {
                    Side::Rx
                } else {
                    continue;
                };
                out.push(Target {
                    path: format!("{vendor}.{group}.{model}"),
                    lua_name: s("lua_name").unwrap_or_else(|| product.clone()),
                    product_name: product,
                    layout_file: layout,
                    platform,
                    firmware,
                    prior_target_name: s("prior_target_name").unwrap_or_default(),
                    min_version: s("min_version"),
                    side,
                    overlay: t["overlay"].as_object().cloned(),
                    logo_file: s("logo_file"),
                });
            }
        }
    }
    Ok(out)
}

/// The one device a name (the CRSF device name) and a side select. A name matches a
/// device's Lua name or its product name, ignoring case. None or several refuses.
pub fn find_target(targets: &[Target], name: &str, side: Side) -> Result<Target, Refusal> {
    let n = name.trim().to_ascii_lowercase();
    let hits: Vec<&Target> = targets
        .iter()
        .filter(|t| t.side == side)
        .filter(|t| {
            t.lua_name.to_ascii_lowercase() == n || t.product_name.to_ascii_lowercase() == n
        })
        .collect();
    match hits.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(Refusal::new(
            RefusalCode::UnknownBoard,
            format!("This release has no {} target named `{name}`.", side.dir()),
        )),
        many => Err(Refusal::new(
            RefusalCode::UnknownBoard,
            format!(
                "{} targets share the name `{name}` ({}); QuadCam will not guess.",
                many.len(),
                many.iter()
                    .map(|t| t.path.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

/// The offset of the blocks in an image, or why it is not a unified image.
pub fn blocks_at(raw: &[u8]) -> Result<usize, Refusal> {
    if raw.len() < 0x1010 || raw[0] != 0xE9 {
        return Err(bad_image("The file is not an ESP firmware image."));
    }
    let is8285 = raw[1] == 2;
    let (segments, mut off) = if is8285 {
        (raw[0x1001] as usize, 0x1008usize)
    } else {
        (raw[1] as usize, 24usize)
    };
    for _ in 0..segments {
        let size = raw
            .get(off + 4..off + 8)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
            .ok_or_else(|| bad_image("The image ends inside its segment table."))?;
        off = off
            .checked_add(8 + size)
            .filter(|o| *o <= raw.len())
            .ok_or_else(|| bad_image("A segment of the image runs past its end."))?;
    }
    let mut end = (off + 16) & !15;
    if !is8285 {
        end += 32;
    }
    if end + HARDWARE_AT + HARDWARE_LEN > raw.len() {
        return Err(bad_image("The image has no room for the options block."));
    }
    Ok(end)
}

fn text_at(raw: &[u8], at: usize, len: usize) -> String {
    let b = &raw[at..at + len];
    let end = b.iter().position(|&c| c == 0).unwrap_or(len);
    String::from_utf8_lossy(&b[..end]).to_string()
}

/// What an image's blocks hold.
#[derive(Debug, Clone, PartialEq)]
pub struct Blocks {
    pub product: String,
    pub lua_name: String,
    pub options: Value,
    pub hardware: Value,
    pub trailer: Option<String>,
}

/// Reads the blocks of an image.
pub fn read_blocks(raw: &[u8]) -> Result<Blocks, Refusal> {
    let end = blocks_at(raw)?;
    let json = |at: usize, len: usize| -> Result<Value, Refusal> {
        let t = text_at(raw, end + at, len);
        if t.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&t)
            .map_err(|e| bad_image(format!("A block of the image is not JSON: {e}")))
    };
    let tail = &raw[end + HARDWARE_AT + HARDWARE_LEN..];
    let trailer = tail.windows(4).position(|w| w == TRAILER_MAGIC).map(|i| {
        let name = &tail[i + 4..];
        let n = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        String::from_utf8_lossy(&name[..n]).to_string()
    });
    Ok(Blocks {
        product: text_at(raw, end + PRODUCT_AT, PRODUCT_LEN),
        lua_name: text_at(raw, end + LUA_AT, LUA_LEN),
        options: json(OPTIONS_AT, OPTIONS_LEN)?,
        hardware: json(HARDWARE_AT, HARDWARE_LEN)?,
        trailer,
    })
}

/// JSON the way the ExpressLRS tools write it: `, ` and `: ` separators.
pub fn json_text(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let items: Vec<String> = m
                .iter()
                .map(|(k, v)| format!("{}: {}", Value::String(k.clone()), json_text(v)))
                .collect();
            format!("{{{}}}", items.join(", "))
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(json_text).collect::<Vec<_>>().join(", ")
        ),
        o => o.to_string(),
    }
}

/// What to configure.
#[derive(Debug, Clone)]
pub struct Configure {
    pub product: String,
    pub lua_name: String,
    pub layout: Value,
    pub prior_target_name: String,
    pub uid: [u8; 6],
    /// Seconds without a link before the device starts its WiFi; None leaves it off.
    pub wifi_interval: Option<u32>,
    /// A number that tells two builds apart.
    pub discriminator: u32,
}

fn put(raw: &mut [u8], at: usize, len: usize, text: &str, what: &str) -> Result<(), Refusal> {
    if text.len() >= len {
        return Err(bad_image(format!(
            "The {what} is {} bytes; the image has room for {}.",
            text.len(),
            len - 1
        )));
    }
    raw[at..at + len].fill(0);
    raw[at..at + text.len()].copy_from_slice(text.as_bytes());
    Ok(())
}

/// A stock unified image configured: name, options, hardware layout and trailer. The result is
/// read back, and every other byte must be the stock image's.
pub fn configure(stock: &[u8], c: &Configure) -> Result<Vec<u8>, Refusal> {
    let before = read_blocks(stock)?;
    if before.product != "Unified" {
        return Err(bad_image(format!(
            "The image is `{}`, not a stock unified image.",
            before.product
        )));
    }
    if before.trailer.is_some() {
        return Err(bad_image("The stock image already has a trailer."));
    }
    let end = blocks_at(stock)?;
    let mut raw = stock.to_vec();
    let mut opts = serde_json::Map::new();
    opts.insert(
        "uid".into(),
        Value::from(c.uid.iter().map(|b| *b as u64).collect::<Vec<_>>()),
    );
    if let Some(s) = c.wifi_interval {
        opts.insert("wifi-on-interval".into(), Value::from(s));
    }
    opts.insert("flash-discriminator".into(), Value::from(c.discriminator));
    put(
        &mut raw,
        end + PRODUCT_AT,
        PRODUCT_LEN,
        &c.product,
        "product name",
    )?;
    put(&mut raw, end + LUA_AT, LUA_LEN, &c.lua_name, "Lua name")?;
    put(
        &mut raw,
        end + OPTIONS_AT,
        OPTIONS_LEN,
        &json_text(&Value::Object(opts.clone())),
        "options",
    )?;
    put(
        &mut raw,
        end + HARDWARE_AT,
        HARDWARE_LEN,
        &json_text(&c.layout),
        "hardware layout",
    )?;
    let prior = c.prior_target_name.trim().to_ascii_uppercase();
    if !prior.is_empty() {
        raw.extend_from_slice(&TRAILER_MAGIC);
        raw.extend_from_slice(prior.as_bytes());
        raw.push(0);
    }

    let after = read_blocks(&raw)?;
    let ok = after.product == c.product
        && after.lua_name == c.lua_name
        && after.options == Value::Object(opts)
        && after.hardware == c.layout
        && after.trailer == (!prior.is_empty()).then_some(prior);
    if !ok {
        return Err(bad_image(
            "The configured image does not read back as written.",
        ));
    }
    let mut diff_outside = raw[..end] != stock[..end];
    let after_blocks = end + HARDWARE_AT + HARDWARE_LEN;
    diff_outside |= raw[after_blocks..stock.len()] != stock[after_blocks..];
    if diff_outside {
        return Err(bad_image(
            "Configuring the image changed bytes outside its blocks.",
        ));
    }
    Ok(raw)
}

/// Synthetic stock images, a layout and a `targets.json` for tests and the mock core.
#[doc(hidden)]
pub mod fixtures {
    use super::*;

    /// A stock-like ESP8285 image: a header, two segments, then the blocks, zeroed but for
    /// the product `Unified` and a stock options text.
    pub fn stock_8285(salt: u8) -> Vec<u8> {
        let mut raw = vec![0xFFu8; 0x1008];
        raw[0] = 0xE9;
        raw[1] = 2;
        raw[0x1000] = 0xE9;
        raw[0x1001] = 2;
        for s in 0..2u8 {
            raw.extend_from_slice(&0x4010_0000u32.to_le_bytes());
            raw.extend_from_slice(&64u32.to_le_bytes());
            raw.extend((0..64).map(|i| (i as u8) ^ salt ^ s));
        }
        let end = (raw.len() + 16) & !15;
        stock_blocks(raw, end)
    }

    /// A stock-like ESP32 image: the 24-byte header, five segments (an app image has more
    /// than two; `blocks_at` reads a count of 2 as an ESP8285 image), the 32-byte digest, then
    /// the blocks.
    pub fn stock_esp32(salt: u8) -> Vec<u8> {
        let mut raw = vec![0u8; 24];
        raw[0] = 0xE9;
        raw[1] = 5;
        for s in 0..5u8 {
            raw.extend_from_slice(&0x3F40_0000u32.to_le_bytes());
            raw.extend_from_slice(&1024u32.to_le_bytes());
            raw.extend((0..1024).map(|i| (i as u8) ^ salt ^ s ^ 0x5A));
        }
        let end = ((raw.len() + 16) & !15) + 32;
        stock_blocks(raw, end)
    }

    /// The zeroed blocks at `end`, but for the product `Unified` and a stock options text.
    fn stock_blocks(mut raw: Vec<u8>, end: usize) -> Vec<u8> {
        raw.resize(end + HARDWARE_AT + HARDWARE_LEN, 0);
        raw[end..end + 7].copy_from_slice(b"Unified");
        let o = br#"{"flash-discriminator": 7, "wifi-on-interval": 60, "lock-on-first-connection": true, "domain": 0}"#;
        raw[end + OPTIONS_AT..end + OPTIONS_AT + o.len()].copy_from_slice(o);
        raw
    }

    pub fn layout() -> Value {
        serde_json::json!({"serial_rx": 3, "serial_tx": 1, "radio_busy": 5, "power_values": [13], "led": 16})
    }

    pub fn targets_json() -> Vec<u8> {
        serde_json::json!({
            "version": 1,
            "vendor": {
                "rx_2400": {
                    "aio": {
                        "product_name": "Vendor 2.4GHz AIO RX",
                        "lua_name": "VND AIO RX",
                        "layout_file": "Generic 2400.json",
                        "upload_methods": ["uart", "wifi", "betaflight"],
                        "min_version": "3.1.0",
                        "platform": "esp8285",
                        "firmware": "Unified_ESP8285_2400_RX",
                        "prior_target_name": "DIY_2400_RX_ESP8285_SX1280"
                    },
                    "twin": {
                        "product_name": "Vendor Twin RX",
                        "lua_name": "VND AIO RX",
                        "layout_file": "Twin.json",
                        "platform": "esp8285",
                        "firmware": "Unified_ESP8285_2400_RX"
                    }
                },
                "tx_2400": {
                    "radio": {
                        "product_name": "Radio Internal 2.4GHz TX",
                        "lua_name": "RM Radio",
                        "layout_file": "Radio.json",
                        "platform": "esp32",
                        "firmware": "Unified_ESP32_2400_TX",
                        "prior_target_name": "Radio_2400_TX",
                        "min_version": "3.0.0"
                    }
                }
            }
        })
        .to_string()
        .into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    fn cfg() -> Configure {
        Configure {
            product: "Vendor 2.4GHz AIO RX".into(),
            lua_name: "VND AIO RX".into(),
            layout: layout(),
            prior_target_name: "DIY_2400_RX_ESP8285_SX1280".into(),
            uid: [1, 2, 3, 4, 5, 6],
            wifi_interval: Some(60),
            discriminator: 99,
        }
    }

    #[test]
    fn the_index_gives_a_commit() {
        let j = br#"{"tags": {"4.1.0": "AbCdEf0123456789abcdef0123456789abcdef01", "4.0.0": "x"}}"#;
        assert_eq!(
            index_commit(j, "4.1.0").unwrap(),
            "abcdef0123456789abcdef0123456789abcdef01"
        );
        assert!(index_commit(j, "9.9.9").is_err());
        assert!(index_commit(j, "4.0.0").is_err(), "not a hash");
        assert!(index_commit(b"{}", "4.1.0").is_err());
    }

    #[test]
    fn targets_are_found_by_name_and_side() {
        let t = parse_targets(&targets_json()).unwrap();
        assert_eq!(t.len(), 3);
        let rx = find_target(&t, "vendor 2.4ghz aio rx", Side::Rx).unwrap();
        assert_eq!(rx.platform, "esp8285");
        assert_eq!(rx.path, "vendor.rx_2400.aio");
        assert_eq!(
            find_target(&t, "RM Radio", Side::Tx).unwrap().firmware,
            "Unified_ESP32_2400_TX"
        );
        assert!(find_target(&t, "RM Radio", Side::Rx).is_err());
        let e = find_target(&t, "VND AIO RX", Side::Rx).unwrap_err();
        assert!(e.reason.contains("share the name"), "{}", e.reason);
        assert_eq!(
            find_target(&t, "nothing", Side::Tx).unwrap_err().code,
            RefusalCode::UnknownBoard
        );
    }

    #[test]
    fn a_stock_image_is_configured_and_read_back() {
        let stock = stock_8285(1);
        let before = read_blocks(&stock).unwrap();
        assert_eq!(before.product, "Unified");
        assert_eq!(before.trailer, None);
        let out = configure(&stock, &cfg()).unwrap();
        assert_eq!(out.len(), stock.len() + 4 + 26 + 1);
        let b = read_blocks(&out).unwrap();
        assert_eq!(b.product, "Vendor 2.4GHz AIO RX");
        assert_eq!(b.lua_name, "VND AIO RX");
        assert_eq!(b.options["uid"], serde_json::json!([1, 2, 3, 4, 5, 6]));
        assert_eq!(b.options["wifi-on-interval"], 60);
        assert_eq!(b.hardware, layout());
        assert_eq!(b.trailer.as_deref(), Some("DIY_2400_RX_ESP8285_SX1280"));
        // Only what QuadCam sets: the stock block's build defaults are not carried over.
        let keys: Vec<&String> = b.options.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["flash-discriminator", "uid", "wifi-on-interval"]);
    }

    #[test]
    fn the_trailer_is_in_capitals_and_absent_without_a_prior_name() {
        let mut c = cfg();
        c.prior_target_name = "RadioMaster_Zorro_2400_TX".into();
        let out = configure(&stock_8285(1), &c).unwrap();
        assert_eq!(
            read_blocks(&out).unwrap().trailer.as_deref(),
            Some("RADIOMASTER_ZORRO_2400_TX")
        );
        c.prior_target_name = String::new();
        let stock = stock_8285(1);
        let out = configure(&stock, &c).unwrap();
        assert_eq!(read_blocks(&out).unwrap().trailer, None);
        assert_eq!(out.len(), stock.len());
    }

    #[test]
    fn a_target_overlay_is_parsed() {
        let mut j: Value = serde_json::from_slice(&targets_json()).unwrap();
        j["vendor"]["rx_2400"]["aio"]["overlay"] =
            serde_json::json!({"power_values": [12], "radio_dcdc": true});
        let t = parse_targets(j.to_string().as_bytes()).unwrap();
        let aio = t.iter().find(|t| t.path == "vendor.rx_2400.aio").unwrap();
        let o = aio.overlay.as_ref().unwrap();
        assert_eq!(o["power_values"], serde_json::json!([12]));
        assert!(t
            .iter()
            .find(|t| t.path == "vendor.rx_2400.twin")
            .unwrap()
            .overlay
            .is_none());
    }

    #[test]
    fn json_is_written_with_the_tools_separators() {
        let v = serde_json::json!({"a": [1, 2], "b": "x"});
        assert_eq!(json_text(&v), r#"{"a": [1, 2], "b": "x"}"#);
    }

    #[test]
    fn a_configured_image_is_not_configured_again() {
        let out = configure(&stock_8285(1), &cfg()).unwrap();
        assert_eq!(
            configure(&out, &cfg()).unwrap_err().code,
            RefusalCode::BadImage
        );
    }

    #[test]
    fn what_does_not_fit_or_is_not_an_image_refuses() {
        let mut c = cfg();
        c.lua_name = "A name longer than sixteen".into();
        assert!(configure(&stock_8285(1), &c).is_err());
        assert!(read_blocks(&[0u8; 100]).is_err());
        let mut junk = stock_8285(1);
        junk[0] = 0x00;
        assert!(read_blocks(&junk).is_err());
        let mut c = cfg();
        c.layout = serde_json::json!({"x": "y".repeat(3000)});
        assert!(configure(&stock_8285(1), &c).is_err());
    }

    /// Against a real unpacked bundle, when `QUADCAM_ELRS_BUNDLE` names one. CI skips it.
    #[test]
    fn a_real_bundle_configures_when_one_is_at_hand() {
        let Some(root) = std::env::var_os("QUADCAM_ELRS_BUNDLE") else {
            return;
        };
        let root = PathBuf::from(root);
        for (fw, name) in [
            ("Unified_ESP8285_2400_RX", "RX"),
            ("Unified_ESP32_2400_TX", "TX"),
        ] {
            let stock = std::fs::read(root.join("FCC").join(fw).join("firmware.bin")).unwrap();
            let blocks = read_blocks(&stock).unwrap();
            assert_eq!(blocks.product, "Unified", "{name}");
            let out = configure(
                &stock,
                &Configure {
                    product: "Test".into(),
                    lua_name: "Test".into(),
                    layout: serde_json::json!({"serial_rx": 3}),
                    prior_target_name: "X".into(),
                    uid: [1, 2, 3, 4, 5, 6],
                    wifi_interval: Some(60),
                    discriminator: 1,
                },
            )
            .unwrap();
            assert_eq!(read_blocks(&out).unwrap().product, "Test");
        }
    }
}
