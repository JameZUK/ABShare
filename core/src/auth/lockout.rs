//! Sign-in throttling (SPEC §5.5). Pure: the server passes the client address and the time.
//!
//! - **Per address** (IPv6 per /64): 5 failures in 15 minutes lock the address. The lock
//!   starts at 15 minutes and doubles each time, up to 24 hours.
//! - **Per username**: after 5 failures in 15 minutes the username is soft-locked. Addresses
//!   that failed for it are refused, and all other addresses share 2 attempts a minute, so a
//!   stranger can't lock the owner out.
//! - An attempt counts as a failure **when it starts** ([`Limiter::begin`]) and is only
//!   forgiven when it succeeds, so parallel guesses can't get past the limits.
//! - Memory is bounded: at most [`MAX_ENTRIES`] addresses and usernames are tracked.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;

pub const WINDOW: i64 = 15 * 60;
pub const MAX_FAILURES: usize = 5;
pub const FIRST_LOCK: i64 = 15 * 60;
pub const MAX_LOCK: i64 = 24 * 3600;
pub const SHARED_PER_MINUTE: usize = 2;
pub const MAX_ENTRIES: usize = 10_000;

/// An address as throttled: IPv4 exactly, IPv6 by /64.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AddrKey(u128);

impl From<IpAddr> for AddrKey {
    fn from(ip: IpAddr) -> Self {
        match ip {
            IpAddr::V4(v4) => Self(u128::from(u32::from(v4))),
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => Self(u128::from(u32::from(v4))),
                None => Self((u128::from(v6) >> 64) << 64 | 1),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The address is locked until this time.
    AddressLocked { until: i64 },
    /// The username is soft-locked for this address, or the shared allowance is used up.
    UserThrottled,
}

/// A started attempt, to be passed back to [`Limiter::succeeded`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attempt {
    addr: AddrKey,
    user: String,
    at: i64,
}

#[derive(Default)]
struct AddrState {
    failures: VecDeque<i64>,
    locked_until: i64,
    locks: u32,
    last_seen: i64,
}

#[derive(Default)]
struct UserState {
    failures: VecDeque<(i64, AddrKey)>,
    shared: VecDeque<i64>,
    last_seen: i64,
}

#[derive(Default)]
pub struct Limiter {
    addrs: HashMap<AddrKey, AddrState>,
    users: HashMap<String, UserState>,
}

impl Limiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts an attempt. Counts it as a failure until [`Limiter::succeeded`] says otherwise.
    pub fn begin(&mut self, ip: IpAddr, user: &str, now: i64) -> Result<Attempt, Refusal> {
        let addr = AddrKey::from(ip);
        let user = user.to_ascii_lowercase();
        self.evict(now);
        let a = self.addrs.entry(addr).or_default();
        a.last_seen = now;
        while a.failures.front().is_some_and(|&t| t <= now - WINDOW) {
            a.failures.pop_front();
        }
        if a.locked_until > now {
            return Err(Refusal::AddressLocked { until: a.locked_until });
        }
        let u = self.users.entry(user.clone()).or_default();
        u.last_seen = now;
        while u.failures.front().is_some_and(|&(t, _)| t <= now - WINDOW) {
            u.failures.pop_front();
        }
        while u.shared.front().is_some_and(|&t| t <= now - 60) {
            u.shared.pop_front();
        }
        if u.failures.len() >= MAX_FAILURES {
            let failed_here = u.failures.iter().any(|&(_, k)| k == addr);
            if failed_here || u.shared.len() >= SHARED_PER_MINUTE {
                return Err(Refusal::UserThrottled);
            }
            u.shared.push_back(now);
        }
        u.failures.push_back((now, addr));
        let a = self.addrs.entry(addr).or_default();
        a.failures.push_back(now);
        if a.failures.len() >= MAX_FAILURES {
            let lock = FIRST_LOCK.saturating_mul(1i64 << a.locks.min(10)).min(MAX_LOCK);
            a.locked_until = now + lock;
            a.locks = a.locks.saturating_add(1);
            a.failures.clear();
        }
        Ok(Attempt { addr, user, at: now })
    }

    /// The attempt succeeded: it no longer counts as a failure.
    pub fn succeeded(&mut self, attempt: &Attempt) {
        if let Some(a) = self.addrs.get_mut(&attempt.addr)
            && let Some(i) = a.failures.iter().position(|&t| t == attempt.at)
        {
            a.failures.remove(i);
        }
        if let Some(u) = self.users.get_mut(&attempt.user)
            && let Some(i) = u.failures.iter().position(|&(t, k)| t == attempt.at && k == attempt.addr)
        {
            u.failures.remove(i);
        }
    }

    pub fn tracked(&self) -> usize {
        self.addrs.len() + self.users.len()
    }

