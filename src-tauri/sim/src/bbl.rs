//! Betaflight blackbox logs (`.bbl`, `.bfl`, a pulled flash image): QuadCam's own decoder,
//! written from the documented format (Betaflight's "Blackbox Internals" and the field
//! definitions every log carries in its header). `blackbox_decode` is GPL and is not used.
//!
//! A flash image holds logs back to back, each starting with `H Product:Blackbox flight data
//! recorder by Nicholas Sherlock`. A log is header lines (`H key:value`), then frames, each
//! led by one type byte:
//!
//! | Byte | Frame |
//! |---|---|
//! | `I` | Main frame, every field against no history (intra) |
//! | `P` | Main frame, each field predicted from the last one or two main frames (inter) |
//! | `S` | Slow frame: flight mode and state flags |
//! | `E` | Event: sync beep, adjustment, resume, disarm, flight mode, log end |
//! | `G`, `H` | GPS and GPS home |
//!
//! Each field has a predictor and an encoding (tables below). The decoder checks that every
//! frame is followed by a frame byte; a frame that is not is corrupt, and decoding picks up
//! again at the next good `I` frame (`DecodeStats.corrupt` counts the skipped stretches).

use std::collections::BTreeMap;

/// The first header line of every log.
pub const LOG_MARKER: &[u8] = b"H Product:Blackbox flight data recorder by Nicholas Sherlock";

// Predictors.
const PRED_ZERO: u8 = 0;
const PRED_PREVIOUS: u8 = 1;
const PRED_STRAIGHT_LINE: u8 = 2;
const PRED_AVERAGE_2: u8 = 3;
const PRED_MINTHROTTLE: u8 = 4;
const PRED_MOTOR_0: u8 = 5;
const PRED_INCREMENT: u8 = 6;
const PRED_HOME_COORD: u8 = 7;
const PRED_1500: u8 = 8;
const PRED_VBATREF: u8 = 9;
const PRED_LAST_MAIN_FRAME_TIME: u8 = 10;
const PRED_MINMOTOR: u8 = 11;

// Encodings.
const ENC_SIGNED_VB: u8 = 0;
const ENC_UNSIGNED_VB: u8 = 1;
const ENC_NEG_14BIT: u8 = 3;
const ENC_TAG8_8SVB: u8 = 6;
const ENC_TAG2_3S32: u8 = 7;
const ENC_TAG8_4S16: u8 = 8;
const ENC_NULL: u8 = 9;

// Events.
const EVENT_SYNC_BEEP: u8 = 0;
const EVENT_INFLIGHT_ADJUSTMENT: u8 = 13;
const EVENT_LOGGING_RESUME: u8 = 14;
const EVENT_DISARM: u8 = 15;
const EVENT_FLIGHT_MODE: u8 = 30;
const EVENT_LOG_END: u8 = 255;

/// A gap between two main frames longer than this is corruption, not flight (µs).
const MAX_TIME_JUMP_US: i64 = 10_000_000;

/// One frame type's field definitions.
#[derive(Debug, Clone, Default)]
struct FrameDef {
    names: Vec<String>,
    signed: Vec<bool>,
    predictor: Vec<u8>,
    encoding: Vec<u8>,
}

impl FrameDef {
    fn complete(&self) -> bool {
        let n = self.names.len();
        n > 0 && self.predictor.len() == n && self.encoding.len() == n
    }
}

/// How the decode went.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecodeStats {
    pub i_frames: usize,
    pub p_frames: usize,
    pub slow_frames: usize,
    pub events: usize,
    /// Corrupt stretches skipped.
    pub corrupt: usize,
    /// The log ended with its log-end event (not cut off by the flash filling up).
    pub ended: bool,
}

