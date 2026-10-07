//! The pinned module manifest: `resources/modules.toml`, compiled in. A QuadCam release also
//! attaches the same data as `modules.json`; `Manifest::newer_pins` takes a pin from it only
//! when the module is known, its version is newer, and every URL is on the module's hosts.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The manifest this build ships.
pub const BUILT_IN: &str = include_str!("../../resources/modules.toml");

/// One download of a module.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct Asset {
    pub url: String,
    /// Lowercase hex SHA-256 of the file as downloaded.
    pub sha256: String,
    /// Bytes.
    pub size: u64,
}

/// One module, pinned to one version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct Pin {
    pub title: String,
    pub about: String,
    pub version: String,
    /// SPDX expression.
    pub license: String,
    pub license_url: String,
    pub source: String,
    pub homepage: String,
    /// Hosts a newer pin's URLs may use.
    pub hosts: Vec<String>,
    /// Tool name to the path of its executable inside the unpacked assets.
    pub tools: BTreeMap<String, String>,
    pub assets: Vec<Asset>,
}

impl Pin {
    /// Total download size in bytes.
    pub fn size(&self) -> u64 {
        self.assets.iter().map(|a| a.size).sum()
    }

    fn check(&self, name: &str) -> Result<()> {
        if !valid_name(name) {
            bail!("module name {name:?} must be lowercase letters, digits and dashes");
        }
        if self.version.is_empty()
            || !self
                .version
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || ".-+_".contains(c))
        {
            bail!("{name}: version {:?} is not a plain version", self.version);
        }
        if self.assets.is_empty() {
            bail!("{name}: no assets");
        }
        if self.tools.is_empty() {
            bail!("{name}: no tools");
        }
        for (tool, path) in &self.tools {
            if !valid_name(tool) {
                bail!("{name}: tool name {tool:?} is not plain");
            }
            let p = std::path::Path::new(path);
            if p.is_absolute()
                || p.components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                bail!("{name}: tool path {path:?} must stay inside the module folder");
            }
        }
        for a in &self.assets {
            if a.sha256.len() != 64
                || !a
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                bail!("{name}: {} has no lowercase hex SHA-256", a.url);
            }
            if file_name(&a.url).is_none() {
                bail!("{name}: {} does not end in a file name", a.url);
            }
        }
        Ok(())
    }

    /// True when every asset URL is `https://<one of hosts>/...` (plain HTTP only for this
    /// Mac, which tests use).
    fn on_hosts(&self, hosts: &[String]) -> bool {
        self.assets.iter().all(|a| {
            let rest = a.url.strip_prefix("https://").or_else(|| {
                a.url
                    .strip_prefix("http://")
                    .filter(|r| r.starts_with("127.0.0.1") || r.starts_with("localhost"))
            });
            let authority = rest
                .and_then(|r| r.split(['/', '?', '#']).next())
                .unwrap_or_default();
            let host = authority.split(':').next().unwrap_or_default();
            !authority.contains('@') && hosts.iter().any(|x| x == host)
        })
    }
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The last path segment of a URL, if it is a plain file name.
pub fn file_name(url: &str) -> Option<&str> {
    let path = url.split(['?', '#']).next()?;
    let name = path.rsplit('/').next()?;
    (!name.is_empty() && name != "." && name != ".." && !name.contains('\\')).then_some(name)
}

/// Every module by name.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct Manifest(pub BTreeMap<String, Pin>);

impl Manifest {
    pub fn built_in() -> Manifest {
        Manifest::from_toml(BUILT_IN).expect("resources/modules.toml is valid")
    }

