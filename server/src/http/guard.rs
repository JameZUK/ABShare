//! Request-level defences (SPEC §5.1, §9.3): the real client address (only via trusted
//! proxies), TLS enforcement, the `Host` check, CSRF protection for cookie routes, security
//! headers on every response, and per-address rate limits for unauthenticated routes.

use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;

/// What the guard learned about a request, attached as an extension.
#[derive(Clone, Copy, Debug)]
pub struct Client {
    pub ip: IpAddr,
}

/// The peer's socket address, set by the serve loop.
#[derive(Clone, Copy, Debug)]
pub struct Peer(pub SocketAddr);

pub struct GuardConfig {
    pub trusted: Vec<IpAddr>,
    /// The exact `Host` value expected, e.g. `books.example.com`.
    pub host: String,
    /// The exact `Origin` expected on cookie routes, e.g. `https://books.example.com`.
    pub origin: String,
}

/// An IPv4 address seen through an IPv6 socket (`::ffff:a.b.c.d`) as plain IPv4, so it
/// matches `trusted_proxies` and rate-limit keys either way.
pub fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

/// The client address: the peer itself, or, when the peer is a trusted proxy, the rightmost
/// `X-Forwarded-For` entry that isn't itself a trusted proxy.
pub fn client_ip(peer: IpAddr, headers: &HeaderMap, trusted: &[IpAddr]) -> IpAddr {
    let peer = canonical(peer);
    if !trusted.contains(&peer) {
        return peer;
    }
    let mut hops: Vec<IpAddr> = Vec::new();
    for v in headers.get_all("x-forwarded-for") {
        let Ok(s) = v.to_str() else { return peer };
        for part in s.split(',') {
            match part.trim().parse::<IpAddr>() {
                Ok(ip) => hops.push(canonical(ip)),
                Err(_) => return peer,
            }
        }
    }
    hops.into_iter().rev().find(|ip| !trusted.contains(ip)).unwrap_or(peer)
}

/// Whether the request reached the proxy over TLS: only a trusted proxy's word counts;
/// loopback is accepted as-is (local tools and tests).
pub fn is_tls(peer: IpAddr, headers: &HeaderMap, trusted: &[IpAddr]) -> bool {
    let peer = canonical(peer);
    if trusted.contains(&peer) {
        return headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok()) == Some("https");
    }
    peer.is_loopback()
}

