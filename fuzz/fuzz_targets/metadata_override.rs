//! Admin corrections to book details: parsing arbitrary JSON never panics, a parsed
//! correction round-trips, and applying it never panics.
#![no_main]
use abshare_core::metadata::BookMeta;
use abshare_core::metadata_override::Override;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(data) else { return };
    if let Ok(o) = Override::parse(&v) {
        assert_eq!(Override::parse(&o.to_json()).ok(), Some(o.clone()));
        let mut m = BookMeta::default();
        o.apply(&mut m);
    }
});
