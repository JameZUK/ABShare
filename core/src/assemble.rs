//! Which files make a book (SPEC §4.3). Pure: works on probed files only.
//!
//! - A loose audio file directly under a library root is a book.
//! - In a directory, files with different ASINs or different non-empty album tags are
//!   separate books (a flat folder of many books).
//! - Files that are copies of the same book (same ASIN or title, durations within 1 %) are
//!   alternates: the best one is used, the rest are reported as ignored duplicates.
//! - Otherwise the files are parts of one book, ordered by disc, track, then natural name.
//! - Disc folders (`CD1`, `Disc 2`, `Part 3`) under a folder with no audio of its own are
//!   merged into the parent's book.
//! - Unplayable files (failed trial decode, xHE-AAC, zero length) are reported, never used.

use crate::model::{Codec, ProbedFile};
use crate::text::clean_line;
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// One book as the scanner found it: indices into the input slice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookPlan {
    /// The folder the book lives in (relative path), or the file itself for a loose file.
    pub key: String,
    /// The files making up the book, in play order.
    pub parts: Vec<usize>,
    /// Whether the book needs a playable copy (several parts, or a codec browsers can't play).
    pub needs_copy: bool,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IgnoreReason {
    /// xHE-AAC: FFmpeg can probe but not decode it (SPEC D10).
    XheAac,
    /// The trial decode reported errors.
    DecodeFailed,
    /// No audio duration.
    Empty,
    /// A copy of another file in the same book; `kept` is the one used.
    Duplicate { kept: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ignored {
    pub file: usize,
    pub reason: IgnoreReason,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Assembly {
    pub books: Vec<BookPlan>,
    pub ignored: Vec<Ignored>,
}

/// Most parts one book may have; more is treated as a mistake and split by album instead.
const MAX_PARTS: usize = 2_000;

pub fn assemble(files: &[ProbedFile]) -> Assembly {
    let mut out = Assembly::default();
    let mut by_dir: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, f) in files.iter().enumerate() {
        if !f.playable() {
            let reason = if f.codec == Codec::XheAac {
                IgnoreReason::XheAac
            } else if f.duration_ms == 0 {
                IgnoreReason::Empty
            } else {
                IgnoreReason::DecodeFailed
            };
            out.ignored.push(Ignored { file: i, reason });
            continue;
        }
        by_dir.entry(book_dir(&f.rel_path, files)).or_default().push(i);
    }
    for (dir, idx) in by_dir {
        if dir.is_empty() {
            // Loose files at the root: one book each.
            for i in idx {
                out.books.push(single(files, i, files[i].rel_path.clone()));
            }
            continue;
        }
        assemble_dir(files, &dir, idx, &mut out);
    }
    dedupe_across_folders(files, &mut out);
    out.books.sort_by(|a, b| natural_cmp(&a.key, &b.key));
    out
}

/// Single-file books in different folders with the same ASIN and durations within 1 % are
/// the same book (for example two folders whose names differ only in case): the best copy
/// is kept, the others are reported as duplicates.
fn dedupe_across_folders(files: &[ProbedFile], out: &mut Assembly) {
    let mut by_asin: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (bi, b) in out.books.iter().enumerate() {
        if let [only] = b.parts[..]
            && let Some(a) = files[only].tags.get(&["audible_asin", "asin"])
        {
            by_asin.entry(a.trim()).or_default().push(bi);
        }
    }
    let mut drop: Vec<usize> = Vec::new();
    for books in by_asin.values().filter(|v| v.len() > 1) {
        let mut idx: Vec<usize> = books.iter().map(|&bi| out.books[bi].parts[0]).collect();
        if !are_alternates(files, &idx) {
            continue;
        }
        idx.sort_by(|&a, &b| better(&files[a], &files[b]));
        let kept = idx[0];
        for &bi in books {
            let f = out.books[bi].parts[0];
            if f != kept {
                out.ignored.push(Ignored { file: f, reason: IgnoreReason::Duplicate { kept } });
                drop.push(bi);
            }
        }
    }
    drop.sort_unstable();
    for bi in drop.into_iter().rev() {
        out.books.remove(bi);
    }
}

/// The folder a file's book is keyed on: its own folder, or the parent when the file sits
/// in a disc folder whose parent has no audio of its own.
fn book_dir(rel: &str, files: &[ProbedFile]) -> String {
    let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
    let Some((parent, leaf)) = dir.rsplit_once('/').or(if dir.is_empty() { None } else { Some(("", dir)) }) else {
        return String::new();
    };
    if disc_number(leaf).is_some() && !parent.is_empty() {
        let parent_has_audio = files.iter().any(|f| f.dir() == parent);
        if !parent_has_audio {
            return parent.to_owned();
        }
    }
    dir.to_owned()
}

/// "CD1", "CD 2", "Disc 03", "Disk4", "Part 5" → the number.
pub fn disc_number(name: &str) -> Option<u32> {
    let lower = name.trim().to_ascii_lowercase();
    for prefix in ["cd", "disc", "disk", "part"] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let rest = rest.trim_start_matches([' ', '_', '-', '.']);
            if !rest.is_empty() && rest.len() <= 3 && rest.bytes().all(|c| c.is_ascii_digit()) {
                return rest.parse().ok();
            }
        }
    }
    None
}

