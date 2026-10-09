//! The ElevenLabs provider (design 7.4): the model and voice lists, the credit balance, and
//! speech with character timestamps. All of it goes through `Http`, so tests answer with
//! canned JSON and never reach the network. The key rides in a header on curl's stdin, never
//! in argv, and no error or log line carries it.

use super::tts::{curl_quote, Aligned, Alignment, Credits, ModelInfo, Tts, TtsRequest, VoiceInfo};
use super::wav::Pcm;
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;

pub const API: &str = "https://api.elevenlabs.io";

/// The raw PCM format asked for: the rate every WAV QuadCam writes, so no resample follows.
pub const OUTPUT_FORMAT: &str = "pcm_32000";

/// One request.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    /// `GET` or `POST`.
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

/// An answer: the status and the body text (every call here answers JSON).
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

pub trait Http: Send + Sync {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse>;
}

/// `/usr/bin/curl` with its whole configuration on stdin. Under cargo it is closed unless
/// `QUADCAM_TTS=real`: the caller (`System::make`) checks that before it builds one.
pub struct CurlHttp;

impl Http for CurlHttp {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse> {
        if !req.url.starts_with("https://") {
            bail!("QuadCam talks to ElevenLabs only over HTTPS");
        }
        let mut cfg = format!("url = {}\nrequest = {}\n", curl_quote(&req.url), req.method);
        for (k, v) in &req.headers {
            cfg.push_str(&format!("header = {}\n", curl_quote(&format!("{k}: {v}"))));
        }
        if let Some(b) = &req.body {
            cfg.push_str(&format!("data = {}\n", curl_quote(b)));
        }
        cfg.push_str("max-time = 300\nsilent\nshow-error\nwrite-out = \"\\n%{http_code}\"\n");
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
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
        Ok(HttpResponse {
            status: code.trim().parse().unwrap_or(0),
            body: body.to_string(),
        })
    }
}

/// What ElevenLabs bills one character on a model that does not say. Flash and turbo v2.5
/// bill half a credit; everything else one.
pub fn fallback_rate(model: &str) -> f64 {
    if model == "eleven_turbo_v2_5" || model == "eleven_flash_v2_5" {
        0.5
    } else {
        1.0
    }
}

pub struct Eleven {
    http: Arc<dyn Http>,
    key: String,
    base: String,
    /// Milliseconds to wait before the first retry of a busy server; doubles each time.
    backoff_ms: u64,
}

impl Eleven {
    pub fn new(http: Arc<dyn Http>, key: String) -> Self {
        Self {
            http,
            key,
            base: API.into(),
            backoff_ms: 1500,
        }
    }

    /// Tests: no waiting between retries.
    pub fn without_backoff(mut self) -> Self {
        self.backoff_ms = 0;
        self
    }

    fn call(&self, method: &'static str, path: &str, body: Option<Value>) -> Result<Value> {
        let req = HttpRequest {
            method,
            url: format!("{}{path}", self.base),
            headers: vec![
                ("xi-api-key".into(), self.key.clone()),
                ("Content-Type".into(), "application/json".into()),
                ("Accept".into(), "application/json".into()),
            ],
            body: body.map(|b| b.to_string()),
        };
        let mut wait = self.backoff_ms;
        for attempt in 0..4 {
            let r = self.http.send(&req).context("ElevenLabs did not answer")?;
            if (200..300).contains(&r.status) {
                return serde_json::from_str(&r.body)
                    .map_err(|_| anyhow!("ElevenLabs answered something that is not JSON"));
            }
            let busy = r.status == 429 || r.status >= 500;
            if busy && attempt < 3 {
                if wait > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(wait));
                }
                wait *= 2;
                continue;
            }
            bail!("{}", self.explain(r.status, &r.body));
        }
        unreachable!("the loop returns or bails")
    }

    /// An error message that never carries the key.
    fn explain(&self, status: u16, body: &str) -> String {
        let detail: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        let d = detail.get("detail").unwrap_or(&Value::Null);
        let msg = d
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| d.as_str())
            .unwrap_or("")
            .replace(&self.key, "[key]");
        let what = match status {
            401 => "ElevenLabs does not accept this key (or the key lacks this permission)".into(),
            402 | 403 => "ElevenLabs refused the request".into(),
            404 => "ElevenLabs does not know that voice or model".into(),
            422 => "ElevenLabs rejected the request".into(),
            429 => "ElevenLabs is busy or rate-limited; try again later".into(),
            s => format!("ElevenLabs answered HTTP {s}"),
        };
        if msg.is_empty() {
            what
        } else {
            format!("{what}: {msg}")
        }
    }

    fn pcm(audio_base64: &str) -> Result<Pcm> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(audio_base64.trim())
            .map_err(|_| anyhow!("the audio ElevenLabs sent is not valid base64"))?;
        if bytes.len() < 2 {
            bail!("ElevenLabs sent no audio");
        }
        Ok(Pcm {
            rate: super::render::OUT_RATE,
            samples: bytes
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect(),
        })
    }
}

