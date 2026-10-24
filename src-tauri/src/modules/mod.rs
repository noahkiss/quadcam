//! The module manager (`docs/gear-design.md`, 7.10): tools QuadCam does not ship, downloaded
//! from their upstream on the user's request into
//! `~/Library/Application Support/app.quadcam/modules/<name>/<version>/`.
//!
//! - `manifest`: the pinned versions (`resources/modules.toml`) and newer pins from a release's
//!   `modules.json`.
//! - `fetch`: the one way to the network (`/usr/bin/curl`); offline under cargo.
//! - `install`: download, checksum, unpack, quarantine and signature, `installed.json`.
//! - `run`: the hash check before each run, and the child-process runner.
//!
//! `media::find_tools` asks `Modules::tool` for ffmpeg and ffprobe before it looks in Homebrew.

pub mod fetch;
pub mod install;
pub mod manifest;
pub mod run;

use anyhow::{bail, Context, Result};
use fetch::Fetch;
use install::Installed;
use manifest::{Manifest, Pin};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Output;
use std::sync::Arc;
use std::time::Duration;

/// Where a release publishes its pins. A check reads it and its signature, `<url>.sig`;
/// nothing applies until an install.
pub const NEWEST_URL: &str =
    "https://github.com/noahkiss/quadcam/releases/latest/download/modules.json";

/// The release key's public half (base64 of the minisign public key file), the one the app
/// updater checks with: `plugins.updater.pubkey` in `tauri.conf.json`.
pub fn release_pubkey() -> String {
    let conf: serde_json::Value =
        serde_json::from_str(include_str!("../../tauri.conf.json")).expect("tauri.conf.json");
    conf["plugins"]["updater"]["pubkey"]
        .as_str()
        .expect("tauri.conf.json names the updater's pubkey")
        .to_string()
}

/// Checks `data` against a signature in `tauri signer sign`'s form (base64 of a minisign
/// signature file) with `pubkey` in the updater's form.
pub fn verify_signed(data: &[u8], sig: &[u8], pubkey: &str) -> Result<()> {
    use base64::Engine;
    let b64 = |b: &[u8]| -> Result<String> {
        let raw = base64::engine::general_purpose::STANDARD.decode(b.trim_ascii())?;
        Ok(String::from_utf8(raw)?)
    };
    let key = minisign_verify::PublicKey::decode(&b64(pubkey.as_bytes())?)
        .map_err(|e| anyhow::anyhow!("the release key: {e}"))?;
    let sig = b64(sig)
        .ok()
        .and_then(|s| minisign_verify::Signature::decode(&s).ok())
        .context("modules.json has no readable signature; the update is ignored.")?;
    key.verify(data, &sig, true).map_err(|_| {
        anyhow::anyhow!(
            "modules.json is not signed by QuadCam's release key; the update is ignored."
        )
    })
}

/// One module: its pin, a newer pin from the last check, and what is installed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct ModuleStatus {
    pub name: String,
    /// The version this QuadCam release pins.
    pub pinned: Pin,
    /// A newer pin from the last check, if any.
    pub newest: Option<Pin>,
    pub installed: Option<Installed>,
    /// The installed version's folder.
    pub folder: Option<PathBuf>,
    /// An install would change the version: a newer pin than the installed one.
    pub update: bool,
    /// Why an installed module cannot run (a changed or missing file).
    pub problem: Option<String>,
}

impl ModuleStatus {
    /// The pin an install takes: the newest known.
    pub fn target(&self) -> &Pin {
        self.newest.as_ref().unwrap_or(&self.pinned)
    }
}

/// The module manager. Every path comes from `paths`, so a test with `HOME` in a temp folder
/// touches nothing real; tests also pass their own manifest and fetcher.
#[derive(Clone)]
pub struct Modules {
    /// `<support>/modules`.
    pub root: PathBuf,
    /// `<cache>/modules`: downloads and the last check.
    pub cache: PathBuf,
    pub manifest: Manifest,
    pub newest_url: String,
    /// The key `modules.json` must be signed with (`release_pubkey`'s form).
    pub pubkey: String,
    fetch: Arc<dyn Fetch>,
    runner: Arc<dyn run::Runner>,
}

impl Default for Modules {
    fn default() -> Self {
        Modules::new(
            crate::paths::support_dir_or_stand_in().join("modules"),
            crate::paths::cache_dir().join("modules"),
            Manifest::built_in(),
            NEWEST_URL.into(),
            Arc::new(fetch::real_fetch()),
        )
    }
}

impl Modules {
    pub fn new(
        root: PathBuf,
        cache: PathBuf,
        manifest: Manifest,
        newest_url: String,
        fetch: Arc<dyn Fetch>,
    ) -> Modules {
        Modules {
            root,
            cache,
            manifest,
            newest_url,
            pubkey: release_pubkey(),
            fetch,
            runner: Arc::new(run::Process),
        }
    }

    pub fn with_pubkey(mut self, pubkey: String) -> Modules {
        self.pubkey = pubkey;
        self
    }

    pub fn with_runner(mut self, runner: Arc<dyn run::Runner>) -> Modules {
        self.runner = runner;
        self
    }

