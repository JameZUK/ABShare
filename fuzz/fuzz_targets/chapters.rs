//! Chapter sanitising: sorted, contiguous, inside the duration, bounded.
#![no_main]
use abshare_core::chapters::{MAX_CHAPTERS, sanitise};
use abshare_core::model::RawChapter;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|i: (Vec<(u32, u32, String)>, u32)| {
    let (raw, dur) = i;
    let dur = u64::from(dur);
    let raw: Vec<RawChapter> =
        raw.into_iter().map(|(s, e, t)| RawChapter { start_ms: u64::from(s), end_ms: u64::from(e), title: t }).collect();
    let c = sanitise(&raw, dur);
    assert!(c.len() <= MAX_CHAPTERS);
    if let (Some(first), Some(last)) = (c.first(), c.last()) {
        assert_eq!(first.start_ms, 0);
        assert_eq!(last.end_ms, dur);
    }
    for w in c.windows(2) {
        assert_eq!(w[0].end_ms, w[1].start_ms);
    }
    assert!(c.iter().all(|ch| ch.start_ms < ch.end_ms && !ch.title.is_empty()));
});
