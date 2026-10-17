//! Series and shows (SPEC §4.3.1): grouping, classification and ordering. Pure.
//!
//! Store tags use one `series` value for book series and for episodic shows. Groups are
//! classified automatically. Shows are ordered by the season and episode parsed from the
//! title, because store `part` tags for shows are sentinels or in the wrong order.

use crate::assemble::natural_cmp;
use crate::metadata::{BookMeta, Position};
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// What a group of items is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    BookSeries,
    Show,
}

/// The parts of one item that matter for grouping and ordering.
#[derive(Clone, Debug)]
pub struct Item<'a> {
    pub meta: &'a BookMeta,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub qualifier: Option<String>,
    pub kind: Kind,
    /// Indices into the input, in play order (episodes first, then extras for shows).
    pub members: Vec<usize>,
    /// For shows: how many leading `members` are episodes (the rest are extras).
    pub episodes: usize,
}

/// Parsed episode numbering.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Episode {
    pub season: Option<u32>,
    pub number: Option<u32>,
    pub extra: bool,
}

const TWO_HOURS: u64 = 2 * 3_600_000;

fn group_key(m: &BookMeta) -> Option<(String, Option<String>)> {
    if let Some(s) = &m.series {
        return Some((s.name.clone(), s.qualifier.clone()));
    }
    let podcast = m.genres.iter().any(|g| g.eq_ignore_ascii_case("podcast"));
    match (&m.album, podcast) {
        (Some(a), true) => Some((a.clone(), None)),
        _ => None,
    }
}

