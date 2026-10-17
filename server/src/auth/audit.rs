//! The audit log (SPEC §5.5): append-only from the application's point of view, and
//! HMAC-chained (each row's MAC covers the previous row's MAC), so editing or deleting a row
//! is detectable. Kept for a year, at most [`MAX_ROWS`] rows; pruning re-anchors the chain.

use crate::db::sql_err;
use hmac::{Hmac, KeyInit, Mac};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::Sha256;

pub const MAX_ROWS: i64 = 100_000;
pub const MAX_AGE: i64 = 365 * 24 * 3600;
const PRUNE_EVERY: i64 = 1_000;

#[derive(Clone, Debug, Default)]
pub struct Entry<'a> {
    pub user_id: Option<i64>,
    pub device_id: Option<i64>,
    pub event: &'a str,
    pub detail: &'a str,
    pub ip: Option<&'a str>,
}

fn mac(key: &[u8; 32], prev: &[u8], at: i64, e: &Entry<'_>) -> Vec<u8> {
    let mut m = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC accepts any key length");
    m.update(prev);
    m.update(&at.to_be_bytes());
    m.update(&e.user_id.unwrap_or(-1).to_be_bytes());
    m.update(&e.device_id.unwrap_or(-1).to_be_bytes());
    for field in [e.event, e.detail, e.ip.unwrap_or("")] {
        m.update(&(field.len() as u64).to_be_bytes());
        m.update(field.as_bytes());
    }
    m.finalize().into_bytes().to_vec()
}

fn last_mac(conn: &Connection) -> Result<Vec<u8>, String> {
    if let Some(m) = conn
        .query_row("SELECT mac FROM audit ORDER BY id DESC LIMIT 1", [], |r| r.get(0))
        .optional()
        .map_err(sql_err)?
    {
        return Ok(m);
    }
    Ok(conn
        .query_row("SELECT mac FROM audit_anchor WHERE id = 1", [], |r| r.get(0))
        .optional()
        .map_err(sql_err)?
        .unwrap_or_else(|| vec![0u8; 32]))
}

/// Appends one entry. Long free-text fields are cut, so a flood of failed sign-ins can't
/// grow the log without bound (the row count is bounded by pruning too).
pub fn append(conn: &Connection, key: &[u8; 32], e: &Entry<'_>, now: i64) -> Result<(), String> {
    let detail: String = e.detail.chars().take(500).collect();
    let ip: Option<String> = e.ip.map(|s| s.chars().take(64).collect());
    let e = Entry { detail: &detail, ip: ip.as_deref(), ..e.clone() };
    let prev = last_mac(conn)?;
    let m = mac(key, &prev, now, &e);
    conn.execute(
        "INSERT INTO audit (at, user_id, device_id, event, detail, ip, mac) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![now, e.user_id, e.device_id, e.event, e.detail, e.ip, m],
    )
    .map_err(sql_err)?;
    let id = conn.last_insert_rowid();
    if id % PRUNE_EVERY == 0 {
        prune(conn, now)?;
    }
    Ok(())
}

/// Removes rows older than a year or beyond the newest [`MAX_ROWS`], keeping the MAC of the
/// last removed row as the new anchor.
pub fn prune(conn: &Connection, now: i64) -> Result<usize, String> {
    let max_id: i64 = conn.query_row("SELECT coalesce(max(id), 0) FROM audit", [], |r| r.get(0)).map_err(sql_err)?;
    let cut: Option<i64> = conn
        .query_row(
            "SELECT max(id) FROM audit WHERE at < ?1 OR id <= ?2",
            params![now - MAX_AGE, max_id - MAX_ROWS],
            |r| r.get(0),
        )
        .map_err(sql_err)?;
    let Some(cut) = cut else { return Ok(0) };
    let anchor: Vec<u8> =
        conn.query_row("SELECT mac FROM audit WHERE id = ?1", params![cut], |r| r.get(0)).map_err(sql_err)?;
    conn.execute(
        "INSERT INTO audit_anchor (id, mac) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET mac = excluded.mac",
        params![anchor],
    )
    .map_err(sql_err)?;
    conn.execute("DELETE FROM audit WHERE id <= ?1", params![cut]).map_err(sql_err)
}

