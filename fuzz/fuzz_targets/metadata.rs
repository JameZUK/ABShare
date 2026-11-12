//! Arbitrary tags, folder and file names and sidecar texts into book metadata.
#![no_main]
use abshare_core::metadata::book_meta;
use abshare_core::model::{Sidecars, Tags};
use abshare_core::text::is_invisible;
use libfuzzer_sys::fuzz_target;

#[derive(arbitrary::Arbitrary, Debug)]
struct Input {
    tags: Vec<(u8, String)>,
    folder: Option<String>,
    file: String,
    desc: Option<String>,
    reader: Option<String>,
    info: Vec<String>,
    multi: bool,
}

/// Every tag `book_meta` reads, so each of its branches can be reached.
const KEYS: &[&str] = &[
    "title",
    "album",
    "artist",
    "album_artist",
    "composer",
    "genre",
    "date",
    "comment",
    "series",
    "part",
    "audible_asin",
    "subtitle",
    "lyrics-eng",
    "narrator",
    "publisher",
    "language",
    "track",
    "disc",
    "isbn",
    "asin",
    "author",
    "writer",
    "performer",
    "description",
    "synopsis",
    "lyrics",
    "year",
    "tyer",
    "tdrc",
    "mvnm",
    "grouping",
    "series-part",
    "mvin",
];

fn clean(s: &str) -> bool {
    !s.chars().any(|c| c.is_control() || is_invisible(c))
}

fuzz_target!(|i: Input| {
    let tags = Tags::new(i.tags.into_iter().map(|(k, v)| (KEYS[k as usize % KEYS.len()].to_owned(), v)));
    let side = Sidecars { desc: i.desc, reader: i.reader, info: i.info.into_iter().take(4).collect() };
    let m = book_meta(&tags, i.folder.as_deref(), &i.file, &side, i.multi);
    assert!(!m.title.is_empty());
    assert!(!m.title.chars().any(char::is_control));
    assert!(m.authors.len() <= 50 && m.narrators.len() <= 50);
    assert!(clean(&m.title));
    for line in [&m.subtitle, &m.publisher, &m.language, &m.album].into_iter().flatten() {
        assert!(!line.is_empty() && clean(line));
    }
    if let Some(s) = &m.series {
        assert!(!s.name.is_empty() && clean(&s.name));
    }
    // Identifiers are passed on only in their strict forms.
    if let Some(isbn) = &m.isbn {
        assert!(matches!(isbn.len(), 10 | 13) && isbn.bytes().all(|c| c.is_ascii_digit() || c == b'X'));
    }
    if let Some(asin) = &m.asin {
        assert!(asin.len() == 10 && asin.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()));
    }
    if let Some(y) = m.year {
        assert!((1000..=2100).contains(&y));
    }
    if let Some(d) = &m.description {
        assert!(!d.contains('<') && !d.contains('>'));
    }
});
