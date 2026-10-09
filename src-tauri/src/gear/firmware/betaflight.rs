//! Betaflight firmware for an FC (design 6.5, preview): the board table, the official build
//! service, the Intel HEX image and its checks, and the time a flash takes. Pure over what the
//! caller hands in; `core/bf_flash.rs` plans and runs the flash.
//!
//! - **Source.** The image comes from Betaflight's own build service (`build.betaflight.com`)
//!   at run time, on the person's action: a unified target needs its board's defaults inside
//!   the image, and the service builds that. QuadCam bundles nothing. The service lists no
//!   checksum, so the plan shows the SHA-256 recorded at the first download, and a cached
//!   image whose hash no longer matches its record is downloaded again.
//! - **Unverified against the live service.** The request shapes in `CurlCloud` follow the
//!   service's public API as understood from its documentation. No test reaches it, and a
//!   reply of another shape refuses the plan. The first real run on an FC proves or fixes it.
//! - **Image checks.** The HEX file must have valid checksums and an end record, start at
//!   the start of flash, fit the board's flash and begin with a Cortex-M vector table. A
//!   refusal is safe; a wrong flash is not.

use super::check::{FirmwareState, FirmwareStatus};
use crate::gear::compat::{self, Product};
use crate::gear::dfu::FLASH_BASE;
use crate::gear::model::DeviceKind;
use crate::gear::model::{Refusal, RefusalCode};
use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The official build service.
pub const BUILD_API: &str = "https://build.betaflight.com";

/// A board QuadCam can flash. A board missing here is read-only for flashing, even when
/// `compat` proves its settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// The `board_name` the FC reports, lowercase.
    pub board: &'static str,
    /// The target name the build service takes.
    pub target: &'static str,
    /// The MCU family, for the plan.
    pub mcu: &'static str,
    /// The internal flash the DFU layout must show, in KB.
    pub flash_kb: u32,
}

pub const TARGETS: &[Target] = &[
    Target {
        board: "betafpvg473",
        target: "BETAFPVG473",
        mcu: "STM32G473",
        flash_kb: 512,
    },
    Target {
        board: "betafpvg473_v2",
        target: "BETAFPVG473_V2",
        mcu: "STM32G473",
        flash_kb: 512,
    },
];

pub fn target_for(board: &str) -> Option<&'static Target> {
    let b = board.trim().to_ascii_lowercase();
    TARGETS.iter().find(|t| t.board == b)
}

fn bad_image(reason: impl Into<String>) -> Refusal {
    Refusal::new(RefusalCode::BadImage, reason)
}

