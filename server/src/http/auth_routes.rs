//! `/v1/auth/*` (SPEC §5.5, §6): invites and enrolment, sign-in (password + TOTP or
//! recovery code, or passkey), refresh, sign-out and step-up. Every route here sits behind
//! the CSRF check. Failures are generic and count towards the lockout.

use super::{
    ApiError, Authed, Enrol, Ip, LoginPending, MEDIA_COOKIE, PasskeyLogin, REFRESH_COOKIE, State, cookie, now,
    passkeys, set_cookie,
};
use crate::auth::crypto::{new_token, random, token_hash};
use crate::auth::sessions::MEDIA_TTL;
use crate::auth::store::{self, DeviceError, NewUser, RefreshOutcome};
use abshare_core::auth::codes::{normalise_recovery, recovery_code};
use abshare_core::auth::lockout::Refusal;
use abshare_core::auth::{totp, validate};
use axum::extract::State as S;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use webauthn_rs::prelude::{PublicKeyCredential, RegisterPublicKeyCredential, Uuid};

pub fn routes(_state: State) -> Router<State> {
    Router::new()
        .route("/invite/inspect", post(invite_inspect))
        .route("/enrol/start", post(enrol_start))
        .route("/enrol/password", post(enrol_password))
        .route("/enrol/totp", post(enrol_totp))
        .route("/enrol/passkey/start", post(enrol_passkey_start))
        .route("/enrol/passkey/finish", post(enrol_passkey_finish))
        .route("/enrol/finish", post(enrol_finish))
        .route("/login/password", post(login_password))
        .route("/login/totp", post(login_totp))
        .route("/login/recovery", post(login_recovery))
        .route("/login/passkey/start", post(login_passkey_start))
        .route("/login/passkey/finish", post(login_passkey_finish))
        .route("/refresh", post(refresh))
        .route("/logout", post(logout))
        .route("/step-up/password", post(step_up_password))
        .route("/step-up/totp", post(step_up_totp))
        .route("/step-up/passkey/start", post(step_up_passkey_start))
        .route("/step-up/passkey/finish", post(step_up_passkey_finish))
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024))
}

fn ua(h: &HeaderMap) -> Option<String> {
    h.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).map(str::to_owned)
}

/// Issues a device, refresh token and session; returns the response that sets the cookies.
async fn complete_login(
    st: &State,
    user_id: i64,
    device_name: &str,
    ip: std::net::IpAddr,
    ua: Option<String>,
    extra: Value,
) -> Result<Response, ApiError> {
    let name = validate::label(device_name, "Device");
    let (refresh, rh) = new_token();
    let ip_s = ip.to_string();
    let (user, device) = st
        .db
        .run(move |c| {
            let user = store::user_by_id(c, user_id)?.ok_or("user vanished")?;
            if user.disabled {
                return Err("disabled".into());
            }
            let tx = c.transaction().map_err(crate::db::sql_err)?;
            let device = match store::create_device(&tx, user_id, &name, Some(&ip_s), ua.as_deref(), now()) {
                Ok(d) => d,
                Err(DeviceError::TooManyNew) => return Ok((user, None)),
                Err(DeviceError::Db(e)) => return Err(e),
            };
            store::issue_refresh(&tx, device, &rh, None, now())?;
            tx.commit().map_err(crate::db::sql_err)?;
            Ok((user, Some(device)))
        })
        .await?;
    let Some(device) = device else { return Err(ApiError::too_many()) };
    let (access, media) =
        st.sessions.issue(user.id, device, user.role, now()).ok_or_else(|| ApiError::internal("session table full"))?;
    st.audit("login.ok", Some(user.id), Some(device), Some(ip), String::new()).await;
    let mut body = json!({
        "access_token": access,
        "expires_in": crate::auth::sessions::ACCESS_TTL,
        "user": { "username": user.username, "display_name": user.display_name, "role": user.role.as_str() },
    });
    if let (Some(b), Value::Object(extra)) = (body.as_object_mut(), extra) {
        b.extend(extra);
    }
    let mut r = Json(body).into_response();
    r.headers_mut()
        .append(header::SET_COOKIE, set_cookie(REFRESH_COOKIE, &refresh, "/v1/auth", store::REFRESH_LIFETIME));
    r.headers_mut().append(header::SET_COOKIE, set_cookie(MEDIA_COOKIE, &media, "/v1/media", MEDIA_TTL));
    Ok(r)
}

