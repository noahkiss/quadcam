//! Publishing a voice pack to the voices repo (`docs/gear.md`, "Publish a pack"): the release
//! names, the preview lines and the spot check, the license a provider's packs carry, the
//! index merge, and GitHub through `gh`. `core/voice_publish.rs` prepares a bundle and
//! approves it; nothing here decides whether a pack may go out.
//!
//! The zip is a release asset of the repo (tag `<id>-v<version>`). The previews and the index
//! are files in the repo, which GitHub Pages serves: Approve pushes them on a branch through
//! the Git Data API and opens a pull request, so a merge publishes the pack. `gh` is optional:
//! without it, or signed out, Approve prints the commands instead.

use super::packs::{PackIndexEntry, PackManifest, VoiceIndex};
use super::sha256_hex;
use crate::modules::run::{Process, Runner};
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// The voices repo on GitHub.
pub const REPO: &str = "noahkiss/quadcam-voices";

/// The index the `voice_index` setting reads when the settings file names none.
pub const INDEX_URL: &str = "https://noahkiss.github.io/quadcam-voices/index.json";

/// Preview lines per pack.
pub const PREVIEW_LINES: usize = 10;

/// The spot check's size: a twentieth of the lines, at least 10, at most 15.
pub const SPOT_MIN: usize = 10;
pub const SPOT_MAX: usize = 15;

/// Where `gh` may be: Homebrew's folders, never the person's PATH.
const GH_PATHS: &[&str] = &["/opt/homebrew/bin/gh", "/usr/local/bin/gh"];

pub fn tag(id: &str, version: &str) -> String {
    format!("{id}-v{version}")
}

pub fn zip_name(id: &str, version: &str) -> String {
    format!("voice-{id}-{version}.zip")
}

pub fn asset_url(repo: &str, id: &str, version: &str) -> String {
    format!(
        "https://github.com/{repo}/releases/download/{}/{}",
        tag(id, version),
        zip_name(id, version)
    )
}

/// The repo folder of a pack version's previews.
pub fn preview_folder(id: &str, version: &str) -> String {
    format!("previews/{id}-{version}")
}

/// The branch Approve pushes.
pub fn branch(id: &str, version: &str) -> String {
    format!("pack/{}", tag(id, version))
}

/// A version: digits and dots, such as `1` or `1.2`.
pub fn check_version(v: &str) -> Result<()> {
    if v.is_empty()
        || v.len() > 16
        || v.starts_with('.')
        || v.ends_with('.')
        || !v.chars().all(|c| c.is_ascii_digit() || c == '.')
    {
        bail!("{v:?} is not a pack version: use digits and dots, such as 1 or 1.2");
    }
    Ok(())
}

/// `owner/name`.
pub fn check_repo(r: &str) -> Result<()> {
    let ok = |s: &str| {
        !s.is_empty()
            && s.len() <= 100
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            && !s.starts_with('.')
    };
    match r.split_once('/') {
        Some((o, n)) if ok(o) && ok(n) => Ok(()),
        _ => bail!("{r:?} is not a GitHub repo: use owner/name"),
    }
}

/// The preview lines: `wanted` (the `sample` set) in its order, those the pack holds, up to
/// `PREVIEW_LINES`; the pack's first files fill the rest.
pub fn pick_previews(files: &[String], wanted: &[String]) -> Vec<String> {
    let mut out: Vec<String> = wanted
        .iter()
        .filter(|w| files.contains(w))
        .take(PREVIEW_LINES)
        .cloned()
        .collect();
    for f in files {
        if out.len() >= PREVIEW_LINES {
            break;
        }
        if !out.contains(f) {
            out.push(f.clone());
        }
    }
    out
}

/// The spot check: lines picked at random, the same pick for the same `seed` (the bundle's
/// digest), in the pack's order.
pub fn spot_check(paths: &[String], seed: &str) -> Vec<String> {
    let n = (paths.len() / 20)
        .clamp(SPOT_MIN, SPOT_MAX)
        .min(paths.len());
    let mut keyed: Vec<(String, &String)> = paths
        .iter()
        .map(|p| (sha256_hex(format!("{seed}\n{p}").as_bytes()), p))
        .collect();
    keyed.sort();
    let picked: Vec<&String> = keyed.into_iter().take(n).map(|(_, p)| p).collect();
    paths
        .iter()
        .filter(|p| picked.contains(p))
        .cloned()
        .collect()
}

