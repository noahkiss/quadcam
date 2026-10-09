//! CRSF framing and the device-parameter messages an ExpressLRS device answers (design 6.4).
//!
//! QuadCam learned the frame layout from the public CRSF description and from observing
//! ExpressLRS devices, and copies no ExpressLRS code (ExpressLRS is GPL-3.0; QuadCam is MIT).
//!
//! A frame is `[address] [length] [type] [payload...] [crc]`. The length counts the type, the
//! payload and the crc. The crc is CRC-8 with polynomial `0xD5` over the type and the payload.
//! The device messages (type `0x28` and up) start their payload with the destination and the
//! origin address:
//!
//! | Type | Name | Payload after destination and origin |
//! |---|---|---|
//! | `0x28` | ping | none |
//! | `0x29` | device info | name, serial, hardware id, firmware id, parameter count, protocol |
//! | `0x2B` | parameter entry | field id, chunks left, a chunk of the entry |
//! | `0x2C` | parameter read | field id, chunk number |
//! | `0x2D` | parameter write | field id, the new value |
//!
//! Nothing here was tried on a real device yet. The parsers refuse what they do not
//! understand instead of guessing, and the callers say so in what they show.

use anyhow::{bail, Result};

pub const ADDR_BROADCAST: u8 = 0x00;
pub const ADDR_RADIO: u8 = 0xEA;
pub const ADDR_RX: u8 = 0xEC;
pub const ADDR_TX: u8 = 0xEE;

pub const T_PING: u8 = 0x28;
pub const T_DEVICE_INFO: u8 = 0x29;
pub const T_ENTRY: u8 = 0x2B;
pub const T_READ: u8 = 0x2C;
pub const T_WRITE: u8 = 0x2D;
pub const T_COMMAND: u8 = 0x32;

/// The longest frame CRSF allows, address and length bytes included.
pub const MAX_FRAME: usize = 64;

/// CRC-8, polynomial `0xD5`, start value 0.
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0xD5
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// One frame, parsed: the address byte, the type and the payload (crc checked).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub addr: u8,
    pub kind: u8,
    pub payload: Vec<u8>,
}

/// Builds a frame.
pub fn encode(addr: u8, kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(payload.len() + 4);
    f.push(addr);
    f.push((payload.len() + 2) as u8);
    f.push(kind);
    f.extend_from_slice(payload);
    f.push(crc8(&f[2..]));
    f
}

/// A device message: destination and origin first.
pub fn encode_ext(addr: u8, kind: u8, dest: u8, origin: u8, body: &[u8]) -> Vec<u8> {
    let mut p = vec![dest, origin];
    p.extend_from_slice(body);
    encode(addr, kind, &p)
}

/// Asks every device on the link to describe itself.
pub fn ping() -> Vec<u8> {
    encode_ext(ADDR_BROADCAST, T_PING, ADDR_BROADCAST, ADDR_RADIO, &[])
}

/// Asks `dest` for chunk `chunk` of parameter `field`.
pub fn read_request(dest: u8, field: u8, chunk: u8) -> Vec<u8> {
    encode_ext(dest, T_READ, dest, ADDR_RADIO, &[field, chunk])
}

/// Writes `value` (already encoded) into parameter `field` of `dest`.
pub fn write_request(dest: u8, field: u8, value: &[u8]) -> Vec<u8> {
    let mut body = vec![field];
    body.extend_from_slice(value);
    encode_ext(dest, T_WRITE, dest, ADDR_RADIO, &body)
}

/// The request that makes a receiver restart into its serial bootloader. The ExpressLRS
/// receiver accepts it as a command frame carrying the two letters `b` `l`.
pub fn bootloader_request() -> Vec<u8> {
    encode(ADDR_RX, T_COMMAND, b"bl")
}

/// Cuts frames out of a byte stream. A frame starts with one of the addresses a link uses
/// (broadcast, FC, radio, receiver, transmitter). Bytes that do not start a valid frame are
/// dropped one at a time, so a half frame at the start of a read resynchronises on the next
/// good one.
#[derive(Default)]
pub struct FrameParser {
    buf: Vec<u8>,
}

