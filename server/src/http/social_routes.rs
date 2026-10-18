//! Reviews and recommendations (SPEC D28, D29).
//!
//! - Reviews: `/v1/books/{id}/reviews` and `/v1/books/{id}/review`. Anyone who can see a book
//!   can rate it 1–5 with a short comment; everyone who can see the book sees the reviews,
//!   with the reviewer's display name. A book outside the caller's libraries is "not found".
//! - Recommendations: admins send books with a note to chosen people or to everyone
//!   (`/v1/admin/recommendations`); recipients see them on Home (`recommended_for`) until they
//!   dismiss them (`/v1/recommendations/{id}/dismiss`) or have started every book in them.
//!   Books a recipient can't see are left out for them.
//!
//! People's text is cleaned by `core::text::limited_text` and only ever shown as text.

use super::admin_routes::{Admin, step_up};
use super::library_routes::{ProgressMap, find, summary, view};
use super::{ApiError, Authed, Ip, State, now};
use crate::auth::store::{self, Role};
use crate::library::Catalog;
use abshare_core::ids::is_valid_id;
use abshare_core::text::limited_text;
use axum::extract::{Path, State as S};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;

/// A review's comment: at most this many words and characters.
pub const REVIEW_WORDS: usize = 50;
pub const REVIEW_CHARS: usize = 400;
/// A recommendation's note.
pub const NOTE_WORDS: usize = 100;
pub const NOTE_CHARS: usize = 800;
const MAX_REC_BOOKS: usize = 10;
const MAX_REC_USERS: usize = 100;
const MAX_RECS: i64 = 500;
const MAX_REVIEWS_LISTED: i64 = 200;
const HOME_RECS: usize = 20;

/// Routes under `/v1`.
pub fn routes() -> Router<State> {
    Router::new()
        .route("/books/{id}/reviews", get(reviews))
        .route("/books/{id}/review", put(put_review).delete(delete_review))
        .route("/recommendations/{id}/dismiss", post(dismiss))
}

/// Routes under `/v1/admin`.
pub fn admin_routes() -> Router<State> {
    Router::new()
        .route("/recommendations", get(admin_list).post(admin_create))
        .route("/recommendations/{id}", delete(admin_withdraw))
        .route("/books/{id}/reviews/{user_id}", delete(admin_remove_review))
}

fn too_long(what: &str, words: usize, chars: usize) -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "too_long",
        format!("The {what} can be at most {words} words ({chars} characters)."),
    )
}

// ---------------------------------------------------------------- reviews

