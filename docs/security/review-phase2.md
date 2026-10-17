# Phase 2 security self-review: authentication and sessions

_2026-10-17 · OWASP ASVS 5.0, chapters V6 (Authentication) and V7 (Session Management) at Level 3, plus related V2, V11, V13 and V16 items · the gate for Phase 2 in SPEC §11_

This review reads the Phase 2 code against ASVS and records what holds, how it is tested, and what is still open. It is a self-review; an external review comes before public exposure (D22).

## What was built

| Area | Code |
|---|---|
| Pure rules: TOTP, sign-in throttling, validation, token and recovery-code formats | `core/src/auth/` |
| Storage: users, factors, invites, devices, refresh tokens, audit | `server/src/auth/store.rs`, `server/src/db.rs` (migration 3) |
| Password hashing, tokens, TOTP-seed encryption | `server/src/auth/crypto.rs` |
| In-memory sessions and ceremonies | `server/src/auth/sessions.rs` |
| Audit chain | `server/src/auth/audit.rs` |
| HTTP guard and API | `server/src/http/` |
| Admin CLI | `server/src/admin.rs` |

## V6 Authentication

| Requirement (summarised) | Status | Evidence |
|---|---|---|
| No self-registration; accounts only by invitation | Holds | Invites are single use, expire (72 h from the CLI), stored hashed, and consumed in the transaction that creates the account. Tests: `invites_must_be_real_unused_and_are_rate_limited`, `enrolment_…_safe_cookies` (the invite is unusable afterwards); store test `invites_are_single_use_and_expire`. |
| Passwords: at least 12 characters, up to 1024, any characters allowed, no composition rules | Holds | `validate::password`. Counts characters, not bytes. Rejects passwords containing the username or with fewer than 4 distinct characters. |
| Check against breached passwords | **Open** | The breached-password list (SPEC §5.5) is not bundled yet. Tracked for Phase 3. |
| Password storage: memory-hard KDF with a per-user salt | Holds | argon2id, m = 64 MiB, t = 3, p = 1, 16-byte random salt, parameters in the PHC string. Test `production_parameters_are_in_the_phc_string`. |
| No user enumeration through responses or timing | Holds | Unknown users are verified against a real dummy hash; responses are identical. Test `wrong_passwords_and_unknown_users_get_the_same_answer`. Passkey start with an unknown username falls back to the discoverable flow rather than saying "no passkeys". |
| Resistance to online guessing | Holds | Per-address lock (5 in 15 min, from 15 min doubling to 24 h) and per-username soft lock that strangers can't trigger against the owner. Attempts count before hashing. Tests: core `an_attacker_cannot_lock_the_owner_out`, `five_failures_lock_the_address_and_locks_double`, `memory_is_bounded`; HTTP `repeated_failures_lock_the_address`, `a_forged_forwarded_for_cannot_dodge_the_lockout`. |
| Multi-factor authentication required (L3) | Holds | A password always needs TOTP or a recovery code. Passkeys require user verification (biometric or PIN), so a passkey alone is MFA. Enrolment can't finish without a complete method. Tests: `enrolment_…` (finish refused without factors), `password_then_totp_…`. |
| TOTP: RFC 6238, single use, limited window, constant-time | Holds | ±1 step; a step at or before the last used is refused, at both layers (pure check, and an SQL guard against two racing requests); every candidate step compared with `subtle`. Tests: RFC vectors, `a_code_cannot_be_used_twice_or_after_a_later_one`, HTTP replay test. Mutation-checked. |
| Lookup secrets (recovery codes): single use, hashed, enough entropy | Holds | 10 codes, 60 bits each, Crockford base32, stored as SHA-256, used once. Tests: `recovery_codes_work_once` (HTTP), store test. Mutation-checked. |
| Phishing-resistant authenticator available (L3) | Holds | Passkeys via `webauthn-rs`, origin and RP ID bound, user verification required. Flow verified on the real origin in Phase 0(e). There are no HTTP-level passkey tests (they need an authenticator emulator); this is open, tracked below. |
| Factor changes require re-authentication | Holds | Every `/v1/me` change needs step-up within 10 minutes. Password alone is not a step-up when TOTP exists. Test `account_changes_need_step_up_and_sign_out_other_devices`. Mutation-checked. |
| Can't remove the last factor | Holds | `still_has_method`. Test `the_last_sign_in_method_cannot_be_removed`. Mutation-checked. |
| Account recovery doesn't weaken authentication | Holds | No email or knowledge-based recovery. An admin issues a reset link (CLI, audited) that removes all old factors and signs out every device when used. |
| TOTP seeds protected at rest | Holds | XChaCha20-Poly1305 with a key derived from the server secret, bound to the user ID. Test `seeds_are_bound_to_their_user_and_tamper_evident`. |

