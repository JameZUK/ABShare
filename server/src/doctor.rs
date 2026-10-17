//! `abshare doctor` (SPEC §5.8): checks a deployment from the inside and says what to fix.
//! Read-only: it never changes the library, the database or the data directory.

use crate::config::{self, Config};
use crate::tools::{Limits, run_jailed};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug)]
pub struct Check {
    pub level: Level,
    pub what: &'static str,
    pub detail: String,
}

fn check(level: Level, what: &'static str, detail: impl Into<String>) -> Check {
    Check { level, what, detail: detail.into() }
}

fn from(r: Result<String, String>, what: &'static str, fail: Level) -> Check {
    match r {
        Ok(d) => check(Level::Ok, what, d),
        Err(e) => check(fail, what, e),
    }
}

/// Every check, in order. Never panics; each failure is reported, not raised.
pub async fn run(cfg: &Config) -> Vec<Check> {
    let mut out = Vec::new();
    out.push(check(Level::Ok, "config", format!("{} libraries, origin {}", cfg.libraries.len(), cfg.public_origin)));
    if cfg.trusted_proxies.is_empty() {
        out.push(check(
            Level::Warn,
            "trusted proxies",
            "none set: behind a reverse proxy, client addresses (lockouts, rate limits) will be the proxy's",
        ));
    } else {
        out.push(check(Level::Ok, "trusted proxies", format!("{} set", cfg.trusted_proxies.len())));
    }
    out.push(from(data_dir(&cfg.data_dir), "data directory", Level::Fail));
    out.push(from(secret_file(&cfg.data_dir.join("secret.key")), "server secret", Level::Fail));
    for l in &cfg.libraries {
        out.push(from(
            config::check_library_root(&l.path).map(|()| format!("{} is read-only", l.name)),
            "library",
            Level::Fail,
        ));
    }
    // The jail refuses to start without Landlock ABI 6, so a jailed run proves the sandbox
    // works. (Containers often hide /sys/kernel/security, so the LSM list can't be relied on.)
    let jailed = jailed_ffmpeg(cfg).await;
    out.push(match &jailed {
        Ok(_) => check(Level::Ok, "sandbox", "Landlock, seccomp and limits applied (the jail started)"),
        Err(_) => check(
            Level::Fail,
            "sandbox",
            "the jail can't start: Linux 6.12+ with lsm=landlock,... is needed (SPEC D24)",
        ),
    });
    out.push(from(jailed, "jailed ffmpeg", Level::Fail));
    out.push(from(database(cfg), "database", Level::Fail));
    out.push(from(audit_chain(cfg), "audit log", Level::Fail));
    if let Some(web) = &cfg.web_dir {
        out.push(if web.join("index.html").is_file() {
            check(Level::Ok, "web app", web.display().to_string())
        } else {
            check(Level::Fail, "web app", format!("{} has no index.html", web.display()))
        });
    } else {
        out.push(check(Level::Warn, "web app", "web_dir not set: only the API is served"));
    }
    out
}

fn data_dir(dir: &Path) -> Result<String, String> {
    let meta = std::fs::symlink_metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if !meta.is_dir() {
        return Err(format!("{} is not a directory", dir.display()));
    }
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(format!("{} is mode {mode:o}; it must be 0700", dir.display()));
    }
    if meta.uid() != rustix::process::geteuid().as_raw() {
        return Err(format!("{} belongs to uid {}, not the user running ABShare", dir.display(), meta.uid()));
    }
    let free = rustix::fs::statvfs(dir).map(|s| s.f_bavail * s.f_frsize).unwrap_or(0);
    Ok(format!("{} (0700), {} GiB free", dir.display(), free >> 30))
}

fn secret_file(path: &Path) -> Result<String, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return Ok("not created yet (made on first start)".into()),
    };
    if !meta.is_file() || meta.permissions().mode() & 0o077 != 0 || meta.len() != 32 {
        return Err(format!("{} must be a 32-byte regular file with mode 0600", path.display()));
    }
    Ok("present, mode 0600".into())
}