fn error(status: StatusCode, code: &str, message: &str) -> Response {
    let body = serde_json::json!({ "error": code, "message": message }).to_string();
    (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// Applied to every request: client address, TLS and `Host`.
pub async fn front(cfg: axum::extract::State<std::sync::Arc<GuardConfig>>, mut req: Request, next: Next) -> Response {
    let peer = req.extensions().get::<Peer>().map(|p| p.0.ip()).unwrap_or(IpAddr::from([127, 0, 0, 1]));
    let headers = req.headers();
    if !is_tls(peer, headers, &cfg.trusted) {
        return headers_on(error(StatusCode::FORBIDDEN, "tls_required", "TLS required"));
    }
    let host_ok =
        headers.get(header::HOST).and_then(|h| h.to_str().ok()).is_some_and(|h| h.eq_ignore_ascii_case(&cfg.host));
    if !host_ok {
        return headers_on(error(StatusCode::MISDIRECTED_REQUEST, "wrong_host", "Unknown host"));
    }
    let ip = client_ip(peer, headers, &cfg.trusted);
    req.extensions_mut().insert(Client { ip });
    headers_on(next.run(req).await)
}

/// Applied to routes that read or set cookies: a custom header (which a cross-site request
/// can't send without a preflight that is never approved) and an exact `Origin`.
pub async fn csrf(cfg: axum::extract::State<std::sync::Arc<GuardConfig>>, req: Request, next: Next) -> Response {
    let h = req.headers();
    let marked = h.get("x-abshare-client").and_then(|v| v.to_str().ok()).is_some_and(|v| v == "1");
    let origin_ok = h.get(header::ORIGIN).and_then(|v| v.to_str().ok()).is_some_and(|o| o == cfg.origin);
    if !marked || !origin_ok {
        return error(StatusCode::FORBIDDEN, "csrf", "Request refused");
    }
    next.run(req).await
}

/// Security headers on every response, refusals included (SPEC §9.3).
pub fn headers_on(mut r: Response) -> Response {
    let h = r.headers_mut();
    let set = |h: &mut HeaderMap, k: &'static str, v: &'static str| {
        h.insert(k, HeaderValue::from_static(v));
    };
    set(
        h,
        "content-security-policy",
        "default-src 'none'; script-src 'self'; worker-src 'self'; manifest-src 'self'; connect-src 'self'; img-src 'self' blob: data:; media-src 'self' blob:; style-src 'self'; style-src-attr 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'; require-trusted-types-for 'script'; trusted-types abshare-sw svelte-trusted-html",
    );
    set(
        h,
        "permissions-policy",
        "accelerometer=(), camera=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), payment=(), usb=(), autoplay=(self), publickey-credentials-get=(self), publickey-credentials-create=(self)",
    );
    set(h, "x-content-type-options", "nosniff");
    set(h, "referrer-policy", "no-referrer");
    set(h, "cross-origin-opener-policy", "same-origin");
    set(h, "cross-origin-resource-policy", "same-origin");
    set(h, "x-frame-options", "DENY");
    if !h.contains_key(header::CACHE_CONTROL) {
        set(h, "cache-control", "no-store");
    }
    r
}

/// A sliding-window counter per key (an address, or a device and book), bounded in memory.
pub struct RateLimit<K = IpAddr> {
    per_minute: usize,
    hits: Mutex<HashMap<K, VecDeque<i64>>>,
}

impl<K: std::hash::Hash + Eq> RateLimit<K> {
    pub fn new(per_minute: usize) -> Self {
        Self { per_minute, hits: Mutex::new(HashMap::new()) }
    }

    pub fn allow(&self, ip: K, now: i64) -> bool {
        let Ok(mut m) = self.hits.lock() else { return false };
        if m.len() > 10_000 {
            m.retain(|_, q| q.back().is_some_and(|&t| t > now - 60));
        }
        let q = m.entry(ip).or_default();
        while q.front().is_some_and(|&t| t <= now - 60) {
            q.pop_front();
        }
        if q.len() >= self.per_minute {
            return false;
        }
        q.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("ip")
    }

    fn xff(v: &[&str]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for x in v {
            h.append("x-forwarded-for", HeaderValue::from_str(x).expect("header"));
        }
        h
    }

    #[test]
    fn an_untrusted_peer_cannot_claim_another_address() {
        let t = [ip("10.0.0.10")];
        assert_eq!(client_ip(ip("198.51.100.9"), &xff(&["192.0.2.1"]), &t), ip("198.51.100.9"));
    }

    #[test]
    fn the_rightmost_untrusted_hop_is_the_client() {
        let t = [ip("10.0.0.10"), ip("10.0.0.11")];
        // The client prepends a lie; the proxy appends the real address.
        assert_eq!(client_ip(ip("10.0.0.10"), &xff(&["1.2.3.4, 203.0.113.7"]), &t), ip("203.0.113.7"));
        assert_eq!(client_ip(ip("10.0.0.10"), &xff(&["203.0.113.7, 10.0.0.11"]), &t), ip("203.0.113.7"));
        assert_eq!(client_ip(ip("10.0.0.10"), &xff(&["1.2.3.4", "203.0.113.8"]), &t), ip("203.0.113.8"));
        // Garbage makes the proxy itself the client (never trusted blindly).
        assert_eq!(client_ip(ip("10.0.0.10"), &xff(&["203.0.113.7, not-an-ip"]), &t), ip("10.0.0.10"));
        assert_eq!(client_ip(ip("10.0.0.10"), &HeaderMap::new(), &t), ip("10.0.0.10"));
    }

    #[test]
    fn tls_is_only_believed_from_a_trusted_proxy() {
        let t = [ip("10.0.0.10")];
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        assert!(is_tls(ip("10.0.0.10"), &h, &t));
        assert!(!is_tls(ip("198.51.100.9"), &h, &t), "a stranger's header means nothing");
        assert!(!is_tls(ip("10.0.0.10"), &HeaderMap::new(), &t));
        assert!(is_tls(ip("127.0.0.1"), &HeaderMap::new(), &t));
        assert!(is_tls(ip("::1"), &HeaderMap::new(), &t));
    }

    #[test]
    fn rate_limits_are_per_address_and_slide() {
        let r = RateLimit::new(2);
        assert!(r.allow(ip("192.0.2.1"), 0));
        assert!(r.allow(ip("192.0.2.1"), 1));
        assert!(!r.allow(ip("192.0.2.1"), 2));
        assert!(r.allow(ip("192.0.2.2"), 2));
        assert!(r.allow(ip("192.0.2.1"), 61));
    }
}
