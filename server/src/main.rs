//! `abshare`: the ABShare server. Phase 1 provides `scan` and `check-config`; `serve` comes
//! with the API in later phases.

#![forbid(unsafe_code)]

mod admin;
mod auth;
mod catalogue;
mod config;
mod covers;
mod db;
mod doctor;
mod http;
mod library;
mod media;
mod probe;
mod scan;
mod secret;
mod tools;
mod walk;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage:
  abshare check-config --config FILE
  abshare doctor --config FILE
  abshare scan --config FILE [--library NAME] [--problems] [--every SECONDS]
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
    let mut every: Option<u64> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => config = Some(args.next().ok_or("--config needs a file")?.into()),
            "--library" => library = Some(args.next().ok_or("--library needs a name")?),
            "--problems" => problems = true,
            "--user" => user = Some(args.next().ok_or("--user needs a username")?),
            "--name" => name = Some(args.next().ok_or("--name needs a value")?),
            "--admin" => as_admin = true,
            "--every" => {
                let n: u64 = args.next().and_then(|v| v.parse().ok()).ok_or("--every needs a number of seconds")?;
                if !(60..=86_400).contains(&n) {
                    return Err("--every is between 60 and 86400 seconds".into());
                }
                every = Some(n);
            }
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
        "doctor" => {
            let checks = doctor::run(&cfg).await;
            if doctor::print(&checks) { Ok(()) } else { Err("doctor found problems".into()) }
        }
        "scan" => {
            config::prepare_data_dir(&cfg.data_dir)?;
            let secret = secret::Secret::load_or_create(&cfg.data_dir)?;
            let mut cat = catalogue::Catalogue::open(&cfg.data_dir.join("abshare.db"))?;
            if !cfg.libraries.iter().any(|l| library.as_deref().is_none_or(|want| want.eq_ignore_ascii_case(&l.name))) {
                return Err("no library by that name".into());
            }
            loop {
                let started = std::time::Instant::now();
                let mut summary = Vec::new();
                for (n, lib) in cfg.libraries.iter().enumerate() {
                    if library.as_deref().is_some_and(|want| !want.eq_ignore_ascii_case(&lib.name)) {
                        continue;
                    }
                    let rep = scan::scan_library(&cfg, &mut cat, &secret, lib, n as u32 + 1).await?;
                    scan::print(&rep, problems);
                    summary.push(scan::summary_json(&rep));
                }
                let took = started.elapsed().as_secs_f64();
                println!("  took {took:.1} s");
                scan::write_last_scan(&cfg.data_dir, &summary, took)?;
                let Some(secs) = every else { break };
                // Wait for the interval, or for an admin's "scan now" (a file the server writes).
                let trigger = cfg.data_dir.join("scan-requested");
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
                while std::time::Instant::now() < deadline {
                    if std::fs::remove_file(&trigger).is_ok() {
                        println!("abshare: scan requested by an admin");
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                }
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
            let proxies: Vec<std::net::IpAddr> = trusted.iter().map(|ip| http::guard::canonical(*ip)).collect();
            let mut state =
                http::AppState::new(conn, &secret, &cfg.public_origin, trusted, auth::crypto::Hasher::new())?;
            let roots = cfg.libraries.iter().map(|l| (l.name.clone(), l.path.clone())).collect();
            let ff = config::Ffmpeg {
                ffmpeg: cfg.ffmpeg.ffmpeg.clone(),
                ffprobe: cfg.ffmpeg.ffprobe.clone(),
                jail: cfg.ffmpeg.jail.clone(),
                libs: cfg.ffmpeg.libs.clone(),
                jobs: cfg.ffmpeg.jobs,
            };
            state.web_dir = cfg.web_dir.clone();
            let engine = std::sync::Arc::new(media::Media::new(ff, cfg.data_dir.clone(), roots));
            state.media = Some(engine.clone());
            engine.clean_jobs();
            // Keep the transcode cache within [cache] max_gib: a sweep at start and every 10 min.
            let limit = cfg.cache.max_gib.saturating_mul(1 << 30);
            tokio::spawn(async move {
                loop {
                    match engine.sweep(limit).await {
                        Ok(0) => {}
                        Ok(freed) => println!("abshare: cache sweep freed {} MiB", freed >> 20),
                        Err(e) => eprintln!("abshare: cache sweep failed: {e}"),
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(600)).await;
                }
            });
            let state = std::sync::Arc::new(state);
            http::spawn_revocation_sync(state.clone());
            let app = http::router(state);
            let listener =
                tokio::net::TcpListener::bind(&cfg.listen).await.map_err(|e| format!("bind {}: {e}", cfg.listen))?;
            println!("abshare listening on {} for {}", cfg.listen, cfg.public_origin);
            tokio::select! {
                r = http::serve(listener, app, proxies) => r,
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
