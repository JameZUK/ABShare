//! `/v1/libraries`, `/v1/books`, `/v1/groups`, `/v1/home`, `/v1/progress`, bookmarks
//! (SPEC §5.6, §6). Every lookup is filtered by the caller's library access, and a book
//! outside it is "not found" (never "forbidden"), so existence isn't revealed.

use super::{ApiError, Authed, State, now};
use crate::auth::store;
use crate::library::{Book, Catalog, allowed};
use abshare_core::genre::{Kind, Shelf};
use abshare_core::ids::is_valid_id;
use abshare_core::progress::{self, Action, Event, Outcome, Progress};
use abshare_core::text::{clean_paragraphs, fold};
use axum::extract::{Path, Query, State as S};
use axum::http::StatusCode;
use axum::routing::{get, patch, put};
use axum::{Json, Router};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub fn routes() -> Router<State> {
    Router::new()
        .route("/libraries", get(libraries))
        .route("/books", get(books))
        .route("/books/{id}", get(book))
        .route("/books/{id}/bookmarks", get(list_bookmarks).post(add_bookmark))
        .route("/bookmarks/{id}", patch(edit_bookmark).delete(delete_bookmark))
        .route("/genres", get(genres))
        .route("/groups", get(groups))
        .route("/groups/{id}", get(group))
        .route("/home", get(home))
        .route("/progress", get(progress_since))
        .route("/progress/{id}", put(put_progress))
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024))
}

/// The caller's view: the catalogue and the libraries they may see.
async fn view(st: &State, s: &crate::auth::sessions::Session) -> Result<(Arc<Catalog>, HashSet<i64>), ApiError> {
    let cat = st.catalog().await?;
    let uid = s.user;
    let perms = st
        .db
        .run(move |c| Ok(store::user_by_id(c, uid)?.map(|u| u.perms.libraries)))
        .await?
        .ok_or_else(ApiError::unauthenticated)?;
    let ok = allowed(&cat, &perms);
    Ok((cat, ok))
}

fn find<'a>(cat: &'a Catalog, ok: &HashSet<i64>, id: &str) -> Result<&'a Book, ApiError> {
    if !is_valid_id(id) {
        return Err(ApiError::not_found());
    }
    cat.by_id.get(id).map(|&i| &cat.books[i]).filter(|b| ok.contains(&b.library_id)).ok_or_else(ApiError::not_found)
}

fn cover_url(b: &Book) -> Value {
    b.cover.as_ref().map_or(Value::Null, |h| json!(format!("media/covers/{}/{h}", b.id)))
}

/// Book → (position ms, finished, event time ms).
type ProgressMap = HashMap<String, (u64, bool, i64)>;

async fn user_progress(st: &State, uid: i64) -> Result<ProgressMap, ApiError> {
    st.db
        .run(move |c| {
            let mut stmt = c
                .prepare("SELECT book_id, position_ms, finished, updated_at FROM progress WHERE user_id = ?1")
                .map_err(crate::db::sql_err)?;
            let rows = stmt
                .query_map([uid], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        (r.get::<_, i64>(1)?.max(0) as u64, r.get::<_, bool>(2)?, r.get::<_, i64>(3)?),
                    ))
                })
                .map_err(crate::db::sql_err)?;
            rows.collect::<Result<_, _>>().map_err(crate::db::sql_err)
        })
        .await
}

fn summary(b: &Book, p: Option<&(u64, bool, i64)>) -> Value {
    json!({
        "type": "book",
        "id": b.id,
        "title": b.title,
        "subtitle": b.meta["subtitle"],
        "authors": b.authors,
        "narrators": b.narrators,
        "series": b.series.as_ref().map(|(n, pos, _)| json!({ "name": n, "position": pos })),
        "duration_ms": b.duration_ms,
        "cover": cover_url(b),
        "added_at": b.added_at,
        "progress": p.map(|(pos, fin, at)| json!({ "position_ms": pos, "finished": fin, "updated_at": at })),
    })
}

