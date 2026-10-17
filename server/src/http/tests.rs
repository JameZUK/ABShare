//! Integration tests for the auth API through the real router (SPEC §13): every route has
//! tests proving attacks are refused, alongside the happy paths.

use super::guard::Peer;
use super::{AppState, State, call, router};
use crate::auth::crypto::Hasher;
use crate::auth::store::{self, Perms, Role};
use crate::secret::Secret;
use abshare_core::auth::totp;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;

const HOST: &str = "books.example.com";
const ORIGIN: &str = "https://books.example.com";

struct T {
    app: Router,
    st: State,
}

fn setup() -> T {
    let conn = crate::db::open(std::path::Path::new(":memory:")).expect("db");
    let st = Arc::new(
        AppState::new(
            conn,
            &Secret::for_tests([5; 32]),
            ORIGIN,
            vec!["10.0.0.10".parse().expect("ip")],
            Hasher::for_tests(),
        )
        .expect("state"),
    );
    T { app: router(st.clone()), st }
}

fn req(method: &str, uri: &str, body: Option<Value>) -> axum::http::request::Builder {
    let b = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", HOST)
        .header("x-abshare-client", "1")
        .header("origin", ORIGIN);
    if body.is_some() { b.header("content-type", "application/json") } else { b }
}

async fn send(t: &T, method: &str, uri: &str, body: Option<Value>, extra: &[(&str, &str)]) -> Response {
    let mut b = req(method, uri, body.clone());
    for (k, v) in extra {
        b = b.header(*k, *v);
    }
    let r = b.body(body.map_or_else(Body::empty, |v| Body::from(v.to_string()))).expect("request");
    call(&t.app, r).await
}

async fn json_of(r: Response) -> Value {
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 20).await.expect("body");
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn b32_decode(s: &str) -> Vec<u8> {
    let (mut acc, mut bits, mut out) = (0u32, 0u32, Vec::new());
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'2'..=b'7' => c - b'2' + 26,
            _ => continue,
        };
        acc = (acc << 5) | u32::from(v);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

fn code(seed: &[u8], offset_steps: i64) -> String {
    let step = totp::step_at(super::now()) + offset_steps;
    format!("{:06}", totp::hotp(seed, step as u64))
}

fn invite(t: &T, role: Role) -> String {
    let (token, hash) = crate::auth::crypto::new_token();
    let st = t.st.clone();
    let c = st.db.0.lock().expect("db");
    store::create_invite(
        &c,
        &hash,
        "enrol",
        None,
        None,
        Some("Ada Example"),
        role,
        &Perms::default(),
        super::now(),
        super::now() + 3600,
    )
    .expect("invite");
    token
}

fn cookies(r: &Response) -> Vec<String> {
    r.headers().get_all("set-cookie").iter().map(|v| v.to_str().unwrap_or("").to_owned()).collect()
}

fn cookie_value(r: &Response, name: &str) -> String {
    cookies(r)
        .iter()
        .find_map(|c| c.strip_prefix(&format!("{name}=")).map(|v| v.split(';').next().unwrap_or("").to_owned()))
        .expect("cookie")
}

/// Enrols a user with password + TOTP; returns (seed, access token, refresh cookie, recovery codes).
async fn enrol(t: &T, username: &str) -> (Vec<u8>, String, String, Vec<String>) {
    let (seed, access, refresh, codes, _) = enrol_with_media(t, username).await;
    (seed, access, refresh, codes)
}

/// As [`enrol`], plus the media cookie.
async fn enrol_with_media(t: &T, username: &str) -> (Vec<u8>, String, String, Vec<String>, String) {
    enrol_role(t, username, Role::Listener).await
}

