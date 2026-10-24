//! Download, check, unpack, sign and record one module version.
//!
//! The order is the safety: a download is checked against its pinned SHA-256 before anything
//! else touches it (a mismatch deletes it); only then is it unpacked, has its quarantine
//! attribute removed and its signature checked: a pin that names a Team ID needs that
//! team's signature, and an unsigned binary of a pin without one gets a local ad-hoc
//! signature. `installed.json` records each tool's hash after signing, and every run checks it.

use super::fetch::Fetch;
use super::manifest::{file_name, Asset, Pin};
use super::run::sha256_file;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// How a tool's executable is signed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Signing {
    /// The upstream signature verified.
    Upstream,
    /// Unsigned upstream; QuadCam signed it ad hoc after the checksum matched.
    AdHoc,
    /// Not a Mach-O binary (a script); nothing to sign.
    NotBinary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct InstalledTool {
    /// The executable, relative to the version folder.
    pub path: String,
    /// SHA-256 of the executable as installed (after any ad-hoc signature).
    pub sha256: String,
    pub signing: Signing,
}

/// `installed.json` in a module's version folder.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct Installed {
    pub name: String,
    pub version: String,
    /// RFC 3339.
    pub installed_at: String,
    pub license: String,
    pub license_url: String,
    pub source: String,
    pub homepage: String,
    /// Each download's URL and SHA-256.
    pub assets: Vec<Asset>,
    pub tools: BTreeMap<String, InstalledTool>,
    /// Bytes on disk.
    pub size: u64,
}

pub const RECORD: &str = "installed.json";

/// The pinned file in the download cache, fetched when it is missing or does not match.
pub fn download(fetch: &dyn Fetch, cache: &Path, asset: &Asset) -> Result<PathBuf> {
    let name = file_name(&asset.url).context("the download URL has no file name")?;
    let dir = cache.join(&asset.sha256);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let dest = dir.join(name);
    if dest.is_file() && sha256_file(&dest)? == asset.sha256 {
        return Ok(dest);
    }
    let part = dir.join(format!("{name}.part"));
    fetch.download(&asset.url, &part)?;
    let got = sha256_file(&part)?;
    if got != asset.sha256 {
        let _ = std::fs::remove_file(&part);
        bail!("The download does not match the expected checksum.");
    }
    std::fs::rename(&part, &dest)?;
    Ok(dest)
}

fn run(cmd: &mut Command, what: &str) -> Result<()> {
    let out = cmd.output().with_context(|| format!("running {what}"))?;
    if !out.status.success() {
        bail!(
            "{what} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Unpacks a checked download into `dir`: zip with `ditto`, tar.gz with `tar`, anything
/// else copied as it is.
pub fn unpack(file: &Path, dir: &Path) -> Result<()> {
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name.ends_with(".zip") {
        run(
            Command::new("/usr/bin/ditto")
                .arg("-x")
                .arg("-k")
                .arg(file)
                .arg(dir),
            "unzipping",
        )
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        run(
            Command::new("/usr/bin/tar")
                .arg("-xzf")
                .arg(file)
                .arg("-C")
                .arg(dir),
            "unpacking",
        )
    } else {
        std::fs::copy(file, dir.join(&name))
            .map(|_| ())
            .context("copying")
    }
}

fn is_mach_o(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    use std::io::Read;
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && matches!(
            magic,
            [0xcf, 0xfa, 0xed, 0xfe] | [0xce, 0xfa, 0xed, 0xfe] | [0xca, 0xfe, 0xba, 0xbe]
        )
}

/// With `team`, the executable must carry a valid signature by that Apple Team ID, or the
/// install fails. Without one, keeps an upstream signature that verifies and signs ad hoc
/// otherwise.
fn sign(path: &Path, team: Option<&str>) -> Result<Signing> {
    if let Some(team) = team {
        let requirement =
            format!("=anchor apple generic and certificate leaf[subject.OU] = \"{team}\"");
        let ok = is_mach_o(path)
            && Command::new("/usr/bin/codesign")
                .args(["--verify", "--strict", "-R"])
                .arg(requirement)
                .arg(path)
                .output()
                .is_ok_and(|o| o.status.success());
        if !ok {
            bail!(
                "{} is not signed by its maker (Team ID {team}); nothing was installed.",
                path.file_name().unwrap_or_default().to_string_lossy()
            );
        }
        return Ok(Signing::Upstream);
    }
    if !is_mach_o(path) {
        return Ok(Signing::NotBinary);
    }
    let ok = Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict"])
        .arg(path)
        .output()
        .is_ok_and(|o| o.status.success());
    if ok {
        return Ok(Signing::Upstream);
    }
    run(
        Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-"])
            .arg(path),
        "codesign",
    )?;
    Ok(Signing::AdHoc)
}

/// Removes the quarantine attribute from every file under `dir`. Called only after the
/// download's checksum matched.
fn clear_quarantine(dir: &Path) {
    let _ = Command::new("/usr/bin/xattr")
        .args(["-r", "-d", "com.apple.quarantine"])
        .arg(dir)
        .output();
}

fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(t) if t.is_file() => total += e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => {}
            }
        }
    }
    total
}