/// One decoded log.
#[derive(Debug, Clone, Default)]
pub struct BblLog {
    /// Position in the image, 1-based.
    pub index: usize,
    /// Byte offset of the log in the image.
    pub offset: usize,
    pub headers: BTreeMap<String, String>,
    /// Main-frame field names, as the header names them (`time`, `gyroADC[0]`, ...).
    pub fields: Vec<String>,
    /// Main frames in order, one value per field.
    pub frames: Vec<Vec<i64>>,
    pub stats: DecodeStats,
}

impl BblLog {
    /// The column of a main-frame field.
    pub fn field(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|f| f == name)
    }

    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers.get(key).map(String::as_str)
    }

    /// A header's comma-separated integers (`motorOutput:48,2047`).
    pub fn header_ints(&self, key: &str) -> Vec<i64> {
        self.header(key)
            .map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect())
            .unwrap_or_default()
    }

    /// Seconds from the first main frame to the last.
    pub fn duration(&self) -> f64 {
        let Some(t) = self.field("time") else {
            return 0.0;
        };
        match (self.frames.first(), self.frames.last()) {
            (Some(a), Some(b)) => (b[t] - a[t]) as f64 / 1e6,
            _ => 0.0,
        }
    }
}

/// The offsets where logs start in an image.
pub fn log_starts(image: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + LOG_MARKER.len() <= image.len() {
        match image[i..]
            .windows(LOG_MARKER.len())
            .position(|w| w == LOG_MARKER)
        {
            Some(p) => {
                out.push(i + p);
                i += p + LOG_MARKER.len();
            }
            None => break,
        }
    }
    out
}

/// Decode every log in an image (a flash dump or a single-log file). A log whose header
/// lacks main-frame definitions is reported as an error in its place.
pub fn decode_image(image: &[u8]) -> Vec<Result<BblLog, String>> {
    let starts = log_starts(image);
    starts
        .iter()
        .enumerate()
        .map(|(n, &s)| {
            let end = starts.get(n + 1).copied().unwrap_or(image.len());
            decode_log(&image[s..end]).map(|mut l| {
                l.index = n + 1;
                l.offset = s;
                l
            })
        })
        .collect()
}

/// Decode one log: its header lines, then its frames.
pub fn decode_log(bytes: &[u8]) -> Result<BblLog, String> {
    let mut headers = BTreeMap::new();
    let mut pos = 0;
    while bytes.get(pos) == Some(&b'H') && bytes.get(pos + 1) == Some(&b' ') {
        let Some(nl) = bytes[pos..].iter().position(|b| *b == b'\n') else {
            break;
        };
        let line = String::from_utf8_lossy(&bytes[pos + 2..pos + nl]);
        if let Some((k, v)) = line.trim_end_matches('\r').split_once(':') {
            headers.insert(k.to_string(), v.to_string());
        }
        pos += nl + 1;
    }
    let mut d = Decoder::new(&headers)?;
    d.run(bytes, pos);
    Ok(BblLog {
        index: 1,
        offset: 0,
        fields: d.main_i.names.clone(),
        headers,
        frames: d.frames,
        stats: d.stats,
    })
}

fn def(headers: &BTreeMap<String, String>, kind: char, names_from: char) -> FrameDef {
    let list = |key: &str| -> Option<&String> { headers.get(&format!("Field {kind} {key}")) };
    let names = headers
        .get(&format!("Field {names_from} name"))
        .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();
    let nums = |v: Option<&String>| -> Vec<u8> {
        v.map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect())
            .unwrap_or_default()
    };
    let signed = headers
        .get(&format!("Field {names_from} signed"))
        .map(|v| v.split(',').map(|x| x.trim() == "1").collect())
        .unwrap_or_default();
    FrameDef {
        names,
        signed,
        predictor: nums(list("predictor")),
        encoding: nums(list("encoding")),
    }
}

/// A byte reader over one log's frames.
struct Stream<'a> {
    b: &'a [u8],
    pos: usize,
    eof: bool,
}

