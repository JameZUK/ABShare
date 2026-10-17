//! `/v1/admin/*` (SPEC §7.2 Admin): users and invites, the library and its problems, the
//! audit log and the security status. Every route needs a signed-in admin (checked against
//! the database, not just the session); every change also needs a fresh step-up.

use super::{ApiError, Authed, Ip, State, now};
use crate::auth::audit;
use crate::auth::crypto::new_token;
use crate::auth::sessions::Session;
use crate::auth::store::{self, Perms, Role};
use axum::extract::{FromRequestParts, Path, Query, State as S};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use rusqlite::params;
use serde::Deserialize;
use serde_json::{Value, json};

pub fn routes() -> Router<State> {
    Router::new()
        .route("/users", get(users))
        .route("/users/{id}", get(user).patch(edit_user))
        .route("/users/{id}/reset", post(reset_user))
        .route("/users/{id}/sign-out", post(sign_out_user))
        .route("/invites", get(invites).post(create_invite))
        .route("/invites/{id}", delete(revoke_invite))
        .route("/audit", get(audit_log))
        .route("/library", get(library))
        .route("/library/scan", post(request_scan))
        .route("/status", get(status))
        .route("/books/{id}/details", get(book_details).put(set_book_details).delete(clear_book_details))
}

/// A signed-in admin whose account is still an enabled admin right now.
pub struct Admin(pub Session);

impl FromRequestParts<State> for Admin {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, st: &State) -> Result<Self, Self::Rejection> {
        let Authed(s) = Authed::from_request_parts(parts, st).await?;
        let uid = s.user;
        let ok = st
            .db
            .run(move |c| Ok(store::user_by_id(c, uid)?.is_some_and(|u| u.role == Role::Admin && !u.disabled)))
            .await?;
        if !ok {
            return Err(ApiError::new(StatusCode::FORBIDDEN, "not_permitted", "Only admins can do that."));
        }
        Ok(Self(s))
    }
}

fn step_up(st: &State, s: &Session) -> Result<(), ApiError> {
    if st.sessions.stepped_up(s.device, now()) { Ok(()) } else { Err(ApiError::step_up()) }
}

fn link(st: &State, token: &str) -> String {
    format!("{}/#/invite/{token}", st.guard.origin.trim_end_matches('/'))
}

/// Library permissions as sent by the admin UI: `"*"` or a list of library names.
fn libraries_value(v: &Value) -> Result<String, ApiError> {
    match v {
        Value::String(s) if s == "*" => Ok("*".into()),
        Value::Array(a) if a.len() <= 32 => {
            let names: Vec<String> = a
                .iter()
                .map(|x| x.as_str().map(str::trim).filter(|n| !n.is_empty() && n.len() <= 64).map(str::to_owned))
                .collect::<Option<_>>()
                .ok_or_else(|| ApiError::bad("Library names must be text."))?;
            serde_json::to_string(&names).map_err(ApiError::internal)
        }
        _ => Err(ApiError::bad("Libraries must be \"*\" or a list of names.")),
    }
}

fn libraries_json(s: &str) -> Value {
    if s.trim() == "*" { json!("*") } else { serde_json::from_str(s).unwrap_or_else(|_| json!([])) }
}

// ---------------------------------------------------------------- users

fn user_json(c: &rusqlite::Connection, u: &store::User) -> Result<Value, String> {
    let devices = store::devices(c, u.id)?;
    let last_seen = devices.iter().map(|d| d.last_seen).max();
    let created: i64 =
        c.query_row("SELECT created_at FROM user WHERE id = ?1", [u.id], |r| r.get(0)).map_err(crate::db::sql_err)?;
    Ok(json!({
        "id": u.id, "username": u.username, "display_name": u.display_name, "role": u.role.as_str(), "disabled": u.disabled,
        "factors": { "passkeys": store::passkeys(c, u.id)?.len(), "password": u.password_hash.is_some(), "totp": u.totp_secret.is_some(), "recovery_codes_left": store::recovery_left(c, u.id)? },
        "devices": devices.len(), "last_seen": last_seen, "created_at": created,
        "permissions": { "libraries": libraries_json(&u.perms.libraries), "offline": u.perms.can_offline, "save_files": u.perms.can_save },
    }))
}

async fn users(S(st): S<State>, Admin(_): Admin) -> Result<Json<Value>, ApiError> {
    let list =
        st.db.run(|c| store::list_users(c)?.iter().map(|u| user_json(c, u)).collect::<Result<Vec<_>, _>>()).await?;
    Ok(Json(json!({ "users": list })))
}

