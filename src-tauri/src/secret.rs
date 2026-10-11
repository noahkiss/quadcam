//! The one way QuadCam describes a secret it holds (an API key, the ExpressLRS binding
//! phrase): the first and last few characters and the length, never more than half the
//! value. Every surface (key status, settings reads, the Settings window, MCP, CLI) uses it.

use serde::{Deserialize, Serialize};

/// A secret as QuadCam may show it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct Hint {
    /// `sk_9…8d4d (51 chars)`; `(3 chars)` for a value under 4 characters.
    pub hint: String,
    pub prefix: String,
    pub suffix: String,
    /// Characters, not bytes.
    pub length: u32,
}

/// How many characters show at each end: 4 for 16 or more, else a quarter (rounded down).
fn shown(len: usize) -> usize {
    if len >= 16 {
        4
    } else {
        len / 4
    }
}

/// The hint of `secret`. Both ends together never exceed half its characters.
pub fn hint(secret: &str) -> Hint {
    let chars: Vec<char> = secret.chars().collect();
    let len = chars.len();
    let n = shown(len);
    let prefix: String = chars[..n].iter().collect();
    let suffix: String = chars[len - n..].iter().collect();
    let hint = if n == 0 {
        format!("({len} chars)")
    } else {
        format!("{prefix}…{suffix} ({len} chars)")
    };
    Hint {
        hint,
        prefix,
        suffix,
        length: len as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_value_shows_four_at_each_end() {
        let k = "sk_9a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c18d4d";
        let h = hint(k);
        assert_eq!(h.hint, format!("sk_9…8d4d ({} chars)", k.len()));
        assert_eq!((h.prefix.as_str(), h.suffix.as_str()), ("sk_9", "8d4d"));
        assert_eq!(h.length as usize, k.len());
        assert_eq!(hint("0123456789abcdef").hint, "0123…cdef (16 chars)");
    }

    #[test]
    fn a_medium_value_shows_a_quarter_at_each_end() {
        assert_eq!(hint("0123456789abcde").hint, "012…cde (15 chars)");
        assert_eq!(hint("01234567").hint, "01…67 (8 chars)");
        assert_eq!(hint("abcd").hint, "a…d (4 chars)");
    }

    #[test]
    fn a_short_value_shows_only_its_length() {
        assert_eq!(hint("abc").hint, "(3 chars)");
        assert_eq!(hint("abc").prefix, "");
        assert_eq!(hint("").hint, "(0 chars)");
    }

    #[test]
    fn a_hint_never_shows_more_than_half() {
        for len in 0..80 {
            let v: String = (0..len)
                .map(|i| char::from(b'a' + (i % 26) as u8))
                .collect();
            let h = hint(&v);
            let shown = h.prefix.chars().count() + h.suffix.chars().count();
            assert!(2 * shown <= len, "{len}: {}", h.hint);
            assert_eq!(h.length as usize, len);
        }
        // Characters, not bytes.
        assert_eq!(hint("ééééééééé").hint, "éé…éé (9 chars)");
    }
}