impl FrameParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// The next whole, crc-correct frame, if the buffer holds one.
    pub fn take(&mut self) -> Option<Frame> {
        loop {
            if self.buf.len() < 4 {
                return None;
            }
            let len = self.buf[1] as usize;
            if !matches!(self.buf[0], 0x00 | 0xC8 | 0xEA | 0xEC | 0xEE)
                || !(2..=MAX_FRAME - 2).contains(&len)
            {
                self.buf.remove(0);
                continue;
            }
            let total = len + 2;
            if self.buf.len() < total {
                return None;
            }
            let body = &self.buf[2..total - 1];
            if crc8(body) != self.buf[total - 1] {
                self.buf.remove(0);
                continue;
            }
            let frame = Frame {
                addr: self.buf[0],
                kind: body[0],
                payload: body[1..].to_vec(),
            };
            self.buf.drain(..total);
            return Some(frame);
        }
    }
}

/// A device message split into its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ext<'a> {
    pub dest: u8,
    pub origin: u8,
    pub body: &'a [u8],
}

pub fn ext(f: &Frame) -> Option<Ext<'_>> {
    if f.kind < T_PING || f.payload.len() < 2 {
        return None;
    }
    Some(Ext {
        dest: f.payload[0],
        origin: f.payload[1],
        body: &f.payload[2..],
    })
}

/// What a device says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The address it answered from: `0xEE` a transmitter module, `0xEC` a receiver.
    pub origin: u8,
    /// For ExpressLRS, the target's display name.
    pub name: String,
    pub serial: u32,
    pub hardware: u32,
    pub firmware: u32,
    /// How many parameters it has (their ids are 1 to this number).
    pub params: u8,
    pub protocol: u8,
}

fn cstr(b: &[u8]) -> Option<(String, &[u8])> {
    let end = b.iter().position(|&c| c == 0)?;
    Some((
        String::from_utf8_lossy(&b[..end]).to_string(),
        &b[end + 1..],
    ))
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Reads a device-info frame.
pub fn parse_device_info(f: &Frame) -> Result<DeviceInfo> {
    let Some(e) = ext(f).filter(|_| f.kind == T_DEVICE_INFO) else {
        bail!("not a device-info frame");
    };
    let Some((name, rest)) = cstr(e.body) else {
        bail!("the device name has no end");
    };
    if rest.len() < 14 {
        bail!("the device info is {} bytes short", 14 - rest.len());
    }
    Ok(DeviceInfo {
        origin: e.origin,
        name,
        serial: be32(&rest[0..4]),
        hardware: be32(&rest[4..8]),
        firmware: be32(&rest[8..12]),
        params: rest[12],
        protocol: rest[13],
    })
}

/// The kinds of a parameter, by the CRSF type number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Uint8,
    Int8,
    Uint16,
    Int16,
    Float,
    Select,
    Text,
    Folder,
    Info,
    Command,
    Other(u8),
}

impl Kind {
    pub fn from_code(c: u8) -> Kind {
        match c {
            0 => Kind::Uint8,
            1 => Kind::Int8,
            2 => Kind::Uint16,
            3 => Kind::Int16,
            8 => Kind::Float,
            9 => Kind::Select,
            10 => Kind::Text,
            11 => Kind::Folder,
            12 => Kind::Info,
            13 => Kind::Command,
            o => Kind::Other(o),
        }
    }
}

/// A parameter's value, as far as QuadCam reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A whole number and its bytes on the wire (1 or 2).
    Number {
        value: i64,
        min: i64,
        max: i64,
        size: u8,
        signed: bool,
    },
    /// A choice: the option texts and the index chosen.
    Select {
        options: Vec<String>,
        index: u8,
    },
    Text(String),
    Info(String),
    Folder,
    Command,
    Unread(u8),
}

/// One parameter of a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub id: u8,
    pub parent: u8,
    pub hidden: bool,
    pub name: String,
    pub value: Value,
}

fn number(b: &[u8], size: usize, signed: bool) -> Option<i64> {
    let w = b.get(..size)?;
    Some(match (size, signed) {
        (1, false) => w[0] as i64,
        (1, true) => w[0] as i8 as i64,
        (2, false) => u16::from_be_bytes([w[0], w[1]]) as i64,
        (2, true) => i16::from_be_bytes([w[0], w[1]]) as i64,
        _ => return None,
    })
}

