//! Genre shelves: the messy subgenre tags books carry ("Hard Science Fiction", "Space Opera",
//! "Biographies & Memoirs", "Murder"…) mapped onto a small, fixed set of shelves people
//! browse, each either fiction or non-fiction. A book can sit on several shelves. Tags
//! that match nothing stay searchable but put the book on no shelf.

use crate::text::fold;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Fiction,
    NonFiction,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Shelf {
    ScienceFiction,
    Fantasy,
    MysteryThriller,
    Horror,
    ActionAdventure,
    Literature,
    Comedy,
    Romance,
    ChildrenYoungAdult,
    Biography,
    History,
    Science,
    Politics,
    Philosophy,
    SelfHelp,
    Arts,
    Podcasts,
}

impl Shelf {
    pub const ALL: [Shelf; 17] = [
        Shelf::ScienceFiction,
        Shelf::Fantasy,
        Shelf::MysteryThriller,
        Shelf::Horror,
        Shelf::ActionAdventure,
        Shelf::Literature,
        Shelf::Comedy,
        Shelf::Romance,
        Shelf::ChildrenYoungAdult,
        Shelf::Biography,
        Shelf::History,
        Shelf::Science,
        Shelf::Politics,
        Shelf::Philosophy,
        Shelf::SelfHelp,
        Shelf::Arts,
        Shelf::Podcasts,
    ];

    /// The stable identifier used in the API (`?shelf=science-fiction`).
    pub const fn id(self) -> &'static str {
        match self {
            Shelf::ScienceFiction => "science-fiction",
            Shelf::Fantasy => "fantasy",
            Shelf::MysteryThriller => "mystery-thriller",
            Shelf::Horror => "horror",
            Shelf::ActionAdventure => "action-adventure",
            Shelf::Literature => "literature",
            Shelf::Comedy => "comedy",
            Shelf::Romance => "romance",
            Shelf::ChildrenYoungAdult => "children-young-adult",
            Shelf::Biography => "biography",
            Shelf::History => "history",
            Shelf::Science => "science",
            Shelf::Politics => "politics",
            Shelf::Philosophy => "philosophy",
            Shelf::SelfHelp => "self-help",
            Shelf::Arts => "arts",
            Shelf::Podcasts => "podcasts",
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Shelf::ScienceFiction => "Science Fiction",
            Shelf::Fantasy => "Fantasy",
            Shelf::MysteryThriller => "Mystery & Thriller",
            Shelf::Horror => "Horror",
            Shelf::ActionAdventure => "Action & Adventure",
            Shelf::Literature => "Literature & Classics",
            Shelf::Comedy => "Comedy",
            Shelf::Romance => "Romance",
            Shelf::ChildrenYoungAdult => "Children & Young Adult",
            Shelf::Biography => "Biography & Memoir",
            Shelf::History => "History",
            Shelf::Science => "Science & Technology",
            Shelf::Politics => "Politics & Society",
            Shelf::Philosophy => "Philosophy & Religion",
            Shelf::SelfHelp => "Business, Money & Self-help",
            Shelf::Arts => "Arts, Entertainment & Travel",
            Shelf::Podcasts => "Podcasts",
        }
    }

    pub const fn kind(self) -> Kind {
        match self {
            Shelf::ScienceFiction
            | Shelf::Fantasy
            | Shelf::MysteryThriller
            | Shelf::Horror
            | Shelf::ActionAdventure
            | Shelf::Literature
            | Shelf::Comedy
            | Shelf::Romance
            | Shelf::ChildrenYoungAdult => Kind::Fiction,
            Shelf::Biography
            | Shelf::History
            | Shelf::Science
            | Shelf::Politics
            | Shelf::Philosophy
            | Shelf::SelfHelp
            | Shelf::Arts => Kind::NonFiction,
            Shelf::Podcasts => Kind::Other,
        }
    }

    pub fn from_id(id: &str) -> Option<Shelf> {
        Shelf::ALL.into_iter().find(|s| s.id() == id)
    }
}

use Shelf::*;

/// Whole tags (folded) and the shelves they mean. Checked before the keywords below.
const EXACT: &[(&str, &[Shelf])] = &[
    ("science fiction & fantasy", &[ScienceFiction, Fantasy]),
    ("sci fi & fantasy", &[ScienceFiction, Fantasy]),
    ("epic", &[Fantasy]),
    ("dragons & mythical creatures", &[Fantasy]),
    ("sword & sorcery", &[Fantasy]),
    ("murder", &[MysteryThriller]),
    ("espionage", &[MysteryThriller]),
    ("suspense", &[MysteryThriller]),
    ("psychological", &[MysteryThriller]),
    ("supernatural", &[Horror]),
    ("ghosts", &[Horror]),
    ("adventure", &[ActionAdventure]),
    ("action & adventure", &[ActionAdventure]),
    ("literature & fiction", &[Literature]),
    ("literary fiction", &[Literature]),
    ("classics", &[Literature]),
    ("historical fiction", &[Literature]),
    ("family life", &[Literature]),
    ("anthologies & short stories", &[Literature]),
    ("humorous", &[Comedy]),
    ("humour", &[Comedy]),
    ("humor", &[Comedy]),
    ("biographies & memoirs", &[Biography]),
    ("presidents & heads of state", &[Biography, Politics]),
    ("war & crisis", &[History]),
    ("world", &[History]),
    ("freedom & security", &[Politics]),
    ("political", &[Politics]),
    ("metaphysical & visionary", &[Philosophy]),
    ("nature & ecology", &[Science]),
    ("authors", &[Biography]),
    ("podcast", &[Podcasts]),
    ("podcasts", &[Podcasts]),
];

