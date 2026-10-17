//! Arbitrary tags, folder and file names into book metadata.
#![no_main]
use abshare_core::metadata::book_meta;
use abshare_core::model::{Sidecars, Tags};
use libfuzzer_sys::fuzz_target;

#[derive(arbitrary::Arbitrary, Debug)]
struct Input {
    tags: Vec<(u8, String)>,
    folder: Option<String>,
    file: String,
    desc: Option<String>,
    info: Vec<String>,
    multi: bool,
}

const KEYS: &[&str] = &[
    "title", "album", "artist", "album_artist", "composer", "genre", "date", "comment", "series", "part",
    "audible_asin", "subtitle", "lyrics-eng", "narrator", "publisher", "language", "track", "disc",
];

fuzz_target!(|i: Input| {
    let tags = Tags::new(i.tags.into_iter().map(|(k, v)| (KEYS[k as usize % KEYS.len()].to_owned(), v)));
    let side = Sidecars { desc: i.desc, reader: None, info: i.info.into_iter().take(4).collect() };
    let m = book_meta(&tags, i.folder.as_deref(), &i.file, &side, i.multi);
    assert!(!m.title.is_empty());
    assert!(!m.title.chars().any(char::is_control));
    assert!(m.authors.len() <= 50 && m.narrators.len() <= 50);
    if let Some(d) = &m.description {
        assert!(!d.contains('<') && !d.contains('>'));
    }
});