/// A book's average rating (one decimal place, or null) and number of reviews, counting
/// only reviews by enabled accounts.
pub(super) fn rating_of(c: &Connection, book: &str) -> Result<Value, String> {
    let (avg, count): (Option<f64>, i64) = c
        .query_row(
            "SELECT avg(r.rating), count(*) FROM review r JOIN user u ON u.id = r.user_id WHERE r.book_id = ?1 AND u.disabled_at IS NULL",
            [book],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(crate::db::sql_err)?;
    Ok(json!({ "average": avg.map(|a| (a * 10.0).round() / 10.0), "count": count }))
}

async fn reviews(S(st): S<State>, Authed(s): Authed, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let (cat, ok) = view(&st, &s).await?;
    let bid = find(&cat, &ok, &id)?.id.clone();
    let uid = s.user;
    let out = st
        .db
        .run(move |c| {
            let admin = store::user_by_id(c, uid)?.is_some_and(|u| u.role == Role::Admin);
            let rating = rating_of(c, &bid)?;
            let mine: Option<Value> = c
                .query_row(
                    "SELECT rating, comment, updated_at FROM review WHERE user_id = ?1 AND book_id = ?2",
                    params![uid, bid],
                    |r| Ok(json!({ "rating": r.get::<_, i64>(0)?, "comment": r.get::<_, String>(1)?, "updated_at": r.get::<_, i64>(2)? })),
                )
                .optional()
                .map_err(crate::db::sql_err)?;
            let mut stmt = c
                .prepare(
                    "SELECT r.user_id, u.display_name, r.rating, r.comment, r.updated_at FROM review r JOIN user u ON u.id = r.user_id
                     WHERE r.book_id = ?1 AND u.disabled_at IS NULL ORDER BY r.updated_at DESC, r.user_id LIMIT ?2",
                )
                .map_err(crate::db::sql_err)?;
            let list: Vec<Value> = stmt
                .query_map(params![bid, MAX_REVIEWS_LISTED], |r| {
                    let who: i64 = r.get(0)?;
                    let mut v = json!({
                        "name": r.get::<_, String>(1)?, "rating": r.get::<_, i64>(2)?, "comment": r.get::<_, String>(3)?,
                        "updated_at": r.get::<_, i64>(4)?, "mine": who == uid,
                    });
                    if admin {
                        v["user_id"] = json!(who);
                    }
                    Ok(v)
                })
                .map_err(crate::db::sql_err)?
                .collect::<Result<_, _>>()
                .map_err(crate::db::sql_err)?;
            Ok(json!({ "average": rating["average"], "count": rating["count"], "mine": mine, "reviews": list }))
        })
        .await?;
    Ok(Json(out))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewBody {
    rating: i64,
    comment: Option<String>,
}

async fn put_review(
    S(st): S<State>,
    Authed(s): Authed,
    Path(id): Path<String>,
    Json(b): Json<ReviewBody>,
) -> Result<StatusCode, ApiError> {
    let (cat, ok) = view(&st, &s).await?;
    let bid = find(&cat, &ok, &id)?.id.clone();
    if !(1..=5).contains(&b.rating) {
        return Err(ApiError::bad("The rating must be a whole number from 1 to 5."));
    }
    let comment = limited_text(b.comment.as_deref().unwrap_or(""), REVIEW_WORDS, REVIEW_CHARS)
        .ok_or_else(|| too_long("comment", REVIEW_WORDS, REVIEW_CHARS))?;
    let (uid, rating) = (s.user, b.rating);
    st.db
        .run(move |c| {
            c.execute(
                "INSERT INTO review (user_id, book_id, rating, comment, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                 ON CONFLICT (user_id, book_id) DO UPDATE SET rating = excluded.rating, comment = excluded.comment, updated_at = excluded.updated_at",
                params![uid, bid, rating, comment, now()],
            )
            .map_err(crate::db::sql_err)
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_review(S(st): S<State>, Authed(s): Authed, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let (cat, ok) = view(&st, &s).await?;
    let bid = find(&cat, &ok, &id)?.id.clone();
    let uid = s.user;
    let n = st
        .db
        .run(move |c| {
            c.execute("DELETE FROM review WHERE user_id = ?1 AND book_id = ?2", params![uid, bid])
                .map_err(crate::db::sql_err)
        })
        .await?;
    if n == 1 { Ok(StatusCode::NO_CONTENT) } else { Err(ApiError::not_found()) }
}

async fn admin_remove_review(
    S(st): S<State>,
    Ip(ip): Ip,
    Admin(s): Admin,
    Path((book, user)): Path<(String, i64)>,
) -> Result<StatusCode, ApiError> {
    step_up(&st, &s)?;
    if !is_valid_id(&book) {
        return Err(ApiError::not_found());
    }
    let b = book.clone();
    let n = st
        .db
        .run(move |c| {
            c.execute("DELETE FROM review WHERE user_id = ?1 AND book_id = ?2", params![user, b])
                .map_err(crate::db::sql_err)
        })
        .await?;
    if n != 1 {
        return Err(ApiError::not_found());
    }
    st.audit("review.removed", Some(s.user), Some(s.device), Some(ip), format!("book={book} user={user}")).await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- recommendations

/// Started means a position past the start, or finished.
fn started(prog: &ProgressMap, book: &str) -> bool {
    prog.get(book).is_some_and(|p| p.1 || p.0 > 0)
}

/// The recommendations addressed to `uid` that they haven't dismissed, newest first, each
/// with the books they can see and haven't started. Ones left with no books are dropped.
pub(super) fn recommended_for(
    c: &Connection,
    uid: i64,
    cat: &Catalog,
    ok: &HashSet<i64>,
    prog: &ProgressMap,
) -> Result<Vec<Value>, String> {
    let mut stmt = c
        .prepare(
            "SELECT r.id, r.note, r.created_at, u.display_name FROM recommendation r JOIN user u ON u.id = r.by_user
             WHERE (r.everyone = 1 OR EXISTS (SELECT 1 FROM recommendation_target t WHERE t.rec_id = r.id AND t.user_id = ?1))
               AND NOT EXISTS (SELECT 1 FROM recommendation_dismissed d WHERE d.rec_id = r.id AND d.user_id = ?1)
             ORDER BY r.created_at DESC, r.id DESC LIMIT ?2",
        )
        .map_err(crate::db::sql_err)?;
    let recs: Vec<(i64, String, i64, String)> = stmt
        .query_map(params![uid, MAX_RECS], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(crate::db::sql_err)?
        .collect::<Result<_, _>>()
        .map_err(crate::db::sql_err)?;
    let mut books = c
        .prepare("SELECT book_id FROM recommendation_book WHERE rec_id = ?1 ORDER BY ord")
        .map_err(crate::db::sql_err)?;
    let mut out = Vec::new();
    for (id, note, created, from) in recs {
        let ids: Vec<String> = books
            .query_map([id], |r| r.get(0))
            .map_err(crate::db::sql_err)?
            .collect::<Result<_, _>>()
            .map_err(crate::db::sql_err)?;
        let shown: Vec<Value> = ids
            .iter()
            .filter_map(|b| cat.by_id.get(b).map(|&i| &cat.books[i]))
            .filter(|b| ok.contains(&b.library_id) && !started(prog, &b.id))
            .map(|b| summary(b, prog.get(&b.id)))
            .collect();
        if !shown.is_empty() {
            out.push(json!({ "id": id, "from": from, "note": note, "created_at": created, "books": shown }));
        }
        if out.len() == HOME_RECS {
            break;
        }
    }
    Ok(out)
}

async fn dismiss(S(st): S<State>, Authed(s): Authed, Path(id): Path<i64>) -> Result<StatusCode, ApiError> {
    let uid = s.user;
    let found = st
        .db
        .run(move |c| {
            let addressed: bool = c
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM recommendation r WHERE r.id = ?1
                       AND (r.everyone = 1 OR EXISTS (SELECT 1 FROM recommendation_target t WHERE t.rec_id = r.id AND t.user_id = ?2)))",
                    params![id, uid],
                    |r| r.get(0),
                )
                .map_err(crate::db::sql_err)?;
            if addressed {
                c.execute(
                    "INSERT OR IGNORE INTO recommendation_dismissed (rec_id, user_id, at) VALUES (?1, ?2, ?3)",
                    params![id, uid, now()],
                )
                .map_err(crate::db::sql_err)?;
            }
            Ok(addressed)
        })
        .await?;
    if found { Ok(StatusCode::NO_CONTENT) } else { Err(ApiError::not_found()) }
}

async fn admin_list(S(st): S<State>, Admin(_): Admin) -> Result<Json<Value>, ApiError> {
    let cat = st.catalog().await?;
    let rows = st
        .db
        .run(move |c| {
            let mut stmt = c
                .prepare(
                    "SELECT r.id, r.note, r.created_at, r.everyone, u.display_name,
                       (SELECT count(*) FROM recommendation_dismissed d WHERE d.rec_id = r.id)
                     FROM recommendation r JOIN user u ON u.id = r.by_user ORDER BY r.created_at DESC, r.id DESC",
                )
                .map_err(crate::db::sql_err)?;
            let recs: Vec<(i64, String, i64, bool, String, i64)> = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
                .map_err(crate::db::sql_err)?
                .collect::<Result<_, _>>()
                .map_err(crate::db::sql_err)?;
            let mut books = c
                .prepare("SELECT book_id FROM recommendation_book WHERE rec_id = ?1 ORDER BY ord")
                .map_err(crate::db::sql_err)?;
            let mut users = c
                .prepare(
                    "SELECT u.id, u.display_name FROM recommendation_target t JOIN user u ON u.id = t.user_id
                     WHERE t.rec_id = ?1 ORDER BY u.display_name",
                )
                .map_err(crate::db::sql_err)?;
            let mut out = Vec::new();
            for (id, note, created, everyone, from, dismissed) in recs {
                let ids: Vec<String> = books
                    .query_map([id], |r| r.get(0))
                    .map_err(crate::db::sql_err)?
                    .collect::<Result<_, _>>()
                    .map_err(crate::db::sql_err)?;
                let to: Vec<Value> = users
                    .query_map([id], |r| Ok(json!({ "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)? })))
                    .map_err(crate::db::sql_err)?
                    .collect::<Result<_, _>>()
                    .map_err(crate::db::sql_err)?;
                out.push((id, note, created, everyone, from, dismissed, ids, to));
            }
            Ok(out)
        })
        .await?;
    let list: Vec<Value> = rows
        .into_iter()
        .map(|(id, note, created, everyone, from, dismissed, ids, to)| {
            let books: Vec<Value> = ids
                .iter()
                .filter_map(|b| cat.by_id.get(b).map(|&i| &cat.books[i]))
                .map(|b| json!({ "id": b.id, "title": b.title }))
                .collect();
            json!({ "id": id, "from": from, "note": note, "created_at": created, "everyone": everyone, "users": to, "books": books, "dismissed": dismissed })
        })
        .collect();
    Ok(Json(json!({ "recommendations": list })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewRecommendation {
    books: Vec<String>,
    note: String,
    everyone: bool,
    #[serde(default)]
    users: Vec<i64>,
}

async fn admin_create(
    S(st): S<State>,
    Ip(ip): Ip,
    Admin(s): Admin,
    Json(b): Json<NewRecommendation>,
) -> Result<Json<Value>, ApiError> {
    step_up(&st, &s)?;
    let cat = st.catalog().await?;
    if b.books.is_empty() || b.books.len() > MAX_REC_BOOKS {
        return Err(ApiError::bad(format!("Recommend between 1 and {MAX_REC_BOOKS} books.")));
    }
    let mut seen = HashSet::new();
    for id in &b.books {
        if !is_valid_id(id) || !cat.by_id.contains_key(id) || !seen.insert(id.as_str()) {
            return Err(ApiError::bad("Each book must be a different book in the library."));
        }
    }
    let note = limited_text(&b.note, NOTE_WORDS, NOTE_CHARS).ok_or_else(|| too_long("note", NOTE_WORDS, NOTE_CHARS))?;
    if note.is_empty() {
        return Err(ApiError::bad("Say why you recommend it."));
    }
    let users: Vec<i64> = b.users.iter().copied().collect::<HashSet<_>>().into_iter().collect();
    if b.everyone && !b.users.is_empty() {
        return Err(ApiError::bad("Choose everyone or a list of people, not both."));
    }
    if !b.everyone && (users.is_empty() || users.len() != b.users.len() || users.len() > MAX_REC_USERS) {
        return Err(ApiError::bad(format!("Choose between 1 and {MAX_REC_USERS} different people, or everyone.")));
    }
    let (by, everyone, books) = (s.user, b.everyone, b.books);
    let count_books = books.len();
    let audience = if everyone { "everyone".to_string() } else { format!("{} people", users.len()) };
    let created = st
        .db
        .run(move |c| {
            let tx = c.unchecked_transaction().map_err(crate::db::sql_err)?;
            for u in &users {
                if !store::user_by_id(&tx, *u)?.is_some_and(|u| !u.disabled) {
                    return Ok(Err(ApiError::bad("Each person must be an existing, enabled account.")));
                }
            }
            let total: i64 =
                tx.query_row("SELECT count(*) FROM recommendation", [], |r| r.get(0)).map_err(crate::db::sql_err)?;
            if total >= MAX_RECS {
                return Ok(Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "too_many",
                    "There are already 500 recommendations; withdraw some first.",
                )));
            }
            tx.execute(
                "INSERT INTO recommendation (by_user, note, everyone, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![by, note, everyone, now()],
            )
            .map_err(crate::db::sql_err)?;
            let id = tx.last_insert_rowid();
            for (i, book) in books.iter().enumerate() {
                tx.execute(
                    "INSERT INTO recommendation_book (rec_id, ord, book_id) VALUES (?1, ?2, ?3)",
                    params![id, i as i64, book],
                )
                .map_err(crate::db::sql_err)?;
            }
            for u in &users {
                tx.execute("INSERT INTO recommendation_target (rec_id, user_id) VALUES (?1, ?2)", params![id, u])
                    .map_err(crate::db::sql_err)?;
            }
            tx.commit().map_err(crate::db::sql_err)?;
            Ok(Ok(id))
        })
        .await??;
    st.audit(
        "recommendation.created",
        Some(s.user),
        Some(s.device),
        Some(ip),
        format!("recommendation={created} books={count_books} to={audience}"),
    )
    .await;
    Ok(Json(json!({ "id": created })))
}

async fn admin_withdraw(
    S(st): S<State>,
    Ip(ip): Ip,
    Admin(s): Admin,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    step_up(&st, &s)?;
    let n = st
        .db
        .run(move |c| c.execute("DELETE FROM recommendation WHERE id = ?1", [id]).map_err(crate::db::sql_err))
        .await?;
    if n != 1 {
        return Err(ApiError::not_found());
    }
    st.audit("recommendation.withdrawn", Some(s.user), Some(s.device), Some(ip), format!("recommendation={id}")).await;
    Ok(StatusCode::NO_CONTENT)
}
