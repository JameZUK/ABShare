//! Formats for secrets people handle: opaque tokens (64 hex characters) and recovery codes
//! (Crockford base32 groups such as `7kq3-x9mz-2bdf`, forgiving of case and look-alikes).
//! The server generates the random bytes and stores only hashes.

/// Crockford base32 without I, L, O, U.
const CROCKFORD: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// Opaque token text: 32 random bytes as 64 lower-case hex characters.
pub fn token_text(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether `s` has the exact shape of a token. Checked before any hashing or lookup.
pub fn is_token(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// A recovery code from 8 random bytes: 12 characters (60 bits) in three groups of four.
pub fn recovery_code(bytes: &[u8; 8]) -> String {
    let v = u64::from_be_bytes(*bytes) >> 4;
    let chars: Vec<char> = (0..12).rev().map(|i| CROCKFORD[((v >> (i * 5)) & 31) as usize] as char).collect();
    format!(
        "{}-{}-{}",
        chars[0..4].iter().collect::<String>(),
        chars[4..8].iter().collect::<String>(),
        chars[8..12].iter().collect::<String>()
    )
}

/// Normalises a typed recovery code: case, spaces and dashes ignored; O→0, I/L→1. Returns
/// `None` unless exactly 12 valid characters remain.
pub fn normalise_recovery(s: &str) -> Option<String> {
    let mut out = String::with_capacity(12);
    for c in s.chars() {
        let c = match c.to_ascii_lowercase() {
            ' ' | '-' => continue,
            'o' => '0',
            'i' | 'l' => '1',
            c => c,
        };
        if !CROCKFORD.contains(&(c as u8)) || !c.is_ascii() {
            return None;
        }
        out.push(c);
        if out.len() > 12 {
            return None;
        }
    }
    (out.len() == 12).then(|| format!("{}-{}-{}", &out[0..4], &out[4..8], &out[8..12]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        let t = token_text(&[0xab; 32]);
        assert_eq!(t.len(), 64);
        assert!(is_token(&t));
        assert!(!is_token(&t.to_uppercase()));
        assert!(!is_token(&t[..63]));
        assert!(!is_token("../../etc/passwd"));
    }

    #[test]
    fn recovery_codes_round_trip_with_typos() {
        let c = recovery_code(&[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(c.len(), 14);
        assert_eq!(normalise_recovery(&c), Some(c.clone()));
        assert_eq!(normalise_recovery(&c.to_uppercase().replace('-', " ")), Some(c.clone()));
        let with_lookalikes = c.replace('0', "O").replace('1', "l");
        assert_eq!(normalise_recovery(&with_lookalikes), Some(c));
        assert_eq!(normalise_recovery("abc"), None);
        assert_eq!(normalise_recovery("uuuu-uuuu-uuuu"), None);
        assert_eq!(normalise_recovery("aaaa-aaaa-aaaa-a"), None);
    }

    #[test]
    fn recovery_codes_differ_for_different_bytes() {
        let a = recovery_code(&[0; 8]);
        let b = recovery_code(&[0, 0, 0, 0, 0, 0, 0, 16]);
        assert_ne!(a, b);
    }
}
