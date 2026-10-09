//! Plain RIFF WAV: read a take from a provider, write a mono 16-bit file for the radio.

use anyhow::{bail, Result};

/// Mono 16-bit samples and their rate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm {
    pub rate: u32,
    pub samples: Vec<i16>,
}

/// Reads a PCM WAV (16-bit, or 8/24/32-bit integer converted to 16). More than one channel
/// is averaged to mono. A streaming WAV whose sizes read 0 or 0xFFFFFFFF takes the rest of
/// the bytes as its data.
pub fn read(bytes: &[u8]) -> Result<Pcm> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bail!("not a WAV file");
    }
    let mut at = 12;
    let (mut rate, mut channels, mut bits, mut format) = (0u32, 0u16, 0u16, 0u16);
    let mut data: Option<&[u8]> = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = at + 8;
        if id == b"fmt " {
            if body + 16 > bytes.len() {
                bail!("the WAV format chunk is cut short");
            }
            format = u16::from_le_bytes(bytes[body..body + 2].try_into().unwrap());
            channels = u16::from_le_bytes(bytes[body + 2..body + 4].try_into().unwrap());
            rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().unwrap());
            bits = u16::from_le_bytes(bytes[body + 14..body + 16].try_into().unwrap());
        } else if id == b"data" {
            let end = if size == 0 || size == 0xFFFF_FFFF || body + size > bytes.len() {
                bytes.len()
            } else {
                body + size
            };
            data = Some(&bytes[body..end]);
            break;
        }
        at = body + size + (size & 1);
    }
    let Some(data) = data else {
        bail!("the WAV has no data chunk");
    };
    // 1 is integer PCM; 0xFFFE is extensible, taken as integer when the bits say so.
    if rate == 0 || channels == 0 || !(format == 1 || format == 0xFFFE) {
        bail!("only PCM WAV is read (format {format}, {channels} channels, {rate} Hz)");
    }
    let width = (bits / 8) as usize;
    if !(1..=4).contains(&width) {
        bail!("{bits}-bit WAV is not read");
    }
    let frame = width * channels as usize;
    let mut samples = Vec::with_capacity(data.len() / frame.max(1));
    for f in data.chunks_exact(frame) {
        let mut sum = 0i64;
        for c in 0..channels as usize {
            let s = &f[c * width..(c + 1) * width];
            let v: i64 = match width {
                1 => (i64::from(s[0]) - 128) << 8,
                2 => i64::from(i16::from_le_bytes([s[0], s[1]])),
                3 => i64::from(i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 16),
                _ => i64::from(i32::from_le_bytes([s[0], s[1], s[2], s[3]]) >> 16),
            };
            sum += v;
        }
        samples.push((sum / i64::from(channels)) as i16);
    }
    Ok(Pcm { rate, samples })
}

/// A mono 16-bit PCM WAV with a 44-byte header.
pub fn write(p: &Pcm) -> Vec<u8> {
    let data = p.samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&p.rate.to_le_bytes());
    out.extend_from_slice(&(p.rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data as u32).to_le_bytes());
    for s in &p.samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_round_trips() {
        let p = Pcm {
            rate: 32000,
            samples: vec![0, 1, -1, 32767, -32768],
        };
        let w = write(&p);
        assert_eq!(w.len(), 44 + 10);
        assert_eq!(read(&w).unwrap(), p);
    }

    #[test]
    fn stereo_averages_and_a_streaming_header_reads_to_the_end() {
        let mut w = write(&Pcm {
            rate: 24000,
            samples: vec![100, 300, -100, -300],
        });
        // Make it stereo: two channels, so two frames of (100,300) and (-100,-300).
        w[22] = 2;
        w[32] = 4;
        let p = read(&w).unwrap();
        assert_eq!(p.samples, vec![200, -200]);
        let mut s = write(&Pcm {
            rate: 24000,
            samples: vec![5, 6, 7],
        });
        s[40..44].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        assert_eq!(read(&s).unwrap().samples, vec![5, 6, 7]);
    }

    #[test]
    fn junk_is_refused() {
        assert!(read(b"nope").is_err());
        let mut w = write(&Pcm {
            rate: 8000,
            samples: vec![1],
        });
        w[20] = 3; // IEEE float
        assert!(read(&w).unwrap_err().to_string().contains("only PCM"));
    }
}
