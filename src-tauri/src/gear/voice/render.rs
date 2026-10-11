//! The render pipeline (design 7.4): spoken text, a provider take (cached), then
//! normalisation to the radio's rate and shape.
//!
//! 1. The raw take is cached by provider, voice, model, provider speed, spoken text and seed
//!    (`<cache>/voice/raw/<provider>/<key>.wav`, the take as mono 16-bit PCM). A re-render
//!    with other trim or tempo settings finds it there and calls no provider.
//! 2. `atempo` and the resample to 32 kHz run in ffmpeg, only when the take needs them
//!    (tempo other than 1, or another rate). Done first, so the pads below keep their length.
//! 3. Trim: windows of 5 ms; speech is every window within `trim_db` of the take's peak.
//!    Everything before the first and after the last such window goes.
//! 4. Lead and tail silence are added, the speech gets a fade in and a fade out, and the
//!    result is a plain RIFF WAV, 32 kHz 16-bit mono. Steps 3 and 4 are integer math in
//!    Rust, so the same take always gives the same bytes.

use super::tts::{Tts, TtsRequest};
use super::wav::{self, Pcm};
use crate::media::{self, Tools};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};

/// The rate of every WAV QuadCam writes (the rate the official packs use).
pub const OUT_RATE: u32 = 32000;

fn one() -> f64 {
    1.0
}
fn trim() -> f64 {
    -55.0
}
fn lead() -> u32 {
    20
}
fn tail() -> u32 {
    150
}
fn fade_in() -> u32 {
    5
}
fn fade_out() -> u32 {
    30
}

/// How a voice is rendered. Every field is in the pack's index entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct RenderSettings {
    /// The provider's own speaking speed. Part of the raw cache key.
    #[serde(default = "one")]
    pub speed: f64,
    /// `atempo` after the take, 0.5 to 2. Not part of the cache key.
    #[serde(default = "one")]
    pub tempo: f64,
    /// Speech is every window within this many dB of the peak.
    #[serde(default = "trim")]
    pub trim_db: f64,
    #[serde(default = "lead")]
    pub lead_ms: u32,
    #[serde(default = "tail")]
    pub tail_ms: u32,
    #[serde(default = "fade_in")]
    pub fade_in_ms: u32,
    #[serde(default = "fade_out")]
    pub fade_out_ms: u32,
    /// Fixes a take where the provider can; always part of the cache key.
    #[serde(default)]
    pub seed: u64,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            speed: 1.0,
            tempo: 1.0,
            trim_db: -55.0,
            lead_ms: 20,
            tail_ms: 150,
            fade_in_ms: 5,
            fade_out_ms: 30,
            seed: 0,
        }
    }
}

impl RenderSettings {
    pub fn check(&self) -> Result<()> {
        if !(0.25..=4.0).contains(&self.speed) {
            bail!("speed takes 0.25 to 4, not {}", self.speed);
        }
        if !(0.5..=2.0).contains(&self.tempo) {
            bail!("tempo takes 0.5 to 2, not {}", self.tempo);
        }
        if !(-90.0..=-10.0).contains(&self.trim_db) {
            bail!("trim_db takes -90 to -10, not {}", self.trim_db);
        }
        Ok(())
    }
}

// ----- the raw cache -----

/// The raw takes: `<cache>/voice/raw/<provider>/<key>.wav`.
#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn new(cache_dir: &Path) -> Self {
        Self {
            root: cache_dir.join("voice").join("raw"),
        }
    }

    /// The key of a take: provider, voice, model, provider speed, spoken text and seed.
    pub fn key(
        provider: &str,
        voice: &str,
        model: &str,
        speed: f64,
        text: &str,
        seed: u64,
    ) -> String {
        let canon = serde_json::json!({
            "provider": provider, "voice": voice, "model": model,
            "speed": format!("{speed:.4}"), "text": text, "seed": seed,
        });
        super::sha256_hex(canon.to_string().as_bytes())
    }

    fn path(&self, provider: &str, key: &str) -> PathBuf {
        let p: String = provider
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        self.root.join(p).join(format!("{key}.wav"))
    }

    pub fn get(&self, provider: &str, key: &str) -> Option<Pcm> {
        wav::read(&std::fs::read(self.path(provider, key)).ok()?).ok()
    }

    pub fn has(&self, provider: &str, key: &str) -> bool {
        self.path(provider, key).is_file()
    }

    /// Removes one take. True when there was one.
    pub fn remove(&self, provider: &str, key: &str) -> bool {
        std::fs::remove_file(self.path(provider, key)).is_ok()
    }

    pub fn put(&self, provider: &str, key: &str, take: &Pcm) -> Result<()> {
        let p = self.path(provider, key);
        std::fs::create_dir_all(p.parent().unwrap())?;
        let tmp = p.with_extension("tmp");
        std::fs::write(&tmp, wav::write(take))?;
        std::fs::rename(&tmp, &p)?;
        Ok(())
    }
}

// ----- the pipeline -----

