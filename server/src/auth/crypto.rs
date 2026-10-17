//! Password hashing, opaque tokens and TOTP-seed encryption (SPEC §5.5, §9.3).

use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

/// argon2id cost: 64 MiB, 3 passes, 1 lane (stored in each PHC string, so it can be raised
/// later and old hashes still verify).
const M_KIB: u32 = 64 * 1024;
const T_COST: u32 = 3;
const P_COST: u32 = 1;
/// Hashes allowed at once; others wait up to [`WAIT`] and are then refused as busy.
const PERMITS: usize = 4;
const WAIT: Duration = Duration::from_secs(2);

pub fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("the operating system's random source failed");
    b
}

#[derive(Debug, PartialEq, Eq)]
pub enum HashError {
    Busy,
    Internal,
}

#[derive(Clone)]
pub struct Hasher {
    permits: Arc<Semaphore>,
    /// A real hash of a random password: unknown usernames are checked against it, so the
    /// time taken doesn't reveal whether an account exists.
    dummy: Arc<String>,
    params: Params,
}

impl Hasher {
    pub fn new() -> Self {
        Self::with_params(Params::new(M_KIB, T_COST, P_COST, None).expect("valid argon2 parameters"))
    }

    /// Cheaper parameters for tests only.
    #[cfg(test)]
    pub fn for_tests() -> Self {
        Self::with_params(Params::new(1024, 1, 1, None).expect("valid argon2 parameters"))
    }

    fn with_params(params: Params) -> Self {
        let a = Argon2::new(Algorithm::Argon2id, Version::V0x13, params.clone());
        let dummy = a
            .hash_password_with_salt(&random::<32>(), &random::<16>())
            .expect("hashing a random password works")
            .to_string();
        Self { permits: Arc::new(Semaphore::new(PERMITS)), dummy: Arc::new(dummy), params }
    }

    fn argon(&self) -> Argon2<'static> {
        Argon2::new(Algorithm::Argon2id, Version::V0x13, self.params.clone())
    }

    pub async fn hash(&self, password: String) -> Result<String, HashError> {
        let _p = tokio::time::timeout(WAIT, self.permits.clone().acquire_owned())
            .await
            .map_err(|_| HashError::Busy)?
            .map_err(|_| HashError::Internal)?;
        let a = self.argon();
        tokio::task::spawn_blocking(move || {
            a.hash_password_with_salt(password.as_bytes(), &random::<16>()).map(|h| h.to_string())
        })
        .await
        .map_err(|_| HashError::Internal)?
        .map_err(|_| HashError::Internal)
    }

    /// Verifies `password` against `stored`, or against the dummy hash when there is no
    /// account (always false then). Takes the same time either way.
    pub async fn verify(&self, password: String, stored: Option<String>) -> Result<bool, HashError> {
        let _p = tokio::time::timeout(WAIT, self.permits.clone().acquire_owned())
            .await
            .map_err(|_| HashError::Busy)?
            .map_err(|_| HashError::Internal)?;
        let a = self.argon();
        let real = stored.is_some();
        let phc = stored.unwrap_or_else(|| self.dummy.as_str().to_owned());
        let ok = tokio::task::spawn_blocking(move || a.verify_password(password.as_bytes(), phc.as_str()).is_ok())
            .await
            .map_err(|_| HashError::Internal)?;
        Ok(ok && real)
    }
}

/// A new opaque token: (text for the client, SHA-256 for storage).
pub fn new_token() -> (String, [u8; 32]) {
    let text = abshare_core::auth::codes::token_text(&random::<32>());
    let h = token_hash(&text);
    (text, h)
}

pub fn token_hash(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

/// Encrypts TOTP seeds at rest (XChaCha20-Poly1305), bound to the user ID, so a copy of the
/// database alone doesn't reveal them.
pub struct SeedBox {
    cipher: XChaCha20Poly1305,
}

impl SeedBox {
    pub fn new(key: &[u8; 32]) -> Self {
        Self { cipher: XChaCha20Poly1305::new_from_slice(key).expect("32-byte key") }
    }

    pub fn seal(&self, user_id: i64, seed: &[u8]) -> Vec<u8> {
        let nonce = random::<24>();
        let ct = self
            .cipher
            .encrypt(&XNonce::from(nonce), Payload { msg: seed, aad: &user_id.to_be_bytes() })
            .expect("encryption with a valid key cannot fail");
        let mut out = nonce.to_vec();
        out.extend(ct);
        out
    }

    pub fn open(&self, user_id: i64, sealed: &[u8]) -> Option<Vec<u8>> {
        if sealed.len() < 24 + 16 {
            return None;
        }
        let (nonce, ct) = sealed.split_at(24);
        let nonce: [u8; 24] = nonce.try_into().ok()?;
        self.cipher.decrypt(&XNonce::from(nonce), Payload { msg: ct, aad: &user_id.to_be_bytes() }).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hashing_verifies_the_right_password_only() {
        let h = Hasher::for_tests();
        let phc = h.hash("correct horse battery".into()).await.expect("hash");
        assert!(phc.starts_with("$argon2id$v=19$"));
        assert_eq!(h.verify("correct horse battery".into(), Some(phc.clone())).await, Ok(true));
        assert_eq!(h.verify("wrong horse battery".into(), Some(phc)).await, Ok(false));
    }

    #[tokio::test]
    async fn an_unknown_account_never_verifies_even_with_the_dummy_password() {
        let h = Hasher::for_tests();
        assert_eq!(h.verify(String::new(), None).await, Ok(false));
    }

    #[tokio::test]
    async fn production_parameters_are_in_the_phc_string() {
        let h = Hasher::new();
        let phc = h.hash("correct horse battery".into()).await.expect("hash");
        assert!(phc.contains("m=65536,t=3,p=1"), "{phc}");
    }

    #[test]
    fn tokens_are_random_and_hashed() {
        let (a, ha) = new_token();
        let (b, _) = new_token();
        assert_ne!(a, b);
        assert!(abshare_core::auth::codes::is_token(&a));
        assert_eq!(token_hash(&a), ha);
    }

    #[test]
    fn seeds_are_bound_to_their_user_and_tamper_evident() {
        let sb = SeedBox::new(&[9; 32]);
        let sealed = sb.seal(7, b"0123456789abcdefghij");
        assert_eq!(sb.open(7, &sealed).as_deref(), Some(&b"0123456789abcdefghij"[..]));
        assert_eq!(sb.open(8, &sealed), None, "moved to another user");
        let mut t = sealed.clone();
        t[30] ^= 1;
        assert_eq!(sb.open(7, &t), None, "tampered");
        assert_eq!(SeedBox::new(&[1; 32]).open(7, &sealed), None, "different key");
        assert_eq!(sb.open(7, &sealed[..10]), None);
    }
}
