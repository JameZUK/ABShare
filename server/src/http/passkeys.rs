//! Storing and loading `webauthn-rs` passkeys.

use webauthn_rs::prelude::Passkey;

pub fn to_json(p: &Passkey) -> Result<String, String> {
    serde_json::to_string(p).map_err(|e| format!("passkey: {e}"))
}

pub fn from_json(s: &str) -> Option<Passkey> {
    serde_json::from_str(s).ok()
}

pub fn cred_id(p: &Passkey) -> Vec<u8> {
    p.cred_id().as_ref().to_vec()
}
