//! The FC's blackbox flash over MSP (design 7.12): summary, chunked read, erase, and the log
//! headers of what was read. Written from the public MSP and blackbox format descriptions.
//!
//! - `MSP_DATAFLASH_SUMMARY` (70): flags (bit 0 ready, bit 1 supported), sector count, total
//!   size, used size; each u32 little-endian.
//! - `MSP_DATAFLASH_READ` (71), MSP v2: the request is the address (u32), the size wanted
//!   (u16) and a compression flag (u8). The reply is the address (u32), the size sent
//!   (u16), a compressed flag (u8) and the data. QuadCam always asks for no compression: a
//!   reply that comes compressed anyway is an error, not something to unpack.
//! - `MSP_DATAFLASH_ERASE` (72): starts the erase and answers at once. The summary reads
//!   not ready until the erase ends, then ready with 0 bytes used.
//!
//! Every function takes an open link and never opens a port, enters the CLI or reboots the
//! FC. Jobs that do (`core/blackbox.rs`) own the port.
//!
//! A log starts with the line `H Product:Blackbox flight data recorder by Nicholas Sherlock`
//! followed by `H name:value` header lines. The flash holds logs back to back. The reader
//! here parses headers only (count, firmware, craft name, date, rates); it decodes no frames.
//! The FC has no clock unless a battery keeps it, so a date of `0000-01-01` means "unknown".

use super::msp::{self, Version};
use crate::gear::serial::PortGone;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::time::{Duration, Instant};

pub const MSP_DATAFLASH_SUMMARY: u16 = 70;
pub const MSP_DATAFLASH_READ: u16 = 71;
pub const MSP_DATAFLASH_ERASE: u16 = 72;

/// Bytes asked per read request (measured 2026-10-07: 4 KB requests gave about 84 KB/s).
pub const CHUNK: u16 = 4096;

/// The measured MSP read rate, bytes per second.
pub const MSP_BYTES_PER_S: f64 = 84_000.0;

/// Tries per chunk before the read fails.
pub const RETRIES: u32 = 3;

/// The first line of every log.
pub const LOG_MARKER: &[u8] = b"H Product:Blackbox flight data recorder by Nicholas Sherlock\n";

/// The first MSP API minor version with the sized, compression-flagged read (`1.40`).
pub const MIN_API_MINOR: u32 = 40;

/// What `MSP_DATAFLASH_SUMMARY` says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct FlashSummary {
    /// The flash can be read and erased now (an erase in progress reads false).
    pub ready: bool,
    /// The board has a flash chip QuadCam can read.
    pub supported: bool,
    pub sectors: u32,
    pub total: u32,
    pub used: u32,
}

/// Parses a summary reply: flags, sectors, total, used.
pub fn parse_summary(p: &[u8]) -> Result<FlashSummary> {
    if p.len() < 13 {
        bail!("short MSP_DATAFLASH_SUMMARY reply ({} bytes)", p.len());
    }
    let u32_at = |i: usize| u32::from_le_bytes([p[i], p[i + 1], p[i + 2], p[i + 3]]);
    Ok(FlashSummary {
        ready: p[0] & 1 != 0,
        supported: p[0] & 2 != 0,
        sectors: u32_at(1),
        total: u32_at(5),
        used: u32_at(9),
    })
}

/// The flash summary.
pub fn summary(link: &mut dyn crate::gear::serial::SerialLink, timeout: Duration) -> Result<FlashSummary> {
    parse_summary(&msp::call(link, MSP_DATAFLASH_SUMMARY, timeout)?)
}

/// The minor number of an MSP API version string (`1.47` is 47).
pub fn api_minor(api: &str) -> Option<u32> {
    api.split('.').nth(1)?.parse().ok()
}

/// The request for `want` bytes at `addr`, compression off.
pub fn read_request(addr: u32, want: u16) -> Vec<u8> {
    let mut p = addr.to_le_bytes().to_vec();
    p.extend_from_slice(&want.to_le_bytes());
    p.push(0);
    p
}