fn single(files: &[ProbedFile], i: usize, key: String) -> BookPlan {
    BookPlan { key, parts: vec![i], needs_copy: !files[i].codec.browser_safe(), duration_ms: files[i].duration_ms }
}

fn norm(s: &str) -> String {
    clean_line(s, 512).to_lowercase()
}

/// Separates a folder from a book's identity in the key of a book that shares its folder
/// with others. A control character: the walker refuses those in names, so no real path can
/// contain it (review C4).
pub const KEY_SEP: char = '\u{1f}';

/// What tells two files in one folder apart as different books.
fn book_identity(f: &ProbedFile) -> String {
    if let Some(a) = f.tags.get(&["audible_asin", "asin"]) {
        return format!("asin:{}", a.trim());
    }
    match f.tags.get(&["album"]) {
        Some(a) => format!("album:{}", norm(a)),
        None => String::new(),
    }
}

fn assemble_dir(files: &[ProbedFile], dir: &str, idx: Vec<usize>, out: &mut Assembly) {
    // Split by identity; files with no identity join the single identified group if there
    // is exactly one, otherwise stay together on their own.
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for i in idx {
        groups.entry(book_identity(&files[i])).or_default().push(i);
    }
    if groups.len() == 2 && groups.contains_key("") {
        let anon = groups.remove("").unwrap_or_default();
        if let Some(v) = groups.values_mut().next() {
            v.extend(anon);
        }
    }
    let many = groups.len() > 1;
    for (identity, members) in groups {
        // Keyed by what the book is (its ASIN or album), never by its position among the
        // folder's books: adding an album must not move another book's key (review C4).
        let key = if many { format!("{dir}{KEY_SEP}{identity}") } else { dir.to_owned() };
        if let Some(book) = group_to_book(files, key, members, out) {
            out.books.push(book);
        }
    }
}

fn group_to_book(files: &[ProbedFile], key: String, mut members: Vec<usize>, out: &mut Assembly) -> Option<BookPlan> {
    if members.len() == 1 {
        return Some(single(files, members[0], key));
    }
    if are_alternates(files, &members) {
        members.sort_by(|&a, &b| better(&files[a], &files[b]));
        let kept = members[0];
        for &other in &members[1..] {
            out.ignored.push(Ignored { file: other, reason: IgnoreReason::Duplicate { kept } });
        }
        return Some(single(files, kept, key));
    }
    members.sort_by(|&a, &b| part_order(&files[a], &files[b]));
    members.truncate(MAX_PARTS);
    let duration_ms = members.iter().map(|&i| files[i].duration_ms).sum();
    Some(BookPlan { key, parts: members, needs_copy: true, duration_ms })
}

/// All files have the same ASIN or the same normalised title, and durations within 1 %.
fn are_alternates(files: &[ProbedFile], members: &[usize]) -> bool {
    let first = &files[members[0]];
    let title_of = |f: &ProbedFile| {
        f.tags.get(&["title"]).map(norm).unwrap_or_else(|| norm(&strip_conflict_suffix(f.file_name())))
    };
    let t0 = title_of(first);
    let a0 = first.tags.get(&["audible_asin", "asin"]);
    members[1..].iter().all(|&i| {
        let f = &files[i];
        let same = (a0.is_some() && a0 == f.tags.get(&["audible_asin", "asin"])) || title_of(f) == t0;
        let (d0, d1) = (first.duration_ms.max(1), f.duration_ms.max(1));
        same && d0.abs_diff(d1) * 100 <= d0.max(d1)
    })
}

/// Sync-conflict copies look like "Name.m4b-1754152438.952120-AbCdEfGh.m4b".
pub fn is_conflict_copy(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    ["m4b", "m4a", "mp3", "mp4", "ogg", "opus", "flac"].iter().any(|ext| {
        lower
            .find(&format!(".{ext}-"))
            .is_some_and(|i| lower[i + ext.len() + 2..].starts_with(|c: char| c.is_ascii_digit()))
    })
}

