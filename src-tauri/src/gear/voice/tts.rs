//! Text-to-speech providers (design 7.4). A provider turns spoken text into one take: mono
//! 16-bit samples at the provider's own rate. Two ship: macOS `say` (offline, free) and an
//! OpenAI-compatible `/v1/audio/speech` endpoint (a local Kokoro-FastAPI server, or any other
//! base URL). Both take their outside world through a trait (`Run`, `Post`), so tests render
//! with fakes and never reach a real voice or network. A key goes to curl on stdin, never in
//! argv, and never into a log.
//!
//! The render hook for batched carrier sentences ("The word is six.", cut out by word
//! timestamps) is not built. Its seam is `TtsRequest::text`: a later renderer can speak a
//! carrier and cut the line out before the take reaches `render::normalise`.

use super::wav::{self, Pcm};
use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// One line to speak.
#[derive(Debug, Clone)]
pub struct TtsRequest<'a> {
    /// The spoken text, after the spelling rules.
    pub text: &'a str,
    /// The provider's voice name or id; empty for its default.
    pub voice: &'a str,
    /// The provider's model; empty for its default.
    pub model: &'a str,
    /// Speaking speed, 1.0 normal.
    pub speed: f64,
    /// The take's seed. Providers that cannot fix one ignore it; it stays in the cache key.
    pub seed: u64,
}

/// A provider.
pub trait Tts: Send + Sync {
    /// `say` or `openai`: the folder of its raw takes in the cache.
    fn id(&self) -> &str;
    /// Whether a render costs money: the caller reports the character count first.
    fn paid(&self) -> bool;
    fn render(&self, req: &TtsRequest<'_>) -> Result<Pcm>;
}

/// What a provider needs from the settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderConfig {
    /// `say` or `openai`.
    pub provider: String,
    pub base_url: String,
    pub key: Option<String>,
}

/// Builds the provider a setting names.
pub trait Providers: Send + Sync {
    fn make(&self, cfg: &ProviderConfig) -> Result<Box<dyn Tts>>;
}

/// Runs a program with stdin; returns nothing (say writes a file).
pub trait Run: Send + Sync {
    fn run(&self, program: &str, args: &[String], stdin: &[u8]) -> Result<()>;
}

/// Sends one JSON POST; returns the body.
pub trait Post: Send + Sync {
    fn post(&self, url: &str, key: Option<&str>, body: &str) -> Result<Vec<u8>>;
}

// ----- say -----

