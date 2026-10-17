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

The Phase 6 security review is done (SPEC §11): five focused code reviews and a black-box
penetration test of the release image behind nginx with TLS. It found 4 high, 11 medium and
about 25 low-severity issues. All are fixed, each with a regression test that fails without
the fix (one low finding was a firewall change on the test network, now made). No critical, high or medium
findings are open. The findings and fixes are in
[docs/security/review-phase6.md](docs/security/review-phase6.md).

That review was AI-assisted. **An independent human review is still recommended**, and
reports from anyone are welcome (see above).