fn strip_conflict_suffix(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    for ext in ["m4b", "m4a", "mp3", "mp4", "ogg", "opus", "flac"] {
        if let Some(i) = lower.find(&format!(".{ext}-")) {
            return name[..i].to_owned();
        }
    }
    name.rsplit_once('.').map_or(name, |(s, _)| s).to_owned()
}

/// Ordering for alternates: best first.
fn better(a: &ProbedFile, b: &ProbedFile) -> Ordering {
    b.codec
        .browser_safe()
        .cmp(&a.codec.browser_safe())
        .then(b.bitrate.cmp(&a.bitrate))
        .then(is_conflict_copy(a.file_name()).cmp(&is_conflict_copy(b.file_name())))
        .then_with(|| natural_cmp(a.file_name(), b.file_name()))
}

/// "5/12" → 5.
fn leading_number(s: &str) -> Option<u32> {
    let digits: String = s.trim().chars().take_while(|c| c.is_ascii_digit()).take(6).collect();
    digits.parse().ok()
}

fn part_order(a: &ProbedFile, b: &ProbedFile) -> Ordering {
    let disc = |f: &ProbedFile| {
        f.tags
            .get(&["disc", "discnumber", "disk"])
            .and_then(leading_number)
            .or_else(|| f.dir().rsplit('/').next().and_then(disc_number))
            .unwrap_or(0)
    };
    let track = |f: &ProbedFile| f.tags.get(&["track", "tracknumber"]).and_then(leading_number);
    disc(a)
        .cmp(&disc(b))
        .then_with(|| match (track(a), track(b)) {
            (Some(x), Some(y)) => x.cmp(&y),
            _ => Ordering::Equal,
        })
        .then_with(|| natural_cmp(a.file_name(), b.file_name()))
}

