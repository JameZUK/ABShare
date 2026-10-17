//! Chapters on the book timeline (SPEC §4.5). Pure.

use crate::assemble::natural_cmp;
use crate::model::{ProbedFile, RawChapter};
use crate::text::clean_line;

/// Most chapters kept for one book.
pub const MAX_CHAPTERS: usize = 2_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chapter {
    pub start_ms: u64,
    pub end_ms: u64,
    pub title: String,
}

/// Makes one file's embedded chapters sane: sorted, inside the duration, no overlaps or gaps
/// (each chapter ends where the next starts; the last ends at the duration), no
/// zero-length chapters, cleaned titles, at most [`MAX_CHAPTERS`]. Returns an empty list
/// when there are no usable chapters.
pub fn sanitise(raw: &[RawChapter], duration_ms: u64) -> Vec<Chapter> {
    let mut starts: Vec<(u64, &str)> =
        raw.iter().filter(|c| c.start_ms < duration_ms).map(|c| (c.start_ms, c.title.as_str())).collect();
    starts.sort_by_key(|&(s, _)| s);
    starts.dedup_by_key(|&mut (s, _)| s);
    starts.truncate(MAX_CHAPTERS);
    if starts.is_empty() || duration_ms == 0 {
        return Vec::new();
    }
    // The first chapter always starts at 0, so the whole book is covered.
    starts[0].0 = 0;
    let n = starts.len();
    (0..n)
        .map(|i| {
            let end = if i + 1 < n { starts[i + 1].0 } else { duration_ms };
            let title = clean_line(starts[i].1, 200);
            Chapter {
                start_ms: starts[i].0,
                end_ms: end,
                title: if title.is_empty() { format!("Chapter {}", i + 1) } else { title },
            }
        })
        .filter(|c| c.end_ms > c.start_ms)
        .collect()
}

/// Chapters for a book made of `parts` in play order: each part's own chapters offset onto
/// the book timeline, or one chapter per part when a part has none.
pub fn for_parts(parts: &[&ProbedFile]) -> Vec<Chapter> {
    let mut out = Vec::new();
    let mut offset = 0u64;
    for (n, p) in parts.iter().enumerate() {
        let own = sanitise(&p.chapters, p.duration_ms);
        if own.is_empty() || parts.len() > 1 && own.len() == 1 {
            out.push(Chapter { start_ms: offset, end_ms: offset + p.duration_ms, title: part_title(p, n + 1) });
        } else {
            out.extend(own.into_iter().map(|c| Chapter {
                start_ms: c.start_ms + offset,
                end_ms: c.end_ms + offset,
                ..c
            }));
        }
        offset += p.duration_ms;
        if out.len() >= MAX_CHAPTERS {
            out.truncate(MAX_CHAPTERS);
            break;
        }
    }
    // A single file with no chapters gets none: the player shows the book as one span.
    if parts.len() == 1 && parts[0].chapters.is_empty() {
        return Vec::new();
    }
    out
}

fn part_title(p: &ProbedFile, n: usize) -> String {
    let t = p.tags.get(&["title"]).map(|t| clean_line(t, 200)).unwrap_or_default();
    if !t.is_empty() {
        return t;
    }
    let stem = p.file_name().rsplit_once('.').map_or(p.file_name(), |(s, _)| s);
    let t = clean_line(&stem.replace('_', " "), 200);
    if t.is_empty() { format!("Part {n}") } else { t }
}

/// The chapter containing `position_ms` (the last one at the very end).
pub fn at(chapters: &[Chapter], position_ms: u64) -> Option<usize> {
    if chapters.is_empty() {
        return None;
    }
    Some(chapters.partition_point(|c| c.start_ms <= position_ms).saturating_sub(1))
}

/// Sorts chapter titles naturally, for tests and admin views.
pub fn sorted_titles(chapters: &[Chapter]) -> Vec<String> {
    let mut v: Vec<String> = chapters.iter().map(|c| c.title.clone()).collect();
    v.sort_by(|a, b| natural_cmp(a, b));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assemble::tests::file;

    fn raw(list: &[(u64, u64, &str)]) -> Vec<RawChapter> {
        list.iter().map(|&(s, e, t)| RawChapter { start_ms: s, end_ms: e, title: t.into() }).collect()
    }

    #[test]
    fn overlaps_gaps_and_unsorted_chapters_are_fixed() {
        let c = sanitise(&raw(&[(5000, 9000, "Two"), (100, 6000, "One"), (9500, 99_999, "Three")]), 12_000);
        assert_eq!(
            c,
            [
                Chapter { start_ms: 0, end_ms: 5000, title: "One".into() },
                Chapter { start_ms: 5000, end_ms: 9500, title: "Two".into() },
                Chapter { start_ms: 9500, end_ms: 12_000, title: "Three".into() },
            ]
        );
    }

    #[test]
    fn chapters_past_the_end_duplicates_and_blank_titles() {
        let c = sanitise(&raw(&[(0, 1, ""), (0, 1, "dup"), (50_000, 60_000, "beyond")]), 10_000);
        assert_eq!(c, [Chapter { start_ms: 0, end_ms: 10_000, title: "Chapter 1".into() }]);
        assert!(sanitise(&[], 10_000).is_empty());
        assert!(sanitise(&raw(&[(0, 5, "x")]), 0).is_empty());
    }

    #[test]
    fn the_cap_holds() {
        let many: Vec<RawChapter> =
            (0..5000).map(|i| RawChapter { start_ms: i * 10, end_ms: i * 10 + 10, title: "c".into() }).collect();
        assert_eq!(sanitise(&many, 1_000_000).len(), MAX_CHAPTERS);
    }

    #[test]
    fn parts_without_chapters_become_one_chapter_each() {
        let a = file("B/Far Station 01-55.mp3", 1000, &[]);
        let b = file("B/Far Station 02-55.mp3", 2000, &[("title", "Far Station 02-55")]);
        let c = for_parts(&[&a, &b]);
        assert_eq!(c.len(), 2);
        assert_eq!((c[1].start_ms, c[1].end_ms), (1000, 3000));
        assert_eq!(c[0].title, "Far Station 01-55");
    }

    #[test]
    fn part_chapters_are_offset_onto_the_book_timeline() {
        let mut a = file("B/a.m4b", 10_000, &[]);
        a.chapters = raw(&[(0, 4000, "A1"), (4000, 10_000, "A2")]);
        let mut b = file("B/b.m4b", 6000, &[]);
        b.chapters = raw(&[(0, 3000, "B1"), (3000, 6000, "B2")]);
        let c = for_parts(&[&a, &b]);
        assert_eq!(
            c.iter().map(|c| (c.start_ms, c.title.as_str())).collect::<Vec<_>>(),
            [(0, "A1"), (4000, "A2"), (10_000, "B1"), (13_000, "B2")]
        );
    }

    #[test]
    fn a_single_file_without_chapters_gets_none() {
        let a = file("B/long.m4b", 10 * 3_600_000, &[]);
        assert!(for_parts(&[&a]).is_empty());
    }

    #[test]
    fn position_lookup() {
        let c = sanitise(&raw(&[(0, 0, "a"), (1000, 0, "b"), (2000, 0, "c")]), 3000);
        assert_eq!(at(&c, 0), Some(0));
        assert_eq!(at(&c, 1999), Some(1));
        assert_eq!(at(&c, 2000), Some(2));
        assert_eq!(at(&c, 99_999), Some(2));
        assert_eq!(at(&[], 5), None);
    }
}
