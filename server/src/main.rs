//! `abshare`: the ABShare server and its commands (`serve`, `scan`, `doctor`, `health`, `admin`).

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
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const USAGE: &str = "usage:
  abshare check-config --config FILE
  abshare doctor --config FILE
  abshare scan --config FILE [--library NAME] [--problems] [--every SECONDS]
  abshare serve --config FILE
  abshare health --config FILE
  abshare admin invite --config FILE [--admin] [--name DISPLAY NAME]
  abshare admin reset --config FILE --user USERNAME
  abshare admin users --config FILE
  abshare admin devices --config FILE --user USERNAME
  abshare admin revoke-all --config FILE --user USERNAME
  abshare admin verify-audit --config FILE";

/// How long `serve` waits, after SIGTERM or SIGINT, for requests in progress. Docker sends
/// SIGKILL 10 seconds after SIGTERM by default, so this stays below that.
const STOP_GRACE: Duration = Duration::from_secs(8);

fn main() -> ExitCode {
    // `abshare render-cover`: the cover renderer the scanner runs in the jail. Not in USAGE;
    // it takes no configuration and starts no runtime.
    if let Some(code) = covers::render_command(std::env::args_os()) {
        return code;
    }
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("abshare: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = rt.block_on(run());
    // Dropping the runtime would wait for blocking work (a database call, a hash) without
    // limit; a stop must be bounded.
    rt.shutdown_timeout(Duration::from_secs(1));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("abshare: {e}");
            ExitCode::FAILURE
        }
    }
}

/// SIGTERM and SIGINT. As PID 1 in a container a process gets no default signal actions, so
/// without handlers `docker stop` would wait out its grace period and then kill.
struct Stop {
    term: tokio::signal::unix::Signal,
    int: tokio::signal::unix::Signal,
}

impl Stop {
    fn new() -> Result<Self, String> {
        use tokio::signal::unix::{SignalKind, signal};
        let listen = |kind| signal(kind).map_err(|e| format!("signal handler: {e}"));
        Ok(Self { term: listen(SignalKind::terminate())?, int: listen(SignalKind::interrupt())? })
    }

    /// Waits for either signal and names it.
    async fn wait(&mut self) -> &'static str {
        tokio::select! {
            _ = self.term.recv() => "SIGTERM",
            _ = self.int.recv() => "SIGINT",
        }
    }
}

/// Counts the requests being served, response bodies included, so a stop can wait for them.
#[derive(Clone, Default)]
struct InFlight(Arc<(AtomicUsize, tokio::sync::Notify)>);

struct InFlightGuard(InFlight);

impl InFlight {
    fn enter(&self) -> InFlightGuard {
        self.0.0.fetch_add(1, Ordering::SeqCst);
        InFlightGuard(self.clone())
    }

    fn count(&self) -> usize {
        self.0.0.load(Ordering::SeqCst)
    }

    /// Returns once no request is in progress.
    async fn idle(&self) {
        loop {
            // Created before the count is read, so a request finishing in between still wakes it.
            let changed = self.0.1.notified();
            if self.count() == 0 {
                return;
            }
            changed.await;
        }
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        let shared = &self.0.0;
        if shared.0.fetch_sub(1, Ordering::SeqCst) == 1 {
            shared.1.notify_waiters();
        }
    }
}

/// A response body that keeps its request counted until it has been sent (or abandoned).
struct Tracked {
    inner: axum::body::Body,
    _guard: InFlightGuard,
}

impl hyper::body::Body for Tracked {
    type Data = bytes::Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        std::pin::Pin::new(&mut self.inner).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        self.inner.size_hint()
    }
}

/// The outermost layer: counts a request from its arrival to the end of its response body.
async fn track(
    axum::extract::State(in_flight): axum::extract::State<InFlight>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let guard = in_flight.enter();
    let (parts, body) = next.run(req).await.into_parts();
    axum::response::Response::from_parts(parts, axum::body::Body::new(Tracked { inner: body, _guard: guard }))
}

/// Runs the accept loop until a stop signal, then drains: the listener is closed at once (no
/// new connections), and requests in progress get up to `grace` to finish. A second signal
/// ends the wait.
async fn serve_until_stopped(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    proxies: Vec<std::net::IpAddr>,
    mut stop: Stop,
    grace: Duration,
) -> Result<(), String> {
    let in_flight = InFlight::default();
    let app = app.layer(axum::middleware::from_fn_with_state(in_flight.clone(), track));
    let signal = tokio::select! {
        r = http::serve(listener, app, proxies) => return r,
        s = stop.wait() => s,
    };
    // The accept loop owned the listener: dropping the loop above closed the socket.
    let waiting = in_flight.count();
    if waiting > 0 {
        println!("abshare: {signal}: waiting up to {} s for {waiting} requests in progress", grace.as_secs());
    }
    tokio::select! {
        () = in_flight.idle() => println!("abshare: {signal}: stopped"),
        () = tokio::time::sleep(grace) => {
            println!("abshare: {signal}: stopped with {} requests unfinished", in_flight.count());
        }
        again = stop.wait() => println!("abshare: {again}: stopped at once"),
    }
    Ok(())
}