/// Enrols with password + TOTP in the given role.
async fn enrol_role(t: &T, username: &str, role: Role) -> (Vec<u8>, String, String, Vec<String>, String) {
    let tok = invite(t, role);
    let r = send(t, "POST", "/v1/auth/enrol/start", Some(json!({"token": tok, "username": username})), &[]).await;
    assert_eq!(r.status(), StatusCode::OK);
    let enrol = json_of(r).await["enrol"].as_str().expect("enrol").to_owned();
    let r = send(
        t,
        "POST",
        "/v1/auth/enrol/password",
        Some(json!({"enrol": enrol, "password": "correct horse battery"})),
        &[],
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let seed = b32_decode(json_of(r).await["secret"].as_str().expect("secret"));
    let r = send(t, "POST", "/v1/auth/enrol/totp", Some(json!({"enrol": enrol, "code": code(&seed, 0)})), &[]).await;
    assert_eq!(r.status(), StatusCode::OK);
    let r = send(t, "POST", "/v1/auth/enrol/finish", Some(json!({"enrol": enrol, "device_name": "Laptop"})), &[]).await;
    assert_eq!(r.status(), StatusCode::OK);
    let refresh = cookie_value(&r, "abshare_refresh");
    let media = cookie_value(&r, "abshare_media");
    let v = json_of(r).await;
    let codes =
        v["recovery_codes"].as_array().expect("codes").iter().map(|c| c.as_str().unwrap_or("").to_owned()).collect();
    (seed, v["access_token"].as_str().expect("access").to_owned(), refresh, codes, media)
}

/// Steps up with the password and a fresh authenticator code (enrolment's password).
async fn step_up(t: &T, username: &str, seed: &[u8], access: &str) {
    reset_totp_step(t, username);
    let r = send(
        t,
        "POST",
        "/v1/auth/step-up/totp",
        Some(json!({"password": "correct horse battery", "code": code(seed, 0)})),
        &[("authorization", &format!("Bearer {access}"))],
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK, "step-up");
}

fn reset_totp_step(t: &T, username: &str) {
    let c = t.st.db.0.lock().expect("db");
    c.execute("UPDATE user SET totp_last_step = NULL WHERE username = ?1", [username]).expect("reset");
}

// ---------------------------------------------------------------- the front door

#[tokio::test]
async fn plain_http_from_outside_is_refused_and_headers_are_always_sent() {
    let t = setup();
    let mut r = Request::builder().uri("/healthz").header("host", HOST).body(Body::empty()).expect("req");
    r.extensions_mut().insert(Peer(SocketAddr::from(([198, 51, 100, 9], 5555))));
    let resp = call(&t.app, r).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let csp = resp.headers().get("content-security-policy").and_then(|v| v.to_str().ok()).unwrap_or("");
    // Trusted Types are enforced, and only the two named policies may ever be created.
    assert!(csp.contains("require-trusted-types-for 'script'"), "{csp}");
    assert!(csp.ends_with("trusted-types abshare-sw svelte-trusted-html"), "{csp}");
    assert!(!csp.contains("unsafe-eval") && !csp.contains("script-src 'self' 'unsafe-inline'"), "{csp}");
    assert_eq!(resp.headers().get("x-frame-options").and_then(|v| v.to_str().ok()), Some("DENY"));
    // Via the trusted proxy with X-Forwarded-Proto: https it is accepted.
    let mut r = Request::builder()
        .uri("/healthz")
        .header("host", HOST)
        .header("x-forwarded-proto", "https")
        .body(Body::empty())
        .expect("req");
    r.extensions_mut().insert(Peer(SocketAddr::from(([10, 0, 0, 10], 5555))));
    assert_eq!(call(&t.app, r).await.status(), StatusCode::OK);
    // A stranger claiming https is not believed.
    let mut r = Request::builder()
        .uri("/healthz")
        .header("host", HOST)
        .header("x-forwarded-proto", "https")
        .body(Body::empty())
        .expect("req");
    r.extensions_mut().insert(Peer(SocketAddr::from(([198, 51, 100, 9], 5555))));
    assert_eq!(call(&t.app, r).await.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_wrong_host_is_refused() {
    let t = setup();
    let r = Request::builder().uri("/healthz").header("host", "evil.example").body(Body::empty()).expect("req");
    let resp = call(&t.app, r).await;
    assert_eq!(resp.status(), StatusCode::MISDIRECTED_REQUEST);
    assert!(resp.headers().get("content-security-policy").is_some());
    let r = Request::builder().uri("/healthz").body(Body::empty()).expect("req");
    assert_eq!(call(&t.app, r).await.status(), StatusCode::MISDIRECTED_REQUEST);
}

#[tokio::test]
async fn unknown_routes_are_404_and_api_answers_are_never_cached() {
    let t = setup();
    let r = send(&t, "GET", "/v1/nothing", None, &[]).await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    assert_eq!(r.headers().get("cache-control").and_then(|v| v.to_str().ok()), Some("no-store"));
}

#[tokio::test]
async fn cookie_routes_need_the_csrf_header_and_exact_origin() {
    let t = setup();
    for (hdr, origin) in [
        (None, Some(ORIGIN)),
        (Some("1"), None),
        (Some("1"), Some("https://evil.example")),
        (Some("0"), Some(ORIGIN)),
        (Some("1"), Some("https://books.example.com.evil.example")),
    ] {
        let mut b = Request::builder().method("POST").uri("/v1/auth/refresh").header("host", HOST);
        if let Some(h) = hdr {
            b = b.header("x-abshare-client", h);
        }
        if let Some(o) = origin {
            b = b.header("origin", o);
        }
        let r = call(&t.app, b.body(Body::empty()).expect("req")).await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "{hdr:?} {origin:?}");
    }
}

#[tokio::test]
async fn unknown_fields_and_oversize_bodies_are_refused() {
    let t = setup();
    let r = send(
        &t,
        "POST",
        "/v1/auth/login/password",
        Some(json!({"username":"a","password":"b","device_name":"c","admin":true})),
        &[],
    )
    .await;
    assert!(r.status().is_client_error(), "{}", r.status());
    let big = "x".repeat(40 * 1024);
    let r = send(
        &t,
        "POST",
        "/v1/auth/login/password",
        Some(json!({"username": big, "password":"b","device_name":"c"})),
        &[],
    )
    .await;
    assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

// ---------------------------------------------------------------- invites and enrolment

#[tokio::test]
async fn invites_must_be_real_unused_and_are_rate_limited() {
    let t = setup();
    for bad in ["", "short", "../../etc/passwd", &"a".repeat(64)] {
        let r = send(&t, "POST", "/v1/auth/invite/inspect", Some(json!({"token": bad})), &[]).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{bad}");
    }
    let mut limited = false;
    for _ in 0..40 {
        let r = send(&t, "POST", "/v1/auth/invite/inspect", Some(json!({"token": "a".repeat(64)})), &[]).await;
        limited |= r.status() == StatusCode::TOO_MANY_REQUESTS;
    }
    assert!(limited, "invite probing must be rate limited");
}

#[tokio::test]
async fn enrolment_with_password_and_totp_signs_in_with_safe_cookies() {
    let t = setup();
    let tok = invite(&t, Role::Listener);
    let r = send(&t, "POST", "/v1/auth/invite/inspect", Some(json!({"token": tok})), &[]).await;
    assert_eq!(json_of(r).await["display_name"], "Ada Example");
    let r = send(&t, "POST", "/v1/auth/enrol/start", Some(json!({"token": tok, "username": "Ada"})), &[]).await;
    let enrol = json_of(r).await["enrol"].as_str().expect("enrol").to_owned();
    // Weak passwords are refused; enrolment carries on.
    let r = send(&t, "POST", "/v1/auth/enrol/password", Some(json!({"enrol": enrol, "password": "short"})), &[]).await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let r = send(
        &t,
        "POST",
        "/v1/auth/enrol/password",
        Some(json!({"enrol": enrol, "password": "ada-is-my-password"})),
        &[],
    )
    .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST, "contains the username");
    // Finishing with no factor is refused.
    let r = send(&t, "POST", "/v1/auth/enrol/finish", Some(json!({"enrol": enrol, "device_name": "x"})), &[]).await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let r = send(
        &t,
        "POST",
        "/v1/auth/enrol/password",
        Some(json!({"enrol": enrol, "password": "correct horse battery"})),
        &[],
    )
    .await;
    let v = json_of(r).await;
    assert!(v["otpauth_uri"].as_str().is_some_and(|u| u.starts_with("otpauth://totp/ABShare:ada?")));
    assert!(v["qr_svg"].as_str().is_some_and(|s| s.contains("<svg")));
    let seed = b32_decode(v["secret"].as_str().expect("secret"));
    // A password without a confirmed code is not enough.
    let r = send(&t, "POST", "/v1/auth/enrol/finish", Some(json!({"enrol": enrol, "device_name": "x"})), &[]).await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let r = send(&t, "POST", "/v1/auth/enrol/totp", Some(json!({"enrol": enrol, "code": "000000"})), &[]).await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let r = send(&t, "POST", "/v1/auth/enrol/totp", Some(json!({"enrol": enrol, "code": code(&seed, 0)})), &[]).await;
    assert_eq!(r.status(), StatusCode::OK);
    let r =
        send(&t, "POST", "/v1/auth/enrol/finish", Some(json!({"enrol": enrol, "device_name": "Laptop"})), &[]).await;
    assert_eq!(r.status(), StatusCode::OK);
    for c in cookies(&r) {
        assert!(c.contains("HttpOnly") && c.contains("Secure") && c.contains("SameSite=Strict"), "{c}");
    }
    assert!(cookies(&r).iter().any(|c| c.starts_with("abshare_refresh=") && c.contains("Path=/v1/auth")));
    assert!(cookies(&r).iter().any(|c| c.starts_with("abshare_media=") && c.contains("Path=/v1/media")));
    let v = json_of(r).await;
    assert_eq!(v["recovery_codes"].as_array().map(Vec::len), Some(10));
    assert_eq!(v["user"]["username"], "ada");
    // The invite is used up.
    let r = send(&t, "POST", "/v1/auth/enrol/start", Some(json!({"token": tok, "username": "eve"})), &[]).await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn usernames_are_unique_and_validated() {
    let t = setup();
    enrol(&t, "taken").await;
    let tok = invite(&t, Role::Listener);
    for bad in ["Taken", ".x", "ab", "ad min", "аdmin"] {
        let r = send(&t, "POST", "/v1/auth/enrol/start", Some(json!({"token": tok, "username": bad})), &[]).await;
        assert!(r.status() == StatusCode::CONFLICT || r.status() == StatusCode::BAD_REQUEST, "{bad}: {}", r.status());
    }
}

#[tokio::test]
async fn enrolment_tokens_are_bound_to_the_server_and_expire_with_misuse() {
    let t = setup();
    let r = send(
        &t,
        "POST",
        "/v1/auth/enrol/password",
        Some(json!({"enrol": "f".repeat(64), "password": "correct horse battery"})),
        &[],
    )
    .await;
    assert_eq!(r.status(), StatusCode::GONE);
}

// ---------------------------------------------------------------- sign-in

#[tokio::test]
async fn wrong_passwords_and_unknown_users_get_the_same_answer() {
    let t = setup();
    enrol(&t, "bea").await;
    let a = send(
        &t,
        "POST",
        "/v1/auth/login/password",
        Some(json!({"username":"bea","password":"wrong wrong wrong","device_name":"x"})),
        &[],
    )
    .await;
    let b = send(
        &t,
        "POST",
        "/v1/auth/login/password",
        Some(json!({"username":"nobody","password":"wrong wrong wrong","device_name":"x"})),
        &[],
    )
    .await;
    assert_eq!(a.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(b.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(json_of(a).await, json_of(b).await);
}

#[tokio::test]
async fn repeated_failures_lock_the_address() {
    let t = setup();
    enrol(&t, "cai").await;
    let mut statuses = Vec::new();
    for _ in 0..7 {
        let r = send(
            &t,
            "POST",
            "/v1/auth/login/password",
            Some(json!({"username":"cai","password":"guess guess guess","device_name":"x"})),
            &[],
        )
        .await;
        statuses.push(r.status());
    }
    assert!(statuses.contains(&StatusCode::TOO_MANY_REQUESTS), "{statuses:?}");
    // Even the right password is refused while locked.
    let r = send(
        &t,
        "POST",
        "/v1/auth/login/password",
        Some(json!({"username":"cai","password":"correct horse battery","device_name":"x"})),
        &[],
    )
    .await;
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn a_forged_forwarded_for_cannot_dodge_the_lockout() {
    let t = setup();
    enrol(&t, "dee").await;
    let mut locked = false;
    for i in 0..8 {
        let mut r = req("POST", "/v1/auth/login/password", Some(json!({})))
            .header("x-forwarded-proto", "https")
            // The client prepends a different address each time; the proxy appends the real one.
            .header("x-forwarded-for", format!("192.0.2.{i}, 203.0.113.7"))
            .body(Body::from(json!({"username":"dee","password":"guess guess guess","device_name":"x"}).to_string()))
            .expect("req");
        r.extensions_mut().insert(Peer(SocketAddr::from(([10, 0, 0, 10], 4000))));
        locked |= call(&t.app, r).await.status() == StatusCode::TOO_MANY_REQUESTS;
    }
    assert!(locked);
}

#[tokio::test]
async fn password_then_totp_signs_in_and_codes_and_challenges_are_single_use() {
    let t = setup();
    let (seed, ..) = enrol(&t, "eve").await;
    let start = || async {
        let r = send(
            &t,
            "POST",
            "/v1/auth/login/password",
            Some(json!({"username":"eve","password":"correct horse battery","device_name":"Phone"})),
            &[],
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let v = json_of(r).await;
        assert_eq!(v["methods"], json!(["totp", "recovery"]));
        v["challenge"].as_str().expect("challenge").to_owned()
    };
    // The code already used during enrolment is refused (replay).
    let ch = start().await;
    let r = send(&t, "POST", "/v1/auth/login/totp", Some(json!({"challenge": ch, "code": code(&seed, 0)})), &[]).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    // A failed challenge can't be retried.
    let r = send(&t, "POST", "/v1/auth/login/totp", Some(json!({"challenge": ch, "code": code(&seed, 1)})), &[]).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let ch = start().await;
    let r = send(&t, "POST", "/v1/auth/login/totp", Some(json!({"challenge": ch, "code": code(&seed, 1)})), &[]).await;
    assert_eq!(r.status(), StatusCode::OK);
    let access = json_of(r).await["access_token"].as_str().expect("access").to_owned();
    let r = send(&t, "GET", "/v1/me", None, &[("authorization", &format!("Bearer {access}"))]).await;
    assert_eq!(json_of(r).await["username"], "eve");
    // The challenge was consumed by the success.
    let r = send(&t, "POST", "/v1/auth/login/totp", Some(json!({"challenge": ch, "code": code(&seed, 1)})), &[]).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn recovery_codes_work_once() {
    let t = setup();
    let (_, _, _, codes) = enrol(&t, "fay").await;
    for (expect, c) in [
        (StatusCode::OK, codes[0].to_uppercase()),
        (StatusCode::UNAUTHORIZED, codes[0].clone()),
        (StatusCode::UNAUTHORIZED, "zzzz-zzzz-zzzz".into()),
    ] {
        let r = send(
            &t,
            "POST",
            "/v1/auth/login/password",
            Some(json!({"username":"fay","password":"correct horse battery","device_name":"x"})),
            &[],
        )
        .await;
        let ch = json_of(r).await["challenge"].as_str().expect("challenge").to_owned();
        let r = send(&t, "POST", "/v1/auth/login/recovery", Some(json!({"challenge": ch, "code": c})), &[]).await;
        assert_eq!(r.status(), expect);
    }
}

#[tokio::test]
async fn bearer_routes_refuse_missing_garbage_and_expired_tokens() {
    let t = setup();
    for h in [
        None,
        Some("Bearer "),
        Some("Bearer abc"),
        Some(&*format!("Bearer {}", "a".repeat(64))),
        Some("Basic Zm9vOmJhcg=="),
    ] {
        let extra: Vec<(&str, &str)> = h.map(|v| vec![("authorization", v)]).unwrap_or_default();
        let r = send(&t, "GET", "/v1/me", None, &extra).await;
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "{h:?}");
    }
}

// ---------------------------------------------------------------- refresh and sign-out

#[tokio::test]
async fn refresh_rotates_and_a_stolen_old_token_signs_the_device_out() {
    let t = setup();
    let (_, access, refresh, _) = enrol(&t, "gus").await;
    let r = send(&t, "POST", "/v1/auth/refresh", None, &[("cookie", &format!("abshare_refresh={refresh}"))]).await;
    assert_eq!(r.status(), StatusCode::OK);
    let new = cookie_value(&r, "abshare_refresh");
    assert_ne!(new, refresh);
    // The legitimate client uses its new token...
    let r = send(&t, "POST", "/v1/auth/refresh", None, &[("cookie", &format!("abshare_refresh={new}"))]).await;
    assert_eq!(r.status(), StatusCode::OK);
    // ...so a thief replaying the original is detected, and the whole device is signed out.
    let r = send(&t, "POST", "/v1/auth/refresh", None, &[("cookie", &format!("abshare_refresh={refresh}"))]).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    // Reuse is "signed out for safety", not "removed": the app keeps this device's downloads.
    assert_eq!(json_of(r).await["error"], "session_reused");
    let r = send(&t, "GET", "/v1/me", None, &[("authorization", &format!("Bearer {access}"))]).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn refresh_without_or_with_a_forged_cookie_is_refused() {
    let t = setup();
    for c in [None, Some("abshare_refresh=nope"), Some(&*format!("abshare_refresh={}", "b".repeat(64)))] {
        let extra: Vec<(&str, &str)> = c.map(|v| vec![("cookie", v)]).unwrap_or_default();
        assert_eq!(send(&t, "POST", "/v1/auth/refresh", None, &extra).await.status(), StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn logout_revokes_everything_and_clears_the_cache() {
    let t = setup();
    let (_, access, refresh, _) = enrol(&t, "hal").await;
    let r = send(&t, "POST", "/v1/auth/logout", None, &[("cookie", &format!("abshare_refresh={refresh}"))]).await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert_eq!(r.headers().get("clear-site-data").and_then(|v| v.to_str().ok()), Some("\"cache\""));
    assert!(cookies(&r).iter().all(|c| c.contains("Max-Age=0")));
    assert_eq!(
        send(&t, "GET", "/v1/me", None, &[("authorization", &format!("Bearer {access}"))]).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&t, "POST", "/v1/auth/refresh", None, &[("cookie", &format!("abshare_refresh={refresh}"))]).await.status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_remotely_signed_out_device_learns_it_was_revoked() {
    let t = setup();
    let (seed, access, _, _) = enrol(&t, "kit").await;
    reset_totp_step(&t, "kit");
    let r = send(
        &t,
        "POST",
        "/v1/auth/login/password",
        Some(json!({"username":"kit","password":"correct horse battery","device_name":"Tablet"})),
        &[],
    )
    .await;
    let ch = json_of(r).await["challenge"].as_str().expect("challenge").to_owned();
    let r = send(&t, "POST", "/v1/auth/login/totp", Some(json!({"challenge": ch, "code": code(&seed, 0)})), &[]).await;
    let tablet_refresh = cookie_value(&r, "abshare_refresh");
    let r = send(&t, "GET", "/v1/me/devices", None, &[("authorization", &format!("Bearer {access}"))]).await;
    let devices = json_of(r).await["devices"].as_array().expect("devices").clone();
    let tablet = devices.iter().find(|d| d["this_device"] == false).expect("tablet")["id"].as_i64().expect("id");
    // Signing out another device needs a fresh factor.
    let r = send(
        &t,
        "DELETE",
        &format!("/v1/me/devices/{tablet}"),
        None,
        &[("authorization", &format!("Bearer {access}"))],
    )
    .await;
    assert_eq!(json_of(r).await["error"], "step_up_required");
    step_up(&t, "kit", &seed, &access).await;
    let r = send(
        &t,
        "DELETE",
        &format!("/v1/me/devices/{tablet}"),
        None,
        &[("authorization", &format!("Bearer {access}"))],
    )
    .await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let r =
        send(&t, "POST", "/v1/auth/refresh", None, &[("cookie", &format!("abshare_refresh={tablet_refresh}"))]).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(json_of(r).await["error"], "device_revoked");
    // A forged or unknown token is just "sign in", never "revoked".
    let r =
        send(&t, "POST", "/v1/auth/refresh", None, &[("cookie", &format!("abshare_refresh={}", "a".repeat(64)))]).await;
    assert_eq!(json_of(r).await["error"], "unauthenticated");
}

// ---------------------------------------------------------------- step-up and account changes

#[tokio::test]
async fn account_changes_need_step_up_and_sign_out_other_devices() {
    let t = setup();
    let (seed, access, ..) = enrol(&t, "ivy").await;
    let auth = format!("Bearer {access}");
    // A second device.
    reset_totp_step(&t, "ivy");
    let r = send(
        &t,
        "POST",
        "/v1/auth/login/password",
        Some(json!({"username":"ivy","password":"correct horse battery","device_name":"Tablet"})),
        &[],
    )
    .await;
    let ch = json_of(r).await["challenge"].as_str().expect("challenge").to_owned();
    let r = send(&t, "POST", "/v1/auth/login/totp", Some(json!({"challenge": ch, "code": code(&seed, 0)})), &[]).await;
    let other = json_of(r).await["access_token"].as_str().expect("access").to_owned();
    // Without step-up: refused.
    let r = send(
        &t,
        "POST",
        "/v1/me/password",
        Some(json!({"password": "another long password"})),
        &[("authorization", &auth)],
    )
    .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_of(r).await["error"], "step_up_required");
    // Password alone is never a step-up: there is no such route (review A4).
    let r = send(
        &t,
        "POST",
        "/v1/auth/step-up/password",
        Some(json!({"password": "correct horse battery"})),
        &[("authorization", &auth)],
    )
    .await;
    assert_ne!(r.status(), StatusCode::OK);
    reset_totp_step(&t, "ivy");
    let r = send(
        &t,
        "POST",
        "/v1/auth/step-up/totp",
        Some(json!({"password": "correct horse battery", "code": code(&seed, 0)})),
        &[("authorization", &auth)],
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let r = send(
        &t,
        "POST",
        "/v1/me/password",
        Some(json!({"password": "another long password"})),
        &[("authorization", &auth)],
    )
    .await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    // The other device was signed out; this one wasn't.
    assert_eq!(
        send(&t, "GET", "/v1/me", None, &[("authorization", &format!("Bearer {other}"))]).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(send(&t, "GET", "/v1/me", None, &[("authorization", &auth)]).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn the_last_sign_in_method_cannot_be_removed() {
    let t = setup();
    let (seed, access, ..) = enrol(&t, "joe").await;
    let auth = format!("Bearer {access}");
    reset_totp_step(&t, "joe");
    send(
        &t,
        "POST",
        "/v1/auth/step-up/totp",
        Some(json!({"password": "correct horse battery", "code": code(&seed, 0)})),
        &[("authorization", &auth)],
    )
    .await;
    let r = send(&t, "DELETE", "/v1/me/totp", None, &[("authorization", &auth)]).await;
    assert_eq!(r.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn nobody_can_revoke_another_users_device() {
    let t = setup();
    let (kim_seed, a, ..) = enrol(&t, "kim").await;
    let (_, b, ..) = enrol(&t, "lee").await;
    let r = send(&t, "GET", "/v1/me/devices", None, &[("authorization", &format!("Bearer {b}"))]).await;
    let lee_device = json_of(r).await["devices"][0]["id"].as_i64().expect("id");
    step_up(&t, "kim", &kim_seed, &a).await;
    let r =
        send(&t, "DELETE", &format!("/v1/me/devices/{lee_device}"), None, &[("authorization", &format!("Bearer {a}"))])
            .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        send(&t, "GET", "/v1/me", None, &[("authorization", &format!("Bearer {b}"))]).await.status(),
        StatusCode::OK
    );
    let r = send(&t, "DELETE", "/v1/me/devices/not-a-number", None, &[("authorization", &format!("Bearer {a}"))]).await;
    assert!(r.status().is_client_error());
}

#[tokio::test]
async fn the_audit_log_records_and_verifies() {
    let t = setup();
    enrol(&t, "max").await;
    send(
        &t,
        "POST",
        "/v1/auth/login/password",
        Some(json!({"username":"max","password":"wrong wrong wrong","device_name":"x"})),
        &[],
    )
    .await;
    let c = t.st.db.0.lock().expect("db");
    let events: Vec<String> = c
        .prepare("SELECT event FROM audit ORDER BY id")
        .expect("q")
        .query_map([], |r| r.get(0))
        .expect("rows")
        .collect::<Result<_, _>>()
        .expect("events");
    assert!(
        events.contains(&"enrol.ok".to_owned())
            && events.contains(&"login.ok".to_owned())
            && events.contains(&"login.failed".to_owned()),
        "{events:?}"
    );
    assert!(crate::auth::audit::verify(&c, &t.st.audit_key).is_ok());
}

// ---------------------------------------------------------------- media

const BOOK: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaa";
const KIDS_BOOK: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbb";
const ESCAPE_BOOK: &str = "cccccccccccccccccccccccccc";
const MULTI_BOOK: &str = "dddddddddddddddddddddddddd";
const COVER: &str = "0123456789abcdef";

struct M {
    t: T,
    dir: std::path::PathBuf,
}

impl Drop for M {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A router with media enabled over a scratch library: one plain book, one in a library the
/// user can't see, one whose part is a symlink out of the root, and one needing a copy.
/// A scan record for a file of the given container.
fn probe_json(container: &str) -> String {
    serde_json::to_string(&crate::probe::ProbeRecord {
        container: container.into(),
        codec: "mp3".into(),
        profile: String::new(),
        duration_ms: 3_600_000,
        bitrate: 64_000,
        sample_rate: 44_100,
        channels: 2,
        tags: vec![],
        chapters: vec![],
        has_cover: false,
        decodes: true,
        problem: None,
    })
    .expect("probe json")
}

fn setup_media() -> M {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("abshare-media-test-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (lib, kids, data) = (dir.join("lib"), dir.join("kids"), dir.join("data"));
    for d in [&lib, &kids, &data] {
        std::fs::create_dir_all(d).expect("mkdir");
    }
    let audio: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(lib.join("book.mp3"), &audio).expect("write");
    std::fs::write(lib.join("part1.mp3"), &audio).expect("write");
    std::fs::write(kids.join("kids.mp3"), &audio).expect("write");
    std::fs::write(dir.join("secret.mp3"), b"outside the library").expect("write");
    std::os::unix::fs::symlink(dir.join("secret.mp3"), lib.join("escape.mp3")).expect("symlink");
    let cover_dir = data.join("covers").join(BOOK);
    std::fs::create_dir_all(&cover_dir).expect("mkdir");
    std::fs::write(cover_dir.join(format!("{COVER}-320.jpg")), b"jpeg").expect("write");

    let conn = crate::db::open(std::path::Path::new(":memory:")).expect("db");
    let rows = [
        (BOOK, 1, "book.mp3", 0, Some(COVER), r#"{"title":"A Book","genres":["Hard Science Fiction","Adventure"]}"#),
        (
            KIDS_BOOK,
            2,
            "kids.mp3",
            0,
            None,
            r#"{"title":"A Book","genres":["Biographies & Memoirs","Science Fiction"]}"#,
        ),
        (ESCAPE_BOOK, 1, "escape.mp3", 0, None, r#"{"title":"A Book"}"#),
        (MULTI_BOOK, 1, "part1.mp3", 1, None, r#"{"title":"A Book","genres":["Biographies & Memoirs"]}"#),
    ];
    conn.execute(
        "INSERT INTO library (id, name, root) VALUES (1, 'Audiobooks', ?1), (2, 'Kids', ?2)",
        [lib.to_string_lossy(), kids.to_string_lossy()],
    )
    .expect("libraries");
    for (id, library, part, copy, cover, meta) in rows {
        conn.execute(
            "INSERT INTO book (id, library_id, folder, meta, duration_ms, needs_copy, cover, updated_at)
             VALUES (?1, ?2, '', ?5, 3600000, ?3, ?4, 1)",
            rusqlite::params![id, library, copy, cover, meta],
        )
        .expect("book");
        conn.execute("INSERT INTO book_part (book_id, ord, rel_path) VALUES (?1, 0, ?2)", [id, part]).expect("part");
        // What a scan records: the size and the probed container.
        conn.execute(
            "INSERT OR IGNORE INTO file (library_id, rel_path, size, mtime_ns, probe, probed_at) VALUES (?1, ?2, 1000, 0, ?3, 1)",
            rusqlite::params![library, part, probe_json("mp3")],
        )
        .expect("file");
    }
    let mut st = AppState::new(
        conn,
        &Secret::for_tests([5; 32]),
        ORIGIN,
        vec!["10.0.0.10".parse().expect("ip")],
        Hasher::for_tests(),
    )
    .expect("state");
    let ff = crate::config::Ffmpeg {
        // Never run here: the tests stay off the transcode paths.
        ffmpeg: "/nonexistent/ffmpeg".into(),
        ffprobe: "/nonexistent/ffprobe".into(),
        jail: "/nonexistent/jail".into(),
        libs: vec![],
        jobs: 1,
    };
    let roots = [("Audiobooks".to_owned(), lib), ("Kids".to_owned(), kids)].into_iter().collect();
    st.media = Some(Arc::new(crate::media::Media::new(ff, data, roots)));
    let st = Arc::new(st);
    M { t: T { app: router(st.clone()), st }, dir }
}

/// A listener who can see only the "Audiobooks" library; returns (access token, media cookie).
async fn listener(m: &M) -> (String, String) {
    let (_, access, _, _, media) = enrol_with_media(&m.t, "joe").await;
    let c = m.t.st.db.0.lock().expect("db");
    c.execute("UPDATE user SET libraries = '[\"Audiobooks\"]' WHERE username = 'joe'", []).expect("perms");
    (access, media)
}

async fn body_of(r: Response) -> Vec<u8> {
    axum::body::to_bytes(r.into_body(), 1 << 20).await.expect("body").to_vec()
}

#[tokio::test]
async fn media_needs_a_valid_cookie_or_token() {
    let m = setup_media();
    let (access, media) = listener(&m).await;
    let uri = format!("/v1/media/{BOOK}");
    for extra in [vec![], vec![("cookie", "abshare_media=forged")], vec![("authorization", "Bearer forged")]] {
        let r = send(&m.t, "GET", &uri, None, &extra).await;
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "{extra:?}");
    }
    // The refresh cookie is not a media credential, and neither is the access token as a cookie.
    let r = send(&m.t, "GET", &uri, None, &[("cookie", &format!("abshare_media={access}"))]).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let r = send(&m.t, "GET", &uri, None, &[("cookie", &format!("abshare_media={media}"))]).await;
    assert_eq!(r.status(), StatusCode::OK);
    let h = r.headers().clone();
    assert_eq!(h["content-type"], "audio/mpeg");
    assert_eq!(h["cache-control"], "private, no-store");
    assert_eq!(h["content-length"], "1000");
    assert_eq!(body_of(r).await.len(), 1000);
    let r = send(&m.t, "GET", &uri, None, &[("authorization", &format!("Bearer {access}"))]).await;
    assert_eq!(r.status(), StatusCode::OK);
}

#[tokio::test]
async fn media_ranges_are_exact_and_bad_ones_refused() {
    let m = setup_media();
    let (access, _) = listener(&m).await;
    let auth = format!("Bearer {access}");
    let uri = format!("/v1/media/{BOOK}");
    let r = send(&m.t, "GET", &uri, None, &[("authorization", &auth), ("range", "bytes=10-19")]).await;
    assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(r.headers()["content-range"], "bytes 10-19/1000");
    let expected: Vec<u8> = (10..20u8).collect();
    assert_eq!(body_of(r).await, expected);
    let r = send(&m.t, "GET", &uri, None, &[("authorization", &auth), ("range", "bytes=-5")]).await;
    assert_eq!(r.headers()["content-range"], "bytes 995-999/1000");
    assert_eq!(body_of(r).await.len(), 5);
    let r = send(&m.t, "GET", &uri, None, &[("authorization", &auth), ("range", "bytes=990-5000")]).await;
    assert_eq!(r.headers()["content-range"], "bytes 990-999/1000");
    let r = send(&m.t, "GET", &uri, None, &[("authorization", &auth), ("range", "bytes=1000-")]).await;
    assert_eq!(r.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(r.headers()["content-range"], "bytes */1000");
}

#[tokio::test]
async fn media_outside_the_users_libraries_or_root_is_not_found() {
    let m = setup_media();
    let (access, _) = listener(&m).await;
    let auth = format!("Bearer {access}");
    for uri in [
        format!("/v1/media/{KIDS_BOOK}"),
        format!("/v1/media/{ESCAPE_BOOK}"),
        "/v1/media/eeeeeeeeeeeeeeeeeeeeeeeeee".to_owned(),
        "/v1/media/..%2f..%2fetc%2fpasswd".to_owned(),
        "/v1/media/AAAAAAAAAAAAAAAAAAAAAAAAAA".to_owned(),
        format!("/v1/media/covers/{KIDS_BOOK}/{COVER}"),
        format!("/v1/media/covers/{BOOK}/fedcba9876543210"),
        format!("/v1/media/covers/{BOOK}/{COVER}?size=321"),
    ] {
        let r = send(&m.t, "GET", &uri, None, &[("authorization", &auth)]).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{uri}");
        assert!(!body_of(r).await.windows(7).any(|w| w == b"outside"), "{uri}");
    }
}

#[tokio::test]
async fn media_qualities_are_validated_and_copies_report_preparing() {
    let m = setup_media();
    let (access, _) = listener(&m).await;
    let auth = format!("Bearer {access}");
    for q in ["64", "abc", "-1", "48.0"] {
        let r = send(&m.t, "GET", &format!("/v1/media/{BOOK}?q={q}"), None, &[("authorization", &auth)]).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{q}");
    }
    let r = send(&m.t, "GET", &format!("/v1/media/{BOOK}?q=48&x=1"), None, &[("authorization", &auth)]).await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST, "unknown parameters are refused");
    for dl in ["2", "yes", "-1", "256"] {
        let r = send(&m.t, "GET", &format!("/v1/media/{BOOK}?q=48&dl={dl}"), None, &[("authorization", &auth)]).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "dl={dl}");
    }
    let r = send(&m.t, "GET", &format!("/v1/media/{MULTI_BOOK}"), None, &[("authorization", &auth)]).await;
    assert_eq!(r.status(), StatusCode::CONFLICT);
    assert_eq!(json_of(r).await["error"], "preparing");
    let r = send(&m.t, "GET", &format!("/v1/media/{BOOK}/state"), None, &[("authorization", &auth)]).await;
    assert_eq!(json_of(r).await["ready"], true);
}

#[tokio::test]
async fn covers_are_served_immutable_to_permitted_users() {
    let m = setup_media();
    let (_, media) = listener(&m).await;
    let r = send(
        &m.t,
        "GET",
        &format!("/v1/media/covers/{BOOK}/{COVER}"),
        None,
        &[("cookie", &format!("abshare_media={media}"))],
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["content-type"], "image/jpeg");
    assert_eq!(r.headers()["cache-control"], "private, max-age=31536000, immutable");
    assert_eq!(body_of(r).await, b"jpeg");
}

#[tokio::test]
async fn the_web_app_is_served_with_safe_caching() {
    let m = setup_media();
    let web = m.dir.join("web");
    std::fs::create_dir_all(web.join("_app/immutable")).expect("mkdir");
    std::fs::write(web.join("index.html"), "<!doctype html>").expect("write");
    std::fs::write(web.join("_app/immutable/app-1a2b.js"), "x").expect("write");
    std::fs::write(m.dir.join("private.txt"), "nope").expect("write");
    let mut st = AppState::new(
        crate::db::open(std::path::Path::new(":memory:")).expect("db"),
        &Secret::for_tests([5; 32]),
        ORIGIN,
        vec![],
        Hasher::for_tests(),
    )
    .expect("state");
    st.web_dir = Some(web);
    let t = T { app: router(Arc::new(st)), st: m.t.st.clone() };
    let r = send(&t, "GET", "/", None, &[]).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["cache-control"], "no-cache");
    assert!(r.headers().contains_key("content-security-policy"));
    let r = send(&t, "GET", "/_app/immutable/app-1a2b.js", None, &[]).await;
    assert_eq!(r.headers()["cache-control"], "public, max-age=31536000, immutable");
    assert_eq!(r.headers()["content-type"], "text/javascript; charset=utf-8");
    for bad in ["/../private.txt", "/%2e%2e/private.txt", "/.hidden", "/v1/nope", "/missing.js"] {
        let r = send(&t, "GET", bad, None, &[]).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{bad}");
    }
    let r = send(&t, "POST", "/", None, &[]).await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------- library, bookmarks, progress

async fn get_json(t: &T, uri: &str, access: &str) -> (StatusCode, Value) {
    let r = send(t, "GET", uri, None, &[("authorization", &format!("Bearer {access}"))]).await;
    (r.status(), json_of(r).await)
}

#[tokio::test]
async fn the_library_shows_only_permitted_books() {
    let m = setup_media();
    let (access, _) = listener(&m).await;
    let (s, v) = get_json(&m.t, "/v1/libraries", &access).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["libraries"], json!([{ "id": 1, "name": "Audiobooks" }]));
    let (_, v) = get_json(&m.t, "/v1/books", &access).await;
    let ids: Vec<&str> = v["items"].as_array().expect("items").iter().filter_map(|b| b["id"].as_str()).collect();
    assert_eq!(v["total"], 3);
    assert!(!ids.contains(&KIDS_BOOK), "{ids:?}");
    assert_eq!(get_json(&m.t, &format!("/v1/books/{BOOK}"), &access).await.0, StatusCode::OK);
    for uri in [format!("/v1/books/{KIDS_BOOK}"), format!("/v1/books/{KIDS_BOOK}/bookmarks"), "/v1/books/nope".into()] {
        assert_eq!(get_json(&m.t, &uri, &access).await.0, StatusCode::NOT_FOUND, "{uri}");
    }
    for uri in ["/v1/books?cursor=x", "/v1/books?bogus=1"] {
        assert_eq!(get_json(&m.t, uri, &access).await.0, StatusCode::BAD_REQUEST, "{uri}");
    }
    assert_eq!(get_json(&m.t, "/v1/books", "forged").await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn bookmarks_belong_to_their_owner() {
    let m = setup_media();
    let (joe, _) = listener(&m).await;
    let (_, amy, _, _) = enrol(&m.t, "amy").await;
    let auth = |a: &str| format!("Bearer {a}");
    let uri = format!("/v1/books/{BOOK}/bookmarks");
    let r = send(
        &m.t,
        "POST",
        &uri,
        Some(json!({"position_ms": 99_999_999_999u64, "note": "a <b>note</b>"})),
        &[("authorization", &auth(&joe))],
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let id = json_of(r).await["id"].as_i64().expect("id");
    let (_, v) = get_json(&m.t, &uri, &joe).await;
    assert_eq!(v["bookmarks"][0]["position_ms"], 3_600_000, "positions are clamped to the book");
    assert_eq!(get_json(&m.t, &uri, &amy).await.1["bookmarks"], json!([]));
    let b = format!("/v1/bookmarks/{id}");
    let r = send(&m.t, "PATCH", &b, Some(json!({"note": "mine now"})), &[("authorization", &auth(&amy))]).await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let r = send(&m.t, "DELETE", &b, None, &[("authorization", &auth(&amy))]).await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let r = send(&m.t, "DELETE", &b, None, &[("authorization", &auth(&joe))]).await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn progress_is_idempotent_validated_and_private() {
    let m = setup_media();
    let (joe, _) = listener(&m).await;
    let (_, amy, _, _) = enrol(&m.t, "amy").await;
    let put = |book: &str, body: Value, who: &str| {
        let (uri, auth) = (format!("/v1/progress/{book}"), format!("Bearer {who}"));
        let t = &m.t;
        async move { send(t, "PUT", &uri, Some(body), &[("authorization", &auth)]).await }
    };
    let t0 = super::now() * 1000;
    let ev = json!({"key": "k-00000001", "event_time_ms": t0, "action": "position", "position_ms": 60_000});
    let r = put(BOOK, ev.clone(), &joe).await;
    let st = r.status();
    assert_eq!(st, StatusCode::OK, "{}", String::from_utf8_lossy(&body_of(r).await));
    assert_eq!(json_of(r).await["result"], "accepted");
    assert_eq!(json_of(put(BOOK, ev, &joe).await).await["result"], "duplicate");
    let older = json!({"key": "k-00000002", "event_time_ms": t0 - 500, "action": "position", "position_ms": 1});
    assert_eq!(json_of(put(BOOK, older, &joe).await).await["result"], "stale");
    for bad in [
        json!({"key": "short", "event_time_ms": t0, "action": "position", "position_ms": 1}),
        json!({"key": "k-0000000%", "event_time_ms": t0, "action": "position", "position_ms": 1}),
        json!({"key": "k-00000003", "event_time_ms": t0, "action": "teleport"}),
        json!({"key": "k-00000003", "event_time_ms": t0, "action": "position"}),
    ] {
        assert_eq!(put(BOOK, bad.clone(), &joe).await.status(), StatusCode::BAD_REQUEST, "{bad}");
    }
    let kids = json!({"key": "k-00000004", "event_time_ms": t0 + 1, "action": "finished"});
    assert_eq!(put(KIDS_BOOK, kids, &joe).await.status(), StatusCode::NOT_FOUND);
    let (_, v) = get_json(&m.t, "/v1/progress", &joe).await;
    assert_eq!(v["items"][0]["book"], BOOK);
    assert_eq!(v["items"][0]["position_ms"], 60_000);
    assert_eq!(get_json(&m.t, "/v1/progress", &amy).await.1["items"], json!([]));
}

// ---------------------------------------------------------------- listening history

/// Reports a position as `who` at `at` ms; returns the response status.
async fn report(t: &T, who: &str, book: &str, at: i64, position_ms: u64) -> StatusCode {
    static KEY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let key = format!("hist-{:08}", KEY.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    let body = json!({"key": key, "event_time_ms": at, "action": "position", "position_ms": position_ms});
    as_user(t, "PUT", &format!("/v1/progress/{book}"), Some(body), who).await.status()
}

/// Listens to `book` for `secs` seconds from `at`, reporting every 15 s as the app does.
async fn listen_to(t: &T, who: &str, book: &str, at: i64, from_ms: u64, secs: u64) {
    for i in 0..=secs / 15 {
        assert_eq!(report(t, who, book, at + (i * 15_000) as i64, from_ms + i * 15_000).await, StatusCode::OK);
    }
}

#[tokio::test]
async fn listening_history_is_built_from_progress_and_seen_only_by_its_owner() {
    let m = setup_media();
    let (joe, _) = listener(&m).await;
    let (_, amy, _, _) = enrol(&m.t, "amy").await;
    let t0 = super::now() * 1000 - 600_000;
    listen_to(&m.t, &joe, BOOK, t0, 60_000, 45).await;
    // A seek forward and a pause add nothing; the session carries on.
    assert_eq!(report(&m.t, &joe, BOOK, t0 + 60_000, 900_000).await, StatusCode::OK);
    assert_eq!(report(&m.t, &joe, BOOK, t0 + 75_000, 900_000).await, StatusCode::OK);
    let (s, v) = get_json(&m.t, "/v1/me/history", &joe).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["enabled"], true);
    assert_eq!(v["sessions"].as_array().map(Vec::len), Some(1), "{v}");
    let session = &v["sessions"][0];
    assert_eq!(session["book"]["id"], BOOK);
    assert_eq!(session["book"]["title"], "A Book");
    assert_eq!((session["started_at"].as_i64(), session["ended_at"].as_i64()), (Some(t0), Some(t0 + 75_000)));
    assert_eq!(session["seconds"], 45);
    assert_eq!(v["totals"], json!({"week_seconds": 45, "month_seconds": 45, "all_seconds": 45, "books_finished": 0}));
    assert_eq!(v["next"], Value::Null);

    // Another person sees none of it, and clearing theirs leaves it alone.
    let (_, v) = get_json(&m.t, "/v1/me/history", &amy).await;
    assert_eq!(v["sessions"], json!([]));
    assert_eq!(v["totals"]["all_seconds"], 0);
    assert_eq!(as_user(&m.t, "DELETE", "/v1/me/history", None, &amy).await.status(), StatusCode::NO_CONTENT);
    assert_eq!(get_json(&m.t, "/v1/me/history", &joe).await.1["totals"]["all_seconds"], 45);

    // A long pause starts a new session.
    listen_to(&m.t, &joe, BOOK, t0 + 300_000, 900_000, 30).await;
    let (_, v) = get_json(&m.t, "/v1/me/history", &joe).await;
    assert_eq!(v["sessions"].as_array().map(Vec::len), Some(2), "{v}");
    assert_eq!(v["sessions"][0]["seconds"], 30, "newest first");
    assert_eq!(v["totals"]["all_seconds"], 75);

    // Clearing removes it all.
    assert_eq!(as_user(&m.t, "DELETE", "/v1/me/history", None, &joe).await.status(), StatusCode::NO_CONTENT);
    let (_, v) = get_json(&m.t, "/v1/me/history", &joe).await;
    assert_eq!((v["sessions"].clone(), v["totals"]["all_seconds"].clone()), (json!([]), json!(0)));
    // Without a valid token there's nothing.
    for method in ["GET", "DELETE"] {
        let r = send(&m.t, method, "/v1/me/history", None, &[("authorization", "Bearer forged")]).await;
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "{method}");
    }
    let r = send(&m.t, "PATCH", "/v1/me/history", Some(json!({"enabled": false})), &[]).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn listening_history_can_be_turned_off_and_its_requests_are_strict() {
    let m = setup_media();
    let (joe, _) = listener(&m).await;
    let r = as_user(&m.t, "PATCH", "/v1/me/history", Some(json!({"enabled": false})), &joe).await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let t0 = super::now() * 1000 - 600_000;
    listen_to(&m.t, &joe, BOOK, t0, 0, 30).await;
    let (_, v) = get_json(&m.t, "/v1/me/history", &joe).await;
    assert_eq!((v["enabled"].clone(), v["sessions"].clone()), (json!(false), json!([])));
    let n: i64 = {
        let c = m.t.st.db.0.lock().expect("db");
        c.query_row("SELECT count(*) FROM listening", [], |r| r.get(0)).expect("count")
    };
    assert_eq!(n, 0, "nothing is stored while history is off");
    // Progress itself is still kept.
    assert_eq!(get_json(&m.t, "/v1/progress", &joe).await.1["items"][0]["position_ms"], 30_000);
    let r = as_user(&m.t, "PATCH", "/v1/me/history", Some(json!({"enabled": true})), &joe).await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    listen_to(&m.t, &joe, BOOK, t0 + 300_000, 30_000, 15).await;
    assert_eq!(get_json(&m.t, "/v1/me/history", &joe).await.1["totals"]["all_seconds"], 15);

    for bad in [json!({"enabled": false, "user": 1}), json!({"enabled": "no"}), json!({}), json!([false]), json!(null)]
    {
        let r = as_user(&m.t, "PATCH", "/v1/me/history", Some(bad.clone()), &joe).await;
        assert!(r.status().is_client_error(), "{bad}: {}", r.status());
    }
    assert_eq!(get_json(&m.t, "/v1/me/history", &joe).await.1["enabled"], true, "refused bodies change nothing");
    for uri in [
        "/v1/me/history?bogus=1",
        "/v1/me/history?user=1",
        "/v1/me/history?limit=0",
        "/v1/me/history?limit=101",
        "/v1/me/history?limit=x",
        "/v1/me/history?limit=-1",
        "/v1/me/history?cursor=abc",
        "/v1/me/history?cursor=0",
        "/v1/me/history?cursor=-5",
        "/v1/me/history?cursor=%2B5",
        "/v1/me/history?cursor=99999999999999999999",
    ] {
        assert_eq!(get_json(&m.t, uri, &joe).await.0, StatusCode::BAD_REQUEST, "{uri}");
    }
    // Changes are rate-limited per device.
    let mut limited = false;
    for _ in 0..12 {
        let r = as_user(&m.t, "PATCH", "/v1/me/history", Some(json!({"enabled": true})), &joe).await;
        limited |= r.status() == StatusCode::TOO_MANY_REQUESTS;
    }
    assert!(limited);
    assert_eq!(as_user(&m.t, "DELETE", "/v1/me/history", None, &joe).await.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn listening_history_pages_and_leaves_out_books_the_listener_cannot_see() {
    let m = setup_media();
    let (_, amy, _, _) = enrol(&m.t, "amy").await;
    let t0 = super::now() * 1000 - 3_600_000;
    for (i, book) in [BOOK, MULTI_BOOK, ESCAPE_BOOK, KIDS_BOOK].iter().enumerate() {
        listen_to(&m.t, &amy, book, t0 + i as i64 * 600_000, 0, 15).await;
    }
    let finished = json!({"key": "hist-finish-1", "event_time_ms": t0 + 3_000_000, "action": "finished"});
    assert_eq!(
        as_user(&m.t, "PUT", &format!("/v1/progress/{KIDS_BOOK}"), Some(finished), &amy).await.status(),
        StatusCode::OK
    );
    let (_, v) = get_json(&m.t, "/v1/me/history?limit=3", &amy).await;
    let ids: Vec<&str> =
        v["sessions"].as_array().expect("sessions").iter().filter_map(|s| s["book"]["id"].as_str()).collect();
    assert_eq!(ids, vec![KIDS_BOOK, ESCAPE_BOOK, MULTI_BOOK]);
    assert_eq!(v["totals"]["books_finished"], 1);
    let next = v["next"].as_str().expect("next").to_owned();
    let (_, v) = get_json(&m.t, &format!("/v1/me/history?limit=3&cursor={next}"), &amy).await;
    let ids: Vec<&str> =
        v["sessions"].as_array().expect("sessions").iter().filter_map(|s| s["book"]["id"].as_str()).collect();
    assert_eq!(ids, vec![BOOK]);
    assert_eq!(v["next"], Value::Null);

    // Once amy can no longer see the Kids library, its book is left out of the list and the
    // finished count; her own listening time still counts.
    {
        let c = m.t.st.db.0.lock().expect("db");
        c.execute("UPDATE user SET libraries = '[\"Audiobooks\"]' WHERE username = 'amy'", []).expect("perms");
    }
    let (_, v) = get_json(&m.t, "/v1/me/history", &amy).await;
    let ids: Vec<&str> =
        v["sessions"].as_array().expect("sessions").iter().filter_map(|s| s["book"]["id"].as_str()).collect();
    assert_eq!(ids, vec![ESCAPE_BOOK, MULTI_BOOK, BOOK]);
    assert!(!v.to_string().contains(KIDS_BOOK));
    assert_eq!(v["totals"]["books_finished"], 0);
    assert_eq!(v["totals"]["all_seconds"], 60);
}

#[tokio::test]
async fn saving_files_and_downloading_follow_the_account_permissions() {
    let m = setup_media();
    let (access, _) = listener(&m).await;
    let auth = format!("Bearer {access}");
    let get = |uri: String| {
        let (t, a) = (&m.t, auth.clone());
        async move { send(t, "GET", &uri, None, &[("authorization", &a)]).await }
    };
    // Off by default.
    let r = get(format!("/v1/media/{BOOK}/file")).await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_of(r).await["error"], "not_permitted");
    {
        let c = m.t.st.db.0.lock().expect("db");
        c.execute("UPDATE user SET can_save = 1 WHERE username = 'joe'", []).expect("perm");
    }
    let r = get(format!("/v1/media/{BOOK}/file")).await;
    assert_eq!(r.status(), StatusCode::OK);
    let cd = r.headers()["content-disposition"].to_str().expect("cd").to_owned();
    assert!(cd.starts_with("attachment; filename=\"A Book.mp3\""), "{cd}");
    assert_eq!(body_of(r).await.len(), 1000);
    // Still only books the user may see.
    assert_eq!(get(format!("/v1/media/{KIDS_BOOK}/file")).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(get(format!("/v1/media/{ESCAPE_BOOK}/file")).await.status(), StatusCode::NOT_FOUND);
    // Downloads (dl=1) need the offline permission.
    assert_eq!(get(format!("/v1/media/{BOOK}?dl=1")).await.status(), StatusCode::OK);
    {
        let c = m.t.st.db.0.lock().expect("db");
        c.execute("UPDATE user SET can_offline = 0 WHERE username = 'joe'", []).expect("perm");
    }
    let r = get(format!("/v1/media/{BOOK}?dl=1")).await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(get(format!("/v1/media/{BOOK}")).await.status(), StatusCode::OK, "streaming is unaffected");
}

#[tokio::test]
async fn genre_shelves_count_and_filter_only_permitted_books() {
    let m = setup_media();
    let (access, _) = listener(&m).await;
    let (_, v) = get_json(&m.t, "/v1/genres", &access).await;
    // The hidden library's book (biography + science fiction) is not counted.
    assert_eq!(
        v["genres"],
        json!([
            { "id": "science-fiction", "name": "Science Fiction", "kind": "fiction", "count": 1 },
            { "id": "biography", "name": "Biography & Memoir", "kind": "non-fiction", "count": 1 },
        ])
    );
    assert_eq!(v["unshelved"], 1);
    let ids = |v: &Value| {
        v["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|b| b["id"].as_str().unwrap_or("").to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&get_json(&m.t, "/v1/books?shelf=science-fiction", &access).await.1), [BOOK]);
    assert_eq!(ids(&get_json(&m.t, "/v1/books?shelf=non-fiction", &access).await.1), [MULTI_BOOK]);
    assert_eq!(ids(&get_json(&m.t, "/v1/books?shelf=fiction", &access).await.1), [BOOK]);
    assert_eq!(ids(&get_json(&m.t, "/v1/books?shelf=biography", &access).await.1), [MULTI_BOOK]);
    for bad in ["nope", "", "Science-Fiction", "../x"] {
        assert_eq!(
            get_json(&m.t, &format!("/v1/books?shelf={bad}"), &access).await.0,
            StatusCode::BAD_REQUEST,
            "{bad}"
        );
    }
    let (_, b) = get_json(&m.t, &format!("/v1/books/{BOOK}"), &access).await;
    assert_eq!(b["shelves"], json!([{ "id": "science-fiction", "name": "Science Fiction" }]));
    assert_eq!(get_json(&m.t, "/v1/genres", "forged").await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn enrolment_refuses_common_leaked_passwords() {
    let t = setup();
    let tok = invite(&t, Role::Listener);
    let r = send(&t, "POST", "/v1/auth/enrol/start", Some(json!({"token": tok, "username": "lea"})), &[]).await;
    let enrol = json_of(r).await["enrol"].as_str().expect("enrol").to_owned();
    let r = send(&t, "POST", "/v1/auth/enrol/password", Some(json!({"enrol": enrol, "password": "Password1234"})), &[])
        .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert!(json_of(r).await["message"].as_str().unwrap_or("").contains("leaked"));
    // The enrolment survives a refused password: a good one still works.
    let r = send(
        &t,
        "POST",
        "/v1/auth/enrol/password",
        Some(json!({"enrol": enrol, "password": "correct horse battery"})),
        &[],
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
}

// ---------------------------------------------------------------- passkeys (software authenticator)

/// A minimal software authenticator: one P-256 key, "none" attestation, user present and
/// verified. Enough to drive the real WebAuthn verification end to end.
struct SoftKey {
    key: openssl::ec::EcKey<openssl::pkey::Private>,
    cred_id: Vec<u8>,
    user_handle: Vec<u8>,
    counter: u32,
}

fn b64u(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for c in b.chunks(3) {
        let n = c.iter().enumerate().fold(0u32, |n, (i, &x)| n | (u32::from(x) << (16 - 8 * i)));
        for i in 0..=c.len() {
            out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

fn b64u_decode(s: &str) -> Vec<u8> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'-' | b'+' => 62,
        _ => 63,
    };
    let bytes: Vec<u8> = s.bytes().filter(|c| *c != b'=').map(val).collect();
    let mut out = Vec::new();
    for c in bytes.chunks(4) {
        let n = c.iter().enumerate().fold(0u32, |n, (i, &x)| n | (u32::from(x) << (18 - 6 * i)));
        for i in 0..c.len().saturating_sub(1) {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    out
}

fn sha256(b: &[u8]) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    Sha256::digest(b).to_vec()
}

impl SoftKey {
    fn new() -> Self {
        let group = openssl::ec::EcGroup::from_curve_name(openssl::nid::Nid::X9_62_PRIME256V1).expect("group");
        Self {
            key: openssl::ec::EcKey::generate(&group).expect("key"),
            cred_id: crate::auth::crypto::random::<16>().to_vec(),
            user_handle: Vec::new(),
            counter: 0,
        }
    }

    fn cose_key(&self) -> Vec<u8> {
        use serde_cbor_2::Value;
        let mut ctx = openssl::bn::BigNumContext::new().expect("ctx");
        let (mut x, mut y) = (openssl::bn::BigNum::new().expect("x"), openssl::bn::BigNum::new().expect("y"));
        self.key.public_key().affine_coordinates(self.key.group(), &mut x, &mut y, &mut ctx).expect("coords");
        let pad = |b: openssl::bn::BigNum| b.to_vec_padded(32).expect("pad");
        let map: std::collections::BTreeMap<Value, Value> = [
            (Value::Integer(1), Value::Integer(2)),
            (Value::Integer(3), Value::Integer(-7)),
            (Value::Integer(-1), Value::Integer(1)),
            (Value::Integer(-2), Value::Bytes(pad(x))),
            (Value::Integer(-3), Value::Bytes(pad(y))),
        ]
        .into_iter()
        .collect();
        serde_cbor_2::to_vec(&Value::Map(map)).expect("cbor")
    }

    /// Answers `navigator.credentials.create` for the server's options.
    fn create(&mut self, options: &Value, origin: &str) -> Value {
        use serde_cbor_2::Value as C;
        let pk = &options["publicKey"];
        self.user_handle = b64u_decode(pk["user"]["id"].as_str().expect("user id"));
        let client =
            json!({ "type": "webauthn.create", "challenge": pk["challenge"], "origin": origin, "crossOrigin": false })
                .to_string();
        let mut auth = sha256(pk["rp"]["id"].as_str().expect("rp").as_bytes());
        auth.push(0x45); // user present, user verified, attested credential data
        auth.extend(0u32.to_be_bytes());
        auth.extend([0u8; 16]);
        auth.extend((self.cred_id.len() as u16).to_be_bytes());
        auth.extend(&self.cred_id);
        auth.extend(self.cose_key());
        let att: std::collections::BTreeMap<C, C> = [
            (C::Text("fmt".into()), C::Text("none".into())),
            (C::Text("attStmt".into()), C::Map(Default::default())),
            (C::Text("authData".into()), C::Bytes(auth)),
        ]
        .into_iter()
        .collect();
        json!({
            "id": b64u(&self.cred_id), "rawId": b64u(&self.cred_id), "type": "public-key", "extensions": {},
            "response": { "attestationObject": b64u(&serde_cbor_2::to_vec(&C::Map(att)).expect("cbor")), "clientDataJSON": b64u(client.as_bytes()) },
        })
    }

    /// Answers `navigator.credentials.get`, signing with `signer` (normally this key).
    fn get_signed_by(
        &mut self,
        options: &Value,
        origin: &str,
        signer: &openssl::ec::EcKey<openssl::pkey::Private>,
    ) -> Value {
        let pk = &options["publicKey"];
        self.counter += 1;
        let client =
            json!({ "type": "webauthn.get", "challenge": pk["challenge"], "origin": origin, "crossOrigin": false })
                .to_string();
        let mut auth = sha256(pk["rpId"].as_str().expect("rpId").as_bytes());
        auth.push(0x05); // user present, user verified
        auth.extend(self.counter.to_be_bytes());
        let mut signed = auth.clone();
        signed.extend(sha256(client.as_bytes()));
        let pkey = openssl::pkey::PKey::from_ec_key(signer.clone()).expect("pkey");
        let mut s = openssl::sign::Signer::new(openssl::hash::MessageDigest::sha256(), &pkey).expect("signer");
        s.update(&signed).expect("update");
        let sig = s.sign_to_vec().expect("sign");
        json!({
            "id": b64u(&self.cred_id), "rawId": b64u(&self.cred_id), "type": "public-key", "extensions": {},
            "response": {
                "authenticatorData": b64u(&auth), "clientDataJSON": b64u(client.as_bytes()),
                "signature": b64u(&sig), "userHandle": b64u(&self.user_handle),
            },
        })
    }

    fn get(&mut self, options: &Value, origin: &str) -> Value {
        let k = self.key.clone();
        self.get_signed_by(options, origin, &k)
    }
}

/// Enrols `username` with only a passkey; returns the key and the access token.
async fn enrol_passkey(t: &T, username: &str) -> (SoftKey, String) {
    let tok = invite(t, Role::Listener);
    let r = send(t, "POST", "/v1/auth/enrol/start", Some(json!({"token": tok, "username": username})), &[]).await;
    let enrol = json_of(r).await["enrol"].as_str().expect("enrol").to_owned();
    let r = send(t, "POST", "/v1/auth/enrol/passkey/start", Some(json!({"enrol": enrol})), &[]).await;
    assert_eq!(r.status(), StatusCode::OK);
    let options = json_of(r).await["options"].clone();
    let mut key = SoftKey::new();
    let credential = key.create(&options, ORIGIN);
    let r = send(
        t,
        "POST",
        "/v1/auth/enrol/passkey/finish",
        Some(json!({"enrol": enrol, "name": "Phone", "credential": credential})),
        &[],
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK, "{}", json_of(r).await);
    let r = send(t, "POST", "/v1/auth/enrol/finish", Some(json!({"enrol": enrol, "device_name": "Phone"})), &[]).await;
    assert_eq!(r.status(), StatusCode::OK);
    let access = json_of(r).await["access_token"].as_str().expect("access").to_owned();
    (key, access)
}

async fn passkey_challenge(t: &T, username: Option<&str>) -> (String, Value) {
    let body = username.map_or_else(|| json!({}), |u| json!({ "username": u }));
    let r = send(t, "POST", "/v1/auth/login/passkey/start", Some(body), &[]).await;
    assert_eq!(r.status(), StatusCode::OK);
    let v = json_of(r).await;
    (v["challenge"].as_str().expect("challenge").to_owned(), v["options"].clone())
}

async fn passkey_finish(t: &T, challenge: &str, credential: &Value) -> Response {
    send(
        t,
        "POST",
        "/v1/auth/login/passkey/finish",
        Some(json!({"challenge": challenge, "device_name": "Laptop", "credential": credential})),
        &[],
    )
    .await
}

#[tokio::test]
async fn passkey_enrolment_and_sign_in_work_end_to_end() {
    let t = setup();
    let (mut key, access) = enrol_passkey(&t, "pam").await;
    let r = send(&t, "GET", "/v1/me", None, &[("authorization", &format!("Bearer {access}"))]).await;
    assert_eq!(json_of(r).await["factors"]["passkeys"], 1);
    // Discoverable sign-in (no username), and with a username.
    for who in [None, Some("pam")] {
        let (ch, options) = passkey_challenge(&t, who).await;
        let r = passkey_finish(&t, &ch, &key.get(&options, ORIGIN)).await;
        assert_eq!(r.status(), StatusCode::OK, "{who:?}");
        assert!(cookie_value(&r, "abshare_refresh").len() == 64);
        assert_eq!(json_of(r).await["user"]["username"], "pam");
    }
}

#[tokio::test]
async fn passkey_sign_in_refuses_forgery_phishing_and_replay() {
    let t = setup();
    let (mut key, _) = enrol_passkey(&t, "quin").await;
    // Signed by a different key (a stolen credential ID is not enough).
    let (ch, options) = passkey_challenge(&t, None).await;
    let other = SoftKey::new();
    let forged = key.get_signed_by(&options, ORIGIN, &other.key);
    assert_eq!(passkey_finish(&t, &ch, &forged).await.status(), StatusCode::UNAUTHORIZED);
    // A look-alike site relaying our challenge: the origin in the signed data is wrong.
    let (ch, options) = passkey_challenge(&t, None).await;
    let phished = key.get(&options, "https://books.example.com.evil.example");
    assert_eq!(passkey_finish(&t, &ch, &phished).await.status(), StatusCode::UNAUTHORIZED);
    // Replay: a good assertion works once; the same challenge again is refused.
    let (ch, options) = passkey_challenge(&t, None).await;
    let good = key.get(&options, ORIGIN);
    assert_eq!(passkey_finish(&t, &ch, &good).await.status(), StatusCode::OK);
    assert_eq!(passkey_finish(&t, &ch, &good).await.status(), StatusCode::UNAUTHORIZED);
    // An assertion for one challenge doesn't work against another.
    let (ch2, _) = passkey_challenge(&t, None).await;
    let (_, options3) = passkey_challenge(&t, None).await;
    let wrong = key.get(&options3, ORIGIN);
    assert_eq!(passkey_finish(&t, &ch2, &wrong).await.status(), StatusCode::UNAUTHORIZED);
    // A key that was never enrolled.
    let mut stranger = SoftKey::new();
    stranger.user_handle = key.user_handle.clone();
    let (ch, options) = passkey_challenge(&t, None).await;
    assert_eq!(passkey_finish(&t, &ch, &stranger.get(&options, ORIGIN)).await.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn passkey_enrolment_refuses_a_credential_from_another_site() {
    let t = setup();
    let tok = invite(&t, Role::Listener);
    let r = send(&t, "POST", "/v1/auth/enrol/start", Some(json!({"token": tok, "username": "ray"})), &[]).await;
    let enrol = json_of(r).await["enrol"].as_str().expect("enrol").to_owned();
    let r = send(&t, "POST", "/v1/auth/enrol/passkey/start", Some(json!({"enrol": enrol})), &[]).await;
    let options = json_of(r).await["options"].clone();
    let credential = SoftKey::new().create(&options, "https://evil.example");
    let r = send(
        &t,
        "POST",
        "/v1/auth/enrol/passkey/finish",
        Some(json!({"enrol": enrol, "name": "Phone", "credential": credential})),
        &[],
    )
    .await;
    assert!(r.status().is_client_error(), "{}", r.status());
    // Nothing was enrolled, so finishing is refused.
    let r = send(&t, "POST", "/v1/auth/enrol/finish", Some(json!({"enrol": enrol, "device_name": "Phone"})), &[]).await;
    assert!(r.status().is_client_error());
}

// ---------------------------------------------------------------- admin

/// An admin, stepped up; returns (seed, access token).
async fn admin(t: &T, username: &str) -> (Vec<u8>, String) {
    let (seed, access, ..) = enrol_role(t, username, Role::Admin).await;
    reset_totp_step(t, username);
    let r = send(
        t,
        "POST",
        "/v1/auth/step-up/totp",
        Some(json!({"password": "correct horse battery", "code": code(&seed, 0)})),
        &[("authorization", &format!("Bearer {access}"))],
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    (seed, access)
}

async fn as_user(t: &T, method: &str, uri: &str, body: Option<Value>, access: &str) -> Response {
    send(t, method, uri, body, &[("authorization", &format!("Bearer {access}"))]).await
}

#[tokio::test]
async fn admin_routes_refuse_listeners_and_strangers() {
    let t = setup();
    let (_, listener, ..) = enrol(&t, "lis").await;
    for (m, uri) in [
        ("GET", "/v1/admin/users"),
        ("GET", "/v1/admin/users/1"),
        ("PATCH", "/v1/admin/users/1"),
        ("POST", "/v1/admin/users/1/reset"),
        ("POST", "/v1/admin/users/1/sign-out"),
        ("GET", "/v1/admin/invites"),
        ("POST", "/v1/admin/invites"),
        ("DELETE", "/v1/admin/invites/1"),
        ("GET", "/v1/admin/audit"),
        ("GET", "/v1/admin/library"),
        ("POST", "/v1/admin/library/scan"),
        ("GET", "/v1/admin/status"),
    ] {
        let body = matches!(m, "PATCH" | "POST").then(|| json!({}));
        assert_eq!(as_user(&t, m, uri, body.clone(), &listener).await.status(), StatusCode::FORBIDDEN, "{m} {uri}");
        assert_eq!(send(&t, m, uri, body, &[]).await.status(), StatusCode::UNAUTHORIZED, "{m} {uri}");
    }
}

#[tokio::test]
async fn admin_changes_need_step_up() {
    let t = setup();
    let (_, access, ..) = enrol_role(&t, "adm", Role::Admin).await;
    assert_eq!(as_user(&t, "GET", "/v1/admin/users", None, &access).await.status(), StatusCode::OK);
    let r = as_user(&t, "POST", "/v1/admin/invites", Some(json!({})), &access).await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_of(r).await["error"], "step_up_required");
}

#[tokio::test]
async fn invites_from_the_admin_screen_work_once_and_can_be_revoked() {
    let t = setup();
    let (_, a) = admin(&t, "boss").await;
    // Inviting an admin needs a second confirmation; bad expiry is refused.
    let r = as_user(&t, "POST", "/v1/admin/invites", Some(json!({"role": "admin"})), &a).await;
    assert_eq!(r.status(), StatusCode::CONFLICT);
    for hours in [0, 169, -5] {
        let r = as_user(&t, "POST", "/v1/admin/invites", Some(json!({"expires_hours": hours})), &a).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{hours}");
    }
    let r = as_user(&t, "POST", "/v1/admin/invites", Some(json!({"bogus": 1})), &a).await;
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let r = as_user(
        &t,
        "POST",
        "/v1/admin/invites",
        Some(json!({"display_name": "Kim", "libraries": ["Audiobooks"], "offline": false, "expires_hours": 24})),
        &a,
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let link = json_of(r).await["link"].as_str().expect("link").to_owned();
    assert!(link.starts_with(&format!("{ORIGIN}/#/invite/")));
    let token = link.rsplit('/').next().expect("token").to_owned();
    let r = as_user(&t, "GET", "/v1/admin/invites", None, &a).await;
    let list = json_of(r).await["invites"].as_array().expect("invites").clone();
    assert_eq!(list.len(), 1);
    assert_eq!((list[0]["display_name"].clone(), list[0]["offline"].clone()), (json!("Kim"), json!(false)));
    // Revoked: the link stops working.
    let id = list[0]["id"].as_i64().expect("id");
    assert_eq!(
        as_user(&t, "DELETE", &format!("/v1/admin/invites/{id}"), None, &a).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        as_user(&t, "DELETE", &format!("/v1/admin/invites/{id}"), None, &a).await.status(),
        StatusCode::NOT_FOUND
    );
    let r = send(&t, "POST", "/v1/auth/enrol/start", Some(json!({"token": token, "username": "kim"})), &[]).await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    // The audit log records it, and its chain verifies.
    let r = as_user(&t, "GET", "/v1/admin/audit", None, &a).await;
    let v = json_of(r).await;
    assert_eq!(v["chain"]["intact"], true);
    let events: Vec<&str> =
        v["entries"].as_array().expect("entries").iter().filter_map(|e| e["event"].as_str()).collect();
    assert!(events.contains(&"invite.created") && events.contains(&"invite.revoked"), "{events:?}");
}

#[tokio::test]
async fn disabling_a_user_signs_them_out_at_once_and_admins_cannot_lock_themselves_out() {
    let t = setup();
    let (_, a) = admin(&t, "chief").await;
    let (_, victim, refresh, ..) = enrol(&t, "max").await;
    let r = as_user(&t, "GET", "/v1/admin/users", None, &a).await;
    let users = json_of(r).await["users"].as_array().expect("users").clone();
    let id = |name: &str| users.iter().find(|u| u["username"] == name).expect("user")["id"].as_i64().expect("id");
    let (max, chief) = (id("max"), id("chief"));
    let r = as_user(&t, "PATCH", &format!("/v1/admin/users/{max}"), Some(json!({"disabled": true})), &a).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(json_of(r).await["disabled"], true);
    assert_eq!(
        as_user(&t, "GET", "/v1/me", None, &victim).await.status(),
        StatusCode::UNAUTHORIZED,
        "signed out at once"
    );
    let r = send(&t, "POST", "/v1/auth/refresh", None, &[("cookie", &format!("abshare_refresh={refresh}"))]).await;
    assert_eq!(json_of(r).await["error"], "device_revoked");
    // Self-protection: an admin can't disable or demote themselves.
    for change in [json!({"disabled": true}), json!({"role": "listener"})] {
        let r = as_user(&t, "PATCH", &format!("/v1/admin/users/{chief}"), Some(change.clone()), &a).await;
        assert_eq!(r.status(), StatusCode::CONFLICT, "{change}");
    }
    // Bad values are refused.
    for bad in [json!({"role": "root"}), json!({"libraries": "everything"}), json!({"display_name": "   "})] {
        let r = as_user(&t, "PATCH", &format!("/v1/admin/users/{max}"), Some(bad.clone()), &a).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{bad}");
    }
    assert_eq!(
        as_user(&t, "PATCH", "/v1/admin/users/9999", Some(json!({"offline": false})), &a).await.status(),
        StatusCode::NOT_FOUND
    );
    // A reset link for the user.
    let r = as_user(&t, "POST", &format!("/v1/admin/users/{max}/reset"), None, &a).await;
    assert!(json_of(r).await["link"].as_str().is_some_and(|l| l.contains("/#/invite/")));
}

#[tokio::test]
async fn admin_status_and_library_views_answer() {
    let m = setup_media();
    let (_, a) = admin(&m.t, "ops").await;
    let r = as_user(&m.t, "GET", "/v1/admin/status", None, &a).await;
    assert_eq!(r.status(), StatusCode::OK);
    let v = json_of(r).await;
    assert_eq!(v["admins"], 1);
    assert!(
        v["warnings"].as_array().expect("warnings").iter().any(|w| w.as_str().unwrap_or("").contains("Only one admin"))
    );
    let r = as_user(&m.t, "GET", "/v1/admin/library", None, &a).await;
    let v = json_of(r).await;
    assert_eq!(v["libraries"].as_array().expect("libs").len(), 2);
    assert_eq!(v["scan_requested"], false);
    assert_eq!(as_user(&m.t, "POST", "/v1/admin/library/scan", None, &a).await.status(), StatusCode::ACCEPTED);
    let r = as_user(&m.t, "GET", "/v1/admin/library", None, &a).await;
    assert_eq!(json_of(r).await["scan_requested"], true);
}

#[tokio::test]
async fn admins_correct_book_details_and_everyone_sees_them_at_once() {
    let m = setup_media();
    let (_, a) = admin(&m.t, "editor").await;
    let (listener_token, _) = listener(&m).await;
    let uri = format!("/v1/admin/books/{BOOK}/details");
    let r = as_user(&m.t, "PUT", &uri, Some(json!({"title": "The Right Title", "authors": ["Ann Author"], "series": {"name": "Harbour", "position": "2"}, "genres": ["Biographies & Memoirs"]})), &a).await;
    assert_eq!(r.status(), StatusCode::OK);
    let (_, b) = get_json(&m.t, &format!("/v1/books/{BOOK}"), &listener_token).await;
    assert_eq!(b["title"], "The Right Title");
    assert_eq!(b["authors"][0]["name"], "Ann Author");
    assert_eq!(b["series"]["position"], "2");
    assert_eq!(
        b["shelves"],
        json!([{ "id": "biography", "name": "Biography & Memoir" }]),
        "genre shelves follow the correction"
    );
    let (_, v) = get_json(&m.t, "/v1/books?q=right%20title", &listener_token).await;
    assert_eq!(v["total"], 1, "search finds the corrected title");
    // Listeners can't edit; bad edits and unknown books are refused.
    assert_eq!(
        as_user(&m.t, "PUT", &uri, Some(json!({"title": "Mine"})), &listener_token).await.status(),
        StatusCode::FORBIDDEN
    );
    for bad in
        [json!({"title": ""}), json!({"path": "/etc"}), json!({"series": {"name": "S", "position": "two"}}), json!([])]
    {
        assert_eq!(as_user(&m.t, "PUT", &uri, Some(bad.clone()), &a).await.status(), StatusCode::BAD_REQUEST, "{bad}");
    }
    assert_eq!(
        as_user(&m.t, "PUT", "/v1/admin/books/zzzzzzzzzzzzzzzzzzzzzzzzzz/details", Some(json!({"title": "X"})), &a)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    // Removing the correction brings the tags back.
    assert_eq!(as_user(&m.t, "DELETE", &uri, None, &a).await.status(), StatusCode::NO_CONTENT);
    let (_, b) = get_json(&m.t, &format!("/v1/books/{BOOK}"), &listener_token).await;
    assert_eq!(b["title"], "A Book");
}

#[tokio::test]
async fn rotating_through_an_ipv6_64_does_not_dodge_the_anonymous_rate_limit() {
    let t = setup();
    let mut limited = false;
    for i in 0..40u32 {
        let mut r = req("POST", "/v1/auth/invite/inspect", Some(json!({})))
            .header("x-forwarded-proto", "https")
            // A different address in the same /64 each time (one attacker's allocation).
            .header("x-forwarded-for", format!("2001:db8:1:2::{:x}", i + 1))
            .body(Body::from(json!({"token": "f".repeat(64)}).to_string()))
            .expect("req");
        r.extensions_mut().insert(Peer(SocketAddr::from(([10, 0, 0, 10], 4000))));
        limited |= call(&t.app, r).await.status() == StatusCode::TOO_MANY_REQUESTS;
    }
    assert!(limited, "40 requests from one /64 must hit the 30-a-minute limit");
}

// ---------------------------------------------------------------- the connection layer (review H1)

async fn listen(proxies: Vec<std::net::IpAddr>) -> SocketAddr {
    let t = setup();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(super::serve(listener, t.app, proxies));
    addr
}

/// Opens a connection that has started a request and then goes quiet.
async fn stalled(addr: SocketAddr) -> tokio::net::TcpStream {
    use tokio::io::AsyncWriteExt;
    let mut s = tokio::net::TcpStream::connect(addr).await.expect("connect");
    s.write_all(b"GET /healthz HTTP/1.1\r\nHost: books.example.com\r\n").await.expect("write");
    s
}

async fn healthz(addr: SocketAddr) -> Option<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.ok()?;
    s.write_all(b"GET /healthz HTTP/1.1\r\nHost: books.example.com\r\nConnection: close\r\n\r\n").await.ok()?;
    let mut out = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), s.read_to_end(&mut out)).await.ok()?.ok()?;
    let text = String::from_utf8_lossy(&out).into_owned();
    (!text.is_empty()).then_some(text)
}

#[tokio::test]
async fn the_proxy_is_not_capped_like_a_single_client() {
    // Behind a proxy every connection comes from its address: capping it at 64 would cap the
    // whole service (and 64 slow bodies would take it down).
    let addr = listen(vec!["127.0.0.1".parse().expect("ip")]).await;
    let mut held = Vec::new();
    for _ in 0..70 {
        held.push(stalled(addr).await);
    }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        healthz(addr).await.is_some_and(|r| r.contains("200 OK")),
        "the 71st connection through the proxy is served"
    );
    // A single client that isn't a proxy is still capped.
    let addr = listen(vec![]).await;
    let mut held = Vec::new();
    for _ in 0..super::MAX_PER_ADDRESS {
        held.push(stalled(addr).await);
    }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(healthz(addr).await.is_none(), "one client can't open more than the cap");
}

#[tokio::test]
async fn a_request_body_that_never_arrives_times_out() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let addr = listen(vec![]).await;
    let mut s = tokio::net::TcpStream::connect(addr).await.expect("connect");
    s.write_all(
        b"POST /v1/auth/invite/inspect HTTP/1.1\r\nHost: books.example.com\r\nContent-Type: application/json\r\nContent-Length: 100\r\nX-ABShare-Client: 1\r\nOrigin: https://books.example.com\r\n\r\n{",
    )
    .await
    .expect("write");
    let started = std::time::Instant::now();
    let mut buf = vec![0u8; 512];
    let n = tokio::time::timeout(super::BODY_TIMEOUT + std::time::Duration::from_secs(5), s.read(&mut buf))
        .await
        .expect("the server answers instead of waiting for ever")
        .expect("read");
    assert!(String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 408"), "{}", String::from_utf8_lossy(&buf[..n]));
    assert!(started.elapsed() >= super::BODY_TIMEOUT - std::time::Duration::from_secs(1));
}

#[tokio::test]
async fn data_saver_without_a_range_is_capped_like_any_other_request() {
    // An hour-long book is 60 segments; one request may make the server encode at most 10.
    let m = setup_media();
    let (access, _) = listener(&m).await;
    for range in [None, Some("bytes=0-"), Some("bytes=0-999999999")] {
        let mut h = vec![("authorization".to_owned(), format!("Bearer {access}"))];
        if let Some(r) = range {
            h.push(("range".to_owned(), r.to_owned()));
        }
        let hs: Vec<(&str, &str)> = h.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let r = send(&m.t, "GET", &format!("/v1/media/{BOOK}?q=48"), None, &hs).await;
        assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT, "{range:?}");
        let cr = r.headers()["content-range"].to_str().expect("cr").to_owned();
        let end: u64 = cr.trim_start_matches("bytes 0-").split('/').next().expect("end").parse().expect("num");
        assert_eq!(end + 1, 10 * 2_500 * 144, "{range:?}: {cr}");
    }
}

#[tokio::test]
async fn searches_are_bounded() {
    let m = setup_media();
    let (access, _) = listener(&m).await;
    let long = "a".repeat(201);
    let many = "a b c d e f g h i";
    let flood = "a ".repeat(4000);
    for q in [long.as_str(), many, flood.trim()] {
        let r = get_json(&m.t, &format!("/v1/books?q={}", q.replace(' ', "%20")), &access).await;
        assert_eq!(r.0, StatusCode::BAD_REQUEST, "{}", &q[..q.len().min(20)]);
    }
    // Repeated words count once; an ordinary search works.
    assert_eq!(
        get_json(&m.t, "/v1/books?q=book%20book%20book%20book%20book%20book%20book%20book%20book", &access).await.0,
        StatusCode::OK
    );
    assert_eq!(get_json(&m.t, "/v1/books?q=a%20book", &access).await.1["total"], 3);
}

#[tokio::test]
async fn strangers_cannot_hold_the_owners_device_out_of_password_sign_in() {
    let t = setup();
    let (_, _, refresh, _) = enrol(&t, "olga").await;
    let login = |xff: String, cookie: Option<String>, pw: &'static str| {
        let mut b = req("POST", "/v1/auth/login/password", Some(json!({})))
            .header("x-forwarded-proto", "https")
            .header("x-forwarded-for", xff);
        if let Some(c) = cookie {
            b = b.header("cookie", c);
        }
        let mut r = b
            .body(Body::from(json!({"username": "olga", "password": pw, "device_name": "x"}).to_string()))
            .expect("req");
        r.extensions_mut().insert(Peer(SocketAddr::from(([10, 0, 0, 10], 4000))));
        call(&t.app, r)
    };
    // Strangers on many networks fail against the account and use up the shared allowance.
    for i in 0..8u8 {
        let _ = login(format!("2001:db8:{i}::1"), None, "guess guess guess").await;
    }
    // A new address without the owner's cookie is held back...
    assert_eq!(
        login("198.51.100.20".into(), None, "correct horse battery").await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    // ...but the owner's own browser, holding its sign-in cookie, still gets its attempt.
    let r = login("198.51.100.21".into(), Some(format!("abshare_refresh={refresh}")), "correct horse battery").await;
    assert_eq!(r.status(), StatusCode::OK);
    assert!(json_of(r).await["challenge"].is_string());
}

#[tokio::test]
async fn series_never_list_books_from_libraries_the_user_cannot_see() {
    let m = setup_media();
    let (access, _) = listener(&m).await;
    let gid = "gggggggggggggggggggggggggg";
    {
        let c = m.t.st.db.0.lock().expect("db");
        c.execute(
            "INSERT INTO grp (id, library_id, name, qualifier, kind) VALUES (?1, 1, 'Mixed', NULL, 'series')",
            [gid],
        )
        .expect("grp");
        for (i, b) in [BOOK, KIDS_BOOK].iter().enumerate() {
            c.execute(
                "INSERT INTO grp_member (grp_id, ord, book_id, extra) VALUES (?1, ?2, ?3, 0)",
                rusqlite::params![gid, i as i64, b],
            )
            .expect("member");
        }
    }
    m.t.st.invalidate_catalog().await;
    let (s, v) = get_json(&m.t, &format!("/v1/groups/{gid}"), &access).await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = v["items"].as_array().expect("items").iter().filter_map(|b| b["id"].as_str()).collect();
    assert_eq!(ids, [BOOK], "the hidden library's book is not listed");
}

// ---------------------------------------------------------------- review batch: Low findings

#[tokio::test]
async fn passkey_sign_in_start_never_reveals_an_account() {
    let t = setup();
    let _ = enrol_passkey(&t, "pia").await;
    for who in [Some("pia"), Some("nobody-here"), None] {
        let (_, options) = passkey_challenge(&t, who).await;
        let allow = &options["publicKey"]["allowCredentials"];
        assert!(allow.is_null() || allow.as_array().is_some_and(Vec::is_empty), "{who:?}: {allow}");
    }
}

#[tokio::test]
async fn a_password_alone_is_never_a_step_up() {
    let t = setup();
    let (_, access) = enrol_passkey(&t, "pat").await;
    {
        let hash = t.st.hasher.hash("a long enough password".to_owned()).await.expect("hash");
        let c = t.st.db.0.lock().expect("db");
        c.execute("UPDATE user SET password_hash = ?1 WHERE username = 'pat'", [hash]).expect("set");
    }
    // With a passkey and a password but no authenticator, the password can't confirm anything.
    for (uri, body) in [
        ("/v1/auth/step-up/password", json!({"password": "a long enough password"})),
        ("/v1/auth/step-up/totp", json!({"password": "a long enough password", "code": "000000"})),
    ] {
        let r = send(&t, "POST", uri, Some(body), &[("authorization", &format!("Bearer {access}"))]).await;
        assert_ne!(r.status(), StatusCode::OK, "{uri}");
    }
    let r =
        send(&t, "POST", "/v1/me/recovery", Some(json!({})), &[("authorization", &format!("Bearer {access}"))]).await;
    assert_eq!(json_of(r).await["error"], "step_up_required");
}

#[tokio::test]
async fn a_cookie_sent_twice_is_not_trusted() {
    let t = setup();
    let (_, _, refresh, _) = enrol(&t, "pam").await;
    let both = format!("abshare_refresh={refresh}; abshare_refresh={refresh}");
    let r = send(&t, "POST", "/v1/auth/refresh", None, &[("cookie", &both)]).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let r = send(&t, "POST", "/v1/auth/refresh", None, &[("cookie", &format!("abshare_refresh={refresh}"))]).await;
    assert_eq!(r.status(), StatusCode::OK);
}

#[tokio::test]
async fn admin_writes_all_need_step_up_and_promotion_needs_confirmation() {
    let m = setup_media();
    let (_, plain, ..) = enrol_role(&m.t, "adm2", Role::Admin).await;
    for (method, uri, body) in [
        ("POST", "/v1/admin/library/scan".to_owned(), None),
        ("PUT", format!("/v1/admin/books/{BOOK}/details"), Some(json!({"title": "X"}))),
        ("DELETE", format!("/v1/admin/books/{BOOK}/details"), None),
    ] {
        let r = as_user(&m.t, method, &uri, body, &plain).await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "{method} {uri}");
        assert_eq!(json_of(r).await["error"], "step_up_required");
    }
    let (_, a) = admin(&m.t, "chief2").await;
    let (_, _, _, _) = enrol(&m.t, "lou").await;
    let users = json_of(as_user(&m.t, "GET", "/v1/admin/users", None, &a).await).await;
    let lou = users["users"].as_array().expect("users").iter().find(|u| u["username"] == "lou").expect("lou")["id"]
        .as_i64()
        .expect("id");
    let r = as_user(&m.t, "PATCH", &format!("/v1/admin/users/{lou}"), Some(json!({"role": "admin"})), &a).await;
    assert_eq!(r.status(), StatusCode::CONFLICT);
    let r = as_user(
        &m.t,
        "PATCH",
        &format!("/v1/admin/users/{lou}"),
        Some(json!({"role": "admin", "confirm_admin": true})),
        &a,
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
}

#[tokio::test]
async fn covers_keep_their_own_sandbox_policy_and_bookmark_edits_are_clamped() {
    let m = setup_media();
    let (access, media) = listener(&m).await;
    let r = send(
        &m.t,
        "GET",
        &format!("/v1/media/covers/{BOOK}/{COVER}"),
        None,
        &[("cookie", &format!("abshare_media={media}"))],
    )
    .await;
    assert!(r.headers()["content-security-policy"].to_str().unwrap_or("").contains("sandbox"));
    let auth = format!("Bearer {access}");
    let r = send(
        &m.t,
        "POST",
        &format!("/v1/books/{BOOK}/bookmarks"),
        Some(json!({"position_ms": 1000})),
        &[("authorization", &auth)],
    )
    .await;
    let id = json_of(r).await["id"].as_i64().expect("id");
    let r = send(
        &m.t,
        "PATCH",
        &format!("/v1/bookmarks/{id}"),
        Some(json!({"position_ms": 99_999_999_999u64})),
        &[("authorization", &auth)],
    )
    .await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let (_, v) = get_json(&m.t, &format!("/v1/books/{BOOK}/bookmarks"), &access).await;
    assert_eq!(v["bookmarks"][0]["position_ms"], 3_600_000);
}

#[tokio::test]
async fn a_book_whose_copy_fails_is_reported_unplayable_not_retried_for_ever() {
    // The test media engine's ffmpeg doesn't exist, so building the multi-part copy fails.
    let m = setup_media();
    let (access, _) = listener(&m).await;
    let auth = format!("Bearer {access}");
    let r = send(&m.t, "GET", &format!("/v1/media/{MULTI_BOOK}"), None, &[("authorization", &auth)]).await;
    assert_eq!(r.status(), StatusCode::CONFLICT, "preparing at first");
    let mut failed = false;
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let (_, v) = get_json(&m.t, &format!("/v1/media/{MULTI_BOOK}/state"), &access).await;
        if v["failed"] == true {
            failed = true;
            break;
        }
    }
    assert!(failed, "the failure is reported");
    let r = send(&m.t, "GET", &format!("/v1/media/{MULTI_BOOK}"), None, &[("authorization", &auth)]).await;
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(json_of(r).await["error"], "unplayable");
}

#[tokio::test]
async fn a_flood_of_failed_sign_ins_cannot_roll_the_audit_log() {
    let t = setup();
    for _ in 0..200 {
        t.st.audit("login.failed", None, None, None, "x".into()).await;
    }
    t.st.audit("admin.user_changed", Some(1), None, None, "a real event".into()).await;
    let c = t.st.db.0.lock().expect("db");
    let noise: i64 =
        c.query_row("SELECT count(*) FROM audit WHERE event = 'login.failed'", [], |r| r.get(0)).expect("n");
    let real: i64 =
        c.query_row("SELECT count(*) FROM audit WHERE event = 'admin.user_changed'", [], |r| r.get(0)).expect("n");
    // At most the cap per minute (twice over if the minute turned during the test).
    assert!(noise <= 2 * super::NOISE_PER_MINUTE as i64, "{noise}");
    assert_eq!(real, 1, "real security events are always written");
}

/// A start-up report from `client`, as the reverse proxy would forward it.
async fn diag_from(t: &T, client: &str, body: Value) -> Response {
    let mut r = req("POST", "/v1/diag", Some(json!({})))
        .header("x-forwarded-proto", "https")
        .header("x-forwarded-for", client)
        .body(Body::from(body.to_string()))
        .expect("req");
    r.extensions_mut().insert(Peer(SocketAddr::from(([10, 0, 0, 10], 4000))));
    call(&t.app, r).await
}

#[tokio::test]
async fn start_up_reports_are_kept_cleaned_and_shown_only_to_admins() {
    let t = setup();
    let long = "x".repeat(1000);
    let r = diag_from(&t, "203.0.113.9", json!({"lines": ["Browser: test\u{202e}\u{0007}", long]})).await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let (_, admin_access, ..) = enrol_role(&t, "adm", Role::Admin).await;
    let r = as_user(&t, "GET", "/v1/admin/diagnostics", None, &admin_access).await;
    assert_eq!(r.status(), StatusCode::OK);
    let v = json_of(r).await;
    let report = &v["reports"][0];
    assert_eq!(report["addr"], "203.0.113.9");
    // Control and bidi characters removed; lines capped.
    assert_eq!(report["lines"][0], "Browser: test");
    assert!(report["lines"][1].as_str().expect("line").chars().count() <= 300);
    // Listeners and strangers can't read them.
    let (_, listener_access, ..) = enrol(&t, "lis").await;
    assert_eq!(
        as_user(&t, "GET", "/v1/admin/diagnostics", None, &listener_access).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(send(&t, "GET", "/v1/admin/diagnostics", None, &[]).await.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn start_up_reports_refuse_other_sites_and_bad_bodies() {
    let t = setup();
    // Same-origin only: the app's header and the exact Origin.
    for (hdr, origin) in [(None, Some(ORIGIN)), (Some("1"), None), (Some("1"), Some("https://evil.example"))] {
        let mut b = Request::builder()
            .method("POST")
            .uri("/v1/diag")
            .header("host", HOST)
            .header("content-type", "application/json");
        if let Some(h) = hdr {
            b = b.header("x-abshare-client", h);
        }
        if let Some(o) = origin {
            b = b.header("origin", o);
        }
        let r = call(&t.app, b.body(Body::from(json!({"lines": ["x"]}).to_string())).expect("req")).await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "{hdr:?} {origin:?}");
    }
    // Strict schema and size.
    assert_eq!(
        diag_from(&t, "203.0.113.1", json!({"lines": ["x"], "extra": 1})).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(diag_from(&t, "203.0.113.2", json!({"lines": []})).await.status(), StatusCode::BAD_REQUEST);
    let many: Vec<String> = (0..41).map(|i| i.to_string()).collect();
    assert_eq!(diag_from(&t, "203.0.113.3", json!({"lines": many})).await.status(), StatusCode::BAD_REQUEST);
    let huge = "y".repeat(20 * 1024);
    assert_eq!(diag_from(&t, "203.0.113.4", json!({"lines": [huge]})).await.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(super::diag_routes::recent(&t.st).is_empty());
}

#[tokio::test]
async fn start_up_reports_are_rate_limited_and_only_the_newest_kept() {
    let t = setup();
    for i in 0..super::diag_routes::PER_MINUTE {
        assert_eq!(
            diag_from(&t, "203.0.113.5", json!({"lines": [format!("r{i}")]})).await.status(),
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(
        diag_from(&t, "203.0.113.5", json!({"lines": ["one too many"]})).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    // Rotating through one IPv6 /64 doesn't help.
    for i in 0..super::diag_routes::PER_MINUTE {
        diag_from(&t, &format!("2001:db8:1:2::{i:x}"), json!({"lines": ["v6"]})).await;
    }
    assert_eq!(
        diag_from(&t, "2001:db8:1:2::ff", json!({"lines": ["v6"]})).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    // Memory only, and bounded: the newest MAX_REPORTS.
    for i in 0..60 {
        diag_from(&t, &format!("198.51.100.{i}"), json!({"lines": [format!("n{i}")]})).await;
    }
    let kept = super::diag_routes::recent(&t.st);
    assert_eq!(kept.len(), super::diag_routes::MAX_REPORTS);
    assert_eq!(kept[0].lines[0], "n59");
}

/// Signs `username` in on another device with the password and a fresh code: (access, refresh).
async fn another_device(t: &T, username: &str, seed: &[u8]) -> (String, String) {
    reset_totp_step(t, username);
    let r = send(
        t,
        "POST",
        "/v1/auth/login/password",
        Some(json!({"username": username, "password": "correct horse battery", "device_name": "Tablet"})),
        &[],
    )
    .await;
    let ch = json_of(r).await["challenge"].as_str().expect("challenge").to_owned();
    let r = send(t, "POST", "/v1/auth/login/totp", Some(json!({"challenge": ch, "code": code(seed, 0)})), &[]).await;
    assert_eq!(r.status(), StatusCode::OK);
    let refresh = cookie_value(&r, "abshare_refresh");
    (json_of(r).await["access_token"].as_str().expect("access").to_owned(), refresh)
}

async fn refresh_error(t: &T, refresh: &str) -> Value {
    let r = send(t, "POST", "/v1/auth/refresh", None, &[("cookie", &format!("abshare_refresh={refresh}"))]).await;
    json_of(r).await["error"].clone()
}

async fn device_ids(t: &T, access: &str) -> (i64, i64) {
    let r = send(t, "GET", "/v1/me/devices", None, &[("authorization", &format!("Bearer {access}"))]).await;
    let list = json_of(r).await["devices"].as_array().expect("devices").clone();
    let id = |this: bool| list.iter().find(|d| d["this_device"] == this).expect("device")["id"].as_i64().expect("id");
    (id(true), id(false))
}

#[tokio::test]
async fn signing_out_another_device_needs_step_up_but_your_own_does_not() {
    let t = setup();
    let (seed, phone, ..) = enrol(&t, "rue").await;
    let (tablet, tablet_refresh) = another_device(&t, "rue", &seed).await;
    let (tablet_id, phone_id) = device_ids(&t, &tablet).await;
    // A stolen access token alone can't sign the owner's other devices out.
    let r = send(
        &t,
        "DELETE",
        &format!("/v1/me/devices/{phone_id}"),
        None,
        &[("authorization", &format!("Bearer {tablet}"))],
    )
    .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_of(r).await["error"], "step_up_required");
    // Signing yourself out needs nothing more.
    let r = send(
        &t,
        "DELETE",
        &format!("/v1/me/devices/{tablet_id}"),
        None,
        &[("authorization", &format!("Bearer {tablet}"))],
    )
    .await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert_eq!(refresh_error(&t, &tablet_refresh).await, "device_revoked");
    // And the route is rate-limited per device.
    step_up(&t, "rue", &seed, &phone).await;
    let mut limited = false;
    for _ in 0..12 {
        let r =
            send(&t, "DELETE", "/v1/me/devices/999999", None, &[("authorization", &format!("Bearer {phone}"))]).await;
        limited |= r.status() == StatusCode::TOO_MANY_REQUESTS;
    }
    assert!(limited);
}

#[tokio::test]
async fn removing_or_replacing_a_factor_signs_out_other_devices() {
    let t = setup();
    let (seed, phone, ..) = enrol(&t, "sol").await;
    let auth = format!("Bearer {phone}");
    // Add a passkey, so the authenticator can be removed.
    step_up(&t, "sol", &seed, &phone).await;
    let r = send(&t, "POST", "/v1/me/passkeys/start", Some(json!({})), &[("authorization", &auth)]).await;
    let v = json_of(r).await;
    let mut key = SoftKey::new();
    let credential = key.create(&v["options"], ORIGIN);
    let r = send(
        &t,
        "POST",
        "/v1/me/passkeys/finish",
        Some(json!({"challenge": v["challenge"], "name": "Laptop", "credential": credential})),
        &[("authorization", &auth)],
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let passkey = json_of(r).await["id"].as_i64().expect("id");

    // Removing the authenticator signs out the tablet.
    let (_, tablet_refresh) = another_device(&t, "sol", &seed).await;
    let r = send(&t, "DELETE", "/v1/me/totp", None, &[("authorization", &auth)]).await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert_eq!(refresh_error(&t, &tablet_refresh).await, "device_revoked");
    // This device stays signed in.
    assert_eq!(send(&t, "GET", "/v1/me", None, &[("authorization", &auth)]).await.status(), StatusCode::OK);

    // Setting one up again signs nobody out; replacing it does.
    let set_totp = || async {
        let r = send(&t, "POST", "/v1/me/totp/start", Some(json!({})), &[("authorization", &auth)]).await;
        let v = json_of(r).await;
        let new_seed = b32_decode(v["secret"].as_str().expect("secret"));
        let r = send(
            &t,
            "POST",
            "/v1/me/totp/finish",
            Some(json!({"challenge": v["challenge"], "code": code(&new_seed, 0)})),
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        new_seed
    };
    let seed = set_totp().await;
    let (_, tablet_refresh) = another_device(&t, "sol", &seed).await;
    assert_ne!(refresh_error(&t, &tablet_refresh).await, "device_revoked", "a refresh that works has no error");
    let (_, tablet_refresh) = another_device(&t, "sol", &seed).await;
    let seed = set_totp().await;
    assert_eq!(refresh_error(&t, &tablet_refresh).await, "device_revoked");

    // Removing a passkey signs out the tablet too.
    let (_, tablet_refresh) = another_device(&t, "sol", &seed).await;
    let r = send(&t, "DELETE", &format!("/v1/me/passkeys/{passkey}"), None, &[("authorization", &auth)]).await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert_eq!(refresh_error(&t, &tablet_refresh).await, "device_revoked");
}

#[tokio::test]
async fn a_recovery_sign_in_must_set_up_a_new_factor_first() {
    let t = setup();
    let (_, _, _, codes) = enrol(&t, "uma").await;
    let r = send(
        &t,
        "POST",
        "/v1/auth/login/password",
        Some(json!({"username":"uma","password":"correct horse battery","device_name":"Borrowed"})),
        &[],
    )
    .await;
    let ch = json_of(r).await["challenge"].as_str().expect("challenge").to_owned();
    let r = send(&t, "POST", "/v1/auth/login/recovery", Some(json!({"challenge": ch, "code": codes[0]})), &[]).await;
    assert_eq!(r.status(), StatusCode::OK);
    let refresh = cookie_value(&r, "abshare_refresh");
    let media = cookie_value(&r, "abshare_media");
    let v = json_of(r).await;
    assert_eq!(v["user"]["needs_factor"], true);
    let auth = format!("Bearer {}", v["access_token"].as_str().expect("access"));

    // Only the account works: the library, admin and media are refused.
    for uri in ["/v1/home", "/v1/books", "/v1/libraries", "/v1/admin/status", "/v1/me/history"] {
        let r = send(&t, "GET", uri, None, &[("authorization", &auth)]).await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "{uri}");
        assert_eq!(json_of(r).await["error"], "factor_required", "{uri}");
    }
    let r = send(
        &t,
        "GET",
        &format!("/v1/media/{}", "a".repeat(26)),
        None,
        &[("cookie", &format!("abshare_media={media}"))],
    )
    .await;
    assert_eq!(json_of(r).await["error"], "factor_required");
    let r = send(&t, "GET", "/v1/me", None, &[("authorization", &auth)]).await;
    assert_eq!(json_of(r).await["needs_factor"], true);

    // A refresh doesn't lift it.
    let r = send(&t, "POST", "/v1/auth/refresh", None, &[("cookie", &format!("abshare_refresh={refresh}"))]).await;
    let v = json_of(r).await;
    assert_eq!(v["user"]["needs_factor"], true);
    let auth = format!("Bearer {}", v["access_token"].as_str().expect("access"));
    assert_eq!(send(&t, "GET", "/v1/home", None, &[("authorization", &auth)]).await.status(), StatusCode::FORBIDDEN);

    // The recovery code counts as a fresh factor, so a new authenticator can be set up at once.
    let r = send(&t, "POST", "/v1/me/totp/start", Some(json!({})), &[("authorization", &auth)]).await;
    assert_eq!(r.status(), StatusCode::OK);
    let v = json_of(r).await;
    let seed = b32_decode(v["secret"].as_str().expect("secret"));
    let r = send(
        &t,
        "POST",
        "/v1/me/totp/finish",
        Some(json!({"challenge": v["challenge"], "code": code(&seed, 0)})),
        &[("authorization", &auth)],
    )
    .await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert_eq!(send(&t, "GET", "/v1/home", None, &[("authorization", &auth)]).await.status(), StatusCode::OK);
    let r = send(&t, "GET", "/v1/me", None, &[("authorization", &auth)]).await;
    assert_eq!(json_of(r).await["needs_factor"], false);
}

#[tokio::test]
async fn home_shows_a_show_once_in_recently_added() {
    let t = setup();
    {
        let c = t.st.db.0.lock().expect("db");
        c.execute("INSERT INTO library (id, name, root) VALUES (1, 'Audiobooks', '/srv/audiobooks')", []).expect("lib");
        let ids = [
            "e1aaaaaaaaaaaaaaaaaaaaaaaa",
            "e2aaaaaaaaaaaaaaaaaaaaaaaa",
            "e3aaaaaaaaaaaaaaaaaaaaaaaa",
            "bkaaaaaaaaaaaaaaaaaaaaaaaa",
        ];
        for (n, id) in ids.iter().enumerate() {
            c.execute(
                "INSERT INTO book (id, library_id, folder, meta, duration_ms, needs_copy, updated_at, added_at)
                 VALUES (?1, 1, '', '{\"title\":\"T\"}', 60000, 0, 1, ?2)",
                rusqlite::params![id, 100 + n as i64],
            )
            .expect("book");
        }
        c.execute("INSERT INTO grp (id, library_id, name, kind) VALUES ('showaaaaaaaaaaaaaaaaaaaaaa', 1, 'Field Notes', 'show')", []).expect("grp");
        for (n, id) in ids[..3].iter().enumerate() {
            c.execute(
                "INSERT INTO grp_member (grp_id, ord, book_id, extra) VALUES ('showaaaaaaaaaaaaaaaaaaaaaa', ?1, ?2, 0)",
                rusqlite::params![n as i64, id],
            )
            .expect("member");
        }
    }
    let (_, access, ..) = enrol(&t, "ned").await;
    let r = send(&t, "GET", "/v1/home", None, &[("authorization", &format!("Bearer {access}"))]).await;
    let recent = json_of(r).await["recently_added"].as_array().expect("recent").clone();
    let kinds: Vec<&str> = recent.iter().map(|v| v["type"].as_str().unwrap_or("")).collect();
    // The newest addition is the standalone book; the three episodes are one show tile.
    assert_eq!(kinds, ["book", "show"], "{recent:?}");
    assert_eq!(recent[1]["title"], "Field Notes");
    assert_eq!(recent[1]["episodes"], 3);
}

// ---------------------------------------------------------------- the demo library, end to end

/// Makes a tree read-only (the scanner refuses a library it could write to), or writable again.
fn set_tree_writable(root: &std::path::Path, writable: bool) {
    use std::os::unix::fs::PermissionsExt;
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let p = entry.path();
        let meta = std::fs::symlink_metadata(&p).expect("meta");
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            if writable {
                let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755));
            }
            set_tree_writable(&p, writable);
            if !writable {
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o555)).expect("chmod");
            }
        } else {
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(if writable { 0o644 } else { 0o444 }))
                .expect("chmod");
        }
    }
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(if writable { 0o755 } else { 0o555 }))
        .expect("chmod");
}

/// The synthetic demo library (tools/demo/make-library.sh), scanned with the real jail and
/// ffmpeg, then used through the real router: browse, stream the original and data saver,
/// build a playable copy, download for offline listening, and sync progress. Needs Landlock;
/// with ABSHARE_REQUIRE_LANDLOCK set (as in CI) it fails rather than skips.
#[tokio::test]
async fn the_demo_library_scans_streams_downloads_and_syncs() {
    let Some(ff) = crate::tools::test_ffmpeg().await else {
        eprintln!("skipped: the jailed ffmpeg can't run here");
        return;
    };
    let dir = std::env::temp_dir().join(format!("abshare-demo-test-{}", std::process::id()));
    if dir.exists() {
        set_tree_writable(&dir, true);
        let _ = std::fs::remove_dir_all(&dir);
    }
    let (lib, data) = (dir.join("lib"), dir.join("data"));
    std::fs::create_dir_all(&data).expect("mkdir");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    }
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/demo/make-library.sh");
    let made = std::process::Command::new("sh").arg(&script).arg(&lib).output().expect("run make-library.sh");
    assert!(made.status.success(), "make-library.sh: {}", String::from_utf8_lossy(&made.stderr));
    set_tree_writable(&lib, false);

    // Scan it, as `abshare scan` does.
    let libs: Vec<String> = ff.libs.iter().map(|p| format!("{:?}", p.to_string_lossy())).collect();
    let toml = format!(
        "data_dir = {:?}\npublic_origin = \"https://books.example.com\"\ntrusted_proxies = [\"10.0.0.10\"]\n\
         [ffmpeg]\nffmpeg = {:?}\nffprobe = {:?}\njail = {:?}\nlibs = [{}]\n\
         [[library]]\nname = \"Audiobooks\"\npath = {:?}\n",
        data.to_string_lossy(),
        ff.ffmpeg.to_string_lossy(),
        ff.ffprobe.to_string_lossy(),
        ff.jail.to_string_lossy(),
        libs.join(", "),
        lib.to_string_lossy(),
    );
    let cfg_path = dir.join("abshare.toml");
    std::fs::write(&cfg_path, toml).expect("config");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cfg_path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    }
    let cfg = crate::config::Config::load(&cfg_path).expect("config loads");
    let secret = Secret::load_or_create(&data).expect("secret");
    let mut cat = crate::catalogue::Catalogue::open(&data.join("abshare.db")).expect("catalogue");
    let rep = crate::scan::scan_library(&cfg, &mut cat, &secret, &cfg.libraries[0], 1).await.expect("scan");
    assert_eq!((rep.files, rep.books, rep.needs_copy, rep.shows), (25, 15, 2, 2), "{rep:?}");
    let kinds = |k: &str| rep.problems.iter().filter(|p| p.1 == k).count();
    assert_eq!((kinds("unplayable"), kinds("duplicate"), kinds("skipped")), (2, 1, 1), "{:?}", rep.problems);
    assert!(rep.problems.iter().any(|p| p.0 == "escape.mp3" && p.1 == "skipped"), "the symlink isn't followed");
    drop(cat);

    // Serve it, as `abshare serve` does.
    let conn = crate::db::open(&data.join("abshare.db")).expect("db");
    let mut st = AppState::new(conn, &secret, ORIGIN, vec!["10.0.0.10".parse().expect("ip")], Hasher::for_tests())
        .expect("state");
    let roots = [("Audiobooks".to_owned(), cfg.libraries[0].path.clone())].into_iter().collect();
    st.media = Some(Arc::new(crate::media::Media::new(ff, data.clone(), roots)));
    let st = Arc::new(st);
    let t = T { app: router(st.clone()), st };
    let (_, access, _, _, media) = enrol_with_media(&t, "demo").await;
    let bearer = format!("Bearer {access}");
    let cookie = format!("abshare_media={media}");

    // Browse: the series is grouped, the show is one tile, the broken files aren't listed.
    let r = send(&t, "GET", "/v1/books?limit=100", None, &[("authorization", &bearer)]).await;
    assert_eq!(r.status(), StatusCode::OK);
    let items = json_of(r).await["items"].as_array().expect("items").clone();
    let title = |v: &Value| v["title"].as_str().unwrap_or("").to_owned();
    let titles: Vec<String> = items.iter().map(title).collect();
    assert!(titles.contains(&"The Copper Tide".to_owned()), "{titles:?}");
    assert!(!titles.iter().any(|t| t.starts_with("Broken")), "{titles:?}");
    assert!(items.iter().any(|v| v["type"] == "show"), "{titles:?}");
    let copper =
        items.iter().find(|v| title(v) == "The Copper Tide").expect("book")["id"].as_str().expect("id").to_owned();

    // Stream the original with a Range request.
    let r =
        send(&t, "GET", &format!("/v1/media/{copper}"), None, &[("cookie", &cookie), ("range", "bytes=0-99")]).await;
    assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
    assert!(r.headers()["content-range"].to_str().expect("range").starts_with("bytes 0-99/"));
    assert_eq!(body_of(r).await.len(), 100);

    // A multi-part book: its playable copy is built in the jail, then streams.
    let multi: String = t
        .st
        .db
        .run(|c| {
            c.query_row("SELECT id FROM book WHERE needs_copy = 1 AND missing_since IS NULL LIMIT 1", [], |r| r.get(0))
                .map_err(crate::db::sql_err)
        })
        .await
        .expect("a multi-part book");
    let mut ready = false;
    for _ in 0..240 {
        let r = send(&t, "GET", &format!("/v1/media/{multi}/state"), None, &[("cookie", &cookie)]).await;
        let v = json_of(r).await;
        assert_ne!(v["failed"], true, "the playable copy failed");
        if v["ready"] == true {
            ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert!(ready, "the playable copy wasn't ready within two minutes");
    let r = send(&t, "GET", &format!("/v1/media/{multi}"), None, &[("cookie", &cookie), ("range", "bytes=0-")]).await;
    assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
    assert!(!body_of(r).await.is_empty());

    // Data saver: a constant-bitrate MP3 made on demand; byte 0 is a frame header.
    let r =
        send(&t, "GET", &format!("/v1/media/{copper}?q=32"), None, &[("cookie", &cookie), ("range", "bytes=0-959")])
            .await;
    assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(r.headers()["content-type"], "audio/mpeg");
    let bytes = body_of(r).await;
    assert_eq!(bytes.len(), 960);
    assert_eq!((bytes[0], bytes[1] & 0xE0), (0xFF, 0xE0), "an MP3 frame sync");

    // An offline download of the data-saver file, as the app fetches it.
    let r =
        send(&t, "GET", &format!("/v1/media/{copper}?q=32&dl=1"), None, &[("cookie", &cookie), ("range", "bytes=0-")])
            .await;
    assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
    assert!(!body_of(r).await.is_empty());

    // Progress syncs and shows up in Continue listening.
    let event = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("time").as_millis() as i64;
    let r = send(
        &t,
        "PUT",
        &format!("/v1/progress/{copper}"),
        Some(json!({"key": "demo-key-0001", "event_time_ms": event, "action": "position", "position_ms": 5000})),
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(json_of(r).await["result"], "accepted");
    let r = send(&t, "GET", "/v1/home", None, &[("authorization", &bearer)]).await;
    let home = json_of(r).await;
    assert!(home["continue"].as_array().expect("continue").iter().any(|v| v["id"] == copper.as_str()), "{home}");

    set_tree_writable(&dir, true);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_content_type_comes_from_the_probe_and_a_changed_file_is_refused() {
    let m = setup_media();
    let (_, media) = listener(&m).await;
    let cookie = format!("abshare_media={media}");
    // The scan probed book.mp3 as Ogg: the name doesn't decide what it is served as.
    {
        let c = m.t.st.db.0.lock().expect("db");
        c.execute("UPDATE file SET probe = ?1 WHERE rel_path = 'book.mp3'", [probe_json("ogg")]).expect("probe");
    }
    let r = send(&m.t, "GET", &format!("/v1/media/{BOOK}"), None, &[("cookie", &cookie), ("range", "bytes=0-9")]).await;
    assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(r.headers()["content-type"], "audio/ogg");
    // A container the app doesn't serve as audio is opaque bytes.
    {
        let c = m.t.st.db.0.lock().expect("db");
        c.execute("UPDATE file SET probe = ?1 WHERE rel_path = 'book.mp3'", [probe_json("html")]).expect("probe");
    }
    let r = send(&m.t, "GET", &format!("/v1/media/{BOOK}"), None, &[("cookie", &cookie), ("range", "bytes=0-9")]).await;
    assert_eq!(r.headers()["content-type"], "application/octet-stream");
    // The file changed since the scan (another size): refused, and a rescan is requested.
    let trigger = m.dir.join("data").join("scan-requested");
    let _ = std::fs::remove_file(&trigger);
    std::fs::write(m.dir.join("lib").join("book.mp3"), vec![7u8; 1500]).expect("rewrite");
    let r = send(&m.t, "GET", &format!("/v1/media/{BOOK}"), None, &[("cookie", &cookie), ("range", "bytes=0-9")]).await;
    assert_eq!(r.status(), StatusCode::CONFLICT);
    assert_eq!(json_of(r).await["error"], "changed");
    assert!(trigger.exists(), "a rescan is requested");
}

#[tokio::test]
async fn author_and_narrator_filters_match_whole_names_and_are_bounded() {
    let m = setup_media();
    let (access, _) = listener(&m).await;
    let auth = format!("Bearer {access}");
    {
        let c = m.t.st.db.0.lock().expect("db");
        c.execute(
            "UPDATE book SET meta = '{\"title\":\"A Book\",\"authors\":[{\"name\":\"Mara Quill\"}],\"narrators\":[{\"name\":\"Tomas Reed\"}]}' WHERE id = ?1",
            [BOOK],
        )
        .expect("meta");
    }
    m.t.st.invalidate_catalog().await;
    let ids = |v: Value| -> Vec<String> {
        v["items"].as_array().expect("items").iter().map(|b| b["id"].as_str().unwrap_or("").to_owned()).collect()
    };
    let r = send(&m.t, "GET", "/v1/books?author=mara%20quill", None, &[("authorization", &auth)]).await;
    assert_eq!(ids(json_of(r).await), [BOOK]);
    let r = send(&m.t, "GET", "/v1/books?narrator=Tomas%20Reed", None, &[("authorization", &auth)]).await;
    assert_eq!(ids(json_of(r).await), [BOOK]);
    // Whole names only.
    let r = send(&m.t, "GET", "/v1/books?author=Mara", None, &[("authorization", &auth)]).await;
    assert!(ids(json_of(r).await).is_empty());
    // Bounded.
    let long = "a".repeat(201);
    let r = send(&m.t, "GET", &format!("/v1/books?author={long}"), None, &[("authorization", &auth)]).await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_jobs_page_shows_failed_copies_and_its_actions_need_an_admin_and_step_up() {
    let m = setup_media();
    let (listener_access, cookie) = listener(&m).await;
    // The multi-part book's playable copy can't be built here (no ffmpeg): it fails and backs off.
    let r = send(
        &m.t,
        "GET",
        &format!("/v1/media/{MULTI_BOOK}/state"),
        None,
        &[("cookie", &format!("abshare_media={cookie}"))],
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let mut failed = false;
    for _ in 0..100 {
        if m.t.st.media.as_ref().expect("media").copy_failed(MULTI_BOOK) {
            failed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(failed, "the copy should fail without ffmpeg");

    // Only admins can look.
    assert_eq!(as_user(&m.t, "GET", "/v1/admin/jobs", None, &listener_access).await.status(), StatusCode::FORBIDDEN);
    assert_eq!(send(&m.t, "GET", "/v1/admin/jobs", None, &[]).await.status(), StatusCode::UNAUTHORIZED);
    let (_, plain, ..) = enrol_role(&m.t, "adm3", Role::Admin).await;
    let r = as_user(&m.t, "GET", "/v1/admin/jobs", None, &plain).await;
    assert_eq!(r.status(), StatusCode::OK);
    let v = json_of(r).await;
    assert_eq!(v["lanes"]["copies"]["of"], 1);
    assert_eq!(v["failed"][0]["id"], MULTI_BOOK);
    assert_eq!(v["failed"][0]["title"], "A Book");

    // Actions need a step-up.
    let r = as_user(&m.t, "POST", &format!("/v1/admin/jobs/copies/{MULTI_BOOK}/retry"), Some(json!({})), &plain).await;
    assert_eq!(json_of(r).await["error"], "step_up_required");
    let r = as_user(&m.t, "POST", "/v1/admin/cache/clear", Some(json!({})), &plain).await;
    assert_eq!(json_of(r).await["error"], "step_up_required");

    let (_, boss) = admin(&m.t, "boss2").await;
    // Retry forgets the failure; an unknown or malformed book is not found.
    let r = as_user(&m.t, "POST", &format!("/v1/admin/jobs/copies/{MULTI_BOOK}/retry"), Some(json!({})), &boss).await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert!(!m.t.st.media.as_ref().expect("media").copy_failed(MULTI_BOOK));
    for id in [BOOK, "not-an-id"] {
        let r = as_user(&m.t, "POST", &format!("/v1/admin/jobs/copies/{id}/retry"), Some(json!({})), &boss).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{id}");
    }
    // Clearing removes data-saver segments nobody is playing; copies stay.
    let saver = m.dir.join("data").join("saver").join(BOOK).join("k48");
    std::fs::create_dir_all(&saver).expect("mkdir");
    std::fs::write(saver.join("000000.mp3"), vec![0u8; 4096]).expect("segment");
    let copy = m.dir.join("data").join("copies").join(format!("{MULTI_BOOK}.m4b"));
    std::fs::create_dir_all(copy.parent().expect("dir")).expect("mkdir");
    std::fs::write(&copy, b"copy").expect("copy");
    let r = as_user(&m.t, "POST", "/v1/admin/cache/clear", Some(json!({})), &boss).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert!(json_of(r).await["freed_bytes"].as_u64().expect("freed") >= 4096);
    assert!(!saver.exists());
    assert!(copy.exists());
}
