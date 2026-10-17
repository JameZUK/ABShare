//! Episode numbering from titles: never panics.
#![no_main]
use abshare_core::series::parse_episode;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|s: &str| {
    let _ = parse_episode(s);
});
