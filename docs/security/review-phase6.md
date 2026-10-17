# Phase 6 security review

Started 2026-10-17. It covers:

- **Code review:** five focused reviews covering auth and sessions; the HTTP surface and admin; media, the filesystem and the jail; the web app; and the untrusted-data parsers in `core`.
- **Black-box pentest:** the release image behind nginx with TLS, run locally with two accounts, plus gentle read-only probes of the LAN test deployment.

**This is not the independent review SPEC §9.4 asks for.** The reviews and the pentest were run with AI assistance. An independent human review is still recommended before public exposure (D22).

**Severity** reflects realistic exploitability for an invite-only service on the internet. Every Critical, High and Medium finding gets a fix with a regression test that fails before the fix.

**Status:** **Fixed** means fixed with a test. **Open** means still to fix. **Accepted** is explained in the row.

## High

| ID | Finding | Status |
|---|---|---|
| J1 | **Jail: x32 system calls bypass the seccomp filter.** seccompiler matched only x86-64 numbers, so a compromised ffmpeg could use x32 numbers (bit `0x40000000`) to call `unshare(CLONE_NEWUSER)`, send UDP, set up `io_uring` and call `memfd_create`. Verified by running it. | **Fixed:** a raw BPF guard kills any foreign-ABI call; escape tests. |
| H1 | **The connection cap and missing body timeout let anyone block the service.** The per-address cap of 64 applies to the proxy, so behind nginx it caps the whole service. With no request-body timeout, slow bodies hold those connections. | **Fixed:** trusted proxies are exempt from the per-address cap, bodies are read within 15 s, and the first byte must arrive within 15 s; nginx buffers request bodies except for media; tests over real TCP. |
| C1 | **Scanner crash from a tag:** `book_number` slices the original string at an index found in its lower-cased copy (for example `İbook é`). With `panic = "abort"` this kills the scanner. | **Fixed:** ASCII lower-casing keeps byte offsets; regression test. |
| C2 | **The same crash through `split_narrated_by`** (for example `İ(narrated by é)`). | **Fixed:** the same, also in `read_by`; fuzz dictionary of case-changing characters. |

## Medium

| ID | Finding | Status |
|---|---|---|
| J2 | **Jail: UNIX sockets.** It could connect to pathname UNIX sockets on the host, such as a user D-Bus or a container runtime socket. | **Fixed:** all sockets refused; escape test. |
| P1 | **Anonymous rate limits keyed on the full IPv6 address.** Rotating through one /64 bypassed them. Found by the pentest and by two reviews. | **Fixed:** keyed like the lockout (/64); test. |
| A1 | **Refresh-token reuse detection can be evaded, and a lost response wipes downloads.** Used tokens were deleted after 2 minutes, siblings issued in the grace window rotate forever, and a renewal response lost for more than 60 s is treated as theft. That revokes the device and wipes its downloads. | **Fixed:** spent tokens are kept for the session, so old copies always read as reuse; a retry is allowed (bounded) until a replacement is used; superseded siblings count as theft; reuse returns `session_reused` (sign out, keep downloads), not `device_revoked`; tests. |
| A2 | **Strangers can keep a username throttled** from fresh /64s, using the shared allowance. Separately, flooding the lockout table evicts a username's soft lock. | **Fixed:** a browser with a live sign-in for the account skips the shared allowance; locked or recently failed entries are never evicted, and a full table refuses new entries (fail closed); username keys are bounded; tests. |
| H2 | **Search can burn CPU.** `q` is unbounded in words, and work is done for every word and every book. | **Fixed:** 200 characters and 8 distinct words at most; test. |
| M1 | **Symlink races in library files.** They are re-opened by path, following symlinks, after the check: sidecar text, cover sidecars, original streaming and ffmpeg inputs. | Open |
| M2 | **Book IDs from ASINs are global.** A book in another library can take over the ID (and its caches). Caches also aren't invalidated when a book's parts change, which can panic the slice in a segment. | Open |
| M3 | **Data saver without a Range header encodes the whole book.** Read-ahead tasks are unbounded. | **Fixed:** no Range is treated as `bytes=0-` and capped; at most 4 read-aheads at once; a stale short segment ends the response instead of panicking; test. |
| C3 | **Quadratic tag merge in ffprobe parsing.** 80k × 80k tags stall a thread for 34 s. | **Fixed:** linear, bounded merge; timing test. |
| C4 | **Book IDs in flat folders move** when albums are added (`dir#n`), and `X#0` can collide. | Open |

