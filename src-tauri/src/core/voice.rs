//! Radio voice on the core (design 7.4, WP9): what the Voice segment, the CLI and MCP drive.
//! `gear_voice` reads the lines and packs; `gear_voice_render` renders QuadCam's lines (and a
//! person's own) with their provider into a local pack; `gear_voice_pack_install` installs a
//! pack from the index; `gear_voice_edit` overrides one line on one radio;
//! `gear_voice_choose` stages one card change that puts a pack's sounds on a radio.
//! `voice_build_pack` is the maintainer's tool and has no API row. Nothing here writes a
//! card: the apply sheet does.

use super::Core;
use crate::gear::blobs::BlobRef;
use crate::gear::changes::ChangeFilter;
use crate::gear::model::{CardFile, ChangeStatus, DeviceKind, Edit, StagedChange};
use crate::gear::voice::lines::{self, Line, Spelling};
use crate::gear::voice::packs::{self, BuildOpts, Installed, PackIndexEntry, VoiceIndex};
use crate::gear::voice::render::{self, Cache, Ctx, Plan, RenderSettings};
use crate::gear::voice::tts::{ProviderConfig, Tts};
use crate::modules::fetch::Fetch;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The title prefix of the change Choose voice keeps.
pub const VOICE_CHANGE: &str = "Voice: ";

// ----- parameters and answers -----

/// `gear_voice`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct VoiceParams {
    /// A saved radio: shows its overrides and the voice chosen for it.
    #[serde(default)]
    pub radio: Option<String>,
    /// Read the pack index again (the only call that goes to the network).
    #[serde(default)]
    pub refresh_index: bool,
}

/// How a line differs on one radio.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct LineOverride {
    /// `pack` (another pack's take) or `text` (a take of the person's own text).
    pub kind: String,
    #[serde(default)]
    pub pack: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct VoiceLine {
    pub path: String,
    pub text: String,
    /// What a voice speaks, after the spelling rules.
    pub spoken: String,
    pub group: String,
    pub why: String,
    /// The person's own line (in `gear.json`, never in a pack).
    pub custom: bool,
    /// Installed packs that hold this line.
    pub packs: Vec<String>,
    #[serde(default, rename = "override")]
    pub override_: Option<LineOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct VoicePack {
    pub id: String,
    pub voice: String,
    pub lang: String,
    pub provider: String,
    pub model: String,
    pub lines: u32,
    pub license: String,
    pub attribution: String,
    pub version: String,
    pub installed: bool,
    /// Rendered here, with the person's own provider.
    pub local: bool,
    pub bytes: u64,
    /// The pack was rendered from other lines than this version's `lines.csv`.
    pub stale: bool,
    /// Where an installed pack's files are (the app plays them from here).
    #[serde(default)]
    pub dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ProviderView {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub voice: String,
    /// A render costs money (a server that is not on this Mac).
    pub paid: bool,
    pub key_set: bool,
    /// The provider can render now; else why not.
    pub ready: bool,
    #[serde(default)]
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct VoiceView {
    pub provider: ProviderView,
    pub lines: Vec<VoiceLine>,
    pub packs: Vec<VoicePack>,
    /// The pack chosen for the radio (staged or applied).
    #[serde(default)]
    pub chosen: Option<String>,
    /// Where the index is read from, when a setting names one.
    #[serde(default)]
    pub index_source: Option<String>,
    pub notes: Vec<String>,
}

/// `gear_voice_edit`: override one line on one radio. `pack` takes another installed pack's
/// take; `text` renders the person's own text with their provider; neither removes the
/// override.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct VoiceEditParams {
    pub radio: String,
    /// The sound's card path (`SOUNDS/en/armed.wav`). A path QuadCam has no line for is a
    /// custom line.
    pub line: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub pack: Option<String>,
    /// Allows a render that costs money.
    #[serde(default)]
    pub confirm: bool,
}

/// `gear_voice_render`: QuadCam's lines, and the person's own, rendered with the provider
/// from the settings into a local pack.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct VoiceRenderParams {
    /// The provider's voice; empty for the setting `tts_voice`.
    #[serde(default)]
    pub voice: String,
    /// Card paths to render; empty for every line.
    #[serde(default)]
    pub lines: Vec<String>,
    /// Report what the render would do and cost; render nothing.
    #[serde(default)]
    pub dry_run: bool,
    /// Allows a render that costs money.
    #[serde(default)]
    pub confirm: bool,
    /// How it is rendered; the defaults when left out.
    #[serde(default)]
    pub settings: Option<RenderSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RenderReport {
    /// The local pack the sounds went into.
    pub pack: String,
    pub provider: String,
    pub voice: String,
    pub plan: Plan,
    /// Takes the provider made.
    pub rendered: u32,
    pub from_cache: u32,
    pub paid: bool,
    /// The render costs money and was not confirmed: nothing was rendered.
    pub needs_confirm: bool,
    pub dry_run: bool,
    pub notes: Vec<String>,
}

/// `gear_voice_preview`: a sound to play.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct VoicePreviewParams {
    /// The card path of the line.
    pub line: String,
    /// An installed pack's take of it.
    #[serde(default)]
    pub pack: Option<String>,
    /// Without a pack: the person's own render for the line on this radio.
    #[serde(default)]
    pub radio: Option<String>,
}

/// `gear_voice_pack_install`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct PackInstallParams {
    /// A pack id from the index.
    pub pack: String,
    /// The index (a path or an address) when the setting `voice_index` names none.
    #[serde(default)]
    pub source: Option<String>,
}

