//! Cleaning for untrusted text from tags, sidecars and file names (SPEC §4.4).
//!
//! Everything a file says about itself is untrusted. Text is NFC-normalised, stripped of
//! control, bidi-override and zero-width characters, has whitespace collapsed, and is
//! capped by length. HTML is turned into plain text by a small bounded tokenizer of ours
//! (never a browser parser). Anything that looks like a filesystem path or a LAN address is
//! removed from free text. Clients still render the result as text only.

use unicode_normalization::UnicodeNormalization;

/// Field length caps, in characters.
pub const MAX_NAME: usize = 512;
pub const MAX_DESCRIPTION: usize = 20_000;
/// Input longer than this is cut before any work is done on it.
const MAX_INPUT: usize = 256 * 1024;

fn is_invisible(c: char) -> bool {
    matches!(c,
        '\u{200B}'..='\u{200F}'   // zero-width space/joiners, LRM, RLM
        | '\u{202A}'..='\u{202E}' // bidi embeddings and overrides
        | '\u{2060}'..='\u{2064}' // word joiner, invisible operators
        | '\u{2066}'..='\u{2069}' // bidi isolates
        | '\u{FEFF}'              // BOM / zero-width no-break space
        | '\u{00AD}'              // soft hyphen
        | '\u{034F}'              // combining grapheme joiner
        | '\u{180E}'              // Mongolian vowel separator
        | '\u{FFF9}'..='\u{FFFB}' // interlinear annotation
    )
}

fn cap_input(s: &str) -> &str {
    if s.len() <= MAX_INPUT {
        return s;
    }
    let mut end = MAX_INPUT;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn truncate_chars(s: &mut String, max: usize) {
    if let Some((i, _)) = s.char_indices().nth(max) {
        s.truncate(i);
        let t = s.trim_end().len();
        s.truncate(t);
    }
}

/// Cleans a one-line value (a title, a name, a series): NFC, invisible and control
/// characters removed, all whitespace collapsed to single spaces, trimmed, capped.
pub fn clean_line(s: &str, max: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max * 4));
    let mut space = false;
    for c in cap_input(s).nfc() {
        if is_invisible(c) {
            continue;
        }
        if c.is_whitespace() || c.is_control() {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(c);
    }
    truncate_chars(&mut out, max);
    out
}

/// Cleans multi-paragraph text (a description): like [`clean_line`] within each
/// paragraph, keeps paragraph breaks (at most one blank line in a row), removes things that
/// look like paths or LAN addresses, and caps the total.
pub fn clean_paragraphs(s: &str, max: usize) -> String {
    let mut paras: Vec<String> = Vec::new();
    for raw in cap_input(s).split('\n') {
        let line = scrub_free_text(&clean_line(raw, max));
        if line.is_empty() {
            if paras.last().is_some_and(|p| !p.is_empty()) {
                paras.push(String::new());
            }
        } else {
            paras.push(line);
        }
    }
    while paras.last().is_some_and(String::is_empty) {
        paras.pop();
    }
    let mut out = paras.join("\n");
    truncate_chars(&mut out, max);
    out
}

