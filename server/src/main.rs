//! `abshare`: the ABShare server. Phase 1 provides `scan` and `check-config`; `serve` comes
//! with the API in later phases.

#![forbid(unsafe_code)]

mod admin;
mod auth;
mod catalogue;
mod config;
mod covers;
mod db;
mod http;
mod probe;
mod scan;
mod secret;
mod tools;
mod walk;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage:
  abshare check-config --config FILE
  abshare scan --config FILE [--library NAME] [--problems]
  abshare serve --config FILE
  abshare admin invite --config FILE [--admin] [--name DISPLAY NAME]
  abshare admin reset --config FILE --user USERNAME
  abshare admin users --config FILE
  abshare admin devices --config FILE --user USERNAME
  abshare admin revoke-all --config FILE --user USERNAME
  abshare admin verify-audit --config FILE";

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("abshare: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut cmd = args.next().ok_or(USAGE)?;
    if cmd == "admin" {
        cmd = format!("admin {}", args.next().ok_or(USAGE)?);
    }
    let mut config: Option<PathBuf> = None;
    let mut library: Option<String> = None;
    let mut problems = false;
    let mut user: Option<String> = None;
    let mut name: Option<String> = None;
    let mut as_admin = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => config = Some(args.next().ok_or("--config needs a file")?.into()),
            "--library" => library = Some(args.next().ok_or("--library needs a name")?),
            "--problems" => problems = true,
            "--user" => user = Some(args.next().ok_or("--user needs a username")?),
            "--name" => name = Some(args.next().ok_or("--name needs a value")?),
            "--admin" => as_admin = true,
            _ => return Err(format!("unknown option {a}\n{USAGE}")),
        }
    }
    let path = config.ok_or_else(|| format!("--config is required\n{USAGE}"))?;
    let cfg = config::Config::load(&path)?;
    match cmd.as_str() {
        "check-config" => {
            config::prepare_data_dir(&cfg.data_dir)?;
            for l in &cfg.libraries {
                config::check_library_root(&l.path)?;
            }
            println!(
                "config OK: listen {}, {} trusted proxies, {} libraries, data dir {}, cache {} GiB",
                cfg.listen,
                cfg.trusted_proxies.len(),
                cfg.libraries.len(),
                cfg.data_dir.display(),
                cfg.cache.max_gib
            );
            Ok(())
        }
        "scan" => {
            config::prepare_data_dir(&cfg.data_dir)?;
            let secret = secret::Secret::load_or_create(&cfg.data_dir)?;
            let mut cat = catalogue::Catalogue::open(&cfg.data_dir.join("abshare.db"))?;
            let mut matched = false;
            for (n, lib) in cfg.libraries.iter().enumerate() {
                if library.as_deref().is_some_and(|want| !want.eq_ignore_ascii_case(&lib.name)) {
                    continue;
                }
                matched = true;
                let started = std::time::Instant::now();
                let rep = scan::scan_library(&cfg, &mut cat, &secret, lib, n as u32 + 1).await?;
                scan::print(&rep, problems);
                println!("  took {:.1} s", started.elapsed().as_secs_f64());
            }
            if !matched {
                return Err("no library by that name".into());
            }
            Ok(())
        }
        "serve" => {
            config::prepare_data_dir(&cfg.data_dir)?;
            let secret = secret::Secret::load_or_create(&cfg.data_dir)?;
            let conn = db::open(&cfg.data_dir.join("abshare.db"))?;
            let trusted = cfg
                .trusted_proxies
                .iter()
                .map(|p| {
                    p.parse::<std::net::IpAddr>().map_err(|_| format!("trusted_proxies: {p} is not an IP address"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let state = http::AppState::new(conn, &secret, &cfg.public_origin, trusted, auth::crypto::Hasher::new())?;
            let app = http::router(std::sync::Arc::new(state));
            let listener =
                tokio::net::TcpListener::bind(&cfg.listen).await.map_err(|e| format!("bind {}: {e}", cfg.listen))?;
            println!("abshare listening on {} for {}", cfg.listen, cfg.public_origin);
            tokio::select! {
                r = http::serve(listener, app) => r,
                _ = tokio::signal::ctrl_c() => Ok(()),
            }
        }
        c if c.starts_with("admin ") => {
            config::prepare_data_dir(&cfg.data_dir)?;
            let secret = secret::Secret::load_or_create(&cfg.data_dir)?;
            let conn = db::open(&cfg.data_dir.join("abshare.db"))?;
            let need_user = || user.clone().ok_or_else(|| format!("--user is required\n{USAGE}"));
            let out = match &c[6..] {
                "invite" => {
                    let role = if as_admin { auth::store::Role::Admin } else { auth::store::Role::Listener };
                    let link = admin::invite(&conn, &secret, &cfg.public_origin, role, name.as_deref())?;
                    format!(
                        "Invite ({}, valid {} h, single use). Send this link privately:\n{link}",
                        role.as_str(),
                        admin::INVITE_HOURS
                    )
                }
                "reset" => {
                    let link = admin::reset(&conn, &secret, &cfg.public_origin, &need_user()?)?;
                    format!(
                        "Reset link (valid {} h, single use; signs out every device when used):\n{link}",
                        admin::INVITE_HOURS
                    )
                }
                "users" => admin::users(&conn)?,
                "devices" => admin::devices(&conn, &need_user()?)?,
                "revoke-all" => format!("{} devices signed out", admin::revoke_all(&conn, &secret, &need_user()?)?),
                "verify-audit" => admin::verify_audit(&conn, &secret)?,
                _ => return Err(USAGE.into()),
            };
            println!("{out}");
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}