/// `gear_voice_choose`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct VoiceChooseParams {
    pub radio: String,
    pub pack: String,
    /// Keep the lines the person rendered or picked one by one (default true).
    #[serde(default = "yes")]
    pub keep_overrides: bool,
    #[serde(default)]
    pub editor: Option<crate::session::Editor>,
}

fn yes() -> bool {
    true
}

/// The maintainer's `build-pack`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildPackParams {
    pub voice: String,
    /// The pack id; default `<lang>-<voice>-v1`.
    pub id: Option<String>,
    pub version: Option<String>,
    pub lang: Option<String>,
    pub out: PathBuf,
    pub license: Option<String>,
    pub attribution: Option<String>,
    pub settings: Option<RenderSettings>,
    pub confirm: bool,
    pub dry_run: bool,
}

// ----- the person's data in gear.json -----

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Override {
    kind: String,
    #[serde(default)]
    pack: Option<String>,
    #[serde(default)]
    text: Option<String>,
    /// The normalised WAV of a `text` override.
    #[serde(default)]
    blob: Option<BlobRef>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Custom {
    path: String,
    text: String,
}

fn voice_obj(v: &mut crate::gear::store::Values) -> Result<&mut serde_json::Map<String, Value>> {
    match v.entry("voice").or_insert_with(|| json!({})) {
        Value::Object(m) => Ok(m),
        _ => bail!("gear.json: voice is not an object; fix or remove it"),
    }
}

fn sub<'a>(
    m: &'a mut serde_json::Map<String, Value>,
    k: &str,
) -> Result<&'a mut serde_json::Map<String, Value>> {
    match m.entry(k).or_insert_with(|| json!({})) {
        Value::Object(o) => Ok(o),
        _ => bail!("gear.json: voice.{k} is not an object; fix or remove it"),
    }
}

impl Core {
    fn voice_scratch(&self) -> PathBuf {
        self.cache.join("voice").join("tmp")
    }

    fn voice_cache(&self) -> Cache {
        Cache::new(&self.cache)
    }

    fn voices_dir(&self) -> PathBuf {
        self.gear_store().voices_dir()
    }