    fn evict(&mut self, now: i64) {
        if self.addrs.len() + self.users.len() < MAX_ENTRIES {
            return;
        }
        // Drop entries with nothing active first, then the least recently seen.
        self.addrs.retain(|_, a| a.locked_until > now || a.failures.back().is_some_and(|&t| t > now - WINDOW));
        self.users.retain(|_, u| u.failures.back().is_some_and(|&(t, _)| t > now - WINDOW));
        while self.addrs.len() + self.users.len() >= MAX_ENTRIES {
            let oldest_addr = self.addrs.iter().min_by_key(|(_, a)| a.last_seen).map(|(k, a)| (*k, a.last_seen));
            let oldest_user = self.users.iter().min_by_key(|(_, u)| u.last_seen).map(|(k, u)| (k.clone(), u.last_seen));
            match (oldest_addr, oldest_user) {
                (Some((k, ta)), Some((_, tu))) if ta <= tu => {
                    self.addrs.remove(&k);
                }
                (_, Some((k, _))) => {
                    self.users.remove(&k);
                }
                (Some((k, _)), None) => {
                    self.addrs.remove(&k);
                }
                (None, None) => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("ip")
    }

    #[test]
    fn five_failures_lock_the_address_and_locks_double() {
        let mut l = Limiter::new();
        let a = ip("203.0.113.5");
        for i in 0..5 {
            assert!(l.begin(a, &format!("user{i}"), 1000 + i).is_ok());
        }
        assert_eq!(l.begin(a, "anyone", 1010), Err(Refusal::AddressLocked { until: 1004 + FIRST_LOCK }));
        let t = 1004 + FIRST_LOCK;
        for i in 0..5 {
            assert!(l.begin(a, &format!("u{i}"), t + i).is_ok());
        }
        assert_eq!(l.begin(a, "x", t + 10), Err(Refusal::AddressLocked { until: t + 4 + 2 * FIRST_LOCK }));
    }

    #[test]
    fn locks_cap_at_a_day() {
        let mut l = Limiter::new();
        let a = ip("203.0.113.6");
        let mut now = 0;
        let mut last = 0;
        for _ in 0..15 {
            for i in 0..5 {
                let _ = l.begin(a, &format!("v{i}"), now + i);
            }
            if let Err(Refusal::AddressLocked { until }) = l.begin(a, "z", now + 5) {
                last = until - (now + 4);
                now = until;
            }
        }
        assert_eq!(last, MAX_LOCK);
    }

    #[test]
    fn a_success_is_forgiven() {
        let mut l = Limiter::new();
        let a = ip("203.0.113.7");
        for i in 0..20 {
            let att = l.begin(a, "owner", 100 + i).expect("allowed");
            l.succeeded(&att);
        }
        assert!(l.begin(a, "owner", 200).is_ok());
    }

    #[test]
    fn an_attacker_cannot_lock_the_owner_out() {
        let mut l = Limiter::new();
        // An attacker spreads guesses for "owner" over many addresses.
        for i in 0..5 {
            let _ = l.begin(ip(&format!("198.51.100.{i}")), "owner", 1000);
        }
        // Attacker addresses that failed are refused for this user...
        assert_eq!(l.begin(ip("198.51.100.1"), "owner", 1001), Err(Refusal::UserThrottled));
        // ...but the owner, from a fresh address, still gets the shared allowance.
        let att = l.begin(ip("192.0.2.50"), "owner", 1002).expect("owner allowed");
        l.succeeded(&att);
    }

    #[test]
    fn the_shared_allowance_is_two_a_minute() {
        let mut l = Limiter::new();
        for i in 0..5 {
            let _ = l.begin(ip(&format!("198.51.100.{i}")), "target", 1000);
        }
        assert!(l.begin(ip("192.0.2.1"), "target", 1001).is_ok());
        assert!(l.begin(ip("192.0.2.2"), "target", 1001).is_ok());
        assert_eq!(l.begin(ip("192.0.2.3"), "target", 1002), Err(Refusal::UserThrottled));
        assert!(l.begin(ip("192.0.2.3"), "target", 1062).is_ok());
    }

    #[test]
    fn ipv6_is_grouped_by_64_and_mapped_v4_is_v4() {
        assert_eq!(AddrKey::from(ip("2001:db8:1:2::1")), AddrKey::from(ip("2001:db8:1:2:ffff::9")));
        assert_ne!(AddrKey::from(ip("2001:db8:1:2::1")), AddrKey::from(ip("2001:db8:1:3::1")));
        assert_eq!(AddrKey::from(ip("::ffff:203.0.113.9")), AddrKey::from(ip("203.0.113.9")));
        let mut l = Limiter::new();
        for i in 0..5 {
            let _ = l.begin(ip(&format!("2001:db8:1:2::{i}")), &format!("w{i}"), 0);
        }
        assert!(matches!(l.begin(ip("2001:db8:1:2::abcd"), "w9", 1), Err(Refusal::AddressLocked { .. })));
    }

    #[test]
    fn usernames_are_case_insensitive() {
        let mut l = Limiter::new();
        for i in 0..5 {
            let _ = l.begin(ip(&format!("198.51.100.{i}")), if i % 2 == 0 { "Owner" } else { "OWNER" }, 0);
        }
        assert_eq!(l.begin(ip("198.51.100.0"), "owner", 1), Err(Refusal::UserThrottled));
    }

    #[test]
    fn memory_is_bounded() {
        let mut l = Limiter::new();
        for i in 0..30_000u32 {
            let a = IpAddr::from(std::net::Ipv4Addr::from(0x0a00_0000 + i));
            let _ = l.begin(a, &format!("u{i}"), i64::from(i));
        }
        assert!(l.tracked() <= MAX_ENTRIES);
    }
}
