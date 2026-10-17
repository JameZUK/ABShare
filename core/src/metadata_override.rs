//! Admin corrections to a book's details (SPEC §7.2 Admin, "metadata overrides"). The
//! library is read-only, so corrections are stored in the database and applied over what the
//! tags say, both when the catalogue loads and before series are grouped. Every field is
//! cleaned like tag text: an admin's edit is still rendered as text, never markup.

use crate::metadata::{BookMeta, Person, Position, SeriesRef, people};
use crate::text::{clean_line, clean_paragraphs};
use serde_json::{Map, Value};

const MAX_LINE: usize = 300;
const MAX_DESCRIPTION: usize = 20_000;
const MAX_LIST: usize = 20;

/// The fields an admin can correct. `None` leaves the tag value; for optional fields, an
/// empty value clears it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Override {
    pub title: Option<String>,
    pub subtitle: Option<Option<String>>,
    pub authors: Option<Vec<String>>,
    pub narrators: Option<Vec<String>>,
    /// `Some(None)` removes the series.
    pub series: Option<Option<(String, Option<String>)>>,
    pub genres: Option<Vec<String>>,
    pub description: Option<Option<String>>,
}

fn line(v: &Value, field: &str) -> Result<String, String> {
    v.as_str().map(|s| clean_line(s, MAX_LINE)).ok_or_else(|| format!("{field} must be text"))
}

fn list(v: &Value, field: &str) -> Result<Vec<String>, String> {
    let a = v.as_array().ok_or_else(|| format!("{field} must be a list"))?;
    if a.len() > MAX_LIST {
        return Err(format!("{field} has more than {MAX_LIST} entries"));
    }
    let mut out: Vec<String> = Vec::new();
    for x in a {
        let s = line(x, field)?;
        if !s.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(&s)) {
            out.push(s);
        }
    }
    Ok(out)
}

fn optional(v: &Value, field: &str, max: usize, paragraphs: bool) -> Result<Option<String>, String> {
    if v.is_null() {
        return Ok(None);
    }
    let s = v.as_str().ok_or_else(|| format!("{field} must be text"))?;
    let c = if paragraphs { clean_paragraphs(s, max) } else { clean_line(s, max) };
    Ok(Some(c).filter(|c| !c.is_empty()))
}

impl Override {
    /// Parses and cleans an override from the admin API (or the database). Unknown fields,
    /// wrong types and over-long lists are refused.
    pub fn parse(v: &Value) -> Result<Self, String> {
        let o = v.as_object().ok_or("an override is an object")?;
        let mut out = Override::default();
        for (k, v) in o {
            match k.as_str() {
                "title" => {
                    let t = line(v, "title")?;
                    if t.is_empty() {
                        return Err("the title can't be empty".into());
                    }
                    out.title = Some(t);
                }
                "subtitle" => out.subtitle = Some(optional(v, "subtitle", MAX_LINE, false)?),
                "authors" => out.authors = Some(list(v, "authors")?),
                "narrators" => out.narrators = Some(list(v, "narrators")?),
                "genres" => out.genres = Some(list(v, "genres")?),
                "description" => out.description = Some(optional(v, "description", MAX_DESCRIPTION, true)?),
                "series" => {
                    out.series = Some(if v.is_null() {
                        None
                    } else {
                        let s = v.as_object().ok_or("series must be an object or null")?;
                        if s.keys().any(|k| k != "name" && k != "position") {
                            return Err("series has only name and position".into());
                        }
                        let name = s.get("name").map(|n| line(n, "series name")).transpose()?.unwrap_or_default();
                        if name.is_empty() {
                            return Err("a series needs a name".into());
                        }
                        let position = match s.get("position") {
                            None | Some(Value::Null) => None,
                            Some(p) => {
                                let p = line(p, "series position")?;
                                if !p.is_empty() && Position::parse(&p).is_none() {
                                    return Err("the series position must be a number such as 2 or 2.5".into());
                                }
                                Some(p).filter(|p| !p.is_empty())
                            }
                        };
                        Some((name, position))
                    });
                }
                other => return Err(format!("unknown field {other}")),
            }
        }
        Ok(out)
    }

    pub fn is_empty(&self) -> bool {
        *self == Override::default()
    }