/// Reads one assembled parameter entry (the chunks joined): parent, type, name, value.
pub fn parse_param(id: u8, data: &[u8]) -> Result<Param> {
    if data.len() < 3 {
        bail!(
            "parameter {id} is {} bytes; it needs at least 3",
            data.len()
        );
    }
    let parent = data[0];
    let hidden = data[1] & 0x80 != 0;
    let kind = Kind::from_code(data[1] & 0x7F);
    let Some((name, rest)) = cstr(&data[2..]) else {
        bail!("parameter {id} has no name end");
    };
    let value = match kind {
        Kind::Uint8 | Kind::Int8 | Kind::Uint16 | Kind::Int16 => {
            let signed = matches!(kind, Kind::Int8 | Kind::Int16);
            let size = if matches!(kind, Kind::Uint8 | Kind::Int8) {
                1
            } else {
                2
            };
            let (Some(value), Some(min), Some(max)) = (
                number(rest, size, signed),
                rest.get(size..).and_then(|r| number(r, size, signed)),
                rest.get(2 * size..).and_then(|r| number(r, size, signed)),
            ) else {
                bail!("parameter {id} ({name}) is too short for its number");
            };
            Value::Number {
                value,
                min,
                max,
                size: size as u8,
                signed,
            }
        }
        Kind::Select => {
            let Some((opts, tail)) = cstr(rest) else {
                bail!("parameter {id} ({name}) has no option list end");
            };
            let Some(&index) = tail.first() else {
                bail!("parameter {id} ({name}) has no chosen option");
            };
            Value::Select {
                options: opts.split(';').map(str::to_string).collect(),
                index,
            }
        }
        Kind::Text => Value::Text(cstr(rest).map(|(s, _)| s).unwrap_or_default()),
        Kind::Info => Value::Info(cstr(rest).map(|(s, _)| s).unwrap_or_default()),
        Kind::Folder => Value::Folder,
        Kind::Command => Value::Command,
        Kind::Float => Value::Unread(8),
        Kind::Other(c) => Value::Unread(c),
    };
    Ok(Param {
        id,
        parent,
        hidden,
        name,
        value,
    })
}

/// The bytes that write `value` into a parameter, or why it cannot be written.
pub fn encode_value(p: &Param, choice: &WriteValue) -> Result<Vec<u8>> {
    match (&p.value, choice) {
        (Value::Select { options, .. }, WriteValue::Index(i)) => {
            if (*i as usize) >= options.len() {
                bail!(
                    "{} has {} options; {i} is out of range",
                    p.name,
                    options.len()
                );
            }
            Ok(vec![*i])
        }
        (
            Value::Number {
                min,
                max,
                size,
                signed,
                ..
            },
            WriteValue::Number(n),
        ) => {
            if n < min || n > max {
                bail!("{} takes {min} to {max}; {n} is out of range", p.name);
            }
            Ok(match (size, signed) {
                (1, _) => vec![*n as u8],
                _ => (*n as i16).to_be_bytes().to_vec(),
            })
        }
        _ => bail!("{} cannot be written that way", p.name),
    }
}

/// What to write into a parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteValue {
    Index(u8),
    Number(i64),
}