/// A release name the build service takes: digits, letters, dots and dashes.
pub fn valid_release(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 32
        && v.starts_with(|c: char| c.is_ascii_digit())
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

/// A version an FC reports (`2026.6.0-alpha`) as a release name (`2026.6.0`).
pub fn release_of(version: &str) -> String {
    version
        .trim()
        .trim_start_matches(['v', 'V'])
        .split('-')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// The Firmware page's FC rows with the preview on: an FC on a board with a target, where
/// `compat` proves the newest release, can be flashed. Any other FC keeps the row the check
/// made, with the reason when an update waits.
pub fn adjust_statuses(rows: &mut [FirmwareStatus], preview: bool) {
    if !preview {
        return;
    }
    for r in rows.iter_mut().filter(|r| r.kind == DeviceKind::Fc) {
        let verdict = match (&r.latest, r.board.as_deref()) {
            (Some(latest), Some(board)) => match target_for(board) {
                None => Err(format!("Board {board} cannot be flashed by QuadCam yet.")),
                Some(_) => compat::check_writable(
                    Product::Betaflight,
                    Some(board),
                    Some(&release_of(latest)),
                )
                .map_err(|e| e.reason),
            },
            (Some(_), None) => Err("Board (none) cannot be flashed by QuadCam yet.".into()),
            (None, _) => Err("The newest release is not known.".into()),
        };
        match verdict {
            Ok(()) => {
                r.flashable = true;
                if r.state == FirmwareState::Update {
                    r.note = None;
                }
            }
            Err(why) => {
                r.flashable = false;
                if r.state == FirmwareState::Update {
                    r.note = Some(format!("QuadCam will not flash it: {why}"));
                }
            }
        }
    }
}

/// A flashable image: the bytes from `base`, gaps filled with 0xFF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub base: u32,
    pub bytes: Vec<u8>,
}

fn hex_byte(s: &str, at: usize) -> Option<u8> {
    u8::from_str_radix(s.get(at..at + 2)?, 16).ok()
}

/// Parses Intel HEX. Refuses a bad checksum, a missing end record, an overlap, a gap over
/// 64 KB or an image over 2 MB.
pub fn parse_hex(text: &str) -> Result<Image, Refusal> {
    const MAX_SPAN: u64 = 2 << 20;
    const MAX_GAP: u32 = 64 << 10;
    let mut upper: u32 = 0;
    let mut chunks: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut ended = false;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let at = n + 1;
        let body = line.strip_prefix(':').ok_or_else(|| {
            bad_image(format!(
                "Line {at} of the HEX file does not start with ':'."
            ))
        })?;
        if ended {
            return Err(bad_image(format!(
                "The HEX file has data after its end record (line {at})."
            )));
        }
        if body.len() < 10 || body.len() % 2 != 0 {
            return Err(bad_image(format!(
                "Line {at} of the HEX file is cut short."
            )));
        }
        let bytes: Option<Vec<u8>> = (0..body.len() / 2).map(|i| hex_byte(body, i * 2)).collect();
        let bytes =
            bytes.ok_or_else(|| bad_image(format!("Line {at} of the HEX file is not hex.")))?;
        let len = bytes[0] as usize;
        if bytes.len() != len + 5 {
            return Err(bad_image(format!(
                "Line {at} of the HEX file has the wrong length."
            )));
        }
        if bytes.iter().fold(0u8, |a, b| a.wrapping_add(*b)) != 0 {
            return Err(bad_image(format!(
                "Line {at} of the HEX file fails its checksum."
            )));
        }
        let addr = u16::from_be_bytes([bytes[1], bytes[2]]) as u32;
        let data = &bytes[4..4 + len];
        match bytes[3] {
            0x00 => chunks.push((upper.wrapping_add(addr), data.to_vec())),
            0x01 => ended = true,
            0x02 if len == 2 => upper = (u16::from_be_bytes([data[0], data[1]]) as u32) << 4,
            0x04 if len == 2 => upper = (u16::from_be_bytes([data[0], data[1]]) as u32) << 16,
            0x03 | 0x05 => {}
            t => {
                return Err(bad_image(format!(
                    "Line {at} of the HEX file has a record type ({t:#04x}) QuadCam does not read."
                )))
            }
        }
    }
    if !ended {
        return Err(bad_image(
            "The HEX file has no end record; it is cut short.",
        ));
    }
    chunks.retain(|(_, d)| !d.is_empty());
    chunks.sort_by_key(|(a, _)| *a);
    let Some((base, _)) = chunks.first().cloned() else {
        return Err(bad_image("The HEX file holds no data."));
    };
    let mut bytes: Vec<u8> = Vec::new();
    for (addr, data) in &chunks {
        let end = (bytes.len() as u64) + base as u64;
        let addr64 = *addr as u64;
        if addr64 < end {
            return Err(bad_image(format!(
                "The HEX file writes address {addr:#010x} twice."
            )));
        }
        let gap = addr64 - end;
        if gap > MAX_GAP as u64 && !bytes.is_empty() {
            return Err(bad_image(format!(
                "The HEX file has a {} KB gap before {addr:#010x}.",
                gap / 1024
            )));
        }
        if addr64 + data.len() as u64 - base as u64 > MAX_SPAN {
            return Err(bad_image("The HEX file spans more than 2 MB."));
        }
        bytes.resize(bytes.len() + gap as usize, 0xFF);
        bytes.extend_from_slice(data);
    }
    Ok(Image { base, bytes })
}

fn word(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4)
        .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
}

/// The checks every image passes before it is planned: it starts where flash starts, fits
/// the board, and begins with a vector table (a stack in RAM, a Thumb reset inside the image).
pub fn check_image(img: &Image, target: &Target) -> Result<(), Refusal> {
    if img.base != FLASH_BASE {
        return Err(bad_image(format!(
            "The image starts at {:#010x}; QuadCam writes only images that start at {FLASH_BASE:#010x}.",
            img.base
        )));
    }
    let kb = img.bytes.len() / 1024;
    if kb < 64 || kb > target.flash_kb as usize {
        return Err(bad_image(format!(
            "The image is {kb} KB; a {} image is 64 to {} KB.",
            target.target, target.flash_kb
        )));
    }
    let (Some(sp), Some(reset)) = (word(&img.bytes, 0), word(&img.bytes, 4)) else {
        return Err(bad_image("The image has no vector table."));
    };
    let end = FLASH_BASE + img.bytes.len() as u32;
    let ram = sp & 0xFFF0_0000 == 0x2000_0000 || sp & 0xFFF0_0000 == 0x1000_0000;
    if !(ram && reset & 1 == 1 && (FLASH_BASE..end).contains(&(reset & !1))) {
        return Err(bad_image(
            "The image does not start with a vector table; it is not a firmware image.",
        ));
    }
    Ok(())
}