async fn jailed_ffmpeg(cfg: &Config) -> Result<String, String> {
    let limits = Limits { mem_mib: 2048, cpu_secs: 10, fsize_mib: 1, wall: Duration::from_secs(20), stdout_cap: 4096 };
    let r =
        run_jailed(&cfg.ffmpeg, &[], None, &cfg.ffmpeg.ffmpeg, &["-hide_banner".into(), "-version".into()], &limits)
            .await?;
    if !r.success {
        return Err(format!("the jail refused to run ffmpeg: {}", r.stderr.lines().next().unwrap_or("no detail")));
    }
    Ok(String::from_utf8_lossy(&r.stdout).lines().next().unwrap_or("ffmpeg").chars().take(60).collect())
}

fn database(cfg: &Config) -> Result<String, String> {
    let path = cfg.data_dir.join("abshare.db");
    if !path.exists() {
        return Ok("not created yet (made on first start)".into());
    }
    let c = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    let version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    let want = crate::db::MIGRATIONS.len() as i64;
    if version != want {
        return Err(format!("schema version {version}, this build expects {want}: start the server once to migrate"));
    }
    let ok: String = c.query_row("PRAGMA quick_check", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    if ok != "ok" {
        return Err(format!("integrity check: {ok}"));
    }
    let books: i64 =
        c.query_row("SELECT count(*) FROM book WHERE missing_since IS NULL", [], |r| r.get(0)).unwrap_or(0);
    Ok(format!("schema {version}, intact, {books} books"))
}

fn audit_chain(cfg: &Config) -> Result<String, String> {
    let path = cfg.data_dir.join("abshare.db");
    if !path.exists() || !cfg.data_dir.join("secret.key").exists() {
        return Ok("nothing to check yet".into());
    }
    let secret = crate::secret::Secret::load_or_create(&cfg.data_dir)?;
    let c = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    match crate::auth::audit::verify(&c, &secret.derive(crate::secret::purpose::AUDIT)) {
        Ok(n) => Ok(format!("{n} entries, chain intact")),
        Err(id) => Err(format!("chain broken at entry {id}: the log was edited or rows removed")),
    }
}

/// Prints the report; returns whether nothing failed.
pub fn print(checks: &[Check]) -> bool {
    for c in checks {
        let tag = match c.level {
            Level::Ok => "  ok  ",
            Level::Warn => " WARN ",
            Level::Fail => " FAIL ",
        };
        println!("[{tag}] {:<16} {}", c.what, c.detail);
    }
    let fails = checks.iter().filter(|c| c.level == Level::Fail).count();
    let warns = checks.iter().filter(|c| c.level == Level::Warn).count();
    println!("{fails} failed, {warns} warnings");
    fails == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("abshare-doctor-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("mkdir");
        d
    }

    #[test]
    fn the_data_directory_must_be_private() {
        let d = tmp("dd");
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        assert!(data_dir(&d).err().is_some_and(|e| e.contains("0700")));
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        assert!(data_dir(&d).is_ok());
        assert!(data_dir(&d.join("missing")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_secret_must_be_private_and_whole() {
        let d = tmp("secret");
        let p = d.join("secret.key");
        assert!(secret_file(&p).is_ok(), "not created yet is fine");
        std::fs::write(&p, [0u8; 32]).expect("write");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        assert!(secret_file(&p).is_err());
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).expect("chmod");
        assert!(secret_file(&p).is_ok());
        std::fs::write(&p, [0u8; 31]).expect("write");
        assert!(secret_file(&p).is_err(), "wrong length");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_report_fails_on_any_failure() {
        assert!(print(&[check(Level::Ok, "a", ""), check(Level::Warn, "b", "")]));
        assert!(!print(&[check(Level::Ok, "a", ""), check(Level::Fail, "b", "")]));
    }
}
