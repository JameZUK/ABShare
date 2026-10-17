//! Start-up reports from the web app (`static/diag.js`). When the app is left showing a blank
//! page, the browser posts what it saw, so an admin can find out why without the person's
//! phone in hand.
//!
//! This route is anonymous by necessity: the app didn't start, so nobody is signed in. It is
//! kept small: same-origin only (the CSRF guard: the app's header and an exact `Origin`), a
//! strict schema of at most 40 cleaned lines of 300 characters, 5 reports a minute per
//! address, and storage in memory only (the newest 50), so it can't be used to fill a disk.
//! Admins read them at `GET /v1/admin/diagnostics`.

use super::{ApiError, Ip, State, now};
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
/// Reports a minute from one address (IPv6 by /64).
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
    pub lines: Vec<String>,
}

pub fn routes() -> Router<State> {
    Router::new().route("/", post(receive)).layer(DefaultBodyLimit::max(16 * 1024))
}

async fn receive(S(st): S<State>, Ip(ip): Ip, Json(b): Json<Report>) -> Result<StatusCode, ApiError> {
    if !st.diag_limit.allow(ip.into(), now()) {
        return Err(ApiError::too_many());
    }
    if b.lines.is_empty() || b.lines.len() > MAX_LINES {
        return Err(ApiError::bad("A report has 1 to 40 lines."));
    }
    let lines: Vec<String> = b.lines.iter().map(|l| clean_line(l, MAX_LINE)).collect();
    // One line in the server log too, JSON-escaped.
    eprintln!("abshare: start-up report from {ip}: {}", serde_json::to_string(&lines).unwrap_or_default());
    let report = DiagReport { at: now(), addr: ip.to_string(), lines };
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
