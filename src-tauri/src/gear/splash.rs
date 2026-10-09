//! The EdgeTX splash screen (design 7.5): an image made 1-bit at the radio's size, a preview
//! of it, and the patch of a firmware image.
//!
//! The binary holds the splash between two markers: `SPS\0`, the width byte `0x80`, the
//! height byte `0x40`, 1,024 image bytes, then `SPE`. The image is 8 vertical pixels per
//! byte: byte `band * 128 + x` holds the pixels of column `x` in rows `band * 8` to
//! `band * 8 + 7`, the top row in bit 0, and a set bit is a dark pixel. The patch refuses a
//! binary where a marker is missing, appears twice, or sits anywhere else than the layout
//! says, then decodes what it wrote and compares it with the picture.
//!
//! This is QuadCam's own code from the layout in the design. No EdgeTX source or binary is
//! in this repository: the tests build a synthetic binary with the markers. The layout has
//! not been compared with a downloaded release binary yet, so `compat.rs` lists a splash
//! pair only after a test on that release passes.

use super::model::{Refusal, RefusalCode};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use specta::Type;

pub const WIDTH: usize = 128;
pub const HEIGHT: usize = 64;
/// Image bytes between the header and the end marker.
pub const IMAGE_BYTES: usize = WIDTH * HEIGHT / 8;
pub const START: &[u8; 4] = b"SPS\0";
pub const END: &[u8; 3] = b"SPE";
/// Marker, width byte and height byte.
const HEADER: usize = 6;

/// EdgeTX boards with a 128 x 64, 1-bit screen. Any other board (colour radios, the wide
/// 212 x 64 screens) is refused: its splash format is not supported yet.
pub const MONO_128X64: &[&str] = &[
    "pocket", "boxer", "zorro", "tx12", "tx12mk2", "mt12", "t12", "t14", "t8", "tlite", "tpro",
    "tpros", "lr3pro", "x7", "xlite", "xlites",
];

/// True when the board's splash is a 128 x 64, 1-bit picture.
pub fn board_supported(board: &str) -> bool {
    let b = board.trim().to_ascii_lowercase();
    MONO_128X64.contains(&b.as_str())
}

/// The refusal for a board with no splash support.
pub fn unsupported(board: &str) -> Refusal {
    Refusal::new(
        RefusalCode::UnknownBoard,
        format!(
            "This radio's splash format is not supported yet (board {}).",
            board.trim()
        ),
    )
}

/// A 128 x 64 picture, one bool per pixel, row by row. `true` is a dark pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mono {
    dark: Vec<bool>,
}

impl Mono {
    pub fn blank() -> Mono {
        Mono {
            dark: vec![false; WIDTH * HEIGHT],
        }
    }

    pub fn get(&self, x: usize, y: usize) -> bool {
        self.dark[y * WIDTH + x]
    }

    pub fn set(&mut self, x: usize, y: usize, dark: bool) {
        self.dark[y * WIDTH + x] = dark;
    }

    pub fn dark_count(&self) -> usize {
        self.dark.iter().filter(|d| **d).count()
    }

    /// The 1,024 bytes of the binary's layout.
    pub fn pack(&self) -> [u8; IMAGE_BYTES] {
        let mut out = [0u8; IMAGE_BYTES];
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                if self.get(x, y) {
                    out[(y / 8) * WIDTH + x] |= 1 << (y % 8);
                }
            }
        }
        out
    }

    /// The picture of 1,024 packed bytes.
    pub fn unpack(bytes: &[u8]) -> Result<Mono> {
        if bytes.len() != IMAGE_BYTES {
            bail!("A splash is {IMAGE_BYTES} bytes, not {}.", bytes.len());
        }
        let mut m = Mono::blank();
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                m.set(x, y, bytes[(y / 8) * WIDTH + x] >> (y % 8) & 1 == 1);
            }
        }
        Ok(m)
    }

    /// The picture as an 8-bit grey PNG, `scale` times larger, each pixel a square
    /// (nearest neighbour). Dark pixels draw black on white.
    pub fn to_png(&self, scale: usize) -> Result<Vec<u8>> {
        let scale = scale.clamp(1, 16);
        let (w, h) = (WIDTH * scale, HEIGHT * scale);
        let mut px = vec![255u8; w * h];
        for y in 0..h {
            for x in 0..w {
                if self.get(x / scale, y / scale) {
                    px[y * w + x] = 0;
                }
            }
        }
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
            enc.set_color(png::ColorType::Grayscale);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().context("writing the preview")?;
            wr.write_image_data(&px).context("writing the preview")?;
        }
        Ok(out)
    }
}

