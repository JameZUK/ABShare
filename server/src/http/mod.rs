//! The HTTP server (SPEC §5.1, §6): a hardened serve loop (header timeout, connection caps),
//! the request guard, and the `/v1` API.

pub mod admin_routes;
pub mod auth_routes;
pub mod diag_routes;
pub mod guard;
pub mod history_routes;
pub mod library_routes;
pub mod me_routes;
pub mod media_routes;
pub mod passkeys;
pub mod social_routes;
pub mod web;

use crate::auth::crypto::{Hasher, SeedBox};
use crate::auth::sessions::{CHALLENGE_TTL, Ceremonies, ENROL_TTL, Session, Sessions};
use crate::auth::store::Perms;
use crate::auth::{audit, store::Role};
use crate::secret::Secret;
use abshare_core::auth::lockout::Limiter;
use axum::Router;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use guard::{Client, GuardConfig, Peer, RateLimit};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rusqlite::Connection;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;
use tower::ServiceExt;

pub const MAX_CONNECTIONS: usize = 1024;
pub const MAX_PER_ADDRESS: usize = 64;
const HEADER_TIMEOUT: Duration = Duration::from_secs(15);
/// A request body must arrive within this (bodies are small: 64 KiB at most).
pub const BODY_TIMEOUT: Duration = Duration::from_secs(15);
/// A new connection must send its first byte within this.
const FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_BODY: usize = 64 * 1024;

/// Reads a request's whole body up front, within [`BODY_TIMEOUT`] and [`MAX_BODY`], so a
/// client trickling a body can't hold a connection or a handler (review H1).
async fn read_body(req: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    use axum::response::IntoResponse;
    if matches!(*req.method(), axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS) {
        return next.run(req).await;
    }
    let (parts, body) = req.into_parts();
    match tokio::time::timeout(BODY_TIMEOUT, axum::body::to_bytes(body, MAX_BODY)).await {
        Ok(Ok(bytes)) => {
            // Every JSON body is an object with named fields. serde would also read a struct
            // from an array of its fields in order, which a strict schema must not allow.
            let json = parts.headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| {
                v.split(';').next().is_some_and(|t| t.trim().eq_ignore_ascii_case("application/json"))
            });
            let first = bytes.iter().find(|b| !b.is_ascii_whitespace());
            if json && first.is_some_and(|&b| b != b'{') {
                return ApiError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "invalid",
                    "A request body must be a JSON object.",
                )
                .into_response();
            }
            next.run(axum::extract::Request::from_parts(parts, axum::body::Body::from(bytes))).await
        }
        Ok(Err(_)) => {
            ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large", "That request is too large.").into_response()
        }
        Err(_) => ApiError::new(StatusCode::REQUEST_TIMEOUT, "timeout", "The request took too long to arrive.")
            .into_response(),
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The database, used from blocking threads only.
#[derive(Clone)]
pub struct Db(Arc<Mutex<Connection>>);

impl Db {
    pub fn new(conn: Connection) -> Self {
        Self(Arc::new(Mutex::new(conn)))
    }

    /// Runs `f` with the connection on a blocking thread.
    pub async fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, String> + Send + 'static,
    ) -> Result<T, ApiError> {
        let db = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut c = db.lock().map_err(|_| "database lock poisoned".to_string())?;
            f(&mut c)
        })
        .await
        .map_err(|_| ApiError::internal("database task failed"))?
        .map_err(ApiError::internal)
    }
}

/// Enrolment in progress (SPEC §5.5): nothing is written until it finishes.
#[derive(Clone)]
pub struct Enrol {
    pub invite_hash: [u8; 32],
    pub purpose: String,
    pub for_user: Option<i64>,
    pub username: String,
    pub webauthn_id: webauthn_rs::prelude::Uuid,
    pub display_name: String,
    pub role: Role,
    pub perms: Perms,
    pub password_hash: Option<String>,
    pub totp_seed: Option<Vec<u8>>,
    pub totp_step: Option<i64>,
    pub passkey: Option<(String, webauthn_rs::prelude::Passkey)>,
    pub passkey_reg: Option<webauthn_rs::prelude::PasskeyRegistration>,
}

/// Password accepted, second factor pending.
#[derive(Clone)]
pub struct LoginPending {
    pub user: i64,
    pub device_name: String,
    pub attempt: abshare_core::auth::lockout::Attempt,
}