impl Stream<'_> {
    fn byte(&mut self) -> u8 {
        match self.b.get(self.pos) {
            Some(&x) => {
                self.pos += 1;
                x
            }
            None => {
                self.eof = true;
                0
            }
        }
    }

    fn unsigned_vb(&mut self) -> u32 {
        let mut v: u32 = 0;
        for i in 0..5 {
            let c = self.byte();
            v |= ((c & 0x7f) as u32) << (7 * i);
            if c < 0x80 {
                return v;
            }
        }
        // Five bytes with the continuation bit still set: corrupt.
        self.eof = true;
        0
    }

    fn signed_vb(&mut self) -> i32 {
        let u = self.unsigned_vb();
        ((u >> 1) as i32) ^ -((u & 1) as i32)
    }
}

fn sign_extend(v: u32, bits: u32) -> i32 {
    let shift = 32 - bits;
    ((v << shift) as i32) >> shift
}

/// TAG2_3S32: three values behind one lead byte whose top two bits pick the width.
fn tag2_3s32(s: &mut Stream) -> [i32; 3] {
    let lead = s.byte() as u32;
    match lead >> 6 {
        0 => [
            sign_extend((lead >> 4) & 3, 2),
            sign_extend((lead >> 2) & 3, 2),
            sign_extend(lead & 3, 2),
        ],
        1 => {
            let b = s.byte() as u32;
            [
                sign_extend(lead & 0x0f, 4),
                sign_extend(b >> 4, 4),
                sign_extend(b & 0x0f, 4),
            ]
        }
        2 => {
            let b = s.byte() as u32;
            let c = s.byte() as u32;
            [
                sign_extend(lead & 0x3f, 6),
                sign_extend(b & 0x3f, 6),
                sign_extend(c & 0x3f, 6),
            ]
        }
        _ => {
            let mut out = [0; 3];
            for (i, o) in out.iter_mut().enumerate() {
                // Little-endian, 1 to 4 bytes.
                let n = ((lead >> (2 * i)) & 3) + 1;
                let mut v: u32 = 0;
                for k in 0..n {
                    v |= (s.byte() as u32) << (8 * k);
                }
                *o = sign_extend(v, 8 * n);
            }
            out
        }
    }
}

/// TAG8_4S16 (data version 2): four values, a 2-bit width each in the lead byte (zero, a
/// nibble, a byte, 16 bits); nibbles pack, so wider values can straddle bytes.
fn tag8_4s16(s: &mut Stream) -> [i32; 4] {
    let mut sel = s.byte() as u32;
    let mut out = [0; 4];
    let mut half = false;
    let mut buf: u32 = 0;
    for o in out.iter_mut() {
        match sel & 3 {
            0 => *o = 0,
            1 => {
                if !half {
                    buf = s.byte() as u32;
                    *o = sign_extend(buf >> 4, 4);
                    half = true;
                } else {
                    *o = sign_extend(buf & 0x0f, 4);
                    half = false;
                }
            }
            2 => {
                if !half {
                    *o = sign_extend(s.byte() as u32, 8);
                } else {
                    let hi = (buf & 0x0f) << 4;
                    buf = s.byte() as u32;
                    *o = sign_extend(hi | (buf >> 4), 8);
                }
            }
            _ => {
                if !half {
                    let a = s.byte() as u32;
                    let b = s.byte() as u32;
                    *o = sign_extend((a << 8) | b, 16);
                } else {
                    let a = s.byte() as u32;
                    let b = s.byte() as u32;
                    *o = sign_extend(((buf & 0x0f) << 12) | (a << 4) | (b >> 4), 16);
                    buf = b;
                }
            }
        }
        sel >>= 2;
    }
    out
}

/// TAG8_8SVB: up to eight signed values behind a byte of "present" bits; a group of one
/// is a bare signed value.
fn tag8_8svb(s: &mut Stream, count: usize) -> Vec<i32> {
    if count == 1 {
        return vec![s.signed_vb()];
    }
    let mut present = s.byte();
    (0..count)
        .map(|_| {
            let v = if present & 1 != 0 { s.signed_vb() } else { 0 };
            present >>= 1;
            v
        })
        .collect()
}