/// What a render needs besides the text.
pub struct Ctx<'a> {
    pub tts: &'a dyn Tts,
    pub cache: &'a Cache,
    pub voice: &'a str,
    pub model: &'a str,
    pub settings: &'a RenderSettings,
    /// Needed only when a take needs `atempo` or a resample.
    pub tools: Option<&'a Tools>,
    /// Takes already cut from batches (`batch::render_all`), by spoken text. A line found
    /// here never reaches the provider.
    pub cuts: Option<&'a std::collections::HashMap<String, Pcm>>,
}

/// One take: from the cache, or from the provider (then cached). Returns whether the cache
/// answered.
pub fn take(ctx: &Ctx<'_>, spoken: &str) -> Result<(Pcm, bool)> {
    if let Some(p) = ctx.cuts.and_then(|c| c.get(spoken)) {
        return Ok((p.clone(), true));
    }
    let s = ctx.settings;
    let key = Cache::key(ctx.tts.id(), ctx.voice, ctx.model, s.speed, spoken, s.seed);
    if let Some(p) = ctx.cache.get(ctx.tts.id(), &key) {
        return Ok((p, true));
    }
    let p = ctx
        .tts
        .render(&TtsRequest {
            text: spoken,
            voice: ctx.voice,
            model: ctx.model,
            speed: s.speed,
            seed: s.seed,
        })
        .with_context(|| format!("rendering {spoken:?}"))?;
    ctx.cache.put(ctx.tts.id(), &key, &p)?;
    Ok((p, false))
}

/// Whether the cache holds the take for this text.
pub fn cached(ctx: &Ctx<'_>, spoken: &str) -> bool {
    let s = ctx.settings;
    ctx.cache.has(
        ctx.tts.id(),
        &Cache::key(ctx.tts.id(), ctx.voice, ctx.model, s.speed, spoken, s.seed),
    )
}

/// `atempo` and the resample to `OUT_RATE`, in ffmpeg.
fn ffmpeg_pass(tools: &Tools, p: &Pcm, tempo: f64) -> Result<Pcm> {
    let dir = std::env::temp_dir().join(format!(
        "quadcam-voice-{}-{}",
        std::process::id(),
        super::sha256_hex(
            &p.samples
                .iter()
                .flat_map(|s| s.to_le_bytes())
                .collect::<Vec<_>>()
        )
        .get(..12)
        .unwrap_or("x")
    ));
    std::fs::create_dir_all(&dir)?;
    let (src, dst) = (dir.join("in.wav"), dir.join("out.wav"));
    std::fs::write(&src, wav::write(p))?;
    let mut args = vec!["-i".to_string(), src.display().to_string()];
    if (tempo - 1.0).abs() > 1e-9 {
        args.push("-af".into());
        args.push(format!("atempo={tempo}"));
    }
    args.extend([
        "-ar".into(),
        OUT_RATE.to_string(),
        "-ac".into(),
        "1".into(),
        "-c:a".into(),
        "pcm_s16le".into(),
        dst.display().to_string(),
    ]);
    let r = media::run_ffmpeg(tools, &args, &mut |_| {});
    let out = std::fs::read(&dst);
    let _ = std::fs::remove_dir_all(&dir);
    r?;
    wav::read(&out.context("ffmpeg wrote no file")?)
}

/// Trims to the speech, adds the pads and the fades. Pure integer math.
pub fn shape(samples: &[i16], s: &RenderSettings) -> Result<Vec<i16>> {
    let peak = samples
        .iter()
        .map(|v| i32::from(*v).abs())
        .max()
        .unwrap_or(0);
    if peak == 0 {
        bail!("the take is silent");
    }
    let win = (OUT_RATE as usize * 5) / 1000;
    let thr = (f64::from(peak) * 10f64.powf(s.trim_db / 20.0)).ceil() as i32;
    let level = |w: &[i16]| w.iter().map(|v| i32::from(*v).abs()).max().unwrap_or(0);
    let windows: Vec<i32> = samples.chunks(win).map(level).collect();
    let first = windows
        .iter()
        .position(|l| *l >= thr)
        .expect("the peak window");
    let last = windows
        .iter()
        .rposition(|l| *l >= thr)
        .expect("the peak window");
    let from = first * win;
    let to = ((last + 1) * win).min(samples.len());
    let mut speech: Vec<i16> = samples[from..to].to_vec();
    let ms = |m: u32| (OUT_RATE as usize * m as usize) / 1000;
    let n_in = ms(s.fade_in_ms).min(speech.len());
    for (i, v) in speech.iter_mut().take(n_in).enumerate() {
        *v = (i64::from(*v) * i as i64 / n_in as i64) as i16;
    }
    let n_out = ms(s.fade_out_ms).min(speech.len());
    let len = speech.len();
    for d in 0..n_out {
        let v = &mut speech[len - 1 - d];
        *v = (i64::from(*v) * d as i64 / n_out as i64) as i16;
    }
    let mut out = vec![0i16; ms(s.lead_ms)];
    out.extend(speech);
    out.extend(std::iter::repeat_n(0i16, ms(s.tail_ms)));
    Ok(out)
}

