//! `abshare admin …` (SPEC §5.8): bootstrap and recovery from the command line, working on
//! the database directly. A running server notices revocations at the next refresh (access
//! tokens live at most 15 minutes).

use crate::auth::audit;
use crate::auth::crypto::new_token;
use crate::auth::store::{self, Perms, Role};
use crate::secret::{Secret, purpose};
use rusqlite::Connection;

pub const INVITE_HOURS: i64 = 72;

fn now() -> i64 {
    crate::http::now()
}

fn link(origin: &str, token: &str) -> String {
    format!("{}/#/invite/{token}", origin.trim_end_matches('/'))
}

/// Creates an invite and returns the link to send (the token itself is stored hashed).
pub fn invite(
    c: &Connection,
    secret: &Secret,
    origin: &str,
    role: Role,
    display: Option<&str>,
) -> Result<String, String> {
    let (token, hash) = new_token();
    store::create_invite(
        c,
        &hash,
        "enrol",
        None,
        None,
        display,
        role,
        &Perms::default(),
        now(),
        now() + INVITE_HOURS * 3600,
    )?;
    audit::append(
        c,
        &secret.derive(purpose::AUDIT),
        &audit::Entry {
            event: "invite.created",
            detail: &format!("role={} via cli", role.as_str()),
            ..audit::Entry::default()
        },
        now(),
    )?;
    Ok(link(origin, &token))
}

/// A link that lets `username` set up new sign-in methods (all old ones are removed and all
/// devices signed out when it is used).
pub fn reset(c: &Connection, secret: &Secret, origin: &str, username: &str) -> Result<String, String> {
    let user = store::user_by_name(c, username)?.ok_or_else(|| format!("no user {username}"))?;
    let (token, hash) = new_token();
    store::create_invite(
        c,
        &hash,
        "reset",
        Some(user.id),
        None,
        None,
        user.role,
        &user.perms,
        now(),
        now() + INVITE_HOURS * 3600,
    )?;
    audit::append(
        c,
        &secret.derive(purpose::AUDIT),
        &audit::Entry {
            user_id: Some(user.id),
            event: "factors.reset_issued",
            detail: "via cli",
            ..audit::Entry::default()
        },
        now(),
    )?;
    Ok(link(origin, &token))
}

pub fn users(c: &Connection) -> Result<String, String> {
    let mut out = String::new();
    for u in store::list_users(c)? {
        let devices = store::devices(c, u.id)?.len();
        let keys = store::passkeys(c, u.id)?.len();
        out.push_str(&format!(
            "{:<24} {:<8} passkeys={keys} password={} totp={} devices={devices}{}\n",
            u.username,
            u.role.as_str(),
            u.password_hash.is_some(),
            u.totp_secret.is_some(),
            if u.disabled { " DISABLED" } else { "" }
        ));
    }
    Ok(out)
}

pub fn devices(c: &Connection, username: &str) -> Result<String, String> {
    let user = store::user_by_name(c, username)?.ok_or_else(|| format!("no user {username}"))?;
    let mut out = String::new();
    for d in store::devices(c, user.id)? {
        out.push_str(&format!(
            "{:>5}  {:<24} last seen {}  {}\n",
            d.id,
            d.name,
            d.last_seen,
            d.last_ip.as_deref().unwrap_or("-")
        ));
    }
    Ok(out)
}

pub fn revoke_all(c: &Connection, secret: &Secret, username: &str) -> Result<usize, String> {
    let user = store::user_by_name(c, username)?.ok_or_else(|| format!("no user {username}"))?;
    let ids = store::revoke_all(c, user.id, None, now())?;
    audit::append(
        c,
        &secret.derive(purpose::AUDIT),
        &audit::Entry {
            user_id: Some(user.id),
            event: "devices.revoked_all",
            detail: "via cli",
            ..audit::Entry::default()
        },
        now(),
    )?;
    Ok(ids.len())
}

pub fn verify_audit(c: &Connection, secret: &Secret) -> Result<String, String> {
    match audit::verify(c, &secret.derive(purpose::AUDIT)) {
        Ok(n) => Ok(format!("audit log OK: {n} entries, chain intact")),
        Err(id) => Err(format!("audit log chain broken at entry {id}: it has been edited or rows removed")),
    }
}
