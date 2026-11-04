//! Playback reports from the web app (SPEC D38). A listener whose book stops by itself, or
//! plays through a call, sends the player's recent events from Settings → About, so an admin
//! can see what happened without the phone.
//!
//! Signed-in listeners only (a bearer token), with the CSRF guard too; a strict schema of at
//! most 40 cleaned lines of 300 characters; 5 reports a minute per person; and storage in
//! memory only (the newest 50), so it can't be used to fill a disk. Admins read them, with
//! who sent each, at `GET /v1/admin/diagnostics`.
//!
//! This route used to take anonymous start-up reports from a page left blank (nobody could be
//! signed in then). That page was removed on 2026-11-04, and the route with it became
//! signed-in only.

use super::{ApiError, Authed, Ip, State, now};
use crate::auth::store;
use abshare_core::text::clean_line;
use axum::extract::{DefaultBodyLimit, State as S};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

/// Reports kept (newest first out).
pub const MAX_REPORTS: usize = 50;
const MAX_LINES: usize = 40;
const MAX_LINE: usize = 300;
/// Reports a minute from one person.
pub const PER_MINUTE: usize = 5;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Report {
    lines: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct DiagReport {
    pub at: i64,
    pub addr: String,
    /// Who sent it.
    pub user: String,
    pub lines: Vec<String>,
}

pub fn routes() -> Router<State> {
    Router::new().route("/", post(receive)).layer(DefaultBodyLimit::max(16 * 1024))
}

async fn receive(
    S(st): S<State>,
    Authed(s): Authed,
    Ip(ip): Ip,
    Json(b): Json<Report>,
) -> Result<StatusCode, ApiError> {
    if !st.diag_limit.allow(s.user, now()) {
        return Err(ApiError::too_many());
    }
    if b.lines.is_empty() || b.lines.len() > MAX_LINES {
        return Err(ApiError::bad("A report has 1 to 40 lines."));
    }
    let lines: Vec<String> = b.lines.iter().map(|l| clean_line(l, MAX_LINE)).collect();
    let uid = s.user;
    let user = st
        .db
        .run(move |c| Ok(store::user_by_id(c, uid)?.map(|u| u.username)))
        .await?
        .ok_or_else(ApiError::unauthenticated)?;
    // One line in the server log too, JSON-escaped.
    eprintln!("abshare: playback report from {user} at {ip}: {}", serde_json::to_string(&lines).unwrap_or_default());
    let report = DiagReport { at: now(), addr: ip.to_string(), user, lines };
    let mut q = st.diag.lock().map_err(|_| ApiError::internal("diagnostics lock"))?;
    if q.len() >= MAX_REPORTS {
        q.pop_front();
    }
    q.push_back(report);
    Ok(StatusCode::NO_CONTENT)
}

/// The kept reports, newest first.
pub fn recent(st: &State) -> Vec<DiagReport> {
    st.diag.lock().map(|q| q.iter().rev().cloned().collect()).unwrap_or_default()
}
