//! `/v1/me/*` (SPEC §6): your profile, devices and sign-in methods. Bearer-authenticated.
//! Every change to sign-in methods needs a step-up within the last 10 minutes, and no change
//! may leave the account without a complete way to sign in (a passkey, or password + TOTP).

use super::{ApiError, Authed, Ip, State, now, passkeys};
use crate::auth::crypto::{random, token_hash};
use crate::auth::sessions::Session;
use crate::auth::store;
use abshare_core::auth::codes::recovery_code;
use abshare_core::auth::{totp, validate};
use axum::extract::{Path, State as S};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use webauthn_rs::prelude::RegisterPublicKeyCredential;

pub fn routes() -> Router<State> {
    Router::new()
        .route("/", get(me))
        .route("/devices", get(devices))
        .route("/devices/{id}", delete(revoke_device))
        .route("/passkeys", get(list_passkeys))
        .route("/passkeys/start", post(add_passkey_start))
        .route("/passkeys/finish", post(add_passkey_finish))
        .route("/passkeys/{id}", delete(delete_passkey))
        .route("/password", post(change_password))
        .route("/totp/start", post(totp_start))
        .route("/totp/finish", post(totp_finish))
        .route("/totp", delete(remove_totp))
        .route("/recovery", post(new_recovery))
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024))
}

fn require_step_up(st: &State, s: &Session) -> Result<(), ApiError> {
    if st.sessions.stepped_up(s.device, now()) { Ok(()) } else { Err(ApiError::step_up()) }
}

/// Signs out every other device of the user, as a password change does: removing or replacing
/// a sign-in method is what someone does after losing one.
async fn sign_out_others(st: &State, s: &Session) -> Result<usize, ApiError> {
    let (uid, keep) = (s.user, s.device);
    let revoked = st.db.run(move |c| store::revoke_all(c, uid, Some(keep), now())).await?;
    for d in &revoked {
        st.sessions.revoke_device(*d);
    }
    Ok(revoked.len())
}

/// A new passkey or authenticator: the limit after a recovery-code sign-in is lifted.
async fn factor_added(st: &State, uid: i64) -> Result<(), ApiError> {
    st.db.run(move |c| store::set_needs_factor(c, uid, false)).await?;
    st.sessions.clear_needs_factor(uid);
    Ok(())
}

async fn me(S(st): S<State>, Authed(s): Authed) -> Result<Json<Value>, ApiError> {
    let uid = s.user;
    let (user, passkeys, recovery_left) = st
        .db
        .run(move |c| {
            let u = store::user_by_id(c, uid)?.ok_or("user vanished")?;
            Ok((u, store::passkeys(c, uid)?.len(), store::recovery_left(c, uid)?))
        })
        .await?;
    Ok(Json(json!({
        "username": user.username,
        "display_name": user.display_name,
        "role": user.role.as_str(),
        "device": s.device,
        "factors": { "passkeys": passkeys, "password": user.password_hash.is_some(), "totp": user.totp_secret.is_some(), "recovery_codes_left": recovery_left },
        "needs_factor": user.needs_factor,
        "permissions": { "offline": user.perms.can_offline, "save_files": user.perms.can_save },
    })))
}

async fn devices(S(st): S<State>, Authed(s): Authed) -> Result<Json<Value>, ApiError> {
    let uid = s.user;
    let list = st.db.run(move |c| store::devices(c, uid)).await?;
    Ok(Json(json!({ "devices": list.iter().map(|d| json!({
        "id": d.id, "name": d.name, "created_at": d.created_at, "last_seen": d.last_seen,
        "last_ip": d.last_ip, "user_agent": d.user_agent, "this_device": d.id == s.device,
    })).collect::<Vec<_>>() })))
}

