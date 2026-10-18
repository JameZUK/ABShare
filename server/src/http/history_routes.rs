//! `/v1/me/history` (SPEC §5.6, §7.2): your own listening history. Sessions are recorded from
//! accepted position reports (see [`record`]), using the rules in `core::history`. Only the
//! owner can read them: there is no admin view. Books the caller can no longer see are left
//! out of the list.

use super::admin_routes::Admin;
use super::library_routes::{cover_url, view};
use super::{ApiError, Authed, State, now};
use abshare_core::history::{self, BookStatus, KEEP_MS, MAX_SESSIONS, Step, Tail, book_status};
use abshare_core::progress::{MAX_AGE_MS, MAX_FUTURE_MS};
use axum::extract::{Path, Query, State as S};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};

pub fn routes() -> Router<State> {
    Router::new()
        .route("/", get(list).delete(clear).patch(set_enabled))
        .route("/books/{id}", get(book_timeline))
        .route("/replay", post(replay))
}

/// Sessions shorter than this aren't listed (a report on pause, or a seek, starts one).
const MIN_LISTED_MS: i64 = 1_000;
const DAY_MS: i64 = 24 * 3600 * 1000;

/// Records an accepted position report into the history, inside the caller's transaction.
/// Does nothing when the person has turned history off.
pub(super) fn record(
    c: &Connection,
    uid: i64,
    device: i64,
    book: &str,
    event_time: i64,
    position_ms: u64,
) -> Result<(), String> {
    let off: bool = c
        .query_row("SELECT history_off FROM user WHERE id = ?1", [uid], |r| r.get(0))
        .optional()
        .map_err(crate::db::sql_err)?
        .unwrap_or(true);
    if off {
        return Ok(());
    }
    let tail: Option<(i64, Tail)> = c
        .query_row(
            "SELECT id, ended_at, end_position_ms FROM listening WHERE user_id = ?1 AND book_id = ?2 AND device_id = ?3
             ORDER BY id DESC LIMIT 1",
            params![uid, book, device],
            |r| Ok((r.get(0)?, Tail { ended_at: r.get(1)?, position_ms: r.get::<_, i64>(2)?.max(0) as u64 })),
        )
        .optional()
        .map_err(crate::db::sql_err)?;
    let pos = |p: u64| i64::try_from(p).unwrap_or(i64::MAX);
    // The same report again (an offline report replayed, then the same position sent as
    // progress): nothing to add.
    if tail.is_some_and(|(_, t)| t.ended_at == event_time && t.position_ms == position_ms) {
        return Ok(());
    }
    match (history::step(tail.map(|t| t.1), event_time, position_ms), tail) {
        (Step::Extend { ended_at, position_ms, heard_ms }, Some((id, _))) => {
            c.execute(
                "UPDATE listening SET ended_at = ?2, end_position_ms = ?3, heard_ms = heard_ms + ?4 WHERE id = ?1",
                params![id, ended_at, pos(position_ms), pos(heard_ms)],
            )
            .map_err(crate::db::sql_err)?;
        }
        (Step::Start { at, position_ms }, _) | (Step::Extend { ended_at: at, position_ms, .. }, None) => {
            c.execute(
                "INSERT INTO listening (user_id, book_id, device_id, started_at, ended_at, start_position_ms, end_position_ms, heard_ms)
                 VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?5, 0)",
                params![uid, book, device, at, pos(position_ms)],
            )
            .map_err(crate::db::sql_err)?;
            cap(c, uid, MAX_SESSIONS)?;
        }
    }
    Ok(())
}

/// Keeps only the person's newest `keep` sessions.
pub(crate) fn cap(c: &Connection, uid: i64, keep: i64) -> Result<usize, String> {
    c.execute(
        "DELETE FROM listening WHERE user_id = ?1 AND id <= (SELECT id FROM listening WHERE user_id = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?2)",
        params![uid, keep],
    )
    .map_err(crate::db::sql_err)
}

/// Removes everyone's sessions that ended more than 13 months before `now_ms`.
pub(crate) fn prune(c: &Connection, now_ms: i64) -> Result<usize, String> {
    c.execute("DELETE FROM listening WHERE ended_at < ?1", [now_ms.saturating_sub(KEEP_MS)]).map_err(crate::db::sql_err)
}

