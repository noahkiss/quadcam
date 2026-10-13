//! Publishing a voice pack on the core (`docs/gear.md`, "Publish a pack"). `gear_voice_publish`
//! prepares: it refuses a pack the speech check has not cleared, then builds the zip, the
//! preview lines and the index entry in the cache, and answers the review (every line, flags
//! first, a spot check) with a digest. `voice_publish_approve` publishes that bundle through
//! `gh`, or answers the commands to run. It has no `api` row: the app's Approve button and a
//! person at the CLI's terminal call it, never MCP or the socket. `gear_voice_pack_preview`
//! fetches an index pack's preview lines to play before an install.

use super::Core;
use crate::gear::voice::packs::{self, PackIndexEntry, PackManifest, PreviewLine};
use crate::gear::voice::publish::{self, Bundle, GhStatus, Published};
use crate::gear::voice::{lines, sets, sha256_hex};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// `gear_voice_publish`: prepare a local pack for the voices repo.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct PublishParams {
    /// A pack on this Mac (`local-…`).
    pub pack: String,
    /// The published id; default `<lang>-<voice name>`.
    #[serde(default)]
    pub id: Option<String>,
    /// Default `1`.
    #[serde(default)]
    pub version: Option<String>,
    /// The voice's name in the index; default the pack's.
    #[serde(default)]
    pub voice_name: Option<String>,
    /// Default by provider: ElevenLabs (paid plan) and Kokoro fill it in.
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub attribution: Option<String>,
    /// `owner/name`; default the QuadCam voices repo.
    #[serde(default)]
    pub repo: Option<String>,
}

/// One line of the review.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct ReviewLine {
    pub path: String,
    pub text: String,
    /// What the speech check heard.
    pub heard: String,
    pub ok: bool,
    /// Heard otherwise, and the person accepted it.
    pub accepted: bool,
    /// The WAV as it goes into the zip (the app plays it).
    pub file: String,
    /// One of the spot check's lines.
    pub spot: bool,
    /// One of the preview lines.
    pub preview: bool,
}

/// What `gear_voice_publish` prepared, for the review.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct PublishReview {
    /// The pack on this Mac.
    pub pack: String,
    pub repo: String,
    /// The index entry Approve adds.
    pub entry: PackIndexEntry,
    /// The bundle's folder in the cache.
    pub dir: String,
    pub zip: String,
    /// Every line: those the person accepted first, then in card order.
    pub lines: Vec<ReviewLine>,
    /// The spot check: lines picked at random to listen to.
    pub spot_check: Vec<String>,
    /// Lines the check heard otherwise and the person accepted.
    pub accepted: u32,
    /// The person's own lines, left out of the published pack.
    pub left_out: Vec<String>,
    /// Approve repeats it.
    pub digest: String,
    /// Whether Approve can publish through `gh`, or prints commands.
    pub gh: GhStatus,
    /// The commands that publish the bundle by hand.
    pub commands: Vec<String>,
    pub notes: Vec<String>,
}

/// `voice_publish_approve`: the person's Approve.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct PublishApproveParams {
    /// The digest of the review the person approved.
    pub digest: String,
    /// The published pack's id, as the person confirmed it: refused unless it is the
    /// bundle's.
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct PublishApproveReport {
    pub id: String,
    pub version: String,
    /// What `gh` created; none when it could not run.
    #[serde(default)]
    pub published: Option<Published>,
    /// Without `gh`: the commands to run.
    pub commands: Vec<String>,
    pub notes: Vec<String>,
}

/// `gear_voice_pack_preview`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct PackPreviewParams {
    /// A pack id from the index.
    pub pack: String,
    /// The index (a path or an address) when the setting `voice_index` names none.
    #[serde(default)]
    pub source: Option<String>,
}

/// One preview line on this Mac.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct PreviewTake {
    pub path: String,
    pub text: String,
    /// The file the app plays.
    pub file: String,
}