/// Seconds a flash of `len` bytes takes, with room: the reboot into the bootloader, two reads
/// of the old firmware, then erase, write and read back over DFU. An estimate for the USB
/// heat check, not a promise.
pub fn flash_seconds(len: usize) -> u32 {
    90 + (len / 8192) as u32
}

/// The heat check: with a battery in, the USB timer must outlast the flash.
pub fn heat_check(battery: bool, remaining_s: Option<u32>, need_s: u32) -> Result<(), Refusal> {
    match (battery, remaining_s) {
        (true, Some(left)) if left < need_s => Err(Refusal::new(
            RefusalCode::UsbHeat,
            format!(
                "The USB timer has {left} s left with the battery in and the flash needs about {need_s} s. Unplug the battery, then plan again."
            ),
        )),
        _ => Ok(()),
    }
}

/// A build the service made.
#[derive(Debug, Clone)]
pub struct Built {
    /// The file name the service gave it.
    pub file: String,
    /// The HEX text.
    pub hex: Vec<u8>,
}

/// The way to the build service. Real downloads are reachable only through here.
pub trait Cloud: Send + Sync {
    /// Builds (or fetches) `target` at `release` and returns its HEX file.
    fn build(&self, target: &str, release: &str) -> Result<Built>;
}

/// An image from the cache or the service, with its hash.
#[derive(Debug, Clone)]
pub struct Obtained {
    pub image: Image,
    pub file: String,
    pub sha256: String,
    /// True when the cache held it and its record still matched.
    pub cached: bool,
}

