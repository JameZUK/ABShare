//! Chapters for a book made of parts (`for_parts`, and through it `part_title`): arbitrary
//! file names, title tags, durations and embedded chapters. The result is bounded, in order
//! and without gaps, and every title is clean.
#![no_main]
use abshare_core::chapters::{MAX_CHAPTERS, for_parts};
use abshare_core::model::{Codec, Container, ProbedFile, RawChapter, Tags};
use abshare_core::text::is_invisible;
use libfuzzer_sys::fuzz_target;

#[derive(arbitrary::Arbitrary, Debug)]
struct Part {
    rel_path: String,
    title: Option<String>,
    duration_ms: u32,
    chapters: Vec<(u32, u32, String)>,
}

fuzz_target!(|parts: Vec<Part>| {
    let files: Vec<ProbedFile> = parts
        .into_iter()
        .take(64)
        .map(|p| ProbedFile {
            rel_path: p.rel_path,
            size: 1,
            container: Container::Mp3,
            codec: Codec::Mp3,
            duration_ms: u64::from(p.duration_ms),
            bitrate: 64_000,
            sample_rate: 44_100,
            channels: 2,
            tags: Tags::new(p.title.map(|t| ("title".to_owned(), t))),
            chapters: p
                .chapters
                .into_iter()
                .map(|(s, e, t)| RawChapter { start_ms: u64::from(s), end_ms: u64::from(e), title: t })
                .collect(),
            has_cover: false,
            decodes: true,
        })
        .collect();
    let refs: Vec<&ProbedFile> = files.iter().collect();
    let total: u64 = files.iter().map(|f| f.duration_ms).sum();
    let c = for_parts(&refs);
    assert!(c.len() <= MAX_CHAPTERS);
    if let (Some(first), Some(last)) = (c.first(), c.last()) {
        assert_eq!(first.start_ms, 0);
        assert!(last.end_ms <= total);
        if c.len() < MAX_CHAPTERS {
            assert_eq!(last.end_ms, total);
        }
    }
    for w in c.windows(2) {
        assert_eq!(w[0].end_ms, w[1].start_ms);
    }
    for ch in &c {
        assert!(ch.start_ms <= ch.end_ms);
        assert!(!ch.title.is_empty() && ch.title.chars().count() <= 200);
        assert!(!ch.title.chars().any(|ch| ch.is_control() || is_invisible(ch)));
    }
});