/// What the bundle folder keeps for Approve.
#[derive(Serialize, Deserialize)]
struct Saved {
    digest: String,
    bundle: Bundle,
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Encodes a WAV as AAC in an `.m4a`, with macOS's `afconvert`.
fn to_aac(wav: &Path, out: &Path) -> Result<()> {
    let ran = Command::new("/usr/bin/afconvert")
        .args(["-f", "m4af", "-d", "aac", "-b", "64000"])
        .arg(wav)
        .arg(out)
        .output()
        .context("starting afconvert")?;
    if !ran.status.success() {
        bail!(
            "afconvert failed on {}: {}",
            wav.display(),
            String::from_utf8_lossy(&ran.stderr).trim()
        );
    }
    Ok(())
}

/// A path inside the index's folder: no `..`, no root, no backslash.
fn check_relative(f: &str) -> Result<()> {
    if f.is_empty() || f.starts_with('/') || f.contains("..") || f.contains('\\') {
        bail!("The index names the file {f:?}, which is not inside its folder.");
    }
    Ok(())
}

impl Core {
    fn publish_root(&self) -> PathBuf {
        self.cache.join("voice").join("publish")
    }

    /// The card paths of every shipped line set: what a published pack may hold.
    fn shipped_paths() -> Result<HashSet<String>> {
        let mut out = HashSet::new();
        for id in sets::ids() {
            if id == sets::CUSTOM {
                continue;
            }
            out.extend(sets::lines_of(id)?.into_iter().map(|l| l.line.path));
        }
        Ok(out)
    }