/// Natural order: "Part 2" before "Part 10", case-insensitive.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut n1 = String::new();
                while let Some(c) = x.peek().copied().filter(char::is_ascii_digit) {
                    n1.push(c);
                    x.next();
                }
                let mut n2 = String::new();
                while let Some(c) = y.peek().copied().filter(char::is_ascii_digit) {
                    n2.push(c);
                    y.next();
                }
                let (t1, t2) = (n1.trim_start_matches('0'), n2.trim_start_matches('0'));
                let o = t1.len().cmp(&t2.len()).then_with(|| t1.cmp(t2));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                let o = c.to_lowercase().cmp(d.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{Container, Tags};

    pub(crate) fn file(path: &str, ms: u64, tags: &[(&str, &str)]) -> ProbedFile {
        ProbedFile {
            rel_path: path.into(),
            size: ms * 8,
            container: Container::Mp4,
            codec: Codec::AacLc,
            duration_ms: ms,
            bitrate: 64_000,
            sample_rate: 22_050,
            channels: 2,
            tags: Tags::new(tags.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))),
            chapters: vec![],
            has_cover: true,
            decodes: true,
        }
    }

    fn paths<'a>(files: &'a [ProbedFile], b: &BookPlan) -> Vec<&'a str> {
        b.parts.iter().map(|&i| files[i].file_name()).collect()
    }

    const H: u64 = 3_600_000;

    /// Review C4: a book's key in a shared folder depends on what it is, not on its position
    /// among the folder's books, so adding an album can't move progress to another book; and
    /// no real folder name can collide with a key.
    #[test]
    fn keys_in_a_shared_folder_are_stable_and_cannot_collide() {
        let two = vec![file("Mix/b.m4b", H, &[("album", "Bravo")]), file("Mix/c.m4b", H, &[("album", "Charlie")])];
        let keys = |fs: &[ProbedFile]| {
            let a = assemble(fs);
            let mut k: Vec<(String, String)> =
                a.books.iter().map(|b| (fs[b.parts[0]].file_name().to_owned(), b.key.clone())).collect();
            k.sort();
            k
        };
        let before = keys(&two);
        let mut three = two.clone();
        three.insert(0, file("Mix/a.m4b", H, &[("album", "Alpha")]));
        let after = keys(&three);
        for (name, key) in &before {
            assert!(after.contains(&(name.clone(), key.clone())), "{name} kept its key {key:?}");
        }
        // The separator never appears in a walked path (control characters are refused there).
        for (_, k) in &after {
            assert!(k.starts_with("Mix") && k.contains(KEY_SEP), "{k:?}");
        }
        assert!(!after.iter().any(|(_, k)| k == "Mix#0" || k == "Mix#1"));
    }

    #[test]
    fn one_file_per_folder_is_one_book() {
        let files = vec![
            file("Book A [B0AAAAAAA1]/Book A.m4b", 10 * H, &[]),
            file("Book B [B0BBBBBBB2]/Book B.m4b", 9 * H, &[]),
        ];
        let a = assemble(&files);
        assert_eq!(a.books.len(), 2);
        assert!(a.books.iter().all(|b| !b.needs_copy && b.parts.len() == 1));
    }

    #[test]
    fn loose_root_files_are_books_on_their_own() {
        let files = vec![file("loose one.mp3", H, &[]), file("loose two.mp3", H, &[])];
        assert_eq!(assemble(&files).books.len(), 2);
    }

    #[test]
    fn conflict_copies_are_alternates_and_the_best_is_kept() {
        let mut xhe = file("Tale [B0CCCCCCC3]/Tale [B0CCCCCCC3].m4b", 9 * H, &[("audible_asin", "B0CCCCCCC3")]);
        xhe.codec = Codec::XheAac;
        let lc = file(
            "Tale [B0CCCCCCC3]/Tale [B0CCCCCCC3].m4b-1754152663.548194-AbCdEfGh.m4b",
            9 * H + 60_000,
            &[("audible_asin", "B0CCCCCCC3")],
        );
        let files = vec![xhe, lc];
        let a = assemble(&files);
        assert_eq!(a.books.len(), 1);
        assert_eq!(a.books[0].parts, [1]);
        assert_eq!(a.ignored, [Ignored { file: 0, reason: IgnoreReason::XheAac }]);
    }

    #[test]
    fn two_identical_conflict_copies_keep_one() {
        let t = [("audible_asin", "B0DDDDDDD4"), ("title", "Twins")];
        let files = vec![
            file("Twins [B0DDDDDDD4]/Twins.m4b-1754152438.952120-AbCdEfGh.m4b", 16 * H, &t),
            file("Twins [B0DDDDDDD4]/Twins.m4b-1754153859.068224-AbCdEfGh.m4b", 16 * H, &t),
        ];
        let a = assemble(&files);
        assert_eq!(a.books.len(), 1);
        assert_eq!(a.ignored.len(), 1);
        assert!(matches!(a.ignored[0].reason, IgnoreReason::Duplicate { .. }));
    }

    #[test]
    fn a_plain_copy_beats_a_conflict_copy_at_equal_quality() {
        let t = [("title", "Same")];
        let files = vec![file("S/Same.m4b-1754152438.952120-AbCdEfGh.m4b", H, &t), file("S/Same.m4b", H, &t)];
        assert_eq!(assemble(&files).books[0].parts, [1]);
    }

    #[test]
    fn numbered_parts_are_one_book_in_natural_order() {
        let mut files = Vec::new();
        for n in [10, 2, 1, 55, 3] {
            files.push(file(
                &format!("Series/Book (1999)/Far Station {n:02}-55.mp3"),
                14 * 60_000,
                &[("album", "Far Station")],
            ));
        }
        let a = assemble(&files);
        assert_eq!(a.books.len(), 1);
        let b = &a.books[0];
        assert!(b.needs_copy);
        assert_eq!(
            paths(&files, b),
            [
                "Far Station 01-55.mp3",
                "Far Station 02-55.mp3",
                "Far Station 03-55.mp3",
                "Far Station 10-55.mp3",
                "Far Station 55-55.mp3"
            ]
        );
        assert_eq!(b.duration_ms, 5 * 14 * 60_000);
    }

    #[test]
    fn track_tags_beat_file_names() {
        let files = vec![
            file("B/z.mp3", 1000, &[("track", "2/3")]),
            file("B/a.mp3", 1000, &[("track", "3/3")]),
            file("B/m.mp3", 1000, &[("track", "1")]),
        ];
        assert_eq!(paths(&files, &assemble(&files).books[0]), ["m.mp3", "z.mp3", "a.mp3"]);
    }

    #[test]
    fn a_flat_folder_of_different_books_is_split() {
        let files = vec![
            file("Mixed/one.m4b", 10 * H, &[("audible_asin", "B0EEEEEEE5")]),
            file("Mixed/two.m4b", 8 * H, &[("audible_asin", "B0FFFFFFF6")]),
        ];
        let a = assemble(&files);
        assert_eq!(a.books.len(), 2);
        assert!(a.books.iter().all(|b| b.parts.len() == 1));
    }

    #[test]
    fn disc_folders_merge_into_the_parent() {
        let files = vec![
            file("Big Book/CD2/01.mp3", 1000, &[]),
            file("Big Book/CD1/02.mp3", 1000, &[]),
            file("Big Book/CD1/01.mp3", 1000, &[]),
            file("Big Book/Disc 10/01.mp3", 1000, &[]),
        ];
        let a = assemble(&files);
        assert_eq!(a.books.len(), 1);
        assert_eq!(a.books[0].key, "Big Book");
        let order: Vec<&str> = a.books[0].parts.iter().map(|&i| files[i].rel_path.as_str()).collect();
        assert_eq!(
            order,
            ["Big Book/CD1/01.mp3", "Big Book/CD1/02.mp3", "Big Book/CD2/01.mp3", "Big Book/Disc 10/01.mp3"]
        );
    }

    #[test]
    fn unplayable_files_are_reported_not_used() {
        let mut bad = file("X/broken.m4b", H, &[]);
        bad.decodes = false;
        let empty = file("Y/empty.m4b", 0, &[]);
        let a = assemble(&[bad, empty]);
        assert!(a.books.is_empty());
        assert_eq!(
            a.ignored.iter().map(|i| i.reason.clone()).collect::<Vec<_>>(),
            [IgnoreReason::DecodeFailed, IgnoreReason::Empty]
        );
    }

    #[test]
    fn a_non_browser_codec_needs_a_copy() {
        let mut f = file("F/book.flac", H, &[]);
        f.codec = Codec::Flac;
        assert!(assemble(&[f]).books[0].needs_copy);
    }

    #[test]
    fn disc_names_are_recognised() {
        for (n, want) in [
            ("CD1", Some(1)),
            ("cd 02", Some(2)),
            ("Disc_3", Some(3)),
            ("Part 12", Some(12)),
            ("Disk-4", Some(4)),
            ("Cdx", None),
            ("Partners", None),
            ("CD", None),
        ] {
            assert_eq!(disc_number(n), want, "{n}");
        }
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["Part 10", "part 2", "Part 1", "Part 02b", "Epilogue"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["Epilogue", "Part 1", "part 2", "Part 02b", "Part 10"]);
    }

    #[test]
    fn the_same_book_in_two_folders_is_one_book() {
        let t = [("audible_asin", "B0GGGGGGG7"), ("title", "Flowers")];
        let mut low = file("Flowers For X [B0GGGGGGG7]/a.m4b", 8 * H, &t);
        low.bitrate = 62_000;
        let mut high = file("Flowers for X [B0GGGGGGG7]/a.m4b", 8 * H + 1000, &t);
        high.bitrate = 125_000;
        let a = assemble(&[low, high]);
        assert_eq!(a.books.len(), 1);
        assert_eq!(a.books[0].parts, [1]);
        assert_eq!(a.ignored, [Ignored { file: 0, reason: IgnoreReason::Duplicate { kept: 1 } }]);
    }

    #[test]
    fn same_asin_but_different_lengths_stays_two_books() {
        let t = [("audible_asin", "B0HHHHHHH8")];
        let a = assemble(&[file("A/x.m4b", 8 * H, &t), file("B/y.m4b", 2 * H, &t)]);
        assert_eq!(a.books.len(), 2);
    }

    #[test]
    fn assembly_never_panics_and_uses_each_file_once() {
        let mut x: u64 = 7;
        for _ in 0..500 {
            let mut files = Vec::new();
            for i in 0..(x % 30) {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let dir = ["", "A", "A/CD1", "A/CD2", "B", "B/x"][(x % 6) as usize];
                let p = if dir.is_empty() { format!("f{i}.mp3") } else { format!("{dir}/f{i}.mp3") };
                let asin = ["", "B0AAAAAAA1", "B0BBBBBBB2"][(x % 3) as usize];
                let mut f = file(&p, x % 5 * 1000, &[("audible_asin", asin), ("track", &(x % 4).to_string())]);
                f.decodes = !x.is_multiple_of(7);
                files.push(f);
            }
            let a = assemble(&files);
            let mut seen: Vec<usize> =
                a.books.iter().flat_map(|b| b.parts.clone()).chain(a.ignored.iter().map(|i| i.file)).collect();
            seen.sort_unstable();
            let n = seen.len();
            seen.dedup();
            assert_eq!(seen.len(), n, "a file used twice");
            assert_eq!(n, files.len(), "a file lost");
        }
    }
}
