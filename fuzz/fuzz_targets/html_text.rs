//! Untrusted text and HTML: the cleaners must never emit markup, controls or overlong output.
#![no_main]
use abshare_core::text::{clean_line, clean_paragraphs, html_to_text, scrub_free_text};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|s: &str| {
    let t = html_to_text(s);
    assert!(!t.contains('<') && !t.contains('>'));
    let p = clean_paragraphs(&t, 500);
    assert!(p.chars().count() <= 500);
    assert!(!p.chars().any(|c| c.is_control() && c != '\n'));
    let l = clean_line(s, 100);
    assert!(l.chars().count() <= 100);
    assert!(!l.chars().any(char::is_control));
    assert!(!l.contains("\u{202E}") && !l.contains("\u{200B}"));
    let _ = scrub_free_text(&l);
});
