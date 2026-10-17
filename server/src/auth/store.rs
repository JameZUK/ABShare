//! Account storage (SPEC §5.5). Synchronous functions on a connection; the HTTP layer calls
//! them on a blocking thread. Secrets are stored only as hashes (tokens, invites, recovery
//! codes) or encrypted (TOTP seeds, by the caller).

use crate::db::sql_err;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

pub const REFRESH_LIFETIME: i64 = 90 * 24 * 3600;
pub const ABSOLUTE_LIFETIME: i64 = 90 * 24 * 3600;
pub const MAX_SIBLINGS: i64 = 8;
pub const MAX_RENEWALS_PER_MINUTE: i64 = 30;
pub const MAX_DEVICES: i64 = 20;
pub const MAX_NEW_DEVICES_PER_HOUR: i64 = 10;
pub const REVOKED_KEPT: i64 = 30 * 24 * 3600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Admin,
    Listener,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Listener => "listener",
        }
    }

    fn parse(s: &str) -> Self {
        if s == "admin" { Self::Admin } else { Self::Listener }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Perms {
    pub can_offline: bool,
    pub can_save: bool,
    /// `*` for all libraries, otherwise a JSON array of library names.
    pub libraries: String,
}

impl Default for Perms {
    fn default() -> Self {
        Self { can_offline: true, can_save: false, libraries: "*".into() }
    }
}

#[derive(Clone, Debug)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub webauthn_id: String,
    pub display_name: String,
    pub role: Role,
    pub password_hash: Option<String>,
    pub totp_secret: Option<Vec<u8>>,
    pub totp_last_step: Option<i64>,
    pub perms: Perms,
    pub disabled: bool,
}

const USER_COLS: &str = "id, username, webauthn_id, display_name, role, password_hash, totp_secret, totp_last_step, can_offline, can_save, libraries, disabled_at";

fn user_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: r.get(0)?,
        username: r.get(1)?,
        webauthn_id: r.get(2)?,
        display_name: r.get(3)?,
        role: Role::parse(&r.get::<_, String>(4)?),
        password_hash: r.get(5)?,
        totp_secret: r.get(6)?,
        totp_last_step: r.get(7)?,
        perms: Perms { can_offline: r.get(8)?, can_save: r.get(9)?, libraries: r.get(10)? },
        disabled: r.get::<_, Option<i64>>(11)?.is_some(),
    })
}

pub fn user_by_name(c: &Connection, username: &str) -> Result<Option<User>, String> {
    c.query_row(&format!("SELECT {USER_COLS} FROM user WHERE username = ?1"), params![username], user_row)
        .optional()
        .map_err(sql_err)
}

pub fn user_by_id(c: &Connection, id: i64) -> Result<Option<User>, String> {
    c.query_row(&format!("SELECT {USER_COLS} FROM user WHERE id = ?1"), params![id], user_row)
        .optional()
        .map_err(sql_err)
}

pub fn list_users(c: &Connection) -> Result<Vec<User>, String> {
    let mut st = c.prepare(&format!("SELECT {USER_COLS} FROM user ORDER BY username")).map_err(sql_err)?;
    st.query_map([], user_row).map_err(sql_err)?.collect::<Result<_, _>>().map_err(sql_err)
}

/// Everything needed to create an account in one step at the end of enrolment.
pub struct NewUser<'a> {
    pub username: &'a str,
    pub webauthn_id: &'a str,
    pub display_name: &'a str,
    pub role: Role,
    pub perms: &'a Perms,
    pub password_hash: Option<&'a str>,
}

pub fn insert_user(tx: &Transaction<'_>, u: &NewUser<'_>, now: i64) -> Result<i64, String> {
    tx.execute(
        "INSERT INTO user (username, webauthn_id, display_name, role, password_hash, can_offline, can_save, libraries, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            u.username,
            u.webauthn_id,
            u.display_name,
            u.role.as_str(),
            u.password_hash,
            u.perms.can_offline,
            u.perms.can_save,
            u.perms.libraries,
            now
        ],
    )
    .map_err(sql_err)?;
    Ok(tx.last_insert_rowid())
}