/// Folds text for searching: lower case, accents removed (é → e), whitespace collapsed.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in cap_input(s).nfkd() {
        if unicode_normalization::char::is_combining_mark(c) || is_invisible(c) {
            continue;
        }
        if c.is_whitespace() || c.is_control() {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// Removes words that look like filesystem paths, `file:` URLs or private network
/// addresses from free text (SPEC §4.4). Ordinary prose and public URLs are left alone.
pub fn scrub_free_text(s: &str) -> String {
    let kept: Vec<&str> = s.split(' ').filter(|w| !looks_internal(w)).collect();
    kept.join(" ")
}

fn looks_internal(word: &str) -> bool {
    let w = word.trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']' | ',' | '.' | ';' | ':' | '"' | '\''));
    if w.is_empty() {
        return false;
    }
    let lower = w.to_ascii_lowercase();
    if lower.starts_with("file:") || lower.starts_with("smb:") || lower.starts_with("nfs:") {
        return true;
    }
    // Unix absolute path with at least two components: /srv/media, /home/x/...
    if w.starts_with('/') && w[1..].contains('/') && !w.starts_with("//") {
        return true;
    }
    // Windows drive or UNC paths.
    let b = w.as_bytes();
    if (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
        || w.starts_with("\\\\")
    {
        return true;
    }
    // Private, loopback or link-local IPv4, alone or inside a URL.
    let host = lower.split("://").last().unwrap_or(&lower);
    let host = host.split(['/', ':']).next().unwrap_or(host);
    if let Some(ip) = parse_ipv4(host) {
        return is_internal_v4(ip);
    }
    host.ends_with(".lan") || host.ends_with(".local") || host.ends_with(".home.arpa") || host.ends_with(".internal")
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = s.split('.');
    for o in &mut out {
        let p = parts.next()?;
        if p.is_empty() || p.len() > 3 || !p.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        *o = p.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

fn is_internal_v4([a, b, ..]: [u8; 4]) -> bool {
    a == 10 || a == 127 || (a == 172 && (16..=31).contains(&b)) || (a == 192 && b == 168) || (a == 169 && b == 254)
}

/// Converts HTML (as found in podcast-style description tags) to plain text: tags removed,
/// block elements become paragraph breaks, entities decoded. Never emits markup.
pub fn html_to_text(html: &str) -> String {
    let src = cap_input(html);
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(lt) = rest.find('<') {
        push_decoded(&mut out, &rest[..lt]);
        let after = &rest[lt + 1..];
        let Some(gt) = after.find('>') else {
            // A stray '<' with no closing '>': keep it as text.
            push_decoded(&mut out, "<");
            rest = after;
            continue;
        };
        let tag = after[..gt].trim_start_matches('/').trim();
        let name: String =
            tag.chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        let skip_body = matches!(name.as_str(), "script" | "style") && !after[..gt].starts_with('/');
        rest = &after[gt + 1..];
        if skip_body {
            let close = format!("</{name}");
            match rest.to_ascii_lowercase().find(&close) {
                Some(i) => {
                    let tail = &rest[i..];
                    rest = tail.find('>').map(|j| &tail[j + 1..]).unwrap_or("");
                }
                None => rest = "",
            }
            continue;
        }
        if matches!(
            name.as_str(),
            "p" | "br" | "div" | "li" | "ul" | "ol" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "tr" | "blockquote"
        ) {
            out.push('\n');
        }
    }
    push_decoded(&mut out, rest);
    // Safety net: whatever happened above, no markup characters survive as markup.
    out.replace(['<', '>'], "")
}

fn push_decoded(out: &mut String, text: &str) {
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let end = after.find(';').filter(|&i| i <= 10);
        let decoded = end.and_then(|i| decode_entity(&after[..i]));
        match (end, decoded) {
            (Some(i), Some(c)) => {
                out.push(c);
                rest = &after[i + 1..];
            }
            _ => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
}

fn decode_entity(name: &str) -> Option<char> {
    let c = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" | "#39" => '\'',
        "nbsp" => ' ',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "rsquo" => '’',
        "lsquo" => '‘',
        "rdquo" => '”',
        "ldquo" => '“',
        "copy" => '©',
        _ => {
            let n = name.strip_prefix('#')?;
            let code = match n.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => n.parse().ok()?,
            };
            let c = char::from_u32(code)?;
            if c.is_control() && c != '\n' && c != '\t' {
                return None;
            }
            c
        }
    };
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_normalised_and_collapsed() {
        assert_eq!(clean_line("  The\tLong\n  Way  ", 100), "The Long Way");
        // NFC: e + combining acute becomes é.
        assert_eq!(clean_line("Cafe\u{301}", 100), "Café");
    }

    #[test]
    fn folding_ignores_case_accents_and_spacing() {
        assert_eq!(fold("  Café  CRÈME\tBrûlée "), "cafe creme brulee");
        assert_eq!(fold("Ångström"), fold("angstrom"));
        assert_eq!(fold("ﬁne"), "fine");
    }

    #[test]
    fn invisible_and_bidi_characters_are_removed() {
        let s = "Sa\u{200B}fe\u{202E}txt.exe\u{2066}\u{FEFF}";
        assert_eq!(clean_line(s, 100), "Safetxt.exe");
        assert_eq!(clean_line("a\u{0007}b\u{001B}[31mc", 100), "a b [31mc");
    }

    #[test]
    fn length_is_capped_on_a_character_boundary() {
        let s = "é".repeat(600);
        let c = clean_line(&s, MAX_NAME);
        assert_eq!(c.chars().count(), MAX_NAME);
    }

    #[test]
    fn paragraphs_keep_single_blank_lines() {
        assert_eq!(clean_paragraphs("One.\n\n\n\nTwo.\n\n", 100), "One.\n\nTwo.");
    }

    #[test]
    fn paths_and_lan_addresses_are_scrubbed_from_free_text() {
        let s = "Ripped from /srv/media/books/x.m4b by 192.168.250.20 and \\\\nas\\share via http://10.0.0.5:8080/feed or C:\\Users\\me, see https://example.com/book";
        assert_eq!(scrub_free_text(s), "Ripped from by and via or see https://example.com/book");
        assert_eq!(scrub_free_text("Visit nas.lan or printer.local today"), "Visit or today");
        assert_eq!(scrub_free_text("A 1/2 price and/or offer at 8.8.8.8"), "A 1/2 price and/or offer at 8.8.8.8");
    }

    #[test]
    fn html_becomes_text_with_paragraphs() {
        let h = "<p><strong>Season 2</strong> is here &amp; it&#39;s <em>big</em>.</p><p>Listen&nbsp;now</p>";
        assert_eq!(clean_paragraphs(&html_to_text(h), 1000), "Season 2 is here & it's big.\n\nListen now");
    }

    #[test]
    fn html_never_emits_markup() {
        for h in [
            "<script>alert(1)</script>Hi",
            "&lt;img src=x onerror=alert(1)&gt;",
            "<<b>>bold<</b>>",
            "<a href=\"javascript:alert(1)\">x</a>",
            "<style>p{}</style>",
            "&#60;svg onload=x&#62;",
            "<",
            "a < b > c",
        ] {
            let t = html_to_text(h);
            assert!(!t.contains('<') && !t.contains('>'), "{h:?} -> {t:?}");
        }
        assert_eq!(html_to_text("<script>alert(1)</script>Hi"), "Hi");
    }

    #[test]
    fn entities_that_would_be_controls_are_not_decoded() {
        assert_eq!(html_to_text("a&#0;b&#27;c"), "a&#0;b&#27;c");
    }

    #[test]
    fn cleaning_never_panics_on_arbitrary_input() {
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        let pieces =
            ["<", ">", "&", ";", "#x", "/", "\\", " ", "\n", "é", "\u{202E}", "a", "<p>", "&amp", "192.168.", "1"];
        for _ in 0..20_000 {
            let mut s = String::new();
            for _ in 0..(x % 40) {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                s.push_str(pieces[(x % pieces.len() as u64) as usize]);
            }
            let h = html_to_text(&s);
            assert!(!h.contains('<') && !h.contains('>'));
            let p = clean_paragraphs(&h, 50);
            assert!(p.chars().count() <= 50);
            let l = clean_line(&s, 20);
            assert!(l.chars().count() <= 20 && !l.contains('\n'));
        }
    }
}
