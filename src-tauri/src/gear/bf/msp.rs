//! MSP, read-only: v1 and v2 framing, and the identity messages (design 6.2). MSP never
//! writes in 1.0: this module has no "set" message.
//!
//! Written from the public MSP protocol description:
//!
//! - v1: `$M<` (request) or `$M>` (reply; `$M!` an error), a size byte, a command byte, the
//!   payload, and a checksum: XOR of size, command and payload.
//! - v2: `$X<` / `$X>` / `$X!`, a flag byte, the command (u16 LE), the size (u16 LE), the
//!   payload, and CRC-8/DVB-S2 (polynomial 0xD5) over flag, command, size and payload.
//!
//! Identity needs no CLI and no reboot: `MSP_API_VERSION`, `MSP_FC_VARIANT`,
//! `MSP_FC_VERSION`, `MSP_BOARD_INFO`, `MSP_BUILD_INFO`, `MSP_UID`. The UID is the MCU's
//! unique id, the same value the CLI prints as `mcu_id`; it is hashed into the device id
//! and never stored or shown raw.

use crate::gear::serial::{read_until, SerialLink, Wait};
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::time::Duration;

pub const MSP_API_VERSION: u16 = 1;
pub const MSP_FC_VARIANT: u16 = 2;
pub const MSP_FC_VERSION: u16 = 3;
pub const MSP_BOARD_INFO: u16 = 4;
pub const MSP_BUILD_INFO: u16 = 5;
pub const MSP_UID: u16 = 160;
/// Battery voltage, mAh, RSSI, current.
pub const MSP_ANALOG: u16 = 110;

/// Which framing a frame uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    V1,
    V2,
}

/// Which way a frame goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// `<`: to the FC.
    Request,
    /// `>`: from the FC.
    Reply,
    /// `!`: the FC does not know the command.
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub version: Version,
    pub direction: Direction,
    pub cmd: u16,
    pub payload: Vec<u8>,
}

/// One step of CRC-8/DVB-S2.
pub fn crc8_dvb_s2(mut crc: u8, b: u8) -> u8 {
    crc ^= b;
    for _ in 0..8 {
        crc = if crc & 0x80 != 0 {
            (crc << 1) ^ 0xd5
        } else {
            crc << 1
        };
    }
    crc
}

fn dir_byte(d: Direction) -> u8 {
    match d {
        Direction::Request => b'<',
        Direction::Reply => b'>',
        Direction::Error => b'!',
    }
}

/// Encodes a frame. v1 takes commands below 255 and payloads below 255 bytes.
pub fn encode(f: &Frame) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(f.payload.len() + 9);
    match f.version {
        Version::V1 => {
            if f.cmd >= 255 || f.payload.len() >= 255 {
                bail!(
                    "MSP v1 cannot carry command {} with {} bytes",
                    f.cmd,
                    f.payload.len()
                );
            }
            out.extend_from_slice(&[b'$', b'M', dir_byte(f.direction)]);
            let (size, cmd) = (f.payload.len() as u8, f.cmd as u8);
            out.push(size);
            out.push(cmd);
            out.extend_from_slice(&f.payload);
            out.push(f.payload.iter().fold(size ^ cmd, |c, b| c ^ b));
        }
        Version::V2 => {
            if f.payload.len() > u16::MAX as usize {
                bail!("MSP v2 payload too long");
            }
            out.extend_from_slice(&[b'$', b'X', dir_byte(f.direction)]);
            let start = out.len();
            out.push(0); // flag
            out.extend_from_slice(&f.cmd.to_le_bytes());
            out.extend_from_slice(&(f.payload.len() as u16).to_le_bytes());
            out.extend_from_slice(&f.payload);
            let crc = out[start..].iter().fold(0u8, |c, b| crc8_dvb_s2(c, *b));
            out.push(crc);
        }
    }
    Ok(out)
}

/// A request for `cmd` with no payload, v1 when it fits.
pub fn request(cmd: u16) -> Vec<u8> {
    let version = if cmd < 255 { Version::V1 } else { Version::V2 };
    encode(&Frame {
        version,
        direction: Direction::Request,
        cmd,
        payload: Vec::new(),
    })
    .expect("an empty payload always fits")
}

