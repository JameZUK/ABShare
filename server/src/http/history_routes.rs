//! `/v1/me/history` (SPEC §5.6, §7.2): your own listening history. Sessions are recorded from
//! accepted position reports (see [`record`]), using the rules in `core::history`. Only the
//! owner can read them: there is no admin view. Books the caller can no longer see are left
//! out of the list.

use super::library_routes::{cover_url, view};
use super::{ApiError, Authed, State, now};
use abshare_core::history::{self, KEEP_MS, MAX_SESSIONS, Step, Tail};
use axum::extract::{Query, State as S};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};

pub fn routes() -> Router<State> {
    Router::new().route("/", get(list).delete(clear).patch(set_enabled))
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
                "INSERT INTO listening (user_id, book_id, device_id, started_at, ended_at, end_position_ms, heard_ms)
                 VALUES (?1, ?2, ?3, ?4, ?4, ?5, 0)",
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