/// A take as the radio's WAV bytes.
pub fn normalise(p: &Pcm, s: &RenderSettings, tools: Option<&Tools>) -> Result<Vec<u8>> {
    s.check()?;
    let p = if (s.tempo - 1.0).abs() > 1e-9 || p.rate != OUT_RATE {
        let Some(t) = tools else {
            bail!("ffmpeg is needed to change the tempo or the rate: brew install ffmpeg, or install QuadCam's ffmpeg module");
        };
        ffmpeg_pass(t, p, s.tempo)?
    } else {
        p.clone()
    };
    let samples = shape(&p.samples, s)?;
    Ok(wav::write(&Pcm {
        rate: OUT_RATE,
        samples,
    }))
}

/// One line, start to finish. Returns the WAV and whether the cache gave the take.
pub fn render_line(ctx: &Ctx<'_>, spoken: &str) -> Result<(Vec<u8>, bool)> {
    let (p, hit) = take(ctx, spoken)?;
    Ok((normalise(&p, ctx.settings, ctx.tools)?, hit))
}

/// What a batch would cost.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct Plan {
    pub lines: u32,
    /// Takes the cache already holds.
    pub cached: u32,
    /// Takes the provider must make.
    pub to_render: u32,
    /// Characters of the takes the provider must make.
    pub chars: u64,
}

pub fn plan(ctx: &Ctx<'_>, spoken: &[String]) -> Plan {
    let mut p = Plan {
        lines: spoken.len() as u32,
        ..Default::default()
    };
    let mut seen = std::collections::HashSet::new();
    for t in spoken {
        if cached(ctx, t) {
            p.cached += 1;
        } else if seen.insert(t.clone()) {
            p.to_render += 1;
            p.chars += t.chars().count() as u64;
        } else {
            p.cached += 1;
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A burst of 12 windows of +/-8000, with silence and a quiet tail around it.
    fn take_samples() -> Vec<i16> {
        let win = 160;
        let mut v = vec![3i16; win * 8];
        for i in 0..win * 12 {
            v.push(if (i / 18) % 2 == 0 { 8000 } else { -8000 });
        }
        v.extend(std::iter::repeat_n(5i16, win * 12));
        v
    }

    #[test]
    fn shape_trims_pads_and_fades() {
        let s = RenderSettings::default();
        let out = shape(&take_samples(), &s).unwrap();
        let ms = |m: usize| 32 * m;
        assert_eq!(out.len(), ms(20) + 160 * 12 + ms(150));
        // Lead silence, then a fade in from zero.
        assert!(out[..ms(20)].iter().all(|v| *v == 0));
        assert_eq!(out[ms(20)], 0);
        assert!(out[ms(20) + 80].abs() < 8000 && out[ms(20) + 80] != 0);
        assert_eq!(out[ms(20) + 200].abs(), 8000);
        // The fade out ends at zero, then the tail is silence.
        assert_eq!(out[ms(20) + 160 * 12 - 1], 0);
        assert!(out[out.len() - ms(150)..].iter().all(|v| *v == 0));
    }

    #[test]
    fn a_silent_take_is_refused() {
        assert!(shape(&[0; 800], &RenderSettings::default())
            .unwrap_err()
            .to_string()
            .contains("silent"));
    }

    #[test]
    fn a_stricter_trim_keeps_less() {
        let mut quiet = take_samples();
        // A word-tail 50 dB under the peak: kept at -55, dropped at -40.
        for v in quiet.iter_mut().skip(160 * 20).take(160 * 3) {
            *v = 30;
        }
        let loose = shape(&quiet, &RenderSettings::default()).unwrap().len();
        let tight = shape(
            &quiet,
            &RenderSettings {
                trim_db: -40.0,
                ..Default::default()
            },
        )
        .unwrap()
        .len();
        assert!(loose > tight, "{loose} {tight}");
    }

    #[test]
    fn tempo_and_rate_need_ffmpeg() {
        let p = Pcm {
            rate: 24000,
            samples: take_samples(),
        };
        let e = normalise(&p, &RenderSettings::default(), None).unwrap_err();
        assert!(e.to_string().contains("ffmpeg is needed"));
        assert!(normalise(
            &p,
            &RenderSettings {
                tempo: 3.0,
                ..Default::default()
            },
            None
        )
        .unwrap_err()
        .to_string()
        .contains("tempo"));
    }

    #[test]
    fn the_cache_key_ignores_post_processing_and_follows_the_rest() {
        let k =
            |text: &str, speed: f64, seed: u64| Cache::key("say", "Voice", "m", speed, text, seed);
        assert_eq!(k("a", 1.0, 1), k("a", 1.0, 1));
        assert_ne!(k("a", 1.0, 1), k("b", 1.0, 1));
        assert_ne!(k("a", 1.0, 1), k("a", 1.2, 1));
        assert_ne!(k("a", 1.0, 1), k("a", 1.0, 2));
        assert_ne!(
            Cache::key("say", "A", "m", 1.0, "a", 1),
            Cache::key("say", "B", "m", 1.0, "a", 1)
        );
    }
}