async fn revoke_device(
    S(st): S<State>,
    Ip(ip): Ip,
    Authed(s): Authed,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let uid = s.user;
    // Signing yourself out is always allowed. Signing out another device needs a fresh factor,
    // so a stolen access token can't sign the owner out of everything (and wipe their
    // downloads); a per-device limit stops it being hammered either way.
    if !st.device_limit.allow(s.device, now()) {
        return Err(ApiError::too_many());
    }
    if id != s.device {
        require_step_up(&st, &s)?;
    }
    // Only your own devices: another user's device is "not found".
    if !st.db.run(move |c| store::revoke_device(c, uid, id, now())).await? {
        return Err(ApiError::not_found());
    }
    st.sessions.revoke_device(id);
    st.audit("device.revoked", Some(uid), Some(id), Some(ip), format!("by device {}", s.device)).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_passkeys(S(st): S<State>, Authed(s): Authed) -> Result<Json<Value>, ApiError> {
    let uid = s.user;
    let rows = st.db.run(move |c| store::passkeys(c, uid)).await?;
    Ok(Json(
        json!({ "passkeys": rows.iter().map(|p| json!({ "id": p.id, "name": p.name, "created_at": p.created_at, "last_used": p.last_used })).collect::<Vec<_>>() }),
    ))
}

async fn add_passkey_start(S(st): S<State>, Authed(s): Authed) -> Result<Json<Value>, ApiError> {
    require_step_up(&st, &s)?;
    let uid = s.user;
    let (user, rows) =
        st.db.run(move |c| Ok((store::user_by_id(c, uid)?.ok_or("user vanished")?, store::passkeys(c, uid)?))).await?;
    let exclude: Vec<_> =
        rows.iter().filter_map(|p| passkeys::from_json(&p.data)).map(|k| k.cred_id().clone()).collect();
    let wid = webauthn_rs::prelude::Uuid::parse_str(&user.webauthn_id).map_err(ApiError::internal)?;
    let (options, reg) = st
        .webauthn
        .start_passkey_registration(wid, &user.username, &user.display_name, Some(exclude))
        .map_err(ApiError::internal)?;
    let challenge = st.passkey_add.start((uid, reg), now()).ok_or_else(ApiError::too_many)?;
    Ok(Json(json!({ "challenge": challenge, "options": options })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddPasskey {
    challenge: String,
    name: String,
    credential: RegisterPublicKeyCredential,
}

async fn add_passkey_finish(
    S(st): S<State>,
    Ip(ip): Ip,
    Authed(s): Authed,
    Json(b): Json<AddPasskey>,
) -> Result<Json<Value>, ApiError> {
    require_step_up(&st, &s)?;
    let (uid, reg) = st.passkey_add.take(&b.challenge, now()).ok_or_else(ApiError::denied)?;
    if uid != s.user {
        return Err(ApiError::denied());
    }
    let pk = st
        .webauthn
        .finish_passkey_registration(&b.credential, &reg)
        .map_err(|_| ApiError::bad("The passkey couldn't be created."))?;
    let (name, cred, data) = (
        validate::label(&b.name, "Passkey"),
        passkeys::cred_id(&pk),
        passkeys::to_json(&pk).map_err(ApiError::internal)?,
    );
    let id = st.db.run(move |c| store::add_passkey(c, uid, &name, &cred, &data, now())).await?;
    factor_added(&st, uid).await?;
    st.audit("passkey.added", Some(uid), Some(s.device), Some(ip), String::new()).await;
    Ok(Json(json!({ "id": id })))
}

/// Whether the account would still have a complete sign-in method.
fn still_has_method(passkeys_left: usize, password: bool, totp: bool) -> bool {
    passkeys_left > 0 || (password && totp)
}

async fn delete_passkey(
    S(st): S<State>,
    Ip(ip): Ip,
    Authed(s): Authed,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    require_step_up(&st, &s)?;
    let uid = s.user;
    let outcome = st
        .db
        .run(move |c| {
            let u = store::user_by_id(c, uid)?.ok_or("user vanished")?;
            let keys = store::passkeys(c, uid)?;
            if !keys.iter().any(|k| k.id == id) {
                return Ok(Err(ApiError::not_found()));
            }
            if !still_has_method(keys.len() - 1, u.password_hash.is_some(), u.totp_secret.is_some()) {
                return Ok(Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "last_method",
                    "That's your only way to sign in. Add another first.",
                )));
            }
            store::delete_passkey(c, uid, id)?;
            Ok(Ok(()))
        })
        .await?;
    outcome?;
    let n = sign_out_others(&st, &s).await?;
    st.audit("passkey.removed", Some(uid), Some(s.device), Some(ip), format!("{n} other devices signed out")).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewPassword {
    password: String,
}

async fn change_password(
    S(st): S<State>,
    Ip(ip): Ip,
    Authed(s): Authed,
    Json(b): Json<NewPassword>,
) -> Result<StatusCode, ApiError> {
    require_step_up(&st, &s)?;
    let uid = s.user;
    let user = st.db.run(move |c| store::user_by_id(c, uid)).await?.ok_or_else(ApiError::unauthenticated)?;
    validate::password(&b.password, &user.username).map_err(|e| ApiError::bad(e.to_string()))?;
    let hash = st.hasher.hash(b.password).await?;
    let keep = s.device;
    let revoked = st
        .db
        .run(move |c| {
            store::set_password(c, uid, Some(&hash))?;
            store::revoke_all(c, uid, Some(keep), now())
        })
        .await?;
    for d in revoked {
        st.sessions.revoke_device(d);
    }
    st.audit("password.changed", Some(uid), Some(s.device), Some(ip), "other devices signed out".into()).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn totp_start(S(st): S<State>, Authed(s): Authed) -> Result<Json<Value>, ApiError> {
    require_step_up(&st, &s)?;
    let uid = s.user;
    let user = st.db.run(move |c| store::user_by_id(c, uid)).await?.ok_or_else(ApiError::unauthenticated)?;
    let seed = random::<{ totp::SECRET_LEN }>().to_vec();
    let uri = totp::otpauth_uri(&st.issuer, &user.username, &seed);
    let secret = totp::secret_base32(&seed);
    let challenge = st.totp_add.start((uid, seed), now()).ok_or_else(ApiError::too_many)?;
    let qr = qrcode::QrCode::new(uri.as_bytes())
        .ok()
        .map(|q| q.render::<qrcode::render::svg::Color>().min_dimensions(200, 200).build());
    Ok(Json(json!({ "challenge": challenge, "otpauth_uri": uri, "secret": secret, "qr_svg": qr })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TotpFinish {
    challenge: String,
    code: String,
}

async fn totp_finish(
    S(st): S<State>,
    Ip(ip): Ip,
    Authed(s): Authed,
    Json(b): Json<TotpFinish>,
) -> Result<StatusCode, ApiError> {
    require_step_up(&st, &s)?;
    let (uid, seed) = st.totp_add.take(&b.challenge, now()).ok_or_else(ApiError::denied)?;
    if uid != s.user {
        return Err(ApiError::denied());
    }
    let step = totp::verify(&seed, &b.code, now(), None).map_err(|_| ApiError::bad("That code didn't match."))?;
    let sealed = st.seeds.seal(uid, &seed);
    let replaced = st
        .db
        .run(move |c| {
            let had = store::user_by_id(c, uid)?.ok_or("user vanished")?.totp_secret.is_some();
            store::set_totp(c, uid, Some(&sealed), Some(step))?;
            Ok(had)
        })
        .await?;
    factor_added(&st, uid).await?;
    // Replacing the authenticator retires the old one, like removing it.
    let detail = if replaced {
        format!("replaced; {} other devices signed out", sign_out_others(&st, &s).await?)
    } else {
        String::new()
    };
    st.audit("totp.enabled", Some(uid), Some(s.device), Some(ip), detail).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_totp(S(st): S<State>, Ip(ip): Ip, Authed(s): Authed) -> Result<StatusCode, ApiError> {
    require_step_up(&st, &s)?;
    let uid = s.user;
    let outcome = st
        .db
        .run(move |c| {
            let keys = store::passkeys(c, uid)?;
            let u = store::user_by_id(c, uid)?.ok_or("user vanished")?;
            if !still_has_method(keys.len(), u.password_hash.is_some(), false) {
                return Ok(Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "last_method",
                    "Two-step codes are part of your only way to sign in. Add a passkey first.",
                )));
            }
            store::set_totp(c, uid, None, None)?;
            Ok(Ok(()))
        })
        .await?;
    outcome?;
    let n = sign_out_others(&st, &s).await?;
    st.audit("totp.removed", Some(uid), Some(s.device), Some(ip), format!("{n} other devices signed out")).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn new_recovery(S(st): S<State>, Ip(ip): Ip, Authed(s): Authed) -> Result<Json<Value>, ApiError> {
    require_step_up(&st, &s)?;
    let uid = s.user;
    let codes: Vec<String> = (0..10).map(|_| recovery_code(&random::<8>())).collect();
    let hashes: Vec<[u8; 32]> = codes.iter().map(|c| token_hash(c)).collect();
    st.db.run(move |c| store::replace_recovery(c, uid, &hashes)).await?;
    st.audit("recovery.regenerated", Some(uid), Some(s.device), Some(ip), String::new()).await;
    Ok(Json(json!({ "recovery_codes": codes })))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_complete_method_must_remain() {
        assert!(super::still_has_method(1, false, false));
        assert!(super::still_has_method(0, true, true));
        assert!(!super::still_has_method(0, true, false));
        assert!(!super::still_has_method(0, false, true));
    }
}