async fn user(S(st): S<State>, Admin(_): Admin, Path(id): Path<i64>) -> Result<Json<Value>, ApiError> {
    let v = st
        .db
        .run(move |c| {
            let Some(u) = store::user_by_id(c, id)? else { return Ok(None) };
            let mut v = user_json(c, &u)?;
            v["device_list"] = json!(
                store::devices(c, id)?
                    .iter()
                    .map(|d| json!({
                        "id": d.id, "name": d.name, "created_at": d.created_at, "last_seen": d.last_seen,
                    }))
                    .collect::<Vec<_>>()
            );
            Ok(Some(v))
        })
        .await?
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(v))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditUser {
    display_name: Option<String>,
    role: Option<String>,
    disabled: Option<bool>,
    libraries: Option<Value>,
    offline: Option<bool>,
    save_files: Option<bool>,
}

/// Active admins other than `except`.
fn other_admins(c: &rusqlite::Connection, except: i64) -> Result<i64, String> {
    c.query_row("SELECT count(*) FROM user WHERE role = 'admin' AND disabled_at IS NULL AND id != ?1", [except], |r| {
        r.get(0)
    })
    .map_err(crate::db::sql_err)
}

async fn edit_user(
    S(st): S<State>,
    Ip(ip): Ip,
    Admin(s): Admin,
    Path(id): Path<i64>,
    Json(b): Json<EditUser>,
) -> Result<Json<Value>, ApiError> {
    step_up(&st, &s)?;
    let role = b.role.as_deref().map(|r| match r {
        "admin" => Ok(Role::Admin),
        "listener" => Ok(Role::Listener),
        _ => Err(ApiError::bad("Unknown role.")),
    });
    let role = role.transpose()?;
    let libraries = b.libraries.as_ref().map(libraries_value).transpose()?;
    let display = b.display_name.as_deref().map(|d| abshare_core::auth::validate::label(d, ""));
    if display.as_deref() == Some("") {
        return Err(ApiError::bad("A display name can't be empty."));
    }
    if id == s.user && (b.disabled == Some(true) || role == Some(Role::Listener)) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "self",
            "You can't disable or demote yourself. Ask another admin.",
        ));
    }
    let now_ = now();
    let (outcome, changes) =
        (b.disabled, format!("{:?}", (&b.role, b.disabled, &b.libraries, b.offline, b.save_files)));
    let result = st
        .db
        .run(move |c| {
            let tx = c.transaction().map_err(crate::db::sql_err)?;
            let Some(u) = store::user_by_id(&tx, id)? else { return Ok(Err(ApiError::not_found())) };
            let losing_admin =
                u.role == Role::Admin && !u.disabled && (role == Some(Role::Listener) || outcome == Some(true));
            if losing_admin && other_admins(&tx, id)? == 0 {
                return Ok(Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "last_admin",
                    "There must always be at least one admin.",
                )));
            }
            if let Some(d) = &display {
                tx.execute("UPDATE user SET display_name = ?2 WHERE id = ?1", params![id, d])
                    .map_err(crate::db::sql_err)?;
            }
            if let Some(r) = role {
                tx.execute("UPDATE user SET role = ?2 WHERE id = ?1", params![id, r.as_str()])
                    .map_err(crate::db::sql_err)?;
            }
            if let Some(l) = &libraries {
                tx.execute("UPDATE user SET libraries = ?2 WHERE id = ?1", params![id, l])
                    .map_err(crate::db::sql_err)?;
            }
            if let Some(o) = b.offline {
                tx.execute("UPDATE user SET can_offline = ?2 WHERE id = ?1", params![id, o])
                    .map_err(crate::db::sql_err)?;
            }
            if let Some(sv) = b.save_files {
                tx.execute("UPDATE user SET can_save = ?2 WHERE id = ?1", params![id, sv])
                    .map_err(crate::db::sql_err)?;
            }
            let mut revoked = Vec::new();
            match outcome {
                Some(true) => {
                    tx.execute(
                        "UPDATE user SET disabled_at = ?2 WHERE id = ?1 AND disabled_at IS NULL",
                        params![id, now_],
                    )
                    .map_err(crate::db::sql_err)?;
                    revoked = store::revoke_all(&tx, id, None, now_)?;
                }
                Some(false) => {
                    tx.execute("UPDATE user SET disabled_at = NULL WHERE id = ?1", [id]).map_err(crate::db::sql_err)?;
                }
                None => {}
            }
            tx.commit().map_err(crate::db::sql_err)?;
            Ok(Ok(revoked))
        })
        .await??;
    for d in result {
        st.sessions.revoke_device(d);
    }
    // A changed role or library access applies to new tokens: end the user's current ones so
    // nothing runs on the old permissions (devices stay signed in and refresh).
    st.sessions.revoke_user(id);
    st.audit("admin.user_changed", Some(s.user), Some(s.device), Some(ip), format!("user={id} {changes}")).await;
    let v = st.db.run(move |c| store::user_by_id(c, id)?.map(|u| user_json(c, &u)).transpose()).await?;
    Ok(Json(v.unwrap_or(Value::Null)))
}

