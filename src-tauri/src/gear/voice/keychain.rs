//! API keys in the macOS Keychain (design 7.4). A voice provider's key lives there and
//! nowhere else: not in `settings.json`, not in `gear.json`, not in a log, an error or a
//! test's output. `KeyStore` is the seam; `Keychain` is the real one, `MemKeys` the test one.
//! Under cargo the real Keychain stays closed unless `QUADCAM_TTS=real`, so a test never
//! reads or writes the developer's items.

use anyhow::{bail, Result};
use std::collections::BTreeMap;
use std::sync::Mutex;

/// The Keychain service every QuadCam item uses.
pub const SERVICE: &str = "app.quadcam";

/// The item of the ElevenLabs key.
pub const ELEVENLABS: &str = "elevenlabs-api-key";

/// Reads and writes secrets by account name.
pub trait KeyStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>>;
    fn set(&self, account: &str, secret: &str) -> Result<()>;
    /// Whether an item was there.
    fn delete(&self, account: &str) -> Result<bool>;
}

/// A key as the app may show it: never the secret, only enough to tell keys apart.
pub fn hint(secret: &str) -> String {
    let n = secret.chars().count();
    if n < 12 {
        return "set".into();
    }
    let tail: String = secret.chars().skip(n - 4).collect();
    format!("ends in {tail}")
}

/// Secrets in memory, for tests.
#[derive(Default)]
pub struct MemKeys(Mutex<BTreeMap<String, String>>);

impl KeyStore for MemKeys {
    fn get(&self, account: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(account).cloned())
    }
    fn set(&self, account: &str, secret: &str) -> Result<()> {
        self.0.lock().unwrap().insert(account.into(), secret.into());
        Ok(())
    }
    fn delete(&self, account: &str) -> Result<bool> {
        Ok(self.0.lock().unwrap().remove(account).is_some())
    }
}

/// The login Keychain, through the Security framework (no secret in argv).
pub struct Keychain;

fn guard() -> Result<()> {
    if std::env::var_os("CARGO_MANIFEST_DIR").is_some()
        && std::env::var("QUADCAM_TTS").as_deref() != Ok("real")
    {
        bail!("the Keychain is off in tests (QUADCAM_TTS=real turns it on)");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
impl KeyStore for Keychain {
    fn get(&self, account: &str) -> Result<Option<String>> {
        use security_framework::passwords::get_generic_password;
        guard()?;
        match get_generic_password(SERVICE, account) {
            Ok(b) => Ok(Some(String::from_utf8(b).map_err(|_| {
                anyhow::anyhow!(
                    "the Keychain item {account} is not text; delete it and set it again"
                )
            })?)),
            Err(e) if e.code() == ERR_NOT_FOUND => Ok(None),
            Err(e) => bail!("the Keychain refused the read of {account}: {e}"),
        }
    }
    fn set(&self, account: &str, secret: &str) -> Result<()> {
        use security_framework::passwords::set_generic_password;
        guard()?;
        if secret.trim().is_empty() {
            bail!("the key is empty");
        }
        set_generic_password(SERVICE, account, secret.trim().as_bytes())
            .map_err(|e| anyhow::anyhow!("the Keychain refused the write of {account}: {e}"))
    }
    fn delete(&self, account: &str) -> Result<bool> {
        use security_framework::passwords::delete_generic_password;
        guard()?;
        match delete_generic_password(SERVICE, account) {
            Ok(()) => Ok(true),
            Err(e) if e.code() == ERR_NOT_FOUND => Ok(false),
            Err(e) => bail!("the Keychain refused the delete of {account}: {e}"),
        }
    }
}

/// `errSecItemNotFound`.
#[cfg(target_os = "macos")]
const ERR_NOT_FOUND: i32 = -25300;

#[cfg(not(target_os = "macos"))]
impl KeyStore for Keychain {
    fn get(&self, _: &str) -> Result<Option<String>> {
        guard()?;
        bail!("the Keychain is a macOS feature")
    }
    fn set(&self, _: &str, _: &str) -> Result<()> {
        bail!("the Keychain is a macOS feature")
    }
    fn delete(&self, _: &str) -> Result<bool> {
        bail!("the Keychain is a macOS feature")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_keys_store_read_and_delete() {
        let k = MemKeys::default();
        assert_eq!(k.get(ELEVENLABS).unwrap(), None);
        k.set(ELEVENLABS, "sk_abcdef123456").unwrap();
        assert_eq!(
            k.get(ELEVENLABS).unwrap().as_deref(),
            Some("sk_abcdef123456")
        );
        assert!(k.delete(ELEVENLABS).unwrap());
        assert!(!k.delete(ELEVENLABS).unwrap());
    }

    #[test]
    fn a_hint_never_shows_the_key() {
        assert_eq!(hint("sk_abcdef123456"), "ends in 3456");
        assert_eq!(hint("short"), "set");
        assert!(!hint("sk_abcdef123456").contains("abcdef"));
    }

    #[test]
    fn the_real_keychain_is_closed_under_cargo() {
        let e = Keychain.get(ELEVENLABS).unwrap_err().to_string();
        assert!(e.contains("off in tests"), "{e}");
        assert!(Keychain.set(ELEVENLABS, "x").is_err());
    }
}
