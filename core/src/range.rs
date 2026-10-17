//! HTTP `Range` header parsing for media and downloads (SPEC §5.2).
//!
//! Only single byte ranges are honoured. A multi-range request is answered with the whole
//! representation (200), which RFC 9110 allows; an unsatisfiable one with 416.

/// What to send for a request's `Range` header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeReply {
    /// No usable range: send the whole representation with 200.
    Full,
    /// Send bytes `start..=end` with 206.
    Partial { start: u64, end: u64 },
    /// Send 416 with `Content-Range: bytes */len`.
    Unsatisfiable,
}

/// The longest `Range` header value considered. Anything longer is ignored (whole file).
const MAX_HEADER: usize = 256;

/// Decides the reply for a `Range` header value (or its absence) and a representation of
/// `len` bytes.
pub fn parse(header: Option<&str>, len: u64) -> RangeReply {
    let Some(h) = header else { return RangeReply::Full };
    if h.len() > MAX_HEADER {
        return RangeReply::Full;
    }
    let Some(spec) = h.trim().strip_prefix("bytes=") else { return RangeReply::Full };
    if spec.contains(',') {
        return RangeReply::Full;
    }
    let Some((a, b)) = spec.trim().split_once('-') else { return RangeReply::Full };
    let (a, b) = (a.trim(), b.trim());
    let num = |s: &str| -> Option<u64> {
        if s.is_empty() || s.len() > 19 || !s.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        s.parse().ok()
    };
    if len == 0 {
        return RangeReply::Unsatisfiable;
    }
    match (a.is_empty(), b.is_empty()) {
        // bytes=-N: the last N bytes.
        (true, false) => match num(b) {
            Some(0) => RangeReply::Unsatisfiable,
            Some(n) => RangeReply::Partial { start: len.saturating_sub(n), end: len - 1 },
            None => RangeReply::Full,
        },
        // bytes=A-: from A to the end.
        (false, true) => match num(a) {
            Some(s) if s < len => RangeReply::Partial { start: s, end: len - 1 },
            Some(_) => RangeReply::Unsatisfiable,
            None => RangeReply::Full,
        },
        // bytes=A-B
        (false, false) => match (num(a), num(b)) {
            (Some(s), Some(e)) if s <= e => {
                if s >= len {
                    RangeReply::Unsatisfiable
                } else {
                    RangeReply::Partial { start: s, end: e.min(len - 1) }
                }
            }
            _ => RangeReply::Full,
        },
        (true, true) => RangeReply::Full,
    }
}

#[cfg(test)]
mod tests {
    use super::RangeReply::*;
    use super::*;

    #[test]
    fn the_common_forms_are_honoured() {
        assert_eq!(parse(Some("bytes=0-99"), 1000), Partial { start: 0, end: 99 });
        assert_eq!(parse(Some("bytes=500-"), 1000), Partial { start: 500, end: 999 });
        assert_eq!(parse(Some("bytes=-100"), 1000), Partial { start: 900, end: 999 });
        assert_eq!(parse(Some("bytes=0-"), 1000), Partial { start: 0, end: 999 });
    }

    #[test]
    fn an_end_past_the_length_is_clamped() {
        assert_eq!(parse(Some("bytes=900-5000"), 1000), Partial { start: 900, end: 999 });
        assert_eq!(parse(Some("bytes=-5000"), 1000), Partial { start: 0, end: 999 });
    }

    #[test]
    fn a_start_past_the_end_is_unsatisfiable() {
        assert_eq!(parse(Some("bytes=1000-"), 1000), Unsatisfiable);
        assert_eq!(parse(Some("bytes=1000-1001"), 1000), Unsatisfiable);
        assert_eq!(parse(Some("bytes=-0"), 1000), Unsatisfiable);
        assert_eq!(parse(Some("bytes=0-0"), 0), Unsatisfiable);
    }

    #[test]
    fn absent_multi_or_malformed_ranges_get_the_whole_file() {
        for h in [
            None,
            Some("bytes=0-1,5-6"),
            Some("items=0-1"),
            Some("bytes=abc-"),
            Some("bytes=5-1"),
            Some("bytes=-"),
            Some("bytes=+5-9"),
            Some("bytes= 1 2-3"),
            Some("bytes=99999999999999999999-"),
        ] {
            assert_eq!(parse(h, 1000), Full, "{h:?}");
        }
        let long = format!("bytes=0-{}", "9".repeat(300));
        assert_eq!(parse(Some(&long), 1000), Full);
    }

    #[test]
    fn parse_never_panics_and_stays_in_bounds() {
        let mut x: u64 = 0x1234_5678;
        let alphabet = b"bytes=-0123456789, ";
        for _ in 0..50_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let n = (x % 24) as usize;
            let s: String = (0..n).map(|i| alphabet[((x >> (i % 50)) as usize) % alphabet.len()] as char).collect();
            let len = x % 10_000;
            if let Partial { start, end } = parse(Some(&format!("bytes={s}")), len) {
                assert!(start <= end && end < len, "{s} {len}");
            }
        }
    }
}
