//! Genre shelves and text cleaning on arbitrary tags: never panic, and a shelf's ID always
//! maps back to it.
#![no_main]
use abshare_core::genre::{Shelf, shelves};
use abshare_core::text::clean_line;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    let tags: Vec<String> = data.split([',', ';']).map(|t| clean_line(t, 80)).collect();
    for s in shelves(&tags) {
        assert_eq!(Shelf::from_id(s.id()), Some(s));
    }
});