#[derive(Deserialize)]
struct WithTimestamps {
    audio_base64: String,
    alignment: Option<Alignment>,
}

/// A model's id, safe in a path or a body.
fn check_ident(what: &str, v: &str) -> Result<()> {
    if v.is_empty()
        || !v
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!("{v:?} is not a {what} id");
    }
    Ok(())
}

impl Tts for Eleven {
    fn id(&self) -> &str {
        "elevenlabs"
    }
    fn paid(&self) -> bool {
        true
    }

    fn render(&self, req: &TtsRequest<'_>) -> Result<Pcm> {
        Ok(self.render_aligned(req)?.pcm)
    }

    fn render_aligned(&self, req: &TtsRequest<'_>) -> Result<Aligned> {
        check_ident("voice", req.voice)?;
        let mut body = json!({"text": req.text});
        if !req.model.is_empty() {
            check_ident("model", req.model)?;
            body["model_id"] = req.model.into();
        }
        if req.seed != 0 {
            body["seed"] = req.seed.into();
        }
        // ElevenLabs takes speed from 0.7 to 1.2; 1.0 is the default and is not sent.
        if (req.speed - 1.0).abs() > 1e-9 {
            body["voice_settings"] = json!({"speed": req.speed.clamp(0.7, 1.2)});
        }
        let v = self.call(
            "POST",
            &format!(
                "/v1/text-to-speech/{}/with-timestamps?output_format={OUTPUT_FORMAT}",
                req.voice
            ),
            Some(body),
        )?;
        let r: WithTimestamps = serde_json::from_value(v)
            .map_err(|_| anyhow!("ElevenLabs answered without audio and timestamps"))?;
        let alignment = r
            .alignment
            .ok_or_else(|| anyhow!("ElevenLabs sent no timestamps for this take"))?;
        if alignment.characters.len() != alignment.starts.len()
            || alignment.characters.len() != alignment.ends.len()
        {
            bail!("the timestamps ElevenLabs sent do not line up");
        }
        Ok(Aligned {
            pcm: Self::pcm(&r.audio_base64)?,
            alignment,
        })
    }

    fn models(&self) -> Result<Vec<ModelInfo>> {
        let v = self.call("GET", "/v1/models", None)?;
        let list = v
            .as_array()
            .ok_or_else(|| anyhow!("ElevenLabs answered the model list in an unknown shape"))?;
        Ok(list
            .iter()
            .filter(|m| {
                m.get("can_do_text_to_speech")
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
            })
            .filter_map(|m| {
                let id = m.get("model_id")?.as_str()?.to_string();
                let rate = m
                    .pointer("/model_rates/character_cost_multiplier")
                    .and_then(Value::as_f64)
                    .unwrap_or_else(|| fallback_rate(&id));
                Some(ModelInfo {
                    name: m
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(&id)
                        .to_string(),
                    cost_per_char: rate,
                    max_chars: m
                        .get("maximum_text_length_per_request")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    id,
                })
            })
            .collect())
    }