// ---------------------------------------------------------------- invites and enrolment

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenBody {
    token: String,
}

async fn invite_inspect(S(st): S<State>, Ip(ip): Ip, Json(b): Json<TokenBody>) -> Result<Json<Value>, ApiError> {
    if !st.anon_limit.allow(ip, now()) {
        return Err(ApiError::too_many());
    }
    if !abshare_core::auth::codes::is_token(&b.token) {
        return Err(ApiError::not_found());
    }
    let h = token_hash(&b.token);
    let inv = st.db.run(move |c| store::find_invite(c, &h, now())).await?.ok_or_else(ApiError::not_found)?;
    Ok(Json(json!({ "purpose": inv.purpose, "display_name": inv.display_name, "expires_at": inv.expires_at })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrolStart {
    token: String,
    username: String,
}

async fn enrol_start(S(st): S<State>, Ip(ip): Ip, Json(b): Json<EnrolStart>) -> Result<Json<Value>, ApiError> {
    if !st.anon_limit.allow(ip, now()) {
        return Err(ApiError::too_many());
    }
    if !abshare_core::auth::codes::is_token(&b.token) {
        return Err(ApiError::not_found());
    }
    let h = token_hash(&b.token);
    let inv = st.db.run(move |c| store::find_invite(c, &h, now())).await?.ok_or_else(ApiError::not_found)?;
    let (username, webauthn_id, display_name) = if inv.purpose == "reset" {
        let uid = inv.for_user.ok_or_else(ApiError::not_found)?;
        let u = st.db.run(move |c| store::user_by_id(c, uid)).await?.ok_or_else(ApiError::not_found)?;
        let wid = Uuid::parse_str(&u.webauthn_id).map_err(ApiError::internal)?;
        (u.username, wid, u.display_name)
    } else {
        let username = validate::username(&b.username).map_err(|e| ApiError::bad(e.to_string()))?;
        let check = username.clone();
        if st.db.run(move |c| store::user_by_name(c, &check)).await?.is_some() {
            return Err(ApiError::new(StatusCode::CONFLICT, "username_taken", "That username is taken."));
        }
        let display = inv.display_name.clone().unwrap_or_else(|| username.clone());
        (username, Uuid::from_bytes(random::<16>()), display)
    };
    let e = Enrol {
        invite_hash: h,
        purpose: inv.purpose,
        for_user: inv.for_user,
        username: username.clone(),
        webauthn_id,
        display_name,
        role: inv.role,
        perms: inv.perms,
        password_hash: None,
        totp_seed: None,
        totp_step: None,
        passkey: None,
        passkey_reg: None,
    };
    let enrol = st.enrol.start(e, now()).ok_or_else(ApiError::too_many)?;
    Ok(Json(json!({ "enrol": enrol, "username": username })))
}

fn take_enrol(st: &State, token: &str) -> Result<Enrol, ApiError> {
    st.enrol.take(token, now()).ok_or_else(|| {
        ApiError::new(StatusCode::GONE, "enrolment_expired", "Enrolment expired. Open the invite link again.")
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrolPassword {
    enrol: String,
    password: String,
}

async fn enrol_password(S(st): S<State>, Json(b): Json<EnrolPassword>) -> Result<Json<Value>, ApiError> {
    let mut e = take_enrol(&st, &b.enrol)?;
    if let Err(err) = validate::password(&b.password, &e.username) {
        st.enrol.put_back(&b.enrol, e, now());
        return Err(ApiError::bad(err.to_string()));
    }
    e.password_hash = Some(st.hasher.hash(b.password).await?);
    let seed = random::<{ totp::SECRET_LEN }>().to_vec();
    let uri = totp::otpauth_uri(&st.issuer, &e.username, &seed);
    let secret = totp::secret_base32(&seed);
    e.totp_seed = Some(seed);
    e.totp_step = None;
    st.enrol.put_back(&b.enrol, e, now());
    Ok(Json(json!({ "otpauth_uri": uri, "secret": secret, "qr_svg": qr_svg(&uri) })))
}

fn qr_svg(data: &str) -> Option<String> {
    let code = qrcode::QrCode::new(data.as_bytes()).ok()?;
    Some(code.render::<qrcode::render::svg::Color>().min_dimensions(200, 200).quiet_zone(true).build())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrolCode {
    enrol: String,
    code: String,
}

async fn enrol_totp(S(st): S<State>, Json(b): Json<EnrolCode>) -> Result<Json<Value>, ApiError> {
    let mut e = take_enrol(&st, &b.enrol)?;
    let Some(seed) = e.totp_seed.clone() else {
        st.enrol.put_back(&b.enrol, e, now());
        return Err(ApiError::bad("Set a password first."));
    };
    match totp::verify(&seed, &b.code, now(), None) {
        Ok(step) => {
            e.totp_step = Some(step);
            st.enrol.put_back(&b.enrol, e, now());
            Ok(Json(json!({ "ok": true })))
        }
        Err(_) => {
            st.enrol.put_back(&b.enrol, e, now());
            Err(ApiError::bad("That code didn't match. Check the time on your phone and try again."))
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrolOnly {
    enrol: String,
}

async fn enrol_passkey_start(S(st): S<State>, Json(b): Json<EnrolOnly>) -> Result<Json<Value>, ApiError> {
    let mut e = take_enrol(&st, &b.enrol)?;
    let (options, reg) = st
        .webauthn
        .start_passkey_registration(e.webauthn_id, &e.username, &e.display_name, None)
        .map_err(ApiError::internal)?;
    e.passkey_reg = Some(reg);
    st.enrol.put_back(&b.enrol, e, now());
    Ok(Json(json!({ "options": options })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrolPasskeyFinish {
    enrol: String,
    name: String,
    credential: RegisterPublicKeyCredential,
}

async fn enrol_passkey_finish(S(st): S<State>, Json(b): Json<EnrolPasskeyFinish>) -> Result<Json<Value>, ApiError> {
    let mut e = take_enrol(&st, &b.enrol)?;
    let Some(reg) = e.passkey_reg.take() else {
        st.enrol.put_back(&b.enrol, e, now());
        return Err(ApiError::bad("Start passkey creation first."));
    };
    match st.webauthn.finish_passkey_registration(&b.credential, &reg) {
        Ok(pk) => {
            e.passkey = Some((validate::label(&b.name, "Passkey"), pk));
            st.enrol.put_back(&b.enrol, e, now());
            Ok(Json(json!({ "ok": true })))
        }
        Err(_) => {
            st.enrol.put_back(&b.enrol, e, now());
            Err(ApiError::bad("The passkey couldn't be created. Try again."))
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrolFinish {
    enrol: String,
    device_name: String,
}

async fn enrol_finish(
    S(st): S<State>,
    Ip(ip): Ip,
    headers: HeaderMap,
    Json(b): Json<EnrolFinish>,
) -> Result<Response, ApiError> {
    let e = take_enrol(&st, &b.enrol)?;
    let has_passkey = e.passkey.is_some();
    let has_pw_totp = e.password_hash.is_some() && e.totp_step.is_some();
    if !has_passkey && !has_pw_totp {
        st.enrol.put_back(&b.enrol, e, now());
        return Err(ApiError::bad("Create a passkey, or set a password and confirm an authenticator code."));
    }
    let codes: Vec<String> = (0..10).map(|_| recovery_code(&random::<8>())).collect();
    let hashes: Vec<[u8; 32]> = codes.iter().map(|c| token_hash(c)).collect();
    let passkey = e
        .passkey
        .as_ref()
        .map(|(name, pk)| Ok::<_, String>((name.clone(), passkeys::cred_id(pk), passkeys::to_json(pk)?)))
        .transpose()
        .map_err(ApiError::internal)?;
    let seeds_key = st.seeds.clone();
    let e2 = e.clone();
    let user_id = st
        .db
        .run(move |c| {
            let tx = c.transaction().map_err(crate::db::sql_err)?;
            if !store::consume_invite(&tx, &e2.invite_hash, now())? {
                return Ok(None);
            }
            let uid = if e2.purpose == "reset" {
                let uid = e2.for_user.ok_or("reset invite without a user")?;
                tx.execute("DELETE FROM passkey WHERE user_id = ?1", [uid]).map_err(crate::db::sql_err)?;
                tx.execute(
                    "UPDATE user SET password_hash = NULL, totp_secret = NULL, totp_last_step = NULL WHERE id = ?1",
                    [uid],
                )
                .map_err(crate::db::sql_err)?;
                store::revoke_all(&tx, uid, None, now())?;
                uid
            } else {
                store::insert_user(
                    &tx,
                    &NewUser {
                        username: &e2.username,
                        webauthn_id: &e2.webauthn_id.to_string(),
                        display_name: &e2.display_name,
                        role: e2.role,
                        perms: &e2.perms,
                        password_hash: None,
                    },
                    now(),
                )?
            };
            if let (Some(ph), Some(seed), Some(step)) = (&e2.password_hash, &e2.totp_seed, e2.totp_step) {
                store::set_password(&tx, uid, Some(ph))?;
                store::set_totp(&tx, uid, Some(&seeds_key.seal(uid, seed)), Some(step))?;
            }
            if let Some((name, cred, data)) = &passkey {
                store::add_passkey(&tx, uid, name, cred, data, now())?;
            }
            store::replace_recovery(&tx, uid, &hashes)?;
            tx.commit().map_err(crate::db::sql_err)?;
            Ok(Some(uid))
        })
        .await?;
    let Some(uid) = user_id else {
        return Err(ApiError::new(
            StatusCode::GONE,
            "invite_used",
            "This invite has already been used or has expired.",
        ));
    };
    if e.purpose == "reset" {
        st.sessions.revoke_user(uid);
    }
    st.audit(
        if e.purpose == "reset" { "factors.reset" } else { "enrol.ok" },
        Some(uid),
        None,
        Some(ip),
        format!("passkey={has_passkey} totp={has_pw_totp}"),
    )
    .await;
    complete_login(&st, uid, &b.device_name, ip, ua(&headers), json!({ "recovery_codes": codes })).await
}

// ---------------------------------------------------------------- sign-in

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginPassword {
    username: String,
    password: String,
    device_name: String,
}

fn refusal(r: Refusal) -> ApiError {
    match r {
        Refusal::AddressLocked { .. } | Refusal::UserThrottled => ApiError::too_many(),
    }
}

async fn login_password(S(st): S<State>, Ip(ip): Ip, Json(b): Json<LoginPassword>) -> Result<Json<Value>, ApiError> {
    if b.password.len() > 4096 {
        return Err(ApiError::denied());
    }
    let username = b.username.trim().to_ascii_lowercase();
    let attempt =
        st.limiter.lock().map_err(|_| ApiError::internal("limiter"))?.begin(ip, &username, now()).map_err(refusal)?;
    let lookup = username.clone();
    let user = st.db.run(move |c| store::user_by_name(c, &lookup)).await?;
    let stored = user.as_ref().filter(|u| !u.disabled).and_then(|u| u.password_hash.clone());
    let ok = st.hasher.verify(b.password, stored).await?;
    let Some(user) = user.filter(|_| ok) else {
        st.audit(
            "login.failed",
            None,
            None,
            Some(ip),
            format!("user={}", username.chars().take(32).collect::<String>()),
        )
        .await;
        return Err(ApiError::denied());
    };
    // A password always needs a second factor: TOTP, or a recovery code. (Passkey sign-in is
    // a complete method of its own, on /login/passkey.)
    let mut methods = Vec::new();
    if user.totp_secret.is_some() {
        methods.push("totp");
    }
    methods.push("recovery");
    let pending = LoginPending { user: user.id, device_name: b.device_name, attempt };
    let challenge = st.login.start(pending, now()).ok_or_else(ApiError::too_many)?;
    Ok(Json(json!({ "challenge": challenge, "methods": methods })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SecondFactor {
    challenge: String,
    code: Option<String>,
}

async fn finish_second(
    st: &State,
    ip: std::net::IpAddr,
    headers: &HeaderMap,
    p: LoginPending,
    ok: bool,
    how: &str,
) -> Result<Response, ApiError> {
    if !ok {
        st.audit("login.second_factor_failed", Some(p.user), None, Some(ip), how.to_owned()).await;
        return Err(ApiError::denied());
    }
    if let Ok(mut l) = st.limiter.lock() {
        l.succeeded(&p.attempt);
    }
    complete_login(st, p.user, &p.device_name, ip, ua(headers), json!({})).await
}

async fn login_totp(
    S(st): S<State>,
    Ip(ip): Ip,
    headers: HeaderMap,
    Json(b): Json<SecondFactor>,
) -> Result<Response, ApiError> {
    let p = st.login.take(&b.challenge, now()).ok_or_else(ApiError::denied)?;
    let uid = p.user;
    let user = st.db.run(move |c| store::user_by_id(c, uid)).await?.ok_or_else(ApiError::denied)?;
    let seed = user.totp_secret.as_deref().and_then(|s| st.seeds.open(uid, s));
    let code = b.code.unwrap_or_default();
    let ok = match seed.map(|s| totp::verify(&s, &code, now(), user.totp_last_step)) {
        Some(Ok(step)) => st.db.run(move |c| store::use_totp_step(c, uid, step)).await?,
        _ => false,
    };
    finish_second(&st, ip, &headers, p, ok, "totp").await
}

async fn login_recovery(
    S(st): S<State>,
    Ip(ip): Ip,
    headers: HeaderMap,
    Json(b): Json<SecondFactor>,
) -> Result<Response, ApiError> {
    let p = st.login.take(&b.challenge, now()).ok_or_else(ApiError::denied)?;
    let uid = p.user;
    let ok = match b.code.as_deref().and_then(normalise_recovery) {
        Some(code) => {
            let h = token_hash(&code);
            st.db.run(move |c| store::use_recovery(c, uid, &h, now())).await?
        }
        None => false,
    };
    if ok {
        st.audit("recovery.used", Some(uid), None, Some(ip), String::new()).await;
    }
    finish_second(&st, ip, &headers, p, ok, "recovery").await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasskeyStart {
    username: Option<String>,
}

async fn login_passkey_start(
    S(st): S<State>,
    Ip(ip): Ip,
    Json(b): Json<PasskeyStart>,
) -> Result<Json<Value>, ApiError> {
    if !st.anon_limit.allow(ip, now()) {
        return Err(ApiError::too_many());
    }
    let (options, state) = match b.username.map(|u| u.trim().to_ascii_lowercase()).filter(|u| !u.is_empty()) {
        Some(u) => {
            let found = st
                .db
                .run(move |c| {
                    let Some(user) = store::user_by_name(c, &u)? else { return Ok(None) };
                    Ok(Some((user.id, store::passkeys(c, user.id)?)))
                })
                .await?;
            let (uid, rows) = found.unwrap_or((0, vec![]));
            let keys: Vec<_> = rows.iter().filter_map(|p| passkeys::from_json(&p.data)).collect();
            if keys.is_empty() {
                // Don't reveal whether the user exists: fall back to the discoverable flow.
                let (o, s) = st.webauthn.start_discoverable_authentication().map_err(ApiError::internal)?;
                (json!(o), PasskeyLogin::Discoverable(s))
            } else {
                let (o, s) = st.webauthn.start_passkey_authentication(&keys).map_err(ApiError::internal)?;
                (json!(o), PasskeyLogin::ForUser(uid, s))
            }
        }
        None => {
            let (o, s) = st.webauthn.start_discoverable_authentication().map_err(ApiError::internal)?;
            (json!(o), PasskeyLogin::Discoverable(s))
        }
    };
    let challenge = st.passkey_login.start(state, now()).ok_or_else(ApiError::too_many)?;
    Ok(Json(json!({ "challenge": challenge, "options": options })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasskeyFinish {
    challenge: String,
    device_name: String,
    credential: PublicKeyCredential,
}

async fn login_passkey_finish(
    S(st): S<State>,
    Ip(ip): Ip,
    headers: HeaderMap,
    Json(b): Json<PasskeyFinish>,
) -> Result<Response, ApiError> {
    let state = st.passkey_login.take(&b.challenge, now()).ok_or_else(ApiError::denied)?;
    let cred: Vec<u8> = b.credential.raw_id.as_ref().to_vec();
    let row = st.db.run(move |c| store::passkey_by_cred(c, &cred)).await?;
    let Some((pk_id, uid, data)) = row else {
        st.audit("login.passkey_failed", None, None, Some(ip), "unknown credential".into()).await;
        return Err(ApiError::denied());
    };
    let Some(mut pk) = passkeys::from_json(&data) else { return Err(ApiError::internal("stored passkey unreadable")) };
    let result = match state {
        PasskeyLogin::ForUser(expected, s) if expected == uid => {
            st.webauthn.finish_passkey_authentication(&b.credential, &s).ok()
        }
        PasskeyLogin::ForUser(..) => None,
        PasskeyLogin::Discoverable(s) => {
            let user = st.db.run(move |c| store::user_by_id(c, uid)).await?;
            let handle_ok = st
                .webauthn
                .identify_discoverable_authentication(&b.credential)
                .ok()
                .zip(user.as_ref())
                .is_some_and(|((h, _), u)| h.to_string() == u.webauthn_id);
            if handle_ok {
                st.webauthn.finish_discoverable_authentication(&b.credential, s, &[(&pk).into()]).ok()
            } else {
                None
            }
        }
    };
    let Some(result) = result.filter(|r| r.user_verified()) else {
        st.audit("login.passkey_failed", Some(uid), None, Some(ip), String::new()).await;
        return Err(ApiError::denied());
    };
    if pk.update_credential(&result).is_some() {
        let data = passkeys::to_json(&pk).map_err(ApiError::internal)?;
        st.db.run(move |c| store::update_passkey(c, pk_id, &data, now())).await?;
    }
    complete_login(&st, uid, &b.device_name, ip, ua(&headers), json!({})).await
}

// ---------------------------------------------------------------- refresh, sign-out

async fn refresh(S(st): S<State>, Ip(ip): Ip, headers: HeaderMap) -> Result<Response, ApiError> {
    let old = cookie(&headers, REFRESH_COOKIE)
        .filter(|t| abshare_core::auth::codes::is_token(t))
        .ok_or_else(ApiError::unauthenticated)?;
    let oh = token_hash(&old);
    let (new, nh) = new_token();
    let outcome = st
        .db
        .run(move |c| {
            let tx = c.transaction().map_err(crate::db::sql_err)?;
            let o = store::rotate_refresh(&tx, &oh, &nh, now())?;
            tx.commit().map_err(crate::db::sql_err)?;
            Ok(o)
        })
        .await?;
    match outcome {
        RefreshOutcome::Rotated { device, user } => {
            let u = st.db.run(move |c| store::user_by_id(c, user)).await?.ok_or_else(ApiError::unauthenticated)?;
            let (access, media) = st
                .sessions
                .issue(user, device, u.role, now())
                .ok_or_else(|| ApiError::internal("session table full"))?;
            let mut r = Json(json!({ "access_token": access, "expires_in": crate::auth::sessions::ACCESS_TTL,
                "user": { "username": u.username, "display_name": u.display_name, "role": u.role.as_str() } }))
            .into_response();
            r.headers_mut()
                .append(header::SET_COOKIE, set_cookie(REFRESH_COOKIE, &new, "/v1/auth", store::REFRESH_LIFETIME));
            r.headers_mut().append(header::SET_COOKIE, set_cookie(MEDIA_COOKIE, &media, "/v1/media", MEDIA_TTL));
            Ok(r)
        }
        RefreshOutcome::Reused { device, user } => {
            st.sessions.revoke_device(device);
            st.audit(
                "refresh.reuse_revoked",
                Some(user),
                Some(device),
                Some(ip),
                "refresh token reused: device signed out".into(),
            )
            .await;
            Err(ApiError::unauthenticated())
        }
        RefreshOutcome::TooFast => Err(ApiError::too_many()),
        RefreshOutcome::Unknown | RefreshOutcome::Ended => Err(ApiError::unauthenticated()),
    }
}

async fn logout(S(st): S<State>, Ip(ip): Ip, headers: HeaderMap) -> Result<Response, ApiError> {
    if let Some(t) = cookie(&headers, REFRESH_COOKIE).filter(|t| abshare_core::auth::codes::is_token(t)) {
        let h = token_hash(&t);
        let found = st
            .db
            .run(move |c| {
                let row: Option<(i64, i64)> = rusqlite::OptionalExtension::optional(c.query_row(
                    "SELECT r.device_id, d.user_id FROM refresh r JOIN device d ON d.id = r.device_id WHERE r.hash = ?1",
                    [h.as_slice()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                ))
                .map_err(crate::db::sql_err)?;
                if let Some((device, user)) = row {
                    store::revoke_device(c, user, device, now())?;
                }
                Ok(row)
            })
            .await?;
        if let Some((device, user)) = found {
            st.sessions.revoke_device(device);
            st.audit("logout", Some(user), Some(device), Some(ip), String::new()).await;
        }
    }
    let mut r = StatusCode::NO_CONTENT.into_response();
    r.headers_mut().append(header::SET_COOKIE, set_cookie(REFRESH_COOKIE, "", "/v1/auth", 0));
    r.headers_mut().append(header::SET_COOKIE, set_cookie(MEDIA_COOKIE, "", "/v1/media", 0));
    r.headers_mut().insert("clear-site-data", axum::http::HeaderValue::from_static("\"cache\""));
    Ok(r)
}

// ---------------------------------------------------------------- step-up

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StepUp {
    password: Option<String>,
    code: Option<String>,
}

async fn step_up_password(
    S(st): S<State>,
    Ip(ip): Ip,
    Authed(s): Authed,
    Json(b): Json<StepUp>,
) -> Result<Json<Value>, ApiError> {
    let uid = s.user;
    let user = st.db.run(move |c| store::user_by_id(c, uid)).await?.ok_or_else(ApiError::unauthenticated)?;
    let attempt = st
        .limiter
        .lock()
        .map_err(|_| ApiError::internal("limiter"))?
        .begin(ip, &user.username, now())
        .map_err(refusal)?;
    let pw = b.password.unwrap_or_default();
    if pw.len() > 4096 || !st.hasher.verify(pw, user.password_hash.clone()).await? {
        st.audit("step_up.failed", Some(uid), Some(s.device), Some(ip), "password".into()).await;
        return Err(ApiError::denied());
    }
    // A password alone isn't enough for step-up when a second factor exists: TOTP follows.
    if user.totp_secret.is_some() {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "totp_required", "Enter an authenticator code too."));
    }
    if let Ok(mut l) = st.limiter.lock() {
        l.succeeded(&attempt);
    }
    st.sessions.mark_stepped_up(s.device, now());
    Ok(Json(json!({ "ok": true })))
}

async fn step_up_totp(
    S(st): S<State>,
    Ip(ip): Ip,
    Authed(s): Authed,
    Json(b): Json<StepUp>,
) -> Result<Json<Value>, ApiError> {
    let uid = s.user;
    let user = st.db.run(move |c| store::user_by_id(c, uid)).await?.ok_or_else(ApiError::unauthenticated)?;
    let attempt = st
        .limiter
        .lock()
        .map_err(|_| ApiError::internal("limiter"))?
        .begin(ip, &user.username, now())
        .map_err(refusal)?;
    let pw_ok = match user.password_hash.clone() {
        Some(h) => st.hasher.verify(b.password.unwrap_or_default(), Some(h)).await?,
        None => false,
    };
    let seed = user.totp_secret.as_deref().and_then(|x| st.seeds.open(uid, x));
    let code = b.code.unwrap_or_default();
    let totp_ok = match seed.map(|x| totp::verify(&x, &code, now(), user.totp_last_step)) {
        Some(Ok(step)) => st.db.run(move |c| store::use_totp_step(c, uid, step)).await?,
        _ => false,
    };
    if !(pw_ok && totp_ok) {
        st.audit("step_up.failed", Some(uid), Some(s.device), Some(ip), "password+totp".into()).await;
        return Err(ApiError::denied());
    }
    if let Ok(mut l) = st.limiter.lock() {
        l.succeeded(&attempt);
    }
    st.sessions.mark_stepped_up(s.device, now());
    Ok(Json(json!({ "ok": true })))
}

async fn step_up_passkey_start(S(st): S<State>, Authed(s): Authed) -> Result<Json<Value>, ApiError> {
    let uid = s.user;
    let rows = st.db.run(move |c| store::passkeys(c, uid)).await?;
    let keys: Vec<_> = rows.iter().filter_map(|p| passkeys::from_json(&p.data)).collect();
    if keys.is_empty() {
        return Err(ApiError::bad("No passkeys on this account."));
    }
    let (options, state) = st.webauthn.start_passkey_authentication(&keys).map_err(ApiError::internal)?;
    let challenge = st.passkey_login.start(PasskeyLogin::ForUser(uid, state), now()).ok_or_else(ApiError::too_many)?;
    Ok(Json(json!({ "challenge": challenge, "options": options })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StepUpPasskey {
    challenge: String,
    credential: PublicKeyCredential,
}

async fn step_up_passkey_finish(
    S(st): S<State>,
    Ip(ip): Ip,
    Authed(s): Authed,
    Json(b): Json<StepUpPasskey>,
) -> Result<Json<Value>, ApiError> {
    let Some(PasskeyLogin::ForUser(uid, state)) = st.passkey_login.take(&b.challenge, now()) else {
        return Err(ApiError::denied());
    };
    let ok = uid == s.user
        && st.webauthn.finish_passkey_authentication(&b.credential, &state).is_ok_and(|r| r.user_verified());
    if !ok {
        st.audit("step_up.failed", Some(s.user), Some(s.device), Some(ip), "passkey".into()).await;
        return Err(ApiError::denied());
    }
    st.sessions.mark_stepped_up(s.device, now());
    Ok(Json(json!({ "ok": true })))
}
