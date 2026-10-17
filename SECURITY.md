# Security

ABShare holds other people's accounts and is built to face the internet, so security is its
first requirement.

## Reporting a vulnerability

Please report vulnerabilities privately, through GitHub's "Report a vulnerability" (Security
tab of this repository), not in public issues. Include what you found, how to reproduce it,
and the version or commit. You'll get an acknowledgement, and credit in the fix if you want it.

## The security model, in short

- **Invite-only.** No self sign-up. Invite links are single-use, expire, and are stored
  hashed.
- **Strong sign-in.** Passkeys (WebAuthn), or a password (argon2id; at least 12 characters;
  common leaked passwords refused) with a mandatory authenticator code. Single-use recovery
  codes. Lockouts per address and per account.
- **Sessions.** Short-lived access tokens in memory; rotating refresh tokens in an HttpOnly,
  Secure, SameSite=Strict cookie with reuse detection (a stolen token signs the device out);
  remote sign-out per device. Account changes need a fresh confirmation (step-up).
- **The web app** has a strict Content Security Policy with Trusted Types, no inline script,
  no third-party anything, and no raw HTML from metadata: everything from audiobook files is
  shown as text.
- **The library is read-only**, and no client-supplied path ever reaches the filesystem:
  books are addressed by opaque IDs, and files are opened inside their library root without
  following symlinks.
- **ffmpeg is sandboxed.** Every ffmpeg and ffprobe run goes through `abshare-jail`
  (Landlock filesystem rules, a seccomp filter, resource limits, no inherited descriptors).
- **No outbound network.** The server never calls out.
- **Audit log.** Security events are recorded in an HMAC-chained log; edits or removed rows
  are detected.
- **Testing.** Every route has negative tests (missing, forged, expired and revoked tokens,
  other users' data, CSRF, Host, spoofed forwarding headers, oversize bodies, rate limits),
  parsers of untrusted bytes are fuzzed, and the jail has escape tests.

Details: [SPEC.md](SPEC.md) §9, [docs/security/threat-model.md](docs/security/threat-model.md)
and [docs/security/asvs.md](docs/security/asvs.md).

## Status

ABShare has not yet had its independent review and penetration test (SPEC §11, Phase 6).
Until then, deploy it for people you know, behind a reverse proxy.