    fn voices(&self) -> Result<Vec<VoiceInfo>> {
        let v = self.call("GET", "/v1/voices", None)?;
        let list = v
            .get("voices")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("ElevenLabs answered the voice list in an unknown shape"))?;
        Ok(list
            .iter()
            .filter_map(|x| {
                let labels = x
                    .get("labels")
                    .and_then(Value::as_object)
                    .map(|o| {
                        o.values()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                Some(VoiceInfo {
                    id: x.get("voice_id")?.as_str()?.to_string(),
                    name: x.get("name")?.as_str()?.to_string(),
                    category: x
                        .get("category")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    labels,
                    preview_url: x
                        .get("preview_url")
                        .and_then(Value::as_str)
                        .map(String::from),
                })
            })
            .collect())
    }

    fn credits(&self) -> Result<Credits> {
        let v = self.call("GET", "/v1/user/subscription", None)?;
        let n = |k: &str| v.get(k).and_then(Value::as_u64);
        let (Some(used), Some(limit)) = (n("character_count"), n("character_limit")) else {
            bail!("ElevenLabs answered the subscription in an unknown shape");
        };
        Ok(Credits {
            used,
            limit,
            remaining: limit.saturating_sub(used),
            tier: v
                .get("tier")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            resets_at: n("next_character_count_reset_unix").unwrap_or(0),
        })
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::sync::Mutex;

    /// Canned answers by URL path; records every request.
    #[derive(Default)]
    pub struct FakeHttp {
        pub answers: Mutex<Vec<(String, u16, String)>>,
        pub seen: Mutex<Vec<HttpRequest>>,
    }

    impl FakeHttp {
        pub fn on(&self, path_part: &str, status: u16, body: impl Into<String>) {
            self.answers
                .lock()
                .unwrap()
                .push((path_part.into(), status, body.into()));
        }
        pub fn posts(&self) -> usize {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.method == "POST")
                .count()
        }
    }

    impl Http for FakeHttp {
        fn send(&self, req: &HttpRequest) -> Result<HttpResponse> {
            self.seen.lock().unwrap().push(req.clone());
            let mut a = self.answers.lock().unwrap();
            let Some(i) = a.iter().position(|(p, _, _)| req.url.contains(p.as_str())) else {
                bail!("no canned answer for {}", req.url);
            };
            // The last answer for a path stays; earlier ones are used up in order.
            let (_, status, body) = if a
                .iter()
                .filter(|(p, _, _)| req.url.contains(p.as_str()))
                .count()
                > 1
            {
                a.remove(i)
            } else {
                a[i].clone()
            };
            Ok(HttpResponse { status, body })
        }
    }

    /// The with-timestamps answer for `text`: each character takes `step` seconds, and the
    /// audio is `loud` samples of +-4000 where a character is not a space.
    pub fn timestamps_json(text: &str, step: f64) -> String {
        let rate = super::super::render::OUT_RATE as f64;
        let n = text.chars().count();
        let total = (n as f64 * step * rate) as usize;
        let mut pcm = Vec::with_capacity(total * 2);
        for i in 0..total {
            let ch = text
                .chars()
                .nth((i as f64 / (step * rate)) as usize)
                .unwrap_or(' ');
            let v: i16 = if ch == ' ' || ch == '.' {
                0
            } else if (i / 16) % 2 == 0 {
                4000
            } else {
                -4000
            };
            pcm.extend_from_slice(&v.to_le_bytes());
        }
        let chars: Vec<String> = text.chars().map(String::from).collect();
        let starts: Vec<f64> = (0..n).map(|i| i as f64 * step).collect();
        let ends: Vec<f64> = (0..n).map(|i| (i + 1) as f64 * step).collect();
        json!({
            "audio_base64": base64::engine::general_purpose::STANDARD.encode(pcm),
            "alignment": {
                "characters": chars,
                "character_start_times_seconds": starts,
                "character_end_times_seconds": ends,
            }
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::fake::*;
    use super::*;

    fn client(h: &Arc<FakeHttp>) -> Eleven {
        Eleven::new(h.clone(), "sk_secret_key_1234".into()).without_backoff()
    }

    #[test]
    fn models_carry_their_billing_rate() {
        let h = Arc::new(FakeHttp::default());
        h.on(
            "/v1/models",
            200,
            r#"[{"model_id":"eleven_turbo_v2_5","name":"Turbo v2.5","can_do_text_to_speech":true,
                 "model_rates":{"character_cost_multiplier":0.5},"maximum_text_length_per_request":40000},
                {"model_id":"eleven_flash_v2_5","name":"Flash","can_do_text_to_speech":true},
                {"model_id":"scribe","name":"Scribe","can_do_text_to_speech":false}]"#,
        );
        let m = client(&h).models().unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].cost_per_char, 0.5);
        assert_eq!(m[0].max_chars, 40000);
        // No rate in the answer: the known half-price models still bill half.
        assert_eq!(m[1].cost_per_char, 0.5);
        let seen = h.seen.lock().unwrap();
        assert!(seen[0].headers.iter().any(|(k, _)| k == "xi-api-key"));
    }