/// Prunes old history at start-up and then hourly, so it expires even for people who have
/// turned history off or stopped listening.
pub fn spawn_prune(st: State) {
    tokio::spawn(async move {
        loop {
            if st.db.run(|c| prune(c, now_ms())).await.is_err() {
                eprintln!("abshare: listening history prune failed");
            }
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    });
}

fn now_ms() -> i64 {
    now().saturating_mul(1000)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    cursor: Option<String>,
    limit: Option<String>,
}

async fn list(S(st): S<State>, Authed(s): Authed, Query(q): Query<ListQuery>) -> Result<Json<Value>, ApiError> {
    let limit: i64 = match q.limit.as_deref() {
        None => 50,
        Some(l) => l.parse().ok().filter(|n| (1..=100).contains(n)).ok_or_else(|| ApiError::bad("Bad limit."))?,
    };
    let before: i64 = match q.cursor.as_deref() {
        None => i64::MAX,
        Some(c) => c
            .parse()
            .ok()
            .filter(|n| *n > 0 && c.len() <= 19 && c.bytes().all(|b| b.is_ascii_digit()))
            .ok_or_else(|| ApiError::bad("Bad cursor."))?,
    };
    let (cat, ok) = view(&st, &s).await?;
    let uid = s.user;
    let now = now_ms();
    type Row = (i64, String, i64, i64, i64);
    let (enabled, week, month, all, finished, rows) = st
        .db
        .run(move |c| {
            let off: bool = c
                .query_row("SELECT history_off FROM user WHERE id = ?1", [uid], |r| r.get(0))
                .map_err(crate::db::sql_err)?;
            let since = |from: i64| -> Result<i64, String> {
                c.query_row(
                    "SELECT coalesce(sum(heard_ms), 0) FROM listening WHERE user_id = ?1 AND ended_at >= ?2",
                    params![uid, from],
                    |r| r.get(0),
                )
                .map_err(crate::db::sql_err)
            };
            let (week, month, all) = (since(now - 7 * DAY_MS)?, since(now - 30 * DAY_MS)?, since(i64::MIN)?);
            let mut f = c
                .prepare("SELECT book_id FROM progress WHERE user_id = ?1 AND finished = 1")
                .map_err(crate::db::sql_err)?;
            let finished: Vec<String> = f
                .query_map([uid], |r| r.get(0))
                .map_err(crate::db::sql_err)?
                .collect::<Result<_, _>>()
                .map_err(crate::db::sql_err)?;
            let mut q = c
                .prepare(
                    "SELECT id, book_id, started_at, ended_at, heard_ms FROM listening
                     WHERE user_id = ?1 AND id < ?2 AND heard_ms >= ?3 ORDER BY id DESC LIMIT ?4",
                )
                .map_err(crate::db::sql_err)?;
            let rows: Vec<Row> = q
                .query_map(params![uid, before, MIN_LISTED_MS, limit], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })
                .map_err(crate::db::sql_err)?
                .collect::<Result<_, _>>()
                .map_err(crate::db::sql_err)?;
            Ok((!off, week, month, all, finished, rows))
        })
        .await?;
    let visible = |id: &str| cat.by_id.get(id).map(|&i| &cat.books[i]).filter(|b| ok.contains(&b.library_id));
    let next = (rows.len() as i64 == limit).then(|| rows.last().map(|r| r.0.to_string())).flatten();
    let sessions: Vec<Value> = rows
        .iter()
        .filter_map(|(id, book, started, ended, heard)| {
            let b = visible(book)?;
            Some(json!({
                "id": id,
                "book": { "id": b.id, "title": b.title, "authors": b.authors, "cover": cover_url(b) },
                "started_at": started, "ended_at": ended, "seconds": heard / 1000,
            }))
        })
        .collect();
    let books_finished = finished.iter().filter(|id| visible(id).is_some()).count();
    Ok(Json(json!({
        "enabled": enabled,
        "totals": { "week_seconds": week / 1000, "month_seconds": month / 1000, "all_seconds": all / 1000, "books_finished": books_finished },
        "sessions": sessions,
        "next": next,
    })))
}

