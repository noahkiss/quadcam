//! Voice packs (design 7.4). A pack is a zip per voice, `voice-<id>-<version>.zip`, holding
//! `pack.json` and the WAVs at their card paths. A `voices.json` index lists the packs. The
//! maintainer builds a pack with `build`; a user renders their own lines through the same
//! code into a local pack (`render_to`). `install` needs the zip's hash and checks it, unpacks
//! it into `<gear>/voices/<id>/` and never trusts a path or a link inside it.

use super::lines::{check_path, Line, Spelling};
use super::render::{self, Ctx, RenderSettings};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The firmware family a pack is for. Only EdgeTX packs exist; the field leaves room for
/// another family's packs, whose card paths differ.
pub const EDGETX: &str = "edgetx";

fn edgetx() -> String {
    EDGETX.into()
}

/// What the index says about one pack.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct PackIndexEntry {
    pub id: String,
    pub voice: String,
    pub lang: String,
    pub provider: String,
    #[serde(default)]
    pub model: String,
    pub settings: RenderSettings,
    pub lines: u32,
    pub bytes: u64,
    pub sha256: String,
    pub license: String,
    pub attribution: String,
    /// The sha256 of the `lines.csv` the pack was rendered from.
    pub lines_csv_sha: String,
    #[serde(default)]
    pub version: String,
    /// The zip's file name; the index's own folder or URL holds it.
    #[serde(default)]
    pub file: String,
    /// The firmware family the pack is for (`edgetx`).
    #[serde(default = "edgetx")]
    pub firmware: String,
}

/// `voices.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Type)]
pub struct VoiceIndex {
    #[serde(default)]
    pub packs: Vec<PackIndexEntry>,
}

/// `pack.json` inside a pack: the index entry without what only the zip knows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct PackManifest {
    pub id: String,
    pub voice: String,
    pub lang: String,
    pub provider: String,
    #[serde(default)]
    pub model: String,
    pub settings: RenderSettings,
    pub lines: u32,
    pub license: String,
    pub attribution: String,
    pub lines_csv_sha: String,
    #[serde(default)]
    pub version: String,
    /// Card paths of the WAVs.
    pub files: Vec<String>,
    /// Lines whose cut failed a check: left out of the pack, they need a re-take.
    #[serde(default)]
    pub retakes: Vec<Retake>,
    /// A batched render: the batches (keyed without the seed) whose lines are all in the
    /// pack. A re-render with another seed and the same settings skips them.
    #[serde(default)]
    pub kept_batches: Vec<String>,
    /// The firmware family the pack is for (`edgetx`).
    #[serde(default = "edgetx")]
    pub firmware: String,
    /// The provider's voice id the takes were made with (a say voice name, an ElevenLabs
    /// voice id). Empty in packs made before 0.12.
    #[serde(default)]
    pub voice_id: String,
}

/// A line that needs a re-take, and why.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct Retake {
    /// The card path.
    pub path: String,
    pub text: String,
    pub reason: String,
}

/// What a pack is called and where it stands.
#[derive(Debug, Clone)]
pub struct BuildOpts {
    pub id: String,
    pub version: String,
    pub voice: String,
    pub lang: String,
    pub license: String,
    pub attribution: String,
    pub lines_csv_sha: String,
}

/// A pack id: letters, digits, `-` and `_`.
pub fn check_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("{id:?} is not a pack id: use letters, digits, - and _");
    }
    Ok(())
}

/// Renders `lines` of `opts.lang` into `dir`, WAVs at their card paths and `pack.json` at the
/// top. Returns the manifest, and how many takes the provider made. `each` hears the line
/// number after every line.
pub fn render_to(
    opts: &BuildOpts,
    ctx: &Ctx<'_>,
    lines: &[Line],
    spelling: &Spelling,
    dir: &Path,
    each: &mut dyn FnMut(usize, usize),
) -> Result<(PackManifest, u32)> {
    check_id(&opts.id)?;
    let prefix = format!("SOUNDS/{}/", opts.lang);
    let mine: Vec<&Line> = lines
        .iter()
        .filter(|l| l.path.starts_with(&prefix))
        .collect();
    if mine.is_empty() {
        bail!("no line is under {prefix}");
    }
    let (mut made, mut files) = (0u32, Vec::new());
    for (i, l) in mine.iter().enumerate() {
        check_path(&l.path)?;
        let spoken = spelling.apply(&l.text);
        let (bytes, hit) = render::render_line(ctx, &spoken)?;
        if !hit {
            made += 1;
        }
        let dest = dir.join(&l.path);
        std::fs::create_dir_all(dest.parent().unwrap())?;
        std::fs::write(&dest, bytes).with_context(|| format!("writing {}", dest.display()))?;
        files.push(l.path.clone());
        each(i + 1, mine.len());
    }
    let m = PackManifest {
        id: opts.id.clone(),
        voice: opts.voice.clone(),
        lang: opts.lang.clone(),
        provider: ctx.tts.id().into(),
        model: ctx.model.into(),
        settings: ctx.settings.clone(),
        lines: files.len() as u32,
        license: opts.license.clone(),
        attribution: opts.attribution.clone(),
        lines_csv_sha: opts.lines_csv_sha.clone(),
        version: opts.version.clone(),
        files,
        retakes: Vec::new(),
        kept_batches: Vec::new(),
        firmware: EDGETX.into(),
        voice_id: ctx.voice.into(),
    };
    std::fs::write(dir.join("pack.json"), serde_json::to_vec_pretty(&m)?)?;
    Ok((m, made))
}

