//! The binding phrase and its UID. ExpressLRS turns the phrase into a 6-byte UID with MD5
//! over a fixed text; QuadCam writes the same hash to the image's options block. The phrase
//! is user data: it lives in one setting, and nothing here prints it. What plans, reports and
//! logs show is the UID's short fingerprint.
//!
//! MD5 is the format's choice, not a security choice; it is written here (RFC 1321) to avoid
//! a dependency for sixty lines.

/// The 6-byte UID for a phrase.
pub fn uid_of(phrase: &str) -> [u8; 6] {
    let d = md5(format!("-DMY_BINDING_PHRASE=\"{phrase}\"").as_bytes());
    [d[0], d[1], d[2], d[3], d[4], d[5]]
}

/// A short, non-reversible label for a UID, for plans and reports: the first 8 hex digits of
/// the SHA-256 of the six bytes. The UID itself goes over the air in every packet, but a
/// plan shows only this.
pub fn fingerprint(uid: &[u8; 6]) -> String {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(uid);
    h[..4].iter().map(|b| format!("{b:02x}")).collect()
}

const S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

fn k(i: usize) -> u32 {
    ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32
}

pub fn md5(data: &[u8]) -> [u8; 16] {
    let (mut a0, mut b0, mut c0, mut d0) =
        (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_le_bytes());
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk
            .chunks(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        #[allow(clippy::needless_range_loop)]
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k(i)).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    for (i, v) in [a0, b0, c0, d0].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn md5_matches_the_rfc_vectors() {
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            hex(&md5(b"The quick brown fox jumps over the lazy dog")),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
        assert_eq!(hex(&md5(&[b'a'; 200])), "887f30b43b2867f4a9accceee7d16e6c");
    }

    #[test]
    fn the_uid_is_the_first_six_bytes_of_the_phrase_hash() {
        assert_eq!(uid_of("test-phrase"), [205, 241, 60, 142, 27, 69]);
        assert_ne!(uid_of("test-phrase"), uid_of("other"));
    }

    #[test]
    fn the_fingerprint_hides_the_uid() {
        let f = fingerprint(&uid_of("test-phrase"));
        assert_eq!(f.len(), 8);
        assert!(!f.contains("cdf1"));
    }
}