pub enum PasskeyLogin {
    Discoverable(webauthn_rs::prelude::DiscoverableAuthentication),
    ForUser(i64, webauthn_rs::prelude::PasskeyAuthentication),
}

pub struct AppState {
    pub db: Db,
    pub hasher: Hasher,
    pub sessions: Sessions,
    pub limiter: Mutex<Limiter>,
    pub seeds: SeedBox,
    pub audit_key: [u8; 32],
    noise_minute: std::sync::atomic::AtomicI64,
    noise_count: std::sync::atomic::AtomicU64,
    noise_dropped: std::sync::atomic::AtomicU64,
    pub webauthn: webauthn_rs::Webauthn,
    pub enrol: Ceremonies<Enrol>,
    pub login: Ceremonies<LoginPending>,
    pub passkey_login: Ceremonies<PasskeyLogin>,
    pub passkey_add: Ceremonies<(i64, webauthn_rs::prelude::PasskeyRegistration)>,
    pub totp_add: Ceremonies<(i64, Vec<u8>)>,
    /// Keyed like the lockout: IPv4 exactly, IPv6 by /64 (one subscriber's allocation).
    pub anon_limit: RateLimit<abshare_core::auth::lockout::AddrKey>,
    pub progress_limit: RateLimit<(i64, String)>,
    /// Device sign-outs, per calling device.
    pub device_limit: RateLimit<i64>,
    /// Listening-history changes (clear, turn on or off), per calling device.
    pub history_limit: RateLimit<i64>,
    /// Offline-history replays, per person (review S1).
    pub replay_limit: RateLimit<i64>,
    /// Review and recommendation writes, per calling device (review S11).
    pub social_limit: RateLimit<i64>,
    /// Start-up reports from blank pages: rate-limited per address, kept in memory only.
    pub diag_limit: RateLimit<abshare_core::auth::lockout::AddrKey>,
    pub diag: Mutex<std::collections::VecDeque<diag_routes::DiagReport>>,
    pub media: Option<Arc<crate::media::Media>>,
    pub web_dir: Option<std::path::PathBuf>,
    /// What Admin → Server settings shows of the config file (D35). Read-only.
    pub settings: ServerSettings,
    pub catalog: tokio::sync::RwLock<(std::sync::Arc<crate::library::Catalog>, i64)>,
    pub guard: Arc<GuardConfig>,
    pub issuer: String,
}

pub type State = Arc<AppState>;

/// The query of a route that takes none: any parameter is refused (`400`), as the API
/// promises (pentest P17).
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoQuery {}

/// The config-file settings Admin → Server settings shows (D35). File paths are left out on
/// purpose: an admin needs to know how the server behaves, not where its files are
/// (`abshare doctor` shows those to whoever has the machine).
#[derive(Debug, Clone, Default)]
pub struct ServerSettings {
    pub listen: String,
    pub ffmpeg_jobs: usize,
    pub cache_max_gib: u64,
}