struct Decoder {
    main_i: FrameDef,
    main_p: FrameDef,
    slow: FrameDef,
    gps: FrameDef,
    home: FrameDef,
    i_interval: i64,
    p_num: i64,
    p_denom: i64,
    minthrottle: i64,
    vbatref: i64,
    minmotor: i64,
    motor0: Option<usize>,
    time_ix: Option<usize>,
    iter_ix: Option<usize>,
    prev: Option<Vec<i64>>,
    prev2: Option<Vec<i64>>,
    home_values: Vec<i64>,
    last_slow: Vec<i64>,
    last_time: Option<i64>,
    last_iter: Option<i64>,
    frames: Vec<Vec<i64>>,
    stats: DecodeStats,
}

/// Whether the logger wrote a main frame at loop iteration `k`.
fn should_have_frame(k: i64, i_interval: i64, p_num: i64, p_denom: i64) -> bool {
    (k % i_interval + p_num - 1) % p_denom < p_num
}

impl Decoder {
    fn new(h: &BTreeMap<String, String>) -> Result<Decoder, String> {
        let main_i = def(h, 'I', 'I');
        let mut main_p = def(h, 'P', 'I');
        if !main_i.complete() {
            return Err("the log header has no main-frame field definitions".into());
        }
        if !main_p.complete() {
            main_p = FrameDef::default();
        }
        let int = |k: &str, d: i64| -> i64 {
            h.get(k)
                .and_then(|v| v.split(',').next())
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(d)
        };
        // `P interval` is `num/denom` in old logs and a plain denominator in newer ones.
        let (p_num, p_denom) = match h.get("P interval") {
            Some(v) => match v.split_once('/') {
                Some((a, b)) => (a.trim().parse().unwrap_or(1), b.trim().parse().unwrap_or(1)),
                None => (1, v.trim().parse().unwrap_or(1)),
            },
            None => (1, 1),
        };
        let motor0 = main_i.names.iter().position(|n| n == "motor[0]");
        let time_ix = main_i.names.iter().position(|n| n == "time");
        let iter_ix = main_i.names.iter().position(|n| n == "loopIteration");
        Ok(Decoder {
            slow: def(h, 'S', 'S'),
            gps: def(h, 'G', 'G'),
            home: def(h, 'H', 'H'),
            i_interval: int("I interval", 32).max(1),
            p_num: p_num.max(1),
            p_denom: p_denom.max(1),
            minthrottle: int("minthrottle", 1150),
            vbatref: int("vbatref", 4095),
            minmotor: int("motorOutput", 0),
            motor0,
            time_ix,
            iter_ix,
            main_i,
            main_p,
            prev: None,
            prev2: None,
            home_values: Vec::new(),
            last_slow: Vec::new(),
            last_time: None,
            last_iter: None,
            frames: Vec::new(),
            stats: DecodeStats::default(),
        })
    }

    fn is_frame_byte(&self, b: Option<&u8>) -> bool {
        match b {
            None => true,
            Some(b'I') | Some(b'P') | Some(b'S') | Some(b'E') => true,
            Some(b'G') => self.gps.complete(),
            Some(b'H') => true,
            _ => false,
        }
    }

    fn run(&mut self, bytes: &[u8], start: usize) {
        let mut s = Stream {
            b: bytes,
            pos: start,
            eof: false,
        };
        // After corruption, main frames wait for the next good I frame.
        let mut synced = false;
        while s.pos < bytes.len() {
            let at = s.pos;
            let kind = s.byte();
            let ok = match kind {
                b'I' => self.main_frame(&mut s, true, &mut synced),
                b'P' if synced && !self.main_p.names.is_empty() => {
                    self.main_frame(&mut s, false, &mut synced)
                }
                b'S' if self.slow.complete() => self.slow_frame(&mut s),
                b'G' if self.gps.complete() => self.gps_frame(&mut s, false),
                b'H' if self.home.complete() => self.gps_frame(&mut s, true),
                b'E' => match self.event(&mut s) {
                    Some(true) => {
                        self.stats.ended = true;
                        return;
                    }
                    Some(false) => true,
                    None => false,
                },
                // A P frame before any good I frame is skipped with its bytes unknown.
                _ => false,
            };
            if !ok {
                self.stats.corrupt += 1;
                synced = false;
                self.prev = None;
                self.prev2 = None;
                // Pick up again at the next I frame.
                s.eof = false;
                s.pos = at + 1;
                match bytes[s.pos..].iter().position(|b| *b == b'I') {
                    Some(p) => s.pos += p,
                    None => return,
                }
            }
        }
    }