/// The size of the files under `dir`.
pub fn dir_bytes(dir: &Path) -> u64 {
    let mut n = 0;
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        n += if p.is_dir() {
            dir_bytes(&p)
        } else {
            e.metadata().map(|m| m.len()).unwrap_or(0)
        };
    }
    n
}

/// Builds the zip and its index entry in `out`, and puts the entry in `out/voices.json`
/// (replacing one with its id). `work` is a scratch folder for the render.
pub fn build(
    opts: &BuildOpts,
    ctx: &Ctx<'_>,
    lines: &[Line],
    spelling: &Spelling,
    out: &Path,
    work: &Path,
    each: &mut dyn FnMut(usize, usize),
) -> Result<(PathBuf, PackIndexEntry)> {
    let stage = work.join(format!("stage-{}", opts.id));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage)?;
    let r = render_to(opts, ctx, lines, spelling, &stage, each);
    let (manifest, _) = match r {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(e);
        }
    };
    std::fs::create_dir_all(out)?;
    let name = format!("voice-{}-{}.zip", opts.id, opts.version);
    let zip = out.join(&name);
    let _ = std::fs::remove_file(&zip);
    let ran = Command::new("/usr/bin/zip")
        .current_dir(&stage)
        .args(["-X", "-q", "-r"])
        .arg(&zip)
        .args(["pack.json", "SOUNDS"])
        .output()
        .context("starting zip")?;
    let bytes_in = dir_bytes(&stage);
    let _ = std::fs::remove_dir_all(&stage);
    if !ran.status.success() {
        bail!(
            "zip failed: {}",
            String::from_utf8_lossy(&ran.stderr).trim()
        );
    }
    let bytes = std::fs::read(&zip)?;
    let entry = PackIndexEntry {
        id: manifest.id,
        voice: manifest.voice,
        lang: manifest.lang,
        provider: manifest.provider,
        model: manifest.model,
        settings: manifest.settings,
        lines: manifest.lines,
        bytes: bytes_in,
        sha256: super::sha256_hex(&bytes),
        license: manifest.license,
        attribution: manifest.attribution,
        lines_csv_sha: manifest.lines_csv_sha,
        version: manifest.version,
        file: name,
        firmware: manifest.firmware,
    };
    let idx_path = out.join("voices.json");
    let mut idx = read_index(&idx_path).unwrap_or_default();
    idx.packs.retain(|p| p.id != entry.id);
    idx.packs.push(entry.clone());
    idx.packs.sort_by(|a, b| a.id.cmp(&b.id));
    std::fs::write(&idx_path, serde_json::to_vec_pretty(&idx)?)?;
    Ok((zip, entry))
}

pub fn read_index(path: &Path) -> Result<VoiceIndex> {
    let b = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&b).with_context(|| format!("{} is not a voice index", path.display()))
}

/// A pack on disk, as installed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct Installed {
    pub manifest: PackManifest,
    pub dir: String,
}

/// Packs under `<gear>/voices/`, by id.
pub fn installed(voices: &Path) -> Vec<Installed> {
    let mut out: Vec<Installed> = std::fs::read_dir(voices)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let dir = e.path();
            let m: PackManifest =
                serde_json::from_slice(&std::fs::read(dir.join("pack.json")).ok()?).ok()?;
            Some(Installed {
                manifest: m,
                dir: dir.display().to_string(),
            })
        })
        .collect();
    out.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    out
}

/// A name inside a pack zip is `pack.json`, a folder under `SOUNDS/`, or a sound path under
/// `SOUNDS/`. The path checks run first, so no folder name gets past them.
fn check_member(name: &str) -> Result<()> {
    if name.starts_with('/') || name.contains("..") || name.contains('\\') {
        bail!("the pack holds {name:?}, a path outside its folder");
    }
    if name == "pack.json" || name.ends_with('/') && name.starts_with("SOUNDS/") {
        return Ok(());
    }
    check_path(name).with_context(|| format!("the pack holds {name:?}"))
}