/// The first `x.y.z` in a text: digits, a dot, digits, a dot, digits.
pub fn find_version(text: &str) -> Option<String> {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !(b[i - 1].is_ascii_digit() || b[i - 1] == b'.')) {
            let mut j = i;
            let mut dots = 0;
            while j < b.len() && (b[j].is_ascii_digit() || (b[j] == b'.' && dots < 2)) {
                if b[j] == b'.' {
                    dots += 1;
                }
                j += 1;
            }
            let cand = text[i..j].trim_end_matches('.');
            if cand.split('.').count() == 3 && cand.split('.').all(|p| !p.is_empty()) {
                return Some(cand.to_string());
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc8_matches_the_known_bootloader_frame() {
        // The receiver's reboot request is EC 04 32 62 6C 0A.
        assert_eq!(
            bootloader_request(),
            vec![0xEC, 0x04, 0x32, 0x62, 0x6C, 0x0A]
        );
        assert_eq!(crc8(&[]), 0);
    }

    #[test]
    fn a_frame_round_trips_and_resynchronises() {
        let f = write_request(ADDR_TX, 3, &[2]);
        let mut p = FrameParser::new();
        p.push(&[0x13, 0x77, 0x55]);
        p.push(&f);
        let got = p.take().expect("a frame");
        assert_eq!(got.kind, T_WRITE);
        assert_eq!(got.payload, vec![ADDR_TX, ADDR_RADIO, 3, 2]);
        assert!(p.take().is_none());
    }

    #[test]
    fn a_bad_crc_is_dropped() {
        let mut f = ping();
        let n = f.len();
        f[n - 1] ^= 0xFF;
        let mut p = FrameParser::new();
        p.push(&f);
        assert!(p.take().is_none());
        p.push(&ping());
        assert_eq!(p.take().unwrap().kind, T_PING);
    }

    #[test]
    fn device_info_is_read() {
        let mut body = b"Test TX\0".to_vec();
        body.extend_from_slice(b"ELRS");
        body.extend_from_slice(&[0, 0, 0, 0, 0, 4, 1, 0]);
        body.push(17);
        body.push(0);
        let f = encode_ext(ADDR_RADIO, T_DEVICE_INFO, ADDR_RADIO, ADDR_TX, &body);
        let mut p = FrameParser::new();
        p.push(&f);
        let info = parse_device_info(&p.take().unwrap()).unwrap();
        assert_eq!(info.name, "Test TX");
        assert_eq!(info.origin, ADDR_TX);
        assert_eq!(info.serial, 0x454C5253);
        assert_eq!(info.params, 17);
    }

    #[test]
    fn a_short_device_info_is_refused() {
        let f = encode_ext(ADDR_RADIO, T_DEVICE_INFO, ADDR_RADIO, ADDR_TX, b"X\0AB");
        let mut p = FrameParser::new();
        p.push(&f);
        assert!(parse_device_info(&p.take().unwrap()).is_err());
    }

    #[test]
    fn a_select_and_a_number_parse_and_encode() {
        let mut d = vec![0, 9];
        d.extend_from_slice(b"Packet Rate\0");
        d.extend_from_slice(b"50Hz;150Hz;250Hz\0");
        d.extend_from_slice(&[1, 0, 2, 0]);
        d.extend_from_slice(b"\0");
        let p = parse_param(1, &d).unwrap();
        assert_eq!(p.name, "Packet Rate");
        let Value::Select { options, index } = &p.value else {
            panic!("select");
        };
        assert_eq!((options.len(), *index), (3, 1));
        assert_eq!(encode_value(&p, &WriteValue::Index(2)).unwrap(), vec![2]);
        assert!(encode_value(&p, &WriteValue::Index(3)).is_err());

        let mut n = vec![0, 0x80];
        n.extend_from_slice(b"Level\0");
        n.extend_from_slice(&[5, 1, 9]);
        let p = parse_param(2, &n).unwrap();
        assert!(p.hidden);
        assert_eq!(encode_value(&p, &WriteValue::Number(7)).unwrap(), vec![7]);
        assert!(encode_value(&p, &WriteValue::Number(10)).is_err());
        assert!(encode_value(&p, &WriteValue::Index(1)).is_err());
    }

    #[test]
    fn a_truncated_entry_is_refused() {
        assert!(parse_param(1, &[0, 9, b'A']).is_err());
        let mut d = vec![0, 0];
        d.extend_from_slice(b"X\0\x01");
        assert!(parse_param(1, &d).is_err());
    }

    #[test]
    fn versions_are_found_in_text() {
        assert_eq!(
            find_version("ELRS 4.1.0 (e5f1ab)").as_deref(),
            Some("4.1.0")
        );
        assert_eq!(find_version("v3.5.3-RC1").as_deref(), Some("3.5.3"));
        assert_eq!(find_version("RM Pocket 2.4GHz"), None);
        assert_eq!(find_version("1.2"), None);
    }
}