    /// Decode a main frame; true when it is good (and kept).
    fn main_frame(&mut self, s: &mut Stream, intra: bool, synced: &mut bool) -> bool {
        let n = self.main_i.names.len();
        let mut cur = vec![0i64; n];
        let skipped = if intra { 0 } else { self.skipped_frames() };
        let (preds, encs) = if intra {
            (self.main_i.predictor.clone(), self.main_i.encoding.clone())
        } else {
            (self.main_p.predictor.clone(), self.main_p.encoding.clone())
        };
        let prev = if intra { None } else { self.prev.clone() };
        let prev2 = if intra { None } else { self.prev2.clone() };
        if !self.read_fields(
            s,
            &preds,
            &encs,
            &self.main_i.signed.clone(),
            &mut cur,
            prev.as_deref(),
            prev2.as_deref(),
            skipped,
        ) {
            return false;
        }
        if s.eof || !self.is_frame_byte(s.b.get(s.pos)) {
            return false;
        }
        if let Some(t) = self.time_ix {
            if let Some(last) = self.last_time {
                let jump = cur[t] - last;
                if !(0..=MAX_TIME_JUMP_US).contains(&jump) {
                    return false;
                }
            }
            self.last_time = Some(cur[t]);
        }
        if let Some(i) = self.iter_ix {
            self.last_iter = Some(cur[i]);
        }
        if intra {
            self.stats.i_frames += 1;
            self.prev2 = Some(cur.clone());
            *synced = true;
        } else {
            self.stats.p_frames += 1;
            self.prev2 = self.prev.take();
        }
        self.prev = Some(cur.clone());
        self.frames.push(cur);
        true
    }

    /// Loop iterations the logger did not log between the last main frame and this one.
    fn skipped_frames(&self) -> i64 {
        let Some(last) = self.last_iter else { return 0 };
        let mut k = last + 1;
        let mut n = 0;
        while !should_have_frame(k, self.i_interval, self.p_num, self.p_denom) && n < 1 << 16 {
            k += 1;
            n += 1;
        }
        n
    }

    #[allow(clippy::too_many_arguments)]
    fn read_fields(
        &self,
        s: &mut Stream,
        preds: &[u8],
        encs: &[u8],
        signed: &[bool],
        cur: &mut [i64],
        prev: Option<&[i64]>,
        prev2: Option<&[i64]>,
        skipped: i64,
    ) -> bool {
        let n = cur.len().min(preds.len()).min(encs.len());
        let mut i = 0;
        while i < n {
            if preds[i] == PRED_INCREMENT {
                cur[i] = skipped + 1 + prev.map_or(0, |p| p[i]);
                i += 1;
                continue;
            }
            let raw: Vec<i32> = match encs[i] {
                ENC_SIGNED_VB => vec![s.signed_vb()],
                ENC_UNSIGNED_VB => vec![s.unsigned_vb() as i32],
                ENC_NEG_14BIT => vec![-sign_extend(s.unsigned_vb() & 0x3fff, 14)],
                ENC_TAG8_4S16 => tag8_4s16(s).to_vec(),
                ENC_TAG2_3S32 => tag2_3s32(s).to_vec(),
                ENC_TAG8_8SVB => {
                    let count = (i..n)
                        .take(8)
                        .take_while(|&k| encs[k] == ENC_TAG8_8SVB && preds[k] != PRED_INCREMENT)
                        .count();
                    tag8_8svb(s, count)
                }
                ENC_NULL => vec![0],
                _ => return false,
            };
            for (j, v) in raw.into_iter().enumerate() {
                let k = i + j;
                if k >= n {
                    break;
                }
                let Some(p) = self.predict(preds[k], k, cur, prev, prev2) else {
                    return false;
                };
                let v = v as i64 + p;
                cur[k] = if signed.get(k).copied().unwrap_or(false) {
                    v as i32 as i64
                } else {
                    v as u32 as i64
                };
            }
            i += match encs[i] {
                ENC_TAG8_4S16 => 4,
                ENC_TAG2_3S32 => 3,
                ENC_TAG8_8SVB => (i..n)
                    .take(8)
                    .take_while(|&k| encs[k] == ENC_TAG8_8SVB && preds[k] != PRED_INCREMENT)
                    .count(),
                _ => 1,
            };
            if s.eof {
                return false;
            }
        }
        true
    }