    /// Prepares a pack on this Mac for the voices repo and answers the review. Writes only
    /// QuadCam's cache; publishes nothing.
    pub fn gear_voice_publish(&self, p: &PublishParams) -> Result<PublishReview> {
        let src = p.pack.trim();
        if src.is_empty() {
            bail!("Name a pack on this Mac (quadcam-cli gear voice lists them).");
        }
        packs::check_id(src)?;
        let inst = packs::installed(&self.voices_dir())
            .into_iter()
            .find(|i| i.manifest.id == src)
            .ok_or_else(|| anyhow!("Pack {src:?} is not on this Mac."))?;
        let m = &inst.manifest;
        let voice = p
            .voice_name
            .clone()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| m.voice.clone());
        let auto = publish::license_for(&m.provider, &m.model, &voice)?;
        let id =
            p.id.clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| format!("{}-{}", m.lang, super::voice::short_slug(&voice)));
        packs::check_id(&id)?;
        if id.starts_with("local-") {
            bail!("{id:?} is a local pack's id: give the published pack its own id (--id).");
        }
        let version = p.version.clone().unwrap_or_else(|| "1".into());
        publish::check_version(&version)?;
        let repo = p.repo.clone().unwrap_or_else(|| publish::REPO.into());
        publish::check_repo(&repo)?;
        let license = p
            .license
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| auto.as_ref().map(|a| a.0.clone()))
            .ok_or_else(|| {
                anyhow!("Give the pack a license (--license): QuadCam fills it in only for ElevenLabs and Kokoro packs.")
            })?;
        let attribution = p
            .attribution
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| auto.as_ref().map(|a| a.1.clone()))
            .unwrap_or_default();

        // The speech check clears every line, or the person accepted what it heard.
        let (unheard, wrong) = publish::not_heard(m);
        if !unheard.is_empty() {
            bail!(
                "The speech check has not heard {} of the pack's {} lines: run Check speech (quadcam-cli gear voice check {src}) first.",
                unheard.len(),
                m.files.len()
            );
        }
        if !wrong.is_empty() {
            bail!(
                "{} lines the speech check heard wrong are not accepted ({}{}): render them again, or listen and accept what was heard.",
                wrong.len(),
                wrong.iter().take(5).cloned().collect::<Vec<_>>().join(", "),
                if wrong.len() > 5 { ", …" } else { "" }
            );
        }

        let shipped = Self::shipped_paths()?;
        let (files, left_out): (Vec<String>, Vec<String>) =
            m.files.iter().cloned().partition(|f| shipped.contains(f));
        if files.is_empty() {
            bail!("The pack holds none of QuadCam's lines, so there is nothing to publish.");
        }
        let mut notes = Vec::new();
        if !left_out.is_empty() {
            notes.push(format!(
                "{} lines of your own stay on this Mac: a published pack holds QuadCam's lines only.",
                left_out.len()
            ));
        }
        if !m.retakes.is_empty() {
            notes.push(format!(
                "{} lines need a re-take and are not in the pack.",
                m.retakes.len()
            ));
        }

        // The bundle: the pack as published, its zip and its previews.
        let dir = self.publish_root().join(publish::tag(&id, &version));
        let _ = std::fs::remove_dir_all(&dir);
        let stage = dir.join("stage");
        std::fs::create_dir_all(&stage)?;
        for f in &files {
            lines::check_path(f)?;
            let to = stage.join(f);
            std::fs::create_dir_all(to.parent().unwrap())?;
            std::fs::copy(Path::new(&inst.dir).join(f), &to)
                .with_context(|| format!("pack {src} lost {f}"))?;
        }
        let out = PackManifest {
            id: id.clone(),
            voice: voice.clone(),
            license: license.clone(),
            attribution: attribution.clone(),
            version: version.clone(),
            lines: files.len() as u32,
            files: files.clone(),
            heard: m
                .heard
                .iter()
                .filter(|h| files.contains(&h.path))
                .cloned()
                .collect(),
            kept_batches: Vec::new(),
            ..m.clone()
        };
        std::fs::write(stage.join("pack.json"), serde_json::to_vec_pretty(&out)?)?;
        let zip = dir.join(publish::zip_name(&id, &version));
        packs::zip_dir(&stage, &zip)?;
        let zip_bytes = std::fs::read(&zip)?;

        let texts: HashMap<&str, &str> = m
            .heard
            .iter()
            .map(|h| (h.path.as_str(), h.text.as_str()))
            .collect();
        let sample: Vec<String> = sets::lines_of("sample")?
            .into_iter()
            .map(|l| l.line.path)
            .collect();
        let picked = publish::pick_previews(&files, &sample);
        let folder = publish::preview_folder(&id, &version);
        std::fs::create_dir_all(dir.join("previews"))?;
        let mut preview = Vec::new();
        let mut preview_files = Vec::new();
        for (i, f) in picked.iter().enumerate() {
            let name = format!("{:02}.m4a", i + 1);
            let local = dir.join("previews").join(&name);
            to_aac(&stage.join(f), &local)?;
            let repo_path = format!("{folder}/{name}");
            preview.push(PreviewLine {
                path: f.clone(),
                text: texts
                    .get(f.as_str())
                    .copied()
                    .unwrap_or_default()
                    .to_string(),
                file: repo_path.clone(),
                sha256: sha256_hex(&std::fs::read(&local)?),
            });
            preview_files.push((repo_path, local));
        }

        let entry = PackIndexEntry {
            id: id.clone(),
            voice: voice.clone(),
            lang: m.lang.clone(),
            provider: m.provider.clone(),
            model: m.model.clone(),
            settings: m.settings.clone(),
            lines: files.len() as u32,
            bytes: packs::dir_bytes(&stage.join("SOUNDS")),
            sha256: sha256_hex(&zip_bytes),
            license,
            attribution,
            lines_csv_sha: m.lines_csv_sha.clone(),
            version: version.clone(),
            file: publish::zip_name(&id, &version),
            firmware: m.firmware.clone(),
            url: publish::asset_url(&repo, &id, &version),
            zip_bytes: zip_bytes.len() as u64,
            date: today(),
            preview,
        };
        let digest = publish::digest(&repo, &entry)?;
        let bundle = Bundle {
            repo: repo.clone(),
            entry: entry.clone(),
            zip: zip.clone(),
            previews: preview_files,
            notes: format!(
                "{voice}: {} lines for EdgeTX radios, made with {}{}.\n\nLicense: {}\n{}\n\nSHA-256 of the zip: {}\n\nInstall it in QuadCam: Gear > Voices > Get voice packs.\n",
                entry.lines,
                entry.provider,
                if entry.model.is_empty() { String::new() } else { format!(" ({})", entry.model) },
                entry.license,
                entry.attribution,
                entry.sha256
            ),
        };
        std::fs::write(dir.join("entry.json"), serde_json::to_vec_pretty(&entry)?)?;
        std::fs::write(dir.join("notes.md"), &bundle.notes)?;
        std::fs::write(
            dir.join("bundle.json"),
            serde_json::to_vec_pretty(&Saved {
                digest: digest.clone(),
                bundle: bundle.clone(),
            })?,
        )?;

        let spot = publish::spot_check(&files, &digest);
        let mut review: Vec<ReviewLine> = files
            .iter()
            .map(|f| {
                let h = m.heard.iter().find(|h| h.path == *f);
                ReviewLine {
                    path: f.clone(),
                    text: h.map(|h| h.text.clone()).unwrap_or_default(),
                    heard: h.map(|h| h.heard.clone()).unwrap_or_default(),
                    ok: h.is_some_and(|h| h.ok),
                    accepted: h.is_some_and(|h| h.accepted),
                    file: stage.join(f).display().to_string(),
                    spot: spot.contains(f),
                    preview: picked.contains(f),
                }
            })
            .collect();
        review.sort_by_key(|l| !l.accepted);
        let accepted = review.iter().filter(|l| l.accepted).count() as u32;
        let gh = self.gear.gh.status();
        if let Some(r) = &gh.reason {
            notes.push(format!("Approve will print the commands to run: {r}"));
        }
        Ok(PublishReview {
            pack: src.to_string(),
            repo,
            commands: publish::manual_commands(&bundle, &dir),
            entry,
            dir: dir.display().to_string(),
            zip: zip.display().to_string(),
            lines: review,
            spot_check: spot,
            accepted,
            left_out,
            digest,
            gh,
            notes,
        })
    }

    /// The person's Approve of a prepared bundle: checks its files against the digest, then
    /// creates the release and the index pull request through `gh`, or answers the commands.
    /// No `api` row: only the app's Approve button and the CLI at a terminal call it.
    pub fn voice_publish_approve(&self, p: &PublishApproveParams) -> Result<PublishApproveReport> {
        let want = p.digest.trim();
        if want.is_empty() {
            bail!("Approve needs the review's digest.");
        }
        let saved = std::fs::read_dir(self.publish_root())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let b = std::fs::read(e.path().join("bundle.json")).ok()?;
                serde_json::from_slice::<Saved>(&b).ok()
            })
            .find(|s| s.digest == want)
            .ok_or_else(|| anyhow!("No prepared pack has the digest {want}: prepare it again."))?;
        let b = saved.bundle;
        let e = &b.entry;
        if let Some(id) = &p.id {
            if id.trim() != e.id {
                bail!(
                    "The digest is for {}, not {:?}: nothing was published.",
                    e.id,
                    id.trim()
                );
            }
        }
        let changed =
            || anyhow!("The prepared files changed after the review: prepare the pack again.");
        if publish::digest(&b.repo, e)? != want {
            return Err(changed());
        }
        if sha256_hex(&std::fs::read(&b.zip).map_err(|_| changed())?) != e.sha256 {
            return Err(changed());
        }
        for (path, local) in &b.previews {
            let want = e
                .preview
                .iter()
                .find(|x| x.file == *path)
                .ok_or_else(changed)?;
            if sha256_hex(&std::fs::read(local).map_err(|_| changed())?) != want.sha256 {
                return Err(changed());
            }
        }
        let dir = b.zip.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut report = PublishApproveReport {
            id: e.id.clone(),
            version: e.version.clone(),
            published: None,
            commands: Vec::new(),
            notes: Vec::new(),
        };
        let gh = self.gear.gh.status();
        if !gh.ready {
            report.commands = publish::manual_commands(&b, &dir);
            report.notes.push(format!(
                "Nothing was published: {} Run these commands to publish it.",
                gh.reason.unwrap_or_default()
            ));
            return Ok(report);
        }
        let done = self.gear.gh.publish(&b, &dir.join("gh"))?;
        report.notes.push(format!(
            "Released {} and opened {}. Merging the pull request publishes the pack.",
            done.release_url, done.pr_url
        ));
        report.published = Some(done);
        Ok(report)
    }

    /// Fetches an index pack's preview lines into the cache, each checked against its hash.
    pub fn gear_voice_pack_preview(&self, p: &PackPreviewParams) -> Result<Vec<PreviewTake>> {
        let source = p
            .source
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.voice_index_source())
            .ok_or_else(|| {
                anyhow!("No pack index: set voice_index, or pass the index's path or address.")
            })?;
        let index = self.refresh_index(&source)?;
        let e = index
            .packs
            .iter()
            .find(|e| e.id == p.pack)
            .ok_or_else(|| anyhow!("The index lists no pack {:?}.", p.pack))?;
        packs::check_id(&e.id)?;
        if e.preview.is_empty() {
            bail!("The index has no preview lines for {}.", e.id);
        }
        let dir = self.cache.join("voice").join("index-preview").join(format!(
            "{}-{}",
            e.id,
            super::voice::slug(&e.version)
        ));
        std::fs::create_dir_all(&dir)?;
        let mut out = Vec::new();
        for (i, l) in e.preview.iter().enumerate() {
            check_relative(&l.file)?;
            lines::check_path(&l.path)?;
            let local = dir.join(format!("{:02}.m4a", i + 1));
            let fresh = std::fs::read(&local)
                .map(|b| sha256_hex(&b) == l.sha256)
                .unwrap_or(false);
            if !fresh {
                self.fetch_beside(&source, &l.file, &local)?;
                if sha256_hex(&std::fs::read(&local)?) != l.sha256 {
                    let _ = std::fs::remove_file(&local);
                    bail!(
                        "The preview {} does not match the hash in the index.",
                        l.file
                    );
                }
            }
            out.push(PreviewTake {
                path: l.path.clone(),
                text: l.text.clone(),
                file: local.display().to_string(),
            });
        }
        Ok(out)
    }
}

