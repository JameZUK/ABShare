//! Cover images (review M5): arbitrary bytes into the decoder and thumbnail renderer that
//! the server's jailed `render-cover` command runs. It must never panic, and what it returns
//! is always one JPEG for each thumbnail size.
#![no_main]
use libfuzzer_sys::fuzz_target;

// The server is a binary crate, so its renderer is built here from the same source file.
#[allow(dead_code)]
#[path = "../../server/src/covers/render.rs"]
mod render;

fuzz_target!(|data: &[u8]| {
    if let Ok(thumbs) = render::render(data) {
        assert_eq!(thumbs.iter().map(|(size, _)| *size).collect::<Vec<_>>(), render::SIZES);
        for (_, jpeg) in &thumbs {
            assert!(jpeg.starts_with(&[0xFF, 0xD8]) && jpeg.ends_with(&[0xFF, 0xD9]));
        }
    }
});
