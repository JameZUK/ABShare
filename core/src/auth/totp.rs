//! Time-based one-time passwords (RFC 6238, SPEC §5.5): SHA-1, 30 s steps, 6 digits, one step
//! of clock skew either way. Codes are single use: a step at or before the last one used is
//! refused. All candidate steps are compared in constant time, with no early exit.

use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;
use subtle::ConstantTimeEq;

pub const STEP_SECS: i64 = 30;
pub const DIGITS: u32 = 6;
pub const SKEW_STEPS: i64 = 1;
/// Secret length in bytes (160 bits, as RFC 4226 recommends).
pub const SECRET_LEN: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TotpError {
    /// Not six ASCII digits.
    Malformed,
    /// Matches no step in the window.
    Wrong,
    /// Matches a step that was already used (or an earlier one).
    Replayed,
}

/// HOTP value for `counter` (RFC 4226), truncated to [`DIGITS`].
pub fn hotp(secret: &[u8], counter: u64) -> u32 {
    let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&counter.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let off = (h[h.len() - 1] & 0x0f) as usize;
    let bin = (u32::from(h[off]) & 0x7f) << 24
        | u32::from(h[off + 1]) << 16
        | u32::from(h[off + 2]) << 8
        | u32::from(h[off + 3]);
    bin % 10u32.pow(DIGITS)
}

/// The step containing `unix_secs`.
pub fn step_at(unix_secs: i64) -> i64 {
    unix_secs.div_euclid(STEP_SECS)
}

/// Checks `code` at `now`. On success returns the step it matched, which the caller stores
/// as the new last-used step.
pub fn verify(secret: &[u8], code: &str, now: i64, last_used_step: Option<i64>) -> Result<i64, TotpError> {
    let code = code.trim();
    if code.len() != DIGITS as usize || !code.bytes().all(|c| c.is_ascii_digit()) {
        return Err(TotpError::Malformed);
    }
    let current = step_at(now);
    let mut matched: Option<i64> = None;
    for d in -SKEW_STEPS..=SKEW_STEPS {
        let step = current + d;
        if step < 0 {
            continue;
        }
        let want = format!("{:0width$}", hotp(secret, step as u64), width = DIGITS as usize);
        // Every candidate is computed and compared; the result is only read afterwards.
        if bool::from(want.as_bytes().ct_eq(code.as_bytes())) && matched.is_none() {
            matched = Some(step);
        }
    }
    match matched {
        None => Err(TotpError::Wrong),
        Some(step) if last_used_step.is_some_and(|last| step <= last) => Err(TotpError::Replayed),
        Some(step) => Ok(step),
    }
}

/// RFC 4648 base32, upper case, no padding: how authenticator apps expect the secret.
pub fn secret_base32(secret: &[u8]) -> String {
    const A: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for &b in secret {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(A[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(A[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// The `otpauth://` URI for enrolment (shown as a QR code and as text).
pub fn otpauth_uri(issuer: &str, account: &str, secret: &[u8]) -> String {
    let enc = |s: &str| -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect()
    };
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits={DIGITS}&period={STEP_SECS}",
        enc(issuer),
        enc(account),
        secret_base32(secret),
        enc(issuer)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const RFC_SECRET: &[u8] = b"12345678901234567890";

    /// RFC 6238 appendix B (SHA-1), last six digits.
    #[test]
    fn rfc_6238_vectors() {
        for (t, want) in [
            (59, 287_082),
            (1_111_111_109, 81_804),
            (1_111_111_111, 50_471),
            (1_234_567_890, 5_924),
            (2_000_000_000, 279_037),
        ] {
            assert_eq!(hotp(RFC_SECRET, step_at(t) as u64), want, "t={t}");
        }
    }

    #[test]
    fn a_current_code_verifies_and_skew_is_one_step() {
        let now = 1_234_567_890;
        let code = |t: i64| format!("{:06}", hotp(RFC_SECRET, step_at(t) as u64));
        assert_eq!(verify(RFC_SECRET, &code(now), now, None), Ok(step_at(now)));
        assert!(verify(RFC_SECRET, &code(now - 30), now, None).is_ok());
        assert!(verify(RFC_SECRET, &code(now + 30), now, None).is_ok());
        assert_eq!(verify(RFC_SECRET, &code(now - 90), now, None), Err(TotpError::Wrong));
    }

    #[test]
    fn a_code_cannot_be_used_twice_or_after_a_later_one() {
        let now = 1_234_567_890;
        let c = format!("{:06}", hotp(RFC_SECRET, step_at(now) as u64));
        let step = verify(RFC_SECRET, &c, now, None).expect("first use");
        assert_eq!(verify(RFC_SECRET, &c, now, Some(step)), Err(TotpError::Replayed));
        let earlier = format!("{:06}", hotp(RFC_SECRET, (step - 1) as u64));
        assert_eq!(verify(RFC_SECRET, &earlier, now, Some(step)), Err(TotpError::Replayed));
    }

    #[test]
    fn malformed_codes_are_refused() {
        for bad in ["", "12345", "1234567", "12a456", "１２３４５６", "123 456", "-12345"] {
            assert_eq!(verify(RFC_SECRET, bad, 59, None), Err(TotpError::Malformed), "{bad:?}");
        }
        assert!(verify(RFC_SECRET, " 287082 ", 59, None).is_ok());
    }

    #[test]
    fn base32_and_uri() {
        assert_eq!(secret_base32(b"foobar"), "MZXW6YTBOI");
        let u = otpauth_uri("ABShare", "ada example", b"foobar");
        assert_eq!(
            u,
            "otpauth://totp/ABShare:ada%20example?secret=MZXW6YTBOI&issuer=ABShare&algorithm=SHA1&digits=6&period=30"
        );
    }
}