/// One read reply: the bytes at `addr`. An address that is not the one asked for, a
/// compressed reply and a length that does not fit are errors.
pub fn parse_read(p: &[u8], addr: u32) -> Result<&[u8]> {
    if p.len() < 7 {
        bail!("short MSP_DATAFLASH_READ reply ({} bytes)", p.len());
    }
    let got = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
    if got != addr {
        bail!("the FC answered address {got}, not {addr}");
    }
    let len = u16::from_le_bytes([p[4], p[5]]) as usize;
    if p[6] != 0 {
        bail!("the FC compressed a read that asked for no compression");
    }
    let data = &p[7..];
    if data.len() != len {
        bail!("the FC announced {len} bytes and sent {}", data.len());
    }
    Ok(data)
}

/// Why a read stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stopped;

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Stopped before the blackbox was fully read.")
    }
}

impl std::error::Error for Stopped {}

/// Reads the first `used` bytes of the flash. A chunk that fails is asked again up to
/// `RETRIES` times; a port that is gone fails at once. `progress(done)` runs after each
/// chunk and answers false to stop (the read ends with `Stopped`).
pub fn read_used(
    link: &mut dyn crate::gear::serial::SerialLink,
    used: u32,
    timeout: Duration,
    progress: &mut dyn FnMut(u64) -> bool,
) -> Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::with_capacity(used as usize);
    while (out.len() as u32) < used {
        let addr = out.len() as u32;
        let want = (used - addr).min(CHUNK as u32) as u16;
        let mut last: Option<anyhow::Error> = None;
        let mut got: Option<Vec<u8>> = None;
        for _ in 0..RETRIES {
            let attempt = msp::call_with(
                link,
                Version::V2,
                MSP_DATAFLASH_READ,
                &read_request(addr, want),
                timeout,
            )
            .and_then(|p| parse_read(&p, addr).map(<[u8]>::to_vec));
            match attempt {
                Ok(d) if !d.is_empty() => {
                    got = Some(d);
                    break;
                }
                Ok(_) => last = Some(anyhow!("the FC sent no data at address {addr}")),
                Err(e) if e.downcast_ref::<PortGone>().is_some() => return Err(e),
                Err(e) => last = Some(e),
            }
        }
        let Some(d) = got else {
            return Err(last.unwrap_or_else(|| anyhow!("read failed")).context(format!(
                "Reading the blackbox failed at byte {addr} of {used} after {RETRIES} tries."
            )));
        };
        // A reply longer than asked cannot be trusted.
        if d.len() > want as usize {
            bail!("the FC sent {} bytes for a request of {want}", d.len());
        }
        out.extend_from_slice(&d);
        if !progress(out.len() as u64) {
            return Err(Stopped.into());
        }
    }
    Ok(out)
}

/// Starts the erase. Check `wait_erased` before you call the flash empty.
pub fn erase(link: &mut dyn crate::gear::serial::SerialLink, timeout: Duration) -> Result<()> {
    msp::call(link, MSP_DATAFLASH_ERASE, timeout)?;
    Ok(())
}

/// Polls the summary until the flash is ready with nothing used, or `limit` passes.
/// Returns the seconds it took.
pub fn wait_erased(
    link: &mut dyn crate::gear::serial::SerialLink,
    timeout: Duration,
    limit: Duration,
    poll: Duration,
) -> Result<f64> {
    let start = Instant::now();
    loop {
        match summary(link, timeout) {
            Ok(s) if s.ready && s.used == 0 => return Ok(start.elapsed().as_secs_f64()),
            Ok(_) => {}
            // A busy flash chip may skip a poll; the deadline decides.
            Err(e) if e.downcast_ref::<PortGone>().is_some() => return Err(e),
            Err(_) => {}
        }
        if start.elapsed() >= limit {
            bail!(
                "The erase did not finish within {} s; the flash may still be erasing. Keep the quad plugged in and check again.",
                limit.as_secs()
            );
        }
        std::thread::sleep(poll);
    }
}

/// Seconds an MSP read of `bytes` takes at the measured rate.
pub fn read_seconds(bytes: u64) -> f64 {
    bytes as f64 / MSP_BYTES_PER_S
}

/// Seconds to allow for an erase of a flash of `total` bytes. A guess from chip data
/// sheets (a 16 MB NOR chip erases in tens of seconds, at most a few minutes): 4 s per MiB,
/// at least 20 and at most 120.
pub fn erase_seconds(total: u64) -> f64 {
    (total as f64 / (1024.0 * 1024.0) * 4.0).clamp(20.0, 120.0)
}