/// Downloads, checks and unpacks `pin` into `module_dir/<version>`, writes `installed.json`,
/// and removes every other version of the module. The new version is complete before the
/// old one goes.
pub fn install(
    fetch: &dyn Fetch,
    cache: &Path,
    module_dir: &Path,
    name: &str,
    pin: &Pin,
) -> Result<Installed> {
    let files = pin
        .assets
        .iter()
        .map(|a| download(fetch, cache, a))
        .collect::<Result<Vec<_>>>()?;
    std::fs::create_dir_all(module_dir)
        .with_context(|| format!("creating {}", module_dir.display()))?;
    let staging = module_dir.join(format!(".{}.{}.tmp", pin.version, std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let done = (|| -> Result<Installed> {
        for f in &files {
            unpack(f, &staging)?;
        }
        clear_quarantine(&staging);
        let root = staging.canonicalize()?;
        let mut tools = BTreeMap::new();
        for (tool, rel) in &pin.tools {
            let p = staging.join(rel);
            let real = p
                .canonicalize()
                .with_context(|| format!("{tool} is not in the download ({rel})"))?;
            if !real.starts_with(&root) || !real.is_file() {
                bail!("{tool} is not a file inside the module ({rel})");
            }
            use std::os::unix::fs::PermissionsExt;
            let mut perm = std::fs::metadata(&real)?.permissions();
            perm.set_mode(perm.mode() | 0o755);
            std::fs::set_permissions(&real, perm)?;
            let signing = sign(&real, pin.team_id.as_deref())?;
            tools.insert(
                tool.clone(),
                InstalledTool {
                    path: rel.clone(),
                    sha256: sha256_file(&real)?,
                    signing,
                },
            );
        }
        let rec = Installed {
            name: name.to_string(),
            version: pin.version.clone(),
            installed_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            license: pin.license.clone(),
            license_url: pin.license_url.clone(),
            source: pin.source.clone(),
            homepage: pin.homepage.clone(),
            assets: pin.assets.clone(),
            tools,
            size: dir_size(&staging),
        };
        std::fs::write(staging.join(RECORD), serde_json::to_vec_pretty(&rec)?)?;
        Ok(rec)
    })();
    let rec = match done {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    let dest = module_dir.join(&pin.version);
    if dest.exists() {
        std::fs::remove_dir_all(&dest).with_context(|| format!("removing {}", dest.display()))?;
    }
    std::fs::rename(&staging, &dest)?;
    for e in std::fs::read_dir(module_dir)?.flatten() {
        if e.file_name() != pin.version.as_str() && e.path().is_dir() {
            std::fs::remove_dir_all(e.path())
                .with_context(|| format!("removing {}", e.path().display()))?;
        }
    }
    Ok(rec)
}

/// The installed version of a module, read from its folder (the newest when there are
/// several), with that version's folder.
pub fn installed(module_dir: &Path) -> Option<(Installed, PathBuf)> {
    let found: Vec<(Installed, PathBuf)> = std::fs::read_dir(module_dir)
        .ok()?
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| {
            let dir = e.path();
            let rec: Installed =
                serde_json::from_slice(&std::fs::read(dir.join(RECORD)).ok()?).ok()?;
            Some((rec, dir))
        })
        .collect();
    // Two folders of the same version (a copy) compare equal: a total order.
    let order = |a: &str, b: &str| {
        use super::manifest::newer;
        if newer(a, b) {
            std::cmp::Ordering::Greater
        } else if newer(b, a) {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    };
    found
        .into_iter()
        .max_by(|a, b| order(&a.0.version, &b.0.version).then_with(|| a.1.cmp(&b.1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_picks_the_newest_and_takes_a_copied_folder() {
        let d = tempfile::tempdir().unwrap();
        let rec = |dir: &str, version: &str| {
            let r = Installed {
                name: "x".into(),
                version: version.into(),
                installed_at: String::new(),
                license: String::new(),
                license_url: String::new(),
                source: String::new(),
                homepage: String::new(),
                assets: vec![],
                tools: BTreeMap::new(),
                size: 0,
            };
            std::fs::create_dir_all(d.path().join(dir)).unwrap();
            std::fs::write(
                d.path().join(dir).join(RECORD),
                serde_json::to_vec(&r).unwrap(),
            )
            .unwrap();
        };
        // Many folders of one version: a comparator without Equal may panic in the sort.
        for i in 0..40 {
            rec(&format!("1.0 copy {i}"), "1.0");
        }
        rec("2.0", "2.0");
        let (r, dir) = installed(d.path()).unwrap();
        assert_eq!(r.version, "2.0");
        assert!(dir.ends_with("2.0"));
    }

    #[test]
    fn mach_o_detection() {
        assert!(is_mach_o(Path::new("/bin/ls")));
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("s");
        std::fs::write(&f, "#!/bin/sh\n").unwrap();
        assert!(!is_mach_o(&f));
        assert_eq!(sign(&f, None).unwrap(), Signing::NotBinary);
        assert!(
            sign(&f, Some("QWXF6GB4AV")).is_err(),
            "a script has no team signature"
        );
    }

    #[test]
    fn unsigned_binaries_get_an_ad_hoc_signature() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("true");
        std::fs::copy("/usr/bin/true", &f).unwrap();
        // Strip the system signature, as an unsigned upstream build would come.
        run(
            Command::new("/usr/bin/codesign")
                .arg("--remove-signature")
                .arg(&f),
            "codesign",
        )
        .unwrap();
        let e = sign(&f, Some("QWXF6GB4AV")).unwrap_err().to_string();
        assert!(e.contains("not signed by its maker"), "{e}");
        assert_eq!(sign(&f, None).unwrap(), Signing::AdHoc);
        assert!(
            sign(&f, Some("QWXF6GB4AV")).is_err(),
            "an ad-hoc signature is no team's"
        );
        assert_eq!(
            sign(&f, None).unwrap(),
            Signing::Upstream,
            "the signature now verifies"
        );
    }
}
