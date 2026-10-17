//! Short-lived state kept in memory only (SPEC §5.5): access tokens (15 min), media tokens
//! (12 h, rolling), step-up times, and pending sign-in or enrolment ceremonies (2–15 min).
//! Everything is keyed by a SHA-256 hash of the secret, bounded in size, and swept on use.
//! A restart signs no one out: clients refresh with their (stored) refresh token.

use super::crypto::{new_token, token_hash};
use super::store::Role;
use std::collections::HashMap;
use std::sync::Mutex;

pub const ACCESS_TTL: i64 = 15 * 60;
pub const MEDIA_TTL: i64 = 12 * 3600;
pub const STEP_UP_TTL: i64 = 10 * 60;
pub const CHALLENGE_TTL: i64 = 2 * 60;
pub const ENROL_TTL: i64 = 15 * 60;
pub const MAX_ACCESS_PER_DEVICE: usize = 8;
const MAX_ENTRIES: usize = 50_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Session {
    pub user: i64,
    pub device: i64,
    pub role: Role,
    pub expires: i64,
}

/// A pending ceremony: password accepted, waiting for the second factor; or a passkey
/// challenge; or enrolment in progress. `T` carries what the next step needs.
struct Pending<T> {
    value: T,
    expires: i64,
}

pub struct Ceremonies<T> {
    map: Mutex<HashMap<[u8; 32], Pending<T>>>,
    ttl: i64,
}

impl<T> Ceremonies<T> {
    pub fn new(ttl: i64) -> Self {
        Self { map: Mutex::new(HashMap::new()), ttl }
    }

    /// Stores `value` and returns the token that redeems it.
    pub fn start(&self, value: T, now: i64) -> Option<String> {
        let (text, h) = new_token();
        let mut m = self.map.lock().ok()?;
        m.retain(|_, p| p.expires > now);
        if m.len() >= MAX_ENTRIES {
            return None;
        }
        m.insert(h, Pending { value, expires: now + self.ttl });
        Some(text)
    }

    /// Removes and returns the value: each ceremony token works once.
    pub fn take(&self, token: &str, now: i64) -> Option<T> {
        if !abshare_core::auth::codes::is_token(token) {
            return None;
        }
        let mut m = self.map.lock().ok()?;
        let p = m.remove(&token_hash(token))?;
        (p.expires > now).then_some(p.value)
    }

    /// Puts a value back under the same token (for multi-step enrolment).
    pub fn put_back(&self, token: &str, value: T, now: i64) {
        if let Ok(mut m) = self.map.lock() {
            m.insert(token_hash(token), Pending { value, expires: now + self.ttl });
        }
    }
}

#[derive(Default)]
struct Inner {
    access: HashMap<[u8; 32], Session>,
    media: HashMap<[u8; 32], Session>,
    step_up: HashMap<i64, i64>,
}

#[derive(Default)]
pub struct Sessions {
    inner: Mutex<Inner>,
}

