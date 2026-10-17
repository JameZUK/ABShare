//! Stable, opaque book IDs (SPEC D17).
//!
//! ID = base32(HMAC-SHA256(id_key, key)[..16]): 26 lower-case characters. The key is
//! `asin:<ASIN>` when one is known, otherwise `path:<library id>/<relative folder>`. IDs
//! survive rescans and (with an ASIN) folder renames, can't be guessed or enumerated, and
//! never reveal paths. `id_key` is derived from the server secret for this purpose only.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

/// Length of an ID in characters (128 bits in base32 without padding).
pub const ID_LEN: usize = 26;
const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// What a book's ID is derived from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BookKey<'a> {
    Asin(&'a str),
    Path {
        library: u32,
        folder: &'a str,
    },
    /// A series or show within a library.
    Group {
        library: u32,
        name: &'a str,
    },
}

impl BookKey<'_> {
    fn bytes(&self) -> Vec<u8> {
        match self {
            Self::Asin(a) => format!("asin:{a}").into_bytes(),
            Self::Path { library, folder } => format!("path:{library}/{folder}").into_bytes(),
            Self::Group { library, name } => format!("group:{library}/{name}").into_bytes(),
        }
    }
}

/// Derives the ID for `key` under `id_key`.
pub fn book_id(id_key: &[u8; 32], key: &BookKey<'_>) -> String {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(id_key).expect("HMAC accepts any key length");
    mac.update(&key.bytes());
    let tag = mac.finalize().into_bytes();
    base32(&tag[..16])
}

fn base32(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(ID_LEN);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &b in bytes {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// Whether `s` has the exact shape of an ID. Checked before any lookup, so malformed input
/// never reaches the database.
pub fn is_valid_id(s: &str) -> bool {
    s.len() == ID_LEN && s.bytes().all(|c| ALPHABET.contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    const K: [u8; 32] = [7; 32];

    #[test]
    fn ids_have_the_right_shape_and_are_stable() {
        let a = book_id(&K, &BookKey::Asin("B0TESTASIN"));
        assert_eq!(a.len(), ID_LEN);
        assert!(is_valid_id(&a));
        assert_eq!(a, book_id(&K, &BookKey::Asin("B0TESTASIN")));
    }

    #[test]
    fn different_keys_and_secrets_give_different_ids() {
        let a = book_id(&K, &BookKey::Asin("B0TESTASIN"));
        assert_ne!(a, book_id(&[8; 32], &BookKey::Asin("B0TESTASIN")));
        assert_ne!(a, book_id(&K, &BookKey::Asin("B0TESTASIM")));
        let p1 = book_id(&K, &BookKey::Path { library: 1, folder: "A" });
        assert_ne!(p1, book_id(&K, &BookKey::Path { library: 2, folder: "A" }));
        // A folder literally named like an ASIN key can't collide with the ASIN key.
        assert_ne!(book_id(&K, &BookKey::Path { library: 1, folder: "asin:B0TESTASIN" }), a);
    }

    #[test]
    fn malformed_ids_are_refused() {
        for bad in [
            "",
            "short",
            &"a".repeat(27),
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
            "abcdefghijklmnopqrstuvwx1z",
            "../../../../etc/passwd....",
            "abcdefghijklmnopqrstuvwxy\u{e9}",
        ] {
            assert!(!is_valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn base32_matches_rfc4648_vectors() {
        // RFC 4648 test vectors, lower-cased, without padding.
        assert_eq!(base32(b"f"), "my");
        assert_eq!(base32(b"fo"), "mzxq");
        assert_eq!(base32(b"foobar"), "mzxw6ytboi");
    }
}