/// The license and attribution a provider's packs carry when the person gives none, or why
/// none fits.
pub fn license_for(provider: &str, model: &str, voice: &str) -> Result<Option<(String, String)>> {
    match provider {
        "say" => bail!(
            "This pack was made with macOS say: Apple's voices are licensed for use on the Mac, so it cannot be published."
        ),
        "elevenlabs" => Ok(Some((
            "ElevenLabs output, paid plan: commercial use allowed".into(),
            format!("Voice: {voice} (ElevenLabs)"),
        ))),
        _ if model.to_ascii_lowercase().contains("kokoro") => Ok(Some((
            "Apache-2.0".into(),
            format!("Kokoro-82M, voice {voice}"),
        ))),
        _ => Ok(None),
    }
}

/// The speech check's verdict on a pack: the files it has not heard, and the lines it heard
/// wrong that the person did not accept.
pub fn not_heard(m: &PackManifest) -> (Vec<String>, Vec<String>) {
    let mut unheard = Vec::new();
    let mut wrong = Vec::new();
    for f in &m.files {
        match m.heard.iter().find(|h| h.path == *f) {
            None => unheard.push(f.clone()),
            Some(h) if !h.ok && !h.accepted => wrong.push(f.clone()),
            Some(_) => {}
        }
    }
    (unheard, wrong)
}

/// Puts `e` into the index, replacing the entry with its id; packs sorted by id.
pub fn merge(idx: &mut VoiceIndex, e: PackIndexEntry) {
    idx.packs.retain(|p| p.id != e.id);
    idx.packs.push(e);
    idx.packs.sort_by(|a, b| a.id.cmp(&b.id));
    idx.schema.get_or_insert(1);
}

/// The index as the repo keeps it: pretty JSON with a final newline.
pub fn index_bytes(idx: &VoiceIndex) -> Result<Vec<u8>> {
    let mut b = serde_json::to_vec_pretty(idx)?;
    b.push(b'\n');
    Ok(b)
}

/// What a bundle's digest covers: the entry (with the zip's and every preview's hash) and the
/// repo. Approve repeats it, so it publishes the files the person reviewed.
pub fn digest(repo: &str, entry: &PackIndexEntry) -> Result<String> {
    let mut b = serde_json::to_vec(entry)?;
    b.extend_from_slice(repo.as_bytes());
    Ok(sha256_hex(&b)[..16].to_string())
}

/// A prepared pack, ready for Approve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle {
    pub repo: String,
    pub entry: PackIndexEntry,
    /// The zip on this Mac.
    pub zip: PathBuf,
    /// Each preview: its repo path and the file on this Mac.
    pub previews: Vec<(String, PathBuf)>,
    /// The release's notes.
    pub notes: String,
}

/// What Approve did.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct Published {
    pub release_url: String,
    pub pr_url: String,
    pub branch: String,
}

/// Whether `gh` can publish.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct GhStatus {
    /// `gh` is installed and signed in to github.com.
    pub ready: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

/// GitHub's CLI, run as a child process.
pub struct Gh {
    program: std::result::Result<PathBuf, String>,
    runner: Arc<dyn Runner>,
}

impl Gh {
    /// `gh` from Homebrew's folders. Off in a process started by cargo unless
    /// `QUADCAM_PUBLISH=real`.
    pub fn system() -> Self {
        let under_cargo = std::env::var_os("CARGO_MANIFEST_DIR").is_some();
        if under_cargo && std::env::var("QUADCAM_PUBLISH").as_deref() != Ok("real") {
            return Self::unavailable(
                "gh is off in a process started by cargo (QUADCAM_PUBLISH=real turns it on).",
            );
        }
        match GH_PATHS.iter().map(Path::new).find(|p| p.is_file()) {
            Some(p) => Self::with(p.to_path_buf(), Arc::new(Process)),
            None => Self::unavailable("GitHub's gh is not installed (brew install gh)."),
        }
    }

