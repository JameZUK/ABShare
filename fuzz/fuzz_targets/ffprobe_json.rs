//! ffprobe JSON is produced by a tool reading hostile files: parse it with bounds, never panic.
#![no_main]
use abshare_core::ffprobe::{parse_ffprobe, to_probed};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(rec) = parse_ffprobe(data) {
        assert!(rec.duration_ms <= 3_600_000_000);
        assert!(rec.tags.len() <= abshare_core::model::Tags::MAX);
        let p = to_probed("x/y.m4b", 1, &rec);
        assert!(p.chapters.iter().all(|c| c.start_ms <= 3_600_000_000));
    }
});