impl AppState {
    pub fn new(
        conn: Connection,
        secret: &Secret,
        origin: &str,
        trusted: Vec<IpAddr>,
        hasher: Hasher,
    ) -> Result<Self, String> {
        let url = webauthn_rs::prelude::Url::parse(origin).map_err(|e| format!("public_origin: {e}"))?;
        let host = url.host_str().ok_or("public_origin needs a host")?.to_owned();
        let host_header = match url.port() {
            Some(p) => format!("{host}:{p}"),
            None => host.clone(),
        };
        let clean_origin = origin.trim_end_matches('/').to_owned();
        let webauthn = webauthn_rs::WebauthnBuilder::new(&host, &url)
            .and_then(|b| b.rp_name("ABShare").build())
            .map_err(|e| format!("webauthn: {e}"))?;
        let audit_key = secret.derive(crate::secret::purpose::AUDIT);
        crate::auth::audit::ensure_tags(&conn, &audit_key)?;
        Ok(Self {
            db: Db::new(conn),
            hasher,
            sessions: Sessions::new(),
            limiter: Mutex::new(Limiter::new()),
            seeds: SeedBox::new(&secret.derive(crate::secret::purpose::TOTP_SEED)),
            audit_key,
            noise_minute: std::sync::atomic::AtomicI64::new(0),
            noise_count: std::sync::atomic::AtomicU64::new(0),
            noise_dropped: std::sync::atomic::AtomicU64::new(0),
            webauthn,
            enrol: Ceremonies::new(ENROL_TTL),
            login: Ceremonies::new(CHALLENGE_TTL * 2),
            passkey_login: Ceremonies::new(CHALLENGE_TTL),
            passkey_add: Ceremonies::new(CHALLENGE_TTL),
            totp_add: Ceremonies::new(ENROL_TTL),
            anon_limit: RateLimit::new(30),
            progress_limit: RateLimit::new(12),
            device_limit: RateLimit::new(10),
            history_limit: RateLimit::new(10),
            replay_limit: RateLimit::new(history_routes::REPLAYS_PER_MINUTE),
            social_limit: RateLimit::new(social_routes::WRITES_PER_MINUTE),
            diag_limit: RateLimit::new(diag_routes::PER_MINUTE),
            diag: Mutex::new(std::collections::VecDeque::new()),
            media: None,
            web_dir: None,
            settings: ServerSettings::default(),
            catalog: tokio::sync::RwLock::new((std::sync::Arc::new(crate::library::Catalog::default()), 0)),
            guard: Arc::new(GuardConfig { trusted, host: host_header, origin: clean_origin }),
            issuer: "ABShare".into(),
        })
    }

    /// Makes the next [`Self::catalog`] call look at the database again (after a change made
    /// in this process, such as an admin's correction).
    pub async fn invalidate_catalog(&self) {
        self.catalog.write().await.1 = 0;
    }

    /// The catalogue view, reloaded when the database has changed (checked at most every 5 s).
    pub async fn catalog(&self) -> Result<Arc<crate::library::Catalog>, ApiError> {
        {
            let g = self.catalog.read().await;
            if now() - g.1 < 5 && g.1 != 0 {
                return Ok(g.0.clone());
            }
        }
        let current = self.catalog.read().await.0.stamp;
        let stamp = self.db.run(|c| crate::library::stamp(c)).await?;
        let mut g = self.catalog.write().await;
        if stamp != current || g.1 == 0 {
            g.0 = Arc::new(self.db.run(|c| crate::library::load(c)).await?);
        }
        g.1 = now();
        Ok(g.0.clone())
    }

    /// Writes an audit entry; failures to audit are logged but never block the request.
    /// Failed sign-in noise is capped (review A7): at most [`NOISE_PER_MINUTE`] such rows a
    /// minute, the rest counted into one `audit.suppressed` row, so a flood can't roll real
    /// security events out of the log.
    pub async fn audit(
        &self,
        event: &'static str,
        user: Option<i64>,
        device: Option<i64>,
        ip: Option<IpAddr>,
        detail: String,
    ) {
        use std::sync::atomic::Ordering::Relaxed;
        let minute = now() / 60;
        let prev = self.noise_minute.swap(minute, Relaxed);
        if prev != minute {
            self.noise_count.store(0, Relaxed);
            let dropped = self.noise_dropped.swap(0, Relaxed);
            if dropped > 0 {
                Box::pin(self.audit(
                    "audit.suppressed",
                    None,
                    None,
                    None,
                    format!("{dropped} failed sign-in events in the previous minute(s)"),
                ))
                .await;
            }
        }
        if NOISE_EVENTS.contains(&event) && self.noise_count.fetch_add(1, Relaxed) >= NOISE_PER_MINUTE {
            self.noise_dropped.fetch_add(1, Relaxed);
            return;
        }
        let key = self.audit_key;
        let ip = ip.map(|i| i.to_string());
        let r = self
            .db
            .run(move |c| {
                audit::append(
                    c,
                    &key,
                    &audit::Entry { user_id: user, device_id: device, event, detail: &detail, ip: ip.as_deref() },
                    now(),
                )
            })
            .await;
        if r.is_err() {
            eprintln!("abshare: could not write audit entry {event}");
        }
    }
}