pub fn set_password(c: &Connection, user: i64, hash: Option<&str>) -> Result<(), String> {
    c.execute("UPDATE user SET password_hash = ?2 WHERE id = ?1", params![user, hash]).map_err(sql_err).map(drop)
}

pub fn set_totp(c: &Connection, user: i64, sealed: Option<&[u8]>, last_step: Option<i64>) -> Result<(), String> {
    c.execute("UPDATE user SET totp_secret = ?2, totp_last_step = ?3 WHERE id = ?1", params![user, sealed, last_step])
        .map_err(sql_err)
        .map(drop)
}

/// Records a used TOTP step, only if it is newer than the stored one (so two racing
/// requests can't both use the same code).
pub fn use_totp_step(c: &Connection, user: i64, step: i64) -> Result<bool, String> {
    let n = c
        .execute(
            "UPDATE user SET totp_last_step = ?2 WHERE id = ?1 AND (totp_last_step IS NULL OR totp_last_step < ?2)",
            params![user, step],
        )
        .map_err(sql_err)?;
    Ok(n == 1)
}

// --- Recovery codes

pub fn replace_recovery(c: &Connection, user: i64, hashes: &[[u8; 32]]) -> Result<(), String> {
    c.execute("DELETE FROM recovery_code WHERE user_id = ?1", params![user]).map_err(sql_err)?;
    for h in hashes {
        c.execute("INSERT INTO recovery_code (user_id, hash) VALUES (?1, ?2)", params![user, h.as_slice()])
            .map_err(sql_err)?;
    }
    Ok(())
}

/// Uses a recovery code once. True if it was valid and unused.
pub fn use_recovery(c: &Connection, user: i64, hash: &[u8; 32], now: i64) -> Result<bool, String> {
    let n = c
        .execute(
            "UPDATE recovery_code SET used_at = ?3 WHERE user_id = ?1 AND hash = ?2 AND used_at IS NULL",
            params![user, hash.as_slice(), now],
        )
        .map_err(sql_err)?;
    Ok(n == 1)
}

pub fn recovery_left(c: &Connection, user: i64) -> Result<i64, String> {
    c.query_row("SELECT count(*) FROM recovery_code WHERE user_id = ?1 AND used_at IS NULL", params![user], |r| {
        r.get(0)
    })
    .map_err(sql_err)
}

// --- Passkeys

pub struct PasskeyRow {
    pub id: i64,
    pub name: String,
    pub data: String,
    pub created_at: i64,
    pub last_used: Option<i64>,
}

pub fn add_passkey(c: &Connection, user: i64, name: &str, cred_id: &[u8], data: &str, now: i64) -> Result<i64, String> {
    let n: i64 =
        c.query_row("SELECT count(*) FROM passkey WHERE user_id = ?1", params![user], |r| r.get(0)).map_err(sql_err)?;
    if n >= 10 {
        return Err("a user may have at most 10 passkeys".into());
    }
    c.execute(
        "INSERT INTO passkey (user_id, name, cred_id, data, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![user, name, cred_id, data, now],
    )
    .map_err(sql_err)?;
    Ok(c.last_insert_rowid())
}

pub fn passkeys(c: &Connection, user: i64) -> Result<Vec<PasskeyRow>, String> {
    let mut st = c
        .prepare("SELECT id, name, data, created_at, last_used FROM passkey WHERE user_id = ?1 ORDER BY id")
        .map_err(sql_err)?;
    st.query_map(params![user], |r| {
        Ok(PasskeyRow { id: r.get(0)?, name: r.get(1)?, data: r.get(2)?, created_at: r.get(3)?, last_used: r.get(4)? })
    })
    .map_err(sql_err)?
    .collect::<Result<_, _>>()
    .map_err(sql_err)
}