/// What `decode` found at the start of a buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decoded {
    /// A whole frame and the bytes it took.
    Frame(Frame, usize),
    /// A frame has started; more bytes are needed.
    Incomplete,
    /// These bytes are not a frame (noise, a bad checksum): skip them.
    Skip(usize),
}

/// Decodes the first frame in `buf`.
pub fn decode(buf: &[u8]) -> Decoded {
    let Some(start) = buf.iter().position(|b| *b == b'$') else {
        return if buf.is_empty() {
            Decoded::Incomplete
        } else {
            Decoded::Skip(buf.len())
        };
    };
    if start > 0 {
        return Decoded::Skip(start);
    }
    if buf.len() < 3 {
        return Decoded::Incomplete;
    }
    let version = match buf[1] {
        b'M' => Version::V1,
        b'X' => Version::V2,
        _ => return Decoded::Skip(1),
    };
    let direction = match buf[2] {
        b'<' => Direction::Request,
        b'>' => Direction::Reply,
        b'!' => Direction::Error,
        _ => return Decoded::Skip(1),
    };
    match version {
        Version::V1 => {
            if buf.len() < 5 {
                return Decoded::Incomplete;
            }
            let (size, cmd) = (buf[3] as usize, buf[4]);
            let end = 5 + size;
            if buf.len() < end + 1 {
                return Decoded::Incomplete;
            }
            let payload = &buf[5..end];
            let ck = payload.iter().fold(buf[3] ^ cmd, |c, b| c ^ b);
            if ck != buf[end] {
                return Decoded::Skip(1);
            }
            Decoded::Frame(
                Frame {
                    version,
                    direction,
                    cmd: cmd as u16,
                    payload: payload.to_vec(),
                },
                end + 1,
            )
        }
        Version::V2 => {
            if buf.len() < 8 {
                return Decoded::Incomplete;
            }
            let cmd = u16::from_le_bytes([buf[4], buf[5]]);
            let size = u16::from_le_bytes([buf[6], buf[7]]) as usize;
            let end = 8 + size;
            if buf.len() < end + 1 {
                return Decoded::Incomplete;
            }
            let crc = buf[3..end].iter().fold(0u8, |c, b| crc8_dvb_s2(c, *b));
            if crc != buf[end] {
                return Decoded::Skip(1);
            }
            Decoded::Frame(
                Frame {
                    version,
                    direction,
                    cmd,
                    payload: buf[8..end].to_vec(),
                },
                end + 1,
            )
        }
    }
}

/// Every whole frame in `buf`, and the bytes left over (an unfinished frame).
pub fn decode_all(mut buf: &[u8]) -> (Vec<Frame>, Vec<u8>) {
    let mut out = Vec::new();
    loop {
        match decode(buf) {
            Decoded::Frame(f, n) => {
                out.push(f);
                buf = &buf[n..];
            }
            Decoded::Skip(n) => buf = &buf[n..],
            Decoded::Incomplete => return (out, buf.to_vec()),
        }
    }
}

/// Sends a request and waits for its reply. A `!` reply (unknown command) is an error.
pub fn call(link: &mut dyn SerialLink, cmd: u16, timeout: Duration) -> Result<Vec<u8>> {
    link.write_all(&request(cmd))?;
    let buf = read_until(link, Wait::new(timeout), |b| {
        decode_all(b)
            .0
            .iter()
            .any(|f| f.cmd == cmd && f.direction != Direction::Request)
    })?;
    let f = decode_all(&buf)
        .0
        .into_iter()
        .find(|f| f.cmd == cmd && f.direction != Direction::Request)
        .ok_or_else(|| anyhow!("no MSP reply to command {cmd}"))?;
    if f.direction == Direction::Error {
        bail!("the FC does not answer MSP command {cmd}");
    }
    Ok(f.payload)
}

/// What MSP says about the FC. `uid` is the raw MCU id (hex): it goes into the device id
/// hash and nowhere else.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct MspIdentity {
    /// MSP protocol version, then the API version (`1.47`).
    pub protocol: u8,
    pub api: String,
    /// `BTFL` for Betaflight.
    pub variant: String,
    /// `2025.12.5`, or the version string when the FC sends one.
    pub version: String,
    /// The 4-letter board identifier (`G473`).
    pub board_id: String,
    /// The board name (`BETAFPVG473`), from API 1.37 on.
    pub board_name: Option<String>,
    pub manufacturer_id: Option<String>,
    pub target_name: Option<String>,
    /// Build date, time and git revision.
    pub build: Option<String>,
    #[serde(skip)]
    pub uid: Option<String>,
}