    /// The provider setting, and why it cannot render, if it cannot.
    fn voice_config(&self) -> (ProviderConfig, String, String) {
        let values = self
            .settings_file
            .as_deref()
            .map(|f| crate::settings::read(f).unwrap_or_default())
            .unwrap_or_default();
        let s = |k: &str| {
            values
                .get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("")
                .to_string()
        };
        let provider = match s("ttsProvider").as_str() {
            "" => "say".to_string(),
            p => p.to_string(),
        };
        let key = std::env::var("QUADCAM_TTS_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .or_else(|| Some(s("ttsKey")).filter(|k| !k.is_empty()));
        (
            ProviderConfig {
                provider,
                base_url: s("ttsBaseUrl"),
                key,
            },
            s("ttsModel"),
            s("ttsVoice"),
        )
    }

    fn voice_provider(&self) -> Result<(Box<dyn Tts>, String, String)> {
        let (cfg, model, voice) = self.voice_config();
        Ok((self.gear.tts.make(&cfg)?, model, voice))
    }

    fn voice_index_source(&self) -> Option<String> {
        let values = self
            .settings_file
            .as_deref()
            .map(|f| crate::settings::read(f).unwrap_or_default())
            .unwrap_or_default();
        values
            .get("voiceIndex")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    }

    fn all_lines(&self) -> Result<Vec<Line>> {
        let mut out = lines::builtin()?;
        for c in self.custom_lines()? {
            if !out.iter().any(|l| l.path == c.path) {
                out.push(Line {
                    path: c.path,
                    text: c.text,
                    group: "extras".into(),
                    why: "Your own line.".into(),
                });
            }
        }
        Ok(out)
    }

    fn custom_lines(&self) -> Result<Vec<Custom>> {
        let v = self.gear_store().read()?;
        Ok(v.get("voice")
            .and_then(|x| x.get("custom_lines"))
            .and_then(|x| serde_json::from_value(x.clone()).ok())
            .unwrap_or_default())
    }

    fn overrides_of(&self, radio: &str) -> Result<std::collections::BTreeMap<String, Override>> {
        let v = self.gear_store().read()?;
        Ok(v.get("voice")
            .and_then(|x| x.get("overrides"))
            .and_then(|x| x.get(radio))
            .and_then(|x| serde_json::from_value(x.clone()).ok())
            .unwrap_or_default())
    }

    fn chosen_of(&self, radio: &str) -> Option<String> {
        let v = self.gear_store().read().ok()?;
        v.get("voice")?
            .get("chosen")?
            .get(radio)?
            .as_str()
            .map(String::from)
    }

    fn radio_device(&self, id: &str) -> Result<()> {
        match self.gear_store().device(id)? {
            Some(d) if d.kind == DeviceKind::Radio => Ok(()),
            Some(_) => bail!("{id:?} is not a radio."),
            None => bail!("No device {id:?} in QuadCam's list (quadcam-cli gear devices)."),
        }
    }

    // ----- reading -----

    fn index_cache_file(&self) -> PathBuf {
        self.cache.join("voice").join("index.json")
    }

    fn cached_index(&self) -> VoiceIndex {
        packs::read_index(&self.index_cache_file()).unwrap_or_default()
    }

    /// Reads the pack index from its source into the cache. A path reads the file; an
    /// address downloads it.
    fn refresh_index(&self, source: &str) -> Result<VoiceIndex> {
        let dest = self.index_cache_file();
        std::fs::create_dir_all(dest.parent().unwrap())?;
        if source.starts_with("http://") || source.starts_with("https://") {
            self.voice_fetch().download(source, &dest)?;
        } else {
            std::fs::copy(source, &dest).with_context(|| format!("reading {source}"))?;
        }
        packs::read_index(&dest)
    }

    fn voice_fetch(&self) -> crate::modules::fetch::Curl {
        crate::modules::fetch::Curl {
            loopback_only: std::env::var_os("CARGO_MANIFEST_DIR").is_some()
                && std::env::var("QUADCAM_FETCH").as_deref() != Ok("real"),
        }
    }

    fn provider_view(&self) -> ProviderView {
        let (cfg, model, voice) = self.voice_config();
        let made = self.gear.tts.make(&cfg);
        ProviderView {
            provider: cfg.provider.clone(),
            base_url: cfg.base_url.clone(),
            model,
            voice,
            paid: made.as_ref().map(|t| t.paid()).unwrap_or(false),
            key_set: cfg.key.is_some(),
            ready: made.is_ok(),
            problem: made.err().map(|e| format!("{e:#}")),
        }
    }

    /// The lines, the packs and the provider. Reads only; `refresh_index` also reads the
    /// pack index from its source.
    pub fn gear_voice(&self, p: &VoiceParams) -> Result<VoiceView> {
        let spelling = Spelling::builtin()?;
        let mut notes = Vec::new();
        let source = self.voice_index_source();
        let index = match (p.refresh_index, &source) {
            (true, Some(s)) => self.refresh_index(s)?,
            (true, None) => {
                notes.push("No pack index is set: set voice_index to read one.".into());
                self.cached_index()
            }
            _ => self.cached_index(),
        };
        let installed = packs::installed(&self.voices_dir());
        let csv_sha = lines::builtin_sha();
        let mut packs_out: Vec<VoicePack> = installed
            .iter()
            .map(|i| {
                let m = &i.manifest;
                VoicePack {
                    id: m.id.clone(),
                    voice: m.voice.clone(),
                    lang: m.lang.clone(),
                    provider: m.provider.clone(),
                    model: m.model.clone(),
                    lines: m.lines,
                    license: m.license.clone(),
                    attribution: m.attribution.clone(),
                    version: m.version.clone(),
                    installed: true,
                    local: m.id.starts_with("local-"),
                    bytes: index
                        .packs
                        .iter()
                        .find(|e| e.id == m.id)
                        .map(|e| e.bytes)
                        .unwrap_or(0),
                    stale: !m.lines_csv_sha.is_empty() && m.lines_csv_sha != csv_sha,
                    dir: Some(i.dir.clone()),
                }
            })
            .collect();
        for e in &index.packs {
            if !packs_out.iter().any(|p| p.id == e.id) {
                packs_out.push(VoicePack {
                    id: e.id.clone(),
                    voice: e.voice.clone(),
                    lang: e.lang.clone(),
                    provider: e.provider.clone(),
                    model: e.model.clone(),
                    lines: e.lines,
                    license: e.license.clone(),
                    attribution: e.attribution.clone(),
                    version: e.version.clone(),
                    installed: false,
                    local: false,
                    bytes: e.bytes,
                    stale: e.lines_csv_sha != csv_sha,
                    dir: None,
                });
            }
        }
        let overrides = match &p.radio {
            Some(r) => self.overrides_of(r)?,
            None => Default::default(),
        };
        let builtin: Vec<String> = lines::builtin()?.into_iter().map(|l| l.path).collect();
        let lines_out = self
            .all_lines()?
            .into_iter()
            .map(|l| VoiceLine {
                spoken: spelling.apply(&l.text),
                custom: !builtin.contains(&l.path),
                packs: installed
                    .iter()
                    .filter(|i| i.manifest.files.contains(&l.path))
                    .map(|i| i.manifest.id.clone())
                    .collect(),
                override_: overrides.get(&l.path).map(|o| LineOverride {
                    kind: o.kind.clone(),
                    pack: o.pack.clone(),
                    text: o.text.clone(),
                }),
                path: l.path,
                text: l.text,
                group: l.group,
                why: l.why,
            })
            .collect();
        Ok(VoiceView {
            provider: self.provider_view(),
            lines: lines_out,
            packs: packs_out,
            chosen: p.radio.as_deref().and_then(|r| self.chosen_of(r)),
            index_source: source,
            notes,
        })
    }

    // ----- rendering -----

    fn render_ctx<'a>(
        &self,
        tts: &'a dyn Tts,
        cache: &'a Cache,
        voice: &'a str,
        model: &'a str,
        settings: &'a RenderSettings,
        tools: Option<&'a crate::media::Tools>,
    ) -> Ctx<'a> {
        Ctx {
            tts,
            cache,
            voice,
            model,
            settings,
            tools,
            cuts: None,
        }
    }

    /// Renders QuadCam's lines, and the person's own, with the provider from the settings,
    /// into the local pack for that voice. A render that costs money waits for `confirm`;
    /// `dry_run` only reports.
    pub fn gear_voice_render(&self, p: &VoiceRenderParams) -> Result<RenderReport> {
        let (tts, model, setting_voice) = self.voice_provider()?;
        let voice = if p.voice.trim().is_empty() {
            setting_voice
        } else {
            p.voice.trim().to_string()
        };
        let settings = p.settings.clone().unwrap_or_default();
        settings.check()?;
        let all = self.all_lines()?;
        let chosen: Vec<Line> = if p.lines.is_empty() {
            all
        } else {
            let mut v = Vec::new();
            for path in &p.lines {
                v.push(
                    all.iter()
                        .find(|l| &l.path == path)
                        .cloned()
                        .ok_or_else(|| anyhow!("No line {path:?}: see quadcam-cli gear voice."))?,
                );
            }
            v
        };
        let spelling = Spelling::builtin()?;
        let cache = self.voice_cache();
        let tools = crate::media::find_tools().ok();
        let ctx = self.render_ctx(
            tts.as_ref(),
            &cache,
            &voice,
            &model,
            &settings,
            tools.as_ref(),
        );
        let spoken: Vec<String> = chosen.iter().map(|l| spelling.apply(&l.text)).collect();
        let plan = render::plan(&ctx, &spoken);
        let id = format!("local-{}-{}", tts.id(), slug(&voice));
        let paid = tts.paid();
        let mut report = RenderReport {
            pack: id.clone(),
            provider: tts.id().into(),
            voice: voice.clone(),
            plan: plan.clone(),
            rendered: 0,
            from_cache: plan.cached,
            paid,
            needs_confirm: false,
            dry_run: p.dry_run,
            notes: Vec::new(),
        };
        if paid && plan.to_render > 0 {
            report.notes.push(format!(
                "{} characters to render with a provider that may charge for them.",
                plan.chars
            ));
            if !p.confirm && !p.dry_run {
                report.needs_confirm = true;
                return Ok(report);
            }
        }
        if p.dry_run {
            return Ok(report);
        }
        let dir = self.voices_dir().join(&id);
        let stage = self.voices_dir().join(format!(".rendering-{id}"));
        let _ = std::fs::remove_dir_all(&stage);
        // A local pack keeps what an earlier render made, so a partial render adds to it.
        let (manifest, made) = {
            let opts = BuildOpts {
                id: id.clone(),
                version: String::new(),
                voice: voice.clone(),
                lang: "en".into(),
                license: "Rendered by you; yours to use.".into(),
                attribution: String::new(),
                lines_csv_sha: lines::builtin_sha(),
            };
            std::fs::create_dir_all(&stage)?;
            if dir.is_dir() && !p.lines.is_empty() {
                copy_dir(&dir, &stage)?;
            }
            let done = packs::render_to(&opts, &ctx, &chosen, &spelling, &stage, &mut |_, _| {});
            match done {
                Ok(v) => v,
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&stage);
                    return Err(e);
                }
            }
        };
        // `pack.json` lists every file in the stage, the earlier ones included.
        let mut m = manifest;
        let mut files: Vec<String> = Vec::new();
        collect_sounds(&stage, &stage, &mut files);
        files.sort();
        m.lines = files.len() as u32;
        m.files = files;
        std::fs::write(stage.join("pack.json"), serde_json::to_vec_pretty(&m)?)?;
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::rename(&stage, &dir)?;
        report.rendered = made;
        report.from_cache = plan.lines.saturating_sub(made);
        Ok(report)
    }

    /// The maintainer's tool: renders QuadCam's lines (never a person's own) into a zip and
    /// an index entry in `out`.
    pub fn voice_build_pack(&self, p: &BuildPackParams) -> Result<(PathBuf, PackIndexEntry, Plan)> {
        let (tts, model, _) = self.voice_provider()?;
        let settings = p.settings.clone().unwrap_or_default();
        settings.check()?;
        let lang = p.lang.clone().unwrap_or_else(|| "en".into());
        let id =
            p.id.clone()
                .unwrap_or_else(|| format!("{lang}-{}-v1", slug(&p.voice)));
        let all = lines::builtin()?;
        let spelling = Spelling::builtin()?;
        let cache = self.voice_cache();
        let tools = crate::media::find_tools().ok();
        let ctx = self.render_ctx(
            tts.as_ref(),
            &cache,
            &p.voice,
            &model,
            &settings,
            tools.as_ref(),
        );
        let prefix = format!("SOUNDS/{lang}/");
        let spoken: Vec<String> = all
            .iter()
            .filter(|l| l.path.starts_with(&prefix))
            .map(|l| spelling.apply(&l.text))
            .collect();
        let plan = render::plan(&ctx, &spoken);
        if tts.paid() && plan.to_render > 0 && !p.confirm {
            bail!(
                "This render sends {} characters to a provider that may charge for them. Check the plan, then pass confirm.",
                plan.chars
            );
        }
        if p.dry_run {
            return Ok((p.out.clone(), dummy_entry(&id), plan));
        }
        let opts = BuildOpts {
            id,
            version: p.version.clone().unwrap_or_else(|| "1".into()),
            voice: p.voice.clone(),
            lang,
            license: p.license.clone().unwrap_or_default(),
            attribution: p.attribution.clone().unwrap_or_default(),
            lines_csv_sha: lines::builtin_sha(),
        };
        let work = self.voice_scratch();
        std::fs::create_dir_all(&work)?;
        let (zip, entry) =
            packs::build(&opts, &ctx, &all, &spelling, &p.out, &work, &mut |_, _| {})?;
        Ok((zip, entry, plan))
    }

    // ----- packs -----

    /// Installs a pack from the index: the zip is fetched next to the index, its hash is
    /// checked against the index, and it unpacks into the gear folder.
    pub fn gear_voice_pack_install(&self, p: &PackInstallParams) -> Result<VoicePack> {
        let source = p
            .source
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.voice_index_source())
            .ok_or_else(|| {
                anyhow!("No pack index: set voice_index, or pass the index's path or address.")
            })?;
        let index = self.refresh_index(&source)?;
        let entry = index
            .packs
            .iter()
            .find(|e| e.id == p.pack)
            .ok_or_else(|| {
                anyhow!(
                    "The index lists no pack {:?}: {}",
                    p.pack,
                    index
                        .packs
                        .iter()
                        .map(|e| e.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?
            .clone();
        let name = if entry.file.is_empty() {
            format!("voice-{}-{}.zip", entry.id, entry.version)
        } else {
            entry.file.clone()
        };
        if name.contains('/') || name.contains("..") {
            bail!("The index names the file {name:?}, which is not a plain file name.");
        }
        let tmp = self.voice_scratch();
        std::fs::create_dir_all(&tmp)?;
        let zip = tmp.join(&name);
        if source.starts_with("http://") || source.starts_with("https://") {
            let base = source.rsplit_once('/').map(|(b, _)| b).unwrap_or("");
            self.voice_fetch()
                .download(&format!("{base}/{name}"), &zip)?;
        } else {
            let from = Path::new(&source)
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(&name);
            std::fs::copy(&from, &zip).with_context(|| format!("reading {}", from.display()))?;
        }
        let r = packs::install(&self.voices_dir(), &zip, &entry);
        let _ = std::fs::remove_file(&zip);
        let inst = r?;
        let m = inst.manifest;
        Ok(VoicePack {
            id: m.id,
            voice: m.voice,
            lang: m.lang,
            provider: m.provider,
            model: m.model,
            lines: m.lines,
            license: m.license,
            attribution: m.attribution,
            version: m.version,
            installed: true,
            local: false,
            bytes: entry.bytes,
            stale: m.lines_csv_sha != lines::builtin_sha(),
            dir: Some(inst.dir),
        })
    }

    // ----- one line, one radio -----

    /// Overrides one line on one radio. `pack` takes another installed pack's take of the
    /// line; `text` renders the person's own text with their provider; neither removes the
    /// override. The override stays in `gear.json` and survives Choose voice when asked.
    pub fn gear_voice_edit(&self, p: &VoiceEditParams) -> Result<VoiceLine> {
        self.radio_device(&p.radio)?;
        lines::check_path(&p.line)?;
        if p.text.is_some() && p.pack.is_some() {
            bail!("Pass a pack or a text for a line, not both.");
        }
        let spelling = Spelling::builtin()?;
        let all = self.all_lines()?;
        let known = all.iter().find(|l| l.path == p.line).cloned();
        let store = self.gear_store();
        let blobs = self.snapshots().blobs();
        let entry: Option<Override> = match (&p.text, &p.pack) {
            (None, None) => None,
            (None, Some(pack)) => {
                let inst = packs::installed(&self.voices_dir())
                    .into_iter()
                    .find(|i| &i.manifest.id == pack)
                    .ok_or_else(|| anyhow!("Pack {pack:?} is not installed."))?;
                if !inst.manifest.files.contains(&p.line) {
                    bail!("Pack {pack:?} has no take of {}.", p.line);
                }
                Some(Override {
                    kind: "pack".into(),
                    pack: Some(pack.clone()),
                    text: None,
                    blob: None,
                })
            }
            (Some(text), None) => {
                if text.trim().is_empty() {
                    bail!("The text is empty.");
                }
                let (tts, model, voice) = self.voice_provider()?;
                let settings = RenderSettings::default();
                let cache = self.voice_cache();
                let tools = crate::media::find_tools().ok();
                let ctx = self.render_ctx(
                    tts.as_ref(),
                    &cache,
                    &voice,
                    &model,
                    &settings,
                    tools.as_ref(),
                );
                let spoken = spelling.apply(text);
                if tts.paid() && !render::cached(&ctx, &spoken) && !p.confirm {
                    bail!(
                        "This render sends {} characters to a provider that may charge for them. Pass confirm to go ahead.",
                        spoken.chars().count()
                    );
                }
                let (wav, _) = render::render_line(&ctx, &spoken)?;
                let blob = blobs.put(&wav)?;
                Some(Override {
                    kind: "text".into(),
                    pack: None,
                    text: Some(text.clone()),
                    blob: Some(blob),
                })
            }
            (Some(_), Some(_)) => unreachable!(),
        };
        let custom_new = known.is_none() && p.text.is_some();
        if known.is_none() && !custom_new {
            bail!(
                "QuadCam has no line {}: give the text to add it as your own line.",
                p.line
            );
        }
        store.update(|v| {
            let voice = voice_obj(v)?;
            if custom_new {
                let list = voice.entry("custom_lines").or_insert_with(|| json!([]));
                let Value::Array(a) = list else {
                    bail!("gear.json: voice.custom_lines is not a list");
                };
                a.retain(|c| c.get("path").and_then(Value::as_str) != Some(p.line.as_str()));
                a.push(json!({"path": p.line, "text": p.text}));
            }
            let per = sub(sub(voice, "overrides")?, &p.radio)?;
            match &entry {
                Some(e) => {
                    per.insert(p.line.clone(), serde_json::to_value(e)?);
                }
                None => {
                    per.remove(&p.line);
                }
            }
            Ok(())
        })?;
        let view = self.gear_voice(&VoiceParams {
            radio: Some(p.radio.clone()),
            refresh_index: false,
        })?;
        view.lines
            .into_iter()
            .find(|l| l.path == p.line)
            .ok_or_else(|| anyhow!("The line is gone."))
    }

    /// A sound the app can play: a pack's take, or the person's own render for a line, copied
    /// into the cache (the app plays files from there). Returns the file's path.
    pub fn gear_voice_preview(&self, p: &VoicePreviewParams) -> Result<String> {
        lines::check_path(&p.line)?;
        let dir = self.cache.join("voice").join("preview");
        std::fs::create_dir_all(&dir)?;
        let name = |tag: &str| {
            format!(
                "{}-{}.wav",
                slug(tag),
                slug(
                    p.line
                        .trim_start_matches("SOUNDS/")
                        .trim_end_matches(".wav")
                )
            )
        };
        let bytes = match (&p.pack, &p.radio) {
            (Some(pack), _) => {
                let inst = packs::installed(&self.voices_dir())
                    .into_iter()
                    .find(|i| &i.manifest.id == pack)
                    .ok_or_else(|| anyhow!("Pack {pack:?} is not installed."))?;
                if !inst.manifest.files.contains(&p.line) {
                    bail!("Pack {pack:?} has no take of {}.", p.line);
                }
                (
                    name(pack),
                    std::fs::read(Path::new(&inst.dir).join(&p.line))?,
                )
            }
            (None, Some(radio)) => {
                let o = self
                    .overrides_of(radio)?
                    .remove(&p.line)
                    .ok_or_else(|| anyhow!("{} has no override on this radio.", p.line))?;
                let blob = o.blob.ok_or_else(|| {
                    anyhow!("The override takes another pack's sound: pass the pack.")
                })?;
                (
                    name(&format!("own-{radio}")),
                    self.snapshots().blobs().get(&blob)?,
                )
            }
            _ => bail!("Name a pack, or a radio whose override to play."),
        };
        let path = dir.join(&bytes.0);
        std::fs::write(&path, &bytes.1)?;
        Ok(path.display().to_string())
    }

    // ----- Choose voice -----

    /// Stages one card change that puts a pack's sounds on a radio, replacing every sound
    /// the pack holds. With `keep_overrides` the lines the person overrode keep their
    /// override; without, the overrides are cleared. Sounds the pack does not know stay on
    /// the card. The change joins the radio's open voice change, if there is one.
    pub fn gear_voice_choose(&self, p: &VoiceChooseParams) -> Result<StagedChange> {
        self.radio_device(&p.radio)?;
        let pack = packs::installed(&self.voices_dir())
            .into_iter()
            .find(|i| i.manifest.id == p.pack)
            .ok_or_else(|| anyhow!("Pack {:?} is not installed: install it first.", p.pack))?;
        let blobs = self.snapshots().blobs();
        let installed = packs::installed(&self.voices_dir());
        let overrides = if p.keep_overrides {
            self.overrides_of(&p.radio)?
        } else {
            Default::default()
        };
        let mut files = packs::files_of(&pack)?;
        let mut notes_skipped = Vec::new();
        for (path, o) in &overrides {
            let bytes: Option<Vec<u8>> = match o.kind.as_str() {
                "text" => o.blob.as_ref().and_then(|b| blobs.get(b).ok()),
                "pack" => o.pack.as_ref().and_then(|id| {
                    installed
                        .iter()
                        .find(|i| &i.manifest.id == id)
                        .and_then(|i| std::fs::read(Path::new(&i.dir).join(path)).ok())
                }),
                _ => None,
            };
            let Some(bytes) = bytes else {
                notes_skipped.push(path.clone());
                continue;
            };
            match files.iter_mut().find(|(f, _)| f == path) {
                Some(slot) => slot.1 = bytes,
                None => files.push((path.clone(), bytes)),
            }
        }
        let mut put = Vec::new();
        for (path, bytes) in &files {
            let r = blobs.put(bytes)?;
            put.push(CardFile {
                path: path.clone(),
                xxh64: r.xxh64,
                size: r.size,
            });
        }
        put.sort_by(|a, b| a.path.cmp(&b.path));
        let edits = vec![Edit::CardFiles {
            put,
            delete: Vec::new(),
        }];
        let title = format!("{VOICE_CHANGE}{}", pack.manifest.voice);
        let open = self
            .changes()
            .list(&ChangeFilter {
                device: Some(p.radio.clone()),
                ..Default::default()
            })
            .into_iter()
            .find(|c| {
                c.title.starts_with(VOICE_CHANGE)
                    && matches!(c.status, ChangeStatus::Draft | ChangeStatus::Ready)
                    && c.edits.iter().all(|e| matches!(e, Edit::CardFiles { .. }))
            });
        self.gear_store().update(|v| {
            let voice = voice_obj(v)?;
            sub(voice, "chosen")?.insert(p.radio.clone(), json!(p.pack));
            if !p.keep_overrides {
                sub(voice, "overrides")?.remove(&p.radio);
            }
            Ok(())
        })?;
        let note = if notes_skipped.is_empty() {
            None
        } else {
            Some(format!(
                "An override could not be read and was left out: {}.",
                notes_skipped.join(", ")
            ))
        };
        match open {
            Some(c) => self.gear_change_update(&super::ChangeUpdateParams {
                id: c.id,
                title: Some(title),
                edits: Some(edits),
                note,
                ..Default::default()
            }),
            None => self.gear_change_stage(&super::StageParams {
                device: p.radio.clone(),
                title: Some(title),
                edits,
                note,
                editor: p.editor,
                ..Default::default()
            }),
        }
    }

    /// An installed pack, for the CLI.
    pub fn voice_installed(&self) -> Vec<Installed> {
        packs::installed(&self.voices_dir())
    }
}

fn slug(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let t = t.trim_matches('-').to_string();
    if t.is_empty() {
        "default".into()
    } else {
        t
    }
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    for e in std::fs::read_dir(from)?.flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            std::fs::create_dir_all(&dst)?;
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

fn collect_sounds(root: &Path, dir: &Path, out: &mut Vec<String>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_sounds(root, &p, out);
        } else if p.extension().is_some_and(|x| x == "wav") {
            if let Ok(r) = p.strip_prefix(root) {
                out.push(r.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

fn dummy_entry(id: &str) -> PackIndexEntry {
    PackIndexEntry {
        id: id.into(),
        voice: String::new(),
        lang: String::new(),
        provider: String::new(),
        model: String::new(),
        settings: RenderSettings::default(),
        lines: 0,
        bytes: 0,
        sha256: String::new(),
        license: String::new(),
        attribution: String::new(),
        lines_csv_sha: String::new(),
        version: String::new(),
        file: String::new(),
    }
}

/// The voice view as text, for the CLI and MCP.
pub fn view_text(v: &VoiceView) -> String {
    use std::fmt::Write;
    let p = &v.provider;
    let mut s = format!(
        "Provider: {}{}{}{}\n",
        p.provider,
        if p.base_url.is_empty() {
            String::new()
        } else {
            format!(" at {}", p.base_url)
        },
        if p.voice.is_empty() {
            String::new()
        } else {
            format!(", voice {}", p.voice)
        },
        if p.ready {
            if p.paid {
                " (may charge)".to_string()
            } else {
                String::new()
            }
        } else {
            format!(
                " (not ready: {})",
                p.problem.as_deref().unwrap_or("unknown")
            )
        }
    );
    let custom = v.lines.iter().filter(|l| l.custom).count();
    let _ = writeln!(
        s,
        "{} lines{}",
        v.lines.len(),
        if custom > 0 {
            format!(", {custom} of them yours")
        } else {
            String::new()
        }
    );
    let _ = writeln!(s, "Packs:");
    if v.packs.is_empty() {
        let _ = writeln!(s, "  none installed");
    }
    for k in &v.packs {
        let _ = writeln!(
            s,
            "  {} {}: {} lines, {}{}{}",
            k.id,
            k.voice,
            k.lines,
            if k.installed {
                "installed"
            } else {
                "available"
            },
            if k.local { ", rendered here" } else { "" },
            if k.stale { ", from older lines" } else { "" }
        );
    }
    if let Some(c) = &v.chosen {
        let _ = writeln!(s, "Chosen for the radio: {c}");
    }
    let o: Vec<&VoiceLine> = v.lines.iter().filter(|l| l.override_.is_some()).collect();
    if !o.is_empty() {
        let _ = writeln!(s, "Overrides:");
        for l in o {
            let ov = l.override_.as_ref().unwrap();
            let _ = writeln!(
                s,
                "  {}: {}",
                l.path,
                match ov.kind.as_str() {
                    "pack" => format!("take from {}", ov.pack.clone().unwrap_or_default()),
                    _ => format!("your text {:?}", ov.text.clone().unwrap_or_default()),
                }
            );
        }
    }
    for n in &v.notes {
        let _ = writeln!(s, "{n}");
    }
    s
}

/// A render report as text.
pub fn report_text(r: &RenderReport) -> String {
    let p = &r.plan;
    if r.needs_confirm {
        return format!(
            "Not rendered: {} characters would go to {} ({}), which may charge for them. Ask the person, then run again with confirm.\nPlan: {} lines, {} from the cache, {} to render.",
            p.chars, r.provider, r.voice, p.lines, p.cached, p.to_render
        );
    }
    if r.dry_run {
        return format!(
            "Dry run: would render {} of {} lines with {} ({}); {} come from the cache; {} characters{}.",
            p.to_render,
            p.lines,
            r.provider,
            r.voice,
            p.cached,
            p.chars,
            if r.paid { ", which may be charged" } else { "" }
        );
    }
    format!(
        "Rendered {} of {} lines with {} ({}) into pack {}; {} came from the cache.",
        r.rendered, p.lines, r.provider, r.voice, r.pack, r.from_cache
    )
}