/// Words that place a tag on a shelf wherever they appear in it (folded), in order.
const KEYWORDS: &[(&str, Shelf)] = &[
    ("science fiction", ScienceFiction),
    ("sci fi", ScienceFiction),
    ("scifi", ScienceFiction),
    ("space opera", ScienceFiction),
    ("space exploration", ScienceFiction),
    ("first contact", ScienceFiction),
    ("genetic engineering", ScienceFiction),
    ("time travel", ScienceFiction),
    ("post apocalyptic", ScienceFiction),
    ("dystopia", ScienceFiction),
    ("cyberpunk", ScienceFiction),
    ("alien", ScienceFiction),
    ("superhero", ScienceFiction),
    ("steampunk", ScienceFiction),
    ("litrpg", Fantasy),
    ("fantasy", Fantasy),
    ("mystery", MysteryThriller),
    ("thriller", MysteryThriller),
    ("crime", MysteryThriller),
    ("detective", MysteryThriller),
    ("horror", Horror),
    ("comedy", Comedy),
    ("satire", Comedy),
    ("stand up", Comedy),
    ("romance", Romance),
    ("romantic", Romance),
    ("children", ChildrenYoungAdult),
    ("young adult", ChildrenYoungAdult),
    ("teen", ChildrenYoungAdult),
    ("biograph", Biography),
    ("memoir", Biography),
    ("autobiograph", Biography),
    ("history", History),
    ("historical", History),
    ("science", Science),
    ("physics", Science),
    ("astronomy", Science),
    ("cosmology", Science),
    ("biolog", Science),
    ("mathemat", Science),
    ("technology", Science),
    ("computer", Science),
    ("encryption", Science),
    ("engineering", Science),
    ("nature", Science),
    ("politic", Politics),
    ("economics", Politics),
    ("sociology", Politics),
    ("current affairs", Politics),
    ("corruption", Politics),
    ("philosoph", Philosophy),
    ("religio", Philosophy),
    ("spiritual", Philosophy),
    ("self help", SelfHelp),
    ("personal development", SelfHelp),
    ("business", SelfHelp),
    ("career", SelfHelp),
    ("health", SelfHelp),
    ("psychology", SelfHelp),
    ("finance", SelfHelp),
    ("money", SelfHelp),
    ("invest", SelfHelp),
    ("taxation", SelfHelp),
    ("insurance", SelfHelp),
    ("entertainment", Arts),
    ("celebrit", Arts),
    ("music", Arts),
    ("film", Arts),
    ("art", Arts),
    ("travel", Arts),
    ("food", Arts),
    ("sport", Arts),
];

/// The shelves one tag means.
fn tag_shelves(tag: &str) -> Vec<Shelf> {
    let f = fold(&tag.replace('-', " "));
    if let Some((_, s)) = EXACT.iter().find(|(t, _)| *t == f) {
        return s.to_vec();
    }
    let mut out = Vec::new();
    let words: Vec<&str> = f.split(' ').collect();
    for (word, shelf) in KEYWORDS {
        // Short keys match whole words only ("art" is not in "earth"); longer ones are stems.
        let hit = if word.len() <= 4 { words.contains(word) } else { f.contains(word) };
        if hit && !out.contains(shelf) {
            out.push(*shelf);
        }
    }
    // "Historical fiction" and similar are fiction, not history: the keyword pass above would
    // say History, so fiction wins when the tag says so.
    if f.contains("fiction") && out.contains(&History) {
        out.retain(|s| *s != History);
        if !out.iter().any(|s| s.kind() == Kind::Fiction) {
            out.push(Literature);
        }
    }
    // "Science fiction" is not science, and "genetic engineering" there is not technology.
    if out.contains(&ScienceFiction) {
        out.retain(|s| *s != Science);
    }
    // Any other kind of fiction ("Contemporary Fiction") belongs with literature.
    if out.is_empty() && f.split(' ').any(|w| w == "fiction") {
        out.push(Literature);
    }
    out
}