/// macOS `say`, writing a WAV file in a scratch folder.
pub struct Say {
    pub run: Arc<dyn Run>,
    pub dir: PathBuf,
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The rate `say` renders at; the pipeline's own rate, so no resample follows.
pub const SAY_RATE: u32 = 32000;

impl Tts for Say {
    fn id(&self) -> &str {
        "say"
    }
    fn paid(&self) -> bool {
        false
    }
    fn render(&self, req: &TtsRequest<'_>) -> Result<Pcm> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating {}", self.dir.display()))?;
        let out = self.dir.join(format!(
            "say-{}-{}.wav",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let mut args = Vec::new();
        if !req.voice.is_empty() {
            args.push("-v".to_string());
            args.push(req.voice.to_string());
        }
        // `say` speaks 175 words a minute by default.
        args.push("-r".into());
        args.push(
            ((175.0 * req.speed).round() as i64)
                .clamp(50, 600)
                .to_string(),
        );
        args.extend([
            "-o".to_string(),
            out.display().to_string(),
            "--file-format=WAVE".into(),
            format!("--data-format=LEI16@{SAY_RATE}"),
            "-f".into(),
            "-".into(),
        ]);
        let r = self.run.run("/usr/bin/say", &args, req.text.as_bytes());
        let bytes = std::fs::read(&out);
        let _ = std::fs::remove_file(&out);
        r.context("say failed")?;
        wav::read(&bytes.context("say wrote no file")?)
    }
}

/// Runs a real program.
pub struct RealRun;

impl Run for RealRun {
    fn run(&self, program: &str, args: &[String], stdin: &[u8]) -> Result<()> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("starting {program}"))?;
        child.stdin.take().expect("piped").write_all(stdin)?;
        let out = child.wait_with_output()?;
        if !out.status.success() {
            bail!(
                "{program} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }
}

// ----- OpenAI-compatible -----

/// An OpenAI-compatible `/v1/audio/speech` endpoint.
pub struct OpenAi {
    pub post: Arc<dyn Post>,
    pub base_url: String,
    pub key: Option<String>,
}

/// The speech URL for a base URL (`http://host:8880` or `http://host:8880/v1`).
pub fn speech_url(base: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    if b.ends_with("/v1") {
        format!("{b}/audio/speech")
    } else {
        format!("{b}/v1/audio/speech")
    }
}

impl Tts for OpenAi {
    fn id(&self) -> &str {
        "openai"
    }
    fn paid(&self) -> bool {
        // A server on this Mac costs nothing; anything else may.
        !crate::modules::fetch::is_loopback(&self.base_url)
    }
    fn render(&self, req: &TtsRequest<'_>) -> Result<Pcm> {
        let mut body = serde_json::json!({
            "input": req.text,
            "response_format": "wav",
            "speed": req.speed,
        });
        if !req.model.is_empty() {
            body["model"] = req.model.into();
        }
        if !req.voice.is_empty() {
            body["voice"] = req.voice.into();
        }
        let bytes = self
            .post
            .post(
                &speech_url(&self.base_url),
                self.key.as_deref(),
                &body.to_string(),
            )
            .context("the voice server did not answer")?;
        wav::read(&bytes).context("the voice server did not return a WAV")
    }
}

/// Escapes a value for a curl config file's double-quoted string.
fn curl_quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// `/usr/bin/curl` with its whole configuration on stdin: the URL, the key and the body
/// stay out of argv. Under cargo it reaches only loopback unless `QUADCAM_FETCH=real`.
pub struct CurlPost {
    pub loopback_only: bool,
}

impl Post for CurlPost {
    fn post(&self, url: &str, key: Option<&str>, body: &str) -> Result<Vec<u8>> {
        if self.loopback_only && !crate::modules::fetch::is_loopback(url) {
            bail!("voice servers other than this Mac are off in tests (QUADCAM_FETCH=real turns them on): {url}");
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            bail!("{url:?} is not an http or https address");
        }
        let mut cfg = format!(
            "url = {}\nheader = \"Content-Type: application/json\"\n",
            curl_quote(url)
        );
        if let Some(k) = key.filter(|k| !k.is_empty()) {
            cfg.push_str(&format!(
                "header = {}\n",
                curl_quote(&format!("Authorization: Bearer {k}"))
            ));
        }
        cfg.push_str(&format!("data = {}\n", curl_quote(body)));
        cfg.push_str("max-time = 120\nsilent\nshow-error\nfail\n");
        let mut child = Command::new("/usr/bin/curl")
            .args(["-K", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("starting curl")?;
        let mut stdin = child.stdin.take().expect("piped");
        let writer = std::thread::spawn(move || stdin.write_all(cfg.as_bytes()));
        let out = child.wait_with_output()?;
        let _ = writer.join();
        if !out.status.success() {
            bail!(
                "curl failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(out.stdout)
    }
}

// ----- the providers a setting names -----

/// The real providers. Under cargo they are off unless `QUADCAM_TTS=real`, except a server
/// on this Mac.
pub struct System {
    pub scratch: PathBuf,
}

impl Providers for System {
    fn make(&self, cfg: &ProviderConfig) -> Result<Box<dyn Tts>> {
        let under_cargo = std::env::var_os("CARGO_MANIFEST_DIR").is_some()
            && std::env::var("QUADCAM_TTS").as_deref() != Ok("real");
        match cfg.provider.as_str() {
            "say" | "" => {
                if under_cargo {
                    bail!("the say voice is off in tests (QUADCAM_TTS=real turns it on)");
                }
                Ok(Box::new(Say {
                    run: Arc::new(RealRun),
                    dir: self.scratch.clone(),
                }))
            }
            "openai" => {
                if cfg.base_url.trim().is_empty() {
                    bail!("the openai voice needs a server: set tts_base_url (for example http://127.0.0.1:8880)");
                }
                Ok(Box::new(OpenAi {
                    post: Arc::new(CurlPost {
                        loopback_only: std::env::var_os("CARGO_MANIFEST_DIR").is_some()
                            && std::env::var("QUADCAM_FETCH").as_deref() != Ok("real"),
                    }),
                    base_url: cfg.base_url.clone(),
                    key: cfg.key.clone(),
                }))
            }
            other => bail!("{other:?} is not a voice provider here: use say or openai"),
        }
    }
}

/// Providers for tests that name none: every `make` says so.
pub struct NoProviders;

impl Providers for NoProviders {
    fn make(&self, _cfg: &ProviderConfig) -> Result<Box<dyn Tts>> {
        bail!("no voice provider is available here")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_speech_url() {
        assert_eq!(
            speech_url("http://127.0.0.1:8880"),
            "http://127.0.0.1:8880/v1/audio/speech"
        );
        assert_eq!(speech_url("http://h:1/v1/"), "http://h:1/v1/audio/speech");
    }

    #[test]
    fn curl_config_quoting() {
        assert_eq!(curl_quote("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
    }

    #[test]
    fn a_server_on_this_mac_is_free() {
        struct Never;
        impl Post for Never {
            fn post(&self, _: &str, _: Option<&str>, _: &str) -> Result<Vec<u8>> {
                unreachable!()
            }
        }
        let t = |u: &str| OpenAi {
            post: Arc::new(Never),
            base_url: u.into(),
            key: None,
        };
        assert!(!t("http://127.0.0.1:8880").paid());
        assert!(!t("http://localhost:8880/v1").paid());
        assert!(t("https://voice.example.com").paid());
    }
}