/// Groups items by series (or, for podcast-genre items without a series, by album) and
/// classifies and orders each group. Items in no group are not returned.
pub fn group(items: &[Item<'_>]) -> Vec<Group> {
    let mut by: BTreeMap<String, (String, Option<String>, Vec<usize>)> = BTreeMap::new();
    for (i, it) in items.iter().enumerate() {
        if let Some((name, q)) = group_key(it.meta) {
            by.entry(name.to_lowercase()).or_insert_with(|| (name, q, Vec::new())).2.push(i);
        }
    }
    let mut out = Vec::new();
    for (_, (name, qualifier, members)) in by {
        if members.len() < 2 && !members.iter().any(|&i| is_podcast(items[i].meta)) {
            continue;
        }
        let kind = classify(items, &members);
        let mut g = Group { name, qualifier, kind, members, episodes: 0 };
        order(items, &mut g);
        out.push(g);
    }
    out
}

fn is_podcast(m: &BookMeta) -> bool {
    m.genres.iter().any(|g| g.eq_ignore_ascii_case("podcast"))
}

/// The rules of SPEC §4.3.1, first match wins.
pub fn classify(items: &[Item<'_>], members: &[usize]) -> Kind {
    if members.iter().any(|&i| is_podcast(items[i].meta)) {
        return Kind::Show;
    }
    let n = members.len();
    let short = members.iter().filter(|&&i| items[i].duration_ms < TWO_HOURS).count();
    let episodic = members.iter().filter(|&&i| parse_episode(&items[i].meta.title).number.is_some()).count();
    if (short * 3 >= n * 2 && episodic * 3 >= n * 2) || (short == n && n >= 3) { Kind::Show } else { Kind::BookSeries }
}

fn order(items: &[Item<'_>], g: &mut Group) {
    match g.kind {
        Kind::BookSeries => {
            g.members.sort_by(|&a, &b| book_order(items[a].meta, items[b].meta));
            g.episodes = 0;
        }
        Kind::Show => {
            let parsed: Vec<(usize, Episode)> =
                g.members.iter().map(|&i| (i, parse_episode(&items[i].meta.title))).collect();
            let (mut eps, mut extras): (Vec<_>, Vec<_>) = parsed.into_iter().partition(|(_, e)| !e.extra);
            eps.sort_by(|(a, ea), (b, eb)| {
                ea.season
                    .unwrap_or(0)
                    .cmp(&eb.season.unwrap_or(0))
                    .then(ea.number.unwrap_or(u32::MAX).cmp(&eb.number.unwrap_or(u32::MAX)))
                    .then_with(|| year_cmp(items[*a].meta, items[*b].meta))
                    .then_with(|| natural_cmp(&items[*a].meta.title, &items[*b].meta.title))
            });
            extras.sort_by(|(a, _), (b, _)| {
                year_cmp(items[*a].meta, items[*b].meta)
                    .then_with(|| natural_cmp(&items[*a].meta.title, &items[*b].meta.title))
            });
            g.episodes = eps.len();
            g.members = eps.into_iter().chain(extras).map(|(i, _)| i).collect();
        }
    }
}

fn year_cmp(a: &BookMeta, b: &BookMeta) -> Ordering {
    a.year.unwrap_or(u16::MAX).cmp(&b.year.unwrap_or(u16::MAX))
}

fn position(m: &BookMeta) -> Option<Position> {
    m.series.as_ref().and_then(|s| s.position)
}

/// Book series: position (companions at 0 first), then year, then title. Unnumbered
/// books go after numbered ones.
fn book_order(a: &BookMeta, b: &BookMeta) -> Ordering {
    match (position(a), position(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
    .then_with(|| year_cmp(a, b))
    .then_with(|| natural_cmp(&a.title, &b.title))
}

const EXTRA_WORDS: &[&str] = &[
    "trailer",
    "announcement",
    "announcements",
    "update",
    "updates",
    "bonus",
    "aftershow",
    "preview",
    "presents",
    "teaser",
    "sneak peek",
    "behind the scenes",
    "q&a",
];

fn num_after(s: &str) -> Option<u32> {
    let digits: String = s.trim_start().chars().take_while(|c| c.is_ascii_digit()).take(5).collect();
    digits.parse().ok()
}

/// Parses season and episode numbers from a title. Recognises "Ep. 7:", "Ep: 7:",
/// "Ep 7", "Episode 7", "E7 -", "S2E7", "Season 2 - … - E7", "Part 7" and "07-55".
pub fn parse_episode(title: &str) -> Episode {
    let t = title.to_lowercase();
    let mut e = Episode { extra: EXTRA_WORDS.iter().any(|w| contains_word(&t, w)), ..Episode::default() };
    // Season.
    if let Some(i) = t.find("season ") {
        e.season = num_after(&t[i + 7..]);
    }
    // SxxEyy.
    let b = t.as_bytes();
    for i in 0..b.len() {
        if b[i] == b's' && (i == 0 || !b[i - 1].is_ascii_alphanumeric()) {
            let rest = &t[i + 1..];
            if let Some(s) = num_after(rest) {
                let after = rest.trim_start_matches(|c: char| c.is_ascii_digit());
                if let Some(ep) = after.strip_prefix('e').and_then(num_after) {
                    e.season = Some(s);
                    e.number = Some(ep);
                    return e;
                }
            }
        }
    }
    // Episode markers, most specific first.
    for marker in ["episode", "ep.", "ep:", "ep "] {
        if let Some(i) = find_word_start(&t, marker) {
            let rest = t[i + marker.len()..].trim_start_matches([' ', ':', '.', '#']);
            if let Some(n) = num_after(rest) {
                e.number = Some(n);
                return e;
            }
        }
    }
    // A bare "E7" word.
    for (i, w) in t.match_indices('e') {
        let before_ok = i == 0 || !b[i - 1].is_ascii_alphanumeric();
        let rest = &t[i + w.len()..];
        if before_ok && rest.starts_with(|c: char| c.is_ascii_digit()) {
            let tail = rest.trim_start_matches(|c: char| c.is_ascii_digit());
            if tail.is_empty() || tail.starts_with([' ', '-', ':', '.']) {
                e.number = num_after(rest);
                return e;
            }
        }
    }
    if let Some(i) = find_word_start(&t, "part ") {
        e.number = num_after(&t[i + 5..]);
        if e.number.is_some() {
            return e;
        }
    }
    // "Name 07-55": number of total.
    if let Some((left, right)) = t.rsplit_once('-') {
        let n: String =
            left.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
        if !n.is_empty() && right.trim().bytes().all(|c| c.is_ascii_digit()) && !right.trim().is_empty() {
            e.number = n.parse().ok();
        }
    }
    e
}

fn find_word_start(t: &str, w: &str) -> Option<usize> {
    t.match_indices(w).map(|(i, _)| i).find(|&i| i == 0 || !t.as_bytes()[i - 1].is_ascii_alphanumeric())
}

fn contains_word(t: &str, w: &str) -> bool {
    t.match_indices(w).any(|(i, _)| {
        let before = i == 0 || !t.as_bytes()[i - 1].is_ascii_alphanumeric();
        let end = i + w.len();
        let after = end >= t.len() || !t.as_bytes()[end].is_ascii_alphanumeric();
        before && after
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::SeriesRef;

    const H: u64 = 3_600_000;

    fn meta(title: &str, series: Option<&str>, part: Option<&str>, genres: &[&str], album: Option<&str>) -> BookMeta {
        BookMeta {
            title: title.into(),
            series: series.map(|s| SeriesRef {
                name: s.into(),
                qualifier: None,
                position: part.and_then(Position::parse),
            }),
            genres: genres.iter().map(|g| (*g).into()).collect(),
            album: album.map(Into::into),
            ..BookMeta::default()
        }
    }

    fn titles<'a>(metas: &'a [BookMeta], g: &Group) -> Vec<&'a str> {
        g.members.iter().map(|&i| metas[i].title.as_str()).collect()
    }

    #[test]
    fn episode_titles_parse() {
        let cases = [
            ("Ep. 11: Do Not Panic.", None, Some(11)),
            ("Ep: 2: A Fridge?", None, Some(2)),
            ("Ep 5: Chatroom", None, Some(5)),
            ("Episode 6", None, Some(6)),
            ("Season 2 - SHOWNAME - E9 - Knights", Some(2), Some(9)),
            ("Season 2 - SHOWNAME - E6- Toys", Some(2), Some(6)),
            ("S03E04 Return", Some(3), Some(4)),
            ("Far Station 07-55", None, Some(7)),
            ("Part 3", None, Some(3)),
            ("Their Second Book", None, None),
            ("Echoes", None, None),
        ];
        for (t, s, n) in cases {
            let e = parse_episode(t);
            assert_eq!((e.season, e.number), (s, n), "{t}");
        }
        assert!(parse_episode("SHOWNAME Season 2 Trailer").extra);
        assert!(parse_episode("Season 3 - Quick Updates").extra);
        assert!(parse_episode("SHOWNAME Presents: Another Show").extra);
        assert!(!parse_episode("The Presentation").extra);
    }

    /// A show whose `part` tags are in purchase order: titles decide the order.
    #[test]
    fn a_show_is_ordered_by_title_not_part() {
        let metas: Vec<BookMeta> = [("Ep. 11: K", "1"), ("Ep. 1: A", "3"), ("Ep. 2: B", "5"), ("Ep. 10: J", "14")]
            .iter()
            .map(|(t, p)| meta(t, Some("The Family"), Some(p), &["Literature & Fiction"], None))
            .collect();
        let items: Vec<Item> = metas.iter().map(|m| Item { meta: m, duration_ms: H / 4 }).collect();
        let g = &group(&items)[0];
        assert_eq!(g.kind, Kind::Show);
        assert_eq!(titles(&metas, g), ["Ep. 1: A", "Ep. 2: B", "Ep. 10: J", "Ep. 11: K"]);
    }

    #[test]
    fn a_podcast_with_seasons_and_extras() {
        let names = [
            "Season 2 - SHOW - E2 - Two",
            "Season 1 - OTHER - E10 - Ten",
            "SHOW Season 2 Trailer",
            "Season 1 - OTHER - E1 - One",
            "SHOW Presents: Guest",
            "Season 2 - SHOW - E1 - One",
        ];
        let metas: Vec<BookMeta> = names.iter().map(|t| meta(t, None, None, &["Podcast"], Some("SHOW"))).collect();
        let items: Vec<Item> = metas.iter().map(|m| Item { meta: m, duration_ms: H }).collect();
        let g = &group(&items)[0];
        assert_eq!(g.kind, Kind::Show);
        assert_eq!(g.episodes, 4);
        assert_eq!(
            titles(&metas, g),
            [
                "Season 1 - OTHER - E1 - One",
                "Season 1 - OTHER - E10 - Ten",
                "Season 2 - SHOW - E1 - One",
                "Season 2 - SHOW - E2 - Two",
                "SHOW Presents: Guest",
                "SHOW Season 2 Trailer",
            ]
        );
    }

    #[test]
    fn a_book_series_is_ordered_by_position_with_companions_first() {
        let metas = vec![
            meta("Third", Some("Saga"), Some("3"), &[], None),
            meta("Prequel", Some("Saga"), Some("0"), &[], None),
            meta("Novella", Some("Saga"), Some("1.5"), &[], None),
            meta("First", Some("Saga"), Some("1"), &[], None),
            meta("Unnumbered", Some("Saga"), None, &[], None),
        ];
        let items: Vec<Item> = metas.iter().map(|m| Item { meta: m, duration_ms: 15 * H }).collect();
        let g = &group(&items)[0];
        assert_eq!(g.kind, Kind::BookSeries);
        assert_eq!(titles(&metas, g), ["Prequel", "First", "Novella", "Third", "Unnumbered"]);
    }

    #[test]
    fn one_short_story_does_not_make_a_series_a_show() {
        let metas = [
            meta("Long One", Some("Dark Things"), Some("1"), &[], None),
            meta("Long Two", Some("Dark Things"), Some("2"), &[], None),
            meta("The Short", Some("Dark Things"), Some("0"), &[], None),
        ];
        let d = [13 * H, 9 * H, H / 2];
        let items: Vec<Item> = metas.iter().zip(d).map(|(m, d)| Item { meta: m, duration_ms: d }).collect();
        assert_eq!(group(&items)[0].kind, Kind::BookSeries);
    }

    #[test]
    fn short_numbered_items_are_a_show_even_without_episode_words() {
        let metas: Vec<BookMeta> = ["Episode 1", "Episode 2", "Episode 3"]
            .iter()
            .map(|t| meta(t, Some("Run"), Some("999999000"), &[], None))
            .collect();
        let items: Vec<Item> = metas.iter().map(|m| Item { meta: m, duration_ms: H / 3 }).collect();
        assert_eq!(group(&items)[0].kind, Kind::Show);
    }

    #[test]
    fn single_items_are_not_groups_unless_podcasts() {
        let metas = [meta("Alone", Some("Solo"), Some("1"), &[], None)];
        let items: Vec<Item> = metas.iter().map(|m| Item { meta: m, duration_ms: H }).collect();
        assert!(group(&items).is_empty());
    }

    #[test]
    fn parse_episode_never_panics() {
        let mut x: u64 = 99;
        let bits = ["ep", ".", ":", " ", "e", "s", "1", "9", "-", "season ", "part ", "é", "x"];
        for _ in 0..20_000 {
            let mut s = String::new();
            for _ in 0..(x % 16) {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                s.push_str(bits[(x % bits.len() as u64) as usize]);
            }
            let _ = parse_episode(&s);
        }
    }
}
