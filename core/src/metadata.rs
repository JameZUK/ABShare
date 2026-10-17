//! Tags to book metadata (SPEC §4.4). Pure; every input string is untrusted.

use crate::model::{Sidecars, Tags};
use crate::text::{MAX_DESCRIPTION, MAX_NAME, clean_line, clean_paragraphs, html_to_text};

/// Most people kept per role (full-cast productions can list dozens).
const MAX_PEOPLE: usize = 50;
const MAX_GENRES: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    /// A role taken from a suffix such as " - adaptation" or " - translator".
    pub role: Option<String>,
}

/// A position within a series. Stored in thousandths so "1.5" is exact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position(u32);

impl Position {
    /// Parses "3", "1.5", "02". Values ≥ 100 000 are store sentinels and are refused.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
        if whole.is_empty() || whole.len() > 5 || !whole.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        if frac.len() > 3 || !frac.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let w: u32 = whole.parse().ok()?;
        if w >= 100_000 {
            return None;
        }
        let f: u32 = if frac.is_empty() { 0 } else { format!("{frac:0<3}").parse().ok()? };
        Some(Self(w * 1000 + f))
    }

    /// 0 means a prequel or companion: sorted first, never shown as a number.
    pub fn is_companion(self) -> bool {
        self.0 == 0
    }

    pub fn thousandths(self) -> u32 {
        self.0
    }

    /// "3", "1.5".
    pub fn display(self) -> String {
        let (w, f) = (self.0 / 1000, self.0 % 1000);
        if f == 0 { w.to_string() } else { format!("{w}.{}", format!("{f:03}").trim_end_matches('0')) }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeriesRef {
    pub name: String,
    /// A trailing bracketed qualifier such as "Chronological Order", shown on the series page.
    pub qualifier: Option<String>,
    pub position: Option<Position>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BookMeta {
    pub title: String,
    pub subtitle: Option<String>,
    pub authors: Vec<Person>,
    pub narrators: Vec<Person>,
    pub series: Option<SeriesRef>,
    pub description: Option<String>,
    pub genres: Vec<String>,
    pub year: Option<u16>,
    pub publisher: Option<String>,
    pub language: Option<String>,
    pub asin: Option<String>,
    pub isbn: Option<String>,
    /// The `album` tag, cleaned: used to tell books apart and to group episodes.
    pub album: Option<String>,
}

/// Builds metadata for a book from its first file's tags, its folder name and sidecars.
pub fn book_meta(tags: &Tags, folder: Option<&str>, file_name: &str, side: &Sidecars) -> BookMeta {
    let line = |keys: &[&str]| tags.get(keys).map(|v| clean_line(v, MAX_NAME)).filter(|v| !v.is_empty());

    let album = line(&["album"]);
    let (mut title, narrated_by) = split_narrated_by(
        line(&["title"])
            .or_else(|| album.as_deref().map(strip_edition_suffix))
            .or_else(|| folder.map(|f| clean_line(strip_trailing_id(f), MAX_NAME)))
            .unwrap_or_else(|| clean_line(strip_trailing_id(strip_extension(file_name)), MAX_NAME)),
    );
    if title.is_empty() {
        title = "Untitled".into();
    }

    let authors = people(tags.get(&["album_artist", "artist", "author", "writer"]).unwrap_or(""));
    let mut narrators = people(tags.get(&["composer", "narrator", "performer"]).unwrap_or(""));
    if narrators.is_empty() {
        if let Some(n) = narrated_by {
            narrators = people(&n);
        } else if let Some(n) = side.reader.as_deref().map(str::to_owned).or_else(|| read_by(&side.info)) {
            narrators = people(&n);
        }
    }

    let series = series(tags, album.as_deref(), &title);
    let description = description(tags, side);
    let genres = genres(tags.get(&["genre"]).unwrap_or(""));
    let year = tags.get(&["date", "year", "tyer", "tdrc"]).and_then(year);
    let asin = tags
        .get(&["audible_asin", "asin"])
        .and_then(valid_asin)
        .or_else(|| folder.and_then(trailing_id).and_then(valid_asin));
    let isbn = tags.get(&["isbn"]).and_then(valid_isbn);

    BookMeta {
        title,
        subtitle: line(&["subtitle"]),
        authors,
        narrators,
        series,
        description,
        genres,
        year,
        publisher: line(&["publisher"]),
        language: line(&["language"]),
        asin,
        isbn,
        album,
    }
}

fn strip_extension(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

/// "Title [B0ABCDE123]" → "Title".
pub fn strip_trailing_id(s: &str) -> &str {
    match (s.trim_end().strip_suffix(']'), s.rfind('[')) {
        (Some(_), Some(i)) if trailing_id(s).is_some() => s[..i].trim_end(),
        _ => s.trim(),
    }
}

/// The ID in a trailing "[...]", if it looks like an ASIN or ISBN.
pub fn trailing_id(s: &str) -> Option<&str> {
    let s = s.trim_end();
    let inner = s.strip_suffix(']')?;
    let id = &inner[inner.rfind('[')? + 1..];
    (matches!(id.len(), 10 | 13) && id.bytes().all(|c| c.is_ascii_alphanumeric())).then_some(id)
}

fn valid_asin(s: &str) -> Option<String> {
    let s = s.trim();
    (s.len() == 10 && s.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())).then(|| s.to_owned())
}

fn valid_isbn(s: &str) -> Option<String> {
    let d: String = s.chars().filter(|c| !matches!(c, '-' | ' ')).collect();
    (matches!(d.len(), 10 | 13) && d.bytes().all(|c| c.is_ascii_digit() || c == b'X')).then_some(d)
}

/// "Title: Series, Book 6 (Unabridged)" → "Title: Series, Book 6".
fn strip_edition_suffix(s: &str) -> String {
    let mut t = s.trim();
    for suffix in [" (Unabridged)", " (Abridged)", " (Dramatized)", " (Dramatised)"] {
        if let Some(x) = t.strip_suffix(suffix) {
            t = x.trim_end();
        }
    }
    t.to_owned()
}

/// "Lock In (Narrated by A Reader)" → ("Lock In", Some("A Reader")).
fn split_narrated_by(title: String) -> (String, Option<String>) {
    let lower = title.to_lowercase();
    for marker in ["(narrated by ", "(read by "] {
        if let Some(i) = lower.rfind(marker) {
            if title.ends_with(')') && title.is_char_boundary(i) {
                let name = title[i + marker.len()..title.len() - 1].trim().to_owned();
                let t = title[..i].trim_end().to_owned();
                if !t.is_empty() && !name.is_empty() {
                    return (t, Some(name));
                }
            }
        }
    }
    (title, None)
}

/// A "Read by X" line in any of the info texts.
fn read_by(info: &[String]) -> Option<String> {
    info.iter().flat_map(|t| t.lines()).find_map(|l| {
        let l = l.trim();
        let lower = l.to_lowercase();
        ["read by ", "narrated by "].iter().find_map(|m| lower.starts_with(m).then(|| l[m.len()..].trim().to_owned()))
    })
}

const NAME_SUFFIXES: &[&str] =
    &["Jr.", "Jr", "Sr.", "Sr", "II", "III", "IV", "PhD", "Ph.D.", "MD", "OBE", "MBE", "CBE"];

/// Splits a people tag on ", " and ";" (store tags join names with ", "; splitting on "&"
/// or "and" would break names), keeping suffixes such as "Jr." with the name before.
pub fn people(raw: &str) -> Vec<Person> {
    let cleaned = clean_line(raw, 4 * MAX_NAME);
    let mut out: Vec<Person> = Vec::new();
    for part in cleaned.split([',', ';']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if NAME_SUFFIXES.contains(&part) {
            if let Some(last) = out.last_mut() {
                last.name = format!("{}, {part}", last.name);
            }
            continue;
        }
        let (name, role) = match part.rsplit_once(" - ") {
            Some((n, r)) if !n.trim().is_empty() && r.split_whitespace().count() <= 3 => {
                (n.trim().to_owned(), Some(r.trim().to_lowercase()))
            }
            _ => (part.to_owned(), None),
        };
        if !out.iter().any(|p| p.name == name) {
            out.push(Person { name: clean_line(&name, 200), role });
        }
        if out.len() >= MAX_PEOPLE {
            break;
        }
    }
    out
}

fn genres(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for g in clean_line(raw, 2000).split([',', ';', '/']) {
        let g = g.trim();
        if !g.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(g)) {
            out.push(clean_line(g, 80));
        }
        if out.len() >= MAX_GENRES {
            break;
        }
    }
    out
}

fn year(s: &str) -> Option<u16> {
    let digits: String = s.trim().chars().take(4).collect();
    let y: u16 = digits.parse().ok()?;
    (1000..=2100).contains(&y).then_some(y)
}

fn description(tags: &Tags, side: &Sidecars) -> Option<String> {
    let raw = tags
        .get(&["description", "comment", "synopsis", "lyrics-eng", "lyrics"])
        .map(str::to_owned)
        .or_else(|| side.desc.clone())?;
    let text = if raw.contains('<') && raw.contains('>') { html_to_text(&raw) } else { raw };
    let d = clean_paragraphs(&text, MAX_DESCRIPTION);
    // Some rips put a placeholder in the comment tag.
    let trivial = matches!(d.to_ascii_lowercase().as_str(), "unabridged" | "abridged" | "audiobook");
    (!d.is_empty() && !trivial).then_some(d)
}

fn series(tags: &Tags, album: Option<&str>, title: &str) -> Option<SeriesRef> {
    let name_raw = tags.get(&["series", "mvnm", "grouping"])?;
    let name_clean = clean_line(name_raw, MAX_NAME);
    let (name, qualifier) = split_qualifier(&name_clean);
    if name.is_empty() {
        return None;
    }
    let position = tags
        .get(&["part", "series-part", "mvin"])
        .and_then(Position::parse)
        .or_else(|| album.and_then(book_number))
        .or_else(|| book_number(title));
    Some(SeriesRef { name, qualifier, position })
}

/// "Long Saga [Chronological Order]" → ("Long Saga", Some("Chronological Order")).
fn split_qualifier(s: &str) -> (String, Option<String>) {
    if let (Some(inner), Some(i)) = (s.strip_suffix(']'), s.rfind('[')) {
        let q = inner[i + 1..].trim();
        let n = s[..i].trim();
        if !n.is_empty() && !q.is_empty() {
            return (n.to_owned(), Some(q.to_owned()));
        }
    }
    (s.to_owned(), None)
}

/// "…, Book 6" or "… Book 2.5 …" → 6 / 2.5.
fn book_number(s: &str) -> Option<Position> {
    let lower = s.to_lowercase();
    let i = lower.rfind("book ")?;
    let rest = &s[i + 5..];
    let num: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    Position::parse(num.trim_end_matches('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(pairs: &[(&str, &str)]) -> Tags {
        Tags::new(pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())))
    }

    fn names(p: &[Person]) -> Vec<&str> {
        p.iter().map(|p| p.name.as_str()).collect()
    }

    /// The common store layout: author in artist, narrator in composer, ASIN tag, series/part.
    #[test]
    fn a_store_style_m4b_maps_every_field() {
        let t = tags(&[
            ("title", "The Copper Tide"),
            ("album", "The Copper Tide: Harbour Lights, Book 2 (Unabridged)"),
            ("artist", "Ada Example"),
            ("album_artist", "Ada Example"),
            ("composer", "Ben Reader, Cass Voice"),
            ("genre", "Fantasy, Adventure, Fantasy"),
            ("date", "2021"),
            ("publisher", "Invented Audio"),
            ("comment", "A storm rises over the harbour."),
            ("language", "English"),
            ("audible_asin", "B0TESTASIN"),
            ("series", "Harbour Lights"),
            ("part", "2"),
            ("subtitle", "A Novel"),
        ]);
        let m = book_meta(&t, Some("The Copper Tide [B0TESTASIN]"), "x.m4b", &Sidecars::default());
        assert_eq!(m.title, "The Copper Tide");
        assert_eq!(m.subtitle.as_deref(), Some("A Novel"));
        assert_eq!(names(&m.authors), ["Ada Example"]);
        assert_eq!(names(&m.narrators), ["Ben Reader", "Cass Voice"]);
        assert_eq!(m.genres, ["Fantasy", "Adventure"]);
        assert_eq!(m.year, Some(2021));
        assert_eq!(m.asin.as_deref(), Some("B0TESTASIN"));
        let s = m.series.expect("series");
        assert_eq!((s.name.as_str(), s.position.map(Position::display)), ("Harbour Lights", Some("2".into())));
        assert_eq!(m.description.as_deref(), Some("A storm rises over the harbour."));
    }

    #[test]
    fn sentinel_parts_are_ignored_and_book_n_is_used_instead() {
        let t = tags(&[("title", "Ep. 3"), ("series", "Night Shift"), ("part", "999999000"), ("album", "Night Shift")]);
        assert_eq!(book_meta(&t, None, "f.m4b", &Sidecars::default()).series.and_then(|s| s.position), None);
        let t = tags(&[
            ("title", "Ember"),
            ("series", "Ash"),
            ("part", "999999000"),
            ("album", "Ember: Ash, Book 6 (Unabridged)"),
        ]);
        assert_eq!(
            book_meta(&t, None, "f.m4b", &Sidecars::default()).series.and_then(|s| s.position).map(Position::display),
            Some("6".into())
        );
    }

    #[test]
    fn part_zero_is_a_companion_and_fractions_are_exact() {
        assert!(Position::parse("0").is_some_and(Position::is_companion));
        assert_eq!(Position::parse("1.5").map(Position::display), Some("1.5".into()));
        assert_eq!(Position::parse("16.25").map(Position::display), Some("16.25".into()));
        assert!(Position::parse("1.5") > Position::parse("1"));
        for bad in ["", "-1", "1e3", "100000", "999999000", "1.2345", "abc", "1..2"] {
            assert_eq!(Position::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn series_qualifiers_are_split_off() {
        let t = tags(&[("title", "X"), ("series", "Long Saga [Chronological Order]"), ("part", "6")]);
        let s = book_meta(&t, None, "f.m4b", &Sidecars::default()).series.expect("series");
        assert_eq!(s.name, "Long Saga");
        assert_eq!(s.qualifier.as_deref(), Some("Chronological Order"));
    }

    #[test]
    fn people_split_on_commas_keep_suffixes_and_roles() {
        let p = people("Charles Old, R. D. Example - adaptation, Martin Luther King, Jr., ; ,  Dee Dupe, Dee Dupe");
        assert_eq!(names(&p), ["Charles Old", "R. D. Example", "Martin Luther King, Jr.", "Dee Dupe"]);
        assert_eq!(p[1].role.as_deref(), Some("adaptation"));
        assert_eq!(people("Penn and Teller")[0].name, "Penn and Teller");
        assert_eq!(people(&"A, ".repeat(200)).len(), 1);
        let many: String = (0..80).map(|i| format!("Reader {i}, ")).collect();
        assert_eq!(people(&many).len(), MAX_PEOPLE);
    }

    #[test]
    fn narrated_by_in_the_title_moves_to_narrators() {
        let t = tags(&[("title", "Lock Step (Narrated by Some Voice)"), ("artist", "An Author")]);
        let m = book_meta(&t, None, "f.m4b", &Sidecars::default());
        assert_eq!(m.title, "Lock Step");
        assert_eq!(names(&m.narrators), ["Some Voice"]);
    }

    #[test]
    fn untagged_files_fall_back_to_folder_then_file_name_and_sidecars() {
        let side = Sidecars { info: vec!["Ripped 1999\nRead by Ray Example\n".into()], ..Default::default() };
        let m = book_meta(&Tags::default(), Some("Far Station [B0FOLDER01]"), "part 01.mp3", &side);
        assert_eq!(m.title, "Far Station");
        assert_eq!(m.asin.as_deref(), Some("B0FOLDER01"));
        assert_eq!(names(&m.narrators), ["Ray Example"]);
        let m = book_meta(&Tags::default(), None, "Loose Track.mp3", &Sidecars::default());
        assert_eq!(m.title, "Loose Track");
    }

    #[test]
    fn album_without_title_loses_its_edition_suffix() {
        let t = tags(&[("album", "Some Gift Story (Unabridged)")]);
        assert_eq!(book_meta(&t, None, "f.m4b", &Sidecars::default()).title, "Some Gift Story");
    }

    #[test]
    fn html_descriptions_become_text_and_placeholders_are_dropped() {
        let t = tags(&[("title", "E1"), ("lyrics-eng", "<p><strong>Hello</strong> &amp; welcome</p><p>Part two</p>")]);
        assert_eq!(
            book_meta(&t, None, "f.mp3", &Sidecars::default()).description.as_deref(),
            Some("Hello & welcome\n\nPart two")
        );
        let t = tags(&[("title", "E1"), ("comment", "Unabridged")]);
        assert_eq!(book_meta(&t, None, "f.mp3", &Sidecars::default()).description, None);
    }

    #[test]
    fn ids_must_look_like_ids() {
        assert_eq!(trailing_id("Name [B0ABCDEF12]"), Some("B0ABCDEF12"));
        assert_eq!(trailing_id("Name [1234567890]"), Some("1234567890"));
        assert_eq!(trailing_id("Name [Chronological Order]"), None);
        assert_eq!(trailing_id("Name [../../etc]"), None);
        assert_eq!(strip_trailing_id("Name [B0ABCDEF12]"), "Name");
        assert_eq!(strip_trailing_id("Name [Book Two]"), "Name [Book Two]");
        assert_eq!(valid_asin("b0lower123"), None);
    }

    #[test]
    fn hostile_tags_are_cleaned() {
        let t = tags(&[
            ("title", "Evil\u{202E}txt.exe\u{200B}"),
            ("comment", "<img src=x onerror=alert(1)> stored at /srv/library/x.m4b"),
            ("artist", "\u{0000}Null Author"),
        ]);
        let m = book_meta(&t, None, "f.m4b", &Sidecars::default());
        assert_eq!(m.title, "Eviltxt.exe");
        assert_eq!(m.description.as_deref(), Some("stored at"));
        assert_eq!(names(&m.authors), ["Null Author"]);
    }
}
