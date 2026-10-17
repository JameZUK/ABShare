//! The server secret (SPEC §9.3): 32 random bytes in `secret.key` (0600), from which each
//! purpose gets its own key with HKDF and a distinct label.

use hkdf::Hkdf;
use sha2::Sha256;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

pub struct Secret([u8; 32]);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Labels for derived keys. Changing one invalidates everything derived from it.
pub mod purpose {
    pub const BOOK_ID: &[u8] = b"abshare v1 book id";
}

impl Secret {
    /// Loads `secret.key` from the data directory, creating it on first start.
    pub fn load_or_create(data_dir: &Path) -> Result<Self, String> {
        let path = data_dir.join("secret.key");
        if !path.exists() {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes).map_err(|e| format!("random: {e}"))?;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            f.write_all(&bytes).and_then(|()| f.sync_all()).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if !meta.is_file() || meta.permissions().mode() & 0o077 != 0 {
            return Err(format!("{} must be a regular file with mode 0600", path.display()));
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let arr: [u8; 32] = bytes.try_into().map_err(|_| format!("{} must hold exactly 32 bytes", path.display()))?;
        Ok(Self(arr))
    }

    /// A 32-byte key for one purpose.
    pub fn derive(&self, label: &[u8]) -> [u8; 32] {
        let hk = Hkdf::<Sha256>::new(None, &self.0);
        let mut out = [0u8; 32];
        hk.expand(label, &mut out).expect("32 bytes is a valid HKDF-SHA256 output length");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_secret_is_created_once_private_and_stable() {
        let dir = std::env::temp_dir().join(format!("abshare-secret-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let a = Secret::load_or_create(&dir).expect("create");
        let mode = std::fs::metadata(dir.join("secret.key")).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let b = Secret::load_or_create(&dir).expect("load");
        assert_eq!(a.derive(purpose::BOOK_ID), b.derive(purpose::BOOK_ID));
        assert_ne!(a.derive(purpose::BOOK_ID), a.derive(b"other"));
        assert_eq!(format!("{a:?}"), "Secret(<redacted>)");
        std::fs::set_permissions(dir.join("secret.key"), std::fs::Permissions::from_mode(0o644)).expect("chmod");
        assert!(Secret::load_or_create(&dir).is_err(), "a readable secret must be refused");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