    /// The override as stored (only the fields set).
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        let names = |v: &Vec<String>| Value::from(v.clone());
        if let Some(t) = &self.title {
            m.insert("title".into(), t.clone().into());
        }
        if let Some(s) = &self.subtitle {
            m.insert("subtitle".into(), s.clone().map_or(Value::Null, Value::from));
        }
        if let Some(a) = &self.authors {
            m.insert("authors".into(), names(a));
        }
        if let Some(n) = &self.narrators {
            m.insert("narrators".into(), names(n));
        }
        if let Some(g) = &self.genres {
            m.insert("genres".into(), names(g));
        }
        if let Some(d) = &self.description {
            m.insert("description".into(), d.clone().map_or(Value::Null, Value::from));
        }
        if let Some(s) = &self.series {
            m.insert(
                "series".into(),
                s.as_ref().map_or(Value::Null, |(n, p)| serde_json::json!({ "name": n, "position": p })),
            );
        }
        Value::Object(m)
    }

    /// Applies the corrections to metadata built from tags.
    pub fn apply(&self, meta: &mut BookMeta) {
        let persons = |v: &Vec<String>| v.iter().flat_map(|n| people(n)).collect::<Vec<Person>>();
        if let Some(t) = &self.title {
            meta.title = t.clone();
        }
        if let Some(s) = &self.subtitle {
            meta.subtitle = s.clone();
        }
        if let Some(a) = &self.authors {
            meta.authors = persons(a);
        }
        if let Some(n) = &self.narrators {
            meta.narrators = persons(n);
        }
        if let Some(g) = &self.genres {
            meta.genres = g.clone();
        }
        if let Some(d) = &self.description {
            meta.description = d.clone();
        }
        if let Some(s) = &self.series {
            meta.series = s.as_ref().map(|(name, pos)| SeriesRef {
                name: name.clone(),
                qualifier: None,
                position: pos.as_deref().and_then(Position::parse),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn corrections_apply_over_tags_and_round_trip() {
        let o = Override::parse(&json!({
            "title": "  The Copper Tide ", "authors": ["Ann Author", "ann author", " "], "series": { "name": "Harbour", "position": "2" },
            "genres": ["Fantasy"], "subtitle": null, "description": "One.\n\nTwo.",
        }))
        .expect("parse");
        assert_eq!(o.title.as_deref(), Some("The Copper Tide"));
        assert_eq!(o.authors.as_deref(), Some(&["Ann Author".to_owned()][..]), "deduplicated, blanks dropped");
        let mut m = BookMeta { title: "Wrong".into(), subtitle: Some("old".into()), ..BookMeta::default() };
        o.apply(&mut m);
        assert_eq!(m.title, "The Copper Tide");
        assert_eq!(m.subtitle, None, "null clears");
        assert_eq!(m.authors.len(), 1);
        assert_eq!(m.series.as_ref().map(|s| s.name.as_str()), Some("Harbour"));
        assert_eq!(m.series.as_ref().and_then(|s| s.position).map(|p| p.display()), Some("2".into()));
        assert_eq!(Override::parse(&o.to_json()).expect("round trip"), o);
        // Removing a series.
        let mut m2 = m.clone();
        Override::parse(&json!({ "series": null })).expect("parse").apply(&mut m2);
        assert!(m2.series.is_none());
        assert!(Override::parse(&json!({})).expect("empty").is_empty());
    }

    #[test]
    fn bad_corrections_are_refused() {
        for bad in [
            json!("x"),
            json!({ "title": "" }),
            json!({ "title": 5 }),
            json!({ "authors": "Ann" }),
            json!({ "authors": vec!["a"; 21] }),
            json!({ "series": { "name": "" } }),
            json!({ "series": { "name": "S", "position": "second" } }),
            json!({ "series": { "name": "S", "colour": "red" } }),
            json!({ "path": "/etc/passwd" }),
        ] {
            assert!(Override::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn edits_are_cleaned_like_tags() {
        let o = Override::parse(&json!({ "title": "A\u{202E}B\u{0}C<script>", "description": "x".repeat(30_000) }))
            .expect("parse");
        let t = o.title.expect("title");
        assert!(!t.contains('\u{202E}') && !t.contains('\u{0}'), "bidi and control characters removed: {t:?}");
        assert!(o.description.flatten().expect("desc").chars().count() <= MAX_DESCRIPTION);
    }
}