/// Applies revocations made outside this process (the `abshare admin` CLI) to the live
/// sessions within 30 seconds: devices revoked and users disabled since the last look.
pub fn spawn_revocation_sync(st: State) {
    tokio::spawn(async move {
        let mut since = now();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            // Overlap the windows a little, so a revocation committed at the boundary (or a small
            // clock step) isn't missed; applying one twice is harmless (review H7).
            let from = since - 5;
            since = now();
            let r = st
                .db
                .run(move |c| {
                    let mut q =
                        c.prepare("SELECT id FROM device WHERE revoked_at >= ?1").map_err(crate::db::sql_err)?;
                    let devices: Vec<i64> = q
                        .query_map([from], |r| r.get(0))
                        .map_err(crate::db::sql_err)?
                        .collect::<Result<_, _>>()
                        .map_err(crate::db::sql_err)?;
                    let mut q = c.prepare("SELECT id FROM user WHERE disabled_at >= ?1").map_err(crate::db::sql_err)?;
                    let users: Vec<i64> = q
                        .query_map([from], |r| r.get(0))
                        .map_err(crate::db::sql_err)?
                        .collect::<Result<_, _>>()
                        .map_err(crate::db::sql_err)?;
                    Ok((devices, users))
                })
                .await;
            if let Ok((devices, users)) = r {
                for d in devices {
                    st.sessions.revoke_device(d);
                }
                for u in users {
                    st.sessions.revoke_user(u);
                }
            }
        }
    });
}

/// Events that an anonymous flood can generate, capped in the audit log (review A7).
const NOISE_EVENTS: &[&str] = &["login.failed", "login.passkey_failed", "login.second_factor_failed", "step_up.failed"];
pub const NOISE_PER_MINUTE: u64 = 30;

/// API errors: a stable code and a message safe to show. Internal details are logged with a
/// correlation ID, never sent.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into() }
    }

    pub fn internal(detail: impl std::fmt::Display) -> Self {
        let id: String = crate::auth::crypto::random::<6>().iter().map(|b| format!("{b:02x}")).collect();
        eprintln!("abshare: internal error {id}: {detail}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", format!("Something went wrong (reference {id})."))
    }

    pub fn factor_required() -> Self {
        Self::new(StatusCode::FORBIDDEN, "factor_required", "Set up a new passkey or authenticator app first.")
    }

    pub fn bad(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid", message)
    }

    pub fn denied() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "invalid_credentials", "Those details didn't work.")
    }

    pub fn unauthenticated() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthenticated", "Please sign in.")
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", "Not found.")
    }

    pub fn too_many() -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "too_many_requests", "Too many attempts. Try again later.")
    }

    pub fn step_up() -> Self {
        Self::new(StatusCode::FORBIDDEN, "step_up_required", "Confirm it's you first.")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({ "error": self.code, "message": self.message }).to_string();
        (self.status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
    }
}

impl From<crate::auth::crypto::HashError> for ApiError {
    fn from(e: crate::auth::crypto::HashError) -> Self {
        match e {
            crate::auth::crypto::HashError::Busy => {
                Self::new(StatusCode::SERVICE_UNAVAILABLE, "busy", "The server is busy. Try again in a moment.")
            }
            crate::auth::crypto::HashError::Internal => Self::internal("password hashing failed"),
        }
    }
}

/// The signed-in caller, from `Authorization: Bearer`.
pub struct Authed(pub Session);

impl FromRequestParts<State> for Authed {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &State) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(ApiError::unauthenticated)?;
        let s = state.sessions.access(token, now()).ok_or_else(ApiError::unauthenticated)?;
        if s.needs_factor {
            let path = parts
                .extensions
                .get::<axum::extract::OriginalUri>()
                .map_or_else(|| parts.uri.path().to_owned(), |u| u.0.path().to_owned());
            if !allowed_while_needing_factor(&path) {
                return Err(ApiError::factor_required());
            }
        }
        Ok(Authed(s))
    }
}

/// After a recovery-code sign-in, only the account routes (to set up a new factor) and the
/// auth routes work until a new factor exists (SPEC §5.5). Listening history is personal data,
/// not a sign-in method, so it waits too.
pub fn allowed_while_needing_factor(path: &str) -> bool {
    if path == "/v1/me/history" || path.starts_with("/v1/me/history/") {
        return false;
    }
    path == "/v1/me" || path.starts_with("/v1/me/") || path.starts_with("/v1/auth/")
}

/// The client address set by the guard.
pub struct Ip(pub IpAddr);

