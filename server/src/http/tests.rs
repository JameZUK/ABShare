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
    let media = cookie_value(&r, "abshare_media");
    let v = json_of(r).await;
    let codes =
        v["recovery_codes"].as_array().expect("codes").iter().map(|c| c.as_str().unwrap_or("").to_owned()).collect();
    (seed, v["access_token"].as_str().expect("access").to_owned(), refresh, codes, media)
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
