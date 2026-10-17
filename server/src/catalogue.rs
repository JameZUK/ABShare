//! The library catalogue in SQLite (`abshare.db`). Schema changes go through numbered
//! migrations tracked in `PRAGMA user_version`. All values are bound as parameters.

use crate::probe::ProbeRecord;
use abshare_core::chapters::Chapter;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub struct Catalogue {
    conn: Connection,
}

pub struct BookRow {
    pub id: String,
    pub folder: String,
    pub meta_json: String,
    pub duration_ms: u64,
    pub needs_copy: bool,
    pub cover: Option<String>,
    /// The cover's source fingerprint (path, size, mtime).
    pub cover_src: Option<String>,
    pub parts: Vec<String>,
    pub chapters: Vec<Chapter>,
}

pub struct GroupRow {
    pub id: String,
    pub name: String,
    pub qualifier: Option<String>,
    pub kind: &'static str,
    /// (book id, is extra)
    pub members: Vec<(String, bool)>,
}

pub struct ProblemRow {
    pub rel_path: String,
    pub kind: &'static str,
    pub detail: String,
}

pub struct StoredFile {
    pub size: u64,
    pub mtime_ns: i64,
    pub probe: ProbeRecord,
}

fn sql_err(e: rusqlite::Error) -> String {
    format!("database: {e}")
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

impl Catalogue {
    pub fn open(path: &Path) -> Result<Self, String> {
        Ok(Self { conn: crate::db::open(path)? })
    }

    pub fn library_id(&self, name: &str, root: &str) -> Result<i64, String> {
        self.conn
            .execute(
                "INSERT INTO library (name, root) VALUES (?1, ?2) ON CONFLICT(name) DO UPDATE SET root = excluded.root",
                params![name, root],
            )
            .map_err(sql_err)?;
        self.conn.query_row("SELECT id FROM library WHERE name = ?1", params![name], |r| r.get(0)).map_err(sql_err)
    }

    pub fn files(&self, lib: i64) -> Result<HashMap<String, StoredFile>, String> {
        let mut st = self
            .conn
            .prepare("SELECT rel_path, size, mtime_ns, probe FROM file WHERE library_id = ?1")
            .map_err(sql_err)?;
        let rows = st
            .query_map(params![lib], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?))
            })
            .map_err(sql_err)?;
        let mut out = HashMap::new();
        for row in rows {
            let (rel, size, mtime_ns, probe) = row.map_err(sql_err)?;
            // A record that no longer parses (older format) is simply probed again.
            if let Ok(probe) = serde_json::from_str(&probe) {
                out.insert(rel, StoredFile { size: size.max(0) as u64, mtime_ns, probe });
            }
        }
        Ok(out)
    }

    pub fn upsert_file(
        &self,
        lib: i64,
        rel: &str,
        size: u64,
        mtime_ns: i64,
        probe: &ProbeRecord,
        now: i64,
    ) -> Result<(), String> {
        let json = serde_json::to_string(probe).map_err(|e| e.to_string())?;
        self.conn
            .execute(
                "INSERT INTO file (library_id, rel_path, size, mtime_ns, probe, probed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(library_id, rel_path) DO UPDATE SET size = excluded.size, mtime_ns = excluded.mtime_ns,
                 probe = excluded.probe, probed_at = excluded.probed_at",
                params![lib, rel, to_i64(size), mtime_ns, json, now],
            )
            .map_err(sql_err)?;
        Ok(())
    }

    /// Each book's current cover hash and the source it was made from.
    pub fn covers(&self, lib: i64) -> Result<HashMap<String, (String, String)>, String> {
        let mut st = self
            .conn
            .prepare("SELECT id, cover, cover_src FROM book WHERE library_id = ?1 AND cover IS NOT NULL AND cover_src IS NOT NULL")
            .map_err(sql_err)?;
        let rows = st
            .query_map(params![lib], |r| Ok((r.get::<_, String>(0)?, (r.get::<_, String>(1)?, r.get::<_, String>(2)?))))
            .map_err(sql_err)?;
        rows.collect::<Result<_, _>>().map_err(sql_err)
    }

    pub fn forget_files_not_in(&mut self, lib: i64, keep: &HashSet<String>) -> Result<usize, String> {
        let existing: Vec<String> = {
            let mut st = self.conn.prepare("SELECT rel_path FROM file WHERE library_id = ?1").map_err(sql_err)?;
            st.query_map(params![lib], |r| r.get(0)).map_err(sql_err)?.collect::<Result<_, _>>().map_err(sql_err)?
        };
        let tx = self.conn.transaction().map_err(sql_err)?;
        let mut n = 0;
        for rel in existing.iter().filter(|r| !keep.contains(*r)) {
            n += tx
                .execute("DELETE FROM file WHERE library_id = ?1 AND rel_path = ?2", params![lib, rel])
                .map_err(sql_err)?;
        }
        tx.commit().map_err(sql_err)?;
        Ok(n)
    }

    /// Admin corrections by book ID (unreadable rows are skipped).
    pub fn overrides(
        &self,
    ) -> Result<std::collections::HashMap<String, abshare_core::metadata_override::Override>, String> {
        let mut q = self.conn.prepare("SELECT book_id, data FROM book_override").map_err(sql_err)?;
        let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(sql_err)?;
        let mut out = std::collections::HashMap::new();
        for row in rows {
            let (id, data) = row.map_err(sql_err)?;
            if let Some(o) =
                serde_json::from_str(&data).ok().and_then(|v| abshare_core::metadata_override::Override::parse(&v).ok())
            {
                out.insert(id, o);
            }
        }
        Ok(out)
    }

    /// Replaces the library's books, groups and problems in one transaction. Books that
    /// disappeared are kept but marked missing (progress and bookmarks survive a remount).
    /// IDs of present books that belong to libraries other than `lib`.
    pub fn ids_elsewhere(&self, lib: i64) -> Result<HashSet<String>, String> {
        let mut st = self
            .conn
            .prepare("SELECT id FROM book WHERE library_id != ?1 AND missing_since IS NULL")
            .map_err(sql_err)?;
        st.query_map(params![lib], |r| r.get(0)).map_err(sql_err)?.collect::<Result<_, _>>().map_err(sql_err)
    }

    /// Returns (books written, books now missing, books whose audio changed: their playable
    /// copies and data-saver segments are stale).
    pub fn replace_library(
        &mut self,
        lib: i64,
        books: &[BookRow],
        groups: &[GroupRow],
        problems: &[ProblemRow],
        now: i64,
    ) -> Result<(usize, usize, Vec<String>), String> {
        let tx = self.conn.transaction().map_err(sql_err)?;
        let mut changed = Vec::new();
        let present: HashSet<&str> = books.iter().map(|b| b.id.as_str()).collect();
        let existing: Vec<String> = {
            let mut st =
                tx.prepare("SELECT id FROM book WHERE library_id = ?1 AND missing_since IS NULL").map_err(sql_err)?;
            st.query_map(params![lib], |r| r.get(0)).map_err(sql_err)?.collect::<Result<_, _>>().map_err(sql_err)?
        };
        let mut gone = 0;
        for id in existing.iter().filter(|id| !present.contains(id.as_str())) {
            gone +=
                tx.execute("UPDATE book SET missing_since = ?2 WHERE id = ?1", params![id, now]).map_err(sql_err)?;
        }
        for b in books {
            // The audio this book is made from, before this scan: parts and length.
            let before: Option<(i64, i64)> = tx
                .query_row("SELECT duration_ms, library_id FROM book WHERE id = ?1", params![b.id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()
                .map_err(sql_err)?;
            if let Some((dur, was_lib)) = before {
                let parts: Vec<String> = {
                    let mut st = tx
                        .prepare("SELECT rel_path FROM book_part WHERE book_id = ?1 ORDER BY ord")
                        .map_err(sql_err)?;
                    st.query_map(params![b.id], |r| r.get(0))
                        .map_err(sql_err)?
                        .collect::<Result<_, _>>()
                        .map_err(sql_err)?
                };
                if dur != to_i64(b.duration_ms) || was_lib != lib || parts != b.parts {
                    changed.push(b.id.clone());
                }
            }
            tx.execute(
                "INSERT INTO book (id, library_id, folder, meta, duration_ms, needs_copy, cover, cover_src, missing_since, updated_at, added_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?9, NULL, ?8, ?8)
                 ON CONFLICT(id) DO UPDATE SET library_id = excluded.library_id, folder = excluded.folder,
                 meta = excluded.meta, duration_ms = excluded.duration_ms, needs_copy = excluded.needs_copy,
                 cover = excluded.cover, cover_src = excluded.cover_src, missing_since = NULL,
                 updated_at = excluded.updated_at",
                params![b.id, lib, b.folder, b.meta_json, to_i64(b.duration_ms), b.needs_copy, b.cover, now, b.cover_src],
            )
            .map_err(sql_err)?;
            tx.execute("DELETE FROM book_part WHERE book_id = ?1", params![b.id]).map_err(sql_err)?;
            for (i, p) in b.parts.iter().enumerate() {
                tx.execute(
                    "INSERT INTO book_part (book_id, ord, rel_path) VALUES (?1, ?2, ?3)",
                    params![b.id, i as i64, p],
                )
                .map_err(sql_err)?;
            }
            tx.execute("DELETE FROM chapter WHERE book_id = ?1", params![b.id]).map_err(sql_err)?;
            for (i, c) in b.chapters.iter().enumerate() {
                tx.execute(
                    "INSERT INTO chapter (book_id, ord, start_ms, end_ms, title) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![b.id, i as i64, to_i64(c.start_ms), to_i64(c.end_ms), c.title],
                )
                .map_err(sql_err)?;
            }
        }
        tx.execute("DELETE FROM grp WHERE library_id = ?1", params![lib]).map_err(sql_err)?;
        for g in groups {
            tx.execute(
                "INSERT INTO grp (id, library_id, name, qualifier, kind) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![g.id, lib, g.name, g.qualifier, g.kind],
            )
            .map_err(sql_err)?;
            for (i, (book, extra)) in g.members.iter().enumerate() {
                tx.execute(
                    "INSERT INTO grp_member (grp_id, ord, book_id, extra) VALUES (?1, ?2, ?3, ?4)",
                    params![g.id, i as i64, book, extra],
                )
                .map_err(sql_err)?;
            }
        }
        tx.execute("DELETE FROM problem WHERE library_id = ?1", params![lib]).map_err(sql_err)?;
        for p in problems {
            tx.execute(
                "INSERT INTO problem (library_id, rel_path, kind, detail) VALUES (?1, ?2, ?3, ?4)",
                params![lib, p.rel_path, p.kind, p.detail],
            )
            .map_err(sql_err)?;
        }
        tx.commit().map_err(sql_err)?;
        Ok((books.len(), gone, changed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec() -> ProbeRecord {
        ProbeRecord {
            container: "mov".into(),
            codec: "aac".into(),
            profile: "LC".into(),
            duration_ms: 1000,
            bitrate: 64_000,
            sample_rate: 22_050,
            channels: 2,
            tags: vec![("title".into(), "Robert'); DROP TABLE book;--".into())],
            chapters: vec![],
            has_cover: false,
            decodes: true,
            problem: None,
        }
    }

    fn book(id: &str) -> BookRow {
        BookRow {
            id: id.into(),
            folder: "F".into(),
            meta_json: "{}".into(),
            duration_ms: 1000,
            needs_copy: false,
            cover: None,
            cover_src: None,
            parts: vec!["F/a.m4b".into()],
            chapters: vec![Chapter { start_ms: 0, end_ms: 1000, title: "One".into() }],
        }
    }

    #[test]
    fn migrations_files_and_hostile_values_round_trip() {
        let mut c = Catalogue::open(Path::new(":memory:")).expect("open");
        let lib = c.library_id("Audiobooks", "/srv/a").expect("lib");
        assert_eq!(c.library_id("Audiobooks", "/srv/b").expect("again"), lib);
        c.upsert_file(lib, "F/a'b\".m4b", 5, 6, &rec(), 1).expect("upsert");
        let files = c.files(lib).expect("files");
        assert_eq!(files["F/a'b\".m4b"].probe, rec());
        let keep = HashSet::new();
        assert_eq!(c.forget_files_not_in(lib, &keep).expect("forget"), 1);
    }

    /// Review M2: a book's caches are invalidated when its audio changes, and an ID held in
    /// one library is visible to the scan of another (so an ASIN can't take it over).
    #[test]
    fn changed_audio_is_reported_and_ids_elsewhere_are_known() {
        let mut c = Catalogue::open(Path::new(":memory:")).expect("open");
        let a = c.library_id("A", "/srv/a").expect("lib");
        let b = c.library_id("B", "/srv/b").expect("lib");
        c.replace_library(a, &[book("aaa")], &[], &[], 10).expect("first");
        let (_, _, changed) = c.replace_library(a, &[book("aaa")], &[], &[], 20).expect("same");
        assert!(changed.is_empty());
        let mut longer = book("aaa");
        longer.duration_ms = 2000;
        let (_, _, changed) = c.replace_library(a, &[longer], &[], &[], 30).expect("longer");
        assert_eq!(changed, ["aaa"]);
        let mut more_parts = book("aaa");
        more_parts.duration_ms = 2000;
        more_parts.parts.push("F/b.m4b".into());
        let (_, _, changed) = c.replace_library(a, &[more_parts], &[], &[], 40).expect("parts");
        assert_eq!(changed, ["aaa"]);
        assert_eq!(c.ids_elsewhere(b).expect("elsewhere"), HashSet::from(["aaa".to_owned()]));
        assert!(c.ids_elsewhere(a).expect("elsewhere").is_empty());
    }

    #[test]
    fn books_that_disappear_are_marked_missing_not_deleted() {
        let mut c = Catalogue::open(Path::new(":memory:")).expect("open");
        let lib = c.library_id("A", "/srv/a").expect("lib");
        c.replace_library(lib, &[book("aaa"), book("bbb")], &[], &[], 10).expect("first");
        let (n, gone, changed) = c.replace_library(lib, &[book("aaa")], &[], &[], 20).expect("second");
        assert_eq!((n, gone), (1, 1));
        assert!(changed.is_empty(), "same audio: caches stay");
        let missing: Option<i64> =
            c.conn.query_row("SELECT missing_since FROM book WHERE id = 'bbb'", [], |r| r.get(0)).expect("row");
        assert_eq!(missing, Some(20));
        // It comes back when found again.
        c.replace_library(lib, &[book("aaa"), book("bbb")], &[], &[], 30).expect("third");
        let missing: Option<i64> =
            c.conn.query_row("SELECT missing_since FROM book WHERE id = 'bbb'", [], |r| r.get(0)).expect("row");
        assert_eq!(missing, None);
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let dir = std::env::temp_dir().join(format!("abshare-db-{}.db", std::process::id()));
        {
            let conn = Connection::open(&dir).expect("open");
            conn.execute_batch("PRAGMA user_version = 99").expect("set");
        }
        assert!(Catalogue::open(&dir).is_err());
        let _ = std::fs::remove_file(&dir);
    }
}
