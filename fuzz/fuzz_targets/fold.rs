//! Search folding on arbitrary text: never panics, and the result has no control or
//! invisible characters and no whitespace but single spaces between words.
#![no_main]
use abshare_core::text::{fold, is_invisible};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|s: &str| {
    let f = fold(s);
    assert!(!f.chars().any(|c| c.is_control() || is_invisible(c)));
    assert!(!f.chars().any(|c| c.is_whitespace() && c != ' '));
    assert!(!f.starts_with(' ') && !f.ends_with(' ') && !f.contains("  "));
    // The same text always folds the same way, and case never matters for ASCII.
    assert_eq!(f, fold(s));
    if s.is_ascii() {
        assert_eq!(f, fold(&s.to_ascii_uppercase()));
        assert!(f.len() <= s.len());
    }
});