/// `abshare health`: asks the running server for `/healthz` and says whether it answered 200.
/// For a container health check (the image has no shell or curl). It connects only to the
/// configured `listen` address, or to loopback when that is every address.
fn health(cfg: &config::Config) -> Result<(), String> {
    use std::io::{Read, Write};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
    let listen = cfg.net()?.listen;
    let ip = match listen.ip() {
        IpAddr::V4(a) if a.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(a) if a.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        a => a,
    };
    let addr = SocketAddr::new(ip, listen.port());
    let io = |e: std::io::Error| format!("health: {addr}: {e}");
    let wait = Some(Duration::from_secs(3));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map_err(io)?;
    s.set_read_timeout(wait).and_then(|()| s.set_write_timeout(wait)).map_err(io)?;
    // The server answers only for its own Host. If this address is itself a trusted proxy,
    // the server wants that proxy's word that TLS was used.
    let request = format!("GET /healthz HTTP/1.0\r\nHost: {}\r\nX-Forwarded-Proto: https\r\n\r\n", cfg.host_header()?);
    s.write_all(request.as_bytes()).map_err(io)?;
    let mut head = [0u8; 12];
    s.read_exact(&mut head).map_err(io)?;
    match &head {
        b"HTTP/1.0 200" | b"HTTP/1.1 200" => Ok(()),
        _ => Err(format!("health: {addr} answered {:?}", String::from_utf8_lossy(&head))),
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
            config::check_ffmpeg_libs(&cfg.ffmpeg.libs)?;
            for l in &cfg.libraries {
                config::check_library_root(&l.path)?;
            }
            if cfg.proxy_warning() {
                eprintln!("abshare: warning: {}", config::PROXY_WARNING);
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
        "health" => health(&cfg),
        "doctor" => {
            let checks = doctor::run(&cfg).await;
            if doctor::print(&checks) { Ok(()) } else { Err("doctor found problems".into()) }
        }
        "scan" => {
            config::prepare_data_dir(&cfg.data_dir)?;
            config::check_ffmpeg_libs(&cfg.ffmpeg.libs)?;
            let secret = secret::Secret::load_or_create(&cfg.data_dir)?;
            let mut cat = catalogue::Catalogue::open(&cfg.data_dir.join("abshare.db"))?;
            if !cfg.libraries.iter().any(|l| library.as_deref().is_none_or(|want| want.eq_ignore_ascii_case(&l.name))) {
                return Err("no library by that name".into());
            }
            // Installed before the first scan. A stop ends the scan at once: the catalogue is
            // written in transactions, and the jailed tools are killed with their jobs.
            let mut stop = Stop::new()?;
            let scans = async {
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
                    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
                    while std::time::Instant::now() < deadline {
                        if std::fs::remove_file(&trigger).is_ok() {
                            println!("abshare: scan requested by an admin");
                            break;
                        }
                        tokio::time::sleep(Duration::from_secs(10)).await;
                    }
                }
                Ok::<(), String>(())
            };
            tokio::select! {
                r = scans => r,
                signal = stop.wait() => {
                    println!("abshare: {signal}: scan stopped");
                    Ok(())
                }
            }
        }
        "serve" => {
            config::prepare_data_dir(&cfg.data_dir)?;
            config::check_ffmpeg_libs(&cfg.ffmpeg.libs)?;
            // A library the server could write to is refused (SPEC D3), as check-config does.
            for l in &cfg.libraries {
                config::check_library_root(&l.path)?;
            }
            let config::Net { listen, trusted } = cfg.net()?;
            if cfg.proxy_warning() {
                eprintln!("abshare: warning: {}", config::PROXY_WARNING);
            }
            let secret = secret::Secret::load_or_create(&cfg.data_dir)?;
            let conn = db::open(&cfg.data_dir.join("abshare.db"))?;
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
            state.settings = http::ServerSettings {
                listen: cfg.listen.clone(),
                ffmpeg_jobs: cfg.ffmpeg.jobs,
                cache_max_gib: cfg.cache.max_gib,
            };
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
            http::history_routes::spawn_prune(state.clone());
            let app = http::router(state);
            let stop = Stop::new()?;
            let listener = tokio::net::TcpListener::bind(listen).await.map_err(|e| format!("bind {listen}: {e}"))?;
            println!("abshare listening on {listen} for {}", cfg.public_origin);
            serve_until_stopped(listener, app, proxies, stop, STOP_GRACE).await
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