## V7 Session management

| Requirement (summarised) | Status | Evidence |
|---|---|---|
| Session tokens: at least 128 bits of entropy, from a CSPRNG | Holds | 256-bit `getrandom` tokens, hex-encoded; stored only as SHA-256. |
| No tokens in URLs | Holds | Bearer header for the API; HttpOnly cookies for refresh and media; invite tokens travel in the URL fragment (never sent to the server) and then in POST bodies. |
| Cookie attributes | Holds | `HttpOnly; Secure; SameSite=Strict`, narrow paths (`/v1/auth`, `/v1/media`). Tested. |
| Short-lived access, rotation, reuse detection | Holds | Access tokens last 15 min (memory only). Refresh tokens rotate on every use, with a 60 s grace (D21); reuse outside it signs the device out. *(Replaced in the Phase 6 review, finding A1: see D15.)* Tests: store `refresh_rotates_and_reuse…`, HTTP `refresh_rotates_and_a_stolen_old_token_signs_the_device_out`. Mutation-checked. |
| Absolute session lifetime (L3) | Holds | 90 days from sign-in, however often refreshed. Store test `unknown_expired_and_absolute_lifetime`. |
| Sign-out terminates the session server-side | Holds | Device revoked in the database; access and media tokens dropped from memory; `Clear-Site-Data: "cache"`. Test `logout_revokes_everything_and_clears_the_cache`. Mutation-checked. |
| Users can see and end their sessions | Holds | `/v1/me/devices` and revoke; another user's device is 404. Test `nobody_can_revoke_another_users_device`. Mutation-checked. |
| Changing credentials ends other sessions | Holds | A password change signs out every other device; a factor reset signs out all of them. Tested. |
| CSRF on cookie-authenticated requests | Holds | Custom header plus exact `Origin` on every `/v1/auth` route; no CORS. Test `cookie_routes_need_the_csrf_header_and_exact_origin` (five variants). Mutation-checked. |
| Renewal rate limit | Holds | At most 30 refreshes a minute per device. Store test `renewals_are_rate_limited`. |

## Related items

- **V2 / V4:** strict JSON (`deny_unknown_fields`), body limits (16 KiB on auth routes, 64 KiB overall), method allow-lists, and one routing table. Tests `unknown_fields_and_oversize_bodies_are_refused`, `unknown_routes_are_404…`.
- **V12 / V13:** non-TLS refused unless loopback; `X-Forwarded-*` believed only from trusted proxies, read from the right; `Host` must match. Tests `plain_http_from_outside…`, `a_wrong_host_is_refused`, guard unit tests. Mutation-checked.
- **V16:** hash-chained audit of sign-ins, failures, enrolment, factor changes, revocations and theft detection; chain verification in tests and `abshare admin verify-audit`. Internal errors return a reference ID only.

## Mutation check

`tools/mutation-check.py` removes 16 protections one at a time (TLS, `Host`, CSRF, rightmost X-Forwarded-For, security headers, TOTP replay at both layers, refresh reuse, step-up, address lockout, last method, device ownership, single-use invites, logout revocation, enrolment factors, single-use recovery codes) and requires a failing test for each. The result is recorded in the commit that adds this review.

## Open items (tracked)

1. ~~**Breached-password list** not bundled yet (V6).~~ **Done (2026-10-17):** 46,146 common leaked passwords of 12+ characters, stored as SHA-256 prefixes (`core/data/`), refused at enrolment and password change.
2. ~~**Passkey HTTP tests**~~ **Done (2026-10-17):** a software authenticator in the test suite (P-256, "none" attestation) drives enrolment and sign-in end to end, and proves forged signatures, phishing origins, replays, cross-challenge assertions, unknown keys and other-site registrations are refused.
3. ~~**Admin web routes**~~ **Done (2026-10-17):** `/v1/admin/*` (admin role re-checked against the database on every request, step-up for every change, admins can't disable or demote themselves, inviting an admin needs a second confirmation) with negative tests, and the admin screens.
4. ~~**CLI operations don't reach a running server's memory.**~~ **Done (2026-10-17):** the server applies devices revoked and users disabled from the CLI within 30 seconds.
5. **The listener-optional 2FA setting** (D2) isn't implemented; 2FA is always required. This is stricter than the spec, so it's not a weakness.
