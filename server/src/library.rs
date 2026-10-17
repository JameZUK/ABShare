//! The catalogue as the API sees it: an in-memory view of the books, reloaded when the
//! database changes (scans run as a separate process), with folded search keys.

use crate::db::sql_err;
use abshare_core::text::fold;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct Book {
    pub id: String,
    pub library_id: i64,
    pub title: String,
    pub authors: Vec<String>,
    pub narrators: Vec<String>,
    pub series: Option<(String, Option<String>, u32)>,
    pub genres: Vec<String>,
    pub duration_ms: u64,
    pub cover: Option<String>,
    pub added_at: i64,
    pub needs_copy: bool,
    pub meta: Value,
    /// Folded title, people, series and genres, for search.
    pub search: String,
    pub sort_title: String,
}

#[derive(Clone, Debug)]
pub struct Group {
    pub id: String,
    pub library_id: i64,
    pub name: String,
    pub qualifier: Option<String>,
    pub kind: String,
    /// (book id, extra)
    pub members: Vec<(String, bool)>,
}

#[derive(Default)]
pub struct Catalog {
    pub books: Vec<Book>,
    pub by_id: HashMap<String, usize>,
    pub groups: Vec<Group>,
    pub libraries: Vec<(i64, String)>,
    pub stamp: (i64, i64, i64),
}

fn names(v: &Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .map(|a| a.iter().filter_map(|p| p["name"].as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

/// "The Copper Tide" sorts as "copper tide".
fn sort_key(title: &str) -> String {
    let f = fold(title);
    for article in ["the ", "a ", "an "] {
        if let Some(rest) = f.strip_prefix(article) {
            return rest.to_owned();
        }
    }
    f
}

/// A cheap fingerprint of the catalogue: when it changes, the view is reloaded.
pub fn stamp(c: &Connection) -> Result<(i64, i64, i64), String> {
    c.query_row(
        "SELECT count(*), coalesce(max(updated_at), 0), (SELECT count(*) FROM grp) FROM book WHERE missing_since IS NULL",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .map_err(sql_err)
}

pub fn load(c: &Connection) -> Result<Catalog, String> {
    let mut cat = Catalog { stamp: stamp(c)?, ..Catalog::default() };
    {
        let mut st = c.prepare("SELECT id, name FROM library ORDER BY id").map_err(sql_err)?;
        cat.libraries = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(sql_err)?
            .collect::<Result<_, _>>()
            .map_err(sql_err)?;
    }
    let mut st = c
        .prepare(
            "SELECT id, library_id, meta, duration_ms, needs_copy, cover, coalesce(added_at, updated_at) FROM book
             WHERE missing_since IS NULL",
        )
        .map_err(sql_err)?;
    let rows = st
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, bool>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })
        .map_err(sql_err)?;
    for row in rows {
        let (id, library_id, meta, duration, needs_copy, cover, added_at) = row.map_err(sql_err)?;
        let meta: Value = serde_json::from_str(&meta).unwrap_or(Value::Null);
        let title = meta["title"].as_str().unwrap_or("Untitled").to_owned();
        let authors = names(&meta, "authors");
        let narrators = names(&meta, "narrators");
        let series = meta["series"]["name"].as_str().map(|n| {
            (
                n.to_owned(),
                meta["series"]["position"].as_str().map(str::to_owned),
                meta["series"]["sort"].as_u64().map_or(u32::MAX, |v| v as u32),
            )
        });
        let genres: Vec<String> = meta["genres"]
            .as_array()
            .map(|a| a.iter().filter_map(|g| g.as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        let search = fold(&format!(
            "{title} {} {} {} {} {}",
            meta["subtitle"].as_str().unwrap_or(""),
            authors.join(" "),
            narrators.join(" "),
            series.as_ref().map_or("", |s| s.0.as_str()),
            genres.join(" ")
        ));
        cat.by_id.insert(id.clone(), cat.books.len());
        cat.books.push(Book {
            sort_title: sort_key(&title),
            id,
            library_id,
            title,
            authors,
            narrators,
            series,
            genres,
            duration_ms: duration.max(0) as u64,
            cover,
            added_at,
            needs_copy,
            meta,
            search,
        });
    }
    let mut st = c.prepare("SELECT id, library_id, name, qualifier, kind FROM grp").map_err(sql_err)?;
    let groups: Vec<Group> = st
        .query_map([], |r| {
            Ok(Group {
                id: r.get(0)?,
                library_id: r.get(1)?,
                name: r.get(2)?,
                qualifier: r.get(3)?,
                kind: r.get(4)?,
                members: vec![],
            })
        })
        .map_err(sql_err)?
        .collect::<Result<_, _>>()
        .map_err(sql_err)?;
    let mut mst = c.prepare("SELECT book_id, extra FROM grp_member WHERE grp_id = ?1 ORDER BY ord").map_err(sql_err)?;
    for mut g in groups {
        g.members = mst
            .query_map([&g.id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)))
            .map_err(sql_err)?
            .collect::<Result<_, _>>()
            .map_err(sql_err)?;
        cat.groups.push(g);
    }
    Ok(cat)
}

/// The library IDs a user may see, from their permission string (`*` or a JSON array of
/// library names). Unknown names grant nothing.
pub fn allowed(cat: &Catalog, libraries: &str) -> HashSet<i64> {
    if libraries.trim() == "*" {
        return cat.libraries.iter().map(|(id, _)| *id).collect();
    }
    let names: Vec<String> = serde_json::from_str(libraries).unwrap_or_default();
    cat.libraries.iter().filter(|(_, n)| names.iter().any(|w| w.eq_ignore_ascii_case(n))).map(|(id, _)| *id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn articles_are_ignored_when_sorting() {
        assert_eq!(sort_key("The Copper Tide"), "copper tide");
        assert_eq!(sort_key("A Tale"), "tale");
        assert_eq!(sort_key("Theory"), "theory");
    }

    #[test]
    fn library_permissions() {
        let cat = Catalog { libraries: vec![(1, "Audiobooks".into()), (2, "Kids".into())], ..Catalog::default() };
        assert_eq!(allowed(&cat, "*").len(), 2);
        assert_eq!(allowed(&cat, r#"["kids"]"#), HashSet::from([2]));
        assert!(allowed(&cat, r#"["Nope"]"#).is_empty());
        assert!(allowed(&cat, "garbage").is_empty(), "a malformed permission grants nothing");
    }
}
