# Threat model

_Living document · updated with every new surface, decision or dependency (SPEC §9.4)_

## What ABShare protects

| Asset | Why it matters |
|---|---|
| Listener accounts and sessions | Access to the library; an attacker could also mark progress, delete bookmarks, and so on. |
| The audiobook library | Must never be modified, deleted or exfiltrated beyond what a listener may stream (D3). |
| The host | ABShare parses hostile media and runs ffmpeg. A compromise must not spread. |
| Listening history and progress | Personal data about what and when people listen. |
| The server secret | Derives book IDs, TOTP-seed encryption and ticket MACs. |

## Trust boundaries

```
 Internet ──TLS──► reverse proxy ──HTTP (trusted hop)──► abshare ──fork/exec──► abshare-jail ──► ffmpeg/ffprobe
                                                            │                     (Landlock, seccomp, rlimits)
                                                            ├── SQLite (data dir, 0700)
                                                            └── library (read-only mount)
```

1. **Internet → proxy:** TLS, HSTS, LAN allow-list until the Phase 6 review (D22).
2. **Proxy → abshare:** only the configured trusted proxy's `X-Forwarded-*` headers are believed; `Host` must match `public_origin`.
3. **abshare → ffmpeg:** the strongest boundary. Everything ffmpeg reads is hostile. It runs jailed, with no network, read access only to its inputs and write access only to its job directory.
4. **Library files → abshare:** tags, chapter names, sidecar text and images are untrusted. They are parsed in `core` (pure, fuzzed) or re-encoded (covers).
5. **Browser → abshare:** every request is untrusted: strict schemas, IDs validated before lookup, default deny.

## Threats and mitigations

Status: **Built** = implemented and tested; **Designed** = specified, not built yet (phase in brackets).

| # | Threat | Mitigation | Status |
|---|---|---|---|
| T1 | Crafted media file exploits ffmpeg/ffprobe | `abshare-jail`: Landlock ABI ≥ 6 (read inputs only, write job directory only, execute only the program, no TCP, signal and abstract-socket scoping), seccomp deny-list (non-Unix sockets, namespaces, ptrace, mount, io_uring, BPF, memfd/execveat, module loading), no-new-privs, rlimits, inherited descriptors refused. Forced demuxer by extension, `-protocol_whitelist file`, `-enable_drefs 0`. 14 escape tests, each protection mutation-checked. | **Built** (`jail/`) |
| T2 | A file disguised as audio (HLS playlist, concat list) makes ffmpeg read other files or the network | The demuxer is forced from the extension; the probed container must match the extension; the protocol whitelist is `file`; and the jail confines anyway. Demo library includes a disguised `.m4b`, which is refused. | **Built** |
| T3 | Path traversal or reading arbitrary files | The walk never follows symlinks, never crosses filesystems, and skips hidden and non-UTF-8 entries. Clients never supply paths: books are addressed by HMAC IDs with a strict format check. | **Built** (walk, IDs); request side **Designed** (Phase 3) |
| T4 | XSS or terminal injection through tags | NFC; invisible, bidi and control characters removed; HTML converted to text (never emits markup, fuzzed); paths and LAN addresses removed from free text. Clients render text only; strict CSP; Trusted Types. | **Built** (server side, fuzzed); client **Designed** (Phase 3) |
| T5 | Malicious cover image exploits a decoder | Only JPEG/PNG; decoded by the Rust `image` crate with dimension (10 000 px) and allocation (512 MiB) limits; re-encoded; never served raw. Extraction is jailed. | **Built** |
| T6 | ffmpeg reports success but produced garbage | Trial decode at the start and middle; any stderr output marks the file unplayable (seen with xHE-AAC). Data-saver segments are checked frame by frame before caching. | **Built** |
| T7 | Library modified or deleted through ABShare | Library mounted read-only at every layer (NFS `ro`, LXC bind `ro=1`, Docker `:ro`); start-up refuses a writable root; files are opened read-only. | **Built** |
| T8 | Resource exhaustion through huge or odd libraries | Walk depth (8) and file (200 000) caps; ffprobe output capped at 4 MiB; tag, chapter and people caps; jail rlimits and wall-clock timeouts; job concurrency caps; read-ahead cap and latest-seek-wins for data saver. | **Built** (scan, spike); request limits **Designed** |
| T9 | Credential stuffing, phishing, stolen devices, CSRF | Invite-only; passkeys preferred; password + TOTP; lockout; opaque rotating tokens; media cookie; step-up; audit log. | **Designed** (Phase 2); passkeys spiked |
| T10 | Stolen database backup | Hashes only for tokens and passwords; TOTP seeds encrypted with a key from the separately stored secret; the secret file is 0600 and refused if readable. | Secret **Built**; the rest **Designed** (Phase 2) |
| T11 | Supply-chain compromise | cargo-deny (licences, advisories, bans, sources) in CI; CI actions pinned by SHA; no runtime JS dependencies; minimal crates. Base images pinned by digest at release. | **Built** (CI); digests **Designed** |
| T12 | Proxy misconfiguration (spoofed client address, double CSP, plain HTTP) | Trusted-proxy list; `X-Forwarded-For` read from the right; `403 TLS required`; on the deployment, Cloudflare real-IP rewriting is off for this site and the site has a LAN allow-list. | Deployment **Built**; server side **Designed** (Phase 3) |
| T13 | Information leaks in errors and logs | Paths only in admin diagnostics; redacted `Debug` for secrets (tested for the server secret). | Partly **Built** |

## Residual risks and assumptions

- **The kernel.** Landlock and seccomp depend on the host kernel. A kernel exploit from inside ffmpeg would bypass the jail. Mitigated by keeping the host patched and by the container and LXC layers outside the jail.
- **OpenSSL in the passkey path** (D25): C code, used only for signature verification on data already parsed in Rust.
- **The seccomp filter is a deny-list** until the bundled minimal ffmpeg is fixed and an allow-list can be tuned to it.
- **Debian's ffmpeg** has a large attack surface (every demuxer and decoder) until the minimal build replaces it. The forced demuxer and the jail bound the exposure.