/// A decoded picture: 8-bit grey values (0 black, 255 white) with alpha applied over white.
#[derive(Debug, Clone)]
pub struct Grey {
    pub width: usize,
    pub height: usize,
    pub px: Vec<u8>,
}

/// Decodes a PNG into grey values. Colour is weighed by luma; transparency counts as white.
pub fn decode_png(bytes: &[u8]) -> Result<Grey> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().map_err(|e| anyhow!("Not a PNG image: {e}"))?;
    let mut buf = vec![0u8; reader.output_buffer_size().ok_or_else(|| anyhow!("The image is too large."))?];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| anyhow!("Not a PNG image: {e}"))?;
    let (w, h) = (info.width as usize, info.height as usize);
    if w == 0 || h == 0 || w > 8192 || h > 8192 {
        bail!("The image is {w} x {h}; the limit is 8192 on each side.");
    }
    let data = &buf[..info.buffer_size()];
    let over_white = |v: f32, a: f32| (v * a + 255.0 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
    let luma = |r: u8, g: u8, b: u8| 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
    let px: Vec<u8> = match info.color_type {
        png::ColorType::Grayscale => data.to_vec(),
        png::ColorType::GrayscaleAlpha => data
            .chunks_exact(2)
            .map(|c| over_white(c[0] as f32, c[1] as f32 / 255.0))
            .collect(),
        png::ColorType::Rgb => data
            .chunks_exact(3)
            .map(|c| luma(c[0], c[1], c[2]).round() as u8)
            .collect(),
        png::ColorType::Rgba => data
            .chunks_exact(4)
            .map(|c| over_white(luma(c[0], c[1], c[2]), c[3] as f32 / 255.0))
            .collect(),
        png::ColorType::Indexed => bail!("The image uses a palette the decoder did not expand."),
    };
    if px.len() != w * h {
        bail!("The image data does not match its size.");
    }
    Ok(Grey {
        width: w,
        height: h,
        px,
    })
}

/// Fits a picture into 128 x 64 (scaled to fit, centred, white around it) and cuts it to
/// two tones: a pixel darker than `threshold` (0-255) is dark. `invert` swaps the tones.
pub fn to_mono(g: &Grey, threshold: u8, invert: bool) -> Mono {
    let scale = (WIDTH as f32 / g.width as f32).min(HEIGHT as f32 / g.height as f32);
    let (tw, th) = (
        ((g.width as f32 * scale).round() as usize).clamp(1, WIDTH),
        ((g.height as f32 * scale).round() as usize).clamp(1, HEIGHT),
    );
    let (ox, oy) = ((WIDTH - tw) / 2, (HEIGHT - th) / 2);
    let mut m = Mono::blank();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let inside = x >= ox && x < ox + tw && y >= oy && y < oy + th;
            let value = if inside {
                // The mean of the source pixels this target pixel covers (a box filter).
                let x0 = (x - ox) * g.width / tw;
                let x1 = (((x - ox + 1) * g.width).div_ceil(tw)).clamp(x0 + 1, g.width);
                let y0 = (y - oy) * g.height / th;
                let y1 = (((y - oy + 1) * g.height).div_ceil(th)).clamp(y0 + 1, g.height);
                let mut sum = 0u64;
                for sy in y0..y1 {
                    for sx in x0..x1 {
                        sum += g.px[sy * g.width + sx] as u64;
                    }
                }
                (sum / ((x1 - x0) * (y1 - y0)) as u64) as u8
            } else {
                255
            };
            m.set(x, y, (value < threshold) != invert);
        }
    }
    m
}