async fn libraries(S(st): S<State>, Authed(s): Authed) -> Result<Json<Value>, ApiError> {
    let (cat, ok) = view(&st, &s).await?;
    Ok(Json(
        json!({ "libraries": cat.libraries.iter().filter(|(id, _)| ok.contains(id)).map(|(id, n)| json!({ "id": id, "name": n })).collect::<Vec<_>>() }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BooksQuery {
    library: Option<i64>,
    q: Option<String>,
    author: Option<String>,
    narrator: Option<String>,
    genre: Option<String>,
    /// A genre shelf id (`science-fiction`, `biography`…), or `fiction` / `non-fiction`.
    shelf: Option<String>,
    status: Option<String>,
    sort: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
}

async fn books(S(st): S<State>, Authed(s): Authed, Query(q): Query<BooksQuery>) -> Result<Json<Value>, ApiError> {
    let (cat, ok) = view(&st, &s).await?;
    let prog = user_progress(&st, s.user).await?;
    let limit = q.limit.unwrap_or(50).clamp(1, 100);
    let offset: usize =
        q.cursor.as_deref().map(|c| c.parse().map_err(|_| ApiError::bad("Bad cursor."))).transpose()?.unwrap_or(0);
    let shelf = match q.shelf.as_deref() {
        None => None,
        Some("fiction") => Some(ShelfFilter::Kind(Kind::Fiction)),
        Some("non-fiction") => Some(ShelfFilter::Kind(Kind::NonFiction)),
        Some(id) => Some(ShelfFilter::One(Shelf::from_id(id).ok_or_else(|| ApiError::bad("Unknown genre."))?)),
    };
    // Bounded work per request (review H2): at most 200 characters and 8 distinct words.
    if q.q.as_deref().is_some_and(|s| s.chars().count() > 200) {
        return Err(ApiError::bad("Searches are at most 200 characters."));
    }
    let needle = q.q.as_deref().map(fold).filter(|s| !s.is_empty());
    let words: Vec<String> = needle.as_deref().map_or_else(Vec::new, |n| {
        let mut w: Vec<String> = Vec::new();
        for x in n.split(' ').filter(|x| !x.is_empty()) {
            if !w.iter().any(|y| y == x) {
                w.push(x.to_owned());
            }
        }
        w
    });
    if words.len() > 8 {
        return Err(ApiError::bad("Searches are at most 8 words."));
    }
    let (fa, fnar, fg) = (q.author.as_deref().map(fold), q.narrator.as_deref().map(fold), q.genre.as_deref().map(fold));
    // Episodes of shows are collapsed into one tile per show (unless searching).
    let shows: Vec<&crate::library::Group> =
        cat.groups.iter().filter(|g| g.kind == "show" && ok.contains(&g.library_id)).collect();
    let in_show: HashSet<&str> = if needle.is_some() {
        HashSet::new()
    } else {
        shows.iter().flat_map(|g| g.members.iter().map(|(b, _)| b.as_str())).collect()
    };
    let status_ok = |b: &Book| match q.status.as_deref() {
        Some("not_started") => !prog.contains_key(&b.id),
        Some("in_progress") => prog.get(&b.id).is_some_and(|p| !p.1),
        Some("finished") => prog.get(&b.id).is_some_and(|p| p.1),
        _ => true,
    };
    let mut items: Vec<&Book> = cat
        .books
        .iter()
        .filter(|b| ok.contains(&b.library_id) && q.library.is_none_or(|l| l == b.library_id))
        .filter(|b| !in_show.contains(b.id.as_str()))
        .filter(|b| words.iter().all(|w| b.search.contains(w.as_str())))
        .filter(|b| fa.as_ref().is_none_or(|a| b.authors.iter().any(|x| fold(x) == *a)))
        .filter(|b| fnar.as_ref().is_none_or(|a| b.narrators.iter().any(|x| fold(x) == *a)))
        .filter(|b| fg.as_ref().is_none_or(|g| b.genres.iter().any(|x| fold(x) == *g)))
        .filter(|b| status_ok(b))
        .filter(|b| match shelf {
            None => true,
            Some(ShelfFilter::One(x)) => b.shelves.contains(&x),
            Some(ShelfFilter::Kind(k)) => b.shelves.iter().any(|x| x.kind() == k),
        })
        .collect();
    match q.sort.as_deref().unwrap_or("title") {
        "added" => items.sort_by_key(|b| std::cmp::Reverse(b.added_at)),
        "recent" => items.sort_by_key(|b| std::cmp::Reverse(prog.get(&b.id).map_or(0, |p| p.2))),
        "author" => items.sort_by(|a, b| {
            a.authors
                .first()
                .map(|s| fold(s))
                .cmp(&b.authors.first().map(|s| fold(s)))
                .then_with(|| a.sort_title.cmp(&b.sort_title))
        }),
        "title" => items.sort_by(|a, b| a.sort_title.cmp(&b.sort_title)),
        _ => return Err(ApiError::bad("Unknown sort.")),
    }
    let mut out: Vec<Value> = items.iter().map(|b| summary(b, prog.get(&b.id))).collect();
    if needle.is_none()
        && q.library.is_none()
        && fa.is_none()
        && fnar.is_none()
        && fg.is_none()
        && q.status.is_none()
        && shelf.is_none()
    {
        for g in &shows {
            let first = g.members.first().and_then(|(id, _)| cat.by_id.get(id)).map(|&i| &cat.books[i]);
            out.push(json!({ "type": "show", "id": g.id, "title": g.name, "episodes": g.members.iter().filter(|m| !m.1).count(), "cover": first.map_or(Value::Null, cover_url) }));
        }
        if q.sort.as_deref().is_none_or(|s| s == "title") {
            out.sort_by_key(|v| fold(v["title"].as_str().unwrap_or("")).trim_start_matches("the ").to_owned());
        }
    }
    let total = out.len();
    let page: Vec<Value> = out.into_iter().skip(offset).take(limit).collect();
    let next = (offset + page.len() < total).then(|| (offset + page.len()).to_string());
    Ok(Json(json!({ "items": page, "total": total, "next": next })))
}

async fn book(S(st): S<State>, Authed(s): Authed, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let (cat, ok) = view(&st, &s).await?;
    let b = find(&cat, &ok, &id)?.clone();
    let (uid, bid) = (s.user, b.id.clone());
    let (chapters, prog, marks) = st
        .db
        .run(move |c| {
            let mut ch = c.prepare("SELECT start_ms, end_ms, title FROM chapter WHERE book_id = ?1 ORDER BY ord").map_err(crate::db::sql_err)?;
            let chapters: Vec<Value> = ch
                .query_map([&bid], |r| Ok(json!({ "start_ms": r.get::<_, i64>(0)?, "end_ms": r.get::<_, i64>(1)?, "title": r.get::<_, String>(2)? })))
                .map_err(crate::db::sql_err)?
                .collect::<Result<_, _>>()
                .map_err(crate::db::sql_err)?;
            let prog: Option<Value> = c
                .query_row(
                    "SELECT position_ms, finished, updated_at FROM progress WHERE user_id = ?1 AND book_id = ?2",
                    params![uid, bid],
                    |r| Ok(json!({ "position_ms": r.get::<_, i64>(0)?, "finished": r.get::<_, bool>(1)?, "updated_at": r.get::<_, i64>(2)? })),
                )
                .optional()
                .map_err(crate::db::sql_err)?;
            let marks = bookmarks_of(c, uid, &bid)?;
            Ok((chapters, prog, marks))
        })
        .await?;
    let groups: Vec<Value> = cat
        .groups
        .iter()
        .filter(|g| ok.contains(&g.library_id) && g.members.iter().any(|(m, _)| *m == b.id))
        .map(|g| json!({ "id": g.id, "name": g.name, "kind": g.kind }))
        .collect();
    Ok(Json(json!({
        "id": b.id, "title": b.title, "subtitle": b.meta["subtitle"], "authors": b.meta["authors"], "narrators": b.meta["narrators"],
        "series": b.meta["series"], "description": b.meta["description"], "genres": b.genres,
        "shelves": b.shelves.iter().map(|sh| json!({ "id": sh.id(), "name": sh.name() })).collect::<Vec<_>>(), "year": b.meta["year"],
        "publisher": b.meta["publisher"], "language": b.meta["language"], "duration_ms": b.duration_ms, "cover": cover_url(&b),
        "added_at": b.added_at, "chapters": chapters, "progress": prog, "bookmarks": marks, "groups": groups,
        "media": { "original": format!("media/{}", b.id), "preparing": b.needs_copy },
    })))
}

#[derive(Clone, Copy)]
enum ShelfFilter {
    One(Shelf),
    Kind(Kind),
}

/// `GET /v1/genres`: the genre shelves with how many of the caller's books are on each.
async fn genres(S(st): S<State>, Authed(s): Authed) -> Result<Json<Value>, ApiError> {
    let (cat, ok) = view(&st, &s).await?;
    let mut counts: HashMap<Shelf, usize> = HashMap::new();
    let mut unshelved = 0;
    for b in cat.books.iter().filter(|b| ok.contains(&b.library_id)) {
        if b.shelves.is_empty() {
            unshelved += 1;
        }
        for sh in &b.shelves {
            *counts.entry(*sh).or_default() += 1;
        }
    }
    let kind = |k: Kind| match k {
        Kind::Fiction => "fiction",
        Kind::NonFiction => "non-fiction",
        Kind::Other => "other",
    };
    let list: Vec<Value> = Shelf::ALL
        .iter()
        .filter_map(|sh| counts.get(sh).map(|n| (sh, n)))
        .map(|(sh, n)| json!({ "id": sh.id(), "name": sh.name(), "kind": kind(sh.kind()), "count": n }))
        .collect();
    Ok(Json(json!({ "genres": list, "unshelved": unshelved })))
}

async fn groups(S(st): S<State>, Authed(s): Authed) -> Result<Json<Value>, ApiError> {
    let (cat, ok) = view(&st, &s).await?;
    let mut v: Vec<Value> = cat
        .groups
        .iter()
        .filter(|g| ok.contains(&g.library_id))
        .map(|g| json!({ "id": g.id, "name": g.name, "qualifier": g.qualifier, "kind": g.kind, "items": g.members.len() }))
        .collect();
    v.sort_by_key(|g| fold(g["name"].as_str().unwrap_or("")));
    Ok(Json(json!({ "groups": v })))
}

async fn group(S(st): S<State>, Authed(s): Authed, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let (cat, ok) = view(&st, &s).await?;
    if !is_valid_id(&id) {
        return Err(ApiError::not_found());
    }
    let g = cat.groups.iter().find(|g| g.id == id && ok.contains(&g.library_id)).ok_or_else(ApiError::not_found)?;
    let prog = user_progress(&st, s.user).await?;
    let items: Vec<Value> = g
        .members
        .iter()
        .filter_map(|(bid, extra)| cat.by_id.get(bid).map(|&i| (&cat.books[i], *extra)))
        .map(|(b, extra)| {
            let mut v = summary(b, prog.get(&b.id));
            v["extra"] = json!(extra);
            v
        })
        .collect();
    let next = items
        .iter()
        .find(|v| !v["extra"].as_bool().unwrap_or(false) && !v["progress"]["finished"].as_bool().unwrap_or(false))
        .map(|v| v["id"].clone());
    Ok(Json(
        json!({ "id": g.id, "name": g.name, "qualifier": g.qualifier, "kind": g.kind, "items": items, "next": next }),
    ))
}

async fn home(S(st): S<State>, Authed(s): Authed) -> Result<Json<Value>, ApiError> {
    let (cat, ok) = view(&st, &s).await?;
    let prog = user_progress(&st, s.user).await?;
    let visible = |id: &String| cat.by_id.get(id).map(|&i| &cat.books[i]).filter(|b| ok.contains(&b.library_id));
    let mut cont: Vec<(&Book, &(u64, bool, i64))> =
        prog.iter().filter(|(_, p)| !p.1).filter_map(|(id, p)| visible(id).map(|b| (b, p))).collect();
    cont.sort_by_key(|(_, p)| std::cmp::Reverse(p.2));
    let mut recent: Vec<&Book> = cat.books.iter().filter(|b| ok.contains(&b.library_id)).collect();
    recent.sort_by_key(|b| std::cmp::Reverse(b.added_at));
    // Series in progress: the next unfinished book after one you finished.
    let mut next_up = Vec::new();
    for g in cat.groups.iter().filter(|g| g.kind == "series" && ok.contains(&g.library_id)) {
        let any_finished = g.members.iter().any(|(id, _)| prog.get(id).is_some_and(|p| p.1));
        if !any_finished {
            continue;
        }
        if let Some(b) = g
            .members
            .iter()
            .filter(|(id, _)| !prog.get(id).is_some_and(|p| p.1) && !prog.get(id).is_some_and(|p| !p.1))
            .find_map(|(id, _)| visible(id))
        {
            next_up.push(json!({ "series": g.name, "book": summary(b, None) }));
        }
    }
    Ok(Json(json!({
        "continue": cont.iter().take(12).map(|(b, p)| summary(b, Some(p))).collect::<Vec<_>>(),
        "series_next": next_up.into_iter().take(12).collect::<Vec<_>>(),
        "recently_added": recent.iter().take(12).map(|b| summary(b, prog.get(&b.id))).collect::<Vec<_>>(),
    })))
}

// ---------------------------------------------------------------- progress

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProgressBody {
    key: String,
    event_time_ms: i64,
    action: String,
    position_ms: Option<u64>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

async fn put_progress(
    S(st): S<State>,
    Authed(s): Authed,
    Path(id): Path<String>,
    Json(b): Json<ProgressBody>,
) -> Result<Json<Value>, ApiError> {
    let (cat, ok) = view(&st, &s).await?;
    let book = find(&cat, &ok, &id)?;
    if !(8..=64).contains(&b.key.len()) || !b.key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_') {
        return Err(ApiError::bad("Bad idempotency key."));
    }
    if !st.progress_limit.allow((s.device, book.id.clone()), now()) {
        return Err(ApiError::too_many());
    }
    let action = match b.action.as_str() {
        "position" => Action::Position(b.position_ms.ok_or_else(|| ApiError::bad("position_ms required."))?),
        "finished" => Action::MarkFinished,
        "unfinished" => Action::MarkUnfinished,
        "restart" => Action::Restart,
        _ => return Err(ApiError::bad("Unknown action.")),
    };
    let (uid, device, bid, duration) = (s.user, s.device, book.id.clone(), book.duration_ms);
    let key = b.key.clone();
    let ev = Event { action, event_time: b.event_time_ms };
    let outcome = st
        .db
        .run(move |c| {
            let tx = c.transaction().map_err(crate::db::sql_err)?;
            let seen: bool = tx
                .query_row("SELECT 1 FROM progress_key WHERE user_id = ?1 AND key = ?2", params![uid, key], |_| Ok(true))
                .optional()
                .map_err(crate::db::sql_err)?
                .unwrap_or(false);
            let stored: Option<Progress> = tx
                .query_row(
                    "SELECT position_ms, duration_ms, finished, updated_at FROM progress WHERE user_id = ?1 AND book_id = ?2",
                    params![uid, bid],
                    |r| Ok(Progress { position_ms: r.get::<_, i64>(0)?.max(0) as u64, duration_ms: r.get::<_, i64>(1)?.max(0) as u64, finished: r.get(2)?, updated_at: r.get(3)? }),
                )
                .optional()
                .map_err(crate::db::sql_err)?;
            let out = progress::merge(stored, ev, duration, now_ms(), seen);
            if let Outcome::Accepted(p) = out {
                tx.execute(
                    "INSERT INTO progress (user_id, book_id, position_ms, duration_ms, finished, updated_at, received_at, device_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(user_id, book_id) DO UPDATE SET position_ms = excluded.position_ms, duration_ms = excluded.duration_ms,
                     finished = excluded.finished, updated_at = excluded.updated_at, received_at = excluded.received_at, device_id = excluded.device_id",
                    params![uid, bid, p.position_ms as i64, p.duration_ms as i64, p.finished, p.updated_at, now(), device],
                )
                .map_err(crate::db::sql_err)?;
            }
            if !seen && !matches!(out, Outcome::TooFarInFuture | Outcome::TooOld) {
                tx.execute("INSERT INTO progress_key (user_id, key, at) VALUES (?1, ?2, ?3)", params![uid, key, now()]).map_err(crate::db::sql_err)?;
                tx.execute("DELETE FROM progress_key WHERE at < ?1", params![now() - 100 * 86400]).map_err(crate::db::sql_err)?;
            }
            tx.commit().map_err(crate::db::sql_err)?;
            Ok((out, stored))
        })
        .await?;
    let show =
        |p: &Progress| json!({ "position_ms": p.position_ms, "finished": p.finished, "updated_at": p.updated_at });
    match outcome {
        (Outcome::Accepted(p), _) => Ok(Json(json!({ "result": "accepted", "progress": show(&p) }))),
        (Outcome::Stale(p), _) => Ok(Json(json!({ "result": "stale", "progress": show(&p) }))),
        (Outcome::Duplicate, stored) => {
            Ok(Json(json!({ "result": "duplicate", "progress": stored.as_ref().map(show) })))
        }
        (Outcome::TooFarInFuture, _) => {
            Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "clock", "This device's clock is too far ahead."))
        }
        (Outcome::TooOld, _) => Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "too_old",
            "That listening was too long ago to record.",
        )),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Since {
    since: Option<i64>,
}

async fn progress_since(S(st): S<State>, Authed(s): Authed, Query(q): Query<Since>) -> Result<Json<Value>, ApiError> {
    let (cat, ok) = view(&st, &s).await?;
    let uid = s.user;
    let since = q.since.unwrap_or(0);
    let rows: Vec<(String, i64, bool, i64, i64)> = st
        .db
        .run(move |c| {
            let mut stmt = c
                .prepare("SELECT book_id, position_ms, finished, updated_at, received_at FROM progress WHERE user_id = ?1 AND received_at > ?2 ORDER BY received_at LIMIT 500")
                .map_err(crate::db::sql_err)?;
            let r = stmt.query_map(params![uid, since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).map_err(crate::db::sql_err)?;
            r.collect::<Result<_, _>>().map_err(crate::db::sql_err)
        })
        .await?;
    let cursor = rows.last().map_or(since, |r| r.4);
    let items: Vec<Value> = rows
        .into_iter()
        .filter(|r| find(&cat, &ok, &r.0).is_ok())
        .map(|(b, p, f, u, _)| json!({ "book": b, "position_ms": p, "finished": f, "updated_at": u }))
        .collect();
    Ok(Json(json!({ "items": items, "cursor": cursor })))
}

// ---------------------------------------------------------------- bookmarks

fn bookmarks_of(c: &rusqlite::Connection, uid: i64, bid: &str) -> Result<Vec<Value>, String> {
    let mut st = c
        .prepare("SELECT id, position_ms, note, created_at, updated_at FROM bookmark WHERE user_id = ?1 AND book_id = ?2 ORDER BY position_ms")
        .map_err(crate::db::sql_err)?;
    let r = st
        .query_map(params![uid, bid], |r| {
            Ok(json!({ "id": r.get::<_, i64>(0)?, "position_ms": r.get::<_, i64>(1)?, "note": r.get::<_, String>(2)?, "created_at": r.get::<_, i64>(3)?, "updated_at": r.get::<_, i64>(4)? }))
        })
        .map_err(crate::db::sql_err)?;
    r.collect::<Result<_, _>>().map_err(crate::db::sql_err)
}

async fn list_bookmarks(S(st): S<State>, Authed(s): Authed, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let (cat, ok) = view(&st, &s).await?;
    let bid = find(&cat, &ok, &id)?.id.clone();
    let uid = s.user;
    Ok(Json(json!({ "bookmarks": st.db.run(move |c| bookmarks_of(c, uid, &bid)).await? })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BookmarkBody {
    position_ms: Option<u64>,
    note: Option<String>,
}

async fn add_bookmark(
    S(st): S<State>,
    Authed(s): Authed,
    Path(id): Path<String>,
    Json(b): Json<BookmarkBody>,
) -> Result<Json<Value>, ApiError> {
    let (cat, ok) = view(&st, &s).await?;
    let book = find(&cat, &ok, &id)?;
    let pos = b.position_ms.ok_or_else(|| ApiError::bad("position_ms required."))?.min(book.duration_ms) as i64;
    let note = clean_paragraphs(b.note.as_deref().unwrap_or(""), 2000);
    let (uid, bid) = (s.user, book.id.clone());
    let new_id = st
        .db
        .run(move |c| {
            let n: i64 = c.query_row("SELECT count(*) FROM bookmark WHERE user_id = ?1 AND book_id = ?2", params![uid, bid], |r| r.get(0)).map_err(crate::db::sql_err)?;
            if n >= 1000 {
                return Ok(None);
            }
            c.execute(
                "INSERT INTO bookmark (user_id, book_id, position_ms, note, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![uid, bid, pos, note, now()],
            )
            .map_err(crate::db::sql_err)?;
            Ok(Some(c.last_insert_rowid()))
        })
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "too_many_bookmarks", "That book already has 1000 bookmarks."))?;
    Ok(Json(json!({ "id": new_id })))
}

async fn edit_bookmark(
    S(st): S<State>,
    Authed(s): Authed,
    Path(id): Path<i64>,
    Json(b): Json<BookmarkBody>,
) -> Result<StatusCode, ApiError> {
    let uid = s.user;
    let note = b.note.map(|n| clean_paragraphs(&n, 2000));
    let pos = b.position_ms.map(|p| p.min(i64::MAX as u64) as i64);
    let n = st
        .db
        .run(move |c| {
            c.execute(
                "UPDATE bookmark SET note = coalesce(?3, note), position_ms = coalesce(?4, position_ms), updated_at = ?5 WHERE id = ?1 AND user_id = ?2",
                params![id, uid, note, pos, now()],
            )
            .map_err(crate::db::sql_err)
        })
        .await?;
    if n == 1 { Ok(StatusCode::NO_CONTENT) } else { Err(ApiError::not_found()) }
}

async fn delete_bookmark(S(st): S<State>, Authed(s): Authed, Path(id): Path<i64>) -> Result<StatusCode, ApiError> {
    let uid = s.user;
    let n = st
        .db
        .run(move |c| {
            c.execute("DELETE FROM bookmark WHERE id = ?1 AND user_id = ?2", params![id, uid])
                .map_err(crate::db::sql_err)
        })
        .await?;
    if n == 1 { Ok(StatusCode::NO_CONTENT) } else { Err(ApiError::not_found()) }
}