fn cache_dir(cache: &Path, release: &str) -> PathBuf {
    cache.join("firmware").join("betaflight").join(release)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The image for `target` at `release`: from the cache when its recorded SHA-256 still
/// matches, else from the service. Parses and checks it either way.
pub fn obtain(cloud: &dyn Cloud, cache: &Path, target: &Target, release: &str) -> Result<Obtained> {
    if !valid_release(release) {
        bail!("`{release}` is not a Betaflight release name.");
    }
    let dir = cache_dir(cache, release);
    std::fs::create_dir_all(&dir)?;
    let hex = dir.join(format!("{}.hex", target.target));
    let record = dir.join(format!("{}.hex.sha256", target.target));
    let name = dir.join(format!("{}.hex.name", target.target));
    if let (Ok(want), Ok(bytes), Ok(file)) = (
        std::fs::read_to_string(&record),
        std::fs::read(&hex),
        std::fs::read_to_string(&name),
    ) {
        let sha = sha256_hex(&bytes);
        if sha == want.trim() {
            if let Ok(image) = parse_hex(&String::from_utf8_lossy(&bytes)) {
                return Ok(Obtained {
                    image,
                    file: file.trim().into(),
                    sha256: sha,
                    cached: true,
                });
            }
        }
        let _ = std::fs::remove_file(&hex);
    }
    let built = cloud
        .build(target.target, release)
        .with_context(|| format!("The build of {} {release} failed", target.target))?;
    let image = parse_hex(&String::from_utf8_lossy(&built.hex))?;
    let sha = sha256_hex(&built.hex);
    std::fs::write(&hex, &built.hex)?;
    std::fs::write(&record, &sha)?;
    std::fs::write(&name, &built.file)?;
    Ok(Obtained {
        image,
        file: built.file,
        sha256: sha,
        cached: false,
    })
}

/// The service over `/usr/bin/curl` (HTTPS only; a process started by cargo reaches only
/// this Mac unless `QUADCAM_FETCH=real`).
pub struct CurlCloud {
    pub base: String,
    pub loopback_only: bool,
}

impl CurlCloud {
    pub fn system() -> CurlCloud {
        CurlCloud {
            base: BUILD_API.into(),
            loopback_only: crate::modules::fetch::real_fetch().loopback_only,
        }
    }

    fn call(&self, path: &str, body: Option<&str>) -> Result<Vec<u8>> {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let url = format!("{}{path}", self.base);
        let loopback = crate::modules::fetch::is_loopback(&url);
        if self.loopback_only && !loopback {
            bail!("downloads are off in tests (QUADCAM_FETCH=real turns them on): {url}");
        }
        if !loopback && !url.starts_with("https://") {
            bail!("QuadCam downloads only over HTTPS: {url}");
        }
        let proto = if loopback { "=http" } else { "=https" };
        let mut cmd = Command::new("/usr/bin/curl");
        cmd.args(["--fail", "--silent", "--show-error", "--location"])
            .args(["--proto", proto, "--proto-redir", proto])
            .args(["--connect-timeout", "20", "--max-time", "300"])
            .args([
                "--user-agent",
                concat!("quadcam/", env!("CARGO_PKG_VERSION")),
            ]);
        if body.is_some() {
            cmd.args(["--header", "Content-Type: application/json"])
                .args(["--data-binary", "@-"]);
        }
        cmd.args(["--url", &url])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().context("running curl")?;
        if let Some(mut stdin) = child.stdin.take() {
            if let Some(b) = body {
                stdin.write_all(b.as_bytes())?;
            }
        }
        let out = child.wait_with_output()?;
        if !out.status.success() {
            bail!(
                "The build service failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(out.stdout)
    }
}

impl Cloud for CurlCloud {
    fn build(&self, target: &str, release: &str) -> Result<Built> {
        let request = serde_json::json!({
            "target": target, "release": release, "options": [], "classic": false
        })
        .to_string();
        let started: serde_json::Value =
            serde_json::from_slice(&self.call("/api/builds", Some(&request))?)
                .context("The build service answered with something that is not JSON")?;
        let key = started["key"]
            .as_str()
            .filter(|k| !k.is_empty() && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
            .ok_or_else(|| anyhow!("The build service gave no build key."))?
            .to_string();
        let mut file = format!("betaflight_{release}_{target}.hex");
        for _ in 0..100 {
            let status: serde_json::Value =
                serde_json::from_slice(&self.call(&format!("/api/builds/{key}/status"), None)?)
                    .context("The build status is not JSON")?;
            match status["status"].as_str() {
                Some("success") => {
                    if let Some(f) = status["file"].as_str().filter(|f| f.ends_with(".hex")) {
                        file = f.rsplit('/').next().unwrap_or(f).to_string();
                    }
                    let hex = self.call(&format!("/api/builds/{key}/hex"), None)?;
                    return Ok(Built { file, hex });
                }
                Some("failed") | Some("error") => bail!("The build service could not build it."),
                _ => std::thread::sleep(std::time::Duration::from_secs(3)),
            }
        }
        bail!("The build service did not finish within 5 minutes.")
    }
}

/// A build service in memory, by target and release. For tests.
#[derive(Default)]
pub struct FixtureCloud {
    builds: Mutex<HashMap<(String, String), Built>>,
    /// Every request, as `target release`, in order.
    pub requested: Mutex<Vec<String>>,
}

impl FixtureCloud {
    pub fn new() -> FixtureCloud {
        FixtureCloud::default()
    }

    pub fn serve(&self, target: &str, release: &str, file: &str, hex: Vec<u8>) {
        self.builds.lock().unwrap().insert(
            (target.into(), release.into()),
            Built {
                file: file.into(),
                hex,
            },
        );
    }

    pub fn count(&self) -> usize {
        self.requested.lock().unwrap().len()
    }
}

impl Cloud for FixtureCloud {
    fn build(&self, target: &str, release: &str) -> Result<Built> {
        self.requested
            .lock()
            .unwrap()
            .push(format!("{target} {release}"));
        self.builds
            .lock()
            .unwrap()
            .get(&(target.to_string(), release.to_string()))
            .cloned()
            .ok_or_else(|| anyhow!("The build service has no build of {target} {release}."))
    }
}

/// Intel HEX text for `bytes` at `base`, 16 bytes a line. For tests and fixtures.
pub fn to_hex(base: u32, bytes: &[u8]) -> String {
    fn rec(out: &mut String, addr: u16, kind: u8, data: &[u8]) {
        let mut all = vec![data.len() as u8, (addr >> 8) as u8, addr as u8, kind];
        all.extend_from_slice(data);
        let sum = all.iter().fold(0u8, |a, b| a.wrapping_add(*b));
        all.push(0u8.wrapping_sub(sum));
        out.push(':');
        for b in all {
            out.push_str(&format!("{b:02X}"));
        }
        out.push('\n');
    }
    let mut out = String::new();
    let mut upper: Option<u16> = None;
    for (i, chunk) in bytes.chunks(16).enumerate() {
        let addr = base as u64 + (i * 16) as u64;
        let hi = (addr >> 16) as u16;
        if upper != Some(hi) {
            rec(&mut out, 0, 0x04, &hi.to_be_bytes());
            upper = Some(hi);
        }
        rec(&mut out, addr as u16, 0x00, chunk);
    }
    rec(&mut out, 0, 0x01, &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A firmware-shaped image of `kb` KB: a vector table, then filler.
    fn image(kb: usize, salt: u8) -> Vec<u8> {
        let mut b = vec![salt; kb * 1024];
        b[0..4].copy_from_slice(&0x2002_0000u32.to_le_bytes());
        b[4..8].copy_from_slice(&0x0800_0101u32.to_le_bytes());
        b
    }

    #[test]
    fn hex_round_trips_across_a_segment_boundary() {
        let bytes = image(80, 7);
        let text = to_hex(FLASH_BASE, &bytes);
        let img = parse_hex(&text).unwrap();
        assert_eq!(img.base, FLASH_BASE);
        assert_eq!(img.bytes, bytes);
    }

    #[test]
    fn hex_refuses_damage() {
        let good = to_hex(FLASH_BASE, &image(4, 1));
        let mut lines: Vec<&str> = good.lines().collect();
        lines.pop();
        let no_end = lines.join("\n");
        let e = parse_hex(&no_end).unwrap_err();
        assert!(e.reason.contains("end record"), "{}", e.reason);
        let e = parse_hex("hello").unwrap_err();
        assert!(e.reason.contains("':'"), "{}", e.reason);
        // One changed data digit fails the checksum.
        let mut chars: Vec<char> = good.chars().collect();
        let at = good.find(":10").unwrap() + 12;
        chars[at] = if chars[at] == 'A' { 'B' } else { 'A' };
        let e = parse_hex(&chars.into_iter().collect::<String>()).unwrap_err();
        assert!(e.reason.contains("checksum"), "{}", e.reason);
        // Data after the end record.
        let e = parse_hex(&format!("{good}{good}")).unwrap_err();
        assert!(e.reason.contains("after its end record"), "{}", e.reason);
    }

    #[test]
    fn hex_refuses_overlap_and_big_gaps() {
        let a = to_hex(FLASH_BASE, &image(1, 1));
        let overlap = a.replace(":00000001FF\n", "") + &to_hex(FLASH_BASE + 16, &[1u8; 16]);
        assert!(parse_hex(&overlap).unwrap_err().reason.contains("twice"));
        let gap = a.replace(":00000001FF\n", "") + &to_hex(FLASH_BASE + 200 * 1024, &[1u8; 16]);
        assert!(parse_hex(&gap).unwrap_err().reason.contains("gap"));
    }

    #[test]
    fn image_checks() {
        let t = target_for("BETAFPVG473_V2").unwrap();
        let ok = Image {
            base: FLASH_BASE,
            bytes: image(300, 3),
        };
        assert!(check_image(&ok, t).is_ok());
        let high = Image {
            base: FLASH_BASE + 0x8000,
            ..ok.clone()
        };
        assert!(check_image(&high, t).unwrap_err().reason.contains("start"));
        let small = Image {
            base: FLASH_BASE,
            bytes: image(8, 3),
        };
        assert!(check_image(&small, t).unwrap_err().reason.contains("KB"));
        let big = Image {
            base: FLASH_BASE,
            bytes: image(600, 3),
        };
        assert!(check_image(&big, t).unwrap_err().reason.contains("KB"));
        let mut junk = image(300, 3);
        junk[0..4].copy_from_slice(&0u32.to_le_bytes());
        let junk = Image {
            base: FLASH_BASE,
            bytes: junk,
        };
        assert!(check_image(&junk, t)
            .unwrap_err()
            .reason
            .contains("vector table"));
    }

    #[test]
    fn targets_and_releases() {
        assert_eq!(target_for(" BetaFPVG473 ").unwrap().target, "BETAFPVG473");
        assert!(target_for("STM32F411").is_none());
        assert_eq!(release_of("2026.6.0-alpha"), "2026.6.0");
        assert_eq!(release_of("v2025.12.5"), "2025.12.5");
        assert!(valid_release("2026.6.0"));
        assert!(!valid_release("../x"));
        assert!(!valid_release(""));
        assert!(!valid_release("a.b"));
    }

    fn fc_row(board: &str, installed: &str, latest: &str) -> FirmwareStatus {
        FirmwareStatus {
            device: "fc-1".into(),
            kind: DeviceKind::Fc,
            name: "FC".into(),
            product: "Betaflight".into(),
            board: Some(board.into()),
            installed: Some(installed.into()),
            latest: Some(latest.into()),
            state: FirmwareState::Update,
            flashable: false,
            note: Some("QuadCam checks Betaflight versions. It does not flash them.".into()),
        }
    }

    #[test]
    fn the_page_offers_a_flash_only_with_the_preview_on_and_a_proven_pair() {
        let mut off = vec![fc_row("BETAFPVG473_V2", "2025.12.5", "2026.6.0")];
        adjust_statuses(&mut off, false);
        assert!(!off[0].flashable);
        assert!(off[0].note.as_deref().unwrap().contains("does not flash"));

        let mut on = vec![
            fc_row("BETAFPVG473_V2", "2025.12.5", "2026.6.0"),
            fc_row("BETAFPVG473_V2", "2025.12.5", "2026.7.0"),
            fc_row("STM32F411", "4.5.1", "2026.6.0"),
        ];
        adjust_statuses(&mut on, true);
        assert!(on[0].flashable && on[0].note.is_none());
        assert!(!on[1].flashable);
        assert!(
            on[1].note.as_deref().unwrap().contains("is not proven"),
            "{:?}",
            on[1].note
        );
        assert!(!on[2].flashable);
        assert_eq!(
            on[2].note.as_deref(),
            Some("QuadCam will not flash it: Board STM32F411 cannot be flashed by QuadCam yet.")
        );
    }

    #[test]
    fn heat() {
        assert!(heat_check(false, Some(5), 100).is_ok());
        assert!(heat_check(true, None, 100).is_ok());
        assert!(heat_check(true, Some(600), 100).is_ok());
        let e = heat_check(true, Some(30), 100).unwrap_err();
        assert_eq!(e.code, RefusalCode::UsbHeat);
        assert!(flash_seconds(500 * 1024) > flash_seconds(100 * 1024));
    }

    #[test]
    fn obtain_caches_and_redownloads_a_changed_file() {
        let dir = tempfile::tempdir().unwrap();
        let cloud = FixtureCloud::new();
        let t = target_for("betafpvg473").unwrap();
        cloud.serve(
            "BETAFPVG473",
            "2025.12.5",
            "betaflight_2025.12.5_BETAFPVG473.hex",
            to_hex(FLASH_BASE, &image(300, 4)).into_bytes(),
        );
        let first = obtain(&cloud, dir.path(), t, "2025.12.5").unwrap();
        assert!(!first.cached);
        assert_eq!(first.file, "betaflight_2025.12.5_BETAFPVG473.hex");
        let second = obtain(&cloud, dir.path(), t, "2025.12.5").unwrap();
        assert!(second.cached);
        assert_eq!(second.sha256, first.sha256);
        assert_eq!(cloud.count(), 1);
        // A cached file edited after the record is dropped and fetched again.
        let hex = dir
            .path()
            .join("firmware/betaflight/2025.12.5/BETAFPVG473.hex");
        std::fs::write(&hex, b"tampered").unwrap();
        let third = obtain(&cloud, dir.path(), t, "2025.12.5").unwrap();
        assert!(!third.cached);
        assert_eq!(cloud.count(), 2);
        assert_eq!(third.sha256, first.sha256);
        // An unknown release is an error, and a bad name never reaches the service.
        assert!(obtain(&cloud, dir.path(), t, "2099.1.1").is_err());
        assert!(obtain(&cloud, dir.path(), t, "../x").is_err());
        assert_eq!(cloud.count(), 3);
    }

    #[test]
    fn the_real_service_stays_offline_under_cargo() {
        if std::env::var("QUADCAM_FETCH").as_deref() != Ok("real") {
            let e = CurlCloud::system()
                .build("BETAFPVG473", "2025.12.5")
                .unwrap_err();
            assert!(format!("{e:#}").contains("off in tests"), "{e:#}");
        }
    }
}