/// Why a firmware image does not hold a splash QuadCam can patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplashError {
    /// The start marker is not in the image.
    MarkerMissing,
    /// The start marker appears this many times.
    MarkerDoubled(usize),
    /// The width and height bytes are not 128 x 64.
    BadHeader,
    /// The end marker is not where the layout puts it.
    EndMissing,
}

impl SplashError {
    pub fn refusal(&self) -> Refusal {
        let reason = match self {
            SplashError::MarkerMissing => {
                "The firmware has no splash start marker (SPS); QuadCam will not patch it.".into()
            }
            SplashError::MarkerDoubled(n) => format!(
                "The splash start marker (SPS) appears {n} times; QuadCam will not guess which to patch."
            ),
            SplashError::BadHeader => {
                "The splash in this firmware is not 128 x 64; QuadCam will not patch it.".into()
            }
            SplashError::EndMissing => {
                "The splash end marker (SPE) is not where it should be; QuadCam will not patch it."
                    .into()
            }
        };
        Refusal::new(RefusalCode::BadImage, reason)
    }
}

/// The offset of the 1,024 image bytes, after every marker check.
pub fn locate(bin: &[u8]) -> Result<usize, SplashError> {
    let hits: Vec<usize> = bin
        .windows(START.len())
        .enumerate()
        .filter(|(_, w)| *w == START)
        .map(|(i, _)| i)
        .collect();
    let at = match hits.as_slice() {
        [] => return Err(SplashError::MarkerMissing),
        [one] => *one,
        many => return Err(SplashError::MarkerDoubled(many.len())),
    };
    let image = at + HEADER;
    let end = image + IMAGE_BYTES;
    if bin.get(at + 4) != Some(&(WIDTH as u8)) || bin.get(at + 5) != Some(&(HEIGHT as u8)) {
        return Err(SplashError::BadHeader);
    }
    if bin.get(end..end + END.len()) != Some(&END[..]) {
        return Err(SplashError::EndMissing);
    }
    Ok(image)
}

/// The splash a firmware image holds now.
pub fn decode(bin: &[u8]) -> Result<Mono, SplashError> {
    let at = locate(bin)?;
    Ok(Mono::unpack(&bin[at..at + IMAGE_BYTES]).expect("1,024 bytes"))
}

/// A copy of the firmware image with `picture` as its splash. Decodes what it wrote and
/// compares it with the picture; only the 1,024 image bytes differ from the input.
pub fn patch(bin: &[u8], picture: &Mono) -> Result<Vec<u8>, SplashError> {
    let at = locate(bin)?;
    let mut out = bin.to_vec();
    out[at..at + IMAGE_BYTES].copy_from_slice(&picture.pack());
    debug_assert_eq!(decode(&out).as_ref(), Ok(picture));
    match decode(&out) {
        Ok(back) if &back == picture => Ok(out),
        _ => Err(SplashError::EndMissing),
    }
}

/// `gear_splash`: the image to preview.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SplashParams {
    /// A PNG file.
    pub image: std::path::PathBuf,
    /// Grey values under this are dark (0-255). Default 128.
    #[serde(default)]
    pub threshold: Option<u8>,
    #[serde(default)]
    pub invert: bool,
    /// The radio's board (`pocket`); a colour radio is refused.
    #[serde(default)]
    pub board: Option<String>,
}

impl SplashParams {
    pub fn threshold(&self) -> u8 {
        self.threshold.unwrap_or(128)
    }
}

/// The picture at the radio's size and depth, ready to draw.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct SplashPreview {
    pub width: u32,
    pub height: u32,
    pub threshold: u8,
    pub invert: bool,
    pub board: Option<String>,
    /// Dark pixels of the 8,192.
    pub dark: u32,
    /// A PNG at 4 times the size, as base64 (nearest-neighbour: each pixel a 4 x 4 square).
    pub png_base64: String,
    /// The board has a 128 x 64, 1-bit screen.
    pub supported: bool,
    /// Why not, when it is not.
    #[serde(default)]
    pub reason: Option<String>,
    /// The 1,024 packed bytes as hex, to compare with `splash_hash` of a binary.
    pub hash: String,
}

