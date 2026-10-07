//! Downloads. `Fetch` is the one way the module manager reaches the network, so tests swap
//! it. The real one runs `/usr/bin/curl` (HTTPS only, redirects too). A process started by
//! cargo gets a fetcher that reaches only this Mac (`127.0.0.1`, `localhost`), so no test
//! downloads from the internet, unless `QUADCAM_FETCH=real`.

use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

pub trait Fetch: Send + Sync {
    /// Downloads `url` into `dest`, replacing it.
    fn download(&self, url: &str, dest: &Path) -> Result<()>;
}

/// `/usr/bin/curl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Curl {
    /// Only `http://127.0.0.1` and `http://localhost` URLs (tests and fixture servers).
    pub loopback_only: bool,
}

fn is_loopback(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority
        .rsplit_once(':')
        .filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit()))
        .map_or(authority, |(h, _)| h);
    !authority.contains('@') && (host == "127.0.0.1" || host == "localhost")
}

impl Fetch for Curl {
    fn download(&self, url: &str, dest: &Path) -> Result<()> {
        let loopback = is_loopback(url);
        if self.loopback_only && !loopback {
            bail!("downloads are off in tests (QUADCAM_FETCH=real turns them on): {url}");
        }
        if !loopback && !url.starts_with("https://") {
            bail!("QuadCam downloads only over HTTPS: {url}");
        }
        let proto = if loopback { "=http" } else { "=https" };
        let out = Command::new("/usr/bin/curl")
            .args(["--fail", "--silent", "--show-error", "--location"])
            .args(["--proto", proto, "--proto-redir", proto])
            .args(["--connect-timeout", "20", "--max-time", "1800"])
            .args([
                "--user-agent",
                concat!("quadcam/", env!("CARGO_PKG_VERSION")),
            ])
            .arg("--output")
            .arg(dest)
            .args(["--url", url])
            .output()
            .context("running curl")?;
        if !out.status.success() {
            let _ = std::fs::remove_file(dest);
            bail!(
                "The download failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }
}

/// True when this process may reach the internet: not started by cargo, or
/// `QUADCAM_FETCH=real`.
pub fn network_allowed(fetch_env: Option<&str>, under_cargo: bool) -> bool {
    fetch_env == Some("real") || !under_cargo
}

/// The fetcher for this process (see the module docs).
pub fn real_fetch() -> Curl {
    let env = std::env::var("QUADCAM_FETCH").ok();
    Curl {
        loopback_only: !network_allowed(
            env.as_deref(),
            std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_processes_stay_offline() {
        assert!(network_allowed(None, false));
        assert!(!network_allowed(None, true));
        assert!(network_allowed(Some("real"), true));
        assert!(!network_allowed(Some("yes"), true));
        if std::env::var("QUADCAM_FETCH").as_deref() != Ok("real") {
            assert!(real_fetch().loopback_only);
            let d = tempfile::tempdir().unwrap();
            let e = real_fetch()
                .download("https://example.com/x", &d.path().join("x"))
                .unwrap_err();
            assert!(e.to_string().contains("off in tests"), "{e}");
        }
    }

    #[test]
    fn loopback_means_this_mac() {
        assert!(is_loopback("http://127.0.0.1:8080/x.zip"));
        assert!(is_loopback("http://localhost/x"));
        assert!(!is_loopback("http://127.0.0.1:80@example.com/x"));
        assert!(!is_loopback("http://127.0.0.1.example.com/x"));
        assert!(!is_loopback("https://127.0.0.1/x"));
    }

    #[test]
    fn plain_http_refuses() {
        let d = tempfile::tempdir().unwrap();
        let e = Curl {
            loopback_only: false,
        }
        .download("http://example.com/x", &d.path().join("x"))
        .unwrap_err();
        assert!(e.to_string().contains("HTTPS"), "{e}");
    }
}