/// The shelves a book's tags put it on, in [`Shelf::ALL`] order.
///
/// "Military" alone is ambiguous (military science fiction, or military history): it counts
/// as Action & Adventure only when nothing else places the book.
pub fn shelves(tags: &[String]) -> Vec<Shelf> {
    let mut found: Vec<Shelf> = Vec::new();
    for t in tags {
        for s in tag_shelves(t) {
            if !found.contains(&s) {
                found.push(s);
            }
        }
    }
    if found.is_empty() && tags.iter().any(|t| fold(t) == "military") {
        found.push(ActionAdventure);
    }
    // Adventure next to science fiction or fantasy is a flavour of those, not its own shelf.
    if found.contains(&ActionAdventure) && (found.contains(&ScienceFiction) || found.contains(&Fantasy)) {
        found.retain(|s| *s != ActionAdventure);
    }
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(s: &str) -> Vec<String> {
        s.split(", ").map(str::to_owned).collect()
    }

    #[test]
    fn subgenre_lists_land_on_tidy_shelves() {
        assert_eq!(shelves(&tags("Hard Science Fiction, Adventure, Space Opera")), [ScienceFiction]);
        assert_eq!(shelves(&tags("Fantasy, Hard Science Fiction, Adventure")), [ScienceFiction, Fantasy]);
        assert_eq!(shelves(&tags("Science Fiction & Fantasy")), [ScienceFiction, Fantasy]);
        assert_eq!(shelves(&tags("Sci-Fi")), [ScienceFiction]);
        assert_eq!(shelves(&tags("Epic, Dark Fantasy, Dragons & Mythical Creatures")), [Fantasy]);
        assert_eq!(shelves(&tags("Murder")), [MysteryThriller]);
        assert_eq!(shelves(&tags("Suspense, Classics, Espionage")), [MysteryThriller, Literature]);
        assert_eq!(shelves(&tags("Literature & Fiction")), [Literature]);
        assert_eq!(shelves(&tags("Historical Fiction")), [Literature]);
        assert_eq!(shelves(&tags("Podcast")), [Podcasts]);
        assert_eq!(shelves(&tags("Adventure")), [ActionAdventure]);
    }

    #[test]
    fn factual_books_are_non_fiction() {
        assert_eq!(shelves(&tags("Biographies & Memoirs")), [Biography]);
        assert_eq!(shelves(&tags("Presidents & Heads of State")), [Biography, Politics]);
        assert_eq!(shelves(&tags("Earth Sciences, Physics, Astronomy, Cosmology, Biological Sciences")), [Science]);
        assert_eq!(shelves(&tags("Freedom & Security, War & Crisis")), [History, Politics]);
        assert_eq!(
            shelves(&tags("World, Science, Earth Sciences, History, Philosophy, Nature & Ecology, Physics")),
            [History, Science, Philosophy]
        );
        for s in [Biography, History, Science, Politics, Philosophy, SelfHelp, Arts] {
            assert_eq!(s.kind(), Kind::NonFiction, "{s:?}");
        }
        assert_eq!(ScienceFiction.kind(), Kind::Fiction);
    }

    #[test]
    fn ambiguous_and_unknown_tags() {
        assert_eq!(shelves(&tags("Military")), [ActionAdventure]);
        assert_eq!(shelves(&tags("Military, Space Opera, Humorous")), [ScienceFiction, Comedy]);
        assert!(shelves(&tags("Movie, TV & Video Game Tie-Ins")).is_empty());
        assert_eq!(shelves(&tags("Contemporary Fiction")), [Literature]);
        assert_eq!(shelves(&tags("Steampunk")), [ScienceFiction]);
        assert_eq!(shelves(&tags("Art, Film & TV, Stand-up Shows")), [Comedy, Arts]);
        assert_eq!(shelves(&tags("Europe, Travel Writing & Commentary")), [Arts]);
        assert_eq!(
            shelves(&tags("Corruption & Misconduct, Corporate & Public Finance, Taxation")),
            [Politics, SelfHelp]
        );
        assert_eq!(shelves(&tags("Security & Encryption, Insurance")), [Science, SelfHelp]);
        assert_eq!(shelves(&tags("Authors")), [Biography]);
        // Short keys are whole words: "Earth Sciences" is science, not art; "Martial" isn't art.
        assert_eq!(shelves(&tags("Earth Sciences")), [Science]);
        assert!(!shelves(&tags("Martial Arts Fiction")).contains(&Science));
        assert!(shelves(&[]).is_empty());
        assert!(shelves(&[String::new()]).is_empty());
        // Science fiction is never filed under science.
        assert!(!shelves(&tags("Hard Science Fiction")).contains(&Science));
    }

    #[test]
    fn ids_round_trip_and_are_url_safe() {
        for s in Shelf::ALL {
            assert_eq!(Shelf::from_id(s.id()), Some(s));
            assert!(s.id().bytes().all(|c| c.is_ascii_lowercase() || c == b'-'));
            assert!(!s.name().is_empty());
        }
        assert_eq!(Shelf::from_id("nope"), None);
        assert_eq!(Shelf::from_id(""), None);
    }

    #[test]
    fn never_panics_on_odd_input() {
        let odd = ["\u{0}", "ＳＣＩ－ＦＩ", "fiction history fiction", &"x".repeat(10_000), "😀", "-", "&&&"];
        for t in odd {
            let _ = shelves(&[t.to_owned()]);
        }
    }
}
