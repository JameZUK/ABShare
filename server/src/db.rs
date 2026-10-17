//! The SQLite database (`abshare.db`): opening, pragmas and numbered migrations tracked in
//! `PRAGMA user_version`. All values are bound as parameters everywhere.

use rusqlite::Connection;
use std::path::Path;

pub const MIGRATIONS: &[&str] = &[
    // 1: catalogue.
    "CREATE TABLE library (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL UNIQUE,
        root TEXT NOT NULL
    );
    CREATE TABLE file (
        library_id INTEGER NOT NULL REFERENCES library(id),
        rel_path TEXT NOT NULL,
        size INTEGER NOT NULL,
        mtime_ns INTEGER NOT NULL,
        probe TEXT NOT NULL,
        probed_at INTEGER NOT NULL,
        PRIMARY KEY (library_id, rel_path)
    );
    CREATE TABLE book (
        id TEXT PRIMARY KEY,
        library_id INTEGER NOT NULL REFERENCES library(id),
        folder TEXT NOT NULL,
        meta TEXT NOT NULL,
        duration_ms INTEGER NOT NULL,
        needs_copy INTEGER NOT NULL,
        cover TEXT,
        missing_since INTEGER,
        updated_at INTEGER NOT NULL
    );
    CREATE INDEX book_library ON book(library_id);
    CREATE TABLE book_part (
        book_id TEXT NOT NULL REFERENCES book(id) ON DELETE CASCADE,
        ord INTEGER NOT NULL,
        rel_path TEXT NOT NULL,
        PRIMARY KEY (book_id, ord)
    );
    CREATE TABLE chapter (
        book_id TEXT NOT NULL REFERENCES book(id) ON DELETE CASCADE,
        ord INTEGER NOT NULL,
        start_ms INTEGER NOT NULL,
        end_ms INTEGER NOT NULL,
        title TEXT NOT NULL,
        PRIMARY KEY (book_id, ord)
    );
    CREATE TABLE grp (
        id TEXT PRIMARY KEY,
        library_id INTEGER NOT NULL REFERENCES library(id),
        name TEXT NOT NULL,
        qualifier TEXT,
        kind TEXT NOT NULL
    );
    CREATE TABLE grp_member (
        grp_id TEXT NOT NULL REFERENCES grp(id) ON DELETE CASCADE,
        ord INTEGER NOT NULL,
        book_id TEXT NOT NULL,
        extra INTEGER NOT NULL,
        PRIMARY KEY (grp_id, ord)
    );
    CREATE TABLE problem (
        library_id INTEGER NOT NULL REFERENCES library(id),
        rel_path TEXT NOT NULL,
        kind TEXT NOT NULL,
        detail TEXT NOT NULL
    );",
    // 2: where each book's cover came from, so unchanged covers are not re-extracted.
    "ALTER TABLE book ADD COLUMN cover_src TEXT;",
    // 3: accounts, factors, invites, devices, refresh tokens, audit (SPEC 5.5).
    "CREATE TABLE user (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        username TEXT NOT NULL UNIQUE,
        webauthn_id TEXT NOT NULL UNIQUE,
        display_name TEXT NOT NULL,
        role TEXT NOT NULL CHECK (role IN ('admin', 'listener')),
        password_hash TEXT,
        totp_secret BLOB,
        totp_last_step INTEGER,
        can_offline INTEGER NOT NULL DEFAULT 1,
        can_save INTEGER NOT NULL DEFAULT 0,
        libraries TEXT NOT NULL DEFAULT '*',
        created_at INTEGER NOT NULL,
        disabled_at INTEGER
    );
    CREATE TABLE passkey (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        user_id INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        cred_id BLOB NOT NULL UNIQUE,
        data TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        last_used INTEGER
    );
    CREATE TABLE recovery_code (
        user_id INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
        hash BLOB NOT NULL,
        used_at INTEGER,
        PRIMARY KEY (user_id, hash)
    );
    CREATE TABLE invite (
        hash BLOB PRIMARY KEY,
        purpose TEXT NOT NULL CHECK (purpose IN ('enrol', 'reset')),
        for_user INTEGER REFERENCES user(id) ON DELETE CASCADE,
        created_by INTEGER,
        display_name TEXT,
        role TEXT NOT NULL CHECK (role IN ('admin', 'listener')),
        can_offline INTEGER NOT NULL,
        can_save INTEGER NOT NULL,
        libraries TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        expires_at INTEGER NOT NULL,
        used_at INTEGER
    );
    CREATE TABLE device (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        user_id INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        session_started INTEGER NOT NULL,
        last_seen INTEGER NOT NULL,
        last_ip TEXT,
        user_agent TEXT,
        revoked_at INTEGER
    );
    CREATE INDEX device_user ON device(user_id);
    CREATE TABLE refresh (
        hash BLOB PRIMARY KEY,
        device_id INTEGER NOT NULL REFERENCES device(id) ON DELETE CASCADE,
        parent BLOB,
        issued_at INTEGER NOT NULL,
        used_at INTEGER
    );
    CREATE INDEX refresh_device ON refresh(device_id);
    CREATE INDEX refresh_parent ON refresh(parent);
    CREATE TABLE audit (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        at INTEGER NOT NULL,
        user_id INTEGER,
        device_id INTEGER,
        event TEXT NOT NULL,
        detail TEXT NOT NULL,
        ip TEXT,
        mac BLOB NOT NULL
    );
    CREATE TABLE audit_anchor (id INTEGER PRIMARY KEY CHECK (id = 1), mac BLOB NOT NULL);",
    // 4: listening (SPEC 5.6): when books arrived, progress, idempotency keys, bookmarks.
    "ALTER TABLE book ADD COLUMN added_at INTEGER;
    UPDATE book SET added_at = updated_at WHERE added_at IS NULL;
    CREATE TABLE progress (
        user_id INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
        book_id TEXT NOT NULL,
        position_ms INTEGER NOT NULL,
        duration_ms INTEGER NOT NULL,
        finished INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        received_at INTEGER NOT NULL,
        device_id INTEGER,
        PRIMARY KEY (user_id, book_id)
    );
    CREATE INDEX progress_recent ON progress(user_id, received_at);
    CREATE TABLE progress_key (
        user_id INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
        key TEXT NOT NULL,
        at INTEGER NOT NULL,
        PRIMARY KEY (user_id, key)
    );
    CREATE TABLE bookmark (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        user_id INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
        book_id TEXT NOT NULL,
        position_ms INTEGER NOT NULL,
        note TEXT NOT NULL DEFAULT '',
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE INDEX bookmark_user_book ON bookmark(user_id, book_id);",
];

pub fn sql_err(e: rusqlite::Error) -> String {
    format!("database: {e}")
}

/// Opens the database, applies pragmas and runs any pending migrations.
pub fn open(path: &Path) -> Result<Connection, String> {
    let mut conn = Connection::open(path).map_err(sql_err)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA secure_delete = ON; PRAGMA busy_timeout = 5000;",
    )
    .map_err(sql_err)?;
    let version: usize = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).map_err(sql_err)? as usize;
    if version > MIGRATIONS.len() {
        return Err(format!("database schema {version} is newer than this ABShare ({})", MIGRATIONS.len()));
    }
    for (i, m) in MIGRATIONS.iter().enumerate().skip(version) {
        let tx = conn.transaction().map_err(sql_err)?;
        tx.execute_batch(m).map_err(sql_err)?;
        tx.execute_batch(&format!("PRAGMA user_version = {}", i + 1)).map_err(sql_err)?;
        tx.commit().map_err(sql_err)?;
    }
    Ok(conn)
}