/// One log in the flash image, from its headers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct LogInfo {
    /// Position in the image, 1-based.
    pub index: u32,
    pub offset: u64,
    pub size: u64,
    /// `H Firmware revision`.
    pub firmware: Option<String>,
    /// `H Craft name`.
    pub craft: Option<String>,
    /// `H Log start datetime`, as the log prints it.
    pub start: Option<String>,
    /// The start is a real date (not the `0000-01-01` an FC without a clock prints).
    pub dated: bool,
    /// `H looptime`, microseconds.
    pub looptime_us: Option<u32>,
    /// Header lines.
    pub headers: u32,
}

/// The offsets where logs start.
fn starts(image: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(p) = find(&image[i..], LOG_MARKER) {
        out.push(i + p);
        i += p + LOG_MARKER.len();
    }
    out
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Parses every log's headers. Bytes before the first marker are no log: `lead` reports them.
pub fn parse_logs(image: &[u8]) -> (Vec<LogInfo>, usize) {
    let s = starts(image);
    let lead = s.first().copied().unwrap_or(image.len());
    let logs = s
        .iter()
        .enumerate()
        .map(|(n, &start)| {
            let end = s.get(n + 1).copied().unwrap_or(image.len());
            parse_one(n as u32 + 1, start, &image[start..end])
        })
        .collect();
    (logs, lead)
}

fn parse_one(index: u32, offset: usize, log: &[u8]) -> LogInfo {
    let mut info = LogInfo {
        index,
        offset: offset as u64,
        size: log.len() as u64,
        firmware: None,
        craft: None,
        start: None,
        dated: false,
        looptime_us: None,
        headers: 0,
    };
    // Header lines are `H ` lines up to the first line that is not one.
    let mut rest = log;
    while rest.starts_with(b"H ") {
        let Some(nl) = rest.iter().position(|b| *b == b'\n') else {
            break;
        };
        let line = String::from_utf8_lossy(&rest[2..nl]).trim_end_matches('\r').to_string();
        rest = &rest[nl + 1..];
        info.headers += 1;
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let v = v.trim().to_string();
        match k {
            "Firmware revision" => info.firmware = Some(v),
            "Craft name" => info.craft = Some(v).filter(|v| !v.is_empty()),
            "Log start datetime" => {
                info.dated = !v.is_empty() && !v.starts_with("0000-01-01");
                info.start = Some(v);
            }
            "looptime" => info.looptime_us = v.parse().ok(),
            _ => {}
        }
    }
    info
}

/// What the checks on a flash image found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ImageCheck {
    pub logs: Vec<LogInfo>,
    /// Problems that stop an erase. Empty: the image checks out.
    pub problems: Vec<String>,
}

/// Checks an image against the used size the flash reported: the size matches, it starts with
/// a log header and each log has a firmware line. An empty flash has no logs and no problem.
pub fn check_image(image: &[u8], used: u64) -> ImageCheck {
    let mut problems = Vec::new();
    if image.len() as u64 != used {
        problems.push(format!(
            "Read {} bytes; the flash reported {used} used.",
            image.len()
        ));
    }
    let (logs, lead) = parse_logs(image);
    if !image.is_empty() {
        if logs.is_empty() {
            problems.push("No log header in the data.".into());
        } else if lead > 0 {
            problems.push(format!("{lead} bytes before the first log header."));
        }
    }
    for l in &logs {
        if l.firmware.is_none() {
            problems.push(format!("Log {} has no firmware line in its header.", l.index));
        }
    }
    ImageCheck { logs, problems }
}

