//! `abshare`: the ABShare server. Phase 1 provides `scan` and `check-config`; `serve` comes
//! with the API in later phases.

#![forbid(unsafe_code)]

mod auth;
mod catalogue;
mod config;
mod covers;
mod db;
mod probe;
mod scan;
mod secret;
mod tools;
mod walk;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage:
  abshare check-config --config FILE
  abshare scan --config FILE [--library NAME] [--problems]";

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
    let cmd = args.next().ok_or(USAGE)?;
    let mut config: Option<PathBuf> = None;
    let mut library: Option<String> = None;
    let mut problems = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => config = Some(args.next().ok_or("--config needs a file")?.into()),
            "--library" => library = Some(args.next().ok_or("--library needs a name")?),
            "--problems" => problems = true,
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
        _ => Err(USAGE.into()),
    }
}