    pub fn from_toml(text: &str) -> Result<Manifest> {
        let m: Manifest = toml::from_str(text).context("reading the module manifest")?;
        m.check()?;
        Ok(m)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Manifest> {
        let m: Manifest = serde_json::from_slice(bytes).context("reading modules.json")?;
        m.check()?;
        Ok(m)
    }

    fn check(&self) -> Result<()> {
        for (name, pin) in &self.0 {
            pin.check(name)?;
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&Pin> {
        self.0.get(name)
    }

    /// Pins from `remote` that this manifest may take: a known module, a newer version, the
    /// same tools, and every URL on the hosts this manifest names for it.
    pub fn newer_pins(&self, remote: &Manifest) -> BTreeMap<String, Pin> {
        let mut out = BTreeMap::new();
        for (name, local) in &self.0 {
            let Some(r) = remote.0.get(name) else {
                continue;
            };
            if newer(&r.version, &local.version)
                && r.on_hosts(&local.hosts)
                && r.tools.keys().eq(local.tools.keys())
            {
                let mut pin = r.clone();
                // Hosts never widen through a remote pin.
                pin.hosts = local.hosts.clone();
                out.insert(name.clone(), pin);
            }
        }
        out
    }
}

/// True when version `a` is newer than `b`: numeric dot-separated parts compared in order,
/// a part that is not a number compared as text.
pub fn newer(a: &str, b: &str) -> bool {
    use std::cmp::Ordering;
    let parts = |s: &str| {
        s.split(['.', '-', '+', '_'])
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    let (pa, pb) = (parts(a), parts(b));
    for i in 0..pa.len().max(pb.len()) {
        let x = pa.get(i).map(String::as_str).unwrap_or("0");
        let y = pb.get(i).map(String::as_str).unwrap_or("0");
        let o = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            _ => x.cmp(y),
        };
        if o != Ordering::Equal {
            return o == Ordering::Greater;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_manifest_is_valid() {
        let m = Manifest::built_in();
        let ff = m.get("ffmpeg").unwrap();
        assert!(ff.tools.contains_key("ffmpeg") && ff.tools.contains_key("ffprobe"));
        assert!(m.get("esptool").is_some());
        for pin in m.0.values() {
            assert!(pin.on_hosts(&pin.hosts), "{} uses its own hosts", pin.title);
            assert!(pin.assets.iter().all(|a| a.url.starts_with("https://")));
        }
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("9.0.10", "9.0.2"));
        assert!(newer("10.0", "9.9.9"));
        assert!(!newer("9.0.2", "9.0.2"));
        assert!(!newer("9.0", "9.0.0"));
        assert!(newer("5.4.1", "5.4.0"));
    }

    #[test]
    fn remote_pins_stay_on_known_hosts() {
        let local = Manifest::built_in();
        let mut remote = local.clone();
        let ff = remote.0.get_mut("ffmpeg").unwrap();
        ff.version = "9.1".into();
        let es = remote.0.get_mut("esptool").unwrap();
        es.version = "6.0.0".into();
        es.assets[0].url = "https://example.com/esptool.tar.gz".into();
        let newer = local.newer_pins(&remote);
        assert!(newer.contains_key("ffmpeg"));
        assert!(
            !newer.contains_key("esptool"),
            "a URL off the module's hosts is ignored"
        );
    }

    #[test]
    fn bad_manifests_refuse() {
        let base = |extra: &str| {
            format!(
                "[x]\ntitle='x'\nabout='x'\nversion='1'\nlicense='MIT'\nlicense_url='u'\nsource='s'\nhomepage='h'\nhosts=['h']\n{extra}"
            )
        };
        let asset = "[[x.assets]]\nurl='https://h/x.zip'\nsha256='0000000000000000000000000000000000000000000000000000000000000000'\nsize=1\n";
        assert!(Manifest::from_toml(&base(&format!("tools={{x='x'}}\n{asset}"))).is_ok());
        assert!(Manifest::from_toml(&base(&format!("tools={{x='../x'}}\n{asset}"))).is_err());
        assert!(Manifest::from_toml(&base(&format!("tools={{x='/bin/sh'}}\n{asset}"))).is_err());
        assert!(Manifest::from_toml(&base(&format!(
            "tools={{x='x'}}\n{}",
            asset.replace("000000000000", "ABCDEF000000")
        )))
        .is_err());
    }
}