    #[test]
    fn voices_list_with_their_labels() {
        let h = Arc::new(FakeHttp::default());
        h.on(
            "/v1/voices",
            200,
            r#"{"voices":[{"voice_id":"abc","name":"Callum","category":"premade",
                "labels":{"accent":"american","gender":"male"},"preview_url":"https://x/y.mp3"},
                {"voice_id":"def","name":"Own","category":"cloned"}]}"#,
        );
        let v = client(&h).voices().unwrap();
        assert_eq!(v[0].name, "Callum");
        assert_eq!(v[0].labels, "american, male");
        assert_eq!(v[1].category, "cloned");
    }

    #[test]
    fn credits_are_what_is_left() {
        let h = Arc::new(FakeHttp::default());
        h.on(
            "/v1/user/subscription",
            200,
            r#"{"tier":"creator","character_count":1500,"character_limit":100000,"next_character_count_reset_unix":1790000000}"#,
        );
        let c = client(&h).credits().unwrap();
        assert_eq!((c.used, c.limit, c.remaining), (1500, 100000, 98500));
        assert_eq!(c.tier, "creator");
        assert_eq!(c.resets_at, 1790000000);
    }

    #[test]
    fn a_take_comes_with_its_timestamps() {
        let h = Arc::new(FakeHttp::default());
        h.on(
            "with-timestamps",
            200,
            timestamps_json("The word is six.", 0.05),
        );
        let e = client(&h);
        let a = e
            .render_aligned(&TtsRequest {
                text: "The word is six.",
                voice: "abc",
                model: "eleven_turbo_v2_5",
                speed: 1.2,
                seed: 7,
            })
            .unwrap();
        assert_eq!(a.alignment.characters.len(), 16);
        assert!(!a.pcm.samples.is_empty());
        let seen = h.seen.lock().unwrap();
        let r = &seen[0];
        assert!(r.url.contains("/v1/text-to-speech/abc/with-timestamps"));
        assert!(r.url.contains("output_format=pcm_32000"));
        let body: Value = serde_json::from_str(r.body.as_ref().unwrap()).unwrap();
        assert_eq!(body["model_id"], "eleven_turbo_v2_5");
        assert_eq!(body["seed"], 7);
        assert_eq!(body["voice_settings"]["speed"], 1.2);
    }

    #[test]
    fn an_error_never_carries_the_key() {
        let h = Arc::new(FakeHttp::default());
        h.on(
            "/v1/models",
            401,
            r#"{"detail":{"status":"invalid_api_key","message":"bad key sk_secret_key_1234"}}"#,
        );
        let e = client(&h).models().unwrap_err().to_string();
        assert!(e.contains("does not accept this key"), "{e}");
        assert!(!e.contains("sk_secret_key_1234"), "{e}");
    }

    #[test]
    fn a_busy_server_is_retried_then_reported() {
        let h = Arc::new(FakeHttp::default());
        h.on("/v1/models", 429, "{}");
        h.on("/v1/models", 200, "[]");
        assert!(client(&h).models().unwrap().is_empty());
        assert_eq!(h.seen.lock().unwrap().len(), 2);
        let h = Arc::new(FakeHttp::default());
        h.on("/v1/models", 500, "{}");
        assert!(client(&h)
            .models()
            .unwrap_err()
            .to_string()
            .contains("HTTP 500"));
        assert_eq!(h.seen.lock().unwrap().len(), 4);
    }

    #[test]
    fn a_voice_id_stays_out_of_the_path() {
        let h = Arc::new(FakeHttp::default());
        let r = client(&h).render_aligned(&TtsRequest {
            text: "x",
            voice: "../../v1/user",
            model: "",
            speed: 1.0,
            seed: 0,
        });
        assert!(r.unwrap_err().to_string().contains("voice id"));
        assert!(h.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn mismatched_timestamps_are_refused() {
        let h = Arc::new(FakeHttp::default());
        h.on(
            "with-timestamps",
            200,
            r#"{"audio_base64":"AAAA","alignment":{"characters":["a","b"],"character_start_times_seconds":[0],"character_end_times_seconds":[0.1]}}"#,
        );
        let e = client(&h)
            .render_aligned(&TtsRequest {
                text: "ab",
                voice: "abc",
                model: "m",
                speed: 1.0,
                seed: 0,
            })
            .unwrap_err()
            .to_string();
        assert!(e.contains("do not line up"), "{e}");
    }
}