/// A synthetic flash image of logs for tests: each log has a header and `body` filler
/// bytes. `craft` names the aircraft; `date` is the start line.
pub fn synth_image(logs: &[(&str, usize)], date: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, (craft, body)) in logs.iter().enumerate() {
        out.extend_from_slice(LOG_MARKER);
        for h in [
            "H Data version:2".to_string(),
            "H I interval:32".to_string(),
            "H Firmware type:Cleanflight".to_string(),
            "H Firmware revision:Betaflight 2025.12.5 (abcdef0) STM32G473".to_string(),
            format!("H Log start datetime:{date}"),
            format!("H Craft name:{craft}"),
            "H looptime:125".to_string(),
            "H Field I name:loopIteration,time".to_string(),
        ] {
            out.extend_from_slice(h.as_bytes());
            out.push(b'\n');
        }
        // Frame bytes that are never a header: no 'H' at the start of a line.
        out.extend((0..*body).map(|j| b'I' + ((i + j) % 7) as u8));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_and_read_parse() {
        let mut p = vec![3];
        for v in [4096u32, 16_777_216, 1234] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        let s = parse_summary(&p).unwrap();
        assert_eq!((s.ready, s.supported, s.total, s.used), (true, true, 16_777_216, 1234));
        assert!(parse_summary(&p[..12]).is_err());
        p[0] = 2;
        assert!(!parse_summary(&p).unwrap().ready);

        let mut r = 100u32.to_le_bytes().to_vec();
        r.extend_from_slice(&3u16.to_le_bytes());
        r.push(0);
        r.extend_from_slice(b"abc");
        assert_eq!(parse_read(&r, 100).unwrap(), b"abc");
        assert!(parse_read(&r, 101).is_err(), "wrong address");
        r[6] = 1;
        assert!(parse_read(&r, 100).is_err(), "compressed");
        r[6] = 0;
        r[4] = 4;
        assert!(parse_read(&r, 100).is_err(), "length mismatch");
        assert_eq!(read_request(5, 4096), vec![5, 0, 0, 0, 0, 16, 0]);
        assert_eq!(api_minor("1.47"), Some(47));
    }

    #[test]
    fn headers_give_count_firmware_craft_and_date() {
        let img = synth_image(&[("Meteor75", 500), ("Meteor75", 900), ("Air65", 100)], "0000-01-01T00:00:00.000+00:00");
        let c = check_image(&img, img.len() as u64);
        assert!(c.problems.is_empty(), "{:?}", c.problems);
        assert_eq!(c.logs.len(), 3);
        assert_eq!(c.logs[0].offset, 0);
        assert_eq!(c.logs[1].craft.as_deref(), Some("Meteor75"));
        assert_eq!(c.logs[2].craft.as_deref(), Some("Air65"));
        assert!(c.logs[0].firmware.as_deref().unwrap().starts_with("Betaflight 2025.12.5"));
        assert!(!c.logs[0].dated);
        assert_eq!(c.logs[0].looptime_us, Some(125));
        assert_eq!(c.logs.iter().map(|l| l.size).sum::<u64>(), img.len() as u64);
        let dated = synth_image(&[("X", 10)], "2026-10-07T12:00:00.000+00:00");
        assert!(check_image(&dated, dated.len() as u64).logs[0].dated);
    }

    #[test]
    fn a_bad_image_is_named() {
        let img = synth_image(&[("A", 100)], "0000-01-01T00:00:00.000+00:00");
        let short = check_image(&img[..img.len() - 1], img.len() as u64);
        assert!(short.problems[0].starts_with("Read "), "{:?}", short.problems);
        let mut lead = b"junk".to_vec();
        lead.extend_from_slice(&img);
        assert!(check_image(&lead, lead.len() as u64).problems[0].contains("before the first"));
        assert!(check_image(b"no header here", 14).problems[0].contains("No log header"));
        let empty = check_image(&[], 0);
        assert!(empty.problems.is_empty() && empty.logs.is_empty());
    }

    use crate::gear::bf::fake::FakeFc;
    use crate::gear::bf::{cli::BAUD, cli::Timing};
    use crate::gear::serial::Ports;

    const DUMP: &str = include_str!("../../../tests/fixtures/bf/g473-2025.12.5.dump_all.txt");
    const PORT: &str = "/dev/cu.usbmodemFAKE1";

    fn flash(fc: &FakeFc) -> Box<dyn crate::gear::serial::SerialLink> {
        fc.ports(PORT, None).open(PORT, BAUD).unwrap()
    }

    fn image() -> Vec<u8> {
        synth_image(&[("Meteor75", 9000), ("Meteor75", 5000)], "0000-01-01T00:00:00.000+00:00")
    }

    #[test]
    fn reads_only_the_used_bytes_in_chunks() {
        let img = image();
        let fc = FakeFc::new(DUMP).with_dataflash(img.clone(), 16 * 1024 * 1024);
        let mut l = flash(&fc);
        let t = Timing::fast().msp;
        let s = summary(l.as_mut(), t).unwrap();
        assert_eq!((s.ready, s.supported, s.used as usize), (true, true, img.len()));
        let mut seen = Vec::new();
        let got = read_used(l.as_mut(), s.used, t, &mut |d| {
            seen.push(d);
            true
        })
        .unwrap();
        assert_eq!(got, img);
        assert_eq!(fc.dataflash_reads() as usize, img.len().div_ceil(CHUNK as usize));
        assert_eq!(*seen.last().unwrap(), img.len() as u64);
        assert!(seen.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn short_replies_and_transient_errors_are_handled() {
        let img = image();
        let fc = FakeFc::new(DUMP)
            .with_dataflash(img.clone(), 1 << 24)
            .short_reads(1000)
            .fail_reads(2);
        let mut l = flash(&fc);
        let got = read_used(l.as_mut(), img.len() as u32, Timing::fast().msp, &mut |_| true).unwrap();
        assert_eq!(got, img, "short replies continue where they ended; two errors retry");
        // Three errors in a row on one chunk give up and say where.
        let fc = FakeFc::new(DUMP).with_dataflash(img.clone(), 1 << 24).fail_reads(3);
        let mut l = flash(&fc);
        let e = read_used(l.as_mut(), img.len() as u32, Timing::fast().msp, &mut |_| true).unwrap_err();
        assert!(format!("{e:#}").contains("byte 0 of"), "{e:#}");
    }

    #[test]
    fn a_compressed_reply_is_an_error_and_a_stop_stops() {
        let img = image();
        let fc = FakeFc::new(DUMP).with_dataflash(img.clone(), 1 << 24).compress_replies();
        let mut l = flash(&fc);
        let e = read_used(l.as_mut(), img.len() as u32, Timing::fast().msp, &mut |_| true).unwrap_err();
        assert!(format!("{e:#}").contains("compressed"), "{e:#}");
        let fc = FakeFc::new(DUMP).with_dataflash(img.clone(), 1 << 24);
        let mut l = flash(&fc);
        let e = read_used(l.as_mut(), img.len() as u32, Timing::fast().msp, &mut |_| false).unwrap_err();
        assert!(e.downcast_ref::<Stopped>().is_some());
    }

    #[test]
    fn erase_polls_until_ready_and_empty() {
        let fc = FakeFc::new(DUMP).with_dataflash(image(), 1 << 24).with_erase_polls(3);
        let mut l = flash(&fc);
        let t = Timing::fast().msp;
        erase(l.as_mut(), t).unwrap();
        assert!(!summary(l.as_mut(), t).unwrap().ready, "an erase in progress is not ready");
        wait_erased(l.as_mut(), t, Duration::from_secs(2), Duration::from_millis(1)).unwrap();
        let s = summary(l.as_mut(), t).unwrap();
        assert_eq!((s.ready, s.used), (true, 0));
        assert_eq!(fc.dataflash_erases(), 1);
        // A flash that never finishes fails with a plain message.
        let fc = FakeFc::new(DUMP).with_dataflash(image(), 1 << 24).stuck_erase();
        let mut l = flash(&fc);
        erase(l.as_mut(), t).unwrap();
        let e = wait_erased(l.as_mut(), t, Duration::from_millis(30), Duration::from_millis(1)).unwrap_err();
        assert!(format!("{e}").contains("did not finish"), "{e}");
    }

    #[test]
    fn a_board_without_flash_does_not_answer() {
        let fc = FakeFc::new(DUMP);
        let mut l = flash(&fc);
        assert!(summary(l.as_mut(), Timing::fast().msp).is_err());
    }

    #[test]
    fn estimates() {
        // 16 MB at the measured rate: about 200 s, as measured.
        let s = read_seconds(16_777_216);
        assert!((195.0..205.0).contains(&s), "{s}");
        assert_eq!(erase_seconds(1024 * 1024), 20.0);
        assert_eq!(erase_seconds(16 * 1024 * 1024), 64.0);
        assert_eq!(erase_seconds(1024 * 1024 * 1024), 120.0);
    }
}