    pub fn with(program: PathBuf, runner: Arc<dyn Runner>) -> Self {
        Gh {
            program: Ok(program),
            runner,
        }
    }

    pub fn unavailable(reason: &str) -> Self {
        Gh {
            program: Err(reason.into()),
            runner: Arc::new(Process),
        }
    }

    fn run(&self, args: &[String], cwd: &Path) -> Result<String> {
        let program = self.program.as_ref().map_err(|e| anyhow!("{e}"))?;
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let out = self
            .runner
            .run(program, &args, cwd, Duration::from_secs(300))?;
        if !out.status.success() {
            bail!(
                "gh {} failed: {}",
                args.first()
                    .map(|a| a.to_string_lossy().to_string())
                    .unwrap_or_default(),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Installed and signed in to github.com.
    pub fn status(&self) -> GhStatus {
        if let Err(e) = &self.program {
            return GhStatus {
                ready: false,
                reason: Some(e.clone()),
            };
        }
        let args = ["auth", "status", "--hostname", "github.com"].map(String::from);
        match self.run(&args, &std::env::temp_dir()) {
            Ok(_) => GhStatus {
                ready: true,
                reason: None,
            },
            Err(_) => GhStatus {
                ready: false,
                reason: Some("gh is not signed in to github.com (gh auth login).".into()),
            },
        }
    }

    /// `gh api`, with a JSON body from a file in `scratch`.
    fn api(&self, scratch: &Path, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        let mut args: Vec<String> = vec!["api".into(), "-X".into(), method.into(), path.into()];
        if let Some(b) = body {
            let f = scratch.join(format!("body-{}.json", sha256_hex(path.as_bytes())));
            std::fs::write(&f, serde_json::to_vec(b)?)?;
            args.push("--input".into());
            args.push(f.display().to_string());
        }
        let out = self.run(&args, scratch)?;
        serde_json::from_str(&out).with_context(|| format!("gh api {path} answered no JSON"))
    }

    /// Creates the release with the zip, then a branch with the previews and the index, then
    /// the pull request. `scratch` holds the request bodies.
    pub fn publish(&self, b: &Bundle, scratch: &Path) -> Result<Published> {
        std::fs::create_dir_all(scratch)?;
        let (repo, e) = (&b.repo, &b.entry);
        let t = tag(&e.id, &e.version);
        let notes = scratch.join("notes.md");
        std::fs::write(&notes, &b.notes)?;
        let release_url = self.run(
            &[
                "release".into(),
                "create".into(),
                t.clone(),
                b.zip.display().to_string(),
                "-R".into(),
                repo.clone(),
                "--title".into(),
                format!("{} ({t})", e.voice),
                "--notes-file".into(),
                notes.display().to_string(),
            ],
            scratch,
        )?;
        let pushed = (|| -> Result<Published> {
            let base = self.api(
                scratch,
                "GET",
                &format!("repos/{repo}/git/ref/heads/main"),
                None,
            )?;
            let base = str_at(&base, &["object", "sha"])?;
            let commit = self.api(
                scratch,
                "GET",
                &format!("repos/{repo}/git/commits/{base}"),
                None,
            )?;
            let tree = str_at(&commit, &["tree", "sha"])?;
            let current = self.api(
                scratch,
                "GET",
                &format!("repos/{repo}/contents/index.json?ref={base}"),
                None,
            )?;
            let text: String = str_at(&current, &["content"])?
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            let raw = base64::engine::general_purpose::STANDARD
                .decode(text)
                .context("the repo's index.json is not base64")?;
            let mut idx: VoiceIndex = serde_json::from_slice(&raw)
                .context("the repo's index.json is not a voice index")?;
            merge(&mut idx, e.clone());
            let mut files: Vec<(String, Vec<u8>)> = vec![("index.json".into(), index_bytes(&idx)?)];
            for (path, local) in &b.previews {
                files.push((path.clone(), std::fs::read(local)?));
            }
            let mut entries = Vec::new();
            for (path, bytes) in &files {
                let blob = self.api(
                    scratch,
                    "POST",
                    &format!("repos/{repo}/git/blobs"),
                    Some(&json!({
                        "content": base64::engine::general_purpose::STANDARD.encode(bytes),
                        "encoding": "base64",
                    })),
                )?;
                entries.push(json!({
                    "path": path, "mode": "100644", "type": "blob", "sha": str_at(&blob, &["sha"])?,
                }));
            }
            let new_tree = self.api(
                scratch,
                "POST",
                &format!("repos/{repo}/git/trees"),
                Some(&json!({"base_tree": tree, "tree": entries})),
            )?;
            let msg = format!("pack: {} v{}", e.id, e.version);
            let new_commit = self.api(
                scratch,
                "POST",
                &format!("repos/{repo}/git/commits"),
                Some(&json!({"message": msg, "tree": str_at(&new_tree, &["sha"])?, "parents": [base]})),
            )?;
            let br = branch(&e.id, &e.version);
            self.api(
                scratch,
                "POST",
                &format!("repos/{repo}/git/refs"),
                Some(&json!({"ref": format!("refs/heads/{br}"), "sha": str_at(&new_commit, &["sha"])?})),
            )?;
            let pr_url = self.run(
                &[
                    "pr".into(),
                    "create".into(),
                    "-R".into(),
                    repo.clone(),
                    "--base".into(),
                    "main".into(),
                    "--head".into(),
                    br.clone(),
                    "--title".into(),
                    msg,
                    "--body-file".into(),
                    notes.display().to_string(),
                ],
                scratch,
            )?;
            Ok(Published {
                release_url: release_url.clone(),
                pr_url,
                branch: br,
            })
        })();
        pushed.map_err(|err| {
            anyhow!(
                "The release {release_url} was created, but the index was not: {err:#}. Add the entry by hand (Approve again prints the commands), or delete the release and approve again."
            )
        })
    }
}

fn str_at(v: &Value, path: &[&str]) -> Result<String> {
    let mut cur = v;
    for k in path {
        cur = cur
            .get(k)
            .ok_or_else(|| anyhow!("gh answered without {}", path.join(".")))?;
    }
    cur.as_str()
        .map(String::from)
        .ok_or_else(|| anyhow!("gh answered without {}", path.join(".")))
}

/// Shell quoting for the printed commands.
fn q(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@+,".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// The commands that publish a bundle by hand: the release, then a clone of the repo with the
/// previews and the entry merged into `index.json` (macOS's `jq`), pushed on a branch, then the
/// pull request. `dir` is the bundle's folder, which holds `entry.json` and `notes.md`.
pub fn manual_commands(b: &Bundle, dir: &Path) -> Vec<String> {
    let e = &b.entry;
    let (t, br) = (tag(&e.id, &e.version), branch(&e.id, &e.version));
    let d = dir.display().to_string();
    let notes = format!("{d}/notes.md");
    let folder = preview_folder(&e.id, &e.version);
    let mut out = vec![
        format!(
            "gh release create {} {} -R {} --title {} --notes-file {}",
            q(&t),
            q(&b.zip.display().to_string()),
            q(&b.repo),
            q(&format!("{} ({t})", e.voice)),
            q(&notes)
        ),
        format!(
            "git clone https://github.com/{}.git {}",
            b.repo,
            q(&format!("{d}/repo"))
        ),
        format!("cd {}", q(&format!("{d}/repo"))),
        format!("git switch -c {}", q(&br)),
        format!("mkdir -p {}", q(&folder)),
    ];
    for (path, local) in &b.previews {
        out.push(format!(
            "cp {} {}",
            q(&local.display().to_string()),
            q(path)
        ));
    }
    out.push(format!(
        "jq --slurpfile e {} '.schema = 1 | .packs = ([.packs[] | select(.id != $e[0].id)] + $e | sort_by(.id))' index.json > index.json.new && mv index.json.new index.json",
        q(&format!("{d}/entry.json"))
    ));
    out.push(format!(
        "git add index.json {} && git commit -m {}",
        q(&folder),
        q(&format!("pack: {} v{}", e.id, e.version))
    ));
    out.push(format!("git push -u origin {}", q(&br)));
    out.push(format!(
        "gh pr create -R {} --base main --head {} --title {} --body-file {}",
        q(&b.repo),
        q(&br),
        q(&format!("pack: {} v{}", e.id, e.version)),
        q(&notes)
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("SOUNDS/en/l{i:03}.wav")).collect()
    }

    #[test]
    fn names() {
        assert_eq!(tag("en-callum", "1"), "en-callum-v1");
        assert_eq!(zip_name("en-callum", "1"), "voice-en-callum-1.zip");
        assert_eq!(
            asset_url(REPO, "en-callum", "1"),
            "https://github.com/noahkiss/quadcam-voices/releases/download/en-callum-v1/voice-en-callum-1.zip"
        );
        assert_eq!(branch("en-callum", "1.2"), "pack/en-callum-v1.2");
        check_version("1.2").unwrap();
        for bad in ["", "v1", "1.", ".1", "1 2", "1/2"] {
            assert!(check_version(bad).is_err(), "{bad}");
        }
        check_repo("owner/name.x").unwrap();
        for bad in ["owner", "/x", "a/b/c", "a/.git", "a b/c"] {
            assert!(check_repo(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn previews_take_the_sample_lines_first() {
        let files = paths(30);
        let wanted = vec![
            files[5].clone(),
            "SOUNDS/en/none.wav".into(),
            files[2].clone(),
        ];
        let p = pick_previews(&files, &wanted);
        assert_eq!(p.len(), PREVIEW_LINES);
        assert_eq!(
            &p[..3],
            &[files[5].clone(), files[2].clone(), files[0].clone()]
        );
        assert_eq!(pick_previews(&files[..3], &[]).len(), 3);
    }

    #[test]
    fn the_spot_check_is_stable_and_sized() {
        let files = paths(300);
        let a = spot_check(&files, "seed");
        assert_eq!(a.len(), 15);
        assert_eq!(a, spot_check(&files, "seed"));
        assert_ne!(a, spot_check(&files, "other"));
        assert!(a.windows(2).all(|w| w[0] < w[1]), "in the pack's order");
        assert_eq!(spot_check(&paths(100), "s").len(), 10);
        assert_eq!(spot_check(&paths(240), "s").len(), 12);
        assert_eq!(spot_check(&paths(4), "s").len(), 4);
    }

    #[test]
    fn licenses_by_provider() {
        assert!(license_for("say", "", "Samantha").is_err());
        let (l, a) = license_for("elevenlabs", "eleven_v3", "Callum")
            .unwrap()
            .unwrap();
        assert!(l.contains("commercial use"));
        assert_eq!(a, "Voice: Callum (ElevenLabs)");
        let (l, _) = license_for("openai", "Kokoro", "af_heart")
            .unwrap()
            .unwrap();
        assert_eq!(l, "Apache-2.0");
        assert!(license_for("openai", "tts-1", "alloy").unwrap().is_none());
    }

    #[test]
    fn printed_commands_quote_and_merge() {
        let b = Bundle {
            repo: REPO.into(),
            entry: serde_json::from_value(json!({
                "id": "en-x", "voice": "Mx O'Neil", "lang": "en", "provider": "elevenlabs",
                "settings": {}, "lines": 1, "bytes": 1, "sha256": "ab", "license": "l",
                "attribution": "a", "lines_csv_sha": "c", "version": "1"
            }))
            .unwrap(),
            zip: "/tmp/b dir/voice-en-x-1.zip".into(),
            previews: vec![("previews/en-x-1/01.m4a".into(), "/tmp/b dir/01.m4a".into())],
            notes: String::new(),
        };
        let c = manual_commands(&b, Path::new("/tmp/b dir"));
        assert!(c[0].starts_with("gh release create en-x-v1 '/tmp/b dir/voice-en-x-1.zip' -R noahkiss/quadcam-voices --title 'Mx O'\\''Neil (en-x-v1)'"), "{}", c[0]);
        assert!(c
            .iter()
            .any(|l| l == "cp '/tmp/b dir/01.m4a' previews/en-x-1/01.m4a"));
        assert!(c
            .iter()
            .any(|l| l.starts_with("jq --slurpfile e '/tmp/b dir/entry.json'")));
        assert!(c.last().unwrap().starts_with(
            "gh pr create -R noahkiss/quadcam-voices --base main --head pack/en-x-v1"
        ));
    }
}