    fn predict(
        &self,
        pred: u8,
        k: usize,
        cur: &[i64],
        prev: Option<&[i64]>,
        prev2: Option<&[i64]>,
    ) -> Option<i64> {
        Some(match pred {
            PRED_ZERO => 0,
            PRED_PREVIOUS => prev.map_or(0, |p| p[k]),
            PRED_STRAIGHT_LINE => match (prev, prev2) {
                (Some(a), Some(b)) => 2 * a[k] - b[k],
                (Some(a), None) => a[k],
                _ => 0,
            },
            PRED_AVERAGE_2 => match (prev, prev2) {
                // The logger averages in C integer arithmetic: truncation toward zero.
                (Some(a), Some(b)) => (a[k] + b[k]) / 2,
                (Some(a), None) => a[k],
                _ => 0,
            },
            PRED_MINTHROTTLE => self.minthrottle,
            PRED_MOTOR_0 => cur[self.motor0?],
            PRED_1500 => 1500,
            PRED_VBATREF => self.vbatref,
            PRED_LAST_MAIN_FRAME_TIME => self.last_time.unwrap_or(0),
            PRED_MINMOTOR => self.minmotor,
            PRED_HOME_COORD => return None,
            _ => return None,
        })
    }

    fn slow_frame(&mut self, s: &mut Stream) -> bool {
        let n = self.slow.names.len();
        let mut cur = vec![0i64; n];
        let (p, e, sg) = (
            self.slow.predictor.clone(),
            self.slow.encoding.clone(),
            self.slow.signed.clone(),
        );
        if !self.read_fields(s, &p, &e, &sg, &mut cur, None, None, 0)
            || s.eof
            || !self.is_frame_byte(s.b.get(s.pos))
        {
            return false;
        }
        self.last_slow = cur;
        self.stats.slow_frames += 1;
        true
    }

    /// GPS and GPS-home frames are read past (the sim does not use them).
    fn gps_frame(&mut self, s: &mut Stream, home: bool) -> bool {
        let d = if home { &self.home } else { &self.gps };
        let n = d.names.len();
        let (p, e, sg) = (d.predictor.clone(), d.encoding.clone(), d.signed.clone());
        // Home coordinates are added after the read: the predictor table cannot see them.
        let p: Vec<u8> = p
            .into_iter()
            .map(|x| if x == PRED_HOME_COORD { PRED_ZERO } else { x })
            .collect();
        let mut cur = vec![0i64; n];
        if !self.read_fields(s, &p, &e, &sg, &mut cur, None, None, 0)
            || s.eof
            || !self.is_frame_byte(s.b.get(s.pos))
        {
            return false;
        }
        if home {
            self.home_values = cur;
        }
        true
    }