async fn reset_user(
    S(st): S<State>,
    Ip(ip): Ip,
    Admin(s): Admin,
    Path(id): Path<i64>,
) -> Result<Json<Value>, ApiError> {
    step_up(&st, &s)?;
    let (token, hash) = new_token();
    let by = s.user;
    let created = st
        .db
        .run(move |c| {
            let Some(u) = store::user_by_id(c, id)? else { return Ok(false) };
            store::create_invite(
                c,
                &hash,
                "reset",
                Some(u.id),
                Some(by),
                None,
                u.role,
                &u.perms,
                now(),
                now() + 72 * 3600,
            )?;
            Ok(true)
        })
        .await?;
    if !created {
        return Err(ApiError::not_found());
    }
    st.audit("factors.reset_issued", Some(s.user), Some(s.device), Some(ip), format!("for user={id}")).await;
    Ok(Json(json!({ "link": link(&st, &token), "expires_in_hours": 72 })))
}

async fn sign_out_user(
    S(st): S<State>,
    Ip(ip): Ip,
    Admin(s): Admin,
    Path(id): Path<i64>,
) -> Result<Json<Value>, ApiError> {
    step_up(&st, &s)?;
    let keep = (id == s.user).then_some(s.device);
    let revoked = st.db.run(move |c| store::revoke_all(c, id, keep, now())).await?;
    for d in &revoked {
        st.sessions.revoke_device(*d);
    }
    st.audit("devices.revoked_all", Some(s.user), Some(s.device), Some(ip), format!("for user={id} by admin")).await;
    Ok(Json(json!({ "signed_out": revoked.len() })))
}

// ---------------------------------------------------------------- invites

