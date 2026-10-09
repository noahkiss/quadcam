//! Radio voice (design 7.4): the lines QuadCam knows and the spelling rules, text-to-speech
//! providers, the render pipeline with its raw cache and normalisation, and voice packs.
//! `core/voice.rs` holds the `Core` methods; nothing here touches a device.

pub mod eleven;
pub mod keychain;
pub mod lines;
pub mod packs;
pub mod render;
pub mod tts;
pub mod wav;

use sha2::{Digest, Sha256};

/// Lowercase hex of a SHA-256.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