impl Sessions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Issues an access token and a media token for a device.
    pub fn issue(&self, user: i64, device: i64, role: Role, now: i64) -> Option<(String, String)> {
        let (access, ah) = new_token();
        let (media, mh) = new_token();
        let mut g = self.inner.lock().ok()?;
        g.access.retain(|_, s| s.expires > now);
        g.media.retain(|_, s| s.expires > now);
        if g.access.len() >= MAX_ENTRIES {
            return None;
        }
        // At most 8 live access tokens per device: drop the oldest.
        let mut mine: Vec<([u8; 32], i64)> =
            g.access.iter().filter(|(_, s)| s.device == device).map(|(k, s)| (*k, s.expires)).collect();
        mine.sort_by_key(|&(_, e)| e);
        while mine.len() >= MAX_ACCESS_PER_DEVICE {
            let (k, _) = mine.remove(0);
            g.access.remove(&k);
        }
        g.access.insert(ah, Session { user, device, role, expires: now + ACCESS_TTL });
        g.media.retain(|_, s| s.device != device);
        g.media.insert(mh, Session { user, device, role, expires: now + MEDIA_TTL });
        Some((access, media))
    }

    pub fn access(&self, token: &str, now: i64) -> Option<Session> {
        Self::lookup(&self.inner.lock().ok()?.access, token, now)
    }

    #[allow(dead_code)] // Used by the media route (Phase 3).
    pub fn media(&self, token: &str, now: i64) -> Option<Session> {
        Self::lookup(&self.inner.lock().ok()?.media, token, now)
    }

    fn lookup(map: &HashMap<[u8; 32], Session>, token: &str, now: i64) -> Option<Session> {
        if !abshare_core::auth::codes::is_token(token) {
            return None;
        }
        map.get(&token_hash(token)).copied().filter(|s| s.expires > now)
    }

    /// Ends every token of a device at once (sign-out, revocation, theft detection).
    pub fn revoke_device(&self, device: i64) {
        if let Ok(mut g) = self.inner.lock() {
            g.access.retain(|_, s| s.device != device);
            g.media.retain(|_, s| s.device != device);
            g.step_up.remove(&device);
        }
    }

    pub fn revoke_user(&self, user: i64) {
        if let Ok(mut g) = self.inner.lock() {
            g.access.retain(|_, s| s.user != user);
            g.media.retain(|_, s| s.user != user);
        }
    }

    pub fn mark_stepped_up(&self, device: i64, now: i64) {
        if let Ok(mut g) = self.inner.lock() {
            g.step_up.insert(device, now + STEP_UP_TTL);
        }
    }

    pub fn stepped_up(&self, device: i64, now: i64) -> bool {
        self.inner.lock().ok().and_then(|g| g.step_up.get(&device).copied()).is_some_and(|until| until > now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_work_until_they_expire_and_die_with_the_device() {
        let s = Sessions::new();
        let (a, m) = s.issue(1, 10, Role::Listener, 1000).expect("issue");
        assert_eq!(s.access(&a, 1000 + ACCESS_TTL - 1).map(|x| x.device), Some(10));
        assert!(s.access(&a, 1000 + ACCESS_TTL).is_none());
        assert!(s.media(&m, 1000 + MEDIA_TTL - 1).is_some());
        assert!(s.access(&m, 1000).is_none(), "a media token is not an access token");
        s.revoke_device(10);
        assert!(s.media(&m, 1001).is_none());
    }

    #[test]
    fn at_most_eight_access_tokens_per_device_and_one_media_token() {
        let s = Sessions::new();
        let first = s.issue(1, 10, Role::Listener, 1000).expect("issue");
        for i in 1..9 {
            s.issue(1, 10, Role::Listener, 1000 + i).expect("issue");
        }
        assert!(s.access(&first.0, 1010).is_none(), "oldest dropped");
        assert!(s.media(&first.1, 1010).is_none(), "media token replaced");
    }

    #[test]
    fn malformed_tokens_are_refused_without_lookup() {
        let s = Sessions::new();
        s.issue(1, 10, Role::Admin, 1000).expect("issue");
        for bad in ["", "x", &"A".repeat(64), "../etc"] {
            assert!(s.access(bad, 1000).is_none());
        }
    }

    #[test]
    fn step_up_lasts_ten_minutes() {
        let s = Sessions::new();
        assert!(!s.stepped_up(10, 0));
        s.mark_stepped_up(10, 1000);
        assert!(s.stepped_up(10, 1000 + STEP_UP_TTL - 1));
        assert!(!s.stepped_up(10, 1000 + STEP_UP_TTL));
    }

    #[test]
    fn ceremony_tokens_work_once_and_expire() {
        let c: Ceremonies<i64> = Ceremonies::new(CHALLENGE_TTL);
        let t = c.start(42, 1000).expect("start");
        assert_eq!(c.take(&t, 1001), Some(42));
        assert_eq!(c.take(&t, 1002), None, "single use");
        let t = c.start(43, 1000).expect("start");
        assert_eq!(c.take(&t, 1000 + CHALLENGE_TTL), None, "expired");
    }
}