async fn invites(S(st): S<State>, Admin(_): Admin) -> Result<Json<Value>, ApiError> {
    let list = st
        .db
        .run(|c| {
            let mut q = c
                .prepare(
                    "SELECT i.rowid, i.purpose, i.display_name, i.role, i.created_at, i.expires_at, u.username, i.libraries, i.can_offline, i.can_save
                     FROM invite i LEFT JOIN user u ON u.id = i.for_user
                     WHERE i.used_at IS NULL AND i.expires_at > ?1 ORDER BY i.created_at DESC LIMIT 200",
                )
                .map_err(crate::db::sql_err)?;
            let rows = q
                .query_map([now()], |r| {
                    Ok(json!({
                        "id": r.get::<_, i64>(0)?, "purpose": r.get::<_, String>(1)?, "display_name": r.get::<_, Option<String>>(2)?,
                        "role": r.get::<_, String>(3)?, "created_at": r.get::<_, i64>(4)?, "expires_at": r.get::<_, i64>(5)?,
                        "for_user": r.get::<_, Option<String>>(6)?, "libraries": libraries_json(&r.get::<_, String>(7)?),
                        "offline": r.get::<_, bool>(8)?, "save_files": r.get::<_, bool>(9)?,
                    }))
                })
                .map_err(crate::db::sql_err)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(crate::db::sql_err)
        })
        .await?;
    Ok(Json(json!({ "invites": list })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewInvite {
    display_name: Option<String>,
    role: Option<String>,
    /// Inviting an admin needs this second confirmation (SPEC §5.5).
    confirm_admin: Option<bool>,
    libraries: Option<Value>,
    offline: Option<bool>,
    save_files: Option<bool>,
    expires_hours: Option<i64>,
}

async fn create_invite(
    S(st): S<State>,
    Ip(ip): Ip,
    Admin(s): Admin,
    Json(b): Json<NewInvite>,
) -> Result<Json<Value>, ApiError> {
    step_up(&st, &s)?;
    let role = match b.role.as_deref().unwrap_or("listener") {
        "listener" => Role::Listener,
        "admin" if b.confirm_admin == Some(true) => Role::Admin,
        "admin" => {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "confirm_admin",
                "Confirm that this person should be an admin.",
            ));
        }
        _ => return Err(ApiError::bad("Unknown role.")),
    };
    let hours = b.expires_hours.unwrap_or(72);
    if !(1..=168).contains(&hours) {
        return Err(ApiError::bad("Invites last between 1 hour and 7 days."));
    }
    let perms = Perms {
        can_offline: b.offline.unwrap_or(true),
        can_save: b.save_files.unwrap_or(false),
        libraries: b.libraries.as_ref().map(libraries_value).transpose()?.unwrap_or_else(|| "*".into()),
    };
    let display =
        b.display_name.as_deref().map(|d| abshare_core::auth::validate::label(d, "")).filter(|d| !d.is_empty());
    let (token, hash) = new_token();
    let by = s.user;
    st.db
        .run(move |c| {
            store::create_invite(
                c,
                &hash,
                "enrol",
                None,
                Some(by),
                display.as_deref(),
                role,
                &perms,
                now(),
                now() + hours * 3600,
            )
        })
        .await?;
    st.audit("invite.created", Some(s.user), Some(s.device), Some(ip), format!("role={} hours={hours}", role.as_str()))
        .await;
    Ok(Json(json!({ "link": link(&st, &token), "expires_in_hours": hours })))
}

async fn revoke_invite(
    S(st): S<State>,
    Ip(ip): Ip,
    Admin(s): Admin,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    step_up(&st, &s)?;
    let n = st
        .db
        .run(move |c| {
            c.execute(
                "UPDATE invite SET expires_at = ?2 WHERE rowid = ?1 AND used_at IS NULL AND expires_at > ?2",
                params![id, now()],
            )
            .map_err(crate::db::sql_err)
        })
        .await?;
    if n != 1 {
        return Err(ApiError::not_found());
    }
    st.audit("invite.revoked", Some(s.user), Some(s.device), Some(ip), format!("invite={id}")).await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- audit log

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuditQuery {
    before: Option<i64>,
    limit: Option<i64>,
}

async fn audit_log(S(st): S<State>, Admin(_): Admin, Query(q): Query<AuditQuery>) -> Result<Json<Value>, ApiError> {
    let limit = q.limit.unwrap_or(100).clamp(1, 200);
    let before = q.before.unwrap_or(i64::MAX);
    let key = st.audit_key;
    let (rows, chain) = st
        .db
        .run(move |c| {
            let mut stmt = c
                .prepare(
                    "SELECT a.id, a.at, u.username, a.device_id, a.event, a.detail, a.ip FROM audit a
                     LEFT JOIN user u ON u.id = a.user_id WHERE a.id < ?1 ORDER BY a.id DESC LIMIT ?2",
                )
                .map_err(crate::db::sql_err)?;
            let rows = stmt
                .query_map(params![before, limit], |r| {
                    Ok(json!({
                        "id": r.get::<_, i64>(0)?, "at": r.get::<_, i64>(1)?, "user": r.get::<_, Option<String>>(2)?,
                        "device": r.get::<_, Option<i64>>(3)?, "event": r.get::<_, String>(4)?, "detail": r.get::<_, String>(5)?,
                        "ip": r.get::<_, Option<String>>(6)?,
                    }))
                })
                .map_err(crate::db::sql_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(crate::db::sql_err)?;
            let chain = match audit::verify(c, &key) {
                Ok(n) => json!({ "intact": true, "entries": n }),
                Err(at) => json!({ "intact": false, "broken_at": at }),
            };
            Ok((rows, chain))
        })
        .await?;
    let next = rows.last().and_then(|r| r["id"].as_i64()).filter(|_| rows.len() as i64 == limit);
    Ok(Json(json!({ "entries": rows, "next": next, "chain": chain })))
}

// ---------------------------------------------------------------- library and scans

/// Problems that need someone to act, as opposed to information (ignored duplicates).
fn needs_attention(kind: &str) -> bool {
    !matches!(kind, "duplicate" | "duplicate ASIN" | "skipped")
}

async fn library(S(st): S<State>, Admin(_): Admin) -> Result<Json<Value>, ApiError> {
    let cat = st.catalog().await?;
    let problems: Vec<(String, String, String, String)> = st
        .db
        .run(|c| {
            let mut q = c
                .prepare(
                    "SELECT l.name, p.rel_path, p.kind, p.detail FROM problem p JOIN library l ON l.id = p.library_id
                     ORDER BY p.kind, p.rel_path LIMIT 5000",
                )
                .map_err(crate::db::sql_err)?;
            let rows =
                q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(crate::db::sql_err)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(crate::db::sql_err)
        })
        .await?;
    let item =
        |(l, p, k, d): &(String, String, String, String)| json!({ "library": l, "path": p, "kind": k, "detail": d });
    let attention: Vec<Value> = problems.iter().filter(|p| needs_attention(&p.2)).map(item).collect();
    let info: Vec<Value> = problems.iter().filter(|p| !needs_attention(&p.2)).map(item).collect();
    let libraries: Vec<Value> = cat
        .libraries
        .iter()
        .map(|(id, name)| {
            let books: Vec<_> = cat.books.iter().filter(|b| b.library_id == *id).collect();
            json!({
                "id": id, "name": name, "books": books.len(),
                "needs_copy": books.iter().filter(|b| b.needs_copy).count(),
                "covers": books.iter().filter(|b| b.cover.is_some()).count(),
                "groups": cat.groups.iter().filter(|g| g.library_id == *id).count(),
            })
        })
        .collect();
    let data_dir = st.media.as_ref().map(|m| m.data_dir.clone());
    let last_scan = match &data_dir {
        Some(d) => {
            tokio::fs::read(d.join("last-scan.json")).await.ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        }
        None => None,
    };
    let requested = match &data_dir {
        Some(d) => tokio::fs::try_exists(d.join("scan-requested")).await.unwrap_or(false),
        None => false,
    };
    Ok(Json(json!({
        "libraries": libraries, "last_scan": last_scan, "scan_requested": requested,
        "problems": { "attention": attention, "info": info },
    })))
}

/// Asks the scanner (`abshare scan --every`) to run now, through a file in the data
/// directory: the web server itself never walks the library.
async fn request_scan(S(st): S<State>, Ip(ip): Ip, Admin(s): Admin) -> Result<StatusCode, ApiError> {
    let dir = st.media.as_ref().map(|m| m.data_dir.clone()).ok_or_else(ApiError::not_found)?;
    tokio::fs::write(dir.join("scan-requested"), b"1").await.map_err(ApiError::internal)?;
    st.audit("scan.requested", Some(s.user), Some(s.device), Some(ip), String::new()).await;
    Ok(StatusCode::ACCEPTED)
}

// ---------------------------------------------------------------- book details (overrides)

/// The current correction for a book, if any (the book page shows the merged result).
async fn book_details(S(st): S<State>, Admin(_): Admin, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let cat = st.catalog().await?;
    if !abshare_core::ids::is_valid_id(&id) || !cat.by_id.contains_key(&id) {
        return Err(ApiError::not_found());
    }
    let data: Option<String> = st
        .db
        .run(move |c| {
            use rusqlite::OptionalExtension;
            c.query_row("SELECT data FROM book_override WHERE book_id = ?1", [&id], |r| r.get(0))
                .optional()
                .map_err(crate::db::sql_err)
        })
        .await?;
    let ov = data.and_then(|d| serde_json::from_str::<Value>(&d).ok()).unwrap_or_else(|| json!({}));
    Ok(Json(json!({ "override": ov })))
}

/// Saves a correction (replacing any earlier one); an empty one removes it.
async fn set_book_details(
    S(st): S<State>,
    Ip(ip): Ip,
    Admin(s): Admin,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let cat = st.catalog().await?;
    if !abshare_core::ids::is_valid_id(&id) || !cat.by_id.contains_key(&id) {
        return Err(ApiError::not_found());
    }
    let o = abshare_core::metadata_override::Override::parse(&b).map_err(ApiError::bad)?;
    let stored = o.to_json();
    let (bid, by, data, empty) = (id.clone(), s.user, stored.to_string(), o.is_empty());
    st.db
        .run(move |c| {
            if empty {
                c.execute("DELETE FROM book_override WHERE book_id = ?1", [&bid]).map_err(crate::db::sql_err)?;
            } else {
                c.execute(
                    "INSERT INTO book_override (book_id, data, updated_at, updated_by) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(book_id) DO UPDATE SET data = excluded.data, updated_at = excluded.updated_at, updated_by = excluded.updated_by",
                    params![bid, data, now(), by],
                )
                .map_err(crate::db::sql_err)?;
            }
            Ok(())
        })
        .await?;
    st.invalidate_catalog().await;
    st.audit("book.details_changed", Some(s.user), Some(s.device), Some(ip), format!("book={id}")).await;
    Ok(Json(json!({ "override": stored })))
}

async fn clear_book_details(
    S(st): S<State>,
    Ip(ip): Ip,
    Admin(s): Admin,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if !abshare_core::ids::is_valid_id(&id) {
        return Err(ApiError::not_found());
    }
    let bid = id.clone();
    st.db
        .run(move |c| c.execute("DELETE FROM book_override WHERE book_id = ?1", [&bid]).map_err(crate::db::sql_err))
        .await?;
    st.invalidate_catalog().await;
    st.audit("book.details_reset", Some(s.user), Some(s.device), Some(ip), format!("book={id}")).await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- security status

async fn status(S(st): S<State>, Admin(_): Admin) -> Result<Json<Value>, ApiError> {
    let (users, admins, disabled, weak, pending_invites): (i64, i64, i64, Vec<String>, i64) = st
        .db
        .run(|c| {
            let count = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0)).map_err(crate::db::sql_err);
            let users = count("SELECT count(*) FROM user WHERE disabled_at IS NULL")?;
            let admins = count("SELECT count(*) FROM user WHERE role = 'admin' AND disabled_at IS NULL")?;
            let disabled = count("SELECT count(*) FROM user WHERE disabled_at IS NOT NULL")?;
            let invites = c
                .query_row("SELECT count(*) FROM invite WHERE used_at IS NULL AND expires_at > ?1", [now()], |r| {
                    r.get(0)
                })
                .map_err(crate::db::sql_err)?;
            // Accounts with no recovery codes left can be locked out by one lost device.
            let mut weak = Vec::new();
            for u in store::list_users(c)?.into_iter().filter(|u| !u.disabled) {
                if store::recovery_left(c, u.id)? == 0 {
                    weak.push(u.username);
                }
            }
            Ok((users, admins, disabled, weak, invites))
        })
        .await?;
    let landlock = tokio::fs::read_to_string("/sys/kernel/security/lsm")
        .await
        .is_ok_and(|s| s.split(',').any(|m| m.trim() == "landlock"));
    let mut warnings: Vec<String> = Vec::new();
    if !landlock {
        warnings
            .push("Landlock is not active on this kernel: ffmpeg can't be sandboxed, so media jobs will fail.".into());
    }
    if st.guard.trusted.is_empty() {
        warnings.push(
            "No trusted proxy is configured: client addresses for rate limits and lockouts come from the connection."
                .into(),
        );
    }
    if admins < 2 {
        warnings.push("Only one admin: if that account is lost, recovery needs the command line.".into());
    }
    for u in &weak {
        warnings.push(format!("{u} has no recovery codes left."));
    }
    let cache = match st.media.as_ref() {
        Some(m) => {
            let d = m.data_dir.clone();
            tokio::task::spawn_blocking(move || crate::media::cache_usage(&d)).await.unwrap_or(0)
        }
        None => 0,
    };
    Ok(Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "users": users, "admins": admins, "disabled": disabled, "pending_invites": pending_invites,
        // Every account needs a passkey or a password with an authenticator app (D2).
        "two_step_coverage": 1.0,
        "sandbox": { "landlock": landlock },
        "cache_bytes": cache,
        "warnings": warnings,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_permissions_are_checked() {
        assert_eq!(libraries_value(&json!("*")).expect("all"), "*");
        assert_eq!(libraries_value(&json!(["Audiobooks", " Kids "])).expect("list"), r#"["Audiobooks","Kids"]"#);
        for bad in
            [json!("all"), json!([1]), json!([""]), json!({ "a": 1 }), json!(["x".repeat(65)]), json!(vec!["a"; 33])]
        {
            assert!(libraries_value(&bad).is_err(), "{bad}");
        }
        assert_eq!(libraries_json("*"), json!("*"));
        assert_eq!(libraries_json(r#"["A"]"#), json!(["A"]));
        assert_eq!(libraries_json("garbage"), json!([]));
    }

    #[test]
    fn only_actionable_problems_need_attention() {
        assert!(needs_attention("unsupported codec"));
        assert!(needs_attention("cover"));
        assert!(!needs_attention("duplicate"));
        assert!(!needs_attention("duplicate ASIN"));
    }
}
