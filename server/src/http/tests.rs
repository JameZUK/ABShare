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
    let tok = invite(t, Role::Listener);
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
    let v = json_of(r).await;
    let codes =
        v["recovery_codes"].as_array().expect("codes").iter().map(|c| c.as_str().unwrap_or("").to_owned()).collect();
    (seed, v["access_token"].as_str().expect("access").to_owned(), refresh, codes)
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
    assert!(resp.headers().get("content-security-policy").is_some());
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
    // Password alone isn't a step-up when TOTP exists.
    let r = send(
        &t,
        "POST",
        "/v1/auth/step-up/password",
        Some(json!({"password": "correct horse battery"})),
        &[("authorization", &auth)],
    )
    .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
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
    let (_, a, ..) = enrol(&t, "kim").await;
    let (_, b, ..) = enrol(&t, "lee").await;
    let r = send(&t, "GET", "/v1/me/devices", None, &[("authorization", &format!("Bearer {b}"))]).await;
    let lee_device = json_of(r).await["devices"][0]["id"].as_i64().expect("id");
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