pub fn passkey_by_cred(c: &Connection, cred_id: &[u8]) -> Result<Option<(i64, i64, String)>, String> {
    c.query_row("SELECT id, user_id, data FROM passkey WHERE cred_id = ?1", params![cred_id], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })
    .optional()
    .map_err(sql_err)
}

pub fn update_passkey(c: &Connection, id: i64, data: &str, now: i64) -> Result<(), String> {
    c.execute("UPDATE passkey SET data = ?2, last_used = ?3 WHERE id = ?1", params![id, data, now])
        .map_err(sql_err)
        .map(drop)
}

pub fn delete_passkey(c: &Connection, user: i64, id: i64) -> Result<bool, String> {
    Ok(c.execute("DELETE FROM passkey WHERE id = ?1 AND user_id = ?2", params![id, user]).map_err(sql_err)? == 1)
}

// --- Invites

pub struct Invite {
    pub purpose: String,
    pub for_user: Option<i64>,
    pub display_name: Option<String>,
    pub role: Role,
    pub perms: Perms,
    pub expires_at: i64,
}

#[allow(clippy::too_many_arguments)]
pub fn create_invite(
    c: &Connection,
    hash: &[u8; 32],
    purpose: &str,
    for_user: Option<i64>,
    created_by: Option<i64>,
    display_name: Option<&str>,
    role: Role,
    perms: &Perms,
    now: i64,
    expires_at: i64,
) -> Result<(), String> {
    c.execute(
        "INSERT INTO invite (hash, purpose, for_user, created_by, display_name, role, can_offline, can_save, libraries, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            hash.as_slice(),
            purpose,
            for_user,
            created_by,
            display_name,
            role.as_str(),
            perms.can_offline,
            perms.can_save,
            perms.libraries,
            now,
            expires_at
        ],
    )
    .map_err(sql_err)
    .map(drop)
}

/// An unused, unexpired invite (does not consume it).
pub fn find_invite(c: &Connection, hash: &[u8; 32], now: i64) -> Result<Option<Invite>, String> {
    c.query_row(
        "SELECT purpose, for_user, display_name, role, can_offline, can_save, libraries, expires_at FROM invite
         WHERE hash = ?1 AND used_at IS NULL AND expires_at > ?2",
        params![hash.as_slice(), now],
        |r| {
            Ok(Invite {
                purpose: r.get(0)?,
                for_user: r.get(1)?,
                display_name: r.get(2)?,
                role: Role::parse(&r.get::<_, String>(3)?),
                perms: Perms { can_offline: r.get(4)?, can_save: r.get(5)?, libraries: r.get(6)? },
                expires_at: r.get(7)?,
            })
        },
    )
    .optional()
    .map_err(sql_err)
}

/// Marks an invite used, only if it still is unused and unexpired.
pub fn consume_invite(tx: &Transaction<'_>, hash: &[u8; 32], now: i64) -> Result<bool, String> {
    let n = tx
        .execute(
            "UPDATE invite SET used_at = ?2 WHERE hash = ?1 AND used_at IS NULL AND expires_at > ?2",
            params![hash.as_slice(), now],
        )
        .map_err(sql_err)?;
    Ok(n == 1)
}

// --- Devices and refresh tokens

