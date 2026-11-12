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
/// Addresses and usernames tracked. Large enough that filling it with live entries takes
/// tens of thousands of networks failing at once (it was 10,000, which about 1,700 could
/// fill: 2026-11-12 review, M7); each entry is a few dozen bytes.
pub const MAX_ENTRIES: usize = 100_000;

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
    /// The table of throttled addresses and names is full of active entries: refused rather
    /// than forgetting a lock.
    Busy,
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
    /// When the table was last swept for room: a full table is swept once a second, not on
    /// every attempt, so keeping it full can't also be made to cost a pass over it each time.
    swept: Option<i64>,
}

impl Limiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts an attempt. Counts it as a failure until [`Limiter::succeeded`] says otherwise.
    pub fn begin(&mut self, ip: IpAddr, user: &str, now: i64) -> Result<Attempt, Refusal> {
        self.begin_with(ip, user, now, false)
    }

    /// As [`Limiter::begin`], for a device that already holds a valid sign-in for this user:
    /// it isn't held by the shared allowance that strangers can use up (review A2). Address
    /// locks still apply.
    pub fn begin_known_device(&mut self, ip: IpAddr, user: &str, now: i64) -> Result<Attempt, Refusal> {
        self.begin_with(ip, user, now, true)
    }

    fn begin_with(&mut self, ip: IpAddr, user: &str, now: i64, known: bool) -> Result<Attempt, Refusal> {
        let addr = AddrKey::from(ip);
        // Usernames are at most 32 characters; a longer key is never a real account.
        let user: String = user.to_ascii_lowercase().chars().take(64).collect();
        if !self.make_room(addr, &user, now, known) {
            return Err(Refusal::Busy);
        }
        let a = self.addrs.entry(addr).or_default();
        a.last_seen = now;
        // A day without a lock in force and the address starts again from the first, short
        // lock: an address many people share (a mobile network, an office) doesn't keep
        // 24-hour locks for ever because of old abuse from it.
        if a.locks > 0 && a.locked_until + MAX_LOCK <= now {
            a.locks = 0;
        }
        while a.failures.front().is_some_and(|&t| t <= now - WINDOW) {
            a.failures.pop_front();
        }
        // Five failures lock the address from the fifth; the lock is decided when the next
        // attempt starts, so a correct fifth password (taken back by `succeeded`) locks
        // nobody out (review C7).
        if a.failures.len() >= MAX_FAILURES {
            let last = a.failures.back().copied().unwrap_or(now);
            let lock = FIRST_LOCK.saturating_mul(1i64 << a.locks.min(10)).min(MAX_LOCK);
            a.locked_until = last + lock;
            a.locks = a.locks.saturating_add(1);
            a.failures.clear();
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
        if u.failures.len() >= MAX_FAILURES && !known {
            let failed_here = u.failures.iter().any(|&(_, k)| k == addr);
            if failed_here || u.shared.len() >= SHARED_PER_MINUTE {
                return Err(Refusal::UserThrottled);
            }
            u.shared.push_back(now);
        }
        u.failures.push_back((now, addr));
        let a = self.addrs.entry(addr).or_default();
        a.failures.push_back(now);
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

    /// Whether a lock or soft lock is in force for an entry (never evicted).
    fn addr_active(a: &AddrState, now: i64) -> bool {
        a.locked_until > now || a.failures.back().is_some_and(|&t| t > now - WINDOW)
    }

    fn user_active(u: &UserState, now: i64) -> bool {
        u.failures.back().is_some_and(|&(t, _)| t > now - WINDOW)
    }

    /// Keeps the table within [`MAX_ENTRIES`]. Entries with nothing in force go first; an
    /// entry holding a lock or recent failures is never dropped (dropping it would hand out
    /// fresh attempts, review A2). If the table is full of those, a new address or username
    /// is refused instead: fail closed. Returns whether this attempt may go ahead.
    ///
    /// A device that already holds a sign-in for the account is the exception: for it, the
    /// entries seen longest ago that hold no lock give way. Only the account's own devices
    /// can cause that, so a table kept full from outside doesn't keep them out, and what an
    /// evicted address gains is its five tries back.
    fn make_room(&mut self, addr: AddrKey, user: &str, now: i64, known: bool) -> bool {
        self.make_room_within(addr, user, now, known, MAX_ENTRIES)
    }

    fn make_room_within(&mut self, addr: AddrKey, user: &str, now: i64, known: bool, max: usize) -> bool {
        let need = |me: &Self| {
            let new = usize::from(!me.addrs.contains_key(&addr)) + usize::from(!me.users.contains_key(user));
            (me.addrs.len() + me.users.len() + new).saturating_sub(max)
        };
        if need(self) == 0 {
            return true;
        }
        if self.swept != Some(now) {
            self.swept = Some(now);
            self.addrs.retain(|_, a| Self::addr_active(a, now));
            self.users.retain(|_, u| Self::user_active(u, now));
        }
        if need(self) == 0 || !known {
            return need(self) == 0;
        }
        for _ in 0..need(self) {
            let oldest = self.addrs.iter().filter(|(_, a)| a.locked_until <= now).min_by_key(|(_, a)| a.last_seen);
            let Some((&k, _)) = oldest else { break };
            self.addrs.remove(&k);
        }
        need(self) == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review C7: four typos then the right password must not lock the address.
    #[test]
    fn a_correct_fifth_attempt_does_not_lock_the_address() {
        let mut l = Limiter::new();
        let a = ip("203.0.113.40");
        for i in 0..4 {
            assert!(l.begin(a, "ann", 1000 + i).is_ok());
        }
        let ok = l.begin(a, "ann", 1004).expect("fifth attempt");
        l.succeeded(&ok);
        assert!(l.begin(a, "bob", 1005).is_ok(), "the address isn't locked after a success");
    }

    /// Review A2: strangers using up the shared allowance can't keep the owner's own device
    /// out of password sign-in.
    #[test]
    fn a_known_device_is_not_held_by_the_shared_allowance() {
        let mut l = Limiter::new();
        for i in 0..5u8 {
            let _ = l.begin(ip(&format!("2001:db8:{i}::1")), "ann", 1000);
        }
        for i in 10..12u8 {
            let _ = l.begin(ip(&format!("2001:db8:{i}::1")), "ann", 1001);
        }
        let owner = ip("198.51.100.7");
        assert_eq!(l.begin(owner, "ann", 1002).err(), Some(Refusal::UserThrottled), "a stranger's view");
        assert!(l.begin_known_device(owner, "ann", 1002).is_ok(), "the owner's device still gets its attempt");
    }

    /// Review A2: flooding the table with throwaway entries can't evict a username's soft
    /// lock (which would hand out fresh guesses).
    #[test]
    fn a_flood_cannot_evict_a_soft_locked_username() {
        let mut l = Limiter::new();
        for i in 0..5u16 {
            let _ = l.begin(ip(&format!("2001:db8:ffff:{i:x}::1")), "victim", 1000);
        }
        for i in 0..(MAX_ENTRIES as u32) {
            let a = std::net::Ipv6Addr::from((0x2001_0db8_u128 << 96) | (u128::from(i) << 64) | 1);
            let _ = l.begin(IpAddr::V6(a), &format!("junk{i}"), 1001);
        }
        assert!(l.tracked() <= MAX_ENTRIES + 2);
        let fresh = ip("192.0.2.200");
        let r1 = l.begin(fresh, "victim", 1002);
        let r2 = l.begin(ip("192.0.2.201"), "victim", 1002);
        let r3 = l.begin(ip("192.0.2.202"), "victim", 1002);
        assert!([r1, r2, r3].iter().any(|r| r.is_err()), "the victim is still soft-locked");
    }

    #[test]
    fn usernames_are_stored_bounded() {
        let mut l = Limiter::new();
        let _ = l.begin(ip("192.0.2.9"), &"x".repeat(16_000), 1);
        assert!(l.users.keys().all(|k| k.len() <= 64));
    }

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
        for i in 0..(MAX_ENTRIES as u32) {
            let a = IpAddr::from(std::net::Ipv4Addr::from(0x0a00_0000 + i));
            let _ = l.begin(a, &format!("u{i}"), i64::from(i / 1000));
        }
        assert!(l.tracked() <= MAX_ENTRIES);
    }

    fn fill(l: &mut Limiter, now: i64, max: usize) {
        // Live entries only: each address has just failed for its own username.
        let mut i = 0u32;
        while l.tracked() + 2 <= max {
            let a = IpAddr::V4(std::net::Ipv4Addr::from(0x0a00_0000 + i));
            assert!(l.begin(a, &format!("user{i}"), now).is_ok());
            i += 1;
        }
    }

    /// 2026-11-12 review, M7: a table kept full of live entries refused every new address,
    /// the account's own devices included.
    #[test]
    fn a_full_table_refuses_strangers_but_not_a_device_already_signed_in() {
        let mut l = Limiter::new();
        fill(&mut l, 1000, 40);
        let owner = ip("198.51.100.7");
        let k = AddrKey::from(owner);
        assert!(!l.make_room_within(k, "ann", 1001, false, 40), "a stranger: fail closed, nothing forgotten");
        assert_eq!(l.tracked(), 40);
        assert!(l.make_room_within(k, "ann", 1001, true, 40), "the owner's device gets room");
        assert_eq!(l.tracked(), 38, "only as many entries as it needs gave way");
    }

    #[test]
    fn room_for_a_known_device_never_costs_a_lock() {
        let mut l = Limiter::new();
        let locked = ip("203.0.113.9");
        for i in 0..6 {
            let _ = l.begin(locked, "zed", 1000 + i);
        }
        assert!(matches!(l.begin(locked, "zed", 1010), Err(Refusal::AddressLocked { .. })));
        fill(&mut l, 1011, 40);
        for n in 0..30u8 {
            let owner = AddrKey::from(ip(&format!("198.51.100.{n}")));
            let _ = l.make_room_within(owner, &format!("owner{n}"), 1012, true, 40);
        }
        assert!(matches!(l.begin(locked, "zed", 1013), Err(Refusal::AddressLocked { .. })), "the lock was forgotten");
    }

    #[test]
    fn an_address_starts_again_from_the_short_lock_after_a_quiet_day() {
        let mut l = Limiter::new();
        let a = ip("203.0.113.50");
        let lock_at = |l: &mut Limiter, t0: i64| {
            for i in 0..5 {
                let _ = l.begin(a, "ann", t0 + i);
            }
            match l.begin(a, "ann", t0 + 5) {
                Err(Refusal::AddressLocked { until }) => until - (t0 + 4),
                other => panic!("expected a lock, got {other:?}"),
            }
        };
        assert_eq!(lock_at(&mut l, 1000), FIRST_LOCK);
        assert_eq!(lock_at(&mut l, 1000 + 2 * FIRST_LOCK), 2 * FIRST_LOCK, "repeat abuse doubles the lock");
        let later = 1000 + 5 * FIRST_LOCK + MAX_LOCK;
        assert_eq!(lock_at(&mut l, later), FIRST_LOCK, "a quiet day later it is the first lock again");
    }
}