/// A cursor over a payload; every read is checked.
struct Rd<'a>(&'a [u8]);

impl<'a> Rd<'a> {
    fn u8(&mut self) -> Option<u8> {
        let (b, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(*b)
    }
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    fn u16(&mut self) -> Option<u16> {
        let b = self.bytes(2)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }
    /// A string with a length byte in front.
    fn pstr(&mut self) -> Option<String> {
        let n = self.u8()? as usize;
        self.bytes(n).map(ascii)
    }
}

fn ascii(b: &[u8]) -> String {
    String::from_utf8_lossy(b)
        .trim_matches(char::from(0))
        .trim()
        .to_string()
}

/// `MSP_FC_VERSION`: three bytes, and on newer firmware a version string after them.
pub fn parse_fc_version(p: &[u8]) -> Option<String> {
    let mut r = Rd(p);
    let (a, b, c) = (r.u8()?, r.u8()?, r.u8()?);
    if let Some(s) = r
        .pstr()
        .filter(|s| s.starts_with(|c: char| c.is_ascii_digit()))
    {
        return Some(s);
    }
    Some(format!("{a}.{b}.{c}"))
}

/// `MSP_BOARD_INFO`: identifier, hardware revision, board type, capabilities, then the
/// target name, board name and manufacturer id as strings with a length byte. Older
/// firmware stops early; what is there is read.
pub fn parse_board_info(p: &[u8], id: &mut MspIdentity) -> Option<()> {
    let mut r = Rd(p);
    id.board_id = ascii(r.bytes(4)?);
    let _hw_revision = r.u16()?;
    let _board_type = r.u8()?;
    let _capabilities = r.u8()?;
    id.target_name = r.pstr().filter(|s| !s.is_empty());
    id.board_name = r.pstr().filter(|s| !s.is_empty());
    id.manufacturer_id = r.pstr().filter(|s| !s.is_empty());
    Some(())
}

/// `MSP_BUILD_INFO`: 11 bytes of date, 8 of time, then the short git revision.
pub fn parse_build_info(p: &[u8]) -> Option<String> {
    let mut r = Rd(p);
    let date = ascii(r.bytes(11)?);
    let time = ascii(r.bytes(8)?);
    let git = r.bytes(7.min(r.0.len())).map(ascii).unwrap_or_default();
    Some(
        [date, time, git]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// `MSP_UID`: three 32-bit words, printed as the CLI prints `mcu_id`.
pub fn parse_uid(p: &[u8]) -> Option<String> {
    if p.len() < 12 {
        return None;
    }
    Some(
        p[..12]
            .chunks(4)
            .map(|w| format!("{:08x}", u32::from_le_bytes([w[0], w[1], w[2], w[3]])))
            .collect(),
    )
}

/// `MSP_ANALOG`: the battery voltage in volts. Newer firmware adds a 0.01 V value at byte
/// 7; older sends 0.1 V in byte 0 only.
pub fn parse_analog_volts(p: &[u8]) -> Option<f32> {
    if p.len() >= 9 {
        return Some(u16::from_le_bytes([p[7], p[8]]) as f32 / 100.0);
    }
    p.first().map(|b| *b as f32 / 10.0)
}

/// Reads the identity over MSP. API, variant and version are required; the rest is read
/// when the FC answers.
pub fn read_identity(link: &mut dyn SerialLink, timeout: Duration) -> Result<MspIdentity> {
    let mut id = MspIdentity::default();
    let api = call(link, MSP_API_VERSION, timeout)?;
    if api.len() < 3 {
        bail!("short MSP_API_VERSION reply");
    }
    id.protocol = api[0];
    id.api = format!("{}.{}", api[1], api[2]);
    id.variant = ascii(&call(link, MSP_FC_VARIANT, timeout)?);
    id.version = parse_fc_version(&call(link, MSP_FC_VERSION, timeout)?)
        .ok_or_else(|| anyhow!("short MSP_FC_VERSION reply"))?;
    if let Ok(p) = call(link, MSP_BOARD_INFO, timeout) {
        parse_board_info(&p, &mut id);
    }
    if let Ok(p) = call(link, MSP_BUILD_INFO, timeout) {
        id.build = parse_build_info(&p);
    }
    if let Ok(p) = call(link, MSP_UID, timeout) {
        id.uid = parse_uid(&p);
    }
    Ok(id)
}

/// The firmware name for a variant code.
pub fn variant_name(v: &str) -> &str {
    match v {
        "BTFL" => "Betaflight",
        "INAV" => "INAV",
        "EMUF" => "EmuFlight",
        "QUIC" => "QuickSilver",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_round_trip_and_checksum() {
        // The well-known empty MSP_API_VERSION request.
        assert_eq!(request(MSP_API_VERSION), b"$M<\x00\x01\x01");
        let f = Frame {
            version: Version::V1,
            direction: Direction::Reply,
            cmd: MSP_API_VERSION,
            payload: vec![0, 1, 47],
        };
        let bytes = encode(&f).unwrap();
        assert_eq!(bytes, b"$M>\x03\x01\x00\x01\x2f\x2c");
        assert_eq!(decode(&bytes), Decoded::Frame(f, bytes.len()));
        let mut bad = bytes.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert_eq!(decode(&bad), Decoded::Skip(1));
        assert_eq!(decode(&bytes[..4]), Decoded::Incomplete);
    }

    #[test]
    fn v2_crc() {
        // CRC-8/DVB-S2 check value over "123456789" is 0xBC.
        assert_eq!(b"123456789".iter().fold(0, |c, b| crc8_dvb_s2(c, *b)), 0xbc);
        let f = Frame {
            version: Version::V2,
            direction: Direction::Reply,
            cmd: 0x1001,
            payload: vec![1, 2, 3],
        };
        let bytes = encode(&f).unwrap();
        assert_eq!(&bytes[..3], b"$X>");
        assert_eq!(decode(&bytes), Decoded::Frame(f, bytes.len()));
        assert_eq!(&request(0x1001)[..3], b"$X<");
    }

    #[test]
    fn noise_and_several_frames() {
        let a = encode(&Frame {
            version: Version::V1,
            direction: Direction::Reply,
            cmd: 2,
            payload: b"BTFL".to_vec(),
        })
        .unwrap();
        let mut buf = b"\r\n# junk".to_vec();
        buf.extend_from_slice(&a);
        buf.extend_from_slice(&a[..5]);
        let (frames, rest) = decode_all(&buf);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].payload, b"BTFL");
        assert_eq!(rest, a[..5].to_vec());
    }

    #[test]
    fn payloads() {
        assert_eq!(parse_fc_version(&[4, 5, 1]).as_deref(), Some("4.5.1"));
        let mut p = vec![25, 12, 5, 15];
        p.extend_from_slice(b"2025.12.5-alpha");
        assert_eq!(parse_fc_version(&p).as_deref(), Some("2025.12.5-alpha"));
        let mut b = b"G473".to_vec();
        b.extend_from_slice(&[0, 0, 2, 0]);
        b.push(4);
        b.extend_from_slice(b"G473");
        b.push(11);
        b.extend_from_slice(b"BETAFPVG473");
        b.push(4);
        b.extend_from_slice(b"BEFH");
        let mut id = MspIdentity::default();
        parse_board_info(&b, &mut id).unwrap();
        assert_eq!(id.board_id, "G473");
        assert_eq!(id.board_name.as_deref(), Some("BETAFPVG473"));
        assert_eq!(id.manufacturer_id.as_deref(), Some("BEFH"));
        // Short (older firmware): what is there.
        let mut id = MspIdentity::default();
        parse_board_info(&b[..8], &mut id);
        assert_eq!(id.board_id, "G473");
        assert_eq!(id.board_name, None);
        assert_eq!(
            parse_build_info(b"Jun 25 202603:24:50eb2bb5a").as_deref(),
            Some("Jun 25 2026 03:24:50 eb2bb5a")
        );
        assert_eq!(
            parse_analog_volts(&[38, 0, 0, 0, 0, 0, 0, 0x7c, 0x01]),
            Some(3.8)
        );
        assert_eq!(parse_analog_volts(&[38]), Some(3.8));
        assert_eq!(parse_analog_volts(&[]), None);
        let uid = [0x01, 0, 0, 0, 0x02, 0, 0, 0, 0xff, 0xee, 0, 0];
        assert_eq!(parse_uid(&uid).unwrap(), "00000001000000020000eeff");
    }
}
