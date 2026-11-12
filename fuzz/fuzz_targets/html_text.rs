//! Untrusted text and HTML: the cleaners must never emit markup, controls or overlong output.
#![no_main]
use abshare_core::text::{
    clean_line, clean_paragraphs, html_to_text, is_invisible, limited_text, scrub_free_text, word_count,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|s: &str| {
    let t = html_to_text(s);
    assert!(!t.contains('<') && !t.contains('>'));
    let p = clean_paragraphs(&t, 500);
    assert!(p.chars().count() <= 500);
    assert!(!p.chars().any(|c| c.is_control() && c != '\n'));
    assert!(!p.chars().any(is_invisible));
    let l = clean_line(s, 100);
    assert!(l.chars().count() <= 100);
    assert!(!l.chars().any(char::is_control));
    // No character of the whole invisible set survives (review L19).
    assert!(!l.chars().any(is_invisible));
    // Scrubbing only ever removes whole words.
    let scrubbed = scrub_free_text(&l);
    assert!(scrubbed.len() <= l.len());
    let mut words = l.split(' ');
    assert!(scrubbed.split(' ').filter(|kept| !kept.is_empty()).all(|kept| words.any(|w| w == kept)));
    // People's own text (reviews, notes): within both limits, clean, and stable when cleaned again.
    if let Some(t) = limited_text(s, 50, 400) {
        assert!(t.chars().count() <= 400 && word_count(&t) <= 50);
        assert!(!t.chars().any(|c| c.is_control() || is_invisible(c)));
        assert_eq!(limited_text(&t, 50, 400).as_deref(), Some(t.as_str()));
        // Never cut: everything the input had survives (review S3).
        assert_eq!(t, clean_line(s, 64 * 1024));
    }
});