/// `GET /v1/me/history/books/{id}`: the caller's own listening of one book, newest first:
/// each stretch with when it began and ended and where in the book, so they can play from
/// any of those places. Owner only; there is no admin equivalent (SPEC D31).
async fn book_timeline(S(st): S<State>, Authed(s): Authed, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let (cat, ok) = view(&st, &s).await?;
    let book = super::library_routes::find(&cat, &ok, &id)?;
    let (uid, bid, duration) = (s.user, book.id.clone(), book.duration_ms);
    type Row = (i64, i64, i64, Option<i64>, i64, i64);
    let (enabled, rows) = st
        .db
        .run(move |c| {
            let off: bool = c
                .query_row("SELECT history_off FROM user WHERE id = ?1", [uid], |r| r.get(0))
                .map_err(crate::db::sql_err)?;
            let mut q = c
                .prepare(
                    "SELECT id, started_at, ended_at, start_position_ms, end_position_ms, heard_ms FROM listening
                     WHERE user_id = ?1 AND book_id = ?2 AND heard_ms >= ?3 ORDER BY started_at DESC, id DESC LIMIT 200",
                )
                .map_err(crate::db::sql_err)?;
            let rows: Vec<Row> = q
                .query_map(params![uid, bid, MIN_LISTED_MS], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
                .map_err(crate::db::sql_err)?
                .collect::<Result<_, _>>()
                .map_err(crate::db::sql_err)?;
            Ok((!off, rows))
        })
        .await?;
    let clamp = |p: i64| p.clamp(0, i64::try_from(duration).unwrap_or(i64::MAX));
    let sessions: Vec<Value> = rows
        .into_iter()
        .map(|(id, started, ended, start_pos, end_pos, heard)| {
            json!({
                "id": id, "started_at": started, "ended_at": ended,
                // Sessions recorded before start positions were kept begin where they ended,
                // less the time heard.
                "start_position_ms": clamp(start_pos.unwrap_or(end_pos - heard)), "end_position_ms": clamp(end_pos),
                "seconds": heard / 1000,
            })
        })
        .collect();
    Ok(Json(json!({ "enabled": enabled, "sessions": sessions })))
}

/// At most this many offline reports in one replay request (they fit well within the 16 KiB
/// body limit of `/v1/me`).
const MAX_REPLAY: usize = 200;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Heard {
    event_time_ms: i64,
    position_ms: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayBody {
    book: String,
    key: String,
    reports: Vec<Heard>,
}

/// `POST /v1/me/history/replay`: positions reported while offline, which the app could only
/// keep locally (the progress outbox sends just the newest), replayed into the caller's own
/// history in time order with the usual rules (SPEC D33). Idempotent by `key`. Progress itself
/// is unchanged: it still follows the newest position, sent as before.
async fn replay(S(st): S<State>, Authed(s): Authed, Json(b): Json<ReplayBody>) -> Result<Json<Value>, ApiError> {
    let (cat, ok) = view(&st, &s).await?;
    let book = super::library_routes::find(&cat, &ok, &b.book)?;
    if !(8..=64).contains(&b.key.len()) || !b.key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_') {
        return Err(ApiError::bad("Bad idempotency key."));
    }
    if b.reports.is_empty() || b.reports.len() > MAX_REPLAY {
        return Err(ApiError::bad(format!("Send between 1 and {MAX_REPLAY} reports.")));
    }
    if !st.progress_limit.allow((s.device, "history-replay".to_owned()), now()) {
        return Err(ApiError::too_many());
    }
    let now = now_ms();
    let duration = book.duration_ms;
    let mut reports: Vec<(i64, u64)> = b
        .reports
        .iter()
        .filter(|r| {
            r.event_time_ms >= now.saturating_sub(MAX_AGE_MS) && r.event_time_ms <= now.saturating_add(MAX_FUTURE_MS)
        })
        .map(|r| (r.event_time_ms, r.position_ms.min(duration)))
        .collect();
    reports.sort_unstable();
    reports.dedup();
    let (uid, device, bid, key) = (s.user, s.device, book.id.clone(), b.key);
    let (duplicate, recorded) = st
        .db
        .run(move |c| {
            let tx = c.transaction().map_err(crate::db::sql_err)?;
            let seen: bool = tx
                .query_row("SELECT 1 FROM progress_key WHERE user_id = ?1 AND key = ?2", params![uid, key], |_| {
                    Ok(true)
                })
                .optional()
                .map_err(crate::db::sql_err)?
                .unwrap_or(false);
            if seen {
                return Ok((true, 0));
            }
            for &(at, position) in &reports {
                record(&tx, uid, device, &bid, at, position)?;
            }
            tx.execute(
                "INSERT INTO progress_key (user_id, key, at) VALUES (?1, ?2, ?3)",
                params![uid, key, super::now()],
            )
            .map_err(crate::db::sql_err)?;
            tx.commit().map_err(crate::db::sql_err)?;
            Ok((false, reports.len()))
        })
        .await?;
    Ok(Json(json!({ "result": if duplicate { "duplicate" } else { "accepted" }, "recorded": recorded })))
}

async fn clear(S(st): S<State>, Authed(s): Authed) -> Result<StatusCode, ApiError> {
    if !st.history_limit.allow(s.device, now()) {
        return Err(ApiError::too_many());
    }
    let uid = s.user;
    st.db
        .run(move |c| c.execute("DELETE FROM listening WHERE user_id = ?1", [uid]).map_err(crate::db::sql_err))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Enabled {
    enabled: bool,
}

/// Takes an object only: serde would otherwise also accept a struct as an array (`[false]`).
async fn set_enabled(
    S(st): S<State>,
    Authed(s): Authed,
    Json(body): Json<serde_json::Map<String, Value>>,
) -> Result<StatusCode, ApiError> {
    let b: Enabled = serde_json::from_value(Value::Object(body))
        .map_err(|_| ApiError::bad("Expected {\"enabled\": true|false}."))?;
    if !st.history_limit.allow(s.device, now()) {
        return Err(ApiError::too_many());
    }
    let uid = s.user;
    st.db
        .run(move |c| {
            c.execute("UPDATE user SET history_off = ?2 WHERE id = ?1", params![uid, !b.enabled])
                .map_err(crate::db::sql_err)
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- admin view (SPEC D30)

/// Routes under `/v1/admin`: everyone's listening at a glance, and one person's in detail.
pub fn admin_routes() -> Router<State> {
    Router::new().route("/listening", get(admin_people)).route("/users/{id}/listening", get(admin_person))
}

/// One person's listening, summarised: seconds heard over 7 and 30 days and in all, the last
/// time they listened, and each book's status (from progress and the last listening).
struct Summary {
    week: i64,
    month: i64,
    all: i64,
    last: Option<i64>,
    /// book → (status, position ms, heard ms, last activity ms)
    books: Vec<(String, BookStatus, u64, i64, i64)>,
}

fn summarise(c: &Connection, uid: i64, now: i64) -> Result<Summary, String> {
    let since = |from: i64| -> Result<i64, String> {
        c.query_row(
            "SELECT coalesce(sum(heard_ms), 0) FROM listening WHERE user_id = ?1 AND ended_at >= ?2",
            params![uid, from],
            |r| r.get(0),
        )
        .map_err(crate::db::sql_err)
    };
    let (week, month, all) = (since(now - 7 * DAY_MS)?, since(now - 30 * DAY_MS)?, since(i64::MIN)?);
    let last: Option<i64> = c
        .query_row("SELECT max(ended_at) FROM listening WHERE user_id = ?1", [uid], |r| r.get(0))
        .map_err(crate::db::sql_err)?;
    let mut q = c
        .prepare(
            "SELECT p.book_id, p.position_ms, p.finished, p.updated_at,
               (SELECT coalesce(sum(l.heard_ms), 0) FROM listening l WHERE l.user_id = p.user_id AND l.book_id = p.book_id),
               (SELECT max(l.ended_at) FROM listening l WHERE l.user_id = p.user_id AND l.book_id = p.book_id)
             FROM progress p WHERE p.user_id = ?1",
        )
        .map_err(crate::db::sql_err)?;
    let books = q
        .query_map([uid], |r| {
            let (book, pos, fin, updated, heard, last_heard): (String, i64, bool, i64, i64, Option<i64>) =
                (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?);
            let activity = last_heard.map_or(updated, |l| l.max(updated));
            let pos = pos.max(0) as u64;
            Ok((book, book_status(fin, pos, activity, now), pos, heard, activity))
        })
        .map_err(crate::db::sql_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(crate::db::sql_err)?;
    Ok(Summary { week, month, all, last, books })
}

fn counts(sm: &Summary) -> Value {
    let n = |st: BookStatus| sm.books.iter().filter(|b| b.1 == st).count();
    json!({
        "week_seconds": sm.week / 1000, "month_seconds": sm.month / 1000, "all_seconds": sm.all / 1000,
        "finished": n(BookStatus::Finished), "in_progress": n(BookStatus::InProgress), "abandoned": n(BookStatus::Abandoned),
    })
}

async fn admin_people(S(st): S<State>, Admin(_): Admin) -> Result<Json<Value>, ApiError> {
    let now = now_ms();
    let people = st
        .db
        .run(move |c| {
            let mut q = c
                .prepare("SELECT id, username, display_name, history_off, disabled_at IS NOT NULL FROM user ORDER BY display_name, id")
                .map_err(crate::db::sql_err)?;
            let users: Vec<(i64, String, String, bool, bool)> = q
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
                .map_err(crate::db::sql_err)?
                .collect::<Result<_, _>>()
                .map_err(crate::db::sql_err)?;
            let mut out = Vec::new();
            for (id, username, name, off, disabled) in users {
                let sm = summarise(c, id, now)?;
                out.push(json!({
                    "id": id, "username": username, "name": name, "history_on": !off, "disabled": disabled,
                    "totals": counts(&sm), "last_listened_at": sm.last,
                }));
            }
            Ok(out)
        })
        .await?;
    Ok(Json(json!({ "people": people })))
}

async fn admin_person(S(st): S<State>, Admin(_): Admin, Path(id): Path<i64>) -> Result<Json<Value>, ApiError> {
    let cat = st.catalog().await?;
    let now = now_ms();
    type Session = (String, i64, i64, i64);
    let found = st
        .db
        .run(move |c| {
            let Some((name, off)) = c
                .query_row("SELECT display_name, history_off FROM user WHERE id = ?1", [id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
                })
                .optional()
                .map_err(crate::db::sql_err)?
            else {
                return Ok(None);
            };
            let sm = summarise(c, id, now)?;
            let mut q = c
                .prepare(
                    "SELECT book_id, started_at, ended_at, heard_ms FROM listening
                     WHERE user_id = ?1 AND heard_ms >= ?2 ORDER BY id DESC LIMIT 50",
                )
                .map_err(crate::db::sql_err)?;
            let sessions: Vec<Session> = q
                .query_map(params![id, MIN_LISTED_MS], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .map_err(crate::db::sql_err)?
                .collect::<Result<_, _>>()
                .map_err(crate::db::sql_err)?;
            Ok(Some((name, off, sm, sessions)))
        })
        .await?;
    let Some((name, off, mut sm, sessions)) = found else { return Err(ApiError::not_found()) };
    // Admins see every book, whatever the person's library access; a book no longer in the
    // library keeps its place with no title.
    let book = |b: &str| {
        cat.by_id.get(b).map(|&i| &cat.books[i]).map_or_else(
            || json!({ "id": b, "title": null, "authors": [], "cover": null, "duration_ms": null }),
            |b| json!({ "id": b.id, "title": b.title, "authors": b.authors, "cover": cover_url(b), "duration_ms": b.duration_ms }),
        )
    };
    sm.books.sort_by_key(|b| std::cmp::Reverse(b.4));
    let books: Vec<Value> = sm
        .books
        .iter()
        .filter(|b| b.1 != BookStatus::NotStarted)
        // No positions here: where someone is in a book, and their timeline, stay theirs.
        .map(|(id, status, _position, heard, last)| {
            json!({ "book": book(id), "status": status.as_str(), "heard_seconds": heard / 1000, "last_at": last })
        })
        .collect();
    let sessions: Vec<Value> = sessions
        .iter()
        .map(|(b, started, ended, heard)| json!({ "book": book(b), "started_at": started, "ended_at": ended, "seconds": heard / 1000 }))
        .collect();
    Ok(Json(json!({
        "id": id, "name": name, "history_on": !off, "totals": counts(&sm), "last_listened_at": sm.last,
        "books": books, "sessions": sessions,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db_with_user() -> Connection {
        let c = crate::db::open(std::path::Path::new(":memory:")).expect("db");
        c.execute(
            "INSERT INTO user (id, username, webauthn_id, display_name, role, created_at) VALUES (1, 'ann', 'w1', 'Ann', 'listener', 0),
             (2, 'bob', 'w2', 'Bob', 'listener', 0)",
            [],
        )
        .expect("users");
        c
    }

    fn sessions(c: &Connection, uid: i64) -> Vec<(i64, i64, i64)> {
        let mut q = c
            .prepare("SELECT started_at, ended_at, heard_ms FROM listening WHERE user_id = ?1 ORDER BY id")
            .expect("prepare");
        q.query_map([uid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).expect("query").map(|r| r.expect("row")).collect()
    }

    const T: i64 = 1_800_000_000_000;

    #[test]
    fn reports_extend_a_session_per_book_and_device() {
        let c = db_with_user();
        for (t, p) in [(T, 0), (T + 15_000, 15_000), (T + 30_000, 30_000)] {
            record(&c, 1, 7, "book-a", t, p).expect("record");
        }
        assert_eq!(sessions(&c, 1), vec![(T, T + 30_000, 30_000)]);
        // Another device, another book, or a long pause each start their own.
        record(&c, 1, 8, "book-a", T + 45_000, 45_000).expect("record");
        record(&c, 1, 7, "book-b", T + 45_000, 45_000).expect("record");
        record(&c, 1, 7, "book-a", T + 30_000 + history::JOIN_GAP_MS + 1, 40_000).expect("record");
        assert_eq!(sessions(&c, 1).len(), 4);
        assert!(sessions(&c, 2).is_empty());
    }

    #[test]
    fn nothing_is_recorded_when_history_is_off() {
        let c = db_with_user();
        c.execute("UPDATE user SET history_off = 1 WHERE id = 1", []).expect("off");
        record(&c, 1, 7, "book-a", T, 0).expect("record");
        assert!(sessions(&c, 1).is_empty());
        // An unknown user records nothing either.
        record(&c, 99, 7, "book-a", T, 0).expect("record");
        assert_eq!(c.query_row("SELECT count(*) FROM listening", [], |r| r.get::<_, i64>(0)).expect("count"), 0);
    }

    #[test]
    fn the_cap_keeps_the_newest_sessions_of_that_person_only() {
        let c = db_with_user();
        for i in 0..5 {
            record(&c, 1, 7, "book-a", T + i * 10 * history::JOIN_GAP_MS, 0).expect("record");
            record(&c, 2, 7, "book-a", T + i * 10 * history::JOIN_GAP_MS, 0).expect("record");
        }
        assert_eq!(cap(&c, 1, 3).expect("cap"), 2);
        let left: Vec<i64> = sessions(&c, 1).iter().map(|s| s.0).collect();
        assert_eq!(
            left,
            vec![T + 20 * history::JOIN_GAP_MS, T + 30 * history::JOIN_GAP_MS, T + 40 * history::JOIN_GAP_MS]
        );
        assert_eq!(sessions(&c, 2).len(), 5);
        assert_eq!(cap(&c, 1, 3).expect("cap"), 0);
    }

    #[test]
    fn sessions_older_than_thirteen_months_are_pruned() {
        let c = db_with_user();
        record(&c, 1, 7, "book-a", T - KEEP_MS - 1, 0).expect("record");
        record(&c, 1, 7, "book-b", T - KEEP_MS, 0).expect("record");
        record(&c, 2, 7, "book-a", T - KEEP_MS - 5, 0).expect("record");
        assert_eq!(prune(&c, T).expect("prune"), 2);
        assert_eq!(sessions(&c, 1), vec![(T - KEEP_MS, T - KEEP_MS, 0)]);
        assert!(sessions(&c, 2).is_empty());
    }

    #[test]
    fn a_deleted_user_takes_their_history_with_them() {
        let c = db_with_user();
        record(&c, 1, 7, "book-a", T, 0).expect("record");
        c.execute("DELETE FROM user WHERE id = 1", []).expect("delete");
        assert!(sessions(&c, 1).is_empty());
    }
}