/// Refuses a zip that holds a symbolic link: `zipinfo` shows its mode as `l...`.
fn check_no_links(zip: &Path) -> Result<()> {
    let listed = Command::new("/usr/bin/zipinfo")
        .arg(zip)
        .output()
        .context("starting zipinfo")?;
    if !listed.status.success() {
        bail!("{} is not a zip file", zip.display());
    }
    for line in String::from_utf8_lossy(&listed.stdout).lines() {
        let mut words = line.split_whitespace();
        let (Some(mode), Some(name)) = (words.next(), words.last()) else {
            continue;
        };
        if mode.len() == 10 && mode.starts_with('l') {
            bail!("the pack holds {name:?}, a symbolic link");
        }
    }
    Ok(())
}

/// Refuses anything under `dir` that is not a plain file or a folder (a link, a device).
fn check_plain(dir: &Path) -> Result<()> {
    for e in std::fs::read_dir(dir)?.flatten() {
        let p = e.path();
        let t = std::fs::symlink_metadata(&p)?.file_type();
        if t.is_dir() {
            check_plain(&p)?;
        } else if !t.is_file() {
            bail!("the pack holds {}, which is not a plain file", p.display());
        }
    }
    Ok(())
}

/// Checks a zip's hash against the index entry, unpacks it into `<voices>/<id>/` and reads
/// its manifest. A zip with a path outside `SOUNDS/` refuses before anything is written.
pub fn install(voices: &Path, zip: &Path, entry: &PackIndexEntry) -> Result<Installed> {
    check_id(&entry.id)?;
    if entry.sha256.trim().is_empty() {
        bail!(
            "the index gives no hash for {}; it was not installed",
            entry.id
        );
    }
    let bytes = std::fs::read(zip).with_context(|| format!("reading {}", zip.display()))?;
    if super::sha256_hex(&bytes) != entry.sha256 {
        bail!(
            "{} does not match the hash in the index; it was not installed",
            zip.display()
        );
    }
    let listed = Command::new("/usr/bin/unzip")
        .args(["-Z1"])
        .arg(zip)
        .output()
        .context("starting unzip")?;
    if !listed.status.success() {
        bail!("{} is not a zip file", zip.display());
    }
    for name in String::from_utf8_lossy(&listed.stdout).lines() {
        check_member(name)?;
    }
    check_no_links(zip)?;
    std::fs::create_dir_all(voices)?;
    let tmp = voices.join(format!(".installing-{}", entry.id));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    let ran = Command::new("/usr/bin/unzip")
        .args(["-q", "-o"])
        .arg(zip)
        .arg("-d")
        .arg(&tmp)
        .output()
        .context("starting unzip")?;
    if !ran.status.success() {
        let _ = std::fs::remove_dir_all(&tmp);
        bail!(
            "unzip failed: {}",
            String::from_utf8_lossy(&ran.stderr).trim()
        );
    }
    let manifest: Result<PackManifest> = std::fs::read(tmp.join("pack.json"))
        .context("the pack has no pack.json")
        .and_then(|b| serde_json::from_slice(&b).context("pack.json is not a pack manifest"));
    let manifest = match manifest {
        Ok(m) if m.id == entry.id => match check_plain(&tmp) {
            Ok(()) => m,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(e);
            }
        },
        Ok(m) => {
            let _ = std::fs::remove_dir_all(&tmp);
            bail!(
                "the pack says it is {:?}, the index says {:?}",
                m.id,
                entry.id
            );
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }
    };
    for f in &manifest.files {
        if let Err(e) = check_path(f).and_then(|_| {
            if tmp.join(f).is_file() {
                Ok(())
            } else {
                bail!("{f} is listed and missing")
            }
        }) {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }
    }
    let dest = voices.join(&entry.id);
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&tmp, &dest)?;
    Ok(Installed {
        manifest,
        dir: dest.display().to_string(),
    })
}

/// Removes the installed pack `id` from `<voices>/`. Returns the bytes it held.
pub fn delete(voices: &Path, id: &str) -> Result<u64> {
    check_id(id)?;
    let dir = voices.join(id);
    if !dir.join("pack.json").is_file() {
        bail!("Pack {id:?} is not installed.");
    }
    let bytes = dir_bytes(&dir);
    std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
    Ok(bytes)
}

/// A pack's files: each card path with its bytes.
pub fn files_of(p: &Installed) -> Result<Vec<(String, Vec<u8>)>> {
    p.manifest
        .files
        .iter()
        .map(|f| {
            check_path(f)?;
            let b = std::fs::read(Path::new(&p.dir).join(f))
                .with_context(|| format!("pack {} lost {f}", p.manifest.id))?;
            Ok((f.clone(), b))
        })
        .collect()
}