## Low

| ID | Finding | Status |
|---|---|---|
| A3 | Passkey sign-in start reveals whether a username exists and has passkeys. | Open |
| A4 | Step-up accepts a password alone when the account also has a passkey. | Open |
| A5 | The audit chain can't detect a cut tail, and the prune anchor isn't authenticated. | Open |
| A6 | Duplicate cookie names (planted from a sibling subdomain) are accepted, and the first one wins. | Open |
| A7 | Audit-log flushing through unaudited-cost failures (summary rows, separate pruning). | Open |
| H3 | The Compose example publishes 8743 on every interface. | **Fixed:** binds loopback by default, with guidance. |
| H4 | Admin "scan now" and book-detail edits skip step-up. | Open |
| H5 | The rate-limit map scans everything when full. | Open |
| H6 | No per-user storage caps (bookmarks), and edited bookmark positions aren't clamped. | Open |
| H7 | The revocation sync window can miss a commit at the boundary. | Open |
| H8 | Group and show listings don't filter members by library permission. | Open |
| H9 | Promoting a user to admin from the users screen skips the second confirmation that invites need. | Open |
| H10 | The cover route's own CSP is overwritten by the global one. | Open |
| M4 | Cache "pinned" uses mtime, so new encodes can overrun `max_gib`. `jobs/` isn't cleaned, and lock entries leak. | Open |
| M5 | Failed playable-copy builds retry without limit, and books with many parts hit the fd limit. | Open |
| C5 | Saver maths underflows for segments shorter than the pre-roll (latent). | Open |
| C6 | Invisible characters survive `clean_line` (U+061C, tag characters, variation selectors, Hangul fillers, unbounded combining marks). | Open |
| C7 | A successful fifth attempt still locks the address. | **Fixed:** the lock is decided when the next attempt starts; test. |
| W1 | Route params are decoded and spliced into API paths (`%2F`). | Open |
| W2 | Completing an invite while signed in as someone else keeps the old player and outbox. | Open |
| W3 | The revocation wipe can be blocked by another tab's open database. | Open |
| W4 | Gaps in the HTML-injection source scan and the boot externaliser. | Open |
| W5 | Per-book speeds and the last user survive sign-out with "Remove downloads". | Open |
| D1 | Test deployment: ABShare's port is reachable from the whole LAN, not only from the reverse proxy. The server refuses such requests (403: not a trusted proxy, no TLS), but the network should allow only the proxy. | Open: a firewall change on the test network. |

## Informational

- A Host with a trailing dot (`books.example.com.`) is accepted. It is the same host.
- Offline mode serves the last user's downloads without the server. Per-user databases separate users in the UI but aren't a security boundary on a shared browser profile. Accepted by design (§8); to be stated in the SPEC.
- axum's built-in body and query rejections return plain-text messages that name expected fields.
- `login.failed` audit entries record the typed username, which may be a mistyped password.
- Recovery codes can be lost if sign-in fails after enrolment commits.
- SPEC §5.5 mentions a configurable refresh lifetime, but it is a constant.
- `/v1/admin/audit` re-verifies the whole chain on every request (admin only).
- Fuzz targets are missing for metadata overrides, series grouping, genre shelves, chapter assembly, the auth validators and the lockout state machine.