fn mb(b: u64) -> String {
    format!("{:.1} MB", b as f64 / 1e6)
}

/// A publish review as text, for the CLI and MCP.
pub fn review_text(r: &PublishReview) -> String {
    use std::fmt::Write;
    let e = &r.entry;
    let mut s = format!(
        "Prepared {} v{} from {}: {} lines, {} zip, {} on the card. Nothing is published.\nLicense: {}{}\nRelease: {} on {}\nDigest: {}\n",
        e.id,
        e.version,
        r.pack,
        e.lines,
        mb(e.zip_bytes),
        mb(e.bytes),
        e.license,
        if e.attribution.is_empty() { String::new() } else { format!(" ({})", e.attribution) },
        publish::tag(&e.id, &e.version),
        r.repo,
        r.digest
    );
    let _ = writeln!(
        s,
        "Spot check ({} lines, listen to these at least):",
        r.spot_check.len()
    );
    for l in r.lines.iter().filter(|l| l.spot) {
        let _ = writeln!(s, "  {}: {:?}  {}", l.path, l.text, l.file);
    }
    let _ = writeln!(
        s,
        "Lines ({} accepted after the speech check heard otherwise):",
        r.accepted
    );
    for l in &r.lines {
        let mark = if l.accepted { "accepted" } else { "ok" };
        let _ = writeln!(s, "  {mark:8} {}: {:?} heard {:?}", l.path, l.text, l.heard);
    }
    for n in &r.notes {
        let _ = writeln!(s, "{n}");
    }
    s.push_str("Only the person approves: the Approve button on QuadCam's Voices page, or `quadcam-cli gear voice publish --approve --digest <digest>` typed in a terminal.");
    s
}

/// An Approve's answer as text.
pub fn approve_text(r: &PublishApproveReport) -> String {
    let mut s = String::new();
    if let Some(p) = &r.published {
        s.push_str(&format!(
            "Released {} v{}: {}\nPull request: {}\n",
            r.id, r.version, p.release_url, p.pr_url
        ));
    }
    for n in &r.notes {
        s.push_str(n);
        s.push('\n');
    }
    for c in &r.commands {
        s.push_str(c);
        s.push('\n');
    }
    s
}

/// Preview lines as text.
pub fn preview_text(pack: &str, t: &[PreviewTake]) -> String {
    let mut s = format!("{} preview lines of {pack}:", t.len());
    for x in t {
        s.push_str(&format!("\n  {}: {}", x.text, x.file));
    }
    s
}
