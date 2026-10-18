# Changelog

What changed and when, newest first. Each entry says how it is tested. The design and the reasons behind it are in [`SPEC.md`](SPEC.md); what differs from the design is in SPEC §16.

## Unreleased

### Changed
- **A new look: Paperback, in light and dark.** Paper and ink by day and a warm lamplight look at night, serif titles (Fraunces) over Instrument Sans, covers drawn as books with a spine, and hairline rules in place of cards. Home leads with the book you're reading and a Resume button; the player has a larger cover, the chapter in italics and a fine scrubber; the library has a status switch; genres read like a contents page. On wide screens the tabs become a sidebar and the mini player floats over the page. Every screen was restyled; behaviour is unchanged.
- **Themes are now Paperback and High contrast.** Material and Classic are gone; a saved Material or Classic choice becomes Paperback, keeping light, dark or system.

### Security
- The fonts are bundled into the app and served by ABShare, so the only change to the Content-Security-Policy is `font-src 'self'` (fonts were previously blocked by `default-src 'none'`). No font CDN, no third-party request (SPEC D27).

**Tests:**
- **Browser tests** (Playwright) run the built app against a fresh server on the demo library, in CI on every push: sign-up from an invite (a wrong authenticator code refused), browsing and search, play, chapter skip, speed, resuming after a reload, switching to data saver, downloading a book and playing it with the connection cut, appearance, the bundled fonts, and screenshots of the main screens in light and dark compared with references.
- The theme tests now check every theme and mode for WCAG contrast, including the accent as text and the ink buttons, and that old saved themes are migrated; the CSP test pins `font-src 'self'`. The screenshots were retaken from the demo library.

## 0.9.0 (2026-10-17)

The first tagged release: everything from Phases 0 to 6, plus the work below. Device tests (SPEC §11, Phases 3 and 4) and an independent security review are still to come before 1.0.0.

### Added
- **Listening history.** Settings → History shows your listening time over the last 7 and 30 days and in all, books finished, and recent sessions, built from your progress as you listen (offline listening isn't counted yet). You can turn it off or clear it. It is kept 13 months, and admins can't read it (schema 8).
- **Author and narrator pages.** Every author and narrator on a book page links to all their books, with search and sorting.
- **Bookmark notes** can be added and edited in the app.
- **Admin → Jobs:** the media lanes, playable copies being built, failed copies with *Retry now*, the cache size and *Clear data-saver cache*.
- **Start-up reports.** A blank page shows what went wrong and sends a short text report to the server, where admins read it on the overview. Same-origin only, strictly limited, kept in memory.
- **README with screenshots** and the documentation set: architecture, API, features, building, platforms, performance.
- **A showcase demo library** (`tools/demo/make-showcase.sh`) of invented books, for screenshots and demos.

### Security
- **A minimal ffmpeg.** The image now carries a static ffmpeg and ffprobe built from checksummed FFmpeg 7.1.5 and LAME 3.100 sources, with only the `file` protocol and the demuxers, decoders and encoders ABShare uses. Every base image is pinned by digest. The image went from 770 MB to 149 MB. (WAV files in ADPCM or GSM, and AC-3 audio, are no longer playable: SPEC D10.)
- **Recovery codes force a new factor.** After signing in with one, only the account pages work until a new passkey or authenticator app is set up (schema 7).
- **Removing or replacing a factor** signs out your other devices, as a password change does. **Signing out another device** needs a fresh confirmation, and is rate-limited.
- **Media:** Content-Type comes from the probed container, never the file name, and a file whose size changed since the scan is refused (and rescanned).
- **Request bodies must be JSON objects.** serde also reads a struct from an array of its fields in order, so `["name", "password", "device"]` could sign in; any non-object JSON body is now refused before the handler (found while building history).
- **The password-only step-up route** (which could never succeed since review A4) is removed.
- **Review D1** is closed: only the reverse proxy can reach the server's port on the test deployment.
- **CI now really runs the jail tests.** The runners leak open descriptors, so the jail refused to start and its escape tests skipped silently. CI now closes them and requires Landlock (`ABSHARE_REQUIRE_LANDLOCK=1`).

### Fixed
- The app no longer goes blank where the browser offers no service worker (Chrome when a site may not store data; some other browsers). It says so when cookies are blocked.
- Home's *Recently added* shows a show as one tile, not every episode.
- Admin → Library's *Scan now* asks for the confirmation the server requires.

**Tests:**
- New HTTP tests for every new control, each checked to fail with the control removed.
- An end-to-end test on the demo library: scan with the real jail and ffmpeg, browse, stream, playable copy, data saver, offline download and progress.
- Web tests for the diagnostics, the service-worker fallback, cookies and people links.
- The new image was checked end to end in the hardened container against every accepted format, with the same books, covers and problems as the old image.
- CI runs everything on every push, in debug and release builds.

## Before 0.9.0

Phases 0 to 6 were built on 2026-10-17. They are recorded in [`SPEC.md`](SPEC.md) §11, [`docs/phase0-report.md`](docs/phase0-report.md) and the security reviews in [`docs/security/`](docs/security).