/// Checks the whole chain. Returns the number of rows, or the ID of the first bad row.
pub fn verify(conn: &Connection, key: &[u8; 32]) -> Result<usize, i64> {
    let mut prev: Vec<u8> = conn
        .query_row("SELECT mac FROM audit_anchor WHERE id = 1", [], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
        .unwrap_or_else(|| vec![0u8; 32]);
    let Ok(mut st) = conn.prepare("SELECT id, at, user_id, device_id, event, detail, ip, mac FROM audit ORDER BY id")
    else {
        return Err(-1);
    };
    let Ok(rows) = st.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, Option<i64>>(2)?,
            r.get::<_, Option<i64>>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, Vec<u8>>(7)?,
        ))
    }) else {
        return Err(-1);
    };
    let mut n = 0;
    for row in rows {
        let Ok((id, at, user_id, device_id, event, detail, ip, stored)) = row else { return Err(-1) };
        let e = Entry { user_id, device_id, event: &event, detail: &detail, ip: ip.as_deref() };
        let want = mac(key, &prev, at, &e);
        if !bool::from(subtle::ConstantTimeEq::ct_eq(want.as_slice(), stored.as_slice())) {
            return Err(id);
        }
        prev = stored;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    const K: [u8; 32] = [3; 32];

    fn db() -> Connection {
        crate::db::open(std::path::Path::new(":memory:")).expect("db")
    }

    fn add(c: &Connection, n: i64, now: i64) {
        for i in 0..n {
            let d = format!("attempt {i}");
            append(
                c,
                &K,
                &Entry { event: "login.failed", detail: &d, ip: Some("203.0.113.9"), ..Entry::default() },
                now,
            )
            .expect("append");
        }
    }

    #[test]
    fn the_chain_verifies_and_detects_edits_and_deletions() {
        let c = db();
        add(&c, 10, 1000);
        assert_eq!(verify(&c, &K), Ok(10));
        c.execute("UPDATE audit SET detail = 'nothing happened' WHERE id = 4", []).expect("edit");
        assert_eq!(verify(&c, &K), Err(4));
        let c = db();
        add(&c, 10, 1000);
        c.execute("DELETE FROM audit WHERE id = 6", []).expect("delete");
        assert_eq!(verify(&c, &K), Err(7));
        assert_eq!(verify(&db(), &[4; 32]), Ok(0), "an empty log verifies");
    }

    #[test]
    fn a_different_key_fails_verification() {
        let c = db();
        add(&c, 3, 1000);
        assert_eq!(verify(&c, &[4; 32]), Err(1));
    }

    #[test]
    fn pruning_keeps_the_chain_verifiable() {
        let c = db();
        add(&c, 5, 1000);
        add(&c, 5, 1000 + MAX_AGE + 10);
        let removed = prune(&c, 1000 + MAX_AGE + 20).expect("prune");
        assert_eq!(removed, 5);
        assert_eq!(verify(&c, &K), Ok(5));
        add(&c, 2, 1000 + MAX_AGE + 30);
        assert_eq!(verify(&c, &K), Ok(7));
    }

    #[test]
    fn long_fields_are_cut() {
        let c = db();
        let long = "x".repeat(10_000);
        append(&c, &K, &Entry { event: "e", detail: &long, ..Entry::default() }, 1).expect("append");
        let len: i64 = c.query_row("SELECT length(detail) FROM audit", [], |r| r.get(0)).expect("row");
        assert_eq!(len, 500);
        assert_eq!(verify(&c, &K), Ok(1));
    }
}