    fn pin(&self, name: &str) -> Result<&Pin> {
        self.manifest.get(name).with_context(|| {
            format!(
                "unknown module {name:?}; modules: {}",
                self.manifest
                    .0
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
    }

    fn newest_file(&self) -> PathBuf {
        self.cache.join("newest.json")
    }

    /// Newer pins from the last check, still checked against this manifest.
    fn newest(&self) -> BTreeMap<String, Pin> {
        std::fs::read(self.newest_file())
            .ok()
            .and_then(|b| Manifest::from_json(&b).ok())
            .map(|m| self.manifest.newer_pins(&m))
            .unwrap_or_default()
    }

    /// Holds `<root>/.lock` so the app and the CLI never install or remove at once.
    fn lock(&self) -> Result<std::fs::File> {
        std::fs::create_dir_all(&self.root)
            .with_context(|| format!("creating {}", self.root.display()))?;
        let f = std::fs::File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join(".lock"))
            .context("opening the modules lock")?;
        f.lock().context("locking the modules folder")?;
        Ok(f)
    }

    fn status_with(&self, name: &str, newest: &BTreeMap<String, Pin>) -> Result<ModuleStatus> {
        let pinned = self.pin(name)?.clone();
        let found = install::installed(&self.root.join(name));
        let newest = newest.get(name).cloned();
        let problem = found.as_ref().and_then(|(rec, dir)| {
            rec.tools
                .iter()
                .find_map(|(tool, t)| run::verify(tool, &dir.join(&t.path), &t.sha256).err())
                .map(|e| e.to_string())
        });
        let target = newest.as_ref().unwrap_or(&pinned);
        let update = found
            .as_ref()
            .is_some_and(|(rec, _)| manifest::newer(&target.version, &rec.version));
        let (installed, folder) = found.map(|(r, d)| (Some(r), Some(d))).unwrap_or_default();
        Ok(ModuleStatus {
            name: name.to_string(),
            pinned,
            newest,
            installed,
            folder,
            update,
            problem,
        })
    }

    pub fn status(&self, name: &str) -> Result<ModuleStatus> {
        self.status_with(name, &self.newest())
    }

    /// Every module in the manifest. Reads only local files.
    pub fn list(&self) -> Vec<ModuleStatus> {
        let newest = self.newest();
        self.manifest
            .0
            .keys()
            .filter_map(|n| self.status_with(n, &newest).ok())
            .collect()
    }

    /// The text an install shows before it downloads anything.
    pub fn license_prompt(pin: &Pin) -> String {
        format!(
            "Installing {} {} downloads {:.1} MB from {}. Its license is {} ({}); its source is at {}.",
            pin.title,
            pin.version,
            pin.size() as f64 / 1e6,
            pin.homepage,
            pin.license,
            pin.license_url,
            pin.source
        )
    }

    /// Downloads and installs the newest known pin of `name`. `confirm` says the person saw
    /// the license prompt; without it nothing is downloaded. Also how an update installs.
    pub fn install(&self, name: &str, confirm: bool) -> Result<ModuleStatus> {
        let before = self.status(name)?;
        let pin = before.target().clone();
        if !confirm {
            bail!(
                "Refused: the install needs confirm=true. {} Show this to the person; install \
                 once they agree.",
                Self::license_prompt(&pin)
            );
        }
        let _lock = self.lock()?;
        install::install(
            self.fetch.as_ref(),
            &self.cache,
            &self.root.join(name),
            name,
            &pin,
        )?;
        self.status(name)
    }

    /// Deletes the module's folder. A feature that needs it asks again.
    pub fn remove(&self, name: &str) -> Result<ModuleStatus> {
        self.pin(name)?;
        let _lock = self.lock()?;
        let dir = self.root.join(name);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        }
        self.status(name)
    }

    /// Reads the newest pins a QuadCam release published and keeps the newer ones for the
    /// next install. The pins count only with a signature by the release key. Installs
    /// nothing.
    pub fn check(&self) -> Result<Vec<ModuleStatus>> {
        std::fs::create_dir_all(&self.cache)
            .with_context(|| format!("creating {}", self.cache.display()))?;
        let fetch = |url: &str, name: &str| -> Result<Vec<u8>> {
            let part = self.cache.join(name);
            let got = self
                .fetch
                .download(url, &part)
                .context("Could not read the newest module pins")
                .and_then(|()| Ok(std::fs::read(&part)?));
            let _ = std::fs::remove_file(&part);
            got
        };
        let bytes = fetch(&self.newest_url, "modules.json.part")?;
        let sig = fetch(&format!("{}.sig", self.newest_url), "modules.json.sig.part")?;
        verify_signed(&bytes, &sig, &self.pubkey)?;
        let remote = Manifest::from_json(&bytes)?;
        let keep = Manifest(self.manifest.newer_pins(&remote));
        std::fs::write(self.newest_file(), serde_json::to_vec_pretty(&keep)?)?;
        Ok(self.list())
    }

    /// The installed, checked executable of `tool`, or None when no installed module has it.
    /// A file changed since install is an error, never a silent fallback.
    pub fn tool(&self, tool: &str) -> Result<Option<PathBuf>> {
        for name in self.manifest.0.keys() {
            let Some((rec, dir)) = install::installed(&self.root.join(name)) else {
                continue;
            };
            if let Some(t) = rec.tools.get(tool) {
                let p = dir.join(&t.path);
                run::verify(tool, &p, &t.sha256)?;
                return Ok(Some(p));
            }
        }
        Ok(None)
    }

    /// Runs a module's tool as a child process in `<cache>/run`, with a minimal environment
    /// and a timeout.
    pub fn run(&self, tool: &str, args: &[OsString], timeout: Duration) -> Result<Output> {
        let program = self.tool(tool)?.with_context(|| {
            format!("{tool} is not installed. Install it in Settings > Modules.")
        })?;
        let cwd = self.cache.join("run");
        std::fs::create_dir_all(&cwd)?;
        self.runner.run(&program, args, &cwd, timeout)
    }

    /// The folder of an installed module, if any.
    pub fn folder(&self, name: &str) -> Option<PathBuf> {
        install::installed(&self.root.join(name)).map(|(_, d)| d)
    }
}
