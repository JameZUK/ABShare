# Changelog

What changed and when, newest first. Each entry says how it is tested. The design and the reasons behind it are in [`SPEC.md`](SPEC.md); what differs from the design is in SPEC §16.

## 0.12.0 (2026-10-23)

A distroless image, and slow connections no longer look like failures.

### Changed
- **The Docker image is now distroless** (`distroless/static`, pinned by digest; SPEC §10). There is no shell, package manager or other program in it, only ABShare's own statically linked binaries and the web app, and it shrank from about 150 MB to 41 MB. It still runs as user 65534, so existing data directories need no change. Run commands by calling `/usr/local/bin/abshare` directly, as DEPLOY.md shows; `docker exec … sh` no longer works.
- **Upgrading: set `libs = []` under `[ffmpeg]`.** An older config may list `/etc/ld.so.cache`, which the image no longer has. ffmpeg has been static since 0.9.0, so it needs no libraries. `serve`, `scan`, `check-config` and `doctor` now refuse a `libs` entry that doesn't exist and name it. Before, the server started but the jail refused every ffmpeg run, and `doctor` blamed the kernel.

- **Slow connections no longer look like failures** (SPEC D36). If the server is slow to answer at start-up, the app opens after 3 seconds for whoever last used the device, with a "Connecting…" banner, instead of showing only the logo. Downloaded books play at once, and everything else fills in when the answer comes. Nothing times out: the app goes offline only when a request actually fails. On a device no one has signed in on, the start screen says "Connecting…", then that the connection is slow.
- **Start-up reports are sent only for real failures.** All 20 reports in the first four days were false alarms: pages loaded in the background (phone locked while listening), whose fade-in hadn't run, or the start screen waiting on a slow connection. Now a page is judged only after 8 seconds on screen, a fade still running is waited for, and still connecting is reported, quietly and labelled, only after 30 seconds. Reports also say whether the page was visible, and the connection type and speed where the browser tells.

### Security
- **ctutils 0.4.5** (RUSTSEC-2026-0385): earlier versions could branch on secret values in constant-time comparisons. It sits under the HMAC and password-hashing code.

### Added
- **[`docs/DEVICE-TESTS.md`](docs/DEVICE-TESTS.md):** the checks only a real phone can do (the lock screen, two devices, airplane mode, the newer features), as a checklist.

**Tests:** a browser test holds back the sign-in answer and checks the app opens as connecting, stays connecting rather than offline, and carries on, with no start-up report. Unit tests cover the session (connecting after 3 s, never offline from slowness alone, a refusal or a different person ending it) and the start-up check (background pages, fades, connecting reported only at 30 s). CI checks that the image has no shell or package manager, and that the web app and ffmpeg are in it, without running anything inside it. The deploy test (the image behind nginx with TLS) passes against the distroless image. `doctor` (with a live jail self-test) and a jailed scan of the demo library pass inside it.

## 0.11.0 (2026-10-18)

A security review of everything added in 0.10, with every finding fixed, and a read-only Admin → Server settings page.

### Added
- **Admin → Server settings.** Shows what the config file sets: the address, listen address, trusted proxies, libraries, ffmpeg jobs, cache limit and whether the web app is served. Also shows the rules built into this version: sign-in and step-up lifetimes, devices per person, invite validity, and history and audit retention. It is read-only: settings are changed in `abshare.toml`, so a stolen admin session can't change how the server behaves. No file paths are shown (SPEC D35).

### Security
The review is in [`docs/security/review-0.11.md`](docs/security/review-0.11.md): a code review of the server and of the web app, plus a black-box test of a local server. It found no Critical or High issues.
- **Offline replay could stall the server for everyone (Medium).** Replays are now limited to 12 a minute per person across all their devices, not per device. The history cap runs once per request, not once per new stretch.
- **Replay could count the same listening again** under a new key, inflating listening totals. Reports a device has already recorded are now skipped. The app also reuses a batch's key when it retries.
- **A comment one word over the limit could be silently cut** instead of refused.
- **Review writes and dismissals are rate-limited**, and are held to 16 KiB like the other library routes. Dismissing refuses a body.
- The new GET routes refuse unknown query parameters, as documented.
- **Dismissing an "everyone" recommendation of books you can't see** now answers "not found", like any recommendation you can't see.
- **Privacy notes:** PRIVACY.md and SPEC D26 now say plainly that admins can see whether someone's history is on, and that the books someone has started or finished still show with history off.
- **"How was it?" memory:** the list of books you were asked about is removed with your other data on *Remove downloads* or remote sign-out.
- **App start-up:** a remote sign-out arriving while the app waits out a "too many renewals" answer now still removes the downloads. An outage during that wait goes offline instead of showing the sign-in screen.
- The HTML-injection scan now also bans inline styles built from data, and checks `action`, `formaction`, `poster` and `xlink:href`.