/// Reads the PNG at `p.image` and previews it.
pub fn preview(p: &SplashParams) -> Result<(Mono, SplashPreview)> {
    use base64::Engine;
    let bytes = std::fs::read(&p.image)
        .with_context(|| format!("Cannot read {}", p.image.display()))?;
    let mono = to_mono(&decode_png(&bytes)?, p.threshold(), p.invert);
    let (supported, reason) = match p.board.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        Some(b) if !board_supported(b) => (false, Some(unsupported(b).reason)),
        _ => (true, None),
    };
    let png = mono.to_png(4)?;
    let hash = super::blobs::hash(&mono.pack());
    Ok((
        mono.clone(),
        SplashPreview {
            width: WIDTH as u32,
            height: HEIGHT as u32,
            threshold: p.threshold(),
            invert: p.invert,
            board: p.board.clone(),
            dark: mono.dark_count() as u32,
            png_base64: base64::engine::general_purpose::STANDARD.encode(png),
            supported,
            reason,
            hash,
        },
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A synthetic firmware: filler, the markers around a blank splash, more filler.
    pub(crate) fn synthetic_binary() -> Vec<u8> {
        let mut bin = vec![0xA5u8; 700];
        bin.extend_from_slice(START);
        bin.push(WIDTH as u8);
        bin.push(HEIGHT as u8);
        bin.extend_from_slice(&[0u8; IMAGE_BYTES]);
        bin.extend_from_slice(END);
        bin.extend_from_slice(&[0x5Au8; 300]);
        bin
    }

    pub(crate) fn picture() -> Mono {
        let mut m = Mono::blank();
        for i in 0..64 {
            m.set(i, i, true);
            m.set(127 - i, i, true);
        }
        for x in 0..128 {
            m.set(x, 0, true);
            m.set(x, 63, true);
        }
        m
    }

    #[test]
    fn pack_puts_the_top_row_in_bit_zero_of_each_column() {
        let mut m = Mono::blank();
        m.set(5, 0, true);
        m.set(5, 9, true);
        let b = m.pack();
        assert_eq!(b[5], 0b0000_0001, "band 0, row 0");
        assert_eq!(b[128 + 5], 0b0000_0010, "band 1, row 9");
        assert_eq!(b.iter().filter(|v| **v != 0).count(), 2);
        assert_eq!(Mono::unpack(&b).unwrap(), m);
    }

    #[test]
    fn patch_and_decode_round_trip_changes_only_the_image() {
        let bin = synthetic_binary();
        assert_eq!(decode(&bin).unwrap(), Mono::blank());
        let out = patch(&bin, &picture()).unwrap();
        assert_eq!(out.len(), bin.len());
        assert_eq!(decode(&out).unwrap(), picture());
        let at = locate(&bin).unwrap();
        assert_eq!(&out[..at], &bin[..at]);
        assert_eq!(&out[at + IMAGE_BYTES..], &bin[at + IMAGE_BYTES..]);
        assert_ne!(&out[at..at + IMAGE_BYTES], &bin[at..at + IMAGE_BYTES]);
    }

    #[test]
    fn a_missing_marker_refuses() {
        let bin = vec![0u8; 4000];
        assert_eq!(patch(&bin, &picture()), Err(SplashError::MarkerMissing));
        let r = SplashError::MarkerMissing.refusal();
        assert_eq!(r.code, RefusalCode::BadImage);
    }

    #[test]
    fn a_doubled_marker_refuses() {
        let mut bin = synthetic_binary();
        bin.extend_from_slice(&synthetic_binary());
        assert_eq!(patch(&bin, &picture()), Err(SplashError::MarkerDoubled(2)));
        assert!(SplashError::MarkerDoubled(2)
            .refusal()
            .reason
            .contains("2 times"));
    }

    #[test]
    fn a_wrong_size_or_end_refuses() {
        let mut bin = synthetic_binary();
        let at = locate(&bin).unwrap();
        bin[at - 2] = 0xC0; // the width byte
        assert_eq!(patch(&bin, &picture()), Err(SplashError::BadHeader));
        let mut bin = synthetic_binary();
        let at = locate(&bin).unwrap();
        bin[at + IMAGE_BYTES] = b'X'; // SPE
        assert_eq!(patch(&bin, &picture()), Err(SplashError::EndMissing));
        // The start marker at the very end of the file: nothing after it.
        let mut bin = vec![1u8; 10];
        bin.extend_from_slice(START);
        assert_eq!(patch(&bin, &picture()), Err(SplashError::BadHeader));
    }

    #[test]
    fn boards_with_a_mono_screen_only() {
        assert!(board_supported("pocket"));
        assert!(board_supported(" Pocket "));
        assert!(!board_supported("tx16s"));
        assert!(!board_supported("x9d"));
        assert!(unsupported("tx16s")
            .reason
            .contains("splash format is not supported yet"));
    }

    fn png_of(color: png::ColorType, w: u32, h: u32, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(data).unwrap();
        out
    }

    #[test]
    fn an_image_becomes_two_tones_at_128_by_64() {
        // Left half black, right half white, 256 x 128 grey.
        let mut px = Vec::new();
        for _y in 0..128 {
            for x in 0..256 {
                px.push(if x < 128 { 0u8 } else { 255 });
            }
        }
        let g = decode_png(&png_of(png::ColorType::Grayscale, 256, 128, &px)).unwrap();
        let m = to_mono(&g, 128, false);
        assert!(m.get(10, 10) && m.get(60, 40));
        assert!(!m.get(70, 10) && !m.get(120, 60));
        let inv = to_mono(&g, 128, true);
        assert!(!inv.get(10, 10) && inv.get(120, 60));
        // A higher threshold darkens mid grey, a lower one does not.
        let mid = decode_png(&png_of(png::ColorType::Grayscale, 2, 1, &[100, 100])).unwrap();
        assert!(to_mono(&mid, 128, false).get(64, 32));
        assert!(!to_mono(&mid, 90, false).get(64, 32));
    }

    #[test]
    fn a_tall_image_is_centred_with_white_beside_it() {
        let g = decode_png(&png_of(png::ColorType::Grayscale, 32, 64, &[0u8; 32 * 64])).unwrap();
        let m = to_mono(&g, 128, false);
        assert!(!m.get(10, 30), "white beside the picture");
        assert!(m.get(64, 30), "black picture in the middle");
        assert!(!m.get(118, 30));
    }

    #[test]
    fn transparency_counts_as_white() {
        let rgba = [0u8, 0, 0, 0, 0, 0, 0, 255];
        let g = decode_png(&png_of(png::ColorType::Rgba, 2, 1, &rgba)).unwrap();
        assert_eq!(g.px, vec![255, 0]);
    }

    #[test]
    fn not_a_png_is_an_error() {
        assert!(decode_png(b"GIF89a").is_err());
    }

    #[test]
    fn the_preview_is_a_4x_nearest_png() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("s.png");
        std::fs::write(
            &f,
            png_of(png::ColorType::Grayscale, 128, 64, &[0u8; 128 * 64]),
        )
        .unwrap();
        let (m, p) = preview(&SplashParams {
            image: f.clone(),
            threshold: None,
            invert: false,
            board: Some("pocket".into()),
        })
        .unwrap();
        assert_eq!(m.dark_count(), 8192);
        assert!(p.supported && p.reason.is_none());
        let g = decode_png(&{
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(&p.png_base64)
                .unwrap()
        })
        .unwrap();
        assert_eq!((g.width, g.height), (512, 256));
        assert!(g.px.iter().all(|v| *v == 0));
        let (_, c) = preview(&SplashParams {
            image: f,
            threshold: Some(10),
            invert: true,
            board: Some("tx16s".into()),
        })
        .unwrap();
        assert!(!c.supported);
        assert_eq!(c.dark, 0);
        assert!(c.reason.unwrap().contains("not supported yet"));
    }
}