    /// An event frame: Some(true) at the log-end event, Some(false) for any other good
    /// event, None when the event is unknown or cut short.
    fn event(&mut self, s: &mut Stream) -> Option<bool> {
        let kind = s.byte();
        match kind {
            EVENT_SYNC_BEEP => {
                s.unsigned_vb();
            }
            EVENT_INFLIGHT_ADJUSTMENT => {
                let f = s.byte();
                if f & 0x80 != 0 {
                    for _ in 0..4 {
                        s.byte();
                    }
                } else {
                    s.signed_vb();
                }
            }
            EVENT_LOGGING_RESUME => {
                let it = s.unsigned_vb() as i64;
                let t = s.unsigned_vb() as i64;
                self.last_iter = Some(it);
                self.last_time = Some(t);
            }
            EVENT_DISARM => {
                s.unsigned_vb();
            }
            EVENT_FLIGHT_MODE => {
                s.unsigned_vb();
                s.unsigned_vb();
            }
            EVENT_LOG_END => {
                let text = b"End of log\0";
                let end = s.pos + text.len();
                if s.b.get(s.pos..end) == Some(&text[..]) {
                    s.pos = end;
                    self.stats.events += 1;
                    return Some(true);
                }
                return None;
            }
            _ => return None,
        }
        if s.eof || !self.is_frame_byte(s.b.get(s.pos)) {
            return None;
        }
        self.stats.events += 1;
        Some(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writers for the encodings, for round-trip tests (the logger's side of the format).
    fn uvb(out: &mut Vec<u8>, mut v: u32) {
        while v >= 0x80 {
            out.push((v as u8) | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }

    fn svb(out: &mut Vec<u8>, v: i32) {
        uvb(out, ((v << 1) ^ (v >> 31)) as u32);
    }

    fn put_tag8_4s16(out: &mut Vec<u8>, v: [i32; 4]) {
        let mut sel = 0u8;
        for x in (0..4).rev() {
            sel <<= 2;
            sel |= match v[x] {
                0 => 0,
                -8..=7 => 1,
                -128..=127 => 2,
                _ => 3,
            };
        }
        out.push(sel);
        let (mut half, mut buf) = (false, 0u8);
        let mut s = sel;
        for x in v {
            match s & 3 {
                0 => {}
                1 => {
                    if !half {
                        buf = (x << 4) as u8;
                        half = true;
                    } else {
                        out.push(buf | (x as u8 & 0x0f));
                        half = false;
                    }
                }
                2 => {
                    if !half {
                        out.push(x as u8);
                    } else {
                        out.push(buf | ((x >> 4) as u8 & 0x0f));
                        buf = (x << 4) as u8;
                    }
                }
                _ => {
                    if !half {
                        out.push((x >> 8) as u8);
                        out.push(x as u8);
                    } else {
                        out.push(buf | ((x >> 12) as u8 & 0x0f));
                        out.push((x >> 4) as u8);
                        buf = (x << 4) as u8;
                    }
                }
            }
            s >>= 2;
        }
        if half {
            out.push(buf);
        }
    }

    #[test]
    fn variable_bytes_round_trip() {
        for v in [0, 1, -1, 63, -64, 127, 300, -300, 1 << 20, i32::MIN / 2] {
            let mut b = Vec::new();
            svb(&mut b, v);
            let mut s = Stream {
                b: &b,
                pos: 0,
                eof: false,
            };
            assert_eq!(s.signed_vb(), v);
        }
    }

    #[test]
    fn tag8_4s16_round_trips_every_width_mix() {
        let vals = [0, 5, -8, 100, -128, 1000, -32768, 7, -3];
        for a in vals {
            for b in vals {
                for c in [0, -7, 200, -2000] {
                    for d in [3, -90, 0, 32767] {
                        let v = [a, b, c, d];
                        let mut bytes = Vec::new();
                        put_tag8_4s16(&mut bytes, v);
                        let mut s = Stream {
                            b: &bytes,
                            pos: 0,
                            eof: false,
                        };
                        assert_eq!(tag8_4s16(&mut s), v, "{v:?}");
                        assert_eq!(s.pos, bytes.len(), "{v:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn tag2_3s32_reads_each_width() {
        // 2-bit fields: 1, -1, 0.
        let mut s = Stream {
            b: &[0b00_01_11_00],
            pos: 0,
            eof: false,
        };
        assert_eq!(tag2_3s32(&mut s), [1, -1, 0]);
        // 4-bit fields: -8, 7, -1.
        let mut s = Stream {
            b: &[0x40 | 0x08, 0x7f],
            pos: 0,
            eof: false,
        };
        assert_eq!(tag2_3s32(&mut s), [-8, 7, -1]);
        // Variable: one byte (-2), two bytes (300), four bytes (-70000).
        let mut b = vec![0xc0 | (3 << 4) | (1 << 2)];
        b.push(0xfe);
        b.extend_from_slice(&300i16.to_le_bytes());
        b.extend_from_slice(&(-70000i32).to_le_bytes());
        let mut s = Stream {
            b: &b,
            pos: 0,
            eof: false,
        };
        assert_eq!(tag2_3s32(&mut s), [-2, 300, -70000]);
    }

    /// A tiny two-field log: an I frame, two P frames, a corrupt byte, a fresh I frame, P,
    /// and the log-end event.
    #[test]
    fn decodes_a_small_log_and_resyncs_after_corruption() {
        let mut b: Vec<u8> = Vec::new();
        let h = |b: &mut Vec<u8>, l: &str| {
            b.extend_from_slice(l.as_bytes());
            b.push(b'\n');
        };
        h(&mut b, std::str::from_utf8(LOG_MARKER).unwrap());
        h(&mut b, "H Data version:2");
        h(&mut b, "H I interval:4");
        h(&mut b, "H P interval:1");
        h(&mut b, "H Field I name:loopIteration,time,gyroADC[0]");
        h(&mut b, "H Field I signed:0,0,1");
        h(&mut b, "H Field I predictor:0,0,0");
        h(&mut b, "H Field I encoding:1,1,0");
        h(&mut b, "H Field P predictor:6,2,3");
        h(&mut b, "H Field P encoding:9,0,0");
        // I: iteration 0, time 1000, gyro -5.
        b.push(b'I');
        uvb(&mut b, 0);
        uvb(&mut b, 1000);
        svb(&mut b, -5);
        // P: time straight line from one frame (prev): +250 -> 1250; gyro avg -> -5 + 3.
        b.push(b'P');
        svb(&mut b, 250);
        svb(&mut b, 3);
        // P: time 2·1250 − 1000 = 1500, +0; gyro (−2 + −5)/2 = −3 (toward zero), −1 → −4.
        b.push(b'P');
        svb(&mut b, 0);
        svb(&mut b, -1);
        // P, then junk: a frame not followed by a frame byte is corrupt and dropped.
        b.push(b'P');
        svb(&mut b, 1);
        svb(&mut b, 1);
        b.push(0xff);
        b.push(b'I');
        uvb(&mut b, 4);
        uvb(&mut b, 2000);
        svb(&mut b, 9);
        b.push(b'P');
        svb(&mut b, 250);
        svb(&mut b, 0);
        b.push(b'E');
        b.push(EVENT_LOG_END);
        b.extend_from_slice(b"End of log\0");
        let logs = decode_image(&b);
        assert_eq!(logs.len(), 1);
        let l = logs[0].as_ref().unwrap();
        let rows: Vec<Vec<i64>> = l.frames.clone();
        assert_eq!(
            rows,
            vec![
                vec![0, 1000, -5],
                vec![1, 1250, -2],
                vec![2, 1500, -4],
                vec![4, 2000, 9],
                vec![5, 2250, 9],
            ]
        );
        assert!(l.stats.ended);
        assert_eq!(l.stats.corrupt, 1);
        assert_eq!((l.stats.i_frames, l.stats.p_frames), (2, 3));
        // The image holds only this log, so the frames after the marker are all of it.
        assert_eq!(l.duration(), 1.25e-3);
    }
}