pub struct Device {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
    pub last_seen: i64,
    pub last_ip: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DeviceError {
    TooManyNew,
    Db(String),
}

/// Creates a device for a new sign-in: at most 10 new devices an hour per user; beyond 20
/// active devices the least recently seen one is signed out.
pub fn create_device(
    tx: &Transaction<'_>,
    user: i64,
    name: &str,
    ip: Option<&str>,
    ua: Option<&str>,
    now: i64,
) -> Result<i64, DeviceError> {
    let db = |e: rusqlite::Error| DeviceError::Db(sql_err(e));
    let recent: i64 = tx
        .query_row(
            "SELECT count(*) FROM device WHERE user_id = ?1 AND created_at > ?2",
            params![user, now - 3600],
            |r| r.get(0),
        )
        .map_err(db)?;
    if recent >= MAX_NEW_DEVICES_PER_HOUR {
        return Err(DeviceError::TooManyNew);
    }
    tx.execute(
        "UPDATE device SET revoked_at = ?2, last_ip = NULL, user_agent = NULL WHERE id IN (
           SELECT id FROM device WHERE user_id = ?1 AND revoked_at IS NULL ORDER BY last_seen DESC LIMIT -1 OFFSET ?3)",
        params![user, now, MAX_DEVICES - 1],
    )
    .map_err(db)?;
    tx.execute("DELETE FROM device WHERE revoked_at IS NOT NULL AND revoked_at < ?1", params![now - REVOKED_KEPT])
        .map_err(db)?;
    let ua: Option<String> = ua.map(|s| s.chars().filter(|c| !c.is_control()).take(200).collect());
    tx.execute(
        "INSERT INTO device (user_id, name, created_at, session_started, last_seen, last_ip, user_agent) VALUES (?1, ?2, ?3, ?3, ?3, ?4, ?5)",
        params![user, name, now, ip, ua],
    )
    .map_err(db)?;
    Ok(tx.last_insert_rowid())
}

fn device_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Device> {
    Ok(Device {
        id: r.get(0)?,
        name: r.get(1)?,
        created_at: r.get(2)?,
        last_seen: r.get(3)?,
        last_ip: r.get(4)?,
        user_agent: r.get(5)?,
    })
}

pub fn devices(c: &Connection, user: i64) -> Result<Vec<Device>, String> {
    let mut st = c
        .prepare(
            "SELECT id, name, created_at, last_seen, last_ip, user_agent FROM device
             WHERE user_id = ?1 AND revoked_at IS NULL ORDER BY last_seen DESC",
        )
        .map_err(sql_err)?;
    st.query_map(params![user], device_row).map_err(sql_err)?.collect::<Result<_, _>>().map_err(sql_err)
}

/// Revokes one of `user`'s devices. False if it isn't theirs (or already revoked).
pub fn revoke_device(c: &Connection, user: i64, device: i64, now: i64) -> Result<bool, String> {
    let n = c
        .execute(
            "UPDATE device SET revoked_at = ?3, last_ip = NULL, user_agent = NULL WHERE id = ?1 AND user_id = ?2 AND revoked_at IS NULL",
            params![device, user, now],
        )
        .map_err(sql_err)?;
    Ok(n == 1)
}

/// Revokes all of a user's devices except `keep`. Returns the revoked IDs.
pub fn revoke_all(c: &Connection, user: i64, keep: Option<i64>, now: i64) -> Result<Vec<i64>, String> {
    let ids: Vec<i64> = {
        let mut st = c.prepare("SELECT id FROM device WHERE user_id = ?1 AND revoked_at IS NULL").map_err(sql_err)?;
        st.query_map(params![user], |r| r.get(0)).map_err(sql_err)?.collect::<Result<_, _>>().map_err(sql_err)?
    };
    let ids: Vec<i64> = ids.into_iter().filter(|&id| Some(id) != keep).collect();
    for id in &ids {
        c.execute(
            "UPDATE device SET revoked_at = ?2, last_ip = NULL, user_agent = NULL WHERE id = ?1",
            params![id, now],
        )
        .map_err(sql_err)?;
    }
    Ok(ids)
}

pub fn issue_refresh(
    c: &Connection,
    device: i64,
    hash: &[u8; 32],
    parent: Option<&[u8; 32]>,
    now: i64,
) -> Result<(), String> {
    c.execute(
        "INSERT INTO refresh (hash, device_id, parent, issued_at) VALUES (?1, ?2, ?3, ?4)",
        params![hash.as_slice(), device, parent.map(|p| p.as_slice()), now],
    )
    .map_err(sql_err)
    .map(drop)
}

#[derive(Debug, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// Rotated: the caller hands out the new token (already stored).
    Rotated {
        device: i64,
        user: i64,
    },
    Unknown,
    /// Session past its lifetime (idle or absolute): sign in again; downloads stay.
    Ended,
    /// The device was signed out remotely (or its user disabled): the app also removes this
    /// device's downloads (SPEC §8.2).
    Revoked,
    /// A used token was presented outside the grace: theft is assumed and the device revoked.
    Reused {
        device: i64,
        user: i64,
    },
    /// More than 30 renewals in a minute for this device.
    TooFast,
}