impl<S: Send + Sync> FromRequestParts<S> for Ip {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<Client>().map(|c| Ip(c.ip)).ok_or_else(|| ApiError::internal("no client address"))
    }
}

pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    // A name sent twice means a cookie planted from elsewhere (a sibling subdomain can set one
    // for a parent domain): neither is trusted (review A6).
    let mut found =
        headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).filter_map(
            |kv| {
                let (k, v) = kv.trim().split_once('=')?;
                (k == name).then(|| v.to_owned())
            },
        );
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

pub fn set_cookie(name: &str, value: &str, path: &str, max_age: i64) -> HeaderValue {
    HeaderValue::from_str(&format!("{name}={value}; Path={path}; Max-Age={max_age}; HttpOnly; Secure; SameSite=Strict"))
        .unwrap_or_else(|_| HeaderValue::from_static("invalid=1; Max-Age=0"))
}

pub const REFRESH_COOKIE: &str = "abshare_refresh";
pub const MEDIA_COOKIE: &str = "abshare_media";

pub fn router(state: State) -> Router {
    let g = state.guard.clone();
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest(
            "/v1/auth",
            auth_routes::routes(state.clone()).layer(axum::middleware::from_fn_with_state(g.clone(), guard::csrf)),
        )
        .nest("/v1/diag", diag_routes::routes().layer(axum::middleware::from_fn_with_state(g.clone(), guard::csrf)))
        .nest("/v1/me", me_routes::routes())
        .nest("/v1", library_routes::routes().merge(social_routes::routes()))
        .nest("/v1/media", media_routes::routes())
        .nest(
            "/v1/admin",
            admin_routes::routes().merge(social_routes::admin_routes()).merge(history_routes::admin_routes()),
        )
        .fallback(web::fallback)
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY))
        .layer(axum::middleware::from_fn(read_body))
        .layer(axum::middleware::from_fn_with_state(g, guard::front))
        .with_state(state)
}

/// Accepts connections with caps (overall, and per address except trusted proxies, which
/// carry everyone's traffic) and timeouts, so slow or greedy clients can't exhaust the server.
pub async fn serve(listener: tokio::net::TcpListener, app: Router, proxies: Vec<IpAddr>) -> Result<(), String> {
    let all = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let per: Arc<Mutex<HashMap<IpAddr, usize>>> = Arc::default();
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(x) => x,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let Ok(permit) = all.clone().try_acquire_owned() else { continue };
        let ip = guard::canonical(peer.ip());
        let counted = !proxies.contains(&ip);
        if counted {
            let Ok(mut m) = per.lock() else { continue };
            let n = m.entry(ip).or_default();
            if *n >= MAX_PER_ADDRESS {
                continue;
            }
            *n += 1;
        }
        let (app, per) = (app.clone(), per.clone());
        tokio::spawn(async move {
            let _permit = permit;
            serve_one(stream, peer, app).await;
            if counted
                && let Ok(mut m) = per.lock()
                && let Some(n) = m.get_mut(&ip)
            {
                *n -= 1;
                if *n == 0 {
                    m.remove(&ip);
                }
            }
        });
    }
}

async fn serve_one(stream: tokio::net::TcpStream, peer: SocketAddr, app: Router) {
    // A connection that never speaks would otherwise hold its slot before hyper's own timers
    // start (protocol detection waits for bytes).
    let mut first = [0u8; 1];
    if !matches!(tokio::time::timeout(FIRST_BYTE_TIMEOUT, stream.peek(&mut first)).await, Ok(Ok(n)) if n > 0) {
        return;
    }
    let svc = hyper::service::service_fn(move |mut req: hyper::Request<hyper::body::Incoming>| {
        req.extensions_mut().insert(Peer(peer));
        let app = app.clone();
        async move { app.oneshot(req.map(axum::body::Body::new)).await }
    });
    let mut b = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
    b.http1().timer(hyper_util::rt::TokioTimer::new()).header_read_timeout(HEADER_TIMEOUT).keep_alive(true);
    let _ = b.serve_connection_with_upgrades(TokioIo::new(stream), svc).await;
}

/// Helper for handlers: run a request through the router in tests.
#[cfg(test)]
pub async fn call(app: &Router, req: axum::extract::Request) -> Response {
    app.clone().oneshot(req).await.expect("infallible")
}

#[cfg(test)]
mod tests;
