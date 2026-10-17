# OWASP ASVS 5.0 map

_Target (SPEC D20): Level 2 throughout, Level 3 for authentication, session management, file handling and configuration. Updated in the same change as any security-relevant code._

Status: **Built** (implemented and tested; tests named) · **Designed** (specified in SPEC; phase) · **N/A**.

| Chapter | Level | Status | Where |
|---|---|---|---|
| V1 Encoding and Sanitization | 2 | **Built** for stored metadata: `core::text` (NFC, invisible/bidi/control removal, HTML → text that never emits markup, path/LAN scrubbing); fuzz target `html_text`, `metadata`. Output encoding in the client: **Designed** (Phase 3: text-only rendering, banned-API scan, Trusted Types). | `core/src/text.rs`, `core/src/metadata.rs` |
| V2 Validation and Business Logic | 2 | **Built:** strict config (`deny_unknown_fields`), ID format check before lookup, bounded ffprobe parsing, progress-merge rules (future, age and duplicate bounds; property-tested and fuzzed). Request schemas: **Designed** (Phase 2–3). | `server/src/config.rs`, `core/src/ids.rs`, `core/src/ffprobe.rs`, `core/src/progress.rs` |
| V3 Web Frontend Security | 2 | **Designed** (Phase 3): strict CSP without unsafe-inline/eval, Trusted Types, COOP/CORP, no CORS, service-worker policy. The Phase 0 spike pages already send a strict CSP. | SPEC §7, §9.3 |
| V4 API and Web Service | 2 | **Designed** (Phase 3): one documented API, default deny, `Host` check, method allow-lists. | SPEC §6 |
| V5 File Handling | **3** | **Built:** read-only library (start-up refusal, `ro` mounts), no symlinks or cross-filesystem walking, extension and container must agree, forced demuxers, jailed processing, image re-encoding with limits, segment frame checks. Upload is N/A (no uploads, D3). | `server/src/walk.rs`, `server/src/probe.rs`, `server/src/covers.rs`, `jail/` |
| V6 Authentication | **3** | **Built** (Phase 2): invite-only, passkeys (UV required), password + TOTP (single use, constant time), argon2id, no enumeration, address and username lockout, recovery codes, step-up, last-method rule. **Open:** breached-password list; HTTP-level passkey tests. Reviewed in [review-phase2.md](review-phase2.md). | `core/src/auth/`, `server/src/auth/`, `server/src/http/` |
| V7 Session Management | **3** | **Built** (Phase 2): 256-bit opaque tokens (hashed at rest), 15 min access, refresh rotation with reuse detection (retries allowed until a replacement is used; Phase 6 A1), 90-day absolute lifetime, renewal cap, strict cookies, CSRF header plus Origin, server-side sign-out, device list and revoke, a password change, or removing or replacing a factor, signs out other devices; signing out another device needs step-up; a recovery-code sign-in is limited to the account until a new factor is set up. Reviewed in [review-phase2.md](review-phase2.md). | `server/src/auth/`, `server/src/http/` |
| V8 Authorization | 2 | **Partly built:** own-devices-only (404 otherwise), step-up for account changes. **Designed:** per-library access, admin routes, last-admin rule (Phases 3 and 5). | `server/src/http/me_routes.rs` |
| V9 Self-contained Tokens | – | **N/A**: no JWTs; opaque tokens only (D15). | |
| V10 OAuth and OIDC | – | **N/A** in v1. | |
| V11 Cryptography | 2 | **Built:** CSPRNG secret (`getrandom`), HKDF-SHA256 per purpose, HMAC-SHA256 IDs (RFC 4648 base32 tested). **Designed:** argon2id, XChaCha20-Poly1305 for TOTP seeds, constant-time comparisons. | `server/src/secret.rs`, `core/src/ids.rs` |
| V12 Secure Communication | 2 | Deployment **Built** (TLS at the proxy, HSTS, LAN allow-list); server-side TLS checks **Designed** (Phase 3). | SPEC §5.1 |
| V13 Configuration | **3** | **Built:** config refused if group- or world-writable or with unknown keys; data dir must be 0700 and ours; secret 0600; no outbound network; hardened container (non-root, read-only rootfs, no capabilities, no-new-privileges); cargo-deny in CI; CI actions pinned by SHA. **Designed:** systemd unit with `systemd-analyze security` ≤ 1.5, base images pinned by digest, image scanning. | `server/src/config.rs`, `deploy/Dockerfile`, `.github/workflows/ci.yml`, `deny.toml` |
| V14 Data Protection | 2 | **Designed:** history retention and controls, `Clear-Site-Data`, per-user offline storage (spiked in Phase 0a). | SPEC §5.6, §8 |
| V15 Secure Coding and Architecture | 2 | **Built:** `#![forbid(unsafe_code)]` in every crate of ours; pure `core` with property tests and nine fuzz targets; the jail as a separate, small, reviewable binary; threat model maintained. | `Cargo.toml` lints, `fuzz/`, this directory |
| V16 Security Logging and Error Handling | 2 | **Partly built:** problems recorded per file for admins; secrets redacted in `Debug`. **Designed:** hash-chained audit log, generic client errors with correlation IDs. | `server/src/catalogue.rs`, SPEC §5.5 |
| V17 WebRTC | – | **N/A**. | |