/// Rotates refresh token `old` to `new` (SPEC §5.5, D21). Runs in one transaction.
pub fn rotate_refresh(
    tx: &Transaction<'_>,
    old: &[u8; 32],
    new: &[u8; 32],
    now: i64,
) -> Result<RefreshOutcome, String> {
    // (device, user, used_at, issued_at, session_started, revoked_at, parent)
    type Row = (i64, i64, Option<i64>, i64, i64, Option<i64>, Option<Vec<u8>>);
    let row: Option<Row> = tx
        .query_row(
            "SELECT r.device_id, d.user_id, r.used_at, r.issued_at, d.session_started, d.revoked_at, r.parent
             FROM refresh r JOIN device d ON d.id = r.device_id WHERE r.hash = ?1",
            params![old.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .optional()
        .map_err(sql_err)?;
    let Some((device, user, used_at, issued_at, started, revoked, parent)) = row else {
        return Ok(RefreshOutcome::Unknown);
    };
    if revoked.is_some() {
        return Ok(RefreshOutcome::Revoked);
    }
    if issued_at + REFRESH_LIFETIME <= now || started + ABSOLUTE_LIFETIME <= now {
        return Ok(RefreshOutcome::Ended);
    }
    let disabled: Option<i64> =
        tx.query_row("SELECT disabled_at FROM user WHERE id = ?1", params![user], |r| r.get(0)).map_err(sql_err)?;
    if disabled.is_some() {
        return Ok(RefreshOutcome::Revoked);
    }
    let renewals: i64 = tx
        .query_row(
            "SELECT count(*) FROM refresh WHERE device_id = ?1 AND issued_at > ?2",
            params![device, now - 60],
            |r| r.get(0),
        )
        .map_err(sql_err)?;
    if renewals >= MAX_RENEWALS_PER_MINUTE {
        return Ok(RefreshOutcome::TooFast);
    }
    let theft = |tx: &Transaction<'_>| -> Result<RefreshOutcome, String> {
        tx.execute(
            "UPDATE device SET revoked_at = ?2, last_ip = NULL, user_agent = NULL WHERE id = ?1",
            params![device, now],
        )
        .map_err(sql_err)?;
        Ok(RefreshOutcome::Reused { device, user })
    };
    if used_at.is_some() {
        // A spent token presented again. Fine while none of its replacements has been used
        // (the response was lost, or two tabs raced): issue another replacement, a bounded
        // number of times. Once a replacement has been used, this is a copy: theft.
        let (children, used_children): (i64, i64) = tx
            .query_row("SELECT count(*), count(used_at) FROM refresh WHERE parent = ?1", params![old.as_slice()], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(sql_err)?;
        if used_children > 0 || children >= MAX_SIBLINGS {
            return theft(tx);
        }
    } else {
        // A fresh token whose sibling (another replacement of the same parent) has already
        // been used was superseded: someone else holds the live chain (review A1).
        if let Some(p) = &parent {
            let used_siblings: i64 = tx
                .query_row(
                    "SELECT count(*) FROM refresh WHERE parent = ?1 AND hash != ?2 AND used_at IS NOT NULL",
                    params![p, old.as_slice()],
                    |r| r.get(0),
                )
                .map_err(sql_err)?;
            if used_siblings > 0 {
                return theft(tx);
            }
        }
        tx.execute("UPDATE refresh SET used_at = ?2 WHERE hash = ?1", params![old.as_slice(), now]).map_err(sql_err)?;
    }
    issue_refresh(tx, device, new, Some(old), now)?;
    tx.execute("UPDATE device SET last_seen = ?2 WHERE id = ?1", params![device, now]).map_err(sql_err)?;
    // Spent tokens are kept for the session, so an old copy is always recognised as reuse;
    // only tokens past their lifetime (which no longer work anyway) are removed.
    tx.execute("DELETE FROM refresh WHERE device_id = ?1 AND issued_at < ?2", params![device, now - REFRESH_LIFETIME])
        .map_err(sql_err)?;
    Ok(RefreshOutcome::Rotated { device, user })
}

#[cfg(test)]
pub fn device_active(c: &Connection, device: i64) -> Result<bool, String> {
    let r: Option<Option<i64>> = c
        .query_row("SELECT revoked_at FROM device WHERE id = ?1", params![device], |r| r.get(0))
        .optional()
        .map_err(sql_err)?;
    Ok(matches!(r, Some(None)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        crate::db::open(std::path::Path::new(":memory:")).expect("db")
    }

    fn user(c: &mut Connection, name: &str) -> i64 {
        let tx = c.transaction().expect("tx");
        let id = insert_user(
            &tx,
            &NewUser {
                username: name,
                webauthn_id: &format!("w-{name}"),
                display_name: name,
                role: Role::Listener,
                perms: &Perms::default(),
                password_hash: None,
            },
            1,
        )
        .expect("user");
        tx.commit().expect("commit");
        id
    }

    fn device(c: &mut Connection, u: i64, now: i64) -> i64 {
        let tx = c.transaction().expect("tx");
        let d = create_device(&tx, u, "Phone", Some("203.0.113.1"), Some("UA"), now).expect("device");
        tx.commit().expect("commit");
        d
    }

    fn rotate(c: &mut Connection, old: [u8; 32], new: [u8; 32], now: i64) -> RefreshOutcome {
        let tx = c.transaction().expect("tx");
        let o = rotate_refresh(&tx, &old, &new, now).expect("rotate");
        tx.commit().expect("commit");
        o
    }

    #[test]
    fn refresh_rotates_and_reuse_after_grace_revokes_the_device() {
        let mut c = db();
        let u = user(&mut c, "ada");
        let d = device(&mut c, u, 1000);
        issue_refresh(&c, d, &[1; 32], None, 1000).expect("issue");
        assert_eq!(rotate(&mut c, [1; 32], [2; 32], 1010), RefreshOutcome::Rotated { device: d, user: u });
        // Grace: the same old token again within 60 s (lost response) gets a sibling.
        assert_eq!(rotate(&mut c, [1; 32], [3; 32], 1020), RefreshOutcome::Rotated { device: d, user: u });
        // A replacement is used...
        assert_eq!(rotate(&mut c, [2; 32], [4; 32], 1030), RefreshOutcome::Rotated { device: d, user: u });
        // ...so the old token presented again is theft: the device is revoked for everyone.
        assert_eq!(rotate(&mut c, [1; 32], [5; 32], 1040), RefreshOutcome::Reused { device: d, user: u });
        assert!(!device_active(&c, d).expect("active"));
        assert_eq!(rotate(&mut c, [4; 32], [6; 32], 1050), RefreshOutcome::Revoked);
    }

    /// Review A1: a renewal whose response was lost (a tunnel) can be retried much later
    /// without being taken for theft.
    #[test]
    fn a_lost_response_can_be_retried_long_after() {
        let mut c = db();
        let u = user(&mut c, "lia");
        let d = device(&mut c, u, 1000);
        issue_refresh(&c, d, &[1; 32], None, 1000).expect("issue");
        assert_eq!(rotate(&mut c, [1; 32], [2; 32], 1000), RefreshOutcome::Rotated { device: d, user: u });
        // The response with [2] never arrived; the app retries with [1] an hour later.
        assert_eq!(rotate(&mut c, [1; 32], [3; 32], 4600), RefreshOutcome::Rotated { device: d, user: u });
        assert!(device_active(&c, d).expect("active"));
        // And it goes on normally from the new token.
        assert_eq!(rotate(&mut c, [3; 32], [4; 32], 4700), RefreshOutcome::Rotated { device: d, user: u });
    }

    /// Review A1: an old stolen token is still recognised as reuse long after it was spent
    /// (it used to be deleted after two minutes and come back as merely "unknown").
    #[test]
    fn an_old_spent_token_is_always_recognised_as_reuse() {
        let mut c = db();
        let u = user(&mut c, "mo");
        let d = device(&mut c, u, 1000);
        issue_refresh(&c, d, &[1; 32], None, 1000).expect("issue");
        rotate(&mut c, [1; 32], [2; 32], 1000);
        rotate(&mut c, [2; 32], [3; 32], 2000);
        rotate(&mut c, [3; 32], [4; 32], 3000);
        assert_eq!(rotate(&mut c, [1; 32], [9; 32], 90_000), RefreshOutcome::Reused { device: d, user: u });
        assert!(!device_active(&c, d).expect("active"));
    }

    /// Review A1: siblings issued for a retry can't run on side by side: once one is used,
    /// using another is theft.
    #[test]
    fn a_sibling_chain_is_detected_once_both_are_used() {
        let mut c = db();
        let u = user(&mut c, "ned");
        let d = device(&mut c, u, 1000);
        issue_refresh(&c, d, &[1; 32], None, 1000).expect("issue");
        rotate(&mut c, [1; 32], [2; 32], 1000); // the owner
        rotate(&mut c, [1; 32], [3; 32], 1010); // a thief with a copy of [1], before [2] is used
        assert_eq!(rotate(&mut c, [2; 32], [4; 32], 1020), RefreshOutcome::Rotated { device: d, user: u });
        assert_eq!(rotate(&mut c, [3; 32], [5; 32], 1030), RefreshOutcome::Reused { device: d, user: u });
    }

    #[test]
    fn a_retry_is_allowed_until_a_replacement_is_used_then_it_is_theft() {
        let mut c = db();
        let u = user(&mut c, "bea");
        let d = device(&mut c, u, 1000);
        issue_refresh(&c, d, &[1; 32], None, 1000).expect("issue");
        rotate(&mut c, [1; 32], [2; 32], 1000);
        assert_eq!(rotate(&mut c, [1; 32], [3; 32], 1000 + 3600), RefreshOutcome::Rotated { device: d, user: u });
        assert_eq!(rotate(&mut c, [3; 32], [4; 32], 1000 + 3700), RefreshOutcome::Rotated { device: d, user: u });
        assert_eq!(rotate(&mut c, [1; 32], [5; 32], 1000 + 3800), RefreshOutcome::Reused { device: d, user: u });
    }

    #[test]
    fn unknown_expired_and_absolute_lifetime() {
        let mut c = db();
        let u = user(&mut c, "cai");
        let d = device(&mut c, u, 1000);
        assert_eq!(rotate(&mut c, [9; 32], [8; 32], 1000), RefreshOutcome::Unknown);
        issue_refresh(&c, d, &[1; 32], None, 1000).expect("issue");
        assert_eq!(rotate(&mut c, [1; 32], [2; 32], 1000 + REFRESH_LIFETIME), RefreshOutcome::Ended);
        // Even with continuous refresh, the session ends at its absolute lifetime.
        let mut t = 1000;
        let mut cur = [50u8; 32];
        let d2 = device(&mut c, u, 1000);
        issue_refresh(&c, d2, &cur, None, 1000).expect("issue");
        let mut last = RefreshOutcome::Unknown;
        while t < 1000 + ABSOLUTE_LIFETIME + 86400 {
            t += 7 * 86400;
            let mut next = cur;
            next[0] = next[0].wrapping_add(1);
            next[1] = (t / 86400) as u8;
            last = rotate(&mut c, cur, next, t);
            if last != (RefreshOutcome::Rotated { device: d2, user: u }) {
                break;
            }
            cur = next;
        }
        assert_eq!(last, RefreshOutcome::Ended);
    }

    #[test]
    fn a_remotely_revoked_device_is_told_so_but_expiry_is_not_revocation() {
        let mut c = db();
        let u = user(&mut c, "eve");
        let d = device(&mut c, u, 1000);
        issue_refresh(&c, d, &[1; 32], None, 1000).expect("issue");
        assert!(revoke_device(&c, u, d, 1001).expect("revoke"));
        assert_eq!(rotate(&mut c, [1; 32], [2; 32], 1002), RefreshOutcome::Revoked);
        let d2 = device(&mut c, u, 1000);
        issue_refresh(&c, d2, &[3; 32], None, 1000).expect("issue");
        assert_eq!(rotate(&mut c, [3; 32], [4; 32], 1000 + REFRESH_LIFETIME), RefreshOutcome::Ended);
    }

    #[test]
    fn renewals_are_rate_limited() {
        let mut c = db();
        let u = user(&mut c, "dee");
        let d = device(&mut c, u, 1000);
        issue_refresh(&c, d, &[0; 32], None, 1000).expect("issue");
        let mut cur = [0u8; 32];
        let mut outcome = RefreshOutcome::Unknown;
        for i in 1..40u8 {
            let next = [i; 32];
            outcome = rotate(&mut c, cur, next, 1001);
            if outcome == RefreshOutcome::TooFast {
                break;
            }
            cur = next;
        }
        assert_eq!(outcome, RefreshOutcome::TooFast);
    }

    #[test]
    fn device_limits() {
        let mut c = db();
        let u = user(&mut c, "eve");
        for i in 0..10 {
            device(&mut c, u, 1000 + i);
        }
        let tx = c.transaction().expect("tx");
        assert_eq!(create_device(&tx, u, "x", None, None, 1100), Err(DeviceError::TooManyNew));
        drop(tx);
        // Over 20 active devices across days: the least recently seen is signed out.
        for day in 1..4 {
            for i in 0..10 {
                device(&mut c, u, day * 86400 + i);
            }
        }
        assert!(devices(&c, u).expect("list").len() <= MAX_DEVICES as usize);
    }

    #[test]
    fn a_user_cannot_revoke_another_users_device() {
        let mut c = db();
        let a = user(&mut c, "fay");
        let b = user(&mut c, "gus");
        let db_dev = device(&mut c, b, 1000);
        assert!(!revoke_device(&c, a, db_dev, 1001).expect("revoke"));
        assert!(device_active(&c, db_dev).expect("active"));
        assert!(revoke_device(&c, b, db_dev, 1001).expect("revoke"));
    }

    #[test]
    fn invites_are_single_use_and_expire() {
        let mut c = db();
        create_invite(&c, &[7; 32], "enrol", None, None, Some("Ada"), Role::Listener, &Perms::default(), 1000, 2000)
            .expect("create");
        assert!(find_invite(&c, &[7; 32], 1500).expect("find").is_some());
        assert!(find_invite(&c, &[7; 32], 2000).expect("find").is_none(), "expired");
        let tx = c.transaction().expect("tx");
        assert!(consume_invite(&tx, &[7; 32], 1500).expect("consume"));
        assert!(!consume_invite(&tx, &[7; 32], 1501).expect("consume twice"));
        tx.commit().expect("commit");
        assert!(find_invite(&c, &[7; 32], 1600).expect("find").is_none(), "used");
    }

    #[test]
    fn recovery_codes_and_totp_steps_are_single_use() {
        let mut c = db();
        let u = user(&mut c, "hal");
        replace_recovery(&c, u, &[[1; 32], [2; 32]]).expect("codes");
        assert!(use_recovery(&c, u, &[1; 32], 10).expect("use"));
        assert!(!use_recovery(&c, u, &[1; 32], 11).expect("reuse"));
        assert!(!use_recovery(&c, u, &[9; 32], 11).expect("wrong"));
        assert_eq!(recovery_left(&c, u).expect("left"), 1);
        assert!(use_totp_step(&c, u, 100).expect("step"));
        assert!(!use_totp_step(&c, u, 100).expect("same step"));
        assert!(!use_totp_step(&c, u, 99).expect("older step"));
        assert!(use_totp_step(&c, u, 101).expect("newer step"));
    }
}
