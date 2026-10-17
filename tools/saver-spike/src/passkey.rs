//! Phase 0(e): passkey registration and sign-in with `webauthn-rs`, in memory only.
//!
//! Proves the flow end to end in Safari, Chrome and Firefox on the real origin. No session
//! is created: a successful sign-in just reports which passkey was used.

use crate::Shared;
use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use webauthn_rs::prelude::*;

/// Ceremonies older than this are refused (SPEC §5.5: challenges last 2 minutes).
const CEREMONY_TTL: Duration = Duration::from_secs(120);
/// The spike keeps at most this many passkeys and pending ceremonies.
const MAX_ITEMS: usize = 32;

pub struct Store {
    webauthn: Webauthn,
    user: Uuid,
    keys: Mutex<Vec<(String, Passkey)>>,
    reg: Mutex<HashMap<Uuid, (Instant, String, PasskeyRegistration)>>,
    auth: Mutex<HashMap<Uuid, (Instant, PasskeyAuthentication)>>,
}

impl Store {
    pub fn new(origin: &str) -> Result<Self, String> {
        let url = Url::parse(origin).map_err(|e| format!("--origin: {e}"))?;
        let rp_id = url.host_str().ok_or("--origin needs a host")?.to_owned();
        let webauthn = WebauthnBuilder::new(&rp_id, &url)
            .and_then(|b| b.rp_name("ABShare spike").build())
            .map_err(|e| format!("webauthn: {e}"))?;
        Ok(Self {
            webauthn,
            user: Uuid::new_v4(),
            keys: Mutex::default(),
            reg: Mutex::default(),
            auth: Mutex::default(),
        })
    }
}

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/pk/register/start", post(register_start))
        .route("/pk/register/finish", post(register_finish))
        .route("/pk/login/start", post(login_start))
        .route("/pk/login/finish", post(login_finish))
}

type Reply<T> = Result<Json<T>, (StatusCode, String)>;

fn bad(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, e.to_string())
}

fn poisoned<T>(_: T) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, "lock poisoned".into())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRegistration {
    name: String,
}

#[derive(Serialize)]
pub struct Started<T> {
    id: Uuid,
    options: T,
}

async fn register_start(
    State(app): State<Shared>,
    Json(req): Json<StartRegistration>,
) -> Reply<Started<CreationChallengeResponse>> {
    let name: String = req.name.chars().filter(|c| !c.is_control()).take(64).collect();
    if name.trim().is_empty() {
        return Err(bad("name required"));
    }
    let store = &app.passkeys;
    let exclude: Vec<CredentialID> =
        store.keys.lock().map_err(poisoned)?.iter().map(|(_, k)| k.cred_id().clone()).collect();
    let (options, state) =
        store.webauthn.start_passkey_registration(store.user, "spike", "ABShare spike", Some(exclude)).map_err(bad)?;
    let id = Uuid::new_v4();
    let mut reg = store.reg.lock().map_err(poisoned)?;
    reg.retain(|_, (t, _, _)| t.elapsed() < CEREMONY_TTL);
    if reg.len() >= MAX_ITEMS {
        return Err((StatusCode::TOO_MANY_REQUESTS, "too many pending registrations".into()));
    }
    reg.insert(id, (Instant::now(), name, state));
    Ok(Json(Started { id, options }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinishRegistration {
    id: Uuid,
    credential: RegisterPublicKeyCredential,
}

#[derive(Serialize)]
pub struct Done {
    ok: bool,
    detail: String,
}

async fn register_finish(State(app): State<Shared>, Json(req): Json<FinishRegistration>) -> Reply<Done> {
    let store = &app.passkeys;
    let (started, name, state) =
        store.reg.lock().map_err(poisoned)?.remove(&req.id).ok_or_else(|| bad("unknown or used ceremony"))?;
    if started.elapsed() >= CEREMONY_TTL {
        return Err(bad("ceremony expired"));
    }
    let key = store.webauthn.finish_passkey_registration(&req.credential, &state).map_err(bad)?;
    let mut keys = store.keys.lock().map_err(poisoned)?;
    if keys.len() >= MAX_ITEMS {
        return Err((StatusCode::TOO_MANY_REQUESTS, "spike passkey store full".into()));
    }
    keys.push((name.clone(), key));
    Ok(Json(Done { ok: true, detail: format!("registered passkey \"{name}\" ({} stored)", keys.len()) }))
}

async fn login_start(State(app): State<Shared>) -> Reply<Started<RequestChallengeResponse>> {
    let store = &app.passkeys;
    let keys: Vec<Passkey> = store.keys.lock().map_err(poisoned)?.iter().map(|(_, k)| k.clone()).collect();
    if keys.is_empty() {
        return Err(bad("register a passkey first"));
    }
    let (options, state) = store.webauthn.start_passkey_authentication(&keys).map_err(bad)?;
    let id = Uuid::new_v4();
    let mut auth = store.auth.lock().map_err(poisoned)?;
    auth.retain(|_, (t, _)| t.elapsed() < CEREMONY_TTL);
    if auth.len() >= MAX_ITEMS {
        return Err((StatusCode::TOO_MANY_REQUESTS, "too many pending sign-ins".into()));
    }
    auth.insert(id, (Instant::now(), state));
    Ok(Json(Started { id, options }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinishLogin {
    id: Uuid,
    credential: PublicKeyCredential,
}

async fn login_finish(State(app): State<Shared>, Json(req): Json<FinishLogin>) -> Reply<Done> {
    let store = &app.passkeys;
    let (started, state) =
        store.auth.lock().map_err(poisoned)?.remove(&req.id).ok_or_else(|| bad("unknown or used ceremony"))?;
    if started.elapsed() >= CEREMONY_TTL {
        return Err(bad("ceremony expired"));
    }
    let result = store.webauthn.finish_passkey_authentication(&req.credential, &state).map_err(bad)?;
    let mut keys = store.keys.lock().map_err(poisoned)?;
    let mut name = String::from("?");
    for (n, k) in keys.iter_mut() {
        if k.update_credential(&result).is_some() {
            name = n.clone();
        }
    }
    Ok(Json(Done {
        ok: true,
        detail: format!(
            "signed in with \"{name}\" (user verified: {}, counter {})",
            result.user_verified(),
            result.counter()
        ),
    }))
}
