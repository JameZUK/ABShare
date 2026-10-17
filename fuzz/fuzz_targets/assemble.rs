//! Book assembly over arbitrary folder layouts: every file ends up in exactly one book or in
//! the ignored list, never both and never lost.
#![no_main]
use abshare_core::assemble::assemble;
use abshare_core::model::{Codec, Container, ProbedFile, Tags};
use libfuzzer_sys::fuzz_target;

#[derive(arbitrary::Arbitrary, Debug)]
struct F {
    dir: u8,
    name: u8,
    ms: u32,
    asin: u8,
    album: u8,
    track: u8,
    decodes: bool,
    xhe: bool,
}

const DIRS: &[&str] = &["", "A", "A/CD1", "A/CD2", "A/Disc 3", "B", "B/x", "C/Part 1", "D/d/e"];

fuzz_target!(|fs: Vec<F>| {
    let files: Vec<ProbedFile> = fs
        .iter()
        .take(64)
        .enumerate()
        .map(|(i, f)| {
            let dir = DIRS[f.dir as usize % DIRS.len()];
            let name = if f.name % 3 == 0 { format!("{i}.m4b-1754152438.952120-AbCd.m4b") } else { format!("{i}.mp3") };
            let rel = if dir.is_empty() { name } else { format!("{dir}/{name}") };
            let tags = Tags::new([
                ("audible_asin".to_owned(), ["", "B0AAAAAAA1", "B0BBBBBBB2"][f.asin as usize % 3].to_owned()),
                ("album".to_owned(), ["", "X", "Y"][f.album as usize % 3].to_owned()),
                ("track".to_owned(), f.track.to_string()),
            ]);
            ProbedFile {
                rel_path: rel,
                size: 1,
                container: Container::Mp3,
                codec: if f.xhe { Codec::XheAac } else { Codec::Mp3 },
                duration_ms: u64::from(f.ms % 100_000),
                bitrate: 64_000,
                sample_rate: 44_100,
                channels: 2,
                tags,
                chapters: vec![],
                has_cover: false,
                decodes: f.decodes,
            }
        })
        .collect();
    let a = assemble(&files);
    let mut seen: Vec<usize> = a.books.iter().flat_map(|b| b.parts.clone()).chain(a.ignored.iter().map(|i| i.file)).collect();
    seen.sort_unstable();
    let n = seen.len();
    seen.dedup();
    assert_eq!(seen.len(), n);
    assert_eq!(n, files.len());
});
