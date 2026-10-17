# ABShare — working rules

The design is in `SPEC.md`. Keep it current when decisions change: add or amend a row in §1 with the date and the reason.

## Standing rule: security comes first

ABShare faces the internet and holds other people's accounts. Security is the first requirement. Weigh every change against attack surface before convenience.
- Default deny. Every route declares its auth, and every body has a strict schema.
- No client-supplied paths. Treat all metadata as hostile and render it as text.
- ffmpeg runs only through `abshare-jail` (SPEC D19).
- No `unsafe`, no outbound network, no tokens in URLs.
- Don't expose the server to the internet before the Phase 6 gate (SPEC D22).

## Standing rule: no personal or homelab information in this repo

This repository is public. Never commit personal or environment-specific information. That includes:
- IP addresses
- hostnames and domains
- real library contents (titles, paths, counts)
- credentials, tokens and email addresses

This applies to code, tests, fixtures, docs, commit messages and PR descriptions.

- Use placeholders in anything committed: `books.example.com`, `10.0.0.10`, `/srv/audiobooks`, invented titles.
- Real values go only in gitignored local files: `.env.local`, `dev/local.*`.
- Before every commit, scan the staged diff for addresses, hostnames and names.

## Standing rule: tests at every point, security first

Every change ships with tests for security, stability and reliability. No component, endpoint or parser lands without tests.

**Security**
- Every externally reachable surface has negative tests that prove attacks are refused:
  - auth and sessions;
  - every API route, media and downloads;
  - the scanner and path resolution;
  - the jail.
- Every parser of untrusted bytes has a `cargo-fuzz` target and robustness tests, and must never panic.
- Security fixes get a regression test that fails before the fix.
- Each security control has a test that fails when the control is removed.

**Stability**
- Unit and property tests for state machines and edge cases: Range requests, the progress merge, book assembly.

**Reliability**
- Integration tests against a synthetic demo library (`tools/demo/`), covering scan, stream, progress and offline.
- Opt-in live tests read gitignored `dev/local.env` and never write to the library.

All tests run in CI on every push. Don't call work done while any tests are failing or skipped.

## Conventions

- British English in docs and UI strings.
- Clean-room implementation: Audiobookshelf, Booksonic and others are references for behaviour only. Never copy their code.
- `core/` does no I/O. `server/` does it.
- Docs claim only what is built and tested.