### Fixed
- Tapping the end point of a "Where was I?" stretch that ran to the end of a book no longer finishes the book at once. Start positions are kept within the book.
- Moving quickly between books could briefly show the previous book's reviews.
- Two reviews by people with the same name, saved in the same second, stopped the reviews from showing.
- You can delete your own review of a book you no longer have access to.
- While the server asks the app to wait, new positions no longer send early.

**Tests:**
- **Server:** a regression test for each server fix, each shown to fail before the fix. They cover the per-person replay limit across two devices, the same batch under three keys counting once, history off recording nothing, replay key pruning, step-up on the three new admin writes, the 16 KiB limit and the review rate limit, dismissal and query strictness, and deleting your own review after losing access.
- **Core and fuzzing:** the comment-limit regression, and a fuzz assertion that accepted text is never shortened.
- **Web unit tests:** the stable replay key, Retry-After respected while listening, the review memory wiped, start-up revocation and outage after a 429, start positions clamped, and stale or duplicate reviews.
- **Browser:** the Server settings page, read-only and without paths.

## 0.10.0 (2026-10-18)

The Paperback redesign, browser tests in CI, reviews, recommendations, "Where was I?", admin listening, offline listening in history, a review prompt and Downloads settings.

### Added
- **Reviews.** Rate any book from 1 to 5 stars with an optional comment of up to 50 words. Everyone who can see the book sees the reviews, each with the reviewer's name, and the average on the book page. Change or delete yours at any time; admins can remove any (SPEC D28).
- **Recommendations.** Admins recommend up to 10 books with a note on why, to everyone or to chosen people (Admin → Recommend). They show at the top of Home until the person chooses *Not now* or starts the book (SPEC D29).
- **"Where was I?"** In a book's menu and the player: your stretches of listening of that book, grouped by day, each with where in the book it began and ended. Tap either to play from there. Only you can see it (SPEC D31).
- **Offline listening counts.** Listening done without a connection is kept on the device and added to your history, "Where was I?" and the admin totals when it reconnects (SPEC D33).
- **"How was it?"** When you finish a book, ABShare asks once for a rating (SPEC D32).
- **Settings → Downloads:** default quality, *Download only on Wi-Fi* (where the browser reports the connection), space used and *Remove all downloads* (SPEC D34).
- **Admin → Listening.** Each person's listening time, books finished, in progress and abandoned (started, not finished, nothing heard for 30 days), and recent sessions; never their position in a book (SPEC D30). The History page no longer says admins can't see it.

### Fixed
- After rating a book in the "How was it?" prompt, the open book page now updates at once.
- An app page starting up while its device had renewed its session too often in the last minute (several tabs, quick reloads) signed out instead of waiting a moment and trying again.

### Changed
- **A new look: Paperback, in light and dark.** Paper and ink by day and a warm lamplight look at night, serif titles (Fraunces) over Instrument Sans, covers drawn as books with a spine, and hairline rules in place of cards. Home leads with the book you're reading and a Resume button; the player has a larger cover, the chapter in italics and a fine scrubber; the library has a status switch; genres read like a contents page. On wide screens the tabs become a sidebar and the mini player floats over the page. Every screen was restyled; behaviour is unchanged.
- **Themes are now Paperback and High contrast.** Material and Classic are gone; a saved Material or Classic choice becomes Paperback, keeping light, dark or system.

### Security
- The fonts are bundled into the app and served by ABShare, so the only change to the Content-Security-Policy is `font-src 'self'` (fonts were previously blocked by `default-src 'none'`). No font CDN, no third-party request (SPEC D27).

**Tests:**
- Server tests for every new route: reviews limited to the book's readers, validated and owned; recommendations reaching only their audience and only books each person can see; dismissing only your own; admin-only creation, withdrawal and review removal with step-up and audit; the timeline private and ordered; the admin view never containing a position. Core: the word limit (with fuzzing), the abandoned rule, and the history rule that a jump starts a new stretch (property-tested). Browser tests for reviews, recommendations, "Where was I?", the admin listening pages, the review prompt, Downloads settings, and offline listening reaching history. Unit tests for the heard-report outbox, replay batching, the Wi-Fi rule and the prompt's memory. Schema 9.
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
